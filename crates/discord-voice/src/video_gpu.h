#ifndef SEREIN_VIDEO_GPU_H
#define SEREIN_VIDEO_GPU_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

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
void *serein_video_vulkan_device(const SereinVideoAdapter *adapter);
/* NULL means the legacy standalone/example path. A supplied target must be
 * bound exactly or fail, so another physical GPU is never selected silently. */
int serein_video_bind_adapter(struct AVCodecContext *codec, int backend,
                             const SereinVideoAdapter *adapter);
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
