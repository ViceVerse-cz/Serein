/* Query oneVPL hardware implementation descriptions without creating an
 * encoder, allocating input pictures or submitting synthetic frames.
 * The dispatcher is the same pinned oneVPL 2.x ABI used by bundled FFmpeg.
 * Legacy Media SDK compatibility descriptions omit Enc: that is Unknown,
 * not proof that an otherwise usable Intel GPU supports no video codecs. */
#include <stdint.h>
#include <string.h>
#include "video_gpu.h"

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

static int description_supports_codec(const mfxImplDescription *description, mfxU32 codec)
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
        if (description->Enc.Codecs[i].CodecID == codec)
            return 1;
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
    if (codec < 0 || codec > 2)
        return 0;
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
        int supported = matching == 1 ? description_supports_codec((const mfxImplDescription *)handle, codecs[codec]) : matching;
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

int serein_query_qsv(int codec) { return query_qsv(codec, NULL); }
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
