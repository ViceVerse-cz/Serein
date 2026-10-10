#if defined(__linux__)
#define _POSIX_C_SOURCE 200809L
#elif defined(__APPLE__)
/* RTLD_DEFAULT is a Darwin extension hidden by strict POSIX feature selection. */
#define _DARWIN_C_SOURCE
#endif
/* Explicit renderer GPU binding. Never guess a physical device from a vendor,
 * GPU model name, performance preference, or an encoder's default adapter. */
#include "video_gpu.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
#include <libavutil/opt.h>

#if defined(_WIN32)
#ifndef COBJMACROS
#define COBJMACROS
#endif
#include <windows.h>
#include <dxgi.h>
#include <d3d11.h>
#include <libavutil/hwcontext_d3d11va.h>
#elif defined(__linux__)
#include <dirent.h>
#include <limits.h>
#include <unistd.h>
#include <dlfcn.h>
#elif defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#include <VideoToolbox/VideoToolbox.h>
#include <dlfcn.h>
#endif

int serein_video_adapter_valid(const SereinVideoAdapter *a)
{
    if (!a)
        return 0;
    switch (a->identity) {
    case SEREIN_GPU_WINDOWS_LUID:
    case SEREIN_GPU_METAL_REGISTRY:
        return a->value != 0;
    case SEREIN_GPU_PCI:
        return a->bus <= 255 && a->slot <= 31 && a->function <= 7;
    default:
        return 0;
    }
}

int serein_video_adapter_equal(const SereinVideoAdapter *a, const SereinVideoAdapter *b)
{
    if (!serein_video_adapter_valid(a) || !serein_video_adapter_valid(b) ||
        a->identity != b->identity || a->vendor_id != b->vendor_id || a->device_id != b->device_id)
        return 0;
    if (a->identity == SEREIN_GPU_PCI)
        return a->domain == b->domain && a->bus == b->bus &&
               a->slot == b->slot && a->function == b->function;
    return a->value == b->value;
}

#if defined(SEREIN_HAVE_NVENC_QUERY) && SEREIN_HAVE_NVENC_QUERY && \
    (defined(_WIN32) || defined(__linux__))
#include <ffnvcodec/dynlink_cuda.h>

int serein_video_cuda_device(const SereinVideoAdapter *a)
{
    tcuInit *init = NULL;
    tcuDeviceGetCount *count = NULL;
    tcuDeviceGet *get = NULL;
    tcuDeviceGetLuid *luid = NULL;
    tcuDeviceGetPCIBusId *pci = NULL;
    int devices = 0, result = -1;
    if (!serein_video_adapter_valid(a) || a->vendor_id != 0x10de)
        return -1;
#if defined(_WIN32)
    if (a->identity != SEREIN_GPU_WINDOWS_LUID)
        return -1;
    HMODULE library = LoadLibraryExW(L"nvcuda.dll", NULL, LOAD_LIBRARY_SEARCH_SYSTEM32);
#define SEREIN_CUDA_SYMBOL(name) GetProcAddress(library, name)
#else
    if (a->identity != SEREIN_GPU_PCI)
        return -1;
    void *library = dlopen("libcuda.so.1", RTLD_NOW | RTLD_LOCAL);
#define SEREIN_CUDA_SYMBOL(name) dlsym(library, name)
#endif
    if (!library)
        return -1;
#define SEREIN_LOAD_CUDA(member, name) do { \
        void (*symbol)(void) = (void (*)(void))SEREIN_CUDA_SYMBOL(name); \
        if (sizeof(member) != sizeof(symbol) || !symbol) goto cleanup; \
        memcpy(&(member), &symbol, sizeof(member)); \
    } while (0)
    SEREIN_LOAD_CUDA(init, "cuInit");
    SEREIN_LOAD_CUDA(count, "cuDeviceGetCount");
    SEREIN_LOAD_CUDA(get, "cuDeviceGet");
    if (a->identity == SEREIN_GPU_WINDOWS_LUID) {
        SEREIN_LOAD_CUDA(luid, "cuDeviceGetLuid");
    } else {
        SEREIN_LOAD_CUDA(pci, "cuDeviceGetPCIBusId");
    }
    if (init(0) != CUDA_SUCCESS || count(&devices) != CUDA_SUCCESS || devices < 0)
        goto cleanup;
    for (int index = 0; index < devices && index < 32; index++) {
        CUdevice device;
        if (get(&device, index) != CUDA_SUCCESS)
            continue;
        if (luid) {
            uint64_t identity = 0;
            unsigned int mask = 0;
            if (luid((char *)&identity, &mask, device) == CUDA_SUCCESS && identity == a->value && mask) {
                result = index;
                break;
            }
        } else {
            char address[32] = {0};
            unsigned int domain, bus, slot, function;
            char suffix;
            const CUresult found = pci(address, (int)sizeof(address), device);
            address[sizeof(address) - 1] = '\0';
            if (found == CUDA_SUCCESS &&
                sscanf(address, "%x:%x:%x.%x%c", &domain, &bus, &slot, &function, &suffix) == 4 &&
                domain == a->domain && bus == a->bus && slot == a->slot && function == a->function) {
                result = index;
                break;
            }
        }
    }
cleanup:
#if defined(_WIN32)
    FreeLibrary(library);
#else
    dlclose(library);
#endif
    return result;
#undef SEREIN_CUDA_SYMBOL
#undef SEREIN_LOAD_CUDA
}
#else
int serein_video_cuda_device(const SereinVideoAdapter *a) { (void)a; return -1; }
#endif

int serein_video_dxgi_device(const SereinVideoAdapter *a)
{
#if defined(_WIN32)
    if (!serein_video_adapter_valid(a) || a->identity != SEREIN_GPU_WINDOWS_LUID)
        return -1;
    HMODULE library = LoadLibraryExW(L"dxgi.dll", NULL, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!library)
        return -1;
    HRESULT (WINAPI *create)(REFIID, void **) = NULL;
    FARPROC symbol = GetProcAddress(library, "CreateDXGIFactory1");
    memcpy(&create, &symbol, sizeof(create));
    IDXGIFactory1 *factory = NULL;
    const IID iid = {0x770aae78, 0xf26f, 0x4dba, {0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87}};
    int result = -1;
    if (!create || FAILED(create(&iid, (void **)&factory)) || !factory)
        goto cleanup;
    for (UINT index = 0; index < 32; index++) {
        IDXGIAdapter1 *adapter = NULL;
        if (FAILED(IDXGIFactory1_EnumAdapters1(factory, index, &adapter)) || !adapter)
            break;
        DXGI_ADAPTER_DESC1 description = {0};
        if (SUCCEEDED(IDXGIAdapter1_GetDesc1(adapter, &description))) {
            uint64_t identity = 0;
            memcpy(&identity, &description.AdapterLuid, sizeof(identity));
            if (identity == a->value && description.VendorId == a->vendor_id &&
                description.DeviceId == a->device_id && !(description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE))
                result = (int)index;
        }
        IDXGIAdapter1_Release(adapter);
        if (result >= 0)
            break;
    }
cleanup:
    if (factory)
        IDXGIFactory1_Release(factory);
    FreeLibrary(library);
    return result;
#else
    (void)a;
    return -1;
#endif
}

#if defined(_WIN32)
static int d3d11_matches(AVBufferRef *device_ref, const SereinVideoAdapter *a)
{
    if (!device_ref || a->identity != SEREIN_GPU_WINDOWS_LUID)
        return 0;
    AVHWDeviceContext *context = (AVHWDeviceContext *)device_ref->data;
    if (context->type != AV_HWDEVICE_TYPE_D3D11VA || !context->hwctx)
        return 0;
    AVD3D11VADeviceContext *device = (AVD3D11VADeviceContext *)context->hwctx;
    if (!device->device)
        return 0;
    const IID iid = {0x54ec77fa, 0x1377, 0x44e6, {0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c}};
    IDXGIDevice *dxgi = NULL;
    IDXGIAdapter *adapter = NULL;
    DXGI_ADAPTER_DESC description = {0};
    int matching = 0;
    if (SUCCEEDED(ID3D11Device_QueryInterface(device->device, &iid, (void **)&dxgi)) && dxgi &&
        SUCCEEDED(IDXGIDevice_GetAdapter(dxgi, &adapter)) && adapter &&
        SUCCEEDED(IDXGIAdapter_GetDesc(adapter, &description))) {
        uint64_t luid = 0;
        memcpy(&luid, &description.AdapterLuid, sizeof(luid));
        matching = luid == a->value && description.VendorId == a->vendor_id &&
                   description.DeviceId == a->device_id;
    }
    if (adapter) IDXGIAdapter_Release(adapter);
    if (dxgi) IDXGIDevice_Release(dxgi);
    return matching;
}
#endif

int serein_video_drm_device(const SereinVideoAdapter *a, char *path, size_t capacity)
{
#if defined(__linux__)
    if (!path || capacity < 32 || !serein_video_adapter_valid(a) || a->identity != SEREIN_GPU_PCI)
        return 0;
    /* The physical PCI address is the kernel's sysfs link target. A bounded
     * render-node enumeration avoids assuming renderD128 belongs to Intel. */
    for (unsigned int node = 128; node < 256; node++) {
        char link[80], target[512];
        snprintf(link, sizeof(link), "/sys/class/drm/renderD%u/device", node);
        const ssize_t bytes = readlink(link, target, sizeof(target) - 1);
        if (bytes <= 0 || (size_t)bytes >= sizeof(target) - 1)
            continue;
        target[bytes] = '\0';
        const char *address = strrchr(target, '/');
        unsigned int domain, bus, slot, function;
        char suffix;
        if (address && sscanf(address + 1, "%x:%x:%x.%x%c", &domain, &bus, &slot, &function, &suffix) == 4 &&
            domain == a->domain && bus == a->bus && slot == a->slot && function == a->function) {
            const int written = snprintf(path, capacity, "/dev/dri/renderD%u", node);
            return written > 0 && (size_t)written < capacity;
        }
    }
#else
    (void)a; (void)path; (void)capacity;
#endif
    return 0;
}

void *serein_video_vt_specification(const SereinVideoAdapter *a)
{
#if defined(__APPLE__)
    if (a && (!serein_video_adapter_valid(a) || a->identity != SEREIN_GPU_METAL_REGISTRY))
        return NULL;
    CFMutableDictionaryRef specification = CFDictionaryCreateMutable(NULL, 2,
        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (!specification)
        return NULL;
    CFDictionarySetValue(specification,
        kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder, kCFBooleanTrue);
    if (a) {
        const CFStringRef *required = (const CFStringRef *)dlsym(RTLD_DEFAULT,
            "kVTVideoEncoderSpecification_RequiredEncoderGPURegistryID");
        if (!required || !*required) {
            CFRelease(specification);
            return NULL;
        }
        const int64_t identity = (int64_t)a->value;
        CFNumberRef number = CFNumberCreate(NULL, kCFNumberSInt64Type, &identity);
        if (!number) {
            CFRelease(specification);
            return NULL;
        }
        CFDictionarySetValue(specification, *required, number);
        CFRelease(number);
    }
    return (void *)specification;
#else
    (void)a;
    return NULL;
#endif
}

void *serein_video_vulkan_device(const SereinVideoAdapter *a)
{
#if defined(__linux__) && defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
    AVBufferRef *drm = NULL, *vulkan = NULL;
    char path[80];
    if (!serein_video_drm_device(a, path, sizeof(path)) ||
        av_hwdevice_ctx_create(&drm, AV_HWDEVICE_TYPE_DRM, path, NULL, 0) < 0)
        return NULL;
    const int created = av_hwdevice_ctx_create_derived(&vulkan, AV_HWDEVICE_TYPE_VULKAN, drm, 0);
    av_buffer_unref(&drm);
    if (created < 0) {
        av_buffer_unref(&vulkan);
        return NULL;
    }
    return vulkan;
#else
    (void)a;
    return NULL;
#endif
}

int serein_video_bind_adapter(AVCodecContext *codec, int backend, const SereinVideoAdapter *a)
{
    if (!codec || !a || !serein_video_adapter_valid(a))
        return 0;
    if (backend == 1) {
        const int device = serein_video_cuda_device(a);
        return device >= 0 && av_opt_set_int(codec->priv_data, "gpu", device, 0) >= 0;
    }
    if (backend == 2) {
#if defined(__APPLE__)
        return a->identity == SEREIN_GPU_METAL_REGISTRY &&
               av_opt_set_int(codec->priv_data, "gpu_registry_id", (int64_t)a->value, 0) >= 0;
#else
        return 0;
#endif
    }
    AVBufferRef *child = NULL;
    char device[80];
    enum AVHWDeviceType type = AV_HWDEVICE_TYPE_NONE;
#if defined(_WIN32)
    if ((backend == 3 && a->vendor_id == 0x1002) || (backend == 4 && a->vendor_id == 0x8086)) {
        const int index = serein_video_dxgi_device(a);
        if (index < 0)
            return 0;
        snprintf(device, sizeof(device), "%d", index);
        type = AV_HWDEVICE_TYPE_D3D11VA;
    }
#elif defined(__linux__)
    if (backend == 3 && a->vendor_id == 0x1002) {
        codec->hw_device_ctx = (AVBufferRef *)serein_video_vulkan_device(a);
        return codec->hw_device_ctx != NULL;
    }
    if (backend == 4 && a->vendor_id == 0x8086 && serein_video_drm_device(a, device, sizeof(device)))
        type = AV_HWDEVICE_TYPE_VAAPI;
#endif
    if (type == AV_HWDEVICE_TYPE_NONE || av_hwdevice_ctx_create(&child, type, device, NULL, 0) < 0)
        return 0;
#if defined(_WIN32)
    /* Recheck the created device: adapter enumeration can reorder when an
     * external GPU is removed between ordinal lookup and context creation. */
    if (!d3d11_matches(child, a)) {
        av_buffer_unref(&child);
        return 0;
    }
#endif
    int result;
    if (backend == 4) {
        result = av_hwdevice_ctx_create_derived(&codec->hw_device_ctx, AV_HWDEVICE_TYPE_QSV, child, 0) >= 0;
        av_buffer_unref(&child);
    } else {
        codec->hw_device_ctx = child;
        result = 1;
    }
    return result;
}
