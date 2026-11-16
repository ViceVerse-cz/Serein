/* Exercise the patched FFmpeg device selection with a fake Vulkan driver. */
#include <assert.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>

typedef struct {
    PFN_vkEnumeratePhysicalDevices EnumeratePhysicalDevices;
    PFN_vkEnumerateDeviceExtensionProperties EnumerateDeviceExtensionProperties;
    PFN_vkGetPhysicalDeviceProperties2 GetPhysicalDeviceProperties2;
    PFN_vkGetPhysicalDeviceFeatures2 GetPhysicalDeviceFeatures2;
} FFVulkanFunctions;
typedef struct {
    VkInstance inst;
    VkPhysicalDevice phys_dev;
    VkPhysicalDeviceFeatures2 device_features;
    const char *const *enabled_dev_extensions;
    int nb_enabled_dev_extensions;
} AVVulkanDeviceContext;
typedef struct {
    AVVulkanDeviceContext p;
    struct { FFVulkanFunctions vkfn; uint64_t extensions; } vkctx;
    VkPhysicalDeviceVulkan12Features device_features_1_2;
    VkPhysicalDeviceCoherentMemoryFeaturesAMD serein_amf_coherent;
} VulkanDevicePriv;
typedef struct { void *hwctx; } AVHWDeviceContext;

#define FF_VK_EXT_DEVICE_DRM (1ULL << 9)
#define AVERROR(e) (-(e))
#define AVERROR_EXTERNAL (-10000)
#define AV_LOG_ERROR 0
#define AV_LOG_VERBOSE 1
#define av_calloc calloc
#define av_free free
static void *av_malloc_array(size_t count, size_t size) {
    return size && count > SIZE_MAX / size ? NULL : malloc(count * size);
}
static void av_log(void *context, int level, const char *message, ...) {
    (void)context; (void)level; (void)message;
}
static const char *ff_vk_ret2str(VkResult result) { (void)result; return "mock"; }

#include "vulkan_device_selection.inc"

static void ff_vk_link_struct(void *first, void *extension) {
    VkBaseOutStructure *current = first;
    while (current->pNext) current = current->pNext;
    current->pNext = extension;
}
#include "vulkan_amf_features.inc"

static int coherent_queries, coherent_supported;
static VKAPI_ATTR void VKAPI_CALL device_features(VkPhysicalDevice device, VkPhysicalDeviceFeatures2 *features) {
    (void)device;
    VkPhysicalDeviceCoherentMemoryFeaturesAMD *coherent = features->pNext;
    assert(coherent && coherent->sType == VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_COHERENT_MEMORY_FEATURES_AMD);
    coherent->deviceCoherentMemory = coherent_supported;
    coherent_queries++;
}

static void amf_features(void) {
    const char *extensions[] = {"VK_AMD_device_coherent_memory"};
    for (int advertised = 0; advertised < 2; ++advertised) {
        for (coherent_supported = 0; coherent_supported < 2; ++coherent_supported) {
            VulkanDevicePriv state = {0};
            AVHWDeviceContext context = {.hwctx = &state};
            VkPhysicalDeviceFeatures2 supported = {0};
            VkPhysicalDeviceVulkan12Features supported12 = {0};
            supported.features.robustBufferAccess = VK_TRUE;
            supported12.descriptorIndexing = VK_TRUE;
            supported12.descriptorBindingStorageBufferUpdateAfterBind = VK_TRUE;
            state.p.device_features.pNext = &state.device_features_1_2;
            state.p.enabled_dev_extensions = extensions;
            state.p.nb_enabled_dev_extensions = advertised;
            state.vkctx.vkfn.GetPhysicalDeviceFeatures2 = device_features;
            coherent_queries = 0;
            serein_vk_amf_features(&context, &supported, &supported12);
            assert(state.p.device_features.features.robustBufferAccess);
            assert(!state.p.device_features.features.geometryShader);
            assert(state.device_features_1_2.descriptorIndexing);
            assert(state.device_features_1_2.descriptorBindingStorageBufferUpdateAfterBind);
            assert(!state.device_features_1_2.descriptorBindingSampledImageUpdateAfterBind);
            assert(coherent_queries == advertised);
            assert((state.device_features_1_2.pNext != NULL) == (advertised && coherent_supported));
            if (advertised && coherent_supported)
                assert(state.device_features_1_2.pNext == &state.serein_amf_coherent);
        }
    }
}

typedef struct {
    int has_extension;
    VkBool32 has_primary, has_render;
    int64_t primary_major, primary_minor, render_major, render_minor;
    int enumeration_fails, incomplete_count, excessive_count;
} MockDevice;
static MockDevice gpus[3];
static uint32_t gpu_count;
static int extension_queries;

static MockDevice *device_data(VkPhysicalDevice device) {
    uintptr_t id = (uintptr_t)device;
    assert(id >= 1 && id <= gpu_count);
    return &gpus[id - 1];
}
static VKAPI_ATTR VkResult VKAPI_CALL enumerate_devices(
    VkInstance instance, uint32_t *count, VkPhysicalDevice *devices) {
    (void)instance;
    if (!devices) { *count = gpu_count; return VK_SUCCESS; }
    assert(*count >= gpu_count);
    for (uint32_t i = 0; i < gpu_count; ++i)
        devices[i] = (VkPhysicalDevice)(uintptr_t)(i + 1);
    *count = gpu_count;
    return VK_SUCCESS;
}
static VKAPI_ATTR VkResult VKAPI_CALL enumerate_extensions(
    VkPhysicalDevice device, const char *layer, uint32_t *count,
    VkExtensionProperties *properties) {
    MockDevice *gpu = device_data(device);
    (void)layer;
    ++extension_queries;
    if (gpu->enumeration_fails) return VK_ERROR_INITIALIZATION_FAILED;
    if (!properties) { *count = gpu->excessive_count ? 5000 : 1; return VK_SUCCESS; }
    assert(*count >= 1);
    snprintf(properties[0].extensionName, sizeof(properties[0].extensionName), "%s",
             gpu->has_extension ? VK_EXT_PHYSICAL_DEVICE_DRM_EXTENSION_NAME : "VK_FAKE_other");
    *count = 1;
    if (gpu->incomplete_count > 0) { --gpu->incomplete_count; return VK_INCOMPLETE; }
    return VK_SUCCESS;
}
static VKAPI_ATTR void VKAPI_CALL device_properties(
    VkPhysicalDevice device, VkPhysicalDeviceProperties2 *properties) {
    MockDevice *gpu = device_data(device);
    VkPhysicalDeviceIDProperties *id = properties->pNext;
    assert(id && id->sType == VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES);
    snprintf(properties->properties.deviceName, VK_MAX_PHYSICAL_DEVICE_NAME_SIZE,
             "mock device %lu", (unsigned long)(uintptr_t)device);
    properties->properties.vendorID = 0x1002;
    properties->properties.deviceType = VK_PHYSICAL_DEVICE_TYPE_DISCRETE_GPU;
    if (id->pNext) {
        VkPhysicalDeviceDrmPropertiesEXT *drm = id->pNext;
        assert(gpu->has_extension);
        assert(drm->sType == VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_DRM_PROPERTIES_EXT);
        drm->hasPrimary = gpu->has_primary;
        drm->hasRender = gpu->has_render;
        drm->primaryMajor = gpu->primary_major;
        drm->primaryMinor = gpu->primary_minor;
        drm->renderMajor = gpu->render_major;
        drm->renderMinor = gpu->render_minor;
    }
}
static int select_device(VulkanDeviceSelection *selection, uintptr_t *selected) {
    VulkanDevicePriv state = {0};
    AVHWDeviceContext context = { .hwctx = &state };
    state.vkctx.vkfn.EnumeratePhysicalDevices = enumerate_devices;
    state.vkctx.vkfn.EnumerateDeviceExtensionProperties = enumerate_extensions;
    state.vkctx.vkfn.GetPhysicalDeviceProperties2 = device_properties;
    int result = find_device(&context, selection);
    *selected = (uintptr_t)state.p.phys_dev;
    return result;
}
static void reset_gpus(void) {
    memset(gpus, 0, sizeof(gpus));
    gpu_count = 3;
    extension_queries = 0;
}
int main(void) {
    amf_features();
    uintptr_t selected;
    VulkanDeviceSelection selection = { .has_drm = 1, .drm_major = 226,
                                        .drm_minor = 129, .vendor_id = 0x1002 };

    reset_gpus();
    gpus[1] = (MockDevice){ .has_extension = 1, .has_render = 1,
                            .render_major = 226, .render_minor = 129 };
    gpus[2] = (MockDevice){ .has_extension = 1, .has_render = 1,
                            .render_major = 226, .render_minor = 130 };
    assert(select_device(&selection, &selected) == 0 && selected == 2);
    assert(extension_queries > 0);

    /* An advertised extension on a different GPU must not cause a vendor fallback. */
    reset_gpus();
    gpus[1] = (MockDevice){ .has_extension = 1, .has_render = 1,
                            .render_major = 226, .render_minor = 130 };
    assert(select_device(&selection, &selected) < 0 && selected == 0);

    reset_gpus();
    assert(select_device(&selection, &selected) < 0 && selected == 0);

    /* A driver must explicitly advertise that a matching node exists. */
    reset_gpus();
    gpus[0] = (MockDevice){ .has_extension = 1, .render_major = 226,
                            .render_minor = 129 };
    assert(select_device(&selection, &selected) < 0 && selected == 0);

    reset_gpus();
    gpus[0] = (MockDevice){ .has_extension = 1, .has_primary = 1,
                            .primary_major = 226, .primary_minor = 129 };
    assert(select_device(&selection, &selected) == 0 && selected == 1);

    reset_gpus();
    gpus[0].enumeration_fails = 1;
    assert(select_device(&selection, &selected) < 0 && selected == 0);

    /* A broken unrelated Vulkan driver must not hide an exact match. */
    reset_gpus();
    gpus[0].enumeration_fails = 1;
    gpus[1] = (MockDevice){ .has_extension = 1, .has_render = 1,
                            .render_major = 226, .render_minor = 129 };
    assert(select_device(&selection, &selected) == 0 && selected == 2);

    reset_gpus();
    gpus[0].excessive_count = 1;
    assert(select_device(&selection, &selected) < 0 && selected == 0);

    reset_gpus();
    gpus[0] = (MockDevice){ .has_extension = 1, .has_render = 1,
                            .render_major = 226, .render_minor = 129,
                            .incomplete_count = 1 };
    assert(select_device(&selection, &selected) == 0 && selected == 1);

    reset_gpus();
    gpus[0] = (MockDevice){ .has_extension = 1, .has_render = 1,
                            .render_major = 226, .render_minor = 129,
                            .incomplete_count = 10 };
    assert(select_device(&selection, &selected) < 0 && selected == 0);

    reset_gpus();
    selection = (VulkanDeviceSelection){ .index = 1 };
    assert(select_device(&selection, &selected) == 0 && selected == 2);
    assert(extension_queries == 0);
    puts("Vulkan selection: exact DRM matching, unsupported devices and bounded failures passed");
    return 0;
}
