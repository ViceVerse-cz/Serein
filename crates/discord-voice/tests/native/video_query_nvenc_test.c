/* Each variant remains a separately compiled offline fixture. */

#if SEREIN_NVENC_FIXTURE == 1
/* Offline CUDA ABI fixture, loaded from the runner's temporary directory.
 * These opaque mock contexts never access a GPU or allocate a surface. */
#include <ffnvcodec/dynlink_cuda.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>

static CUcontext current;
static int created, destroyed;
static int mode(const char *value) { return strcmp(getenv("SEREIN_QUERY_FIXTURE"), value) == 0; }
void fixture_cuda_reset(void) { created = destroyed = 0; current = NULL; }
int fixture_cuda_created(void) { return created; }
int fixture_cuda_destroyed(void) { return destroyed; }
CUresult CUDAAPI cuInit(unsigned int flags) {
    (void)flags;
    return mode("init-no-device") ? (CUresult)100 : mode("init-failed") ? CUDA_ERROR_UNKNOWN : CUDA_SUCCESS;
}
CUresult CUDAAPI cuDeviceGetCount(int *count) {
    if (mode("count-failed")) return CUDA_ERROR_UNKNOWN;
    *count = mode("no-device") ? 0 : mode("bound") ? 33 : (mode("multi") || mode("error-then-supported")) ? 2 : 1;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuDeviceGet(CUdevice *device, int ordinal) {
    *device = ordinal;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuDeviceGetPCIBusId(char *address, int capacity, CUdevice device) {
    if (mode("identity-error")) return CUDA_ERROR_UNKNOWN;
    snprintf(address, (size_t)capacity, "0000:%02x:00.0", device + 1);
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuCtxCreate_v2(CUcontext *context, unsigned int flags, CUdevice device) {
    (void)flags;
    if (mode("context-failed")) return CUDA_ERROR_UNKNOWN;
    *context = (CUcontext)(uintptr_t)(device + 1);
    current = *context;
    created++;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuCtxPopCurrent_v2(CUcontext *context) {
    *context = current;
    current = NULL;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuCtxDestroy_v2(CUcontext context) {
    (void)context;
    destroyed++;
    return CUDA_SUCCESS;
}

#elif SEREIN_NVENC_FIXTURE == 2
/* Offline NVENC ABI fixture. It advertises codecs without an actual driver;
 * every encoder initialization, frame/buffer operation is forbidden. */
#include <ffnvcodec/nvEncodeAPI.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

static int opened, closed, forbidden;
static int mode(const char *value) { return strcmp(getenv("SEREIN_QUERY_FIXTURE"), value) == 0; }
void fixture_nv_reset(void) { opened = closed = forbidden = 0; }
int fixture_nv_opened(void) { return opened; }
int fixture_nv_closed(void) { return closed; }
int fixture_nv_forbidden(void) { return forbidden; }
static NVENCSTATUS NVENCAPI open_session(NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS *params, void **encoder) {
    if (mode("open-unavailable")) return NV_ENC_ERR_NO_ENCODE_DEVICE;
    if (mode("open-error") || (mode("error-then-supported") && (uintptr_t)params->device == 1)) return NV_ENC_ERR_ENCODER_BUSY;
    *encoder = params->device;
    opened++;
    return NV_ENC_SUCCESS;
}
static NVENCSTATUS NVENCAPI guid_count(void *encoder, uint32_t *count) {
    (void)encoder;
    if (mode("guid-error")) return NV_ENC_ERR_GENERIC;
    *count = mode("zero-guids") ? 0 : mode("excess-guids") ? 65 : 1;
    return NV_ENC_SUCCESS;
}
static NVENCSTATUS NVENCAPI guids(void *encoder, GUID *values, uint32_t size, uint32_t *count) {
    (void)size;
    values[0] = (mode("multi") && (uintptr_t)encoder == 2) || mode("error-then-supported") ? NV_ENC_CODEC_AV1_GUID : NV_ENC_CODEC_H264_GUID;
    *count = mode("short-guids") ? 0 : 1;
    return NV_ENC_SUCCESS;
}
static NVENCSTATUS NVENCAPI caps(void *encoder, GUID codec, NV_ENC_CAPS_PARAM *params, int *value) {
    (void)encoder; (void)codec;
    if (mode("caps-error")) return NV_ENC_ERR_GENERIC;
    if (params->capsToQuery == NV_ENC_CAPS_NUM_MAX_BFRAMES)
        *value = mode("no-advanced") ? 0 : 3;
    else if (params->capsToQuery == NV_ENC_CAPS_SUPPORT_LOOKAHEAD)
        *value = mode("no-advanced") ? 0 : 1;
    else if (params->capsToQuery == NV_ENC_CAPS_SUPPORT_10BIT_ENCODE) {
        const char *profile = getenv("SEREIN_HDR_FIXTURE");
        if (profile && !strcmp(profile,"error")) return NV_ENC_ERR_GENERIC;
        *value = profile && !strcmp(profile,"eight") ? 0 : 1;
    } else
        *value = mode("zero-dimensions") ? 0 : 8192;
    return NV_ENC_SUCCESS;
}
static NVENCSTATUS NVENCAPI close_encoder(void *encoder) {
    (void)encoder;
    closed++;
    return mode("close-error") ? NV_ENC_ERR_GENERIC : NV_ENC_SUCCESS;
}
static NVENCSTATUS NVENCAPI initialize_encoder(void *encoder, NV_ENC_INITIALIZE_PARAMS *params) {
    (void)encoder; (void)params;
    forbidden++;
    return NV_ENC_ERR_GENERIC;
}
static NVENCSTATUS NVENCAPI create_input(void *encoder, NV_ENC_CREATE_INPUT_BUFFER *params) {
    (void)encoder; (void)params;
    forbidden++;
    return NV_ENC_ERR_GENERIC;
}
static NVENCSTATUS NVENCAPI create_bitstream(void *encoder, NV_ENC_CREATE_BITSTREAM_BUFFER *params) {
    (void)encoder; (void)params;
    forbidden++;
    return NV_ENC_ERR_GENERIC;
}
static NVENCSTATUS NVENCAPI encode_picture(void *encoder, NV_ENC_PIC_PARAMS *params) {
    (void)encoder; (void)params;
    forbidden++;
    return NV_ENC_ERR_GENERIC;
}
NVENCSTATUS NVENCAPI NvEncodeAPIGetMaxSupportedVersion(uint32_t *version) {
    *version = mode("old-api") ? 0 : ((NVENCAPI_MAJOR_VERSION << 4) | NVENCAPI_MINOR_VERSION);
    return NV_ENC_SUCCESS;
}
NVENCSTATUS NVENCAPI NvEncodeAPICreateInstance(NV_ENCODE_API_FUNCTION_LIST *functions) {
    functions->nvEncOpenEncodeSessionEx = open_session;
    functions->nvEncGetEncodeGUIDCount = guid_count;
    functions->nvEncGetEncodeGUIDs = guids;
    functions->nvEncGetEncodeCaps = mode("missing-caps") ? NULL : caps;
    functions->nvEncDestroyEncoder = close_encoder;
    functions->nvEncInitializeEncoder = initialize_encoder;
    functions->nvEncCreateInputBuffer = create_input;
    functions->nvEncCreateBitstreamBuffer = create_bitstream;
    functions->nvEncEncodePicture = encode_picture;
    return NV_ENC_SUCCESS;
}

#elif !defined(SEREIN_NVENC_FIXTURE) || SEREIN_NVENC_FIXTURE == 0
#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <assert.h>
#include "video_encode_ffmpeg.h"
#ifdef NDEBUG
#error Driver query fixtures require assertions enabled
#endif
int serein_query_nvenc(int codec);
void fixture_cuda_reset(void);
int fixture_cuda_created(void);
int fixture_cuda_destroyed(void);
void fixture_nv_reset(void);
int fixture_nv_opened(void);
int fixture_nv_closed(void);
int fixture_nv_forbidden(void);
int main(void) {
    const struct { const char *mode; int codec, expected, contexts; } cases[] = {
        {"normal", 0, 1, 1}, {"normal", 1, 0, 1}, {"normal", 2, 0, 1},
        {"multi", 2, 1, 2}, {"error-then-supported", 2, 1, 2},
        {"no-device", 0, 0, 0}, {"init-no-device", 0, 0, 0},
        {"init-failed", 0, -1, 0}, {"count-failed", 0, -1, 0},
        {"context-failed", 0, -1, 0}, {"open-unavailable", 0, 0, 1},
        {"open-error", 0, -1, 1}, {"guid-error", 0, -1, 1},
        {"zero-guids", 0, 0, 1}, {"excess-guids", 0, -1, 1},
        {"short-guids", 0, -1, 1}, {"caps-error", 0, -1, 1},
        {"zero-dimensions", 0, 0, 1}, {"close-error", 0, -1, 1},
        {"old-api", 0, -1, 0}, {"missing-caps", 0, -1, 0},
        {"bound", 2, -1, 32}, {"normal", -1, 0, 0}, {"normal", 3, 0, 0},
    };
    for (unsigned int i = 0; i < sizeof(cases)/sizeof(cases[0]); i++) {
        assert(setenv("SEREIN_QUERY_FIXTURE", cases[i].mode, 1) == 0);
        fixture_cuda_reset(); fixture_nv_reset();
        int result = serein_query_nvenc(cases[i].codec);
        if (result != cases[i].expected) {
            fprintf(stderr, "%s codec%d expected%d got%d\n", cases[i].mode, cases[i].codec, cases[i].expected, result);
            return 1;
        }
        assert(fixture_cuda_created() == cases[i].contexts);
        assert(fixture_cuda_destroyed() == fixture_cuda_created());
        assert(fixture_nv_closed() == fixture_nv_opened());
        assert(fixture_nv_forbidden() == 0);
    }
    SereinVideoAdapter target = {SEREIN_GPU_PCI, 0x10de, 0x2684, 0, 1, 0, 0, 0};
    assert(setenv("SEREIN_QUERY_FIXTURE", "multi", 1) == 0);
    fixture_cuda_reset(); fixture_nv_reset();
    /* First GPU has H264 only, second (identical model) has AV1. Never
     * borrow a positive answer from the second GPU for the first target. */
    assert(serein_query_nvenc_on_adapter(2, &target) == 0);
    assert(fixture_cuda_created() == 1 && fixture_cuda_destroyed() == 1);
    target.bus = 2;
    fixture_cuda_reset(); fixture_nv_reset();
    assert(serein_query_nvenc_on_adapter(2, &target) == 1);
    assert(fixture_cuda_created() == 1 && fixture_cuda_destroyed() == 1);
    int b_frames = 0, lookahead = 0;
    assert(serein_nvenc_features(2, &target, &b_frames, &lookahead) == 1);
    assert(b_frames == 3 && lookahead == 1);
    assert(fixture_nv_forbidden() == 0);
    /* Codec support is distinct from profile support on the selected device. */
    assert(setenv("SEREIN_HDR_FIXTURE", "ten", 1) == 0);
    assert(serein_query_nvenc_on_adapter(5, &target) == 1);
    assert(setenv("SEREIN_HDR_FIXTURE", "eight", 1) == 0);
    assert(serein_query_nvenc_on_adapter(5, &target) == 0);
    assert(serein_query_nvenc_on_adapter(2, &target) == 1);
    assert(setenv("SEREIN_HDR_FIXTURE", "error", 1) == 0);
    assert(serein_query_nvenc_on_adapter(5, &target) == -1);
    assert(serein_query_nvenc_on_adapter(3, &target) == 0);
    assert(unsetenv("SEREIN_HDR_FIXTURE") == 0);
    assert(fixture_cuda_created() == fixture_cuda_destroyed());
    assert(fixture_nv_closed() == fixture_nv_opened() && fixture_nv_forbidden() == 0);
    target.bus = 3;
    fixture_cuda_reset(); fixture_nv_reset();
    assert(serein_query_nvenc_on_adapter(2, &target) == -1);
    assert(fixture_cuda_created() == 0 && fixture_nv_opened() == 0);
    target.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(serein_query_nvenc_on_adapter(2, &target) == -1);
    assert(fixture_cuda_created() == 0 && fixture_nv_forbidden() == 0);
    printf("NVENC query: %zu offline driver-boundary cases passed, contexts/sessions released, no encoding calls\n", sizeof(cases)/sizeof(cases[0]));
}

#else
#error Unknown offline fixture variant
#endif
