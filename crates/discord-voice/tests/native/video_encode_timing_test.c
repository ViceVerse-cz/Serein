/* Exercise the production shim's delayed/reordered packet ABI without drivers. */
#include <libavcodec/avcodec.h>
int fixture_open(AVCodecContext *codec, const AVCodec *implementation, AVDictionary **options);
void fixture_free(AVCodecContext **codec);
int fixture_send(AVCodecContext *codec, const AVFrame *frame);
int fixture_receive(AVCodecContext *codec, AVPacket *packet);
#define avcodec_open2 fixture_open
#define avcodec_free_context fixture_free
#define avcodec_send_frame fixture_send
#define avcodec_receive_packet fixture_receive
#include "video_encode_ffmpeg.c"
#undef avcodec_send_frame
#undef avcodec_receive_packet
#undef avcodec_open2
#undef avcodec_free_context
#include <assert.h>

static int submitted, emitted, never_output;
static const int64_t decode_order[] = {0, 3, 1, 2};
static int mock_open, opens, frees, bindings, reject_all;
static const SereinVideoAdapter *bound_adapter;

int fixture_open(AVCodecContext *codec, const AVCodec *implementation, AVDictionary **options)
{
    if (!mock_open)
        return avcodec_open2(codec, implementation, options);
    opens++;
    int64_t split = 0, columns = 0;
    av_opt_get_int(codec->priv_data, "split_encode_mode", 0, &split);
    av_opt_get_int(codec->priv_data, "tile_cols", 0, &columns);
    return reject_all || split == 2 || columns == 2 ? AVERROR(ENOSYS) : 0;
}

void fixture_free(AVCodecContext **codec)
{
    if (mock_open && *codec)
        frees++;
    avcodec_free_context(codec);
}

int fixture_send(AVCodecContext *codec, const AVFrame *frame)
{
    (void)codec;
    assert(frame->pts == submitted);
    submitted++;
    return 0;
}

int fixture_receive(AVCodecContext *codec, AVPacket *packet)
{
    (void)codec;
    if (never_output || submitted <= 16 || emitted >= 4)
        return AVERROR(EAGAIN);
    const uint8_t idr[] = {0,0,1,0x67,0x80,0,0,1,0x68,0x80,0,0,1,0x65,0x88};
    const uint8_t delta[] = {0,0,1,0x41,0x88};
    const size_t bytes = emitted ? sizeof(delta) : sizeof(idr);
    assert(av_new_packet(packet, (int)bytes) == 0);
    memcpy(packet->data, emitted ? delta : idr, bytes);
    packet->pts = decode_order[emitted];
    if (!emitted)
        packet->flags |= AV_PKT_FLAG_KEY;
    emitted++;
    return 0;
}

/* Configuration fixtures never identify or load a physical GPU. */
int serein_video_bind_adapter(AVCodecContext *codec, int backend, const SereinVideoAdapter *adapter)
{
    (void)codec; (void)backend;
    assert(mock_open && adapter == bound_adapter);
    bindings++;
    return 1;
}
int serein_nvenc_features(int codec, const SereinVideoAdapter *adapter, int *b, int *lookahead)
{ (void)codec; (void)adapter; *b = 2; *lookahead = 1; return 1; }

static void check_presets(void)
{
    const char *names[] = {"h264_nvenc", "hevc_nvenc", "av1_nvenc"};
    for (int kind = 0; kind < 3; kind++) {
        const AVCodec *implementation = avcodec_find_encoder_by_name(names[kind]);
        if (!implementation)
            continue;
        AVCodecContext *codec = avcodec_alloc_context3(implementation);
        assert(codec);
        SereinAvc encoder = {.codec = codec, .kind = kind, .features = kind ? 3 : 2};
        assert(configure_backend(&encoder, 1));
        assert(codec->refs == (kind ? 0 : 1));
        int64_t value;
        const AVOption *p5 = av_opt_find(codec->priv_data, "p5", "preset", 0, 0);
        assert(p5 && av_opt_get_int(codec->priv_data, "preset", 0, &value) == 0);
        assert(value == p5->default_val.i64);
        assert(av_opt_get_int(codec->priv_data, "rc-lookahead", 0, &value) == 0 && value == 16);
        assert(av_opt_get_int(codec->priv_data, "surfaces", 0, &value) == 0 && value == 24);
        avcodec_free_context(&codec);
    }
    const char *qsv[] = {"h264_qsv", "hevc_qsv", "av1_qsv"};
    for (int kind = 0; kind < 3; kind++) {
        const AVCodec *implementation = avcodec_find_encoder_by_name(qsv[kind]);
        if (!implementation)
            continue;
        AVCodecContext *codec = avcodec_alloc_context3(implementation);
        assert(codec);
        SereinAvc encoder = {.codec = codec, .kind = kind, .features = kind ? 3 : 2};
        assert(configure_backend(&encoder, 4));
        assert(codec->refs == (kind ? 0 : 1));
        avcodec_free_context(&codec);
    }
}

static void check_amf_acceptance(void)
{
    assert(serein_avc_amf_split(NULL) == 0);
    const char *names[] = {"hevc_amf", "av1_amf"};
    for (unsigned int kind = 0; kind < sizeof(names)/sizeof(names[0]); kind++) {
        const AVCodec *implementation = avcodec_find_encoder_by_name(names[kind]);
        if (!implementation)
            continue;
        AVCodecContext *codec = avcodec_alloc_context3(implementation);
        assert(codec);
        const AVOption *accepted = av_opt_find(codec->priv_data, "split_accepted", NULL, 0, 0);
        assert(accepted && accepted->type == AV_OPT_TYPE_BOOL);
        assert(accepted->flags & AV_OPT_FLAG_READONLY);
        /* The request option must not itself be mistaken for acceptance. */
        assert(av_opt_set(codec->priv_data, "split_encode", "1", 0) == 0);
        assert(av_opt_set(codec->priv_data, "split_accepted", "1", 0) < 0);
        SereinAvc encoder = {.codec = codec, .split_requested = 1};
        record_amf_split(&encoder, 3);
        assert(serein_avc_amf_split(&encoder) == 1);
        /* Stand in for the bundled wrapper's private SetProperty result. No
         * encoder is opened and no real GPU/driver is consulted. */
        int result = 1;
        memcpy((uint8_t *)codec->priv_data + accepted->offset, &result, sizeof(result));
        record_amf_split(&encoder, 3);
        assert(serein_avc_amf_split(&encoder) == 2);
        encoder.split_requested = 0;
        record_amf_split(&encoder, 3);
        assert(serein_avc_amf_split(&encoder) == 0);
        encoder.split_requested = 1;
        record_amf_split(&encoder, 1);
        assert(serein_avc_amf_split(&encoder) == 0);
        avcodec_free_context(&codec);
    }
    /* A missing status option, including an older library, cannot report
     * acceptance simply because opening the encoder succeeded. */
    AVCodecContext *codec = avcodec_alloc_context3(avcodec_find_encoder_by_name("libopenh264"));
    assert(codec);
    SereinAvc encoder = {.codec = codec, .split_requested = 1};
    record_amf_split(&encoder, 3);
    assert(serein_avc_amf_split(&encoder) == 1);
    avcodec_free_context(&codec);
}

static void check_splitting(void)
{
    /* width, height, AV1 columns, AV1 rows, AMF request; includes required
     * AV1 tiling below our split threshold, a superblock-area boundary and
     * the AMF 4K boundary in both orientations. */
    const int sizes[][5] = {{1920, 1080, 1, 1, 0}, {2560, 1438, 1, 1, 0}, {2560, 1440, 2, 1, 0},
                           {1440, 2560, 2, 1, 0}, {3838, 2160, 2, 1, 0}, {3840, 2158, 2, 1, 0},
                           {2560, 2160, 2, 1, 0}, {3840, 2160, 2, 1, 1}, {2160, 3840, 2, 1, 1},
                           {2160, 3838, 2, 1, 0}, {2158, 3840, 2, 1, 0}, {7680, 4320, 2, 2, 1},
                           {3840, 1080, 1, 1, 0}, {7680, 1080, 2, 1, 0}, {7296, 2560, 2, 2, 1}};
    const char *names[][3] = {{"hevc_nvenc", "hevc_amf", "hevc_qsv"},
                              {"av1_nvenc", "av1_amf", "av1_qsv"}};
    const int backends[] = {1, 3, 4};
    for (unsigned int s = 0; s < sizeof(sizes)/sizeof(sizes[0]); s++) {
        const int width = sizes[s][0], height = sizes[s][1];
        const int request = width >= 1440 && height >= 1440;
        assert(wants_split(width, height, 1, 1) == request);
        assert(wants_split(width, height, 4, 2) == request);
        assert(split_resolution(width, height, 3) == sizes[s][4]);
        assert(!wants_split(width, height, 1, 0));
        assert(!wants_split(width, height, 0, 1));
        assert(!wants_split(width, height, 2, 1));
#if defined(_WIN32)
        assert(wants_split(width, height, 3, 1) == sizes[s][4]);
        assert(wants_split(width, height, 3, 2) == sizes[s][4]);
#else
        assert(!wants_split(width, height, 3, 1));
#endif
        assert(!wants_split(width, height, 3, 0));
        for (int kind = 1; kind <= 2; kind++) {
            for (unsigned int b = 0; b < sizeof(backends)/sizeof(backends[0]); b++) {
                const AVCodec *implementation = avcodec_find_encoder_by_name(names[kind - 1][b]);
                if (!implementation)
                    continue;
                AVCodecContext *codec = avcodec_alloc_context3(implementation);
                assert(codec);
                codec->width = width; codec->height = height;
                /* Inspect AMF's Windows request with real option tables without
                 * loading its DX11 driver on this offline Linux fixture. */
                const int backend_request = backends[b] == 3 ? split_resolution(width, height, 3) : request;
                SereinAvc encoder = {.codec = codec, .kind = kind, .features = 2, .split_requested = backend_request};
                assert(configure_backend(&encoder, backends[b]));
                int64_t value, rows;
                if (backends[b] == 1) {
                    assert(av_opt_get_int(codec->priv_data, "split_encode_mode", 0, &value) == 0);
                    assert(value == (request ? 2 : 15));
                    assert(av_opt_get_int(codec->priv_data, "rc-lookahead", 0, &value) == 0 && value == 16);
                } else if (backends[b] == 3) {
                    assert(av_opt_get_int(codec->priv_data, "split_encode", 0, &value) == 0 && value == sizes[s][4]);
                    /* A non-split session retains PA/lookahead and its requested
                     * quality. The bundled wrapper disables PA only if a split
                     * request is accepted on a two-engine DX11 device. */
                    assert(av_opt_get_int(codec->priv_data, "preanalysis", 0, &value) == 0 && value == 1);
                    assert(av_opt_get_int(codec->priv_data, "pa_lookahead_buffer_depth", 0, &value) == 0 && value == 16);
                    const AVOption *quality = av_opt_find(codec->priv_data, kind == 1 ? "quality" : "balanced", "quality", 0, 0);
                    assert(quality && av_opt_get_int(codec->priv_data, "quality", 0, &value) == 0);
                    assert(value == quality->default_val.i64);
                } else {
                    assert(av_opt_get_int(codec->priv_data, "tile_cols", 0, &value) == 0);
                    assert(value == (kind == 2 ? sizes[s][2] : request ? 2 : 1));
                    assert(av_opt_get_int(codec->priv_data, "tile_rows", 0, &rows) == 0);
                    assert(rows == (kind == 2 ? sizes[s][3] : 1));
                    assert(av_opt_get_int(codec->priv_data, "look_ahead_depth", 0, &value) == 0 && value == 16);
                }
                avcodec_free_context(&codec);
            }
        }
    }
    /* Refused optional split mode retries once without discarding lookahead,
     * the selected codec, or the selected GPU. No real driver is loaded. */
    static const SereinVideoAdapter adapter = {0};
    bound_adapter = &adapter;
    mock_open = 1;
    for (int backend = 1; backend <= 4; backend += 3) {
        const char *name = backend == 1 ? "hevc_nvenc" : "hevc_qsv";
        if (!avcodec_find_encoder_by_name(name))
            continue;
        opens = frees = bindings = reject_all = 0;
        void *encoder = serein_avc_open_on_adapter(2560, 1440, 30, 8000000, 0,
                                                  backend, 1, 4096, &adapter, 2);
        assert(encoder && opens == 2 && frees == 1 && bindings == 2);
        assert(((SereinAvc *)encoder)->features == 2 && !((SereinAvc *)encoder)->split_requested);
        serein_avc_close(encoder);
        assert(frees == 2);
        reject_all = 1;
        opens = frees = bindings = 0;
        assert(!serein_avc_open_on_adapter(2560, 1440, 30, 8000000, 0,
                                           backend, 1, 4096, &adapter, 2));
        assert(opens == 2 && frees == 2 && bindings == 2);
        opens = frees = bindings = 0;
        assert(!serein_avc_open_on_adapter(1920, 1080, 30, 8000000, 0,
                                           backend, 1, 4096, &adapter, 2));
        assert(opens == 1 && frees == 1 && bindings == 1);
    }
    mock_open = 0;
}

int main(void)
{
    check_presets();
    check_amf_acceptance();
    check_splitting();
    uint8_t input[64 * 64 * 3 / 2] = {0}, output[4096];
    SereinVideoAdapter unknown = {0};
    /* A supplied unidentified renderer must still permit software fallback. */
    void *encoder = serein_avc_open_on_adapter(64, 64, 30, 100000, 0, 0, 0, sizeof(output), &unknown, 0);
    assert(encoder);
    for (int index = 0; index < 20; index++) {
        size_t bytes = 99;
        int keyframe = 99;
        int64_t pts = 99;
        int result = serein_avc_encode_timed(encoder, input, sizeof(input), 0, output, sizeof(output), &bytes, &keyframe, &pts);
        if (index < 16) {
            assert(result == 0 && bytes == 0 && keyframe == 0 && pts == -1);
        } else {
            assert(result == 1 && bytes > 0 && pts == decode_order[index - 16]);
            assert(keyframe == (index == 16));
        }
    }
    serein_avc_close(encoder);
    submitted = emitted = 0;
    never_output = 1;
    encoder = serein_avc_open(64, 64, 30, 100000, 0, 0, 0, sizeof(output));
    assert(encoder);
    for (int index = 0; index <= 48; index++) {
        size_t bytes = 99;
        int keyframe = 99;
        int64_t pts = 99;
        const int result = serein_avc_encode_timed(encoder, input, sizeof(input), 0, output, sizeof(output), &bytes, &keyframe, &pts);
        assert(result == (index < 48 ? 0 : -1));
        assert(bytes == 0 && keyframe == 0 && pts == -1);
    }
    assert(submitted == 48);
    serein_avc_close(encoder);
    assert(!serein_avc_open_on_adapter(64, 64, 30, 100000, 0, 1, 0, sizeof(output), NULL, 1));
    puts("Presets, AMF split acceptance ABI, NVENC/QSV 1440p and AMF 4K split policy, preserved AMF PA/quality, same-GPU retry/cleanup, delayed PTS and 48-picture bound passed");
    return 0;
}
