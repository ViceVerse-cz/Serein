#ifndef SEREIN_VIDEO_ENCODE_FFMPEG_H
#define SEREIN_VIDEO_ENCODE_FFMPEG_H

#include <stddef.h>
#include <stdint.h>
#ifndef SEREIN_VIDEO_GPU_H
#define SEREIN_VIDEO_GPU_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Must match model::voice_settings::EncoderPreset. Zero retains the existing
 * vendor default. Named presets are only exposed after a driver query. */
static inline const char *serein_video_preset_name(int backend, int preset)
{
    static const char *const nvenc[] = {"p1", "p2", "p3", "p4", "p5", "p6", "p7"};
    static const char *const amf[] = {"speed", "balanced", "quality", "high_quality"};
    static const char *const qsv[] = {"veryslow", "slower", "slow", "medium", "fast", "faster", "veryfast"};
    if (backend == 1 && preset >= 1 && preset <= 7) return nvenc[preset - 1];
    if (backend == 3 && preset >= 8 && preset <= 11) return amf[preset - 8];
    if (backend == 4 && preset >= 12 && preset <= 18) return qsv[preset - 12];
    return NULL;
}

/* Physical renderer identity, never a vendor-only or performance preference.
 * All fields use fixed-width integers so Rust and native helper ABIs agree. */
typedef struct SereinVideoAdapter {
    uint32_t identity;
    uint32_t vendor_id;
    uint32_t device_id;
    uint32_t domain;
    uint32_t bus;
    uint32_t slot;
    uint32_t function;
    uint64_t value;
} SereinVideoAdapter;

enum {
    SEREIN_GPU_UNIDENTIFIED = 0,
    SEREIN_GPU_WINDOWS_LUID = 1,
    SEREIN_GPU_PCI = 2,
    SEREIN_GPU_METAL_REGISTRY = 3
};

typedef struct SereinCaptureLuminance {
    uint32_t unit_nits, white_nits, peak_nits;
} SereinCaptureLuminance;
int serein_capture_luminance(uint64_t source, int window, SereinCaptureLuminance *output);

struct AVCodecContext;
int serein_video_adapter_valid(const SereinVideoAdapter *adapter);
int serein_video_adapter_equal(const SereinVideoAdapter *a, const SereinVideoAdapter *b);
int serein_video_cuda_device(const SereinVideoAdapter *adapter);
int serein_video_dxgi_device(const SereinVideoAdapter *adapter);
int serein_video_drm_device(const SereinVideoAdapter *adapter, char *path, size_t capacity);
/* Returns a +1 CFDictionary on macOS; the caller must release it. */
void *serein_video_vt_specification(const SereinVideoAdapter *adapter);
/* Returns a retained AVBufferRef to a Vulkan device derived from the exact
 * physical DRM node, or NULL. No encoder or input surface is initialized. */
void *serein_video_vulkan_device(const SereinVideoAdapter *adapter, const char *extensions);
void *serein_video_amf_device(const SereinVideoAdapter *adapter);
/* NULL means the legacy standalone/example path. A supplied target must be
 * bound exactly or fail, so another physical GPU is never selected silently. */
int serein_video_bind_adapter(struct AVCodecContext *codec, int backend,
                             const SereinVideoAdapter *adapter);
int serein_video_bind_decoder(struct AVCodecContext *codec,
                              const SereinVideoAdapter *adapter);
/* Codec queries return 1/0 for support, -1 for errors, or -2 when Linux AMF
 * cannot load its runtime. Preset queries return a bit mask. */
int serein_video_query_on_adapter(int backend, int codec, const SereinVideoAdapter *adapter);
int serein_query_nvenc_on_adapter(int codec, const SereinVideoAdapter *adapter);
int serein_query_qsv_on_adapter(int codec, const SereinVideoAdapter *adapter);
int serein_query_amf_on_adapter(int codec, const SereinVideoAdapter *adapter);
int serein_query_videotoolbox_on_adapter(int codec, const SereinVideoAdapter *adapter);
/* Startup capability metadata only; no pictures or synthetic encoding. */
int serein_nvenc_features(int codec, const SereinVideoAdapter *adapter,
                          int *max_b_frames, int *lookahead);

#ifdef __cplusplus
}
#endif
#endif


#ifdef __cplusplus
extern "C" {
#endif

/* The returned encoder must be used and closed on its owning media worker.
 * Backend: 0 = OpenH264, 1 = NVENC, 2 = VideoToolbox (hardware required),
 * 3 = AMD AMF, 4 = Intel QSV (hardware-only D3D11/VA driver session).
 * baseline selects independent test pictures (GOP 1); FFmpeg
 * camera and screen pictures use Main profile and a two-second GOP. Codec: 0 H.264, 1 HEVC,
 * 2 AV1. Software is provided only for H.264. NULL means unavailable. */
void *serein_avc_open(int width, int height, int fps, int bitrate, int minimum, int maximum, int preset, int baseline,
                     int backend, int codec, size_t max_bytes);
/* Features: bit 0 requests up to two B frames (HEVC/AV1 only); bit 1 requests
 * sixteen-frame lookahead. Unsupported combinations fail without switching GPU. */
void *serein_avc_open_on_adapter(int width, int height, int fps, int bitrate, int minimum, int maximum, int preset,
                                int baseline, int backend, int codec,
                                size_t max_bytes, const SereinVideoAdapter *adapter,
                                int features);
/* Surface APIs accept only a trusted, completed D3D11 NV12 capture AVFrame.
 * The context retains its frames/device binding, never the picture itself.
 * Unsupported or physically mismatched devices fail without switching GPUs. */
void *serein_avc_surface_context(const void *frame);
void serein_avc_surface_context_free(void *context);
void *serein_avc_open_on_surface(int width, int height, int fps, int bitrate, int minimum, int maximum, int preset,
                                int baseline, int backend, int codec,
                                size_t max_bytes, const SereinVideoAdapter *adapter,
                                int features, void *frames_context);
void serein_avc_close(void *encoder);
/* Private, process-local interrupt ABI. The callback and opaque owner must
 * outlive encoder close; it may run on a native encoder thread. Never pass
 * account data or assume the layout of a Rust atomic in C. */
typedef struct SereinVideoInterruptV1 {
    uint32_t version, size;
    int (*cancelled)(void *opaque);
    void *opaque;
} SereinVideoInterruptV1;
void *serein_avc_open_on_adapter_controlled(int width, int height, int fps, int bitrate,
    int minimum, int maximum, int preset, int baseline, int backend, int codec,
    size_t max_bytes, const SereinVideoAdapter *adapter, int features,
    const SereinVideoInterruptV1 *interrupt);
void *serein_avc_open_on_surface_controlled(int width, int height, int fps, int bitrate,
    int minimum, int maximum, int preset, int baseline, int backend, int codec,
    size_t max_bytes, const SereinVideoAdapter *adapter, int features, void *frames_context,
    const SereinVideoInterruptV1 *interrupt);
/* Timed encode/receive return -2 for cancellation, -3 for no-output timeout.
 * Both are terminal for this share, never instructions to try another encoder. */
/* AMF split request on this live session: 0 = off, 1 = declined, 2 = accepted.
 * Accepted means SetProperty succeeded and the encoder opened successfully.
 * A successful retry without split remains declined. This is diagnostic metadata. */
int serein_avc_amf_split(void *encoder);

/* Input is exactly width * height * 3 / 2 contiguous limited-range BT.601
 * I420 bytes, preserving the callers' SDR sRGB primaries/transfer. All pointers
 * must refer to live nonoverlapping storage of the indicated lengths. The
 * output buffer is written only after the complete packet passes its bounds
 * and Annex B / sized OBU checks. Returns 1 for a packet, 0 for pending output, or -1 for
 * failure. Output length/keyframe are reset even when encoding fails. */
int serein_avc_encode(void *encoder, const uint8_t *contiguous_i420,
                      size_t length, int force_keyframe, uint8_t *output,
                      size_t output_capacity, size_t *output_length,
                      int *keyframe);
int serein_avc_encode_timed(void *encoder, const uint8_t *contiguous_i420,
                            size_t length, int force_keyframe, uint8_t *output,
                            size_t output_capacity, size_t *output_length,
                            int *keyframe, int64_t *presentation_index);

/* Surface submission and output-only polling retain the byte, packet and
 * presentation-timestamp validation of the host-picture API above. */
/* Nonblocking identity check; equal dimensions alone do not establish compatibility. */
int serein_avc_accepts_surface(void *encoder, const void *frame);
int serein_avc_encode_surface_timed(void *encoder, const void *frame,
                                    int force_keyframe, uint8_t *output,
                                    size_t output_capacity, size_t *output_length,
                                    int *keyframe, int64_t *presentation_index);
int serein_avc_receive_timed(void *encoder, uint8_t *output,
                            size_t output_capacity, size_t *output_length,
                            int *keyframe, int64_t *presentation_index);

/* Decoded pixels remain on the owning media worker. SDR is packed RGBA8;
 * PQ/HLG uses tightly packed P010 (Y followed by interleaved UV), preserving
 * ten-bit samples and color metadata for display conversion. */
typedef struct SereinPicture {
    uint32_t width, height, format, primaries, transfer, matrix, full_range, depth;
    uint32_t rotation;
    float peak_nits;
    int64_t pts;
    size_t bytes;
    uint32_t chroma_location;
} SereinPicture;

/* Codec 0=H264, 1=HEVC, 2=AV1. hardware requires an exact adapter identity;
 * unavailable hardware returns NULL so the caller can reopen in software. */
void *serein_decode_open(int codec, const SereinVideoAdapter *adapter, int hardware);
void serein_decode_close(void *decoder);
int serein_decode_hardware(void *decoder);
/* 1=accepted/picture, 0=needs output/input, -1=malformed, -2=unsupported,
 * -3=resource limit. Receive(NULL,0) inspects one retained picture; copying it
 * requires exactly the reported capacity and consumes that picture. */
int serein_decode_send(void *decoder, const uint8_t *data, size_t bytes, int64_t pts);
int serein_decode_receive(void *decoder, SereinPicture *picture, uint8_t *output, size_t capacity);

/* Anonymous bounded reader only. Callbacks never receive a URL or credentials. */
typedef int (*SereinMediaRead)(void *reader, uint8_t *output, int capacity);
typedef int64_t (*SereinMediaSeek)(void *reader, int64_t offset, int whence);
typedef struct SereinMediaInfo {
    uint32_t width, height, depth, references, sample_rate, rotation;
    double duration;
} SereinMediaInfo;
void *serein_media_open(void *reader, SereinMediaRead read, SereinMediaSeek seek,
                        const SereinVideoAdapter *adapter, SereinMediaInfo *info);
void serein_media_close(void *media);
/* poll: 1=sample ready, 0=pending, 2=EOF, -4=compressed queue budget exceeded.
 * Both tracks together retain at most 8 MiB / 512 compressed packets. */
int serein_media_poll(void *media, int audio);
int serein_media_video(void *media, SereinPicture *picture, uint8_t *output, size_t capacity, double *pts);
int serein_media_audio(void *media, float *output, size_t frames, size_t *written, double *pts);
int serein_media_seek(void *media, double seconds);

/* Audio packets are demuxed and size-checked by the application's anonymous reader.
 * Return 0 only for an unavailable decoder; malformed parameters return negative. */
int serein_audio_open(const char *name, uint32_t rate, uint32_t channels,
                      const uint8_t *extra, size_t extra_size, void **output);
void serein_audio_close(void *audio);
int serein_audio_decode(void *audio, const uint8_t *packet, size_t bytes,
                        float *output, size_t capacity, size_t *written);

#ifdef __cplusplus
}
#endif

#endif
