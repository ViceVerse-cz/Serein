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
    else
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
