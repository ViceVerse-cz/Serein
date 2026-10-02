"""Measure production attachment admission with the package build's release dependencies.

Run after cargo xtask package, with the same CARGO_TARGET_DIR when set. This copies
the validator into a temporary release harness; it never fetches media or opens audio.
"""
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile

root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[3]
target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target")).resolve()
source = (root / "apps/desktop/src/downloads.rs").read_text()
function = source[source.index("pub(crate) fn original_url("):source.index("\nstruct Partial<")]
program = '''use model::Attachment;
const MAX_BYTES: u64 = 100 * 1024 * 1024;
''' + function + '''
fn main() {
    let mut file = Attachment { id: model::Id(2), filename: "voice-message.ogg".into(),
        description: None, content_type: Some("audio/ogg".into()), size: 12288,
        media: model::EmbedMedia::default(), spoiler: false, duration_ms: Some(3000),
        waveform: vec![64; 256] };
    for path in ["attachments/1/2/voice-message.ogg", "attachments/1/3/2/voice-message.ogg"] {
        file.media.url = Some(format!("https://cdn.discordapp.com/{path}?ex=123&is=123&hm=synthetic"));
        let start = std::time::Instant::now();
        let mut admitted = 0;
        for _ in 0..100000 {
            admitted += usize::from(std::hint::black_box(original_url(std::hint::black_box(&file))).is_some());
        }
        println!("{} {} {}", path, start.elapsed().as_nanos(), admitted);
    }
}
'''
with tempfile.TemporaryDirectory(prefix="serein-attachment-benchmark-") as temporary:
    directory = Path(temporary)
    harness = directory / "benchmark.rs"
    executable = directory / "benchmark"
    harness.write_text(program)
    dependencies = target / "release/deps"
    arguments = ["rustc", "--edition=2024", "-O", str(harness), "-o", str(executable),
                 "-L", f"dependency={dependencies}"]
    for crate in ["model", "url"]:
        library = max(dependencies.glob(f"lib{crate}-*.rlib"), key=lambda path: path.stat().st_mtime)
        arguments.extend(["--extern", f"{crate}={library}"])
    subprocess.run(arguments, check=True, cwd=root)
    subprocess.run([str(executable)], check=True, stdout=subprocess.DEVNULL)
    results = [subprocess.check_output([str(executable)], text=True).splitlines() for _ in range(5)]
    report = {}
    for index in range(2):
        rows = [result[index].split() for result in results]
        times = [int(row[1]) / 1e6 for row in rows]
        report[rows[0][0]] = {"median_ms": statistics.median(times),
                            "admitted": [int(row[2]) for row in rows], "samples_ms": times}
    print(json.dumps(report, indent=2))
