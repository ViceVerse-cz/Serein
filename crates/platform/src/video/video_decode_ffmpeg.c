/* Bounded native media boundary. No URLs, network protocols, UI or account state. */
#include "video_encode_ffmpeg.h"
#include <errno.h>
#include <limits.h>
#include <math.h>
#include <stdlib.h>
#include <string.h>
#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
#include <libavutil/imgutils.h>
#include <libavutil/mastering_display_metadata.h>
#include <libavutil/pixdesc.h>
#include <libswscale/swscale.h>
#include <libavformat/avformat.h>
#include <libavutil/display.h>
#include <libswresample/swresample.h>

#define SEREIN_MAX_SIDE 7680
#define SEREIN_MAX_PIXELS ((int64_t)7680 * 4320)
#define SEREIN_MAX_CODED_PIXELS ((int64_t)7680 * 4352)
#define SEREIN_MAX_UNIT ((size_t)8 * 1024 * 1024 + 65536)
#define SEREIN_MAX_PICTURE ((size_t)7680 * 4320 * 4)

typedef struct SereinDecoder {
    AVCodecContext *codec;
    AVCodecContext *header_codec;
    AVCodecParserContext *parser;
    AVFrame *frame, *downloaded;
    AVPacket *packet;
    struct SwsContext *scale;
    enum AVPixelFormat hardware_format;
    int hardware, pending, surface_width, surface_height;
} SereinDecoder;

static int valid_geometry(int width, int height, int coded)
{
    return width > 0 && height > 0 && width <= SEREIN_MAX_SIDE && height <= SEREIN_MAX_SIDE &&
           (int64_t)width * height <= (coded ? SEREIN_MAX_CODED_PIXELS : SEREIN_MAX_PIXELS);
}

static int bounded_buffer(AVCodecContext *codec, AVFrame *frame, int flags)
{
    const AVPixFmtDescriptor *description = av_pix_fmt_desc_get((enum AVPixelFormat)frame->format);
    const SereinDecoder *decoder = codec->opaque;
    if (!valid_geometry(frame->width, frame->height, 1) || !description)
        return AVERROR(EINVAL);
    if (decoder && decoder->surface_width &&
        (frame->width > decoder->surface_width || frame->height > decoder->surface_height))
        return AVERROR(ENOSYS);
    if (!(description->flags & AV_PIX_FMT_FLAG_HWACCEL) &&
        (description->comp[0].depth > 10 || (description->flags & AV_PIX_FMT_FLAG_RGB) ||
         (description->nb_components != 1 &&
          (description->nb_components != 3 || description->log2_chroma_w != 1 || description->log2_chroma_h != 1))))
        return AVERROR(ENOSYS);
    return avcodec_default_get_buffer2(codec, frame, flags);
}

static enum AVPixelFormat choose_format(AVCodecContext *codec, const enum AVPixelFormat *formats)
{
    SereinDecoder *decoder = codec->opaque;
    const int width = codec->coded_width > 0 ? codec->coded_width : codec->width;
    const int height = codec->coded_height > 0 ? codec->coded_height : codec->height;
    if (!valid_geometry(width, height, 1) ||
        (decoder->surface_width && (width > decoder->surface_width || height > decoder->surface_height)))
        return AV_PIX_FMT_NONE;
    for (int i = 0; formats[i] != AV_PIX_FMT_NONE; i++)
        if (formats[i] == decoder->hardware_format)
            return formats[i];
    /* Explicitly decline unsupported hardware profiles; the worker retries
     * this keyframe in software, without opening another physical GPU. */
    return AV_PIX_FMT_NONE;
}

void serein_decode_close(void *pointer)
{
    SereinDecoder *decoder = pointer;
    if (!decoder)
        return;
    sws_freeContext(decoder->scale);
    av_packet_free(&decoder->packet);
    av_frame_free(&decoder->downloaded);
    av_frame_free(&decoder->frame);
    av_parser_close(decoder->parser);
    avcodec_free_context(&decoder->header_codec);
    avcodec_free_context(&decoder->codec);
    free(decoder);
}

static const AVCodec *find_decoder(int codec, int hardware, const SereinVideoAdapter *adapter)
{
    if (codec < 0 || codec > 2)
        return NULL;
#if defined(__linux__)
    if (hardware && adapter && adapter->vendor_id == 0x8086) {
        const char *names[] = {"h264_qsv", "hevc_qsv", "av1_qsv"};
        return avcodec_find_decoder_by_name(names[codec]);
    }
#else
    (void)adapter;
#endif
    const char *names[] = {"h264", "hevc", hardware ? "av1" : "libdav1d"};
    return avcodec_find_decoder_by_name(names[codec]);
}

static void *decoder_open(int codec, const SereinVideoAdapter *adapter, int hardware,
                          const AVCodecParameters *parameters, AVRational time_base)
{
    const AVCodec *implementation = find_decoder(codec, hardware, adapter);
    if (!implementation || (hardware && !serein_video_adapter_valid(adapter)))
        return NULL;
    SereinDecoder *decoder = calloc(1, sizeof(*decoder));
    if (!decoder)
        return NULL;
    decoder->hardware_format = AV_PIX_FMT_NONE;
    if (parameters) {
        decoder->surface_width = (parameters->width + 127) & ~127;
        decoder->surface_height = (parameters->height + 127) & ~127;
    }
    decoder->codec = avcodec_alloc_context3(implementation);
    decoder->header_codec = avcodec_alloc_context3(NULL);
    decoder->parser = av_parser_init(implementation->id);
    decoder->frame = av_frame_alloc();
    decoder->downloaded = av_frame_alloc();
    decoder->packet = av_packet_alloc();
    if (!decoder->codec || !decoder->header_codec || !decoder->parser || !decoder->frame || !decoder->downloaded || !decoder->packet)
        goto failed;
    decoder->parser->flags |= PARSER_FLAG_COMPLETE_FRAMES;
    decoder->header_codec->max_pixels = SEREIN_MAX_CODED_PIXELS;
    if (parameters && (avcodec_parameters_to_context(decoder->header_codec, parameters) < 0 || avcodec_parameters_to_context(decoder->codec, parameters) < 0))
        goto failed;
    decoder->codec->opaque = decoder;
    decoder->codec->max_pixels = SEREIN_MAX_CODED_PIXELS;
    decoder->codec->thread_count = 2;
    decoder->codec->thread_type = FF_THREAD_SLICE;
    decoder->codec->pkt_timebase = time_base;
    decoder->codec->get_buffer2 = bounded_buffer;
    decoder->codec->err_recognition = AV_EF_CAREFUL;
    AVDictionary *options = NULL;
    if (codec == 2 && !hardware) {
        av_dict_set(&options, "max_frame_delay", "2", 0);
    }
    if (hardware) {
        if (!serein_video_bind_decoder(decoder->codec, adapter)) {
            av_dict_free(&options);
            goto failed;
        }
        const AVHWDeviceContext *device = (const AVHWDeviceContext *)decoder->codec->hw_device_ctx->data;
        for (int i = 0;; i++) {
            const AVCodecHWConfig *configuration = avcodec_get_hw_config(implementation, i);
            if (!configuration)
                break;
            if ((configuration->methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX) &&
                configuration->device_type == device->type) {
                decoder->hardware_format = configuration->pix_fmt;
                break;
            }
        }
        if (decoder->hardware_format == AV_PIX_FMT_NONE) {
            av_dict_free(&options);
            goto failed;
        }
        decoder->codec->get_format = choose_format;
        decoder->hardware = 1;
    }
    const int result = avcodec_open2(decoder->codec, implementation, &options);
    av_dict_free(&options);
    if (result < 0)
        goto failed;
    return decoder;
failed:
    serein_decode_close(decoder);
    return NULL;
}

void *serein_decode_open(int codec, const SereinVideoAdapter *adapter, int hardware)
{
    return decoder_open(codec, adapter, hardware, NULL, (AVRational){1, 90000});
}

int serein_decode_hardware(void *pointer)
{
    const SereinDecoder *decoder = pointer;
    return decoder && decoder->hardware;
}

/* Parsers inspect sequence headers without creating decoded surfaces. The
 * decoder worker reserves the worst-case reference footprint before send. */
typedef struct SereinHeader { uint32_t coded_width, coded_height, width, height, depth, chroma, references; } SereinHeader;
int serein_decode_header(void *pointer, const uint8_t *data, size_t bytes, SereinHeader *header)
{
    SereinDecoder *decoder = pointer;
    if (!decoder || !data || !header || !bytes || bytes > SEREIN_MAX_UNIT) return -1;
    memset(header, 0, sizeof(*header));
    av_packet_unref(decoder->packet);
    if (av_new_packet(decoder->packet, (int)bytes) < 0) return -3;
    memcpy(decoder->packet->data, data, bytes);
    uint8_t *parsed = NULL;
    int parsed_bytes = 0;
    int result = av_parser_parse2(decoder->parser, decoder->header_codec, &parsed, &parsed_bytes,
        decoder->packet->data, (int)bytes, AV_NOPTS_VALUE, AV_NOPTS_VALUE, 0);
    av_packet_unref(decoder->packet);
    if (result < 0) return -1;
    const AVCodecParserContext *parser = decoder->parser;
    const AVPixFmtDescriptor *format = av_pix_fmt_desc_get(parser->format);
    if (!parser->width || !parser->height) return 0;
    if (!valid_geometry(parser->width, parser->height, 0)) return -3;
    int width = parser->coded_width ? parser->coded_width : parser->width;
    int height = parser->coded_height ? parser->coded_height : parser->height;
    if (!valid_geometry(width, height, 1)) return -3;
    if (!format || format->comp[0].depth > 10 ||
        (format->nb_components != 1 && (format->nb_components != 3 ||
         format->log2_chroma_w != 1 || format->log2_chroma_h != 1)) || (format->flags & AV_PIX_FMT_FLAG_RGB)) return -2;
    header->coded_width = (uint32_t)width;
    header->coded_height = (uint32_t)height;
    header->width = (uint32_t)parser->width;
    header->height = (uint32_t)parser->height;
    header->depth = (uint32_t)format->comp[0].depth;
    header->chroma = format->nb_components == 1 ? 0 : 1;
    header->references = decoder->codec->codec_id == AV_CODEC_ID_AV1 ? 8 : 16;
    return 1;
}

int serein_decode_send(void *pointer, const uint8_t *data, size_t bytes, int64_t pts)
{
    SereinDecoder *decoder = pointer;
    if (!decoder || !data || !bytes || bytes > SEREIN_MAX_UNIT || bytes > INT_MAX)
        return -3;
    av_packet_unref(decoder->packet);
    if (av_new_packet(decoder->packet, (int)bytes) < 0)
        return -3;
    memcpy(decoder->packet->data, data, bytes);
    decoder->packet->pts = pts;
    decoder->packet->dts = AV_NOPTS_VALUE;
    const int result = avcodec_send_packet(decoder->codec, decoder->packet);
    av_packet_unref(decoder->packet);
    if (result == AVERROR(EAGAIN))
        return 0;
    if (result == AVERROR(ENOSYS))
        return -2;
    if (result == AVERROR(ENOMEM))
        return -3;
    return result < 0 ? -1 : 1;
}

static float peak_nits(const AVFrame *frame)
{
    /* PQ specifies absolute light through 10,000 nits even without MaxCLL.
     * HLG needs the conventional 1,000-nit reference display for its OOTF. */
    const float fallback = frame->color_trc == AVCOL_TRC_SMPTE2084 ? 10000.0f : 1000.0f;
    float peak = fallback;
    const AVFrameSideData *mastering = av_frame_get_side_data(frame, AV_FRAME_DATA_MASTERING_DISPLAY_METADATA);
    if (mastering && mastering->size >= sizeof(AVMasteringDisplayMetadata)) {
        const AVMasteringDisplayMetadata *metadata = (const void *)mastering->data;
        if (metadata->has_luminance)
            peak = (float)av_q2d(metadata->max_luminance);
    }
    const AVFrameSideData *light = av_frame_get_side_data(frame, AV_FRAME_DATA_CONTENT_LIGHT_LEVEL);
    if (light && light->size >= sizeof(AVContentLightMetadata)) {
        const AVContentLightMetadata *metadata = (const void *)light->data;
        if (metadata->MaxCLL)
            peak = (float)metadata->MaxCLL;
    }
    return isfinite(peak) && peak >= 100.0f && peak <= 10000.0f ? peak : fallback;
}

static int matrix_coefficients(enum AVColorSpace matrix)
{
    switch (matrix) {
    case AVCOL_SPC_BT709: return SWS_CS_ITU709;
    case AVCOL_SPC_BT2020_NCL: return SWS_CS_BT2020;
    default: return SWS_CS_ITU601;
    }
}

static unsigned hdr_component(const AVFrame *frame, const AVPixFmtDescriptor *description,
                              int component, int x, int y)
{
    const AVComponentDescriptor *c = &description->comp[component];
    const uint8_t *at = frame->data[c->plane] + (ptrdiff_t)y * frame->linesize[c->plane] +
                        (ptrdiff_t)x * c->step + c->offset;
    unsigned word = at[0];
    if (c->depth + c->shift > 8)
        word = description->flags & AV_PIX_FMT_FLAG_BE ? (word << 8) | at[1] : word | ((unsigned)at[1] << 8);
    word = (word >> c->shift) & ((1u << c->depth) - 1);
    if (frame->color_range == AVCOL_RANGE_JPEG && component == 0)
        return (word * 1023u + ((1u << c->depth) - 1) / 2) / ((1u << c->depth) - 1);
    return word << (10 - c->depth);
}

static void write_p010(uint8_t *at, unsigned sample)
{
    const unsigned value = sample << 6;
    at[0] = (uint8_t)value;
    at[1] = (uint8_t)(value >> 8);
}

int serein_decode_receive(void *pointer, SereinPicture *picture, uint8_t *output, size_t capacity)
{
    SereinDecoder *decoder = pointer;
    if (!decoder || !picture)
        return -1;
    memset(picture, 0, sizeof(*picture));
    if (!decoder->pending) {
        av_frame_unref(decoder->frame);
        av_frame_unref(decoder->downloaded);
        const int result = avcodec_receive_frame(decoder->codec, decoder->frame);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF)
            return 0;
        if (result < 0)
            return result == AVERROR(ENOSYS) ? -2 : result == AVERROR(ENOMEM) ? -3 : -1;
        decoder->pending = 1;
    }
    const AVFrame *frame = decoder->frame;
    const AVPixFmtDescriptor *description = av_pix_fmt_desc_get((enum AVPixelFormat)frame->format);
    if (!description || !valid_geometry(frame->width, frame->height, 0))
        return -3;
    if (description->flags & AV_PIX_FMT_FLAG_HWACCEL) {
        if (!decoder->downloaded->data[0]) {
            if (av_hwframe_transfer_data(decoder->downloaded, frame, 0) < 0 ||
                av_frame_copy_props(decoder->downloaded, frame) < 0)
                return -2;
        }
        frame = decoder->downloaded;
        description = av_pix_fmt_desc_get((enum AVPixelFormat)frame->format);
    }
    if (!description || description->comp[0].depth > 10)
        return -2;
    const int hdr = frame->color_trc == AVCOL_TRC_SMPTE2084 || frame->color_trc == AVCOL_TRC_ARIB_STD_B67;
    if (hdr && ((description->flags & (AV_PIX_FMT_FLAG_RGB | AV_PIX_FMT_FLAG_BITSTREAM)) ||
        description->comp[0].depth < 8 ||
        (description->nb_components != 1 &&
         (description->nb_components != 3 || description->log2_chroma_w != 1 || description->log2_chroma_h != 1))))
        return -2;
    const enum AVPixelFormat format = hdr ? AV_PIX_FMT_P010LE : AV_PIX_FMT_RGBA;
    const int bytes = av_image_get_buffer_size(format, frame->width, frame->height, 1);
    if (bytes <= 0 || (size_t)bytes > SEREIN_MAX_PICTURE)
        return -3;
    picture->width = (uint32_t)frame->width;
    picture->height = (uint32_t)frame->height;
    picture->format = (uint32_t)hdr;
    picture->primaries = (uint32_t)(hdr && frame->color_primaries == AVCOL_PRI_UNSPECIFIED ? AVCOL_PRI_BT2020 : frame->color_primaries);
    picture->transfer = (uint32_t)frame->color_trc;
    picture->matrix = (uint32_t)(hdr && frame->colorspace == AVCOL_SPC_UNSPECIFIED ? AVCOL_SPC_BT2020_NCL : frame->colorspace);
    picture->full_range = frame->color_range == AVCOL_RANGE_JPEG;
    picture->chroma_location = (uint32_t)frame->chroma_location;
    picture->depth = (uint32_t)description->comp[0].depth;
    picture->peak_nits = peak_nits(frame);
    picture->pts = frame->best_effort_timestamp;
    picture->bytes = (size_t)bytes;
    if (!output)
        return capacity == 0 ? 1 : -1;
    if (capacity != picture->bytes)
        return -3;
    uint8_t *planes[4] = {NULL};
    int strides[4] = {0};
    if (av_image_fill_arrays(planes, strides, output, format, frame->width, frame->height, 1) != bytes)
        return -1;
    if (hdr) {
        /* Preserve code values and metadata. swscale's P010 packer leaves the
         * last chroma pair unwritten for odd visible crops; explicit ceil-sized
         * planes also avoid an unnecessary HDR colorspace/range conversion. */
        for (int y = 0; y < frame->height; ++y)
            for (int x = 0; x < frame->width; ++x)
                write_p010(planes[0] + (ptrdiff_t)y * strides[0] + x * 2,
                           hdr_component(frame, description, 0, x, y));
        for (int y = 0; y < (frame->height + 1) / 2; ++y)
            for (int x = 0; x < (frame->width + 1) / 2; ++x) {
                uint8_t *at = planes[1] + (ptrdiff_t)y * strides[1] + x * 4;
                write_p010(at, description->nb_components == 1 ? 512 : hdr_component(frame, description, 1, x, y));
                write_p010(at + 2, description->nb_components == 1 ? 512 : hdr_component(frame, description, 2, x, y));
            }
        decoder->pending = 0;
        return 1;
    }
    decoder->scale = sws_getCachedContext(decoder->scale, frame->width, frame->height,
        (enum AVPixelFormat)frame->format, frame->width, frame->height, format,
        SWS_BILINEAR | SWS_ACCURATE_RND, NULL, NULL, NULL);
    if (!decoder->scale)
        return -3;
    const int *coefficients = sws_getCoefficients(matrix_coefficients((enum AVColorSpace)picture->matrix));
    if (sws_setColorspaceDetails(decoder->scale, coefficients, picture->full_range,
        coefficients, hdr ? picture->full_range : 1, 0, 1 << 16, 1 << 16) < 0 ||
        sws_scale(decoder->scale, (const uint8_t *const *)frame->data, frame->linesize,
                  0, frame->height, planes, strides) != frame->height)
        return -1;
    decoder->pending = 0;
    return 1;
}

/* One pending compressed packet across both tracks prevents demux from building
 * hidden unbounded queues when the player temporarily stops draining audio. */
typedef struct SereinMedia {
    AVFormatContext *format;
    AVIOContext *io;
    SereinDecoder *video;
    AVCodecContext *audio;
    AVPacket *packet;
    AVFrame *audio_frame;
    SwrContext *resample;
    AVChannelLayout resample_layout;
    enum AVSampleFormat resample_format;
    int resample_rate;
    int video_stream, audio_stream, eof, video_drained, audio_drained, audio_pending;
    uint32_t rotation;
    double duration, audio_next_pts;
    int video_started, resample_drained;
} SereinMedia;

static void media_reset_resampler(SereinMedia *media)
{
    swr_free(&media->resample);
    av_channel_layout_uninit(&media->resample_layout);
    media->resample_format = AV_SAMPLE_FMT_NONE;
    media->resample_rate = 0;
}

void serein_media_close(void *pointer)
{
    SereinMedia *media = pointer;
    if (!media) return;
    serein_decode_close(media->video);
    media_reset_resampler(media);
    avcodec_free_context(&media->audio);
    av_frame_free(&media->audio_frame);
    av_packet_free(&media->packet);
    avformat_close_input(&media->format);
    if (media->io) av_freep(&media->io->buffer);
    avio_context_free(&media->io);
    free(media);
}

static int stream_codec(enum AVCodecID id)
{
    switch (id) {
    case AV_CODEC_ID_H264: return 0;
    case AV_CODEC_ID_HEVC: return 1;
    case AV_CODEC_ID_AV1: return 2;
    default: return -1;
    }
}

void *serein_media_open(void *reader, SereinMediaRead read, SereinMediaSeek seek,
                        const SereinVideoAdapter *adapter, SereinMediaInfo *info)
{
    if (!reader || !read || !seek || !info) return NULL;
    memset(info, 0, sizeof(*info));
    SereinMedia *media = calloc(1, sizeof(*media));
    if (!media) return NULL;
    media->video_stream = media->audio_stream = -1;
    uint8_t *buffer = av_malloc(65536);
    if (!buffer) goto failed;
    media->io = avio_alloc_context(buffer, 65536, 0, reader, read, NULL, seek);
    if (!media->io) { av_free(buffer); goto failed; }
    media->format = avformat_alloc_context();
    if (!media->format) goto failed;
    media->format->pb = media->io;
    media->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    media->format->probesize = 1024 * 1024;
    media->format->max_analyze_duration = 500000;
    media->format->max_streams = 8;
    media->format->max_index_size = 1024 * 1024;
    AVDictionary *options = NULL;
    av_dict_set(&options, "format_whitelist", "mov,matroska,webm", 0);
    av_dict_set(&options, "protocol_whitelist", "", 0);
    const int opened = avformat_open_input(&media->format, NULL, NULL, &options);
    av_dict_free(&options);
    if (opened < 0 || !media->format || media->format->nb_streams > 8) goto failed;
    /* MOV/Matroska already supply codec parameters. find_stream_info would
     * speculatively open decoders before the application's surface admission. */
    media->video_stream = av_find_best_stream(media->format, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    if (media->video_stream < 0) goto failed;
    AVStream *video = media->format->streams[media->video_stream];
    AVCodecParameters *parameters = video->codecpar;
    const int codec = stream_codec(parameters->codec_id);
    if (codec < 0 || !valid_geometry(parameters->width, parameters->height, 0) ||
        parameters->extradata_size < 0 || parameters->extradata_size > 65536) goto failed;
    const AVPixFmtDescriptor *pixel = av_pix_fmt_desc_get(parameters->format);
    if (parameters->bits_per_raw_sample > 10 ||
        (pixel && (pixel->comp[0].depth > 10 || (pixel->flags & AV_PIX_FMT_FLAG_RGB) ||
         (pixel->nb_components != 1 && (pixel->nb_components != 3 ||
          pixel->log2_chroma_w != 1 || pixel->log2_chroma_h != 1))))) goto failed;
    media->duration = media->format->duration > 0 ? media->format->duration / (double)AV_TIME_BASE :
        video->duration > 0 ? video->duration * av_q2d(video->time_base) : 0;
    if (!isfinite(media->duration) || media->duration <= 0 || media->duration > 7200) goto failed;
    size_t matrix_bytes = 0;
    const uint8_t *matrix = av_packet_side_data_get(parameters->coded_side_data,
        parameters->nb_coded_side_data, AV_PKT_DATA_DISPLAYMATRIX) ?
        av_packet_side_data_get(parameters->coded_side_data, parameters->nb_coded_side_data,
                               AV_PKT_DATA_DISPLAYMATRIX)->data : NULL;
    const AVPacketSideData *rotation = av_packet_side_data_get(parameters->coded_side_data,
        parameters->nb_coded_side_data, AV_PKT_DATA_DISPLAYMATRIX);
    if (rotation) matrix_bytes = rotation->size;
    if (matrix && matrix_bytes >= 9 * sizeof(int32_t)) {
        double degrees = -av_display_rotation_get((const int32_t *)matrix);
        if (!isfinite(degrees)) goto failed;
        int quarter = (int)llround(degrees / 90.0);
        if (fabs(degrees - quarter * 90.0) > 1.0) goto failed;
        media->rotation = (uint32_t)((quarter % 4 + 4) % 4) * 90;
    }
    media->video = adapter ? decoder_open(codec, adapter, 1, parameters, video->time_base) : NULL;
    if (!media->video) media->video = decoder_open(codec, NULL, 0, parameters, video->time_base);
    if (!media->video) goto failed;
    /* Containers cannot silently replace a reserved surface with a larger one. */
    media->video->codec->max_pixels = (int64_t)((parameters->width + 127) & ~127) *
                                               ((parameters->height + 127) & ~127);
    info->width = (uint32_t)parameters->width;
    info->height = (uint32_t)parameters->height;
    info->depth = codec != 0 || parameters->bits_per_raw_sample > 8 || parameters->profile == FF_PROFILE_H264_HIGH_10 ? 10 : 8;
    info->references = codec == 2 ? 8 : 16;
    info->rotation = media->rotation;
    info->duration = media->duration;
    media->audio_stream = av_find_best_stream(media->format, AVMEDIA_TYPE_AUDIO, -1, -1, NULL, 0);
    if (media->audio_stream >= 0) {
        AVCodecParameters *audio = media->format->streams[media->audio_stream]->codecpar;
        if (audio->ch_layout.nb_channels < 1 || audio->ch_layout.nb_channels > 8 ||
            audio->sample_rate < 8000 || audio->sample_rate > 192000 || audio->extradata_size > 65536) goto failed;
        const AVCodec *implementation = avcodec_find_decoder(audio->codec_id);
        if (!implementation) goto failed;
        media->audio = avcodec_alloc_context3(implementation);
        media->audio_frame = av_frame_alloc();
        if (!media->audio || !media->audio_frame || avcodec_parameters_to_context(media->audio, audio) < 0) goto failed;
        media->audio->thread_count = 1;
        media->audio->max_samples = 65536;
        media->audio->pkt_timebase = media->format->streams[media->audio_stream]->time_base;
        if (avcodec_open2(media->audio, implementation, NULL) < 0) goto failed;
        info->sample_rate = 48000;
    }
    media->packet = av_packet_alloc();
    if (!media->packet) goto failed;
    return media;
failed:
    serein_media_close(media);
    return NULL;
}

/* A first-frame hardware rejection restarts both tracks together. The player
 * receives a restart notification and retires its PCM ring before resuming. */
static int media_software_fallback(SereinMedia *media)
{
    AVStream *stream = media->format->streams[media->video_stream];
    SereinDecoder *software = decoder_open(stream_codec(stream->codecpar->codec_id), NULL, 0,
                                           stream->codecpar, stream->time_base);
    if (!software) return -2;
    software->codec->max_pixels = media->video->codec->max_pixels;
    serein_decode_close(media->video);
    media->video = software;
    if (av_seek_frame(media->format, media->video_stream,
        stream->start_time == AV_NOPTS_VALUE ? 0 : stream->start_time, AVSEEK_FLAG_BACKWARD) < 0) return -1;
    av_packet_unref(media->packet);
    if (media->audio) avcodec_flush_buffers(media->audio);
    media_reset_resampler(media);
    if (media->audio_frame) av_frame_unref(media->audio_frame);
    media->eof = media->video_drained = media->audio_drained = media->audio_pending = 0;
    media->audio_next_pts = 0;
    media->resample_drained = 0;
    return 3;
}

int serein_media_poll(void *pointer, int audio)
{
    SereinMedia *media = pointer;
    if (!media || (audio && !media->audio)) return 2;
    for (int step = 0; step < 16; step++) {
        if (audio) {
            if (media->audio_pending) return 1;
            int result = avcodec_receive_frame(media->audio, media->audio_frame);
            if (result == 0) { media->audio_pending = 1; return 1; }
            if (result == AVERROR_EOF) {
                if (!media->resample_drained && media->resample && swr_get_delay(media->resample, 48000) > 0) { media->audio_pending = 2; return 1; }
                return 2;
            }
            if (result != AVERROR(EAGAIN)) return -1;
        } else {
            SereinPicture picture;
            int result = serein_decode_receive(media->video, &picture, NULL, 0);
            if (result < 0 && media->video->hardware && !media->video_started)
                return media_software_fallback(media);
            if (result != 0) return result;
            if (media->video_drained && !media->packet->data) return 2;
        }
        if (!media->packet->data && !media->eof) {
            int result = av_read_frame(media->format, media->packet);
            if (result == AVERROR_EOF) media->eof = 1;
            else if (result < 0) return -1;
            else if (media->packet->size < 0 || (size_t)media->packet->size > SEREIN_MAX_UNIT) return -3;
        }
        if (media->eof) {
            if (audio && !media->audio_drained) {
                if (avcodec_send_packet(media->audio, NULL) < 0) return -1;
                media->audio_drained = 1;
            } else if (!audio && !media->video_drained) {
                if (avcodec_send_packet(media->video->codec, NULL) < 0) return -1;
                media->video_drained = 1;
            } else return 2;
            continue;
        }
        const int stream = media->packet->stream_index;
        if (stream != media->video_stream && stream != media->audio_stream) {
            av_packet_unref(media->packet);
            continue;
        }
        if (stream != (audio ? media->audio_stream : media->video_stream)) return 0;
        AVCodecContext *codec = audio ? media->audio : media->video->codec;
        int result = avcodec_send_packet(codec, media->packet);
        if (result == AVERROR(EAGAIN)) continue;
        av_packet_unref(media->packet);
        if (result < 0 && !audio && media->video->hardware && !media->video_started)
            return media_software_fallback(media);
        if (result < 0) return -1;
    }
    return 0;
}

int serein_media_video(void *pointer, SereinPicture *picture, uint8_t *output, size_t capacity, double *pts)
{
    SereinMedia *media = pointer;
    if (!media || !pts) return -1;
    const int result = serein_decode_receive(media->video, picture, output, capacity);
    if (result == 1) {
        if (picture->pts == AV_NOPTS_VALUE) return -1;
        *pts = picture->pts * av_q2d(media->format->streams[media->video_stream]->time_base);
        picture->rotation = media->rotation;
        if (output) media->video_started = 1;
        if (!isfinite(*pts) || *pts < -1 || *pts > 7201) return -1;
    }
    return result;
}

int serein_media_audio(void *pointer, float *output, size_t capacity, size_t *written, double *pts)
{
    SereinMedia *media = pointer;
    if (!media || !output || capacity < 65536 || !written || !pts || !media->audio_pending) return -1;
    uint8_t *planes[] = {(uint8_t *)output};
    if (media->audio_pending == 2) {
        int samples = swr_convert(media->resample, planes, 65536, NULL, 0);
        if (samples < 0) return -1;
        *pts = media->audio_next_pts; *written = (size_t)samples;
        media->audio_next_pts += samples/48000.0;
        media->resample_drained = 1;
        media->audio_pending = 0;
        return 1;
    }
    AVFrame *frame = media->audio_frame;
    if (frame->nb_samples < 0 || frame->nb_samples > 65536 || frame->ch_layout.nb_channels < 1 ||
        frame->ch_layout.nb_channels > 8 || frame->sample_rate < 8000 || frame->sample_rate > 192000 ||
        frame->sample_rate != media->audio->sample_rate) return -3;
    if (!av_channel_layout_check(&frame->ch_layout) ||
        av_get_bytes_per_sample((enum AVSampleFormat)frame->format) <= 0) return -2;
    /* A decoder may change channel layout or sample representation mid-stream.
     * Never pass its new plane array to a resampler configured for older input. */
    if (media->resample &&
        (av_channel_layout_compare(&frame->ch_layout, &media->resample_layout) != 0 ||
         frame->format != media->resample_format || frame->sample_rate != media->resample_rate))
        media_reset_resampler(media);
    if (!media->resample) {
        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
        if (av_channel_layout_copy(&media->resample_layout, &frame->ch_layout) < 0) {
            media_reset_resampler(media);
            return -1;
        }
        media->resample_format = (enum AVSampleFormat)frame->format;
        media->resample_rate = frame->sample_rate;
        if (swr_alloc_set_opts2(&media->resample, &stereo, AV_SAMPLE_FMT_FLT, 48000,
                               &frame->ch_layout, frame->format, frame->sample_rate, 0, NULL) < 0 ||
            swr_init(media->resample) < 0) {
            media_reset_resampler(media);
            return -1;
        }
    }
    int64_t delay = swr_get_delay(media->resample, 48000);
    int samples = swr_convert(media->resample, planes, 65536, (const uint8_t **)frame->extended_data, frame->nb_samples);
    if (samples < 0) return -1;
    if (frame->best_effort_timestamp == AV_NOPTS_VALUE) return -1;
    *pts = frame->best_effort_timestamp * av_q2d(media->format->streams[media->audio_stream]->time_base) - delay / 48000.0;
    media->audio_next_pts = *pts + samples / 48000.0;
    *written = (size_t)samples;
    media->audio_pending = 0;
    av_frame_unref(frame);
    return isfinite(*pts) && *pts >= -1 && *pts <= 7201 ? 1 : -1;
}

int serein_media_seek(void *pointer, double seconds)
{
    SereinMedia *media = pointer;
    if (!media || !isfinite(seconds) || seconds < 0 || seconds > media->duration) return -1;
    if (avformat_seek_file(media->format, -1, INT64_MIN, (int64_t)(seconds * AV_TIME_BASE), INT64_MAX, 0) < 0) return -1;
    av_packet_unref(media->packet);
    avcodec_flush_buffers(media->video->codec);
    media->video->pending = 0;
    av_frame_unref(media->video->frame);
    av_frame_unref(media->video->downloaded);
    if (media->audio) avcodec_flush_buffers(media->audio);
    media_reset_resampler(media);
    if (media->audio_frame) av_frame_unref(media->audio_frame);
    media->eof = media->video_drained = media->audio_drained = media->audio_pending = 0;
    media->audio_next_pts = seconds;
    media->resample_drained = 0;
    return 1;
}
