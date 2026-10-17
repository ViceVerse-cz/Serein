/* Vendor sections keep their original platform/SDK guards. Fixtures may
 * compile one section; the shipping build compiles all sections. */
#ifndef SEREIN_QUERY_PART
#define SEREIN_QUERY_PART 0
#endif

#if SEREIN_QUERY_PART == 0 || SEREIN_QUERY_PART == 1
/* Codec discovery only: CUDA context + NVENC capability session, never an
 * initialized encoder, input surface, bitstream buffer or submitted picture.
 * The pinned nv-codec-headers SDK also supplies FFmpeg's CUDA/NVENC ABI.
 * Query every bounded CUDA adapter because FFmpeg defaults to gpu=any; a
 * positive result describes an advertised codec, not our streaming presets. */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "video_encode_ffmpeg.h"

#if defined(SEREIN_HAVE_NVENC_QUERY) && SEREIN_HAVE_NVENC_QUERY && \
    (defined(_WIN32) || defined(__linux__))

#include <ffnvcodec/dynlink_cuda.h>
#include <ffnvcodec/nvEncodeAPI.h>

#if defined(_WIN32)
typedef HMODULE SereinNvLibrary;

static SereinNvLibrary load_library(const wchar_t *name, int *result)
{
    HMODULE library = LoadLibraryExW(name, NULL, LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!library) {
        DWORD error = GetLastError();
        *result = (error == ERROR_MOD_NOT_FOUND || error == ERROR_FILE_NOT_FOUND) ? 0 : -1;
    }
    return library;
}

static int load_symbol(SereinNvLibrary library, const char *name, void *destination, size_t size)
{
    FARPROC symbol = GetProcAddress(library, name);
    if (!symbol || size != sizeof(symbol))
        return 0;
    memcpy(destination, &symbol, size);
    return 1;
}

static void unload_library(SereinNvLibrary library)
{
    if (library)
        FreeLibrary(library);
}

#else
#include <dlfcn.h>
typedef void *SereinNvLibrary;

static SereinNvLibrary load_library(const char *name, int *result)
{
    void *library = dlopen(name, RTLD_NOW | RTLD_LOCAL);
    if (!library) {
        const char *error = dlerror();
        /* A missing runtime (including one of its dependencies) cannot make
         * this FFmpeg path available. Loader/ABI errors stay inconclusive. */
        *result = error && strstr(error, "No such file or directory") ? 0 : -1;
    }
    return library;
}

static int load_symbol(SereinNvLibrary library, const char *name, void *destination, size_t size)
{
    void *symbol = dlsym(library, name);
    if (!symbol || size != sizeof(symbol))
        return 0;
    memcpy(destination, &symbol, size);
    return 1;
}

static void unload_library(SereinNvLibrary library)
{
    if (library)
        dlclose(library);
}
#endif

typedef NVENCSTATUS (NVENCAPI *SereinNvCreate)(NV_ENCODE_API_FUNCTION_LIST *functions);
typedef NVENCSTATUS (NVENCAPI *SereinNvVersion)(uint32_t *version);

typedef struct {
    tcuInit *init;
    tcuDeviceGetCount *device_count;
    tcuDeviceGet *device_get;
    tcuCtxCreate_v2 *context_create;
    tcuCtxPopCurrent_v2 *context_pop;
    tcuCtxDestroy_v2 *context_destroy;
} SereinNvCuda;

#define SEREIN_NV_LOAD(library, member, symbol) \
    load_symbol((library), (symbol), &(member), sizeof(member))
#define SEREIN_NV_MAX_ADAPTERS 32
#define SEREIN_NV_MAX_CODEC_GUIDS 64

static int query_adapter(const SereinNvCuda *cuda,
                         NV_ENCODE_API_FUNCTION_LIST *api, int index, GUID codec,
                         int *max_b_frames, int *lookahead, int ten_bit)
{
    CUdevice device;
    CUcontext context = NULL;
    CUcontext popped = NULL;
    void *encoder = NULL;
    NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS session = {0};
    NV_ENC_CAPS_PARAM caps = {0};
    GUID guids[SEREIN_NV_MAX_CODEC_GUIDS];
    uint32_t count = 0, written = 0;
    int result = -1, width = 0, height = 0;
    NVENCSTATUS status;

    if (cuda->device_get(&device, index) != CUDA_SUCCESS ||
        cuda->context_create(&context, 0, device) != CUDA_SUCCESS)
        return -1;

    /* cuCtxCreate pushes its context. Keep it current for the lightweight
     * capability session, then pop it before destruction to restore callers. */
    session.version = NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS_VER;
    session.deviceType = NV_ENC_DEVICE_TYPE_CUDA;
    session.device = context;
    session.apiVersion = NVENCAPI_VERSION;
    status = api->nvEncOpenEncodeSessionEx(&session, &encoder);
    if (status != NV_ENC_SUCCESS) {
        encoder = NULL;
        if (status == NV_ENC_ERR_NO_ENCODE_DEVICE || status == NV_ENC_ERR_UNSUPPORTED_DEVICE)
            result = 0;
        goto cleanup;
    }
    if (!encoder || api->nvEncGetEncodeGUIDCount(encoder, &count) != NV_ENC_SUCCESS)
        goto cleanup;
    if (count == 0) {
        result = 0;
        goto cleanup;
    }
    if (count > SEREIN_NV_MAX_CODEC_GUIDS ||
        api->nvEncGetEncodeGUIDs(encoder, guids, count, &written) != NV_ENC_SUCCESS ||
        written != count)
        goto cleanup;
    result = 0;
    for (uint32_t i = 0; i < written; i++) {
        if (memcmp(&guids[i], &codec, sizeof(codec)) != 0)
            continue;
        caps.version = NV_ENC_CAPS_PARAM_VER;
        caps.capsToQuery = NV_ENC_CAPS_WIDTH_MAX;
        if (api->nvEncGetEncodeCaps(encoder, codec, &caps, &width) != NV_ENC_SUCCESS) {
            result = -1;
            break;
        }
        caps.capsToQuery = NV_ENC_CAPS_HEIGHT_MAX;
        if (api->nvEncGetEncodeCaps(encoder, codec, &caps, &height) != NV_ENC_SUCCESS) {
            result = -1;
            break;
        }
        result = width > 0 && height > 0 ? 1 : 0;
        if (result == 1 && ten_bit) {
            int supported = 0;
            caps.capsToQuery = NV_ENC_CAPS_SUPPORT_10BIT_ENCODE;
            result = api->nvEncGetEncodeCaps(encoder, codec, &caps, &supported) != NV_ENC_SUCCESS ? -1 : supported ? 1 : 0;
        }
        if (result == 1 && max_b_frames && lookahead) {
            caps.capsToQuery = NV_ENC_CAPS_NUM_MAX_BFRAMES;
            if (api->nvEncGetEncodeCaps(encoder, codec, &caps, max_b_frames) != NV_ENC_SUCCESS) {
                result = -1;
                break;
            }
            caps.capsToQuery = NV_ENC_CAPS_SUPPORT_LOOKAHEAD;
            if (api->nvEncGetEncodeCaps(encoder, codec, &caps, lookahead) != NV_ENC_SUCCESS)
                result = -1;
        }
        break;
    }

cleanup:
    if (encoder && api->nvEncDestroyEncoder(encoder) != NV_ENC_SUCCESS)
        result = -1;
    if (cuda->context_pop(&popped) != CUDA_SUCCESS || popped != context)
        result = -1;
    if (cuda->context_destroy(context) != CUDA_SUCCESS)
        result = -1;
    return result;
}

static int query_nvenc(int codec, const SereinVideoAdapter *target,
                       int *max_b_frames, int *lookahead)
{
    static const GUID *const codecs[] = {
        &NV_ENC_CODEC_H264_GUID, &NV_ENC_CODEC_HEVC_GUID, &NV_ENC_CODEC_AV1_GUID
    };
    SereinNvLibrary cuda_library = NULL, nv_library = NULL;
    SereinNvCuda cuda = {0};
    SereinNvCreate create = NULL;
    SereinNvVersion get_version = NULL;
    NV_ENCODE_API_FUNCTION_LIST api = {0};
    uint32_t version = 0;
    int result = -1, adapters = 0, incomplete = 0;
    CUresult cuda_status;
    const int ten_bit = codec >= 3;
    if (codec < 0 || codec > 5)
        return 0;
    codec %= 3;
    if (ten_bit && codec == 0) return 0;
    const int selected = target ? serein_video_cuda_device(target) : -1;
    if (target && selected < 0)
        return -1;
#if defined(_WIN32)
    cuda_library = load_library(L"nvcuda.dll", &result);
#if defined(_WIN64)
    if (cuda_library)
        nv_library = load_library(L"nvEncodeAPI64.dll", &result);
#else
    if (cuda_library)
        nv_library = load_library(L"nvEncodeAPI.dll", &result);
#endif
#else
    cuda_library = load_library("libcuda.so.1", &result);
    if (cuda_library)
        nv_library = load_library("libnvidia-encode.so.1", &result);
#endif
    if (!cuda_library || !nv_library)
        goto cleanup;
    result = -1;
    if (!SEREIN_NV_LOAD(cuda_library, cuda.init, "cuInit") ||
        !SEREIN_NV_LOAD(cuda_library, cuda.device_count, "cuDeviceGetCount") ||
        !SEREIN_NV_LOAD(cuda_library, cuda.device_get, "cuDeviceGet") ||
        !SEREIN_NV_LOAD(cuda_library, cuda.context_create, "cuCtxCreate_v2") ||
        !SEREIN_NV_LOAD(cuda_library, cuda.context_pop, "cuCtxPopCurrent_v2") ||
        !SEREIN_NV_LOAD(cuda_library, cuda.context_destroy, "cuCtxDestroy_v2") ||
        !SEREIN_NV_LOAD(nv_library, create, "NvEncodeAPICreateInstance") ||
        !SEREIN_NV_LOAD(nv_library, get_version, "NvEncodeAPIGetMaxSupportedVersion"))
        goto cleanup;
    if (get_version(&version) != NV_ENC_SUCCESS ||
        version < ((NVENCAPI_MAJOR_VERSION << 4) | NVENCAPI_MINOR_VERSION))
        goto cleanup;
    api.version = NV_ENCODE_API_FUNCTION_LIST_VER;
    if (create(&api) != NV_ENC_SUCCESS || !api.nvEncOpenEncodeSessionEx ||
        !api.nvEncGetEncodeGUIDCount || !api.nvEncGetEncodeGUIDs ||
        !api.nvEncGetEncodeCaps || !api.nvEncDestroyEncoder)
        goto cleanup;
    cuda_status = cuda.init(0);
    if (cuda_status != CUDA_SUCCESS) {
        /* CUDA_ERROR_NO_DEVICE is 100 in the CUDA driver ABI. The minimal
         * nv-codec-headers CUDA declarations do not name that enumerator. */
        if ((int)cuda_status == 100)
            result = 0;
        goto cleanup;
    }
    if (cuda.device_count(&adapters) != CUDA_SUCCESS || adapters < 0)
        goto cleanup;
    incomplete = adapters > SEREIN_NV_MAX_ADAPTERS;
    for (int i = 0; i < adapters && i < SEREIN_NV_MAX_ADAPTERS; i++) {
        if (target && i != selected)
            continue;
        int adapter = query_adapter(&cuda, &api, i, *codecs[codec], max_b_frames, lookahead, ten_bit);
        if (adapter == 1) {
            result = 1;
            goto cleanup;
        }
        if (adapter < 0)
            incomplete = 1;
    }
    result = incomplete ? -1 : 0;

cleanup:
    unload_library(nv_library);
    unload_library(cuda_library);
    return result;
}

int serein_query_nvenc(int codec) { return codec < 0 || codec > 2 ? 0 : query_nvenc(codec, NULL, NULL, NULL); }
int serein_query_nvenc_on_adapter(int codec, const SereinVideoAdapter *adapter)
{ return adapter ? query_nvenc(codec, adapter, NULL, NULL) : -1; }
int serein_nvenc_features(int codec, const SereinVideoAdapter *adapter,
                          int *max_b_frames, int *lookahead)
{
    if (!adapter || !max_b_frames || !lookahead)
        return -1;
    *max_b_frames = *lookahead = 0;
    return query_nvenc(codec, adapter, max_b_frames, lookahead);
}

#else
int serein_query_nvenc(int codec)
{
    (void)codec;
    /* The dispatcher has already checked that the FFmpeg encoder exists.
     * Missing query SDK support is inconclusive, not unsupported hardware. */
    return -1;
}
int serein_query_nvenc_on_adapter(int codec, const SereinVideoAdapter *adapter)
{ (void)codec; (void)adapter; return -1; }
int serein_nvenc_features(int codec, const SereinVideoAdapter *adapter,
                          int *max_b_frames, int *lookahead)
{
    (void)codec; (void)adapter;
    if (max_b_frames) *max_b_frames = 0;
    if (lookahead) *lookahead = 0;
    return -1;
}
#endif

#endif

#if SEREIN_QUERY_PART == 0 || SEREIN_QUERY_PART == 2
/* Query oneVPL hardware implementation descriptions without creating an
 * encoder, allocating input pictures or submitting synthetic frames.
 * The dispatcher is the same pinned oneVPL 2.x ABI used by bundled FFmpeg.
 * Legacy Media SDK compatibility descriptions omit Enc: that is Unknown,
 * not proof that an otherwise usable Intel GPU supports no video codecs. */
#include <stdint.h>
#include <string.h>
#include "video_encode_ffmpeg.h"

#if defined(SEREIN_HAVE_QSV_QUERY) && SEREIN_HAVE_QSV_QUERY && \
    (defined(_WIN32) || defined(__linux__))
#include <vpl/mfxdispatcher.h>
#include <vpl/mfxstructures.h>

#define SEREIN_QSV_MAX_IMPLEMENTATIONS 32
#define SEREIN_QSV_MAX_CODECS 64

static int set_filter(mfxLoader loader, const char *name, mfxU32 value)
{
    mfxConfig config = MFXCreateConfig(loader);
    mfxVariant property = {0};
    if (!config)
        return 0;
    property.Version.Version = MFX_VARIANT_VERSION;
    property.Type = MFX_VARIANT_TYPE_U32;
    property.Data.U32 = value;
    return MFXSetConfigFilterProperty(config, (const mfxU8 *)name, property) == MFX_ERR_NONE;
}

static int description_supports_codec(const mfxImplDescription *description, mfxU32 codec, int ten_bit)
{
    if (!description || description->Version.Major != 1 ||
        description->Impl != MFX_IMPL_TYPE_HARDWARE || description->VendorID != 0x8086)
        return -1;
    /* API 1.x compatibility descriptions are dispatcher-created metadata,
     * with no per-codec capabilities. Do not guess from the GPU generation. */
    if (description->ApiVersion.Major < 2 || description->Enc.Version.Major != 1 ||
        description->Enc.NumCodecs > SEREIN_QSV_MAX_CODECS ||
        (description->Enc.NumCodecs && !description->Enc.Codecs))
        return -1;
    for (mfxU16 i = 0; i < description->Enc.NumCodecs; i++) {
        if (description->Enc.Codecs[i].CodecID == codec) {
            if (!ten_bit) return 1;
            const mfxEncoderDescription *enc = &description->Enc;
            if (!enc->Codecs[i].NumProfiles || enc->Codecs[i].NumProfiles > 64 || !enc->Codecs[i].Profiles) return -1;
            for (mfxU16 j = 0; j < enc->Codecs[i].NumProfiles; j++) {
                if (codec == MFX_CODEC_HEVC && enc->Codecs[i].Profiles[j].Profile != MFX_PROFILE_HEVC_MAIN10) continue;
                if (codec == MFX_CODEC_AV1 && enc->Codecs[i].Profiles[j].Profile != MFX_PROFILE_AV1_MAIN) continue;
                if (enc->Codecs[i].Profiles[j].NumMemTypes > 16 || (enc->Codecs[i].Profiles[j].NumMemTypes && !enc->Codecs[i].Profiles[j].MemDesc)) return -1;
                for (mfxU16 k = 0; k < enc->Codecs[i].Profiles[j].NumMemTypes; k++) {
                    if (enc->Codecs[i].Profiles[j].MemDesc[k].NumColorFormats > 64 || (enc->Codecs[i].Profiles[j].MemDesc[k].NumColorFormats && !enc->Codecs[i].Profiles[j].MemDesc[k].ColorFormats)) return -1;
                    for (mfxU16 n = 0; n < enc->Codecs[i].Profiles[j].MemDesc[k].NumColorFormats; n++)
                        if (enc->Codecs[i].Profiles[j].MemDesc[k].ColorFormats[n] == MFX_FOURCC_P010) return 1;
                }
            }
            return 0;
        }
    }
    return 0;
}

static int implementation_matches(mfxLoader loader, mfxU32 index, const SereinVideoAdapter *target)
{
    mfxHDL handle = NULL;
    const mfxStatus status = MFXEnumImplementations(loader, index, MFX_IMPLCAPS_DEVICE_ID_EXTENDED, &handle);
    int result = -1;
    if (status == MFX_ERR_NONE && handle) {
        const mfxExtendedDeviceId *device = (const mfxExtendedDeviceId *)handle;
        if (device->Version.Major == 1) {
            result = 0;
            if (device->VendorID == target->vendor_id && device->DeviceID == target->device_id) {
                if (target->identity == SEREIN_GPU_PCI)
                    result = device->PCIDomain == target->domain && device->PCIBus == target->bus &&
                             device->PCIDevice == target->slot && device->PCIFunction == target->function;
                else if (target->identity == SEREIN_GPU_WINDOWS_LUID) {
                    uint64_t luid = 0;
                    memcpy(&luid, device->DeviceLUID, sizeof(luid));
                    result = device->LUIDValid && luid == target->value;
                }
            }
        }
    }
    if (handle && MFXDispReleaseImplDescription(loader, handle) != MFX_ERR_NONE)
        result = -1;
    return result;
}

static int query_qsv(int codec, const SereinVideoAdapter *target)
{
    static const mfxU32 codecs[] = {MFX_CODEC_AVC, MFX_CODEC_HEVC, MFX_CODEC_AV1};
    mfxLoader loader;
    int result = -1, incomplete = 0;
    mfxU32 acceleration;
    const int ten_bit = codec >= 3;
    if (codec < 0 || codec > 5)
        return 0;
    codec %= 3;
    if (ten_bit && codec == 0) return 0;
    loader = MFXLoad();
    if (!loader)
        return -1;
#if defined(_WIN32)
    acceleration = MFX_ACCEL_MODE_VIA_D3D11;
#else
    /* Linux QSV uses Intel's VA driver interface, just as FFmpeg's selected
     * child_device_type=vaapi does. This does not enable an FFmpeg VA encoder. */
    acceleration = MFX_ACCEL_MODE_VIA_VAAPI;
#endif
    if (!set_filter(loader, "mfxImplDescription.Impl", MFX_IMPL_TYPE_HARDWARE) ||
        !set_filter(loader, "mfxImplDescription.VendorID", 0x8086) ||
        !set_filter(loader, "mfxImplDescription.AccelerationMode", acceleration))
        goto cleanup;
    for (mfxU32 i = 0; i < SEREIN_QSV_MAX_IMPLEMENTATIONS; i++) {
        mfxHDL handle = NULL;
        mfxStatus status = MFXEnumImplementations(loader, i,
            MFX_IMPLCAPS_IMPLDESCSTRUCTURE, &handle);
        if (status == MFX_ERR_NOT_FOUND) {
            result = incomplete ? -1 : 0;
            goto cleanup;
        }
        if (status != MFX_ERR_NONE || !handle) {
            /* Even error-producing implementations count towards the bound.
             * Continue so another Intel adapter can advertise the codec. */
            incomplete = 1;
            if (handle)
                MFXDispReleaseImplDescription(loader, handle);
            continue;
        }
        const int matching = target ? implementation_matches(loader, i, target) : 1;
        int supported = matching == 1 ? description_supports_codec((const mfxImplDescription *)handle, codecs[codec], ten_bit) : matching;
        if (MFXDispReleaseImplDescription(loader, handle) != MFX_ERR_NONE)
            supported = -1;
        if (supported == 1) {
            result = 1;
            goto cleanup;
        }
        if (supported < 0)
            incomplete = 1;
    }
    /* The enumeration bound prevents an unbounded scan, but prevents us
     * claiming absence when additional implementations may remain. */

cleanup:
    MFXUnload(loader);
    return result;
}

int serein_query_qsv(int codec) { return codec < 0 || codec > 2 ? 0 : query_qsv(codec, NULL); }
int serein_query_qsv_on_adapter(int codec, const SereinVideoAdapter *adapter)
{
    if (!serein_video_adapter_valid(adapter) || adapter->vendor_id != 0x8086)
        return -1;
    return query_qsv(codec, adapter);
}

#else
int serein_query_qsv(int codec)
{
    (void)codec;
    /* An installed FFmpeg encoder without query headers is not a negative
     * driver capability report. */
    return -1;
}
int serein_query_qsv_on_adapter(int codec, const SereinVideoAdapter *adapter)
{ (void)codec; (void)adapter; return -1; }
#endif

#endif

#if SEREIN_QUERY_PART == 0 || SEREIN_QUERY_PART == 3
#include "video_encode_ffmpeg.h"
/* VideoToolbox's public encoder inventory, without creating an encoder or
 * submitting pictures. This is hardware codec discovery, not a guarantee that
 * a particular resolution/profile or a busy device can encode right now.
 */
#if defined(__APPLE__)
#include <CoreFoundation/CoreFoundation.h>
#include <CoreMedia/CoreMedia.h>
#include <VideoToolbox/VideoToolbox.h>
#include <stdint.h>
#include <dlfcn.h>

#define SEREIN_MAX_VT_ENCODERS 512

static int query_videotoolbox(int codec, const SereinVideoAdapter *target)
{
    if (codec < 0 || codec > 5) return -1;
    const int ten_bit = codec >= 3;
    codec %= 3;
    if (ten_bit && codec == 0) return 0;
    CMVideoCodecType wanted;
    switch (codec) {
    case 0:
        wanted = kCMVideoCodecType_H264;
        break;
    case 1:
        wanted = kCMVideoCodecType_HEVC;
        break;
    case 2:
        /* kCMVideoCodecType_AV1's 'av01' FourCC, also usable with SDKs that
         * predate that enum constant. The caller separately checks whether
         * the bundled FFmpeg has an encoder for this codec.
         */
        wanted = (CMVideoCodecType)0x61763031u;
        break;
    default:
        return -1;
    }

    /* The inventory predates this OS version, but its public hardware flag
     * does not. Never infer encoding support from the decoding-only
     * VTIsHardwareDecodeSupported API or from vendor-specific encoder IDs.
     */
    if (__builtin_available(macOS 10.14, *)) {
        CFArrayRef encoders = NULL;
        const OSStatus status = VTCopyVideoEncoderList(NULL, &encoders);
        if (status != 0 || !encoders) {
            if (encoders)
                CFRelease(encoders);
            return -1;
        }
        if (CFGetTypeID(encoders) != CFArrayGetTypeID()) {
            CFRelease(encoders);
            return -1;
        }
        const CFIndex count = CFArrayGetCount(encoders);
        if (count < 0 || count > SEREIN_MAX_VT_ENCODERS) {
            CFRelease(encoders);
            return -1;
        }

        int available = 0;
        int unknown = 0;
        const CFStringRef *registry_key = target ?
            (const CFStringRef *)dlsym(RTLD_DEFAULT, "kVTVideoEncoderList_GPURegistryID") : NULL;
        if (target && (!registry_key || !*registry_key)) {
            CFRelease(encoders);
            return -1;
        }
        for (CFIndex index = 0; index < count; ++index) {
            const CFTypeRef entry = CFArrayGetValueAtIndex(encoders, index);
            if (!entry || CFGetTypeID(entry) != CFDictionaryGetTypeID()) {
                unknown = 1;
                continue;
            }
            const CFDictionaryRef encoder = (CFDictionaryRef)entry;
            const CFNumberRef kind = (CFNumberRef)CFDictionaryGetValue(
                encoder, kVTVideoEncoderList_CodecType);
            int64_t value = 0;
            if (!kind || CFGetTypeID(kind) != CFNumberGetTypeID() ||
                !CFNumberGetValue(kind, kCFNumberSInt64Type, &value)) {
                unknown = 1;
                continue;
            }
            if (value != (int64_t)wanted)
                continue;
            if (target) {
                const CFNumberRef registry = (CFNumberRef)CFDictionaryGetValue(encoder, *registry_key);
                int64_t registry_id = 0;
                if (!registry || CFGetTypeID(registry) != CFNumberGetTypeID() ||
                    !CFNumberGetValue(registry, kCFNumberSInt64Type, &registry_id)) {
                    unknown = 1;
                    continue;
                }
                if ((uint64_t)registry_id != target->value)
                    continue;
            }
            const CFBooleanRef hardware = (CFBooleanRef)CFDictionaryGetValue(
                encoder, kVTVideoEncoderList_IsHardwareAccelerated);
            /* This optional flag can be absent. Absence does not establish
             * that the encoder is software-only, so preserve Unknown.
             */
            if (!hardware || CFGetTypeID(hardware) != CFBooleanGetTypeID()) {
                unknown = 1;
                continue;
            }
            if (CFBooleanGetValue(hardware)) {
                if (ten_bit) {
                    /* Query supported profile values on this exact encoder. No
                     * compression session or synthetic picture is created. */
                    CFStringRef identity = CFDictionaryGetValue(encoder, kVTVideoEncoderList_EncoderID);
                    if (!identity || CFGetTypeID(identity) != CFStringGetTypeID()) { unknown = 1; continue; }
                    CFMutableDictionaryRef specification = CFDictionaryCreateMutable(NULL, 0,
                        &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
                    if (!specification) { unknown = 1; continue; }
                    CFDictionarySetValue(specification, kVTVideoEncoderSpecification_EncoderID, identity);
                    CFDictionarySetValue(specification, kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder, kCFBooleanTrue);
                    CFDictionaryRef properties = NULL;
                    CFStringRef selected = NULL;
                    OSStatus result = VTCopySupportedPropertyDictionaryForEncoder(1920, 1080,
                        wanted, specification, &selected, &properties);
                    CFRelease(specification);
                    if (selected) CFRelease(selected);
                    if (result || !properties) { if (properties) CFRelease(properties); unknown = 1; continue; }
                    CFDictionaryRef profile = CFDictionaryGetValue(properties, kVTCompressionPropertyKey_ProfileLevel);
                    CFArrayRef values = profile && CFGetTypeID(profile) == CFDictionaryGetTypeID() ?
                        CFDictionaryGetValue(profile, kVTPropertySupportedValueListKey) : NULL;
                    if (!values || CFGetTypeID(values) != CFArrayGetTypeID() || CFArrayGetCount(values) > 512) {
                        CFRelease(properties); unknown = 1; continue;
                    }
                    int supports = codec == 1 && CFArrayContainsValue(values,
                        CFRangeMake(0, CFArrayGetCount(values)), kVTProfileLevel_HEVC_Main10_AutoLevel);
                    CFRelease(properties);
                    if (!supports) continue;
                }
                available = 1;
                break;
            }
        }
        CFRelease(encoders);
        return available ? 1 : unknown ? -1 : 0;
    }
    return -1;
}
int serein_query_videotoolbox(int codec) { return query_videotoolbox(codec, NULL); }
int serein_query_videotoolbox_on_adapter(int codec, const SereinVideoAdapter *target)
{
    if (!serein_video_adapter_valid(target) || target->identity != SEREIN_GPU_METAL_REGISTRY)
        return -1;
    return query_videotoolbox(codec, target);
}
#else
int serein_query_videotoolbox(int codec)
{
    (void)codec;
    return 0;
}
int serein_query_videotoolbox_on_adapter(int codec, const SereinVideoAdapter *target)
{ (void)codec; (void)target; return 0; }
#endif

#endif

#if SEREIN_QUERY_PART == 0 || SEREIN_QUERY_PART == 4
/* Driver-advertised support for the exact FFmpeg paths compiled into this build.
 * No encoder is initialized and no picture, capture or network session is opened.
 */
#include <libavcodec/avcodec.h>
#include "video_encode_ffmpeg.h"

int serein_query_nvenc(int codec);
#if defined(SEREIN_HAVE_AMF_QUERY)
int serein_query_amf(int codec);
#else
static int serein_query_amf(int codec) { (void)codec; return -1; }
/* Without the SDK, exact adapter capabilities remain inconclusive. */
int serein_query_amf_on_adapter(int codec, const SereinVideoAdapter *adapter)
{ (void)codec; (void)adapter; return -1; }
#endif
int serein_query_qsv(int codec);
int serein_query_videotoolbox(int codec);

int serein_video_query(int backend, int codec)
{
    static const char *const names[4][3] = {
        {"h264_nvenc", "hevc_nvenc", "av1_nvenc"},
        {"h264_videotoolbox", "hevc_videotoolbox", NULL},
        {"h264_amf", "hevc_amf", "av1_amf"},
        {"h264_qsv", "hevc_qsv", "av1_qsv"},
    };
    if (backend < 1 || backend > 4 || codec < 0 || codec > 2)
        return -1;
    const char *name = names[backend - 1][codec % 3];
    if (!name || !avcodec_find_encoder_by_name(name))
        return 0;
    switch (backend) {
    case 1: return serein_query_nvenc(codec);
    case 2: return serein_query_videotoolbox(codec);
    case 3: return serein_query_amf(codec);
    case 4: return serein_query_qsv(codec);
    default: return -1;
    }
}

int serein_video_query_on_adapter(int backend, int codec, const SereinVideoAdapter *adapter)
{
    static const char *const names[4][3] = {
        {"h264_nvenc", "hevc_nvenc", "av1_nvenc"},
        {"h264_videotoolbox", "hevc_videotoolbox", NULL},
        {"h264_amf", "hevc_amf", "av1_amf"},
        {"h264_qsv", "hevc_qsv", "av1_qsv"},
    };
    if (backend < 1 || backend > 4 || codec < 0 || codec > 5 ||
        !serein_video_adapter_valid(adapter))
        return -1;
    const char *name = names[backend - 1][codec % 3];
    if (!name || !avcodec_find_encoder_by_name(name))
        return 0;
    /* A capability on another GPU must not advertise the selected adapter. */
    switch (backend) {
    case 1: return adapter->vendor_id == 0x10de ? serein_query_nvenc_on_adapter(codec, adapter) : 0;
    case 2: return serein_query_videotoolbox_on_adapter(codec, adapter);
    case 3: return adapter->vendor_id == 0x1002 ? serein_query_amf_on_adapter(codec, adapter) : 0;
    case 4: return adapter->vendor_id == 0x8086 ? serein_query_qsv_on_adapter(codec, adapter) : 0;
    default: return -1;
    }
}

#endif
