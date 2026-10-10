/* Bounded FFmpeg H.264, HEVC and AV1 encoding, shared by the camera and screen workers.
 * This translation unit uses only libavcodec/libavutil: it opens encoder GPU
 * devices, never capture sessions, containers or network transports. Its explicit
 * encoder names cannot select Media Foundation, VA-API or a GPL encoder.
 */
#include "video_encode_ffmpeg.h"

#include <errno.h>
#include <limits.h>
#include <string.h>

#include <libavcodec/avcodec.h>
#include <libavutil/error.h>
#include <libavutil/frame.h>
#include <libavutil/hwcontext.h>
#include <libavutil/mem.h>
#include <libavutil/opt.h>

#define SEREIN_MAX_PACKET_BYTES ((size_t)2 * 1024 * 1024)
#define SEREIN_MAX_IN_FLIGHT 48
#define SEREIN_MAX_NALS 2048

typedef struct SereinAvc {
    AVCodecContext *codec;
    AVFrame *frame;
    AVPacket *packet;
    size_t input_bytes;
    size_t max_bytes;
    int64_t next_pts;
    int in_flight;
    int baseline;
    int failed;
    int kind;
    int av1_reduced_still;
    int has_output;
    int features;
    int split_requested;
    int amf_split;
} SereinAvc;

/* Use picture dimensions, including portrait capture, rather than pixel count:
 * AMF keeps its ordinary PA/quality settings below 4K; NVENC/QSV start at 1440p.
 * A 1080p ultrawide does not qualify for either optional split request. */
static int split_resolution(int width, int height, int backend)
{
    if (backend == 3)
        return width >= 2160 && height >= 2160 && (width >= 3840 || height >= 3840);
    return width >= 1440 && height >= 1440;
}

static int wants_split(int width, int height, int backend, int kind)
{
    if (kind == 0 || !split_resolution(width, height, backend))
        return 0;
    if (backend == 1 || backend == 4)
        return 1;
#if defined(_WIN32)
    /* AMF split-frame encoding currently requires DX11 memory. */
    if (backend == 3)
        return 1;
#endif
    return 0;
}

/* This callback reads immutable state only, including if libavcodec invokes
 * it from an encoder thread. Reject the size before the default allocator or
 * FFmpeg's codec-specific packet-copy routine can touch the payload. */
static int bounded_encode_buffer(AVCodecContext *codec, AVPacket *packet,
                                 int flags)
{
    const SereinAvc *encoder = codec->opaque;
    if (!encoder || packet->size <= 0 ||
        (size_t)packet->size > encoder->max_bytes)
        return AVERROR(ENOSPC);
    return avcodec_default_get_encode_buffer(codec, packet, flags);
}

static int set_option(AVCodecContext *codec, const char *name,
                      const char *value)
{
    return av_opt_set(codec->priv_data, name, value, 0) >= 0;
}

static void record_amf_split(SereinAvc *encoder, int backend)
{
    encoder->amf_split = 0;
    if (backend == 3 && encoder->split_requested) {
        int64_t accepted = 0;
        encoder->amf_split = av_opt_get_int(encoder->codec->priv_data, "split_accepted", 0, &accepted) >= 0 &&
                             accepted == 1 ? 2 : 1;
    }
}

static int configure_backend(SereinAvc *encoder, int backend)
{
    AVCodecContext *codec = encoder->codec;
    const int h264 = encoder->kind == 0;
    const int av1 = encoder->kind == 2;
    /* A one-picture DPB cannot retain both references for reordered output.
     * Let the selected preset/driver choose its reference structure when B
     * frames are requested; keep the existing single-reference non-B modes. */
    codec->refs = encoder->features & 1 ? 0 : 1;
    if (backend == 0) {
        /* OpenH264 2.6 preserves real-time / low-complexity defaults. */
        return h264 && set_option(codec, "profile", encoder->baseline ? "constrained_baseline" : "main") &&
               set_option(codec, "coder", encoder->baseline ? "cavlc" : "cabac") &&
               set_option(codec, "rc_mode", "bitrate") && set_option(codec, "allow_skip_frames", "0");
    }
    if (backend == 1) {
        /* AV1 NVENC has no private profile option; the context requests Main. */
        return (av1 || set_option(codec, "profile", h264 && encoder->baseline ? "baseline" : "main")) &&
               (h264 || set_option(codec, "split_encode_mode", encoder->split_requested ? "2" : "disabled")) &&
               set_option(codec, "preset", "p5") && set_option(codec, "tune", "hq") &&
               set_option(codec, "rc", "cbr") &&
               set_option(codec, "rc-lookahead", encoder->features & 2 ? "16" : "0") &&
               set_option(codec, "forced-idr", "1") &&
               set_option(codec, "surfaces", encoder->features & 2 ? "24" : encoder->features & 1 ? "12" : "4");
    }
    if (backend == 3) {
        const int hevc = encoder->kind == 1;
        if (!set_option(codec, "profile", h264 && encoder->baseline ? "constrained_baseline" : "main") ||
            !set_option(codec, "usage", hevc ? "high_quality" : "transcoding") ||
            !set_option(codec, "quality", hevc ? "quality" : "balanced") ||
            !set_option(codec, "rc", "cbr") ||
            !set_option(codec, "preanalysis", encoder->features & 2 ? "1" : "0") ||
            !set_option(codec, "preencode", "0") ||
            !set_option(codec, "latency", av1 ? "none" : "0"))
            return 0;
        if ((encoder->features & 2) && !set_option(codec, "pa_lookahead_buffer_depth", "16"))
            return 0;
        if (h264)
            return set_option(codec, "frame_skipping", "0") && set_option(codec, "bf", "0") &&
                   set_option(codec, "max_b_frames", "0") && set_option(codec, "header_spacing", "1") &&
                   av_opt_set_int(codec->priv_data, "max_au_size", (int64_t)encoder->max_bytes * 8, 0) >= 0;
        /* The bundled wrapper checks codec-specific engine count on the bound
         * GPU. Only eligible two-engine DX11 sessions disable PA/filler/boost;
         * a one-engine device keeps its ordinary quality configuration. */
        if (!set_option(codec, "split_encode", encoder->split_requested ? "1" : "0"))
            return 0;
        if (av1)
            return set_option(codec, "skip_frame", "0") && set_option(codec, "header_insertion_mode", "frame");
        return set_option(codec, "skip_frame", "0") && set_option(codec, "header_insertion_mode", "idr") &&
               set_option(codec, "gops_per_idr", "1") &&
               av_opt_set_int(codec->priv_data, "max_au_size", (int64_t)encoder->max_bytes * 8, 0) >= 0;
    }
    if (backend == 4) {
        if (!set_option(codec, "profile", h264 && encoder->baseline ? "baseline" : "main") ||
            !set_option(codec, "preset", "medium") ||
            !set_option(codec, "look_ahead_depth", encoder->features & 2 ? "16" : "0") ||
            !set_option(codec, "extbrc", encoder->features & 2 ? "1" : "0") ||
            !set_option(codec, "forced_idr", "1") ||
            av_opt_set_int(codec->priv_data, "max_frame_size", (int64_t)encoder->max_bytes, 0) < 0)
            return 0;
        if (!h264) {
            /* Two tile columns let Intel's runtime use at most two engines on
             * this GPU. AV1 needs additional rows at 8K to meet MAX_TILE_AREA;
             * those rows do not increase the requested column/engine count. */
            const int columns = encoder->split_requested || (av1 && codec->width > 4096) ? 2 : 1;
            /* Accommodate either 64- or 128-pixel AV1 superblocks. */
            const int tile_width = ((codec->width + columns * 128 - 1) / (columns * 128)) * 128;
            const int rows = av1 && (int64_t)tile_width * codec->height > 4096 * 2304 ? 2 : 1;
            if (av_opt_set_int(codec->priv_data, "tile_cols", columns, 0) < 0 ||
                av_opt_set_int(codec->priv_data, "tile_rows", rows, 0) < 0)
                return 0;
        }
        if (av1)
            return 1;
        if (!set_option(codec, "idr_interval", "0"))
            return 0;
        return !h264 || (set_option(codec, "look_ahead", "0") && set_option(codec, "repeat_pps", "1") &&
                          set_option(codec, "cavlc", encoder->baseline ? "1" : "0"));
    }
    return !av1 && set_option(codec, "profile", h264 && encoder->baseline ? "baseline" : "main") &&
           set_option(codec, "realtime", "0") && set_option(codec, "allow_sw", "0") &&
           set_option(codec, "max_ref_frames", encoder->features & 1 ? "0" : "1");
}

static int initialize_device(AVCodecContext *codec, int backend)
{
    AVDictionary *options = NULL;
    enum AVHWDeviceType type = AV_HWDEVICE_TYPE_NONE;
    const char *implementation = NULL;
    int status = 0;
    if (backend == 4) {
        type = AV_HWDEVICE_TYPE_QSV;
        implementation = "hw_any";
#if defined(_WIN32)
        status = av_dict_set(&options, "child_device_type", "d3d11va", 0);
#elif defined(__linux__)
        status = av_dict_set(&options, "child_device_type", "vaapi", 0);
#else
        return 0;
#endif
    }
#if defined(_WIN32)
    if (backend == 3) {
        /* An Intel/NVIDIA display adapter must not mask a second AMD GPU. */
        type = AV_HWDEVICE_TYPE_D3D11VA;
        status = av_dict_set(&options, "vendor_id", "0x1002", 0);
    }
#endif
    if (type == AV_HWDEVICE_TYPE_NONE)
        return 1;
    /* AVHWDeviceContext has no public logging offset. Its driver-init errors
     * may reach stderr; never modify a process-wide logger to suppress them.
     * Each backend is tried once per stream, keeping failure attempts bounded. */
    if (status >= 0)
        status = av_hwdevice_ctx_create(&codec->hw_device_ctx, type,
                                        implementation, options, 0);
    av_dict_free(&options);
    return status >= 0;
}

void serein_avc_close(void *opaque)
{
    SereinAvc *encoder = opaque;
    if (!encoder)
        return;
    /* Closing libavcodec stops its native work before the immutable callback
     * state or retained frame storage is released. There is no flush on close:
     * stopping a share/camera must discard pending pictures. */
    avcodec_free_context(&encoder->codec);
    av_frame_free(&encoder->frame);
    av_packet_free(&encoder->packet);
    av_free(encoder);
}

int serein_avc_amf_split(void *opaque)
{
    const SereinAvc *encoder = opaque;
    return encoder ? encoder->amf_split : 0;
}

void *serein_avc_open(int width, int height, int fps, int bitrate, int baseline,
                     int backend, int kind, size_t max_bytes)
{
    return serein_avc_open_on_adapter(width, height, fps, bitrate, baseline,
                                      backend, kind, max_bytes, NULL, 0);
}

static void *open_with_split(int width, int height, int fps, int bitrate,
                                int baseline, int backend, int kind,
                                size_t max_bytes, const SereinVideoAdapter *adapter,
                                int features, int allow_split)
{
    static const char *const names[][5] = {
        {"libopenh264", "h264_nvenc", "h264_videotoolbox", "h264_amf", "h264_qsv"},
        {NULL, "hevc_nvenc", "hevc_videotoolbox", "hevc_amf", "hevc_qsv"},
        {NULL, "av1_nvenc", NULL, "av1_amf", "av1_qsv"}
    };
    static const enum AVCodecID ids[] = {AV_CODEC_ID_H264, AV_CODEC_ID_HEVC, AV_CODEC_ID_AV1};
    const AVCodec *implementation;
    SereinAvc *encoder;
    AVCodecContext *codec;
    int b_frames = features & 1 ? 2 : 0;

    if (width <= 0 || height <= 0 || width > 7680 || height > 4320 ||
        (width & 1) || (height & 1) || fps <= 0 || fps > 60 ||
        bitrate <= 0 || bitrate > 100000000 ||
        (baseline != 0 && baseline != 1) || backend < 0 || backend > 4 || kind < 0 || kind > 2 ||
        features < 0 || features > 3 || (baseline && features) || (kind == 0 && (features & 1)) ||
        max_bytes == 0 || max_bytes > SEREIN_MAX_PACKET_BYTES)
        return NULL;

    if (!names[kind][backend])
        return NULL;
    if (backend == 1 && adapter && features) {
        int max_b_frames = 0, lookahead = 0;
        if (serein_nvenc_features(kind, adapter, &max_b_frames, &lookahead) == 1 &&
            (((features & 1) && max_b_frames < 1) || ((features & 2) && !lookahead)))
            return NULL;
        if ((features & 1) && max_b_frames == 1)
            b_frames = 1;
    }
    implementation = avcodec_find_encoder_by_name(names[kind][backend]);
    if (!implementation || implementation->id != ids[kind])
        return NULL;

    encoder = av_mallocz(sizeof(*encoder));
    if (!encoder)
        return NULL;
    /* Dimension checks above bound every input picture to at most 47.5 MiB. */
    encoder->input_bytes = (size_t)width * (size_t)height * 3 / 2;
    encoder->max_bytes = max_bytes;
    encoder->baseline = baseline;
    encoder->kind = kind;
    encoder->features = features;
    encoder->split_requested = allow_split && wants_split(width, height, backend, kind);
    encoder->codec = avcodec_alloc_context3(implementation);
    encoder->frame = av_frame_alloc();
    encoder->packet = av_packet_alloc();
    if (!encoder->codec || !encoder->frame || !encoder->packet)
        goto failed;

    codec = encoder->codec;
    /* The default FFmpeg logger masks the low byte of a level. 128 keeps every
     * documented level (FATAL..TRACE) above TRACE without wrapping that byte.
     * This is per context: never replace another component's global callback.
     * FFmpeg routes the OpenH264 trace callback through this context as well. */
    codec->log_level_offset = 128;
    codec->opaque = encoder;
    codec->get_encode_buffer = bounded_encode_buffer;
    codec->width = width;
    codec->height = height;
    codec->pix_fmt = backend == 4 ? AV_PIX_FMT_NV12 : AV_PIX_FMT_YUV420P;
    /* Both RGB/BGRA converters pack limited-range BT.601 samples, including
     * at HD resolutions. Signal that matrix instead of allowing a decoder or
     * hardware encoder to infer BT.709 from picture size. Matrix conversion
     * preserves the captured SDR RGB primaries and sRGB transfer function. */
    codec->color_range = AVCOL_RANGE_MPEG;
    codec->colorspace = AVCOL_SPC_SMPTE170M;
    codec->color_primaries = AVCOL_PRI_BT709;
    codec->color_trc = AVCOL_TRC_IEC61966_2_1;
    codec->time_base = (AVRational){1, fps};
    codec->framerate = (AVRational){fps, 1};
    codec->sample_aspect_ratio = (AVRational){1, 1};
    codec->bit_rate = bitrate;
    codec->rc_min_rate = bitrate;
    codec->rc_max_rate = bitrate;
    codec->rc_buffer_size = bitrate;
    if (backend == 4 && (size_t)bitrate > max_bytes * 8)
        codec->rc_buffer_size = (int)(max_bytes * 8);
    codec->rc_initial_buffer_occupancy = codec->rc_buffer_size / 2;
    codec->gop_size = baseline ? 1 : fps * 2;
    codec->max_b_frames = b_frames;
    codec->thread_count = width * height <= 640 * 480 ? 2 : 4;
    codec->profile = kind == 0 ? (baseline ? AV_PROFILE_H264_BASELINE : AV_PROFILE_H264_MAIN) :
                     kind == 1 ? AV_PROFILE_HEVC_MAIN : AV_PROFILE_AV1_MAIN;
    /* Output stays in decoder submission order. Packet PTS travels through the
     * bounded Rust timeline instead of being replaced with the output clock. */
    codec->flags = (int)((unsigned int)codec->flags | AV_CODEC_FLAG_CLOSED_GOP);
    /* The pinned FFmpeg encoders repeat parameter sets on IDRs without
     * GLOBAL_HEADER. VideoToolbox's wrapper converts native AVCC to Annex B
     * and prepends its CMSampleBuffer's current SPS/PPS, so no out-of-band or
     * untrusted extradata parser is needed here. Verify the result below. */
    codec->flags &= ~AV_CODEC_FLAG_GLOBAL_HEADER;
    if (!configure_backend(encoder, backend) ||
        (adapter && backend != 0 ? !serein_video_bind_adapter(codec, backend, adapter) : !initialize_device(codec, backend)) ||
        avcodec_open2(codec, implementation, NULL) < 0)
        goto failed;

    record_amf_split(encoder, backend);

    encoder->frame->format = codec->pix_fmt;
    encoder->frame->width = width;
    encoder->frame->height = height;
    /* VideoToolbox also takes color attachments from the submitted frame. */
    encoder->frame->color_range = codec->color_range;
    encoder->frame->colorspace = codec->colorspace;
    encoder->frame->color_primaries = codec->color_primaries;
    encoder->frame->color_trc = codec->color_trc;
    if (av_frame_get_buffer(encoder->frame, 32) < 0)
        goto failed;
    return encoder;

failed:
    serein_avc_close(encoder);
    return NULL;
}

void *serein_avc_open_on_adapter(int width, int height, int fps, int bitrate,
                                int baseline, int backend, int kind,
                                size_t max_bytes, const SereinVideoAdapter *adapter,
                                int features)
{
    void *encoder = open_with_split(width, height, fps, bitrate, baseline,
                                    backend, kind, max_bytes, adapter, features, 1);
    /* Some older GPUs/drivers reject split mode or two-column tiling. Release
     * the complete failed context before one conservative retry on the same
     * physical adapter, codec and quality settings. No second session survives. */
    if (!encoder && wants_split(width, height, backend, kind)) {
        encoder = open_with_split(width, height, fps, bitrate, baseline,
                                  backend, kind, max_bytes, adapter, features, 0);
        if (encoder && backend == 3)
            ((SereinAvc *)encoder)->amf_split = 1;
    }
    return encoder;
}

static size_t start_code_size(const uint8_t *bytes, size_t length, size_t at)
{
    if (length - at >= 4 && bytes[at] == 0 && bytes[at + 1] == 0 &&
        bytes[at + 2] == 0 && bytes[at + 3] == 1)
        return 4;
    if (length - at >= 3 && bytes[at] == 0 && bytes[at + 1] == 0 &&
        bytes[at + 2] == 1)
        return 3;
    return 0;
}

typedef struct PacketInfo {
    int keyframe;
    unsigned int parameters;
    unsigned int key_parameters;
    int picture;
} PacketInfo;

/* Inspect Annex B in place. HEVC uses two-byte headers and VPS/SPS/PPS;
 * H.264 uses one-byte headers and SPS/PPS. Parameter availability is checked
 * after optionally inspecting the codec's bounded raw extradata below. */
static int inspect_annex_b(const uint8_t *bytes, size_t length, int kind, PacketInfo *info)
{
    size_t at = 0;
    int nals = 0;
    *info = (PacketInfo){0};
    while (at < length) {
        size_t prefix = start_code_size(bytes, length, at);
        size_t payload, end;
        int type;
        if (!prefix || ++nals > SEREIN_MAX_NALS)
            return 0;
        payload = at + prefix;
        if (payload == length)
            return 0;
        end = payload;
        while (end < length && !start_code_size(bytes, length, end))
            end++;
        if (end == payload || (bytes[payload] & 0x80))
            return 0;
        if (kind == 0) {
            type = bytes[payload] & 0x1f;
            if (type == 0 || type >= 24)
                return 0;
            if (type == 7) info->parameters |= 1;
            if (type == 8) info->parameters |= 2;
            if (type == 5) {
                if (!info->keyframe) info->key_parameters = info->parameters;
                info->keyframe = 1;
            }
            if (type == 1 || type == 5) info->picture = 1;
        } else {
            if (end - payload < 2 || !(bytes[payload + 1] & 7))
                return 0;
            type = (bytes[payload] >> 1) & 0x3f;
            if (type == 32) info->parameters |= 1;
            if (type == 33) info->parameters |= 2;
            if (type == 34) info->parameters |= 4;
            if (type >= 16 && type <= 21) {
                if (!info->keyframe) info->key_parameters = info->parameters;
                info->keyframe = 1;
            }
            if (type <= 31) info->picture = 1;
        }
        at = end;
    }
    return nals != 0;
}

/* Low-overhead AV1 OBUs have an explicit bounded LEB128 payload size. The
 * first uncompressed-header bits distinguish show-existing and inter/intra
 * pictures from KEY_FRAME; the sequence's reduced-still mode omits those bits.
 * No payload is passed to a generic demuxer or dynamically allocated parser. */
static int inspect_av1(const uint8_t *bytes, size_t length, int *reduced_still, PacketInfo *info)
{
    size_t at = 0;
    int units = 0;
    *info = (PacketInfo){0};
    while (at < length) {
        uint8_t header = bytes[at++];
        int type = (header >> 3) & 15;
        uint64_t payload = 0;
        int completed = 0;
        if ((header & 0x81) || !(header & 2) || ++units > SEREIN_MAX_NALS ||
            type == 0 || (type >= 9 && type <= 14))
            return 0;
        if (header & 4) {
            if (at == length || (bytes[at++] & 7))
                return 0;
        }
        for (unsigned int index = 0; index < 8; index++) {
            uint8_t byte;
            if (at == length)
                return 0;
            byte = bytes[at++];
            payload |= (uint64_t)(byte & 127) << (index * 7);
            if (!(byte & 128)) { completed = 1; break; }
        }
        if (!completed || payload > (uint64_t)(length - at))
            return 0;
        if (type == 1) {
            if (!payload || (bytes[at] >> 5) > 2)
                return 0;
            *reduced_still = !!(bytes[at] & 8);
            if (*reduced_still && !(bytes[at] & 16))
                return 0;
            info->parameters |= 1;
        } else if (type == 3 || type == 6) {
            if (!payload)
                return 0;
            info->picture = 1;
            if (*reduced_still || (!(bytes[at] & 128) && !(bytes[at] & 96))) {
                if (!info->keyframe) info->key_parameters = info->parameters;
                info->keyframe = 1;
            }
        } else if (type == 2 && payload != 0) {
            return 0;
        }
        at += (size_t)payload;
    }
    return units != 0;
}

static int inspect_packet(const uint8_t *bytes, size_t length, int kind,
                          int *reduced_still, PacketInfo *info)
{
    return kind == 2 ? inspect_av1(bytes, length, reduced_still, info) :
                       inspect_annex_b(bytes, length, kind, info);
}

/* A native driver may report a packet length larger than its negotiated
 * storage. Check actual refcounted capacity before reading any packet bytes.
 * The selected encoders use the default padded packet allocator and no data
 * offset. This does not protect against a driver overwriting its MaxLength. */
static int packet_fits(const AVPacket *packet, size_t max_bytes, size_t capacity)
{
    if (!packet->buf || !packet->data || packet->size <= 0 ||
        packet->data != packet->buf->data ||
        packet->buf->size < AV_INPUT_BUFFER_PADDING_SIZE)
        return 0;
    return (size_t)packet->size <= max_bytes &&
           (size_t)packet->size <= capacity &&
           (size_t)packet->size <= packet->buf->size - AV_INPUT_BUFFER_PADDING_SIZE;
}

/* The validated I420 source is copied into owned planes. Intel's NV12 layout
 * interleaves U/V directly into the allocated frame, with no extra scratch. */
static void copy_picture(AVFrame *frame, const uint8_t *input)
{
    size_t offset = 0;
    for (int plane = 0; plane < (frame->format == AV_PIX_FMT_NV12 ? 1 : 3); plane++) {
        int width = frame->width >> (plane != 0);
        int height = frame->height >> (plane != 0);
        for (int row = 0; row < height; row++) {
            memcpy(frame->data[plane] + (size_t)row * (size_t)frame->linesize[plane],
                   input + offset, (size_t)width);
            offset += (size_t)width;
        }
    }
    if (frame->format == AV_PIX_FMT_NV12) {
        size_t luma = (size_t)frame->width * (size_t)frame->height;
        size_t chroma_width = (size_t)frame->width / 2;
        for (size_t row = 0; row < (size_t)frame->height / 2; row++) {
            uint8_t *destination = frame->data[1] + row * (size_t)frame->linesize[1];
            for (size_t column = 0; column < chroma_width; column++) {
                size_t source = row * chroma_width + column;
                destination[column * 2] = input[luma + source];
                destination[column * 2 + 1] = input[luma + luma / 4 + source];
            }
        }
    }
}

int serein_avc_encode(void *opaque, const uint8_t *input, size_t length,
                      int force_keyframe, uint8_t *output,
                      size_t output_capacity, size_t *output_length,
                      int *keyframe)
{
    int64_t presentation_index;
    return serein_avc_encode_timed(opaque, input, length, force_keyframe,
                                   output, output_capacity, output_length,
                                   keyframe, &presentation_index);
}

int serein_avc_encode_timed(void *opaque, const uint8_t *input, size_t length,
                            int force_keyframe, uint8_t *output,
                            size_t output_capacity, size_t *output_length,
                            int *keyframe, int64_t *presentation_index)
{
    SereinAvc *encoder = opaque;
    int status, reduced_still;
    PacketInfo info;
    unsigned int required;
    size_t prefix_bytes = 0, packet_bytes;

    if (output_length)
        *output_length = 0;
    if (keyframe)
        *keyframe = 0;
    if (presentation_index)
        *presentation_index = -1;
    if (!encoder || !input || !output || !output_length || !keyframe || !presentation_index ||
        encoder->failed || length != encoder->input_bytes || output_capacity == 0)
        return -1;
    if (encoder->in_flight >= SEREIN_MAX_IN_FLIGHT || encoder->next_pts == INT64_MAX)
        goto failed;
    if (av_frame_make_writable(encoder->frame) < 0)
        goto failed;
    copy_picture(encoder->frame, input);
    encoder->frame->pts = encoder->next_pts++;
    encoder->frame->duration = 1;
    encoder->frame->pict_type = (force_keyframe || encoder->baseline) ?
                               AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
    status = avcodec_send_frame(encoder->codec, encoder->frame);
    /* We drain output on every submission. Normal hardware modes can retain
     * pictures up to the bounded in-flight limit. An unexpected send-side
     * EAGAIN would mean this input was not accepted; the
     * ABI cannot ambiguously return an older packet while losing that input. */
    if (status < 0)
        goto failed;
    encoder->in_flight++;
    av_packet_unref(encoder->packet);
    status = avcodec_receive_packet(encoder->codec, encoder->packet);
    if (status == AVERROR(EAGAIN))
        return 0;
    if (status < 0 || !packet_fits(encoder->packet, encoder->max_bytes, output_capacity))
        goto failed;
    packet_bytes = (size_t)encoder->packet->size;
    required = encoder->kind == 0 ? 3U : encoder->kind == 1 ? 7U : 1U;
    reduced_still = encoder->av1_reduced_still;
    if (!inspect_packet(encoder->packet->data, packet_bytes, encoder->kind, &reduced_still, &info) || !info.picture)
        goto failed;
    if (encoder->kind == 2 && !(encoder->packet->flags & AV_PKT_FLAG_KEY))
        info.keyframe = 0;
    if (info.keyframe && info.key_parameters != required) {
        PacketInfo extra;
        /* Some QSV HEVC drivers supply VPS in extradata rather than each AU.
         * Accept only raw, parameter-only Annex B / sized OBUs. Never interpret
         * arbitrary AVCC/HVCC/container bytes as an encoded access unit. */
        int bytes = encoder->codec->extradata_size;
        if (bytes <= 0 || !encoder->codec->extradata || (size_t)bytes > encoder->max_bytes ||
            !inspect_packet(encoder->codec->extradata, (size_t)bytes, encoder->kind, &reduced_still, &extra) ||
            extra.picture || (extra.parameters | info.key_parameters) != required)
            goto failed;
        prefix_bytes = (size_t)bytes;
    }
    if ((encoder->baseline || !encoder->has_output) && !info.keyframe)
        goto failed;
    if (prefix_bytes > encoder->max_bytes - packet_bytes ||
        prefix_bytes > output_capacity - packet_bytes)
        goto failed;
    /* All native/driver sizes, syntax, independence and combined bounds pass
     * before any caller-owned output byte is written. */
    if (prefix_bytes)
        memcpy(output, encoder->codec->extradata, prefix_bytes);
    memcpy(output + prefix_bytes, encoder->packet->data, packet_bytes);
    *output_length = prefix_bytes + packet_bytes;
    *keyframe = info.keyframe;
    *presentation_index = encoder->packet->pts;
    encoder->in_flight--;
    encoder->has_output = 1;
    encoder->av1_reduced_still = reduced_still;
    av_packet_unref(encoder->packet);
    return 1;

failed:
    encoder->failed = 1;
    av_packet_unref(encoder->packet);
    return -1;
}
