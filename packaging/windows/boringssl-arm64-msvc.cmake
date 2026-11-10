# CMake toolchain file for BoringSSL (btls-sys) on native Windows ARM64 hosts.
#
# BoringSSL's ARM64 assembly is GNU-syntax `.S`, which the Visual Studio generator does not
# assemble, so the link fails on the missing objects. btls-sys disables assembly only when
# cross-compiling to Windows; this applies the same setting to the native build.
set(OPENSSL_NO_ASM YES CACHE BOOL "" FORCE)
# btls-sys skips its own CRT selection when a toolchain file is given; keep the release DLL CRT.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
