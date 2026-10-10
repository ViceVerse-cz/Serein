#include "video_gpu.h"
/* AMD driver capability discovery. This creates a device context and reads
 * AMF caps, but never initializes an encoder or submits pictures. */
#if (defined(_WIN32) || defined(__linux__)) && __has_include(<AMF/core/Factory.h>)

#if defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <d3d11.h>
#include <dxgi.h>
#else
#include <cstring>
#include <dlfcn.h>
#endif

#if defined(__GNUC__) || defined(__clang__)
/* The pinned SDK contains MSVC pragmas and intentionally hides observer
 * overloads. Keep those third-party diagnostics out of the native build. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunknown-pragmas"
#pragma GCC diagnostic ignored "-Woverloaded-virtual"
#endif
#include <AMF/core/Factory.h>
#include <AMF/components/VideoEncoderVCE.h>
#include <AMF/components/VideoEncoderHEVC.h>
#include <AMF/components/VideoEncoderAV1.h>
#if defined(__linux__) && defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
#include <AMF/core/VulkanAMF.h>
extern "C" {
#include <libavutil/buffer.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
}
#endif
#if defined(__GNUC__) || defined(__clang__)
#pragma GCC diagnostic pop
#endif

namespace {

class Library {
public:
#if defined(_WIN32)
    explicit Library(const wchar_t *name)
        : handle_(LoadLibraryExW(name, nullptr, LOAD_LIBRARY_SEARCH_SYSTEM32)) {
        if (!handle_) {
            const DWORD error = GetLastError();
            if (error == ERROR_MOD_NOT_FOUND || error == ERROR_FILE_NOT_FOUND)
                failure_ = 0;
        }
    }
    ~Library() { if (handle_) FreeLibrary(handle_); }
    FARPROC symbol(const char *name) const {
        return handle_ ? GetProcAddress(handle_, name) : nullptr;
    }
#else
    explicit Library(const char *name) : handle_(dlopen(name, RTLD_NOW | RTLD_LOCAL)) {
        if (!handle_) {
            const char *error = dlerror();
            if (error && std::strstr(error, "No such file or directory"))
                failure_ = 0;
        }
    }
    ~Library() { if (handle_) dlclose(handle_); }
    void *symbol(const char *name) const {
        return handle_ ? dlsym(handle_, name) : nullptr;
    }
#endif
    bool loaded() const { return handle_ != nullptr; }
    int failure_result() const { return failure_; }
    Library(const Library &) = delete;
    Library &operator=(const Library &) = delete;
private:
#if defined(_WIN32)
    HMODULE handle_;
#else
    void *handle_;
#endif
    int failure_ = -1;
};

/* AMF's factory belongs to the loaded runtime and has no Release method.
 * Every refcounted interface must be released before unloading that runtime. */
class Context {
public:
    amf::AMFContextPtr value;
    ~Context() { if (value) value->Terminate(); }
};

bool unavailable(AMF_RESULT result) {
    return result == AMF_NOT_SUPPORTED || result == AMF_NO_DEVICE ||
           result == AMF_CODEC_NOT_SUPPORTED || result == AMF_ENCODER_NOT_PRESENT ||
           result == AMF_NOT_FOUND;
}

int query_caps(amf::AMFFactory *factory, amf::AMFContext *context, int codec) {
    static const wchar_t *const components[] = {
        AMFVideoEncoderVCE_AVC, AMFVideoEncoder_HEVC, AMFVideoEncoder_AV1
    };
    amf::AMFComponentPtr component;
    AMF_RESULT result = factory->CreateComponent(context, components[codec], &component);
    if (result != AMF_OK)
        return unavailable(result) ? 0 : -1;
    if (!component)
        return -1;

    amf::AMFCapsPtr caps;
    result = component->GetCaps(&caps);
    if (result != AMF_OK || !caps)
        return -1;
    switch (caps->GetAccelerationType()) {
    case amf::AMF_ACCEL_NOT_SUPPORTED:
    case amf::AMF_ACCEL_GPU:
    case amf::AMF_ACCEL_SOFTWARE:
        return 0;
    case amf::AMF_ACCEL_HARDWARE:
        break;
    default:
        return -1;
    }

    /* Read advertised 8-bit 4:2:0 input support. This is a capability hint;
     * the optional encode test separately checks FFmpeg's YUV420P path. */
    amf::AMFIOCapsPtr input;
    if (caps->GetInputCaps(&input) != AMF_OK || !input)
        return -1;
    const amf_int32 count = input->GetNumOfFormats();
    if (count < 0 || count > 64)
        return -1;
    for (amf_int32 index = 0; index < count; ++index) {
        amf::AMF_SURFACE_FORMAT format = amf::AMF_SURFACE_UNKNOWN;
        amf_bool native = false;
        if (input->GetFormatAt(index, &format, &native) != AMF_OK)
            return -1;
        if (format == amf::AMF_SURFACE_NV12 || format == amf::AMF_SURFACE_YUV420P)
            return 1;
    }
    return 0;
}

#if defined(_WIN32)
template<class T> class ComPtr {
public:
    T *value = nullptr;
    ~ComPtr() { if (value) value->Release(); }
    ComPtr() = default;
    ComPtr(const ComPtr &) = delete;
    ComPtr &operator=(const ComPtr &) = delete;
};

int query_devices(amf::AMFFactory *factory, int codec, const SereinVideoAdapter *target) {
    Library dxgi(L"dxgi.dll");
    Library d3d11(L"d3d11.dll");
    const auto create_factory = reinterpret_cast<HRESULT (WINAPI *)(REFIID, void **)>(
        dxgi.symbol("CreateDXGIFactory1"));
    const auto create_device = reinterpret_cast<decltype(&D3D11CreateDevice)>(
        d3d11.symbol("D3D11CreateDevice"));
    if (!create_factory || !create_device)
        return -1;
    ComPtr<IDXGIFactory1> adapters;
    const IID iid_factory = {0x770aae78, 0xf26f, 0x4dba,
                            {0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87}};
    if (FAILED(create_factory(iid_factory, reinterpret_cast<void **>(&adapters.value))) ||
        !adapters.value)
        return -1;

    bool uncertain = false;
    /* Bound adapter enumeration even if the driver never returns NOT_FOUND. */
    for (UINT index = 0; index < 16; ++index) {
        ComPtr<IDXGIAdapter1> adapter;
        const HRESULT found = adapters.value->EnumAdapters1(index, &adapter.value);
        if (found == DXGI_ERROR_NOT_FOUND)
            return uncertain ? -1 : 0;
        if (FAILED(found) || !adapter.value)
            return -1;
        DXGI_ADAPTER_DESC1 description = {};
        if (FAILED(adapter.value->GetDesc1(&description))) {
            uncertain = true;
            continue;
        }
        if (description.VendorId != 0x1002 || (description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE))
            continue;
        if (target) {
            uint64_t luid = 0;
            memcpy(&luid, &description.AdapterLuid, sizeof(luid));
            if (target->identity != SEREIN_GPU_WINDOWS_LUID || luid != target->value ||
                description.VendorId != target->vendor_id || description.DeviceId != target->device_id)
                continue;
        }

        ComPtr<ID3D11Device> device;
        if (FAILED(create_device(adapter.value, D3D_DRIVER_TYPE_UNKNOWN, nullptr, 0,
                                 nullptr, 0, D3D11_SDK_VERSION, &device.value, nullptr, nullptr)) ||
            !device.value) {
            uncertain = true;
            continue;
        }
        Context context;
        if (factory->CreateContext(&context.value) != AMF_OK || !context.value) {
            uncertain = true;
            continue;
        }
        const AMF_RESULT initialized = context.value->InitDX11(device.value, amf::AMF_DX11_1);
        if (initialized != AMF_OK) {
            uncertain |= !unavailable(initialized);
            continue;
        }
        const int result = query_caps(factory, context.value, codec);
        if (result == 1 || target)
            return result;
        uncertain |= result < 0;
    }
    return -1;
}
#else
int query_devices(amf::AMFFactory *factory, int codec, const SereinVideoAdapter *target) {
#if defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
    class Device {
    public:
        AVBufferRef *value = nullptr;
        ~Device() { av_buffer_unref(&value); }
    } device;
    amf::AMFVulkanDevice selected = {};
    if (target) {
        device.value = static_cast<AVBufferRef *>(serein_video_vulkan_device(target));
        if (!device.value)
            return -1;
        const AVHWDeviceContext *gpu = reinterpret_cast<const AVHWDeviceContext *>(device.value->data);
        if (gpu->type != AV_HWDEVICE_TYPE_VULKAN || !gpu->hwctx)
            return -1;
        const AVVulkanDeviceContext *native = static_cast<const AVVulkanDeviceContext *>(gpu->hwctx);
        selected.cbSizeof = sizeof(selected);
        selected.hInstance = native->inst;
        selected.hPhysicalDevice = native->phys_dev;
        selected.hDevice = native->act_dev;
        if (!selected.hInstance || !selected.hPhysicalDevice || !selected.hDevice)
            return -1;
    }
#else
    if (target)
        return -1;
#endif
    Context context;
    if (factory->CreateContext(&context.value) != AMF_OK || !context.value)
        return -1;
    amf::AMFContext1Ptr vulkan(context.value);
    if (!vulkan)
        return -1;
    /* FFmpeg's Linux AMF encoder uses this same runtime-selected Vulkan
     * device. AMF owns its device; no Vulkan header or link dependency needed. */
    const AMF_RESULT initialized = vulkan->InitVulkan(
#if defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
        target ? &selected : nullptr
#else
        nullptr
#endif
    );
    if (initialized != AMF_OK)
        return unavailable(initialized) ? 0 : -1;
    return query_caps(factory, context.value, codec);
}
#endif

} // namespace

static int query_amf(int codec, const SereinVideoAdapter *target) {
    if (codec < 0 || codec > 2)
        return -1;
#if defined(_WIN32)
    Library runtime(AMF_DLL_NAME);
#else
    Library runtime(AMF_DLL_NAMEA);
#endif
    if (!runtime.loaded())
        return runtime.failure_result();
    const auto initialize = reinterpret_cast<AMFInit_Fn>(runtime.symbol(AMF_INIT_FUNCTION_NAME));
    if (!initialize)
        return -1;
    amf::AMFFactory *factory = nullptr;
    if (initialize(AMF_FULL_VERSION, &factory) != AMF_OK || !factory)
        return -1;
    return query_devices(factory, codec, target);
}
extern "C" int serein_query_amf(int codec) { return query_amf(codec, nullptr); }
extern "C" int serein_query_amf_on_adapter(int codec, const SereinVideoAdapter *target) {
    if (!serein_video_adapter_valid(target) || target->vendor_id != 0x1002)
        return -1;
    return query_amf(codec, target);
}

#else
/* A distribution without the SDK can still build software/other vendors.
 * The common dispatcher separately rejects absent FFmpeg encoder modules. */
extern "C" int serein_query_amf(int codec) {
    (void)codec;
#if defined(_WIN32) || defined(__linux__)
    return -1;
#else
    return 0;
#endif
}
extern "C" int serein_query_amf_on_adapter(int codec, const SereinVideoAdapter *target) {
    (void)codec; (void)target;
    return -1;
}
#endif
