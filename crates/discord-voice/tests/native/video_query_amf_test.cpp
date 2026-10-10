#include <cstdlib>
#include <cassert>
#include <cstring>
#include <AMF/core/Factory.h>

extern "C" int serein_query_amf(int codec);
extern "C" void *__real_dlopen(const char *name, int flags);

/* Link with --wrap=dlopen to keep every case offline, including a missing
 * runtime. Falling through the host library search could load a real driver. */
extern "C" void *__wrap_dlopen(const char *name, int flags) {
    assert(std::strcmp(name, AMF_DLL_NAMEA) == 0);
    const char *fixture = std::getenv("SEREIN_AMF_TEST_RUNTIME");
    assert(fixture);
    return __real_dlopen(fixture, flags);
}

int main(int argc, char **argv) {
    assert(argc == 3);
    const int result = serein_query_amf(std::atoi(argv[1]));
    assert(result == std::atoi(argv[2]));
}
