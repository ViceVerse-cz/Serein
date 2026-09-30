use std::io::{Cursor, Read};
#[cfg(feature = "timings")]
use std::time::Instant;

use anyhow::{bail, ensure, Context, Result};
use flate2::read::GzDecoder;
use ini::Ini;
use ndarray::{prelude::*, Axis};
use tar::Archive;
use tract_core::internal::tract_itertools::izip;
use tract_core::internal::tract_smallvec::alloc::collections::VecDeque;
use tract_core::prelude::*;
use tract_pulse_opl::WithPulse;

use crate::*;

#[derive(Clone)]
pub struct DfParams {
	config: Ini,
	enc: Vec<u8>,
	erb_dec: Vec<u8>,
	df_dec: Vec<u8>,
}

impl DfParams {
	/// Load the one trusted, embedded model; no arbitrary model files are accepted.
	///
	/// The archive holds the upstream ONNX graphs already pulsed and decluttered into
	/// Tract NNEF by `tools/deep-filter-model`, so runtime needs no ONNX parser or pulsifier.
	pub fn embedded() -> Result<Self> {
		Self::from_targz(include_bytes!("../models/DeepFilterNet3_nnef.tar.gz").as_slice())
	}
	fn from_targz<R: Read>(f: R) -> Result<Self> {
		let tar = GzDecoder::new(f);
		let mut archive = Archive::new(tar);
		let mut enc = Vec::new();
		let mut erb_dec = Vec::new();
		let mut df_dec = Vec::new();
		let mut config = Ini::new();
		for e in archive
			.entries()
			.context("Could not extract models from tar file.")?
		{
			let mut file = e.context("Could not open model tar entry.")?;
			let path = file.path().unwrap();
			if path.ends_with("enc.nnef.tar") {
				file.read_to_end(&mut enc)?;
			} else if path.ends_with("erb_dec.nnef.tar") {
				file.read_to_end(&mut erb_dec)?;
			} else if path.ends_with("df_dec.nnef.tar") {
				file.read_to_end(&mut df_dec)?;
			} else if path.ends_with("config.ini") {
				config =
					Ini::read_from(&mut file).context("Could not load config from tar file.")?;
			} else if path.ends_with("version.txt") {
				let mut version = String::new();
				file.read_to_string(&mut version)
					.expect("Could not read version.txt");
				log::info!("Loading model with id: {}", version);
			} else {
				log::warn!("Found non-matching item in model tar file: {:?}", path)
			}
		}
		Ok(Self {
			config,
			enc,
			erb_dec,
			df_dec,
		})
	}
}
impl Default for DfParams {
	fn default() -> Self {
		Self::embedded().expect("Could not load embedded DeepFilterNet3 model")
	}
}

pub struct RuntimeParams {
	pub n_ch: usize,
	pub post_filter: bool,
	pub post_filter_beta: f32,
	pub atten_lim_db: f32,
	pub min_db_thresh: f32,
	pub max_db_erb_thresh: f32,
	pub max_db_df_thresh: f32,
}
impl RuntimeParams {
	pub fn new(
		n_ch: usize,
		post_filter_beta: f32,
		atten_lim_db: f32,
		min_db_thresh: f32,
		max_db_erb_thresh: f32,
		max_db_df_thresh: f32,
	) -> Self {
		let post_filter = post_filter_beta > 0.;
		Self {
			n_ch,
			post_filter,
			post_filter_beta,
			atten_lim_db,
			min_db_thresh,
			max_db_erb_thresh,
			max_db_df_thresh,
		}
	}
	pub fn with_post_filter(mut self, beta: f32) -> Self {
		assert!(beta >= 0.); // Cannot be negative
		if beta > 0. {
			self.post_filter = true;
		}
		self.post_filter_beta = beta;
		self
	}
	pub fn with_atten_lim(mut self, atten_lim_db: f32) -> Self {
		self.atten_lim_db = atten_lim_db;
		self
	}
	pub fn with_thresholds(
		mut self,
		min_db_thresh: f32,
		max_db_erb_thresh: f32,
		max_db_df_thresh: f32,
	) -> Self {
		self.min_db_thresh = min_db_thresh;
		self.max_db_erb_thresh = max_db_erb_thresh;
		self.max_db_df_thresh = max_db_df_thresh;
		self
	}
	pub fn default_with_ch(channels: usize) -> Self {
		RuntimeParams {
			n_ch: channels,
			post_filter: false,
			post_filter_beta: 0.02,
			atten_lim_db: 100.,
			min_db_thresh: -10.,
			max_db_erb_thresh: 30.,
			max_db_df_thresh: 20.,
		}
	}
}
impl Default for RuntimeParams {
	fn default() -> Self {
		Self::default_with_ch(1)
	}
}

pub type TractModel = TypedSimpleState<TypedModel, Arc<TypedSimplePlan<TypedModel>>>;
type FrozenTractModel = TypedFrozenSimpleState<TypedModel, Arc<TypedSimplePlan<TypedModel>>>;

#[derive(Clone)]
pub struct DfTract {
	enc: TractModel,
	erb_dec: TractModel,
	df_dec: TractModel,
	pub lookahead: usize,
	pub df_lookahead: usize,
	pub conv_lookahead: usize,
	pub sr: usize,
	pub ch: usize,
	pub fft_size: usize,
	pub hop_size: usize,
	pub nb_erb: usize,
	pub min_nb_erb_freqs: usize,
	pub nb_df: usize,
	pub n_freqs: usize,
	pub df_order: usize,
	pub post_filter: bool,
	pub post_filter_beta: f32,
	pub alpha: f32,
	pub min_db_thresh: f32,
	pub max_db_erb_thresh: f32,
	pub max_db_df_thresh: f32,
	pub atten_lim: Option<f32>,
	pub df_states: Vec<DFState>,
	pub spec_buf: Tensor, // Real-valued spectrogram buffer of shape [n_ch, 1, 1, n_freqs, 2]
	erb_buf: Tensor,      // Real-valued ERB feature buffer of shape [n_ch, 1, 1, n_erb]
	cplx_buf: Tensor,     // Real-valued complex epectrum shape for DF of shape [n_ch, 1, nb_df, 2]
	m_zeros: Vec<f32>,    // Preallocated buffer for applying a zero mask
	rolling_spec_buf_y: VecDeque<Tensor>, // Enhanced stage 1 spec buf
	rolling_spec_buf_x: VecDeque<Tensor>, // Noisy spec buf
	skip_counter: usize,  // Increment when wanting to skip processing due to low RMS
}

/// A Send representation used only to hand the prepared model to its media worker.
/// Compiled plans are shared through Arc; captured recurrent state is frozen explicitly.
pub struct FrozenDfTract {
	enc: FrozenTractModel,
	erb_dec: FrozenTractModel,
	df_dec: FrozenTractModel,
	pub lookahead: usize,
	pub df_lookahead: usize,
	pub conv_lookahead: usize,
	pub sr: usize,
	pub ch: usize,
	pub fft_size: usize,
	pub hop_size: usize,
	pub nb_erb: usize,
	pub min_nb_erb_freqs: usize,
	pub nb_df: usize,
	pub n_freqs: usize,
	pub df_order: usize,
	pub post_filter: bool,
	pub post_filter_beta: f32,
	pub alpha: f32,
	pub min_db_thresh: f32,
	pub max_db_erb_thresh: f32,
	pub max_db_df_thresh: f32,
	pub atten_lim: Option<f32>,
	pub df_states: Vec<DFState>,
	pub spec_buf: Tensor, // Real-valued spectrogram buffer of shape [n_ch, 1, 1, n_freqs, 2]
	erb_buf: Tensor,      // Real-valued ERB feature buffer of shape [n_ch, 1, 1, n_erb]
	cplx_buf: Tensor,     // Real-valued complex epectrum shape for DF of shape [n_ch, 1, nb_df, 2]
	m_zeros: Vec<f32>,    // Preallocated buffer for applying a zero mask
	rolling_spec_buf_y: VecDeque<Tensor>, // Enhanced stage 1 spec buf
	rolling_spec_buf_x: VecDeque<Tensor>, // Noisy spec buf
	skip_counter: usize,  // Increment when wanting to skip processing due to low RMS
}

impl FrozenDfTract {
	/// Restore worker-owned inference states, without parsing or optimizing model graphs.
	pub fn unfreeze(self) -> DfTract {
		DfTract {
			enc: self.enc.unfreeze(),
			erb_dec: self.erb_dec.unfreeze(),
			df_dec: self.df_dec.unfreeze(),
			lookahead: self.lookahead,
			df_lookahead: self.df_lookahead,
			conv_lookahead: self.conv_lookahead,
			sr: self.sr,
			ch: self.ch,
			fft_size: self.fft_size,
			hop_size: self.hop_size,
			nb_erb: self.nb_erb,
			min_nb_erb_freqs: self.min_nb_erb_freqs,
			nb_df: self.nb_df,
			n_freqs: self.n_freqs,
			df_order: self.df_order,
			post_filter: self.post_filter,
			post_filter_beta: self.post_filter_beta,
			alpha: self.alpha,
			min_db_thresh: self.min_db_thresh,
			max_db_erb_thresh: self.max_db_erb_thresh,
			max_db_df_thresh: self.max_db_df_thresh,
			atten_lim: self.atten_lim,
			df_states: self.df_states,
			spec_buf: self.spec_buf,
			erb_buf: self.erb_buf,
			cplx_buf: self.cplx_buf,
			m_zeros: self.m_zeros,
			rolling_spec_buf_y: self.rolling_spec_buf_y,
			rolling_spec_buf_x: self.rolling_spec_buf_x,
			skip_counter: self.skip_counter,
		}
	}
}

impl Default for DfTract {
	fn default() -> Self {
		let r_params = RuntimeParams::default();
		let df_params = DfParams::default();
		DfTract::new(df_params, &r_params).expect("Could not load DfTract")
	}
}

impl DfTract {
	/// Move prepared inference across a thread boundary using Tract's safe frozen states.
	pub fn freeze(self) -> FrozenDfTract {
		FrozenDfTract {
			enc: self.enc.freeze(),
			erb_dec: self.erb_dec.freeze(),
			df_dec: self.df_dec.freeze(),
			lookahead: self.lookahead,
			df_lookahead: self.df_lookahead,
			conv_lookahead: self.conv_lookahead,
			sr: self.sr,
			ch: self.ch,
			fft_size: self.fft_size,
			hop_size: self.hop_size,
			nb_erb: self.nb_erb,
			min_nb_erb_freqs: self.min_nb_erb_freqs,
			nb_df: self.nb_df,
			n_freqs: self.n_freqs,
			df_order: self.df_order,
			post_filter: self.post_filter,
			post_filter_beta: self.post_filter_beta,
			alpha: self.alpha,
			min_db_thresh: self.min_db_thresh,
			max_db_erb_thresh: self.max_db_erb_thresh,
			max_db_df_thresh: self.max_db_df_thresh,
			atten_lim: self.atten_lim,
			df_states: self.df_states,
			spec_buf: self.spec_buf,
			erb_buf: self.erb_buf,
			cplx_buf: self.cplx_buf,
			m_zeros: self.m_zeros,
			rolling_spec_buf_y: self.rolling_spec_buf_y,
			rolling_spec_buf_x: self.rolling_spec_buf_x,
			skip_counter: self.skip_counter,
		}
	}

	pub fn new(dfp: DfParams, rp: &RuntimeParams) -> Result<Self> {
		#[cfg(feature = "timings")]
		let t0 = Instant::now();
		let config = dfp.config;
		let model_cfg = config.section(Some("deepfilternet")).unwrap();
		let df_cfg = config.section(Some("df")).unwrap();
		let ch = rp.n_ch;
		ensure!(ch == 1, "Serein DeepFilterNet accepts mono audio only");

		let nnef = tract_nnef::nnef().with_tract_core().with_pulse();
		let load = |graph: &[u8]| -> Result<TractModel> {
			let model = nnef.model_for_read(&mut Cursor::new(graph))?;
			Ok(SimpleState::new(Arc::new(
				model.into_optimized()?.into_runnable()?,
			))?)
		};
		let enc = load(&dfp.enc)?;
		let erb_dec = load(&dfp.erb_dec)?;
		let df_dec = load(&dfp.df_dec)?;
		#[cfg(feature = "timings")]
		let t1 = Instant::now();

		let sr = df_cfg.get("sr").unwrap().parse::<usize>()?;
		let hop_size = df_cfg.get("hop_size").unwrap().parse::<usize>()?;
		let fft_size = df_cfg.get("fft_size").unwrap().parse::<usize>()?;
		let min_nb_erb_freqs = df_cfg.get("min_nb_erb_freqs").unwrap().parse::<usize>()?;
		let nb_erb = df_cfg.get("nb_erb").unwrap().parse::<usize>()?;
		let nb_df = df_cfg.get("nb_df").unwrap().parse::<usize>()?;
		let df_order = df_cfg
			.get("df_order")
			.unwrap_or_else(|| model_cfg.get("df_order").unwrap())
			.parse::<usize>()?;
		let conv_lookahead = model_cfg.get("conv_lookahead").unwrap().parse::<usize>()?;
		let df_lookahead = df_cfg
			.get("df_lookahead")
			.unwrap_or_else(|| model_cfg.get("df_lookahead").unwrap())
			.parse::<usize>()?;
		let n_freqs = fft_size / 2 + 1;
		let alpha = if let Some(a) = df_cfg.get("norm_alpha") {
			a.parse::<f32>()?
		} else {
			let tau = df_cfg.get("norm_tau").unwrap().parse::<f32>()?;
			calc_norm_alpha(sr, hop_size, tau)
		};
		let atten_lim = rp.atten_lim_db.abs();
		let atten_lim = if atten_lim >= 100. {
			None
		} else if atten_lim < 0.01 {
			log::warn!("Attenuation limit too strong. No noise reduction will be performed");
			Some(1.)
		} else {
			log::info!("Running with an attenuation limit of {:.0} dB", atten_lim);
			Some(10f32.powf(-atten_lim / 20.))
		};
		let spec_shape = [1, 1, 1, n_freqs, 2];
		let spec_buf = Tensor::zero::<f32>(&spec_shape)?;
		let erb_buf = Tensor::zero::<f32>(&[1, 1, 1, nb_erb])?;
		let cplx_buf = Tensor::zero::<f32>(&[1, 1, nb_df, 2])?;
		let m_zeros = vec![0.; nb_erb];

		let model_type = config.section(Some("train")).unwrap().get("model").unwrap();
		let lookahead = match model_type {
			"deepfilternet2" => bail!(
				"DeepFilterNet2 models are deprecated. Please use version v0.3.1 for these models."
			),
			"deepfilternet3" => conv_lookahead.max(df_lookahead),
			_ => bail!("Unsupported model type {}", model_type),
		};
		log::info!(
			"Running with model type {} lookahead {}",
			model_type,
			lookahead
		);

		let rolling_spec_buf_y = VecDeque::with_capacity(df_order + lookahead);
		let rolling_spec_buf_x = VecDeque::with_capacity(lookahead.max(df_order));

		let mut state = DFState::new(sr, fft_size, hop_size, nb_erb, min_nb_erb_freqs);
		state.init_norm_states(nb_df);
		let df_states = vec![state];

		let mut m = Self {
			enc,
			erb_dec,
			df_dec,
			lookahead,
			conv_lookahead,
			df_lookahead,
			sr,
			ch,
			fft_size,
			hop_size,
			nb_erb,
			min_nb_erb_freqs,
			nb_df,
			n_freqs,
			df_order,
			alpha,
			min_db_thresh: rp.min_db_thresh,
			max_db_erb_thresh: rp.max_db_erb_thresh,
			max_db_df_thresh: rp.max_db_df_thresh,
			atten_lim,
			spec_buf,
			erb_buf,
			cplx_buf,
			m_zeros,
			rolling_spec_buf_y,
			rolling_spec_buf_x,
			df_states,
			post_filter: rp.post_filter,
			post_filter_beta: rp.post_filter_beta,
			skip_counter: 0,
		};
		m.init()?;
		#[cfg(feature = "timings")]
		log::info!(
			"Init DfTract in {:.2}ms (models in {:.2}ms)",
			t0.elapsed().as_secs_f32() * 1000.,
			(t1 - t0).as_secs_f32() * 1000.
		);

		Ok(m)
	}

	pub fn set_pf_beta(&mut self, beta: f32) {
		log::debug!("Setting post-filter beta to {beta}");
		self.post_filter_beta = beta;
		if beta > 0. {
			self.post_filter = true;
		} else if beta == 0. {
			self.post_filter = false;
		} else {
			log::warn!("Post-filter beta cannot be smaller than 0.");
			self.post_filter = false;
			self.post_filter_beta = 0.;
		}
	}

	pub fn set_atten_lim(&mut self, db: f32) {
		let lim = db.abs();
		self.atten_lim = if lim >= 100. {
			None
		} else if lim < 0.01 {
			log::warn!("Attenuation limit too strong. No noise reduction will be performed");
			Some(1.)
		} else {
			log::debug!("Setting attenuation limit to {:.1} dB", lim);
			Some(10f32.powf(-lim / 20.))
		};
	}

	fn init(&mut self) -> Result<()> {
		let ch = self.ch;
		let spec_shape = [ch, 1, 1, self.n_freqs, 2];
		self.rolling_spec_buf_y.clear();
		self.rolling_spec_buf_x.clear();
		for _ in 0..(self.df_order + self.conv_lookahead) {
			self.rolling_spec_buf_y
				.push_back(tensor0(0f32).broadcast_scalar_to_shape(&spec_shape)?);
		}
		for _ in 0..self.df_order.max(self.lookahead) {
			self.rolling_spec_buf_x
				.push_back(tensor0(0f32).broadcast_scalar_to_shape(&spec_shape)?);
		}
		if ch > self.df_states.len() {
			for _ in self.df_states.len()..ch {
				let mut state = DFState::new(
					self.sr,
					self.fft_size,
					self.hop_size,
					self.nb_erb,
					self.min_nb_erb_freqs,
				);
				state.init_norm_states(self.nb_df);
				self.df_states.push(state)
			}
		}
		self.spec_buf = Tensor::zero::<f32>(&spec_shape)?;
		self.erb_buf = Tensor::zero::<f32>(&[ch, 1, 1, self.nb_erb])?;
		self.cplx_buf = Tensor::zero::<f32>(&[ch, 1, self.nb_df, 2])?;

		Ok(())
	}

	/// Reset a capture generation completely while retaining compiled model and FFT plans.
	///
	/// This belongs on the media worker, never an audio callback. Recurrent op states may
	/// allocate bounded model-sized scratch on reset/first use; the graph is not reloaded,
	/// cloned, or optimized. Repeated resets retain no prior spectral or recurrent history.
	pub fn reset(&mut self) -> Result<()> {
		for model in [&mut self.enc, &mut self.erb_dec, &mut self.df_dec] {
			model.reset_turn()?;
			model.session_state = Default::default();
			model.reset_op_states()?;
		}
		for state in &mut self.df_states {
			state.reset_stream();
		}
		for spectrum in self
			.rolling_spec_buf_x
			.iter_mut()
			.chain(self.rolling_spec_buf_y.iter_mut())
		{
			spectrum.as_slice_mut::<f32>()?.fill(0.);
		}
		self.spec_buf.as_slice_mut::<f32>()?.fill(0.);
		self.erb_buf.as_slice_mut::<f32>()?.fill(0.);
		self.cplx_buf.as_slice_mut::<f32>()?.fill(0.);
		self.skip_counter = 0;
		Ok(())
	}

	/// Process a FD sample and return the raw gains and DF coefs.
	///
	/// Warning:
	///     `self.spec_buf` needs to be initialized correctly before calling this method!
	///
	/// Returns:
	///     - lsnr: Local SNR estiamte.
	///     - gains: Gain estimates of shape `[n_ch, 1, 1, n_erb]`.
	///     - coefs: Real-valued DF coefficients estimates of shape `[n_ch, 1, 1, n_erb, 2]`.
	pub fn process_raw(&mut self) -> Result<(f32, Option<Tensor>, Option<Tensor>)> {
		let spec = self.spec_buf.to_array_view()?;
		let ch = spec.len_of(Axis(0));

		for (nsy_ch, mut erb_ch, mut cplx_ch, state) in izip!(
			spec.axis_iter(Axis(0)),
			self.erb_buf
				.to_array_view_mut::<f32>()?
				.axis_iter_mut(Axis(0)),
			self.cplx_buf
				.to_array_view_mut::<f32>()?
				.axis_iter_mut(Axis(0)),
			self.df_states.iter_mut()
		) {
			let nsy_ch = as_slice_complex(nsy_ch.as_slice().unwrap());
			state.feat_erb(nsy_ch, self.alpha, erb_ch.as_slice_mut().unwrap());
			state.feat_cplx(
				&nsy_ch[..self.nb_df],
				self.alpha,
				as_slice_mut_complex(cplx_ch.as_slice_mut().unwrap()),
			);
		}
		// Run encoder
		let mut enc_emb = self.enc.run(tvec!(
			self.erb_buf.clone().into(),
			TValue::from(self.cplx_buf.clone().permute_axes(&[0, 3, 1, 2])?)
		))?;

		let &lsnr = enc_emb.pop().unwrap().to_scalar::<f32>()?;
		let c0 = enc_emb.pop().unwrap();
		let emb = enc_emb.pop().unwrap();

		let (apply_gains, apply_gain_zeros, apply_df) = self.apply_stages(lsnr);

		log::trace!(
			"Enhancing frame with lsnr {:>5.1} dB. Applying stage 1: {} and stage 2: {}.",
			lsnr,
			apply_gains,
			apply_df
		);

		let m = if apply_gains {
			let dec_input = tvec!(
				emb.clone(),
				enc_emb.pop().unwrap(), // e3
				enc_emb.pop().unwrap(), // e2
				enc_emb.pop().unwrap(), // e1
				enc_emb.pop().unwrap(), // e0
			);
			let mut m = self.erb_dec.run(dec_input)?;
			let mut m = m.pop().unwrap().into_tensor();
			m.remove_axis(1)?;
			m.remove_axis(1)?;
			Some(m)
		} else if apply_gain_zeros {
			Some(Tensor::zero::<f32>(&[self.ch, self.nb_erb])?)
		} else {
			None
		};

		let coefs = if apply_df {
			let mut coefs = self
				.df_dec
				.run(tvec!(emb, c0))?
				.pop()
				.unwrap()
				.into_tensor();
			coefs.set_shape(&[ch, self.nb_df, self.df_order, 2])?;
			Some(coefs)
		} else {
			None
		};

		Ok((lsnr, m, coefs))
	}

	/// Process a noisy time domain sample and return the enhanced sample via mutable argument.
	pub fn process(&mut self, noisy: ArrayView2<f32>, mut enh: ArrayViewMut2<f32>) -> Result<f32> {
		ensure!(
			noisy.shape() == [self.ch, self.hop_size]
				&& enh.shape() == [self.ch, self.hop_size]
				&& noisy.is_standard_layout()
				&& enh.is_standard_layout(),
			"DeepFilterNet requires contiguous mono hop-sized frames"
		);
		let (max_a, e) = noisy.iter().fold((0f32, 0f32), |acc, x| {
			(acc.0.max(x.abs()), acc.1 + x.powi(2))
		});
		let rms = e / noisy.len() as f32;
		if rms < 1e-7 {
			self.skip_counter = self.skip_counter.saturating_add(1);
		} else {
			self.skip_counter = 0;
		}
		if self.skip_counter > 5 {
			enh.fill(0.);
			return Ok(-15.);
		}
		if max_a > 0.9999 {
			log::warn!("Possible clipping detected ({:.3}).", max_a)
		}

		// Signal model: y = f(s + n) = f(x)
		self.rolling_spec_buf_y.pop_front();
		self.rolling_spec_buf_x.pop_front();
		for (ns_ch, mut rbuf, state) in izip!(
			noisy.axis_iter(Axis(0)),
			self.spec_buf.to_array_view_mut()?.axis_iter_mut(Axis(0)),
			self.df_states.iter_mut(),
		) {
			let spec = as_slice_mut_complex(rbuf.as_slice_mut().unwrap());
			state.analysis(ns_ch.as_slice().unwrap(), spec);
		}
		self.rolling_spec_buf_y.push_back(self.spec_buf.clone());
		self.rolling_spec_buf_x.push_back(self.spec_buf.clone());
		if self.atten_lim.unwrap_or_default() == 1. {
			enh.assign(&noisy);
			return Ok(35.);
		}

		let (lsnr, gains, coefs) = self.process_raw()?;

		let (apply_erb, _, _) = self.apply_stages(lsnr);
		let mut spec = self
			.rolling_spec_buf_y
			.get_mut(self.df_order - 1)
			.unwrap()
			.to_array_view_mut()?;
		if let Some(gains) = gains {
			let mut gains = gains.into_array()?;
			if gains.shape()[0] < noisy.shape()[0] {
				// Mask was reduced to single channel
				let gain_slc = gains.as_slice_mut().unwrap();
				for mut spec_ch in spec.axis_iter_mut(Axis(0)) {
					self.df_states[0].apply_mask(
						as_slice_mut_complex(spec_ch.as_slice_mut().unwrap()),
						gain_slc,
					);
				}
			} else {
				// Same number of channels of gains and spec
				for (gains_ch, mut spec_ch) in
					gains.axis_iter(Axis(0)).zip(spec.axis_iter_mut(Axis(0)))
				{
					let gain_slc = gains_ch.as_slice().unwrap();
					self.df_states[0].apply_mask(
						as_slice_mut_complex(spec_ch.as_slice_mut().unwrap()),
						gain_slc,
					);
				}
			}
			self.skip_counter = 0;
		} else {
			// gains are None => skipped due to LSNR
			self.skip_counter = self.skip_counter.saturating_add(1);
		}

		// This spectrum will only be used for the upper frequecies
		let spec = self.rolling_spec_buf_y.get_mut(self.df_order - 1).unwrap();
		self.spec_buf.clone_from(spec);
		if let Some(coefs) = coefs {
			df(
				&self.rolling_spec_buf_x,
				coefs,
				self.nb_df,
				self.df_order,
				self.n_freqs,
				&mut self.spec_buf,
			)?;
		};

		let spec_noisy = as_arrayview_complex(
			self.rolling_spec_buf_x
				.get(self.lookahead.max(self.df_order) - self.lookahead - 1)
				.unwrap()
				.to_array_view::<f32>()
				.unwrap(),
			&[self.ch, self.n_freqs],
		)
		.into_dimensionality::<Ix2>()
		.unwrap();
		let mut spec_enh = as_arrayview_mut_complex(
			self.spec_buf.to_array_view_mut::<f32>().unwrap(),
			&[self.ch, self.n_freqs],
		)
		.into_dimensionality::<Ix2>()
		.unwrap();

		// Run post filter
		if apply_erb && self.post_filter {
			post_filter(
				spec_noisy.as_slice().unwrap(),
				spec_enh.as_slice_mut().unwrap(),
				self.post_filter_beta,
			);
		}

		// Limit noise attenuation by mixing back some of the noisy signal
		if let Some(lim) = self.atten_lim {
			spec_enh.map_inplace(|x| *x *= 1. - lim);
			spec_enh.scaled_add(lim.into(), &spec_noisy);
		}

		for (state, spec_ch, mut enh_out_ch) in izip!(
			self.df_states.iter_mut(),
			spec_enh.axis_iter(Axis(0)),
			enh.axis_iter_mut(Axis(0)),
		) {
			state.synthesis(
				spec_ch.to_owned().as_slice_mut().unwrap(),
				enh_out_ch.as_slice_mut().unwrap(),
			);
		}
		Ok(lsnr)
	}

	/// For some frames, processing may be skipped based on the current local snr and the defined
	/// thresholds. This methods indiciated whether stage 1 (gains) and stage 2 (DF) can be
	/// skipped.
	///
	/// Args:
	///     - lsnr: Current local SNR estimate
	///
	/// Returns:
	///     - apply_gains: Local SNR is above `min_dfb_erb_thresh`, gains are estimated and should be
	///         applied
	///     - apply_gain_zeros: Local SNR is less than `min_db_thresh`, no speech is detected.
	///         Zeros should be applied instead of the gain estimates.
	///     - apply_df: Local SNR is greater than `max_db_df_thresh` and the estimated DF coefs
	///         should be applied
	pub fn apply_stages(&self, lsnr: f32) -> (bool, bool, bool) {
		if lsnr < self.min_db_thresh {
			// Only noise detected, just apply a zero mask
			(false, true, false)
		} else if lsnr > self.max_db_erb_thresh {
			// Clean speech signal detected, don't apply any processing
			(false, false, false)
		} else if lsnr > self.max_db_df_thresh {
			// Only little noisy signal detected, just apply 1st stage, skip DF stage
			(true, false, false)
		} else {
			// Regular noisy signal detected, apply 1st and 2nd stage
			(true, false, true)
		}
	}

	pub fn set_spec_buffer(&mut self, spec: ArrayView2<f32>) -> Result<()> {
		debug_assert_eq!(self.spec_buf.shape(), spec.shape());
		let mut buf = self
			.spec_buf
			.to_array_view_mut()?
			.into_shape_with_order([self.ch, self.n_freqs])?;
		for (i_ch, mut b_ch) in spec.outer_iter().zip(buf.outer_iter_mut()) {
			for (&i, b) in i_ch.iter().zip(b_ch.iter_mut()) {
				*b = i
			}
		}
		Ok(())
	}

	pub fn get_spec_noisy(&self) -> ArrayView2<'_, Complex32> {
		as_arrayview_complex(
			self.rolling_spec_buf_x
				.get(self.lookahead.max(self.df_order) - self.lookahead - 1)
				.unwrap()
				.to_array_view::<f32>()
				.unwrap(),
			&[self.ch, self.n_freqs],
		)
		.into_dimensionality::<Ix2>()
		.unwrap()
	}
	pub fn get_spec_enh(&self) -> ArrayView2<'_, Complex32> {
		as_arrayview_complex(
			self.spec_buf.to_array_view::<f32>().unwrap(),
			&[self.ch, self.n_freqs],
		)
		.into_dimensionality::<Ix2>()
		.unwrap()
	}
	pub fn get_mut_spec_enh(&mut self) -> ArrayViewMut2<'_, Complex32> {
		as_arrayview_mut_complex(
			self.spec_buf.to_array_view_mut::<f32>().unwrap(),
			&[self.ch, self.n_freqs],
		)
		.into_dimensionality::<Ix2>()
		.unwrap()
	}
}

/// Deep Filtering.
///
/// Args:
///     - spec: Spectrogram buffer for the corresponding time steps. Needs to contain `df_order + conv_lookahead` frames and applies DF to the oldest frames.
///     - coefs: Complex DF coefficients of shape `[C, N, F', 2]`, `N`: `df_order`, `F'`: `nb_df`
///     - nb_df: Number of DF frequency bins
///     - df_order: Deep Filtering order
///     - n_freqs: Number of FFT bins
///     - spec_out: Ouput buffer of shape `[C, F, 2]`, `F`: `n_freqs`
fn df(
	spec: &VecDeque<Tensor>,
	coefs: Tensor,
	nb_df: usize,
	df_order: usize,
	n_freqs: usize,
	spec_out: &mut Tensor,
) -> Result<()> {
	let ch = spec.back().unwrap().shape()[0];
	debug_assert_eq!(n_freqs, spec.back().unwrap().shape()[3]);
	debug_assert_eq!(n_freqs, spec_out.shape()[3]);
	debug_assert_eq!(ch, coefs.shape()[0]);
	debug_assert_eq!(nb_df, coefs.shape()[1]);
	debug_assert_eq!(df_order, coefs.shape()[2]);
	debug_assert_eq!(ch, spec_out.shape()[0]);
	debug_assert!(spec.len() >= df_order);
	let mut o_f: ArrayViewMut2<Complex32> =
		as_arrayview_mut_complex(spec_out.to_array_view_mut::<f32>()?, &[ch, n_freqs])
			.into_dimensionality()?;
	// Zero relevant frequency bins of output
	o_f.slice_mut(s![.., ..nb_df]).fill(Complex32::default());
	let coefs_arr: ArrayView3<Complex32> =
		as_arrayview_complex(coefs.to_array_view::<f32>()?, &[ch, nb_df, df_order])
			.into_dimensionality()?;
	// Transform spec to an complex array and iterate over time frames of spec and coefs
	let spec_iter = spec.iter().map(|s| {
		as_arrayview_complex(s.to_array_view::<f32>().unwrap(), &[ch, n_freqs])
			.into_dimensionality::<Ix2>()
			.unwrap()
	});
	// Iterate over DF frames
	for (s_f, c_f) in spec_iter.zip(coefs_arr.axis_iter(Axis(2))) {
		// Iterate over channels
		for (s_ch, c_ch, mut o_ch) in
			izip!(s_f.outer_iter(), c_f.outer_iter(), o_f.outer_iter_mut())
		{
			// Apply DF for each frequency bin up to `nb_df`
			for (&s, &c, o) in izip!(s_ch, c_ch, o_ch.iter_mut()) {
				*o += s * c
			}
		}
	}
	Ok(())
}

fn calc_norm_alpha(sr: usize, hop_size: usize, tau: f32) -> f32 {
	let dt = hop_size as f32 / sr as f32;
	let alpha = f32::exp(-dt / tau);
	let mut a = 1.0;
	let mut precision = 3;
	while a >= 1.0 {
		a = (alpha * 10i32.pow(precision) as f32).round() / 10i32.pow(precision) as f32;
		precision += 1;
	}
	a
}

// num_complex::Complex is repr(C), with real then imaginary components.
const _: () = assert!(std::mem::size_of::<Complex32>() == 2 * std::mem::size_of::<f32>());
const _: () = assert!(std::mem::align_of::<Complex32>() == std::mem::align_of::<f32>());

pub fn as_slice_complex(buffer: &[f32]) -> &[Complex32] {
	assert_eq!(
		buffer.len() % 2,
		0,
		"Complex samples require pairs of floats"
	);
	unsafe {
		let ptr = buffer.as_ptr() as *const Complex32;
		let len = buffer.len();
		std::slice::from_raw_parts(ptr, len / 2)
	}
}

#[allow(clippy::needless_pass_by_ref_mut)]
pub fn as_slice_mut_complex(buffer: &mut [f32]) -> &mut [Complex32] {
	assert_eq!(
		buffer.len() % 2,
		0,
		"Complex samples require pairs of floats"
	);
	unsafe {
		let ptr = buffer.as_mut_ptr() as *mut Complex32;
		let len = buffer.len();
		std::slice::from_raw_parts_mut(ptr, len / 2)
	}
}

pub fn as_arrayview_complex<'a>(
	buffer: ArrayViewD<'a, f32>,
	shape: &[usize], // having an explicit shape parameter allows to also squeeze axes.
) -> ArrayViewD<'a, Complex32> {
	assert_eq!(buffer.shape().last(), Some(&2));
	assert!(
		buffer.is_standard_layout(),
		"Complex view must be contiguous"
	);
	let floats = shape
		.iter()
		.try_fold(2usize, |n, &dim| n.checked_mul(dim))
		.expect("Complex view shape overflow");
	assert_eq!(buffer.len(), floats);
	unsafe {
		let ptr = buffer.as_ptr() as *const Complex32;
		ArrayViewD::from_shape_ptr(shape, ptr)
	}
}
pub fn as_arrayview_mut_complex<'a>(
	mut buffer: ArrayViewMutD<'a, f32>,
	shape: &[usize], // having an explicit shape parameter allows to also squeeze axes.
) -> ArrayViewMutD<'a, Complex32> {
	assert_eq!(buffer.shape().last(), Some(&2));
	assert!(
		buffer.is_standard_layout(),
		"Complex view must be contiguous"
	);
	let floats = shape
		.iter()
		.try_fold(2usize, |n, &dim| n.checked_mul(dim))
		.expect("Complex view shape overflow");
	assert_eq!(buffer.len(), floats);
	unsafe {
		let ptr = buffer.as_mut_ptr() as *mut Complex32;
		ArrayViewMutD::from_shape_ptr(shape, ptr)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn synthetic_frame(index: usize) -> [f32; 480] {
		std::array::from_fn(|sample| {
			let n = index * 480 + sample;
			let noise = ((n as u32)
				.wrapping_mul(1_664_525)
				.wrapping_add(1_013_904_223)
				>> 16) as f32
				/ 65_535.0 - 0.5;
			(n as f32 * 0.047).sin() * 0.15 + noise * 0.04
		})
	}

	fn process_frame(model: &mut DfTract, input: &[f32; 480]) -> [f32; 480] {
		let mut output = [0.; 480];
		model
			.process(
				ArrayView2::from_shape((1, 480), input).unwrap(),
				ArrayViewMut2::from_shape((1, 480), &mut output).unwrap(),
			)
			.unwrap();
		assert!(output.iter().all(|sample| sample.is_finite()));
		output
	}

	#[test]
	fn reset_forgets_audio_and_recurrent_state_and_keeps_history_bounded() {
		let mut model = DfTract::new(DfParams::default(), &RuntimeParams::default()).unwrap();
		let frozen = model.freeze();
		model = std::thread::spawn(move || frozen)
			.join()
			.unwrap()
			.unfreeze();
		assert_eq!((model.sr, model.ch, model.hop_size), (48_000, 1, 480));
		let expected: Vec<_> = (0..16)
			.map(|index| process_frame(&mut model, &synthetic_frame(index)))
			.collect();
		let history_sizes = (
			model.rolling_spec_buf_x.len(),
			model.rolling_spec_buf_y.len(),
		);
		for cycle in 0..4 {
			// Contaminate STFT, feature normalization, recurrent nets and lookahead.
			for index in 0..30 {
				process_frame(&mut model, &synthetic_frame(100 + cycle * 30 + index));
			}
			// Exercise upstream's silence shortcut as well as active processing.
			for _ in 0..12 {
				process_frame(&mut model, &[0.; 480]);
			}
			model.reset().unwrap();
			assert_eq!(
				history_sizes,
				(
					model.rolling_spec_buf_x.len(),
					model.rolling_spec_buf_y.len()
				)
			);
			for (index, expected) in expected.iter().enumerate() {
				assert_eq!(
					&process_frame(&mut model, &synthetic_frame(index)),
					expected
				);
			}
		}
		model.reset().unwrap();
		assert_eq!(process_frame(&mut model, &[0.; 480]), [0.; 480]);
		assert!(model
			.process(
				ArrayView2::from_shape((1, 1), &[0.]).unwrap(),
				ArrayViewMut2::from_shape((1, 1), &mut [0.]).unwrap(),
			)
			.is_err());
	}
}
