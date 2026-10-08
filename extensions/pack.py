"""Package a reviewed local build; this script never downloads or executes creator code."""
import argparse
import hashlib
import json
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("manifest", type=Path)
parser.add_argument("wasm", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
if args.manifest.stat().st_size > 16 * 1024 or args.wasm.stat().st_size > 4 * 1024 * 1024:
    parser.error("manifest or Wasm exceeds the package limits")
package = {"manifest": json.loads(args.manifest.read_text(encoding="utf-8")), "wasm": list(args.wasm.read_bytes())}
data = json.dumps(package, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
if len(data) > 16 * 1024 * 1024:
    parser.error("package exceeds 16 MiB")
args.output.parent.mkdir(parents=True, exist_ok=True)
args.output.write_bytes(data)
print(f"{args.output}: {len(data)} bytes; sha256={hashlib.sha256(data).hexdigest()}")
