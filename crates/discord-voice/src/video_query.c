/* Driver-advertised support for the exact FFmpeg paths compiled into this build.
 * No encoder is initialized and no picture, capture or network session is opened.
 */
#include <libavcodec/avcodec.h>
#include "video_gpu.h"

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
    const char *name = names[backend - 1][codec];
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
    if (backend < 1 || backend > 4 || codec < 0 || codec > 2 ||
        !serein_video_adapter_valid(adapter))
        return -1;
    const char *name = names[backend - 1][codec];
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
