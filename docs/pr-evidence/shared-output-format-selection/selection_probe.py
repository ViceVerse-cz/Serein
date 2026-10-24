#!/usr/bin/env python3
"""Measure output selection with synthetic devices; never opens a host or stream."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--revision', required=True, help='Git revision, or WORKTREE')
parser.add_argument('--cpal-lib', required=True, type=Path, help='Locked release build libcpal-*.rlib')
args = parser.parse_args()
root = Path(subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], text=True).strip())

def source(path):
    if args.revision == 'WORKTREE':
        return (root / path).read_text()
    return subprocess.check_output(['git', 'show', f'{args.revision}:{path}'], cwd=root, text=True, stderr=subprocess.PIPE)

try:
    shared = source('crates/platform/src/audio.rs')
except subprocess.CalledProcessError:
    shared = None
if shared is None:
    voice = source('crates/discord-voice/src/audio.rs')
    selection = voice[voice.index('fn config(device:'):voice.index('fn is_fatal_error(')]
    selection = selection.replace('&cpal::Device', '&FixtureDevice')
    # Exact default-query/channel/rate gate from notification open(), followed by
    # its supported stream match. No decoding, callbacks or OS/device calls timed.
    notification = source('apps/desktop/src/notification_sounds.rs')
    start = notification.index('let supported = device.default_output_config()')
    end = notification.index('\n\tlet samples = samples(', start)
    selection += '\nfn notification(device: &FixtureDevice) -> Result<cpal::SupportedStreamConfig, ()> {\n' + notification[start:end] + '''
    match supported.sample_format() {
        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::I32 | cpal::SampleFormat::U16 => Ok(supported),
        _ => Err(())
    }
}
'''
    voice_call = 'config(black_box(&alternate), false)'
    notification_call = 'notification(black_box(device))'
else:
    selection = shared.replace('//!', '//').replace('use cpal::traits::DeviceTrait;', '')
    selection = selection.replace('&cpal::Device', '&FixtureDevice')
    voice_call = 'output_config(black_box(&alternate), OutputConfigPreference::Voice)'
    notification_call = 'output_config(black_box(device), OutputConfigPreference::Notification)'

harness = '''
use cpal::{SampleFormat as F, SupportedStreamConfig as Config, SupportedStreamConfigRange as Range, SupportedBufferSize as B};
use std::{hint::black_box, time::Instant};
pub struct FixtureDevice { ranges: Vec<Range>, default: Config }
impl FixtureDevice {
    fn supported_output_configs(&self) -> Result<impl Iterator<Item = Range>, ()> { Ok(self.ranges.iter().cloned()) }
    fn supported_input_configs(&self) -> Result<impl Iterator<Item = Range>, ()> { self.supported_output_configs() }
    fn default_output_config(&self) -> Result<Config, ()> { Ok(self.default.clone()) }
    fn default_input_config(&self) -> Result<Config, ()> { self.default_output_config() }
}
fn measure(name: &str, mut run: impl FnMut() -> bool) {
    let mut durations = Vec::new();
    let mut successes = 0;
    for round in 0..6 {
        let start = Instant::now();
        let mut count = 0;
        for _ in 0..1_000_000 { count += usize::from(black_box(run())); }
        let ns = start.elapsed().as_nanos() as f64 / 1_000_000.0;
        if round > 0 { durations.push(ns); successes = count; }
    }
    println!("{{\\"workload\\":\\"{name}\\",\\"ns_per_selection\\":{durations:?},\\"successes\\":{successes}}}");
}
fn main() {
    let fixture = |format| FixtureDevice {
        default: Config::new(2, 44100, B::Unknown, format),
        ranges: (0..64).map(|i| Range::new(if i == 63 {2} else {1}, 8000, 192000, B::Unknown, if i == 63 {F::F32} else {F::I24})).collect()
    };
    let valid = fixture(F::F32);
    let alternate = fixture(F::I24);
    let notification = |device: &FixtureDevice| NOTIFICATION_CALL.is_ok();
    measure("notification-supported-default", || notification(&valid));
    measure("notification-I24-default-64-ranges", || notification(&alternate));
    measure("voice-64-ranges", || VOICE_CALL.is_ok());
}
'''.replace('NOTIFICATION_CALL', notification_call).replace('VOICE_CALL', voice_call)
lib = args.cpal_lib.resolve()
with tempfile.TemporaryDirectory(prefix='serein-selection-probe-') as temporary:
    temporary = Path(temporary)
    rust = temporary / 'probe.rs'
    rust.write_text(selection + harness)
    binary = temporary / 'probe'
    subprocess.run(['rustc', '--edition=2024', '-O', '-C', 'lto=thin', '-A', 'dead_code',
                    '-L', f'dependency={lib.parent}', '--extern', f'cpal={lib}',
                    str(rust), '-o', str(binary)], check=True)
    runs = subprocess.check_output([str(binary)], text=True)
print(json.dumps({'revision': args.revision, 'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                  'warmups': 1, 'measured_runs': 5, 'selections_per_run': 1000000,
                  'runs': [json.loads(line) for line in runs.splitlines()]}, indent=2))
