/* Each variant remains a separately compiled offline fixture. */

#if defined(SEREIN_AMF_PROPERTY_FIXTURE)
/* Return a real C++ SDK property record: its virtual destructor means the C
 * record has a different layout. The C ABI mock only forwards this pointer. */
#include <AMF/core/PropertyStorageEx.h>
#include <AMF/components/VideoEncoderVCE.h>
#include <AMF/components/VideoEncoderHEVC.h>
#include <AMF/components/VideoEncoderAV1.h>
extern "C" const void *fixture_amf_property(int codec, int variant) {
    static amf::AMFPropertyInfo info;
    static amf::AMFEnumDescriptionEntry entries[32];
    const int values[3][4] = {
        {AMF_VIDEO_ENCODER_QUALITY_PRESET_SPEED, AMF_VIDEO_ENCODER_QUALITY_PRESET_BALANCED,
         AMF_VIDEO_ENCODER_QUALITY_PRESET_QUALITY, AMF_VIDEO_ENCODER_QUALITY_PRESET_HIGH_QUALITY},
        {AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_SPEED, AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_BALANCED,
         AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_QUALITY, AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_HIGH_QUALITY},
        {AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_SPEED, AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_BALANCED,
         AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_QUALITY, AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_HIGH_QUALITY}};
    info.type = variant == 1 ? amf::AMF_VARIANT_BOOL : amf::AMF_VARIANT_INT64;
    info.accessType = variant == 2 ? amf::AMF_PROPERTY_ACCESS_READ : amf::AMF_PROPERTY_ACCESS_FULL;
    for (unsigned int i = 0; i < 32; ++i) {
        entries[i].value = values[codec][i % 4];
        entries[i].name = i < 4 || variant == 3 ? L"preset" : nullptr;
    }
    if (variant == 4) entries[0].name = nullptr;
    info.pEnumDescription = variant == 5 ? nullptr : entries;
    return &info;
}

#elif defined(SEREIN_AMF_SCOPED_FIXTURE)
/* Exact AMD device handles cross the real scoped query ABI. No real loader,
 * driver, encoder initialization or frame submission occurs. */
#include "video_encode_ffmpeg.h"
#include <cassert>
#include <cstdlib>
#include <cstring>
#include <AMF/core/Factory.h>
#ifdef NDEBUG
#error AMF GPU fixtures require assertions
#endif
extern "C" {
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
void *__real_dlopen(const char *name, int flags);
}

static int devices, released;
extern "C" void *__wrap_dlopen(const char *name, int flags) {
    assert(!std::strcmp(name, AMF_DLL_NAMEA));
    return __real_dlopen(std::getenv("SEREIN_AMF_TEST_RUNTIME"), flags);
}

extern "C" void *__wrap_serein_video_vulkan_device(const SereinVideoAdapter *target, const char *extensions) {
    assert(target && target->vendor_id == 0x1002);
    assert(extensions && std::strstr(extensions, "VK_KHR_swapchain+VK_KHR_synchronization2+"));
    assert(std::strstr(extensions, "+VK_AMD_device_coherent_memory+"));
    assert(std::strstr(extensions, "+VK_EXT_pci_bus_info+"));
    if (target->bus != 1 && target->bus != 2)
        return nullptr;
    AVBufferRef *ref = new AVBufferRef{};
    AVHWDeviceContext *device = new AVHWDeviceContext{};
    AVVulkanDeviceContext *native = new AVVulkanDeviceContext{};
    native->inst = reinterpret_cast<VkInstance>(uintptr_t(1));
    native->phys_dev = reinterpret_cast<VkPhysicalDevice>(uintptr_t(target->bus));
    native->act_dev = reinterpret_cast<VkDevice>(uintptr_t(target->bus + 100));
    device->type = AV_HWDEVICE_TYPE_VULKAN;
    device->hwctx = native;
    ref->data = reinterpret_cast<uint8_t *>(device);
    devices++;
    return ref;
}

extern "C" void av_buffer_unref(AVBufferRef **reference) {
    if (*reference) {
        AVHWDeviceContext *device = reinterpret_cast<AVHWDeviceContext *>((*reference)->data);
        delete static_cast<AVVulkanDeviceContext *>(device->hwctx);
        delete device;
        delete *reference;
        *reference = nullptr;
        released++;
    }
}

int main() {
    SereinVideoAdapter target = {SEREIN_GPU_PCI, 0x1002, 0x744c, 0, 1, 0, 0, 0};
    assert(serein_query_amf_on_adapter(2, &target) == 0);
    target.bus = 2;
    assert(serein_query_amf_on_adapter(2, &target) == 1);
    assert(devices == 2 && released == 2);
    assert(setenv("SEREIN_HDR_FIXTURE","ten",1) == 0);
    assert(serein_query_amf_on_adapter(5, &target) == 1);
    assert(serein_query_amf_on_adapter(4, &target) == 1);
    assert(setenv("SEREIN_HDR_FIXTURE","eight",1) == 0);
    assert(serein_query_amf_on_adapter(5, &target) == 0);
    assert(serein_query_amf_on_adapter(2, &target) == 1);
    assert(setenv("SEREIN_HDR_FIXTURE","main",1) == 0);
    assert(serein_query_amf_on_adapter(4, &target) == 0);
    assert(setenv("SEREIN_HDR_FIXTURE","error",1) == 0);
    assert(serein_query_amf_on_adapter(4, &target) == -1);
    assert(setenv("SEREIN_HDR_FIXTURE","bad-type",1) == 0);
    assert(serein_query_amf_on_adapter(4, &target) == -1);
    assert(serein_query_amf_on_adapter(3, &target) == 0);
    assert(unsetenv("SEREIN_HDR_FIXTURE") == 0);
    const struct { const char *mode; int expected; } presets[] = {
        {"scoped", 0xf00}, {"preset-error", -1}, {"preset-null", -1},
        {"preset-bad-type", -1}, {"preset-read-only", -1}, {"preset-unbounded", -1},
        {"preset-none", 0}, {"preset-no-enum", -1}, {"preset-rejected", 0}
    };
    for (const auto &test : presets) {
        assert(setenv("AMF_MOCK_MODE", test.mode, 1) == 0);
        for (int codec = 6; codec < 9; ++codec)
            assert(serein_query_amf_on_adapter(codec, &target) == test.expected);
    }
    assert(setenv("AMF_MOCK_MODE", "scoped", 1) == 0);
    AVBufferRef *encoding_device = static_cast<AVBufferRef *>(serein_video_amf_device(&target));
    assert(encoding_device);
    av_buffer_unref(&encoding_device);
    const int previous = devices;
    const char *invalid_extensions[] = {"extensions-error", "extensions-too-many", "extensions-invalid",
                                        "extensions-null", "extensions-growing"};
    for (const char *mode : invalid_extensions) {
        assert(setenv("AMF_MOCK_MODE", mode, 1) == 0);
        assert(serein_query_amf_on_adapter(2, &target) == -1);
        assert(!serein_video_amf_device(&target));
        assert(devices == previous);
    }
    assert(setenv("AMF_MOCK_MODE", "scoped", 1) == 0);
    target.bus = 3;
    assert(serein_query_amf_on_adapter(2, &target) == -1);
    target.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(serein_query_amf_on_adapter(2, &target) == -1);
    assert(devices == released);
    assert(devices == 37);
}

#elif !defined(SEREIN_AMF_SCOPED_FIXTURE)
#include <cstdlib>
#include <cassert>
#include <cstring>
#include <AMF/core/Factory.h>

extern "C" int serein_query_amf(int codec);
extern "C" void *__real_dlopen(const char *name, int flags);

/* Link with --wrap=dlopen to keep every case offline, including a missing
 * runtime. Falling through the host library search could load a real driver. */
extern "C" void *__wrap_dlopen(const char *name, int flags) {
    assert(std::strcmp(name, AMF_DLL_NAMEA) == 0);
    const char *fixture = std::getenv("SEREIN_AMF_TEST_RUNTIME");
    assert(fixture);
    return __real_dlopen(fixture, flags);
}

int main(int argc, char **argv) {
    assert(argc == 3);
    const int result = serein_query_amf(std::atoi(argv[1]));
    assert(result == std::atoi(argv[2]));
}

#else
#error Unknown offline fixture variant
#endif
