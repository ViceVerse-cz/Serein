#!/bin/sh
# Check an ARM64 Serein executable from a macOS release build (requires macOS nm).
set -eu

if [ "$#" -ne 1 ]; then
	printf 'Usage: %s path/to/Serein.app/Contents/MacOS/serein\n' "$0" >&2
	exit 2
fi

imports=$(LC_ALL=C nm -m -u "$1")
if printf '%s\n' "$imports" | awk '
	/[[:space:]]_AVCaptureDeviceType(ContinuityCamera|External)([[:space:]]|$)/ && !/weak external/ {
		print
		found = 1
	}
	END { exit !found }
'; then
	printf 'Found strong imports of macOS 14 camera device types. macOS 13 cannot load this executable.\n' >&2
	exit 1
fi
printf 'No strong imports of macOS 14 camera device types.\n'
