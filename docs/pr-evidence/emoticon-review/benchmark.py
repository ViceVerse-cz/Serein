from pathlib import Path
import json, subprocess, tempfile, statistics, platform, sys
roots = [Path(value) for value in sys.argv[1:3]]
if len(roots) != 2:
    raise SystemExit('Usage: python3 benchmark.py BASELINE_WORKTREE AFTER_WORKTREE')
with tempfile.TemporaryDirectory(prefix='serein-emoticon-bench-') as work:
    work = Path(work)
    bins = []
    for index, root in enumerate(roots):
        source = (root / 'crates/ui/src/emoticons.rs').read_text()
        source = '\n'.join(line for line in source.splitlines() if not line.startswith('//!'))
        code = 'mod emoticons {\n' + source + '\n}\n' + r'''
use std::{hint::black_box, time::Instant};
fn main() {
    let inputs = [
        "Hello :) welcome to Serein ;) <3".repeat(40),
        "Text :) `code :D` ```js\n:)``` :) https://example.test/:) ".repeat(30),
        "Literal \\` :) `` unmatched :D".repeat(40),
    ];
    for text in inputs {
        let begin = Instant::now();
        for _ in 0..100_000 { black_box(emoticons::convert(black_box(&text))); }
        println!("{} {}", text.len(), begin.elapsed().as_nanos());
    }
}
'''
        file = work / f'bench{index}.rs'
        file.write_text(code)
        binary = work / f'bench{index}'
        subprocess.run(['rustc', '--edition=2024', '-O', str(file), '-o', str(binary)], check=True)
        bins.append(binary)
    # One warmup and five alternating pairs; one conversion allocates one output string.
    samples = [[] for _ in bins]
    for run in range(6):
        for index, binary in enumerate(bins):
            rows = [list(map(int, line.split())) for line in subprocess.check_output([str(binary)], text=True).splitlines()]
            if run: samples[index].append(rows)
    result = {'host': platform.platform(), 'rust': subprocess.check_output(['rustc','--version'],text=True).strip(), 'iterations': 100000, 'warmup_batches': 1, 'measured_batches': 5, 'scenarios': ['plain', 'code_and_links', 'escaped_and_unmatched_ticks'], 'samples': samples, 'median_ns_per_conversion': [[statistics.median(run[scenario][1] for run in variant)/100000 for scenario in range(3)] for variant in samples]}
    Path(__file__).with_name('benchmark.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({k: v for k,v in result.items() if k != 'samples'},indent=2))
