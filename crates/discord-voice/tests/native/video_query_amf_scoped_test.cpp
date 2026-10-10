/* Exact AMD device handles cross the real scoped query ABI. No real loader,
 * driver, encoder initialization or frame submission occurs. */
#include "video_gpu.h"
#include <cassert>
#include <cstdlib>
#include <cstring>
#include <AMF/core/Factory.h>
#ifdef NDEBUG
#error AMF GPU fixtures require assertions
#endif
extern "C" {
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
void *__real_dlopen(const char *name, int flags);
}

static int devices, released;
extern "C" void *__wrap_dlopen(const char *name, int flags) {
    assert(!std::strcmp(name, AMF_DLL_NAMEA));
    return __real_dlopen(std::getenv("SEREIN_AMF_TEST_RUNTIME"), flags);
}

extern "C" void *__wrap_serein_video_vulkan_device(const SereinVideoAdapter *target) {
    assert(target && target->vendor_id == 0x1002);
    if (target->bus != 1 && target->bus != 2)
        return nullptr;
    AVBufferRef *ref = new AVBufferRef{};
    AVHWDeviceContext *device = new AVHWDeviceContext{};
    AVVulkanDeviceContext *native = new AVVulkanDeviceContext{};
    native->inst = reinterpret_cast<VkInstance>(uintptr_t(1));
    native->phys_dev = reinterpret_cast<VkPhysicalDevice>(uintptr_t(target->bus));
    native->act_dev = reinterpret_cast<VkDevice>(uintptr_t(target->bus + 100));
    device->type = AV_HWDEVICE_TYPE_VULKAN;
    device->hwctx = native;
    ref->data = reinterpret_cast<uint8_t *>(device);
    devices++;
    return ref;
}

extern "C" void av_buffer_unref(AVBufferRef **reference) {
    if (*reference) {
        AVHWDeviceContext *device = reinterpret_cast<AVHWDeviceContext *>((*reference)->data);
        delete static_cast<AVVulkanDeviceContext *>(device->hwctx);
        delete device;
        delete *reference;
        *reference = nullptr;
        released++;
    }
}

int main() {
    SereinVideoAdapter target = {SEREIN_GPU_PCI, 0x1002, 0x744c, 0, 1, 0, 0, 0};
    assert(serein_query_amf_on_adapter(2, &target) == 0);
    target.bus = 2;
    assert(serein_query_amf_on_adapter(2, &target) == 1);
    target.bus = 3;
    assert(serein_query_amf_on_adapter(2, &target) == -1);
    target.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(serein_query_amf_on_adapter(2, &target) == -1);
    assert(devices == 2 && released == 2);
}
