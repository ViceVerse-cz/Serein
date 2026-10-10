/* Offline oneVPL dispatcher ABI fixture. It does not load Intel drivers or
 * create a session; initialization and frame submission are forbidden. */
#include <vpl/mfxdispatcher.h>
#include <vpl/mfxvideo.h>
#include <stdio.h>
#include <string.h>
#include <assert.h>
#include <stdint.h>
#include "video_gpu.h"
#ifdef NDEBUG
#error Driver query fixtures require assertions enabled
#endif

static const char *mode;
static int releases, unloads, enumerations, filters, forbidden;
static mfxImplDescription description;
static struct encoder encoder;
static mfxExtendedDeviceId extended_device;
int serein_query_qsv(int codec);
mfxLoader MFX_CDECL MFXLoad(void) {
    return strcmp(mode, "load-error") ? (mfxLoader)(uintptr_t)1 : NULL;
}
void MFX_CDECL MFXUnload(mfxLoader loader) { assert(loader); unloads++; }
mfxConfig MFX_CDECL MFXCreateConfig(mfxLoader loader) { assert(loader); return (mfxConfig)(uintptr_t)1; }
mfxStatus MFX_CDECL MFXSetConfigFilterProperty(mfxConfig config, const mfxU8 *name, mfxVariant value) {
    assert(config); assert(value.Version.Version == MFX_VARIANT_VERSION); assert(value.Type == MFX_VARIANT_TYPE_U32);
    filters++;
    if (!strcmp((const char *)name, "mfxImplDescription.Impl")) assert(value.Data.U32 == MFX_IMPL_TYPE_HARDWARE);
    else if (!strcmp((const char *)name, "mfxImplDescription.VendorID")) assert(value.Data.U32 == 0x8086);
    else if (!strcmp((const char *)name, "mfxImplDescription.AccelerationMode")) assert(value.Data.U32 == MFX_ACCEL_MODE_VIA_VAAPI);
    else assert(0);
    return !strcmp(mode, "filter-error") ? MFX_ERR_UNSUPPORTED : MFX_ERR_NONE;
}
mfxStatus MFX_CDECL MFXEnumImplementations(mfxLoader loader, mfxU32 index, mfxImplCapsDeliveryFormat format, mfxHDL *handle) {
    assert(loader); enumerations++;
    if (format == MFX_IMPLCAPS_DEVICE_ID_EXTENDED) {
        assert(!strcmp(mode, "scoped"));
        memset(&extended_device, 0, sizeof(extended_device));
        extended_device.Version.Version = MFX_EXTENDEDDEVICEID_VERSION;
        extended_device.VendorID = 0x8086;
        extended_device.DeviceID = 0x56a0;
        extended_device.PCIBus = index + 1;
        *handle = &extended_device;
        return MFX_ERR_NONE;
    }
    assert(format == MFX_IMPLCAPS_IMPLDESCSTRUCTURE);
    if (!strcmp(mode, "no-driver") || (index > 0 && strcmp(mode, "bound") &&
        (strcmp(mode, "error-then-supported") || index > 1) && (strcmp(mode, "scoped") || index > 1))) return MFX_ERR_NOT_FOUND;
    if (!strcmp(mode, "enum-error")) return MFX_ERR_UNSUPPORTED;
    memset(&description, 0, sizeof(description)); memset(&encoder, 0, sizeof(encoder));
    description.Version.Version = MFX_IMPLDESCRIPTION_VERSION;
    description.ApiVersion.Major = 2;
    description.Impl = MFX_IMPL_TYPE_HARDWARE;
    description.VendorID = 0x8086;
    description.Enc.Version.Version = MFX_ENCODERDESCRIPTION_VERSION;
    description.Enc.NumCodecs = 1;
    description.Enc.Codecs = &encoder;
    encoder.CodecID = !strcmp(mode, "av1") ? MFX_CODEC_AV1 : MFX_CODEC_AVC;
    if (!strcmp(mode, "scoped") && index == 1) encoder.CodecID = MFX_CODEC_AV1;
    if (!strcmp(mode, "legacy") || (!strcmp(mode, "error-then-supported") && !index)) {
        description.ApiVersion.Major = 1;
        memset(&description.Enc, 0, sizeof(description.Enc));
    }
    if (!strcmp(mode, "wrong-vendor")) description.VendorID = 0x1002;
    if (!strcmp(mode, "software")) description.Impl = MFX_IMPL_TYPE_SOFTWARE;
    if (!strcmp(mode, "missing-codecs")) description.Enc.Codecs = NULL;
    if (!strcmp(mode, "excess-codecs")) description.Enc.NumCodecs = 65;
    if (!strcmp(mode, "zero-codecs")) description.Enc.NumCodecs = 0;
    *handle = &description;
    return MFX_ERR_NONE;
}
mfxStatus MFX_CDECL MFXDispReleaseImplDescription(mfxLoader loader, mfxHDL handle) {
    assert(loader); assert(handle == &description || handle == &extended_device); releases++;
    return !strcmp(mode, "release-error") ? MFX_ERR_UNKNOWN : MFX_ERR_NONE;
}
mfxStatus MFX_CDECL MFXCreateSession(mfxLoader loader, mfxU32 index, mfxSession *session) {
    (void)loader; (void)index; (void)session;
    forbidden++;
    return MFX_ERR_UNSUPPORTED;
}
mfxStatus MFX_CDECL MFXVideoENCODE_Init(mfxSession session, mfxVideoParam *params) {
    (void)session; (void)params;
    forbidden++;
    return MFX_ERR_UNSUPPORTED;
}
mfxStatus MFX_CDECL MFXVideoENCODE_EncodeFrameAsync(mfxSession session, mfxEncodeCtrl *control,
        mfxFrameSurface1 *surface, mfxBitstream *bitstream, mfxSyncPoint *sync) {
    (void)session; (void)control; (void)surface; (void)bitstream; (void)sync;
    forbidden++;
    return MFX_ERR_UNSUPPORTED;
}
int main(void) {
    const struct {const char *mode; int codec, expected, released, enumerated;} cases[] = {
        {"normal",0,1,1,1}, {"normal",1,0,1,2}, {"normal",2,0,1,2},
        {"av1",2,1,1,1}, {"no-driver",0,0,0,1}, {"load-error",0,-1,0,0},
        {"filter-error",0,-1,0,0}, {"enum-error",0,-1,0,2},
        {"legacy",0,-1,1,2}, {"wrong-vendor",0,-1,1,2}, {"software",0,-1,1,2},
        {"missing-codecs",0,-1,1,2}, {"excess-codecs",0,-1,1,2},
        {"zero-codecs",0,0,1,2}, {"release-error",0,-1,1,2},
        {"error-then-supported",0,1,2,2}, {"bound",2,-1,32,32},
        {"normal",-1,0,0,0}, {"normal",3,0,0,0},
    };
    for (unsigned int i = 0; i < sizeof(cases)/sizeof(cases[0]); i++) {
        mode = cases[i].mode; releases = unloads = enumerations = filters = forbidden = 0;
        int result = serein_query_qsv(cases[i].codec);
        if (result != cases[i].expected) {
            fprintf(stderr, "%s codec%d expected%d got%d\n", mode, cases[i].codec, cases[i].expected, result);
            return 1;
        }
        assert(releases == cases[i].released); assert(enumerations == cases[i].enumerated);
        assert(unloads == (strcmp(mode,"load-error") && cases[i].codec >= 0 && cases[i].codec <=2 ? 1 : 0));
        if (enumerations) assert(filters == 3);
        assert(forbidden == 0);
    }
    mode = "scoped";
    SereinVideoAdapter target = {SEREIN_GPU_PCI, 0x8086, 0x56a0, 0, 1, 0, 0, 0};
    releases = unloads = enumerations = filters = forbidden = 0;
    assert(serein_query_qsv_on_adapter(2, &target) == 0);
    assert(unloads == 1 && releases == 4 && forbidden == 0);
    target.bus = 2;
    releases = unloads = enumerations = filters = forbidden = 0;
    assert(serein_query_qsv_on_adapter(2, &target) == 1);
    assert(unloads == 1 && releases == 4 && forbidden == 0);
    target.bus = 3;
    assert(serein_query_qsv_on_adapter(2, &target) == 0);
    target.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(serein_query_qsv_on_adapter(2, &target) == -1);
    assert(forbidden == 0);
    printf("QSV query: %zu offline dispatcher-boundary cases passed, descriptions/loaders released, no encoding calls\n", sizeof(cases)/sizeof(cases[0]));
}
