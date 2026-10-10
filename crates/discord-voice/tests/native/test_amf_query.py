#!/usr/bin/env python3
"""Exercise AMF driver discovery against an offline synthetic SDK runtime."""

import argparse
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile


def run(include_path):
    if sys.platform != "linux":
        print("AMF native mock tests require Linux's dlopen wrapper; skipped.")
        return
    include = Path(include_path).resolve()
    if not (include / "AMF/core/Factory.h").is_file():
        raise RuntimeError(f"AMF SDK headers are missing from {include}")
    fixture_dir = Path(__file__).resolve().parent
    source_dir = fixture_dir.parent.parent / "src"
    c_compiler = shlex.split(os.environ.get("CC", "cc"))
    cpp_compiler = shlex.split(os.environ.get("CXX", "c++"))
    sdk_flags = ["-isystem", str(include)]
    warnings = ["-Wall", "-Wextra", "-Werror", "-ffunction-sections", "-fdata-sections", "-Wl,--gc-sections"]
    modes = {
        "support": 1,
        "software": 0,
        "gpu": 0,
        "unsupported": 0,
        "bad-accel": -1,
        "caps-failed": -1,
        "null-caps": -1,
        "bad-count": -1,
        "format-failed": -1,
        "no-nv12": 1,
        "no-420": 0,
        "absent": 0,
        "component-failed": -1,
        "null-component": -1,
        "init-failed": -1,
        "no-device": 0,
        "no-interface": -1,
    }
    with tempfile.TemporaryDirectory(prefix="serein-amf-query-") as temp:
        directory = Path(temp)
        library = directory / "mock-amf.so"
        executable = directory / "query-test"
        subprocess.run(
            c_compiler + ["-std=c11"] + warnings + sdk_flags + [
                "-fPIC", "-shared", str(fixture_dir / "video_query_amf_mock.c"),
                "-o", str(library),
            ],
            check=True,
        )
        subprocess.run(
            cpp_compiler + ["-std=c++11"] + warnings + sdk_flags + [
                str(source_dir / "video_query_amf.cpp"),
                str(fixture_dir / "video_query_amf_test.cpp"),
                "-Wl,--wrap=dlopen", "-ldl", "-o", str(executable),
            ],
            check=True,
        )
        environment = {**os.environ, "SEREIN_AMF_TEST_RUNTIME": str(library)}
        for codec in range(3):
            for mode, expected in modes.items():
                subprocess.run(
                    [str(executable), str(codec), str(expected)],
                    env={**environment, "AMF_MOCK_MODE": mode},
                    check=True,
                    timeout=5,
                )
        for codec in [-1, 3]:
            subprocess.run(
                [str(executable), str(codec), "-1"],
                env=environment,
                check=True,
                timeout=5,
            )
        missing_library = directory / "absent-amf.so"
        subprocess.run(
            [str(executable), "0", "0"],
            env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(missing_library)},
            check=True,
            timeout=5,
        )
        invalid_library = directory / "invalid-amf.so"
        invalid_library.write_text("This fixture is deliberately not a shared library.\n")
        subprocess.run(
            [str(executable), "0", "-1"],
            env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(invalid_library)},
            check=True,
            timeout=5,
        )
        no_api_source = directory / "no-api.c"
        no_api_source.write_text("int serein_mock_no_api(void) { return 0; }\n")
        no_api_library = directory / "no-api-amf.so"
        subprocess.run(
            c_compiler + ["-std=c11"] + warnings + [
                "-fPIC", "-shared", str(no_api_source), "-o", str(no_api_library),
            ],
            check=True,
        )
        subprocess.run(
            [str(executable), "0", "-1"],
            env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(no_api_library)},
            check=True,
            timeout=5,
        )
        if (include / "vulkan/vulkan.h").is_file():
            scoped_library = directory / "scoped-amf.so"
            scoped_executable = directory / "scoped-query-test"
            gpu_object = directory / "gpu.o"
            subprocess.run(c_compiler + ["-std=c11"] + warnings + sdk_flags + [
                "-DSEREIN_AMF_SCOPED_FIXTURE=1", "-fPIC", "-shared",
                str(fixture_dir / "video_query_amf_mock.c"), "-o", str(scoped_library),
            ], check=True)
            subprocess.run(c_compiler + ["-std=c11"] + warnings + sdk_flags + [
                "-c", str(source_dir / "video_gpu.c"), "-o", str(gpu_object),
            ], check=True)
            subprocess.run(cpp_compiler + ["-std=c++11"] + warnings + sdk_flags + [
                "-I" + str(source_dir), "-DSEREIN_HAVE_VULKAN_GPU=1",
                str(source_dir / "video_query_amf.cpp"),
                str(fixture_dir / "video_query_amf_scoped_test.cpp"), str(gpu_object),
                "-Wl,--wrap=dlopen", "-Wl,--wrap=serein_video_vulkan_device", "-ldl",
                "-o", str(scoped_executable),
            ], check=True)
            subprocess.run([str(scoped_executable)],
                env={**environment, "SEREIN_AMF_TEST_RUNTIME": str(scoped_library), "AMF_MOCK_MODE": "scoped"},
                timeout=5, check=True)
            print("AMF scoped query: identical models retain exact Vulkan handles; missing targets stay unknown; all objects released")
        print(
            "AMF query: 51 capability cases, invalid codecs, missing/bad runtime "
            "and missing ABI entry point passed; no encoder Init or submitted frames."
        )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--include", required=True, help="Prefix's SDK include directory")
    run(parser.parse_args().include)
