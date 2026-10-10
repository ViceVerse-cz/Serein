/* Pure offline identities: no hardware library is loaded or media opened. */
#include "video_gpu.h"
#include <assert.h>
#include <stdio.h>
#ifdef NDEBUG
#error GPU identity fixtures require assertions
#endif

int main(void)
{
    SereinVideoAdapter first = {SEREIN_GPU_PCI, 0x10de, 0x2684, 0, 1, 0, 0, 0};
    SereinVideoAdapter second = first;
    assert(serein_video_adapter_valid(&first));
    assert(serein_video_adapter_equal(&first, &second));
    first.domain = second.domain = 0x10000;
    assert(serein_video_adapter_valid(&first));
    assert(serein_video_adapter_equal(&first, &second));
    second.bus = 2;
    assert(!serein_video_adapter_equal(&first, &second));
    second = first;
    second.function = 1;
    assert(!serein_video_adapter_equal(&first, &second));
    second = first;
    second.device_id++;
    assert(!serein_video_adapter_equal(&first, &second));
    second = first;
    second.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(!serein_video_adapter_valid(&second));
    assert(!serein_video_adapter_equal(&second, &second));
    assert(!serein_video_adapter_valid(NULL));
    assert(!serein_video_adapter_equal(NULL, &first));
    second = first;
    second.slot = 32;
    assert(!serein_video_adapter_valid(&second));
    second = first;
    second.function = 8;
    assert(!serein_video_adapter_valid(&second));
    second = first;
    second.identity = SEREIN_GPU_WINDOWS_LUID;
    assert(!serein_video_adapter_valid(&second));
    second.value = 123;
    first = second;
    assert(serein_video_adapter_equal(&first, &second));
    second.value = 124;
    assert(!serein_video_adapter_equal(&first, &second));
    first.identity = SEREIN_GPU_METAL_REGISTRY;
    second = first;
    assert(serein_video_adapter_equal(&first, &second));
    second.value++;
    assert(!serein_video_adapter_equal(&first, &second));
    puts("Physical GPU identity: same-vendor and same-model GPUs stay distinct; absent/invalid identities reject hardware");
}
