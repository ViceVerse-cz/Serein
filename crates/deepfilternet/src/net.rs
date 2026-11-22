//! DeepFilterNet3 forward pass for one 10 ms frame. Feature maps are stored
//! `[frequency][channel]` and the weights as `tools/generate-deepfilternet-weights.py` lays
//! them out, so every inner loop walks contiguous memory.
use crate::dsp::{BANDS, FILTERED};
use std::ops::Range;

const CHANNELS: usize = 64;
const HIDDEN: usize = 256;
const GATES: usize = 3 * HIDDEN;
const EMBEDDING: usize = 512;
/// Frames each deep-filter bin combines.
pub const TAPS: usize = 5;
/// Real and imaginary filter coefficients per bin.
pub const COEFFICIENTS: usize = 2 * TAPS;
/// One frequency position of a feature map.
type Row = [f32; CHANNELS];

/// Keeps the embedded weights aligned for reading as `f32`.
#[repr(C)]
struct Aligned<T: ?Sized> {
	align: [f32; 0],
	bytes: T,
}
static WEIGHTS: &Aligned<[u8]> = &Aligned {
	align: [],
	bytes: *include_bytes!("dfn3.weights"),
};
#[cfg(target_endian = "big")]
compile_error!("the embedded DeepFilterNet weights are little-endian");

struct Weights(&'static [f32]);
impl Weights {
	fn take(&mut self, len: usize) -> &'static [f32] {
		let (head, rest) = self.0.split_at(len);
		self.0 = rest;
		head
	}
	fn rows<const N: usize>(&mut self, count: usize) -> &'static [[f32; N]] {
		self.take(count * N).as_chunks().0
	}
	fn separable(&mut self) -> Separable {
		Separable {
			taps: self.rows(3),
			mix: self.take(CHANNELS * CHANNELS),
			bias: self.take(CHANNELS),
		}
	}
	fn skip(&mut self) -> Skip {
		Skip {
			scale: self.take(CHANNELS),
			bias: self.take(CHANNELS),
		}
	}
	fn gru(&mut self) -> Gru {
		Gru {
			input: self.rows(HIDDEN),
			recurrent: self.rows(HIDDEN),
			bias: self.rows(2),
		}
	}
}

/// Depthwise three-tap convolution along frequency, then a channel mix with bias and ReLU.
struct Separable {
	taps: &'static [Row],
	mix: &'static [f32],
	bias: &'static [f32],
}
#[derive(Clone, Copy)]
enum Resample {
	Keep,
	/// Stride two.
	Halve,
	/// Transposed convolution with stride two.
	Double,
}
impl Resample {
	/// The input position that `tap` reads for output position `out`, if any.
	fn source(self, out: usize, tap: usize) -> Option<usize> {
		match self {
			Self::Keep => (out + tap).checked_sub(1),
			Self::Halve => (2 * out + tap).checked_sub(1),
			Self::Double => (out + 1)
				.checked_sub(tap)
				.filter(|twice| twice % 2 == 0)
				.map(|twice| twice / 2),
		}
	}
}
impl Separable {
	fn run(&self, out: &mut [Row], input: &[Row], resample: Resample) {
		for (position, out) in out.iter_mut().enumerate() {
			let mut depthwise = [0.0; CHANNELS];
			for (tap, taps) in self.taps.iter().enumerate() {
				let source = resample.source(position, tap);
				if let Some(input) = source.and_then(|source| input.get(source)) {
					for ((sum, x), weight) in depthwise.iter_mut().zip(input).zip(taps) {
						*sum += x * weight;
					}
				}
			}
			mix(out, &depthwise, self.mix, self.bias);
		}
	}
}

/// Per-channel scale, bias and ReLU on an encoder map, added to the decoder path.
struct Skip {
	scale: &'static [f32],
	bias: &'static [f32],
}
impl Skip {
	fn add(&self, out: &mut [Row], input: &[Row]) {
		for (out, input) in out.iter_mut().zip(input) {
			let weights = self.scale.iter().zip(self.bias);
			for ((out, x), (scale, bias)) in out.iter_mut().zip(input).zip(weights) {
				*out += (x * scale + bias).max(0.0);
			}
		}
	}
}

struct Gru {
	input: &'static [[f32; GATES]],
	recurrent: &'static [[f32; GATES]],
	/// Input bias, then recurrent bias.
	bias: &'static [[f32; GATES]],
}
impl Gru {
	fn step(&self, input: &[f32; HIDDEN], hidden: &mut [f32; HIDDEN]) {
		let (mut from_input, mut from_hidden) = (self.bias[0], self.bias[1]);
		for (&x, weights) in input.iter().zip(self.input) {
			// Inputs come out of a ReLU, so many of them are zero.
			if x != 0.0 {
				axpy(&mut from_input, x, weights);
			}
		}
		for (&x, weights) in hidden.iter().zip(self.recurrent) {
			axpy(&mut from_hidden, x, weights);
		}
		for (i, hidden) in hidden.iter_mut().enumerate() {
			let update = sigmoid(from_input[i] + from_hidden[i]);
			let reset = sigmoid(from_input[HIDDEN + i] + from_hidden[HIDDEN + i]);
			let new = (from_input[2 * HIDDEN + i] + reset * from_hidden[2 * HIDDEN + i]).tanh();
			*hidden = (1.0 - update) * new + update * *hidden;
		}
	}
}

/// History the model carries between frames.
struct State {
	/// The two previous band-feature frames.
	bands: [[f32; BANDS]; 2],
	/// The two previous bin-feature frames.
	bins: [[[f32; FILTERED]; 2]; 2],
	/// Bin feature maps of the last `TAPS` frames, oldest first.
	maps: Vec<Row>,
	/// Encoder, two mask-decoder and two filter-decoder recurrent states.
	hidden: [[f32; HIDDEN]; 5],
}

pub struct Network {
	band_input: &'static [[Row; 3]],
	band_input_bias: &'static [f32],
	band_down: [Separable; 3],
	bin_input: &'static [[Row; 3]],
	bin_input_mix: &'static [f32],
	bin_input_bias: &'static [f32],
	bin_down: Separable,
	bin_embedding: &'static [f32],
	encoder_in: &'static [f32],
	encoder: Gru,
	encoder_out: &'static [f32],
	mask_in: &'static [f32],
	mask: [Gru; 2],
	mask_out: &'static [f32],
	mask_skips: [Skip; 4],
	mask_up: [Separable; 3],
	mask_taps: &'static [Row],
	mask_bias: f32,
	filter_in: &'static [f32],
	filter: [Gru; 2],
	filter_skip: &'static [f32],
	filter_pathway: &'static [[[f32; CHANNELS / 2]; TAPS]],
	filter_pathway_mix: &'static [f32],
	filter_pathway_bias: &'static [f32],
	filter_out: &'static [f32],
	state: State,
}
impl Network {
	pub fn new() -> Self {
		let mut weights = Weights(bytemuck::cast_slice(&WEIGHTS.bytes));
		let band_input = weights.rows(9).as_chunks().0;
		let band_input_bias = weights.take(CHANNELS);
		let band_down = [
			weights.separable(),
			weights.separable(),
			weights.separable(),
		];
		let bin_input = weights.rows(9).as_chunks().0;
		let bin_input_mix = weights.take(CHANNELS * CHANNELS);
		let bin_input_bias = weights.take(CHANNELS);
		let bin_down = weights.separable();
		let bin_embedding = weights.take(FILTERED / 2 * CHANNELS * EMBEDDING / 32);
		let encoder_in = weights.take(EMBEDDING * HIDDEN / 16);
		let encoder = weights.gru();
		let encoder_out = weights.take(HIDDEN * EMBEDDING / 16);
		let mask_in = weights.take(EMBEDDING * HIDDEN / 16);
		let mask = [weights.gru(), weights.gru()];
		let mask_out = weights.take(HIDDEN * EMBEDDING / 16);
		let (skip3, up3) = (weights.skip(), weights.separable());
		let (skip2, up2) = (weights.skip(), weights.separable());
		let (skip1, up1) = (weights.skip(), weights.separable());
		let skip0 = weights.skip();
		let mask_taps = weights.rows(3);
		let mask_bias = weights.take(1)[0];
		let filter_in = weights.take(EMBEDDING * HIDDEN / 8);
		let filter = [weights.gru(), weights.gru()];
		let filter_skip = weights.take(EMBEDDING * HIDDEN / 16);
		let filter_pathway = weights.rows(COEFFICIENTS * TAPS).as_chunks().0;
		let filter_pathway_mix = weights.take(COEFFICIENTS * COEFFICIENTS);
		let filter_pathway_bias = weights.take(COEFFICIENTS);
		let filter_out = weights.take(HIDDEN * FILTERED * COEFFICIENTS / 16);
		debug_assert!(weights.0.is_empty());
		Self {
			band_input,
			band_input_bias,
			band_down,
			bin_input,
			bin_input_mix,
			bin_input_bias,
			bin_down,
			bin_embedding,
			encoder_in,
			encoder,
			encoder_out,
			mask_in,
			mask,
			mask_out,
			mask_skips: [skip3, skip2, skip1, skip0],
			mask_up: [up3, up2, up1],
			mask_taps,
			mask_bias,
			filter_in,
			filter,
			filter_skip,
			filter_pathway,
			filter_pathway_mix,
			filter_pathway_bias,
			filter_out,
			state: State {
				bands: [[0.0; BANDS]; 2],
				bins: [[[0.0; FILTERED]; 2]; 2],
				maps: vec![[0.0; CHANNELS]; TAPS * FILTERED],
				hidden: [[0.0; HIDDEN]; 5],
			},
		}
	}

	pub fn reset(&mut self) {
		self.state.bands = [[0.0; BANDS]; 2];
		self.state.bins = [[[0.0; FILTERED]; 2]; 2];
		self.state.maps.fill([0.0; CHANNELS]);
		self.state.hidden = [[0.0; HIDDEN]; 5];
	}

	/// Estimates the band gains and each filtered bin's coefficients (real and imaginary
	/// part per tap) for the frame two frames before these features: the model looks ahead.
	pub fn forward(
		&mut self,
		bands: &[f32; BANDS],
		bins: &[[f32; FILTERED]; 2],
		gains: &mut [f32; BANDS],
		coefficients: &mut [[f32; COEFFICIENTS]; FILTERED],
	) {
		let state = &mut self.state;

		// Encoder: band energies down to a quarter of the bands.
		let mut e0 = [[0.0; CHANNELS]; BANDS];
		for row in &mut e0 {
			row.copy_from_slice(self.band_input_bias);
		}
		let frames = [&state.bands[0][..], &state.bands[1][..], &bands[..]];
		input_conv(&mut e0, frames, self.band_input, 0..CHANNELS);
		relu(e0.as_flattened_mut());
		state.bands = [state.bands[1], *bands];
		let mut e1 = [[0.0; CHANNELS]; BANDS / 2];
		self.band_down[0].run(&mut e1, &e0, Resample::Halve);
		let mut e2 = [[0.0; CHANNELS]; BANDS / 4];
		self.band_down[1].run(&mut e2, &e1, Resample::Halve);
		let mut e3 = [[0.0; CHANNELS]; BANDS / 4];
		self.band_down[2].run(&mut e3, &e2, Resample::Keep);

		// Encoder: real parts feed the first half of the channels, imaginary parts the rest.
		let mut split = [[0.0; CHANNELS]; FILTERED];
		for (part, lanes) in [0..CHANNELS / 2, CHANNELS / 2..CHANNELS]
			.into_iter()
			.enumerate()
		{
			let [older, old] = &state.bins;
			let frames = [&older[part][..], &old[part][..], &bins[part][..]];
			input_conv(&mut split, frames, self.bin_input, lanes);
		}
		state.bins = [state.bins[1], *bins];
		state.maps.copy_within(FILTERED.., 0);
		let c0 = &mut state.maps[(TAPS - 1) * FILTERED..];
		for (out, input) in c0.iter_mut().zip(&split) {
			mix(out, input, self.bin_input_mix, self.bin_input_bias);
		}
		let mut c1 = [[0.0; CHANNELS]; FILTERED / 2];
		self.bin_down.run(&mut c1, c0, Resample::Halve);

		let mut embedding = [0.0; EMBEDDING];
		grouped(&mut embedding, c1.as_flattened(), self.bin_embedding, 32);
		for (embedding, e3) in embedding.iter_mut().zip(e3.as_flattened()) {
			*embedding = embedding.max(0.0) + e3;
		}
		let mut squeezed = [0.0; HIDDEN];
		grouped(&mut squeezed, &embedding, self.encoder_in, 16);
		relu(&mut squeezed);
		self.encoder.step(&squeezed, &mut state.hidden[0]);
		grouped(&mut embedding, &state.hidden[0], self.encoder_out, 16);
		relu(&mut embedding);

		// Mask decoder: back up to one gain per band, with skips from the encoder maps.
		grouped(&mut squeezed, &embedding, self.mask_in, 16);
		relu(&mut squeezed);
		let [_, first, second, ..] = &mut state.hidden;
		self.mask[0].step(&squeezed, first);
		self.mask[1].step(first, second);
		let mut d3 = [[0.0; CHANNELS]; BANDS / 4];
		grouped(d3.as_flattened_mut(), &second[..], self.mask_out, 16);
		relu(d3.as_flattened_mut());
		self.mask_skips[0].add(&mut d3, &e3);
		let mut d2 = [[0.0; CHANNELS]; BANDS / 4];
		self.mask_up[0].run(&mut d2, &d3, Resample::Keep);
		self.mask_skips[1].add(&mut d2, &e2);
		let mut d1 = [[0.0; CHANNELS]; BANDS / 2];
		self.mask_up[1].run(&mut d1, &d2, Resample::Double);
		self.mask_skips[2].add(&mut d1, &e1);
		let mut d0 = [[0.0; CHANNELS]; BANDS];
		self.mask_up[2].run(&mut d0, &d1, Resample::Double);
		self.mask_skips[3].add(&mut d0, &e0);
		for (band, gain) in gains.iter_mut().enumerate() {
			let mut sum = self.mask_bias;
			for (tap, taps) in self.mask_taps.iter().enumerate() {
				if let Some(input) = (band + tap)
					.checked_sub(1)
					.and_then(|source| d0.get(source))
				{
					sum += dot(input, taps);
				}
			}
			*gain = sigmoid(sum);
		}

		// Filter decoder: coefficients from the embedding, plus a direct path from the maps.
		grouped(&mut squeezed, &embedding, self.filter_in, 8);
		relu(&mut squeezed);
		let [.., first, second] = &mut state.hidden;
		self.filter[0].step(&squeezed, first);
		self.filter[1].step(first, second);
		grouped(&mut squeezed, &embedding, self.filter_skip, 16);
		for (skip, hidden) in squeezed.iter_mut().zip(&*second) {
			*skip += hidden;
		}
		grouped(
			coefficients.as_flattened_mut(),
			&squeezed,
			self.filter_out,
			16,
		);
		// The trained weights leave this path at zero after its ReLU for speech-range
		// features; it stays so that the port computes exactly what the model defines.
		let maps = state.maps.as_chunks::<FILTERED>().0;
		for (bin, out) in coefficients.iter_mut().enumerate() {
			let mut pathway = [0.0; COEFFICIENTS];
			for (channel, (sum, weights)) in pathway.iter_mut().zip(self.filter_pathway).enumerate()
			{
				let lanes = if channel < TAPS {
					0..CHANNELS / 2
				} else {
					CHANNELS / 2..CHANNELS
				};
				for (map, weights) in maps.iter().zip(weights) {
					*sum += dot(&map[bin][lanes.clone()], weights);
				}
			}
			let mut mixed = [0.0; COEFFICIENTS];
			mix(
				&mut mixed,
				&pathway,
				self.filter_pathway_mix,
				self.filter_pathway_bias,
			);
			for (out, pathway) in out.iter_mut().zip(mixed) {
				*out = out.tanh() + pathway;
			}
		}
	}
}

/// `out += x * weights`.
fn axpy(out: &mut [f32], x: f32, weights: &[f32]) {
	for (out, weight) in out.iter_mut().zip(weights) {
		*out += x * weight;
	}
}

/// Eight running sums let the compiler vectorize this without reordering one float chain.
fn dot(a: &[f32], b: &[f32]) -> f32 {
	let (a, a_rest) = a.as_chunks::<8>();
	let (b, b_rest) = b.as_chunks::<8>();
	let mut sums = [0.0; 8];
	for (a, b) in a.iter().zip(b) {
		for ((sum, a), b) in sums.iter_mut().zip(a).zip(b) {
			*sum += a * b;
		}
	}
	let rest = a_rest.iter().zip(b_rest).map(|(a, b)| a * b).sum::<f32>();
	sums.iter().sum::<f32>() + rest
}

fn relu(values: &mut [f32]) {
	for value in values {
		*value = value.max(0.0);
	}
}

fn sigmoid(x: f32) -> f32 {
	1.0 / (1.0 + (-x).exp())
}

/// `out = relu(bias + input * weights)` with `weights` as `[input][output]`.
fn mix(out: &mut [f32], input: &[f32], weights: &[f32], bias: &[f32]) {
	out.copy_from_slice(bias);
	for (&x, weights) in input.iter().zip(weights.chunks_exact(bias.len())) {
		axpy(out, x, weights);
	}
	relu(out);
}

/// Independent linear layers over equal slices of the input; `weights` is
/// `[group][input][output]`.
fn grouped(out: &mut [f32], input: &[f32], weights: &[f32], groups: usize) {
	let (ins, outs) = (input.len() / groups, out.len() / groups);
	out.fill(0.0);
	let groups = out.chunks_exact_mut(outs).zip(input.chunks_exact(ins));
	for ((out, input), weights) in groups.zip(weights.chunks_exact(ins * outs)) {
		for (&x, weights) in input.iter().zip(weights.chunks_exact(outs)) {
			axpy(out, x, weights);
		}
	}
}

/// Causal 3x3 convolution over the two previous frames and the current one, writing the
/// output channels in `lanes`; `weights` is `[frame][tap][channel]`.
fn input_conv(out: &mut [Row], frames: [&[f32]; 3], weights: &[[Row; 3]], lanes: Range<usize>) {
	for (position, out) in out.iter_mut().enumerate() {
		for (frame, weights) in frames.iter().zip(weights) {
			for (tap, weights) in weights.iter().enumerate() {
				let source = (position + tap).checked_sub(1);
				if let Some(&x) = source.and_then(|source| frame.get(source)) {
					axpy(&mut out[lanes.clone()], x, &weights[lanes.clone()]);
				}
			}
		}
	}
}
