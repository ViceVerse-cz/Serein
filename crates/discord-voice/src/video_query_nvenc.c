/* Codec discovery only: CUDA context + NVENC capability session, never an
 * initialized encoder, input surface, bitstream buffer or submitted picture.
 * The pinned nv-codec-headers SDK also supplies FFmpeg's CUDA/NVENC ABI.
 * Query every bounded CUDA adapter because FFmpeg defaults to gpu=any; a
 * positive result describes an advertised codec, not our streaming presets. */
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "video_gpu.h"

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
                         int *max_b_frames, int *lookahead)
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
    if (codec < 0 || codec > 2)
        return 0;
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
        int adapter = query_adapter(&cuda, &api, i, *codecs[codec], max_b_frames, lookahead);
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

int serein_query_nvenc(int codec) { return query_nvenc(codec, NULL, NULL, NULL); }
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
