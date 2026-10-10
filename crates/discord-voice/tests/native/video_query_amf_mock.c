/* Synthetic AMF C ABI runtime. The C and C++ SDK interfaces share the same
 * vtable ABI; no AMD driver or physical device is loaded by these tests. */
#include <stdarg.h>
#include <AMF/core/Factory.h>
#include <AMF/components/VideoEncoderVCE.h>
#include <AMF/components/VideoEncoderHEVC.h>
#include <AMF/components/VideoEncoderAV1.h>
#include <assert.h>
#include <stdlib.h>
#include <stdio.h>
#if defined(SEREIN_AMF_SCOPED_FIXTURE)
#include <stdint.h>
#include <AMF/core/VulkanAMF.h>
static uintptr_t physical_device;
#endif
static int context_refs, component_refs, caps_refs, input_refs;
static int context_term, encoder_init, pictures, codec_received;
static AMFContext1 context;
static AMFComponent component;
static AMFCaps caps;
static AMFIOCaps input;
static int is(const char *value) { const char *mode = getenv("AMF_MOCK_MODE"); return mode && !strcmp(mode, value); }
static amf_long context_acquire(AMFContext1 *self) { (void)self; return ++context_refs; }
static amf_long context_release(AMFContext1 *self) { (void)self; assert(context_refs > 0); return --context_refs; }
static AMF_RESULT context_query(AMFContext1 *self, const AMFGuid *iid, void **result) {
    (void)iid; if(is("no-interface")) return AMF_NO_INTERFACE;
    *result = self; context_acquire(self); return AMF_OK;
}
static AMF_RESULT context_terminate(AMFContext1 *self) {
    (void)self; assert(component_refs == 0 && caps_refs == 0 && input_refs == 0);
    ++context_term; return AMF_OK;
}
static AMF_RESULT context_vulkan(AMFContext1 *self, void *device) {
    (void)self;
#if defined(SEREIN_AMF_SCOPED_FIXTURE)
    if (device) {
        const AMFVulkanDevice *gpu = device;
        assert(gpu->cbSizeof == sizeof(*gpu));
        physical_device = (uintptr_t)gpu->hPhysicalDevice;
        assert(physical_device == 1 || physical_device == 2);
        assert((uintptr_t)gpu->hDevice == physical_device + 100);
    }
#else
    assert(device == NULL);
#endif
    if(is("init-failed")) return AMF_FAIL;
    if(is("no-device")) return AMF_NO_DEVICE;
    return AMF_OK;
}
static amf_long component_release(AMFComponent *self) { (void)self; assert(caps_refs == 0 && input_refs == 0); return --component_refs; }
static AMF_RESULT component_init(AMFComponent *self, AMF_SURFACE_FORMAT fmt, amf_int32 w, amf_int32 h) {
    (void)self; (void)fmt; (void)w; (void)h; ++encoder_init; return AMF_FAIL;
}
static AMF_RESULT component_submit(AMFComponent *self, AMFData *data) { (void)self; (void)data; ++pictures; return AMF_FAIL; }
static AMF_RESULT component_caps(AMFComponent *self, AMFCaps **out) {
    (void)self; if(is("caps-failed")) return AMF_FAIL;
    if(is("null-caps")) { *out = NULL; return AMF_OK; }
    *out = &caps; ++caps_refs; return AMF_OK;
}
static amf_long caps_release(AMFCaps *self) { (void)self; assert(input_refs == 0); return --caps_refs; }
static AMF_ACCELERATION_TYPE caps_accel(AMFCaps *self) {
    (void)self;
    if(is("software")) return AMF_ACCEL_SOFTWARE;
    if(is("gpu")) return AMF_ACCEL_GPU;
    if(is("unsupported")) return AMF_ACCEL_NOT_SUPPORTED;
    if(is("bad-accel")) return (AMF_ACCELERATION_TYPE)100;
    return AMF_ACCEL_HARDWARE;
}
static AMF_RESULT caps_input(AMFCaps *self, AMFIOCaps **out) { (void)self; *out = &input; ++input_refs; return AMF_OK; }
static amf_long input_release(AMFIOCaps *self) { (void)self; return --input_refs; }
static amf_int32 input_count(AMFIOCaps *self) { (void)self; return is("bad-count") ? 100000 : 2; }
static AMF_RESULT input_format(AMFIOCaps *self, amf_int32 index, AMF_SURFACE_FORMAT *format, amf_bool *native) {
    (void)self; assert(index >= 0 && index < 2); *native = true;
    if(is("format-failed")) return AMF_FAIL;
    *format = is("no-420") ? AMF_SURFACE_BGRA : index == 1 && !is("no-nv12") ? AMF_SURFACE_NV12 : AMF_SURFACE_YUV420P;
    return AMF_OK;
}
static AMF_RESULT create_context(AMFFactory *self, AMFContext **out) { (void)self; *out = (AMFContext *)&context; ++context_refs; return AMF_OK; }
static AMF_RESULT create_component(AMFFactory *self, AMFContext *ctx, const wchar_t *id, AMFComponent **out) {
    (void)self; assert(ctx == (AMFContext *)&context);
    codec_received = !wcscmp(id, AMFVideoEncoderVCE_AVC) ? 0 : !wcscmp(id, AMFVideoEncoder_HEVC) ? 1 : !wcscmp(id, AMFVideoEncoder_AV1) ? 2 : -1;
    assert(codec_received >= 0);
#if defined(SEREIN_AMF_SCOPED_FIXTURE)
    if(is("scoped") && physical_device == 1 && codec_received != 0)
        return AMF_ENCODER_NOT_PRESENT;
#endif
    if(is("absent")) return AMF_ENCODER_NOT_PRESENT;
    if(is("component-failed")) return AMF_FAIL;
    if(is("null-component")) { *out = NULL; return AMF_OK; }
    *out = &component; ++component_refs; return AMF_OK;
}
static const AMFContext1Vtbl context_vtable = {
    .Acquire=context_acquire, .Release=context_release, .QueryInterface=context_query,
    .Terminate=context_terminate, .InitVulkan=context_vulkan
};
static const AMFComponentVtbl component_vtable = {
    .Release=component_release, .Init=component_init, .SubmitInput=component_submit, .GetCaps=component_caps
};
static const AMFCapsVtbl caps_vtable = {
    .Release=caps_release, .GetAccelerationType=caps_accel, .GetInputCaps=caps_input
};
static const AMFIOCapsVtbl input_vtable = {
    .Release=input_release, .GetNumOfFormats=input_count, .GetFormatAt=input_format
};
static const AMFFactoryVtbl factory_vtable = {.CreateContext=create_context, .CreateComponent=create_component};
static AMFFactory factory = {&factory_vtable};
AMF_RESULT AMFInit(amf_uint64 version, AMFFactory **out) {
    assert(version == AMF_FULL_VERSION);
    context.pVtbl=&context_vtable; component.pVtbl=&component_vtable;
    caps.pVtbl=&caps_vtable; input.pVtbl=&input_vtable;
    *out=&factory; return AMF_OK;
}
__attribute__((destructor)) static void check_cleanup(void) {
    assert(context_refs == 0 && component_refs == 0 && caps_refs == 0 && input_refs == 0);
    assert((context_term == 1 || (is("scoped") && context_term == 0)) && encoder_init == 0 && pictures == 0);
}
