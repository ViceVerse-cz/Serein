#include "video_encode_ffmpeg.h"
#include <cstdio>
#include <cstring>
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
        if (!handle_) failure_ = -2;
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

int failed(const char *stage, AMF_RESULT result) {
    std::fprintf(stderr, "[NuSerein] AMD AMF: %s failed (AMF result=%d).\n", stage, result);
    return -1;
}

amf::AMFFactory *load_factory(const Library &runtime) {
    const auto initialize = reinterpret_cast<AMFInit_Fn>(runtime.symbol(AMF_INIT_FUNCTION_NAME));
    if (!initialize) {
        failed("runtime entry point", AMF_NOT_FOUND);
        return nullptr;
    }
    amf::AMFFactory *factory = nullptr;
    const AMF_RESULT result = initialize(AMF_FULL_VERSION, &factory);
    if (result != AMF_OK || !factory) {
        failed("runtime initialization", result);
        return nullptr;
    }
    const auto version_query = reinterpret_cast<AMFQueryVersion_Fn>(
        runtime.symbol(AMF_QUERY_VERSION_FUNCTION_NAME));
    amf_uint64 version = 0;
    if (version_query && version_query(&version) == AMF_OK)
        std::fprintf(stderr, "[NuSerein] AMD AMF runtime %u.%u.%u.%u.\n",
            static_cast<unsigned>(AMF_GET_MAJOR_VERSION(version)),
            static_cast<unsigned>(AMF_GET_MINOR_VERSION(version)),
            static_cast<unsigned>(AMF_GET_SUBMINOR_VERSION(version)),
            static_cast<unsigned>(AMF_GET_BUILD_VERSION(version)));
    return factory;
}

#if defined(__linux__) && defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
bool device_extensions(amf::AMFContext1 *context, char *names, size_t capacity) {
    size_t written = 0;
    if (!names || !capacity) return false;
    names[0] = '\0';
    amf_size count = 0;
    AMF_RESULT result = context->GetVulkanDeviceExtensions(&count, nullptr);
    if (result != AMF_OK || count > 64) {
        failed("Vulkan extension count", result);
        return false;
    }
    const char *extensions[64] = {};
    if (count) {
        const amf_size capacity = count;
        result = context->GetVulkanDeviceExtensions(&count, extensions);
        if (result != AMF_OK || count > capacity) {
            failed("Vulkan extension list", result);
            return false;
        }
    }
    for (amf_size i = 0; i < count; ++i) {
        const char *name = extensions[i];
        if (!name) {
            failed("Vulkan extension name", AMF_INVALID_ARG);
            return false;
        }
        size_t length = 0;
        for (; length < 256 && name[length]; ++length) {
            const char c = name[length];
            if (!(c == '_' || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
                  (c >= '0' && c <= '9'))) {
                failed("Vulkan extension name", AMF_INVALID_ARG);
                return false;
            }
        }
        if (length < 4 || length >= 256 || std::strncmp(name, "VK_", 3)) {
            failed("Vulkan extension name", AMF_INVALID_ARG);
            return false;
        }
        const size_t separator = written != 0;
        if (length + separator >= capacity - written) {
            failed("Vulkan extension list size", AMF_INVALID_ARG);
            return false;
        }
        if (separator) names[written++] = '+';
        std::memcpy(names + written, name, length);
        written += length;
        names[written] = '\0';
    }
    return true;
}
#endif

bool unavailable(AMF_RESULT result) {
    return result == AMF_NOT_SUPPORTED || result == AMF_NO_DEVICE ||
           result == AMF_CODEC_NOT_SUPPORTED || result == AMF_ENCODER_NOT_PRESENT ||
           result == AMF_NOT_FOUND;
}

int query_caps(amf::AMFFactory *factory, amf::AMFContext *context, int codec) {
    const bool presets = codec >= 6;
    const bool ten_bit = codec >= 3 && codec < 6; codec %= 3;
    if (ten_bit && codec == 0) return 0;
    static const wchar_t *const components[] = {
        AMFVideoEncoderVCE_AVC, AMFVideoEncoder_HEVC, AMFVideoEncoder_AV1
    };
    amf::AMFComponentPtr component;
    AMF_RESULT result = factory->CreateComponent(context, components[codec], &component);
    if (result != AMF_OK)
        return unavailable(result) ? 0 : failed("codec creation", result);
    if (!component)
        return -1;

    amf::AMFCapsPtr caps;
    result = component->GetCaps(&caps);
    if (result != AMF_OK || !caps)
        return failed("codec capability query", result);
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

    if (presets) {
        static const wchar_t *const properties[] = {AMF_VIDEO_ENCODER_QUALITY_PRESET,
            AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET, AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET};
        static const amf_int64 values[3][4] = {
            {AMF_VIDEO_ENCODER_QUALITY_PRESET_SPEED, AMF_VIDEO_ENCODER_QUALITY_PRESET_BALANCED,
             AMF_VIDEO_ENCODER_QUALITY_PRESET_QUALITY, AMF_VIDEO_ENCODER_QUALITY_PRESET_HIGH_QUALITY},
            {AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_SPEED, AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_BALANCED,
             AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_QUALITY, AMF_VIDEO_ENCODER_HEVC_QUALITY_PRESET_HIGH_QUALITY},
            {AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_SPEED, AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_BALANCED,
             AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_QUALITY, AMF_VIDEO_ENCODER_AV1_QUALITY_PRESET_HIGH_QUALITY}};
        const amf::AMFPropertyInfo *info = nullptr;
        if (component->GetPropertyInfo(properties[codec], &info) != AMF_OK || !info ||
            info->type != amf::AMF_VARIANT_INT64 || !info->AllowedWrite() || !info->pEnumDescription)
            return -1;
        int supported = 0;
        for (unsigned int i = 0; i < 32; ++i) {
            const auto &entry = info->pEnumDescription[i];
            if (!entry.name) return supported;
            for (unsigned int preset = 0; preset < 4; ++preset)
                if (entry.value == values[codec][preset] &&
                    component->SetProperty(properties[codec], amf::AMFVariant(values[codec][preset])) == AMF_OK)
                    supported |= 1 << (preset + 8);
        }
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
        if (ten_bit) {
            if (format != amf::AMF_SURFACE_P010) continue;
            if (codec == 1) {
                amf::AMFVariant profile;
                if (caps->GetProperty(AMF_VIDEO_ENCODER_HEVC_CAP_MAX_PROFILE, &profile) != AMF_OK || profile.type != amf::AMF_VARIANT_INT64) return -1;
                if (profile.int64Value < AMF_VIDEO_ENCODER_HEVC_PROFILE_MAIN_10) return 0;
            }
            return 1;
        }
        if (format == amf::AMF_SURFACE_NV12 || format == amf::AMF_SURFACE_YUV420P) return 1;
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
        if (result > 0 || target)
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
#endif
    Context context;
    AMF_RESULT result = factory->CreateContext(&context.value);
    if (result != AMF_OK || !context.value)
        return failed("context creation", result);
    amf::AMFContext1Ptr vulkan(context.value);
    if (!vulkan)
        return failed("Vulkan context interface", AMF_NO_INTERFACE);
#if defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
    amf::AMFVulkanDevice selected = {};
    if (target) {
        char extensions[16384];
        if (!device_extensions(vulkan, extensions, sizeof(extensions)))
            return -1;
        device.value = static_cast<AVBufferRef *>(serein_video_vulkan_device(target, extensions));
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
    /* The selected device remains alive until the AMF context terminates. */
    const AMF_RESULT initialized = vulkan->InitVulkan(
#if defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
        target ? &selected : nullptr
#else
        nullptr
#endif
    );
    if (initialized != AMF_OK) {
        failed("Vulkan initialization", initialized);
        return unavailable(initialized) ? 0 : -1;
    }
    return query_caps(factory, context.value, codec);
}
#endif

} // namespace

static int query_amf(int codec, const SereinVideoAdapter *target) {
    if (codec < 0 || codec > 8)
        return -1;
#if defined(_WIN32)
    Library runtime(AMF_DLL_NAME);
#else
    Library runtime(AMF_DLL_NAMEA);
#endif
    if (!runtime.loaded()) {
        std::fprintf(stderr, "[NuSerein] AMD AMF: the runtime could not be loaded.\n");
        return runtime.failure_result();
    }
    amf::AMFFactory *factory = load_factory(runtime);
    if (!factory) return -1;
    return query_devices(factory, codec, target);
}

extern "C" void *serein_video_amf_device(const SereinVideoAdapter *target) {
#if defined(__linux__) && defined(SEREIN_HAVE_VULKAN_GPU) && SEREIN_HAVE_VULKAN_GPU
    if (!serein_video_adapter_valid(target) || target->vendor_id != 0x1002)
        return nullptr;
    Library runtime(AMF_DLL_NAMEA);
    if (!runtime.loaded()) {
        std::fprintf(stderr, "[NuSerein] AMD AMF: the runtime could not be loaded.\n");
        return nullptr;
    }
    amf::AMFFactory *factory = load_factory(runtime);
    if (!factory) return nullptr;
    Context context;
    const AMF_RESULT result = factory->CreateContext(&context.value);
    if (result != AMF_OK || !context.value) {
        failed("context creation", result);
        return nullptr;
    }
    amf::AMFContext1Ptr vulkan(context.value);
    if (!vulkan) return nullptr;
    char extensions[16384];
    if (!device_extensions(vulkan, extensions, sizeof(extensions))) return nullptr;
    return serein_video_vulkan_device(target, extensions);
#else
    (void)target;
    return nullptr;
#endif
}
extern "C" int serein_query_amf(int codec) { return codec < 0 || codec > 2 ? -1 : query_amf(codec, nullptr); }
extern "C" int serein_query_amf_on_adapter(int codec, const SereinVideoAdapter *target) {
    if (codec == 3) return 0; /* H.264 stays the compatible SDR path. */
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
extern "C" void *serein_video_amf_device(const SereinVideoAdapter *target) {
    (void)target;
    return nullptr;
}
#endif
