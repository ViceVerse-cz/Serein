#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <assert.h>
#include "video_gpu.h"
#ifdef NDEBUG
#error Driver query fixtures require assertions enabled
#endif
int serein_query_nvenc(int codec);
void fixture_cuda_reset(void);
int fixture_cuda_created(void);
int fixture_cuda_destroyed(void);
void fixture_nv_reset(void);
int fixture_nv_opened(void);
int fixture_nv_closed(void);
int fixture_nv_forbidden(void);
int main(void) {
    const struct { const char *mode; int codec, expected, contexts; } cases[] = {
        {"normal", 0, 1, 1}, {"normal", 1, 0, 1}, {"normal", 2, 0, 1},
        {"multi", 2, 1, 2}, {"error-then-supported", 2, 1, 2},
        {"no-device", 0, 0, 0}, {"init-no-device", 0, 0, 0},
        {"init-failed", 0, -1, 0}, {"count-failed", 0, -1, 0},
        {"context-failed", 0, -1, 0}, {"open-unavailable", 0, 0, 1},
        {"open-error", 0, -1, 1}, {"guid-error", 0, -1, 1},
        {"zero-guids", 0, 0, 1}, {"excess-guids", 0, -1, 1},
        {"short-guids", 0, -1, 1}, {"caps-error", 0, -1, 1},
        {"zero-dimensions", 0, 0, 1}, {"close-error", 0, -1, 1},
        {"old-api", 0, -1, 0}, {"missing-caps", 0, -1, 0},
        {"bound", 2, -1, 32}, {"normal", -1, 0, 0}, {"normal", 3, 0, 0},
    };
    for (unsigned int i = 0; i < sizeof(cases)/sizeof(cases[0]); i++) {
        assert(setenv("SEREIN_QUERY_FIXTURE", cases[i].mode, 1) == 0);
        fixture_cuda_reset(); fixture_nv_reset();
        int result = serein_query_nvenc(cases[i].codec);
        if (result != cases[i].expected) {
            fprintf(stderr, "%s codec%d expected%d got%d\n", cases[i].mode, cases[i].codec, cases[i].expected, result);
            return 1;
        }
        assert(fixture_cuda_created() == cases[i].contexts);
        assert(fixture_cuda_destroyed() == fixture_cuda_created());
        assert(fixture_nv_closed() == fixture_nv_opened());
        assert(fixture_nv_forbidden() == 0);
    }
    SereinVideoAdapter target = {SEREIN_GPU_PCI, 0x10de, 0x2684, 0, 1, 0, 0, 0};
    assert(setenv("SEREIN_QUERY_FIXTURE", "multi", 1) == 0);
    fixture_cuda_reset(); fixture_nv_reset();
    /* First GPU has H264 only, second (identical model) has AV1. Never
     * borrow a positive answer from the second GPU for the first target. */
    assert(serein_query_nvenc_on_adapter(2, &target) == 0);
    assert(fixture_cuda_created() == 1 && fixture_cuda_destroyed() == 1);
    target.bus = 2;
    fixture_cuda_reset(); fixture_nv_reset();
    assert(serein_query_nvenc_on_adapter(2, &target) == 1);
    assert(fixture_cuda_created() == 1 && fixture_cuda_destroyed() == 1);
    int b_frames = 0, lookahead = 0;
    assert(serein_nvenc_features(2, &target, &b_frames, &lookahead) == 1);
    assert(b_frames == 3 && lookahead == 1);
    assert(fixture_nv_forbidden() == 0);
    target.bus = 3;
    fixture_cuda_reset(); fixture_nv_reset();
    assert(serein_query_nvenc_on_adapter(2, &target) == -1);
    assert(fixture_cuda_created() == 0 && fixture_nv_opened() == 0);
    target.identity = SEREIN_GPU_UNIDENTIFIED;
    assert(serein_query_nvenc_on_adapter(2, &target) == -1);
    assert(fixture_cuda_created() == 0 && fixture_nv_forbidden() == 0);
    printf("NVENC query: %zu offline driver-boundary cases passed, contexts/sessions released, no encoding calls\n", sizeof(cases)/sizeof(cases[0]));
}
