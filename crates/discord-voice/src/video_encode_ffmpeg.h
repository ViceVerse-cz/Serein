#ifndef SEREIN_VIDEO_ENCODE_FFMPEG_H
#define SEREIN_VIDEO_ENCODE_FFMPEG_H

#include <stddef.h>
#include <stdint.h>
#include "video_gpu.h"

#ifdef __cplusplus
extern "C" {
#endif

/* The returned encoder must be used and closed on its owning media worker.
 * Backend: 0 = OpenH264, 1 = NVENC, 2 = VideoToolbox (hardware required),
 * 3 = AMD AMF, 4 = Intel QSV (hardware-only D3D11/VA driver session).
 * baseline selects independent Stable/test pictures (GOP 1); Experimental
 * camera and screen pictures use Main profile and a two-second GOP. Codec: 0 H.264, 1 HEVC,
 * 2 AV1. Software is provided only for H.264. NULL means unavailable. */
void *serein_avc_open(int width, int height, int fps, int bitrate, int baseline,
                     int backend, int codec, size_t max_bytes);
/* Features: bit 0 requests up to two B frames (HEVC/AV1 only); bit 1 requests
 * sixteen-frame lookahead. Unsupported combinations fail without switching GPU. */
void *serein_avc_open_on_adapter(int width, int height, int fps, int bitrate,
                                int baseline, int backend, int codec,
                                size_t max_bytes, const SereinVideoAdapter *adapter,
                                int features);
void serein_avc_close(void *encoder);
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

#ifdef __cplusplus
}
#endif

#endif
