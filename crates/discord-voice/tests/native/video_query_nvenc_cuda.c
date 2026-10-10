/* Offline CUDA ABI fixture, loaded from the runner's temporary directory.
 * These opaque mock contexts never access a GPU or allocate a surface. */
#include <ffnvcodec/dynlink_cuda.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>

static CUcontext current;
static int created, destroyed;
static int mode(const char *value) { return strcmp(getenv("SEREIN_QUERY_FIXTURE"), value) == 0; }
void fixture_cuda_reset(void) { created = destroyed = 0; current = NULL; }
int fixture_cuda_created(void) { return created; }
int fixture_cuda_destroyed(void) { return destroyed; }
CUresult CUDAAPI cuInit(unsigned int flags) {
    (void)flags;
    return mode("init-no-device") ? (CUresult)100 : mode("init-failed") ? CUDA_ERROR_UNKNOWN : CUDA_SUCCESS;
}
CUresult CUDAAPI cuDeviceGetCount(int *count) {
    if (mode("count-failed")) return CUDA_ERROR_UNKNOWN;
    *count = mode("no-device") ? 0 : mode("bound") ? 33 : (mode("multi") || mode("error-then-supported")) ? 2 : 1;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuDeviceGet(CUdevice *device, int ordinal) {
    *device = ordinal;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuDeviceGetPCIBusId(char *address, int capacity, CUdevice device) {
    if (mode("identity-error")) return CUDA_ERROR_UNKNOWN;
    snprintf(address, (size_t)capacity, "0000:%02x:00.0", device + 1);
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuCtxCreate_v2(CUcontext *context, unsigned int flags, CUdevice device) {
    (void)flags;
    if (mode("context-failed")) return CUDA_ERROR_UNKNOWN;
    *context = (CUcontext)(uintptr_t)(device + 1);
    current = *context;
    created++;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuCtxPopCurrent_v2(CUcontext *context) {
    *context = current;
    current = NULL;
    return CUDA_SUCCESS;
}
CUresult CUDAAPI cuCtxDestroy_v2(CUcontext context) {
    (void)context;
    destroyed++;
    return CUDA_SUCCESS;
}
