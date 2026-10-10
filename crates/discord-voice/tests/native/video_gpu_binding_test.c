/* Physical GPU binding boundary, using mock sysfs and native device contexts.
 * No actual device is opened, encoder initialized, or picture allocated. */
#define _POSIX_C_SOURCE 200809L
#include "video_gpu.h"
#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#ifdef NDEBUG
#error GPU binding fixtures require assertions
#endif

static int created, derived, released, missing, last_type;
static char last_path[80];

ssize_t __wrap_readlink(const char *path, char *output, size_t capacity)
{
    const char *address = NULL;
    if (!strcmp(path, "/sys/class/drm/renderD128/device"))
        address = "../../../devices/pci0000:00/0000:01:00.0";
    if (!missing && !strcmp(path, "/sys/class/drm/renderD129/device"))
        address = "../../../devices/pci0000:00/0000:02:00.0";
    if (!address)
        return -1;
    const size_t length = strlen(address);
    assert(length < capacity);
    memcpy(output, address, length);
    return (ssize_t)length;
}

static AVBufferRef *context(enum AVHWDeviceType type)
{
    AVBufferRef *ref = calloc(1, sizeof(*ref));
    AVHWDeviceContext *device = calloc(1, sizeof(*device));
    assert(ref && device);
    device->type = type;
    ref->data = (uint8_t *)device;
    return ref;
}

int av_hwdevice_ctx_create(AVBufferRef **output, enum AVHWDeviceType type,
    const char *device, AVDictionary *options, int flags)
{
    assert(device && !options && flags == 0);
    assert(type == AV_HWDEVICE_TYPE_DRM || type == AV_HWDEVICE_TYPE_VAAPI);
    snprintf(last_path, sizeof(last_path), "%s", device);
    last_type = type;
    *output = context(type);
    created++;
    return 0;
}

int av_hwdevice_ctx_create_derived(AVBufferRef **output, enum AVHWDeviceType type,
    AVBufferRef *source, int flags)
{
    assert(source && !flags);
    const AVHWDeviceContext *parent = (const AVHWDeviceContext *)source->data;
    assert((type == AV_HWDEVICE_TYPE_VULKAN && parent->type == AV_HWDEVICE_TYPE_DRM) ||
           (type == AV_HWDEVICE_TYPE_QSV && parent->type == AV_HWDEVICE_TYPE_VAAPI));
    *output = context(type);
    derived++;
    return 0;
}

void av_buffer_unref(AVBufferRef **ref)
{
    if (*ref) {
        free((*ref)->data);
        free(*ref);
        *ref = NULL;
        released++;
    }
}

int av_opt_set_int(void *object, const char *name, int64_t value, int flags)
{
    (void)object; (void)name; (void)value; (void)flags;
    assert(0 && "This fixture has no CUDA or Metal adapter");
    return -1;
}

int main(void)
{
    AVCodecContext codec = {0};
    SereinVideoAdapter target = {SEREIN_GPU_PCI, 0x1002, 0x744c, 0, 2, 0, 0, 0};
    /* Two AMD GPUs of the same model map to different render nodes. */
    assert(serein_video_bind_adapter(&codec, 3, &target));
    assert(!strcmp(last_path, "/dev/dri/renderD129"));
    assert(last_type == AV_HWDEVICE_TYPE_DRM && created == 1 && derived == 1 && released == 1);
    assert(((AVHWDeviceContext *)codec.hw_device_ctx->data)->type == AV_HWDEVICE_TYPE_VULKAN);
    av_buffer_unref(&codec.hw_device_ctx);
    target.bus = 1;
    assert(serein_video_bind_adapter(&codec, 3, &target));
    assert(!strcmp(last_path, "/dev/dri/renderD128"));
    av_buffer_unref(&codec.hw_device_ctx);
    /* Intel must derive QSV from the selected GPU's explicit VA child node. */
    target.vendor_id = 0x8086;
    target.bus = 2;
    assert(serein_video_bind_adapter(&codec, 4, &target));
    assert(!strcmp(last_path, "/dev/dri/renderD129"));
    assert(last_type == AV_HWDEVICE_TYPE_VAAPI);
    assert(((AVHWDeviceContext *)codec.hw_device_ctx->data)->type == AV_HWDEVICE_TYPE_QSV);
    av_buffer_unref(&codec.hw_device_ctx);
    const int previous = created;
    missing = 1;
    assert(!serein_video_bind_adapter(&codec, 4, &target));
    assert(created == previous && !codec.hw_device_ctx);
    target.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(!serein_video_bind_adapter(&codec, 4, &target));
    assert(!serein_video_bind_adapter(&codec, 4, NULL));
    assert(created == previous && created + derived == released);
    puts("Exact GPU binding: identical AMD/Intel models use selected render node; missing identities fail closed; all contexts released");
}
