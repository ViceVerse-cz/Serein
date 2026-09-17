//! Device I/O is created only after an explicit call reaches encrypted readiness.
//! CPAL callbacks use preallocated lock-free rings; codecs and channels stay off them.
use crate::Frame;
use crate::diagnostics::{Metrics, Scope, Stage};
mod echo;
#[cfg(target_os = "macos")]
mod permission_macos;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
	Arc,
	atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering},
	mpsc,
};
use std::time::{Duration, Instant};

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Devices {
	pub input: Option<String>,
	pub output: Option<String>,
}
pub struct DeviceList {
	pub inputs: Vec<(String, String)>,
	pub outputs: Vec<(String, String)>,
}
pub fn devices() -> Result<DeviceList, &'static str> {
	let host = cpal::default_host();
	let mut list = DeviceList {
		inputs: vec![],
		outputs: vec![],
	};
	for device in host
		.devices()
		.map_err(|_| "Audio devices are unavailable")?
		.take(64)
	{
		let Ok(id) = device.id() else {
			continue;
		};
		let id = id.to_string();
		if id.len() > 512 {
			continue;
		}
		let name: String = device.to_string().chars().take(128).collect();
		if device.supports_input() && list.inputs.len() < 32 {
			list.inputs.push((id.clone(), name.clone()));
		}
		if device.supports_output() && list.outputs.len() < 32 {
			list.outputs.push((id, name));
		}
	}
	Ok(list)
}
pub struct Gate {
	pub ready: AtomicBool,
	pub muted: AtomicBool,
	pub deafened: AtomicBool,
	stopped: AtomicBool,
	failed_revision: AtomicU64,
	input_failed_revision: AtomicU64,
	input_callbacks: AtomicU64,
	revision: AtomicU64,
	acknowledged_revision: AtomicU64,
	media_generation: AtomicU64,
	input_enabled: AtomicBool,
	input_gain: AtomicU16,
	output_gain: AtomicU16,
	echo_reset: AtomicBool,
	noise_suppression: AtomicBool,
}
impl Default for Gate {
	fn default() -> Self {
		Self {
			ready: AtomicBool::new(false),
			muted: AtomicBool::new(false),
			deafened: AtomicBool::new(false),
			stopped: AtomicBool::new(false),
			failed_revision: AtomicU64::new(0),
			input_failed_revision: AtomicU64::new(0),
			input_callbacks: AtomicU64::new(0),
			revision: AtomicU64::new(1),
			acknowledged_revision: AtomicU64::new(0),
			media_generation: AtomicU64::new(0),
			input_enabled: AtomicBool::new(true),
			input_gain: AtomicU16::new(100),
			output_gain: AtomicU16::new(100),
			echo_reset: AtomicBool::new(false),
			noise_suppression: AtomicBool::new(false),
		}
	}
}
impl Gate {
	fn is_ready(&self) -> bool {
		let revision = self.revision.load(Ordering::Acquire);
		self.ready.load(Ordering::Acquire)
			&& !self.stopped.load(Ordering::Acquire)
			&& self.failed_revision.load(Ordering::Acquire) != revision
			&& self.acknowledged_revision.load(Ordering::Acquire) == revision
	}
	fn acknowledge(&self, revision: u64) -> bool {
		if self.revision.load(Ordering::Acquire) != revision
			|| !self.ready.load(Ordering::Acquire)
			|| self.stopped.load(Ordering::Acquire)
		{
			return false;
		}
		self.acknowledged_revision
			.store(revision, Ordering::Release);
		self.is_ready()
	}
	fn capture(&self) -> bool {
		self.is_ready()
			&& self.input_enabled.load(Ordering::Acquire)
			&& self.input_failed_revision.load(Ordering::Acquire)
				!= self.revision.load(Ordering::Acquire)
			&& !self.muted.load(Ordering::Acquire)
			&& !self.stopped.load(Ordering::Acquire)
	}
	fn playback(&self) -> bool {
		self.is_ready()
			&& !self.deafened.load(Ordering::Acquire)
			&& !self.stopped.load(Ordering::Acquire)
	}
}
pub struct Audio {
	pub gate: Arc<Gate>,
	settings: tokio::sync::watch::Sender<Devices>,
	thread: std::thread::Thread,
	done: Option<mpsc::Receiver<()>>,
}
impl Audio {
	pub fn start(
		settings: Devices,
		capture: mpsc::SyncSender<Frame>,
		playback: mpsc::Receiver<Frame>,
		emit: impl Fn(Result<(), &'static str>) + Send + 'static,
	) -> Result<Self, &'static str> {
		let gate = Arc::new(Gate::default());
		let worker_gate = gate.clone();
		let (settings, mut selected) = tokio::sync::watch::channel(settings);
		let (finished, done) = mpsc::sync_channel(1);
		let thread = std::thread::Builder::new()
			.name("voice-audio".into())
			.spawn(move || {
				let mut metrics = Metrics::new(Scope::Audio);
				let mut streams: Option<(u64, Streams)> = None;
				let mut opened_at: Option<Instant> = None;
				let mut last_selection: Option<Devices> = None;
				let mut opened_once = false;
				// Unplugged headphones or a changed system default reopen devices instead of
				// ending the call; only a persistent failure is reported.
				let mut recovery_attempts = 0u8;
				let mut next_default_check = Instant::now();
				let mut next_input_retry = Instant::now() + Duration::from_secs(2);
				'audio: while !worker_gate.stopped.load(Ordering::Acquire) {
					let revision = worker_gate.revision.load(Ordering::Acquire);
					let current = selected.borrow_and_update().clone();
					if last_selection.as_ref() != Some(&current) {
						last_selection = Some(current.clone());
						recovery_attempts = 0;
					}
					if streams.is_some()
						&& opened_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5))
					{
						recovery_attempts = 0;
					}
					if streams
						.as_ref()
						.is_some_and(|(opened, _)| *opened != revision)
					{
						streams = None;
					}
					if let Some((_, active)) = &streams
						&& Instant::now() >= next_default_check
					{
						next_default_check = Instant::now() + Duration::from_secs(1);
						if active.default_changed(&current) {
							worker_gate.revision.fetch_add(1, Ordering::AcqRel);
							streams = None;
							continue;
						}
					}
					if !worker_gate.ready.load(Ordering::Acquire) {
						// An established stream can stay open across a brief MLS rekey.
						// The callback gate silences it, and the generation flushes old PCM.
						for _ in 0..8 {
							if playback.try_recv().is_err() {
								break;
							}
						}
						metrics.poll(false, 0, false, 0);
						std::thread::park_timeout(Duration::from_millis(10));
						continue;
					}
					if worker_gate.failed_revision.load(Ordering::Acquire) == revision
						&& worker_gate.revision.load(Ordering::Acquire) == revision
					{
						if recovery_attempts >= MAX_RECOVERY_ATTEMPTS {
							emit(Err(
								"Audio device stopped or disconnected; choose a device and call again",
							));
							break;
						}
						recovery_attempts += 1;
						worker_gate.revision.fetch_add(1, Ordering::AcqRel);
						streams = None;
						std::thread::park_timeout(RECOVERY_DELAY);
						continue;
					}
					if streams.is_none() {
						match Streams::open_with_fallback(
							&current,
							worker_gate.clone(),
							revision,
							recovery_attempts > 0,
						) {
							Ok(value) => {
								if !worker_gate.acknowledge(revision) {
									continue;
								}
								streams = Some((revision, value));
								opened_at = Some(Instant::now());
								opened_once = true;
								emit(Ok(()));
							}
							Err(error) => {
								if worker_gate.revision.load(Ordering::Acquire) != revision
									|| !worker_gate.ready.load(Ordering::Acquire)
								{
									continue;
								}
								if opened_once && recovery_attempts < MAX_RECOVERY_ATTEMPTS {
									recovery_attempts += 1;
									std::thread::park_timeout(RECOVERY_DELAY);
									continue;
								}
								emit(Err(error));
								break;
							}
						}
					}
					let Some((_, active)) = &mut streams else {
						continue;
					};
					let callbacks = worker_gate.input_callbacks.load(Ordering::Acquire);
					if callbacks != active.input_callbacks {
						active.input_callbacks = callbacks;
						active.input_activity = Instant::now();
					}
					if active._input.is_some()
						&& active.input_activity.elapsed() >= Duration::from_secs(5)
					{
						worker_gate
							.input_failed_revision
							.fetch_max(revision, Ordering::AcqRel);
					}
					if worker_gate.input_failed_revision.load(Ordering::Acquire) == revision
						&& active._input.is_some()
					{
						active._input = None;
						active.input_id = None;
						next_input_retry = Instant::now() + Duration::from_secs(2);
						worker_gate.echo_reset.store(true, Ordering::Release);
						emit(Ok(())); // Wake the UI for the recoverable microphone warning.
					}
					if active._input.is_none()
						&& worker_gate.input_enabled.load(Ordering::Acquire)
						&& worker_gate.ready.load(Ordering::Acquire)
						&& Instant::now() >= next_input_retry
					{
						next_input_retry = Instant::now() + Duration::from_secs(2);
						let host = cpal::default_host();
						if active.try_reopen_input(&host, &current, &worker_gate, revision) {
							emit(Ok(()));
						}
					}
					let reset = worker_gate.echo_reset.swap(false, Ordering::AcqRel);
					let mut drops = 0;
					let mut noise_frames = 0;
					if reset {
						active.echo = echo::Echo::new();
						for _ in 0..8 {
							let _ = active.input.pop();
							let _ = active.reference.pop();
						}
					}
					let noise = worker_gate.noise_suppression.load(Ordering::Acquire);
					active.echo.set_noise_suppression(noise);
					for _ in 0..8 {
						let Ok(frame) = active.reference.pop() else {
							break;
						};
						if worker_gate.capture() {
							let start = metrics.start();
							let result = active.echo.render(&frame);
							metrics.finish(Stage::EchoRender, start);
							if let Err(error) = result {
								emit(Err(error));
								break 'audio;
							}
						}
					}
					for _ in 0..8 {
						let Ok(mut frame) = active.input.pop() else {
							break;
						};
						if worker_gate.capture() {
							let start = metrics.start();
							let result = active.echo.capture(&mut frame, start.is_some());
							metrics.finish(Stage::EchoCapture, start);
							noise_frames += u64::from(noise);
							match result {
								Ok(noise_time) => metrics.add(Stage::Noise, noise_time),
								Err(error) => {
									emit(Err(error));
									break 'audio;
								}
							}
							let gain =
								f32::from(worker_gate.input_gain.load(Ordering::Relaxed)) / 100.0;
							for sample in &mut frame {
								*sample = amplify(*sample, gain);
							}
							if worker_gate.capture()
								&& !worker_gate.echo_reset.load(Ordering::Acquire)
							{
								drops += u64::from(capture.try_send(frame).is_err());
							}
						}
					}
					for _ in 0..8 {
						let Ok(frame) = playback.try_recv() else {
							break;
						};
						if worker_gate.playback() {
							drops += u64::from(active.output.push(frame).is_err());
						}
					}
					metrics.poll(reset, drops, false, noise_frames);
					std::thread::park_timeout(Duration::from_millis(5));
				}
				worker_gate.stopped.store(true, Ordering::Release);
				drop(streams);
				let _ = finished.send(());
			})
			.map_err(|_| "Could not start audio device worker")?;
		Ok(Self {
			gate,
			settings,
			thread: thread.thread().clone(),
			done: Some(done),
		})
	}
	/// The worker ended. A failing device alone is not final: the worker reopens it first.
	pub fn is_stopped(&self) -> bool {
		self.gate.stopped.load(Ordering::Acquire)
	}
	pub fn shutdown(mut self) -> mpsc::Receiver<()> {
		self.done.take().expect("audio owns completion")
	}
	pub fn set_devices(&self, settings: Devices) {
		self.settings.send_if_modified(|current| {
			if *current == settings {
				return false;
			}
			self.gate.revision.fetch_add(1, Ordering::AcqRel);
			*current = settings;
			true
		});
		self.thread.unpark();
	}
	pub fn set_ready(&self, ready: bool) {
		let established = self.gate.is_ready();
		if self.gate.ready.swap(ready, Ordering::AcqRel) && !ready {
			self.gate.media_generation.fetch_add(1, Ordering::AcqRel);
			self.gate.echo_reset.store(true, Ordering::Release);
			if !established {
				// An in-flight open must not acknowledge a later security epoch.
				self.gate.revision.fetch_add(1, Ordering::AcqRel);
			}
		}
		self.thread.unpark();
	}
	/// Permission-driven microphone availability, independent of mute and push-to-talk.
	pub fn set_input_enabled(&self, enabled: bool) {
		if self.gate.input_enabled.swap(enabled, Ordering::AcqRel) != enabled {
			self.gate.revision.fetch_add(1, Ordering::AcqRel);
			self.thread.unpark();
		}
	}
	/// True only after streams for the current device/security revision have opened.
	pub fn is_ready(&self) -> bool {
		self.gate.is_ready()
	}
	pub fn microphone_unavailable(&self) -> bool {
		self.gate.input_enabled.load(Ordering::Acquire)
			&& self.gate.input_failed_revision.load(Ordering::Acquire)
				== self.gate.revision.load(Ordering::Acquire)
	}
	pub fn set_controls(&self, muted: bool, deafened: bool) {
		let mute_changed =
			self.gate.muted.swap(muted || deafened, Ordering::AcqRel) != (muted || deafened);
		let deafen_changed = self.gate.deafened.swap(deafened, Ordering::AcqRel) != deafened;
		if mute_changed || deafen_changed {
			self.gate.echo_reset.store(true, Ordering::Release);
		}
	}
	/// Applies on the audio worker without restarting devices or resetting echo cancellation.
	pub fn set_noise_suppression(&self, enabled: bool) {
		if self.gate.noise_suppression.swap(enabled, Ordering::AcqRel) != enabled {
			self.thread.unpark();
		}
	}
	/// Adjusts software gain without reopening devices. Defaults to 100%; clamps to 0..=200%.
	pub fn set_gain(&self, input_percent: u16, output_percent: u16) {
		self.gate
			.input_gain
			.store(input_percent.min(200), Ordering::Relaxed);
		self.gate
			.output_gain
			.store(output_percent.min(200), Ordering::Relaxed);
	}
}
impl Drop for Audio {
	fn drop(&mut self) {
		self.gate.stopped.store(true, Ordering::Release);
		self.gate.ready.store(false, Ordering::Release);
		self.thread.unpark();
	}
}

/// Six seconds of reopen attempts before a device loss ends the call.
const MAX_RECOVERY_ATTEMPTS: u8 = 24;
const RECOVERY_DELAY: Duration = Duration::from_millis(250);

struct Streams {
	input_callbacks: u64,
	input_activity: Instant,
	_input: Option<cpal::Stream>,
	_output: cpal::Stream,
	input: rtrb::Consumer<Frame>,
	output: rtrb::Producer<Frame>,
	reference: rtrb::Consumer<Frame>,
	echo: echo::Echo,
	/// Devices actually opened, so a changed system default can be followed.
	output_id: Option<String>,
	input_id: Option<String>,
}
impl Streams {
	fn open_with_fallback(
		settings: &Devices,
		gate: Arc<Gate>,
		revision: u64,
		prefer_default: bool,
	) -> Result<Self, &'static str> {
		let mut candidates = [
			settings.clone(),
			Devices {
				input: settings.input.clone(),
				output: None,
			},
		];
		if prefer_default {
			candidates.rotate_right(1);
		}
		let mut attempted = Vec::with_capacity(candidates.len());
		let mut last_error = "No default audio device is available";
		for candidate in candidates {
			if attempted.contains(&candidate) {
				continue;
			}
			attempted.push(candidate.clone());
			match Self::open(&candidate, gate.clone(), revision) {
				Ok(streams) => return Ok(streams),
				Err(error) => last_error = error,
			}
			if gate.stopped.load(Ordering::Acquire)
				|| !gate.ready.load(Ordering::Acquire)
				|| gate.revision.load(Ordering::Acquire) != revision
			{
				break;
			}
		}
		Err(last_error)
	}
	/// True when the call follows the system default and that default now points elsewhere.
	fn default_changed(&self, settings: &Devices) -> bool {
		let host = cpal::default_host();
		let id = |device: Option<cpal::Device>| {
			device.and_then(|d| d.id().ok()).map(|id| id.to_string())
		};
		(settings.output.is_none()
			&& self.output_id.is_some()
			&& id(host.default_output_device()) != self.output_id)
			|| (settings.input.is_none()
				&& self.input_id.is_some()
				&& id(host.default_input_device()) != self.input_id)
	}
	fn open(settings: &Devices, gate: Arc<Gate>, revision: u64) -> Result<Self, &'static str> {
		if gate.stopped.load(Ordering::Acquire)
			|| !gate.ready.load(Ordering::Acquire)
			|| gate.revision.load(Ordering::Acquire) != revision
		{
			return Err("Call changed before audio devices could open");
		}
		let host = cpal::default_host();
		let output = choose(&host, settings.output.as_deref(), false)?;
		let output_id = output.id().ok().map(|id| id.to_string());
		let output_config = config(&output, false)?;
		let (output_write, output_read) = rtrb::RingBuffer::new(8);
		let (reference_write, reference_read) = rtrb::RingBuffer::new(8);
		let render = Playback::new(output_config.sample_rate(), output_read, reference_write);
		let echo = echo::Echo::new();
		let (input_stream, input_read, input_id) = if gate.input_enabled.load(Ordering::Acquire) {
			match open_input_stream(&host, settings, &gate, revision) {
				Ok((stream, read, id)) => (Some(stream), read, id),
				Err(_) => {
					gate.input_failed_revision
						.fetch_max(revision, Ordering::AcqRel);
					(None, rtrb::RingBuffer::new(8).1, None)
				}
			}
		} else {
			(None, rtrb::RingBuffer::new(8).1, None)
		};
		let output_stream = match output_config.sample_format() {
			cpal::SampleFormat::F32 => output_stream::<f32>(
				&output,
				&output_config.config(),
				render,
				gate.clone(),
				revision,
			),
			cpal::SampleFormat::I16 => output_stream::<i16>(
				&output,
				&output_config.config(),
				render,
				gate.clone(),
				revision,
			),
			cpal::SampleFormat::I32 => output_stream::<i32>(
				&output,
				&output_config.config(),
				render,
				gate.clone(),
				revision,
			),
			cpal::SampleFormat::U16 => output_stream::<u16>(
				&output,
				&output_config.config(),
				render,
				gate.clone(),
				revision,
			),
			_ => Err("Speaker sample format is not supported"),
		}?;
		if !gate.ready.load(Ordering::Acquire)
			|| gate.stopped.load(Ordering::Acquire)
			|| gate.revision.load(Ordering::Acquire) != revision
		{
			return Err("Call ended before audio devices were ready");
		}
		output_stream
			.play()
			.map_err(|_| "Could not start speaker playback")?;
		Ok(Self {
			input_callbacks: gate.input_callbacks.load(Ordering::Acquire),
			input_activity: Instant::now(),
			_input: input_stream,
			_output: output_stream,
			input: input_read,
			output: output_write,
			reference: reference_read,
			echo,
			output_id,
			input_id,
		})
	}
	fn try_reopen_input(
		&mut self,
		host: &cpal::Host,
		settings: &Devices,
		gate: &Arc<Gate>,
		revision: u64,
	) -> bool {
		match open_input_stream(host, settings, gate, revision) {
			Ok((stream, input_read, id)) => {
				self._input = Some(stream);
				self.input = input_read;
				self.input_id = id;
				self.input_callbacks = gate.input_callbacks.load(Ordering::Acquire);
				self.input_activity = Instant::now();
				gate.input_failed_revision.store(0, Ordering::Release);
				gate.echo_reset.store(true, Ordering::Release);
				true
			}
			Err(_) => {
				gate.input_failed_revision
					.fetch_max(revision, Ordering::AcqRel);
				false
			}
		}
	}
}
fn open_input_stream(
	host: &cpal::Host,
	settings: &Devices,
	gate: &Arc<Gate>,
	revision: u64,
) -> Result<(cpal::Stream, rtrb::Consumer<Frame>, Option<String>), &'static str> {
	#[cfg(target_os = "macos")]
	permission_macos::authorize(gate, revision)?;
	let input = choose(host, settings.input.as_deref(), true)?;
	let input_id = input.id().ok().map(|id| id.to_string());
	let input_config = config(&input, true)?;
	let (input_write, input_read) = rtrb::RingBuffer::new(8);
	let capture = Capture::new(input_config.sample_rate(), input_write);
	let stream = match input_config.sample_format() {
		cpal::SampleFormat::F32 => input_stream::<f32>(
			&input,
			&input_config.config(),
			capture,
			gate.clone(),
			revision,
		),
		cpal::SampleFormat::I16 => input_stream::<i16>(
			&input,
			&input_config.config(),
			capture,
			gate.clone(),
			revision,
		),
		cpal::SampleFormat::I32 => input_stream::<i32>(
			&input,
			&input_config.config(),
			capture,
			gate.clone(),
			revision,
		),
		cpal::SampleFormat::U16 => input_stream::<u16>(
			&input,
			&input_config.config(),
			capture,
			gate.clone(),
			revision,
		),
		_ => Err("Microphone sample format is not supported"),
	}?;
	stream
		.play()
		.map_err(|_| "Could not start microphone; check system microphone permission")?;
	Ok((stream, input_read, input_id))
}
fn choose(host: &cpal::Host, id: Option<&str>, input: bool) -> Result<cpal::Device, &'static str> {
	if let Some(id) = id {
		let id = id.parse().map_err(|_| "Invalid audio device selection")?;
		if let Some(device) = host.device_by_id(&id) {
			return Ok(device);
		}
		if input {
			return Err("Selected microphone is unavailable");
		}
	}
	// Keep the preference, but use the default while the selected device is absent.
	if input {
		host.default_input_device()
	} else {
		host.default_output_device()
	}
	.ok_or("No default audio device is available")
}
fn config(device: &cpal::Device, input: bool) -> Result<cpal::SupportedStreamConfig, &'static str> {
	let supported: Vec<_> = if input {
		device
			.supported_input_configs()
			.map_err(|_| "Microphone formats are unavailable")?
			.take(64)
			.collect()
	} else if cfg!(target_os = "windows") {
		// WASAPI advertises converted formats too; enumeration order can pick
		// 16-bit PCM that a driver rejects. Use its native shared-mode mix format
		// below and let Playback handle conversion from our 48 kHz frames.
		Vec::new()
	} else {
		device
			.supported_output_configs()
			.map_err(|_| "Speaker formats are unavailable")?
			.take(64)
			.collect()
	};
	if let Some(config) = supported
		.into_iter()
		.filter(|c| c.channels() > 0 && c.channels() <= 8)
		.filter(|c| supported_format(c.sample_format()))
		.filter_map(|c| c.try_with_sample_rate(48_000))
		.min_by_key(|c| c.channels())
	{
		return Ok(config);
	}
	let config = if input {
		device.default_input_config()
	} else {
		device.default_output_config()
	}
	.map_err(|_| "Default audio format is unavailable")?;
	if config.channels() == 0
		|| config.channels() > 8
		|| !(8_000..=192_000).contains(&config.sample_rate())
		|| !supported_format(config.sample_format())
	{
		return Err("Audio device format is unsupported; choose another device");
	}
	Ok(config)
}
fn supported_format(format: cpal::SampleFormat) -> bool {
	matches!(
		format,
		cpal::SampleFormat::F32
			| cpal::SampleFormat::I16
			| cpal::SampleFormat::I32
			| cpal::SampleFormat::U16
	)
}
fn is_fatal_error(error: &cpal::Error) -> bool {
	!matches!(
		error.kind(),
		cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied | cpal::ErrorKind::DeviceChanged
	)
}
fn input_stream<T>(
	device: &cpal::Device,
	config: &cpal::StreamConfig,
	mut capture: Capture,
	gate: Arc<Gate>,
	revision: u64,
) -> Result<cpal::Stream, &'static str>
where
	T: cpal::SizedSample,
	f32: cpal::FromSample<T>,
{
	let channels = usize::from(config.channels);
	let failure = gate.clone();
	device
		.build_input_stream(
			*config,
			move |data: &[T], _| {
				if !data.is_empty() {
					gate.input_callbacks.fetch_add(1, Ordering::Release);
				}
				capture.process(data, channels, &gate);
			},
			move |error| {
				if is_fatal_error(&error) {
					failure
						.input_failed_revision
						.fetch_max(revision, Ordering::AcqRel);
				}
			},
			None,
		)
		.map_err(|_| "Could not open microphone; check device and microphone permission")
}
fn output_stream<T>(
	device: &cpal::Device,
	config: &cpal::StreamConfig,
	mut output: Playback,
	gate: Arc<Gate>,
	revision: u64,
) -> Result<cpal::Stream, &'static str>
where
	T: cpal::SizedSample + cpal::FromSample<f32>,
{
	let channels = usize::from(config.channels);
	let failure = gate.clone();
	device
		.build_output_stream(
			*config,
			move |data: &mut [T], _| {
				output.render(data, channels, &gate);
			},
			move |error| {
				if is_fatal_error(&error) {
					failure
						.failed_revision
						.fetch_max(revision, Ordering::AcqRel);
				}
			},
			None,
		)
		.map_err(|error| match error.kind() {
			cpal::ErrorKind::DeviceBusy => {
				"Could not open speaker device: device is busy; close other audio apps and retry"
			}
			cpal::ErrorKind::DeviceNotAvailable | cpal::ErrorKind::StreamInvalidated => {
				"Could not open speaker device: device disconnected or changed; select another output"
			}
			cpal::ErrorKind::UnsupportedConfig => {
				"Could not open speaker device: audio format is unsupported; check system sound settings"
			}
			cpal::ErrorKind::PermissionDenied => {
				"Could not open speaker device: system denied audio access"
			}
			_ => "Could not open speaker device: audio driver failed; check system sound settings",
		})
}
fn amplify(sample: f32, gain: f32) -> f32 {
	if sample.is_finite() {
		(sample * gain).clamp(-1.0, 1.0)
	} else {
		0.0
	}
}
// ponytail: linear conversion is a fallback for devices lacking 48 kHz; use a band-limited resampler if aliasing is measured to matter.
struct Capture {
	media_generation: u64,
	previous: Option<f32>,
	phase: f64,
	step: f64,
	frame: Frame,
	index: usize,
	output: rtrb::Producer<Frame>,
	overrun: bool,
}
impl Capture {
	fn process<T: cpal::SizedSample>(&mut self, data: &[T], channels: usize, gate: &Gate)
	where
		f32: cpal::FromSample<T>,
	{
		let generation = gate.media_generation.load(Ordering::Acquire);
		if generation != self.media_generation {
			self.media_generation = generation;
			self.reset();
		}
		if !gate.capture() {
			self.reset();
			return;
		}
		for frame in data.chunks_exact(channels) {
			let sample = frame.iter().map(|v| v.to_sample::<f32>()).sum::<f32>() / channels as f32;
			// Keep microphone gain after AEC to avoid clipping the echo before cancellation.
			self.sample(amplify(sample, 1.0));
		}
		if std::mem::take(&mut self.overrun) {
			gate.echo_reset.store(true, Ordering::Release);
		}
	}
	fn new(rate: u32, output: rtrb::Producer<Frame>) -> Self {
		Self {
			media_generation: 0,
			previous: None,
			phase: 0.0,
			step: f64::from(rate) / 48_000.0,
			frame: [0.0; 960],
			index: 0,
			output,
			overrun: false,
		}
	}
	fn reset(&mut self) {
		self.previous = None;
		self.phase = 0.0;
		self.index = 0;
		self.frame.fill(0.0);
	}
	fn sample(&mut self, sample: f32) {
		if let Some(previous) = self.previous {
			while self.phase < 1.0 {
				self.frame[self.index] = previous + (sample - previous) * self.phase as f32;
				self.index += 1;
				if self.index == 960 {
					self.overrun |= self.output.push(self.frame).is_err();
					self.index = 0;
				}
				self.phase += self.step;
			}
			self.phase -= 1.0;
		}
		self.previous = Some(sample);
	}
}
struct Playback {
	media_generation: u64,
	input: rtrb::Consumer<Frame>,
	reference: Capture,
	frame: Frame,
	index: usize,
	previous: f32,
	next: f32,
	phase: f64,
	step: f64,
}
impl Playback {
	fn render<T: cpal::SizedSample + cpal::FromSample<f32>>(
		&mut self,
		data: &mut [T],
		channels: usize,
		gate: &Gate,
	) {
		let generation = gate.media_generation.load(Ordering::Acquire);
		if generation != self.media_generation {
			self.media_generation = generation;
			self.reset();
		}
		if !gate.playback() {
			self.reset();
			data.fill(T::from_sample(0.0));
			for _ in data.chunks_exact(channels) {
				self.reference.sample(0.0);
			}
		} else {
			let gain = f32::from(gate.output_gain.load(Ordering::Relaxed)) / 100.0;
			for frame in data.chunks_mut(channels) {
				let sample = amplify(self.sample(), gain);
				frame.fill(T::from_sample(sample));
				// Reference actual output, including gain, underrun silence and resampling.
				self.reference.sample(sample);
			}
		}
		if std::mem::take(&mut self.reference.overrun) {
			gate.echo_reset.store(true, Ordering::Release);
		}
	}
	fn new(rate: u32, input: rtrb::Consumer<Frame>, reference: rtrb::Producer<Frame>) -> Self {
		Self {
			media_generation: 0,
			input,
			reference: Capture::new(rate, reference),
			frame: [0.0; 960],
			index: 960,
			previous: 0.0,
			next: 0.0,
			phase: 1.0,
			step: 48_000.0 / f64::from(rate),
		}
	}
	fn reset(&mut self) {
		for _ in 0..8 {
			if self.input.pop().is_err() {
				break;
			}
		}
		self.reference.reset();
		self.frame.fill(0.0);
		self.index = 960;
		self.previous = 0.0;
		self.next = 0.0;
		self.phase = 1.0;
	}
	fn pull(&mut self) -> f32 {
		if self.index == 960 {
			self.frame = self.input.pop().unwrap_or([0.0; 960]);
			self.index = 0;
		}
		let value = self.frame[self.index];
		self.index += 1;
		if value.is_finite() {
			value.clamp(-1.0, 1.0)
		} else {
			0.0
		}
	}
	fn sample(&mut self) -> f32 {
		while self.phase >= 1.0 {
			self.previous = self.next;
			self.next = self.pull();
			self.phase -= 1.0;
		}
		let value = self.previous + (self.next - self.previous) * self.phase as f32;
		self.phase += self.step;
		value
	}
}
#[cfg(test)]
mod tests {
	use super::*;
	fn audio_without_devices() -> Audio {
		let (settings, _) = tokio::sync::watch::channel(Devices::default());
		Audio {
			gate: Arc::new(Gate::default()),
			settings,
			thread: std::thread::current(),
			done: None,
		}
	}

	#[test]
	fn device_readiness_rejects_stale_open_and_fast_security_transitions() {
		let audio = audio_without_devices();
		audio.set_ready(true);
		let first = audio.gate.revision.load(Ordering::Acquire);
		assert!(!audio.is_ready());
		assert!(audio.gate.acknowledge(first));
		assert!(audio.is_ready() && audio.gate.capture() && audio.gate.playback());
		audio.set_ready(false);
		assert!(!audio.is_ready() && !audio.gate.capture() && !audio.gate.playback());
		audio.set_ready(true); // Worker has not observed the intervening pause.
		let second = audio.gate.revision.load(Ordering::Acquire);
		assert_eq!(first, second); // Established devices survive a security pause.
		assert!(audio.is_ready());
		assert_eq!(audio.gate.media_generation.load(Ordering::Acquire), 1);
		// A pause while devices are still opening must invalidate their late result.
		audio.gate.acknowledged_revision.store(0, Ordering::Release);
		audio.set_ready(false);
		audio.set_ready(true);
		let pending = audio.gate.revision.load(Ordering::Acquire);
		assert_ne!(second, pending);
		assert!(!audio.gate.acknowledge(second));
		assert!(audio.gate.acknowledge(pending));
		audio.set_ready(true);
		assert_eq!(audio.gate.revision.load(Ordering::Acquire), pending);
		audio.set_devices(Devices::default());
		assert_eq!(audio.gate.revision.load(Ordering::Acquire), pending);
		audio.set_devices(Devices {
			input: Some("synthetic-device".into()),
			output: None,
		});
		let third = audio.gate.revision.load(Ordering::Acquire);
		assert_ne!(pending, third);
		assert!(!audio.is_ready() && !audio.gate.capture());
		assert!(!audio.gate.acknowledge(pending));
		audio.gate.failed_revision.store(pending, Ordering::Release);
		assert!(!audio.is_stopped()); // A retired device cannot fail the new configuration.
		assert!(audio.gate.acknowledge(third));
		assert!(audio.is_ready());
		audio.gate.failed_revision.store(third, Ordering::Release);
		// A failing device pauses media for the reopen; only the worker ending is final.
		assert!(!audio.is_stopped());
		assert!(!audio.is_ready());
	}

	#[test]
	fn listen_only_keeps_playback_without_input_and_mute_does_not_reopen_devices() {
		let audio = audio_without_devices();
		audio.set_input_enabled(false);
		audio.set_ready(true);
		let revision = audio.gate.revision.load(Ordering::Acquire);
		assert!(audio.gate.acknowledge(revision));
		assert!(audio.is_ready() && audio.gate.playback());
		assert!(!audio.gate.capture());
		let (send, mut received) = rtrb::RingBuffer::new(8);
		let mut capture = Capture::new(48_000, send);
		capture.process(&[0.25_f32; 961], 1, &audio.gate);
		assert!(received.pop().is_err());
		audio.set_controls(true, false);
		audio.set_controls(false, false);
		audio.set_input_enabled(false);
		assert_eq!(audio.gate.revision.load(Ordering::Acquire), revision);
		assert!(!audio.gate.capture() && audio.gate.playback());
		audio.set_input_enabled(true);
		let next = audio.gate.revision.load(Ordering::Acquire);
		assert_ne!(next, revision);
		assert!(!audio.is_ready() && !audio.gate.capture());
		assert!(audio.gate.acknowledge(next));
		assert!(audio.gate.capture() && audio.gate.playback());
	}

	#[test]
	fn callback_gain_defaults_clamps_and_sanitizes_without_devices() {
		let audio = audio_without_devices();
		assert_eq!(audio.gate.input_gain.load(Ordering::Relaxed), 100);
		assert_eq!(audio.gate.output_gain.load(Ordering::Relaxed), 100);
		audio.set_ready(true);
		assert!(
			audio
				.gate
				.acknowledge(audio.gate.revision.load(Ordering::Acquire))
		);
		for (percent, expected) in [(0, 0.0), (100, 0.75), (200, 1.0), (u16::MAX, 1.0)] {
			audio.set_gain(percent, percent);
			for sample in [0.75_f32, -0.75] {
				let expected = expected * sample.signum();
				let (send, mut receive) = rtrb::RingBuffer::new(8);
				let mut capture = Capture::new(48_000, send);
				capture.process(&[sample; 961], 1, &audio.gate);
				assert!(receive.pop().unwrap().iter().all(|s| *s == sample));
				assert_eq!(
					amplify(sample, f32::from(percent.min(200)) / 100.0),
					expected
				);
				let (mut send, receive) = rtrb::RingBuffer::new(8);
				send.push([sample; 960]).unwrap();
				let mut playback = Playback::new(48_000, receive, rtrb::RingBuffer::new(8).0);
				let mut rendered = [0.0_f32; 1920];
				playback.render(&mut rendered, 2, &audio.gate);
				assert_eq!(&rendered[..2], &[0.0, 0.0]);
				assert!(rendered[2..].iter().all(|s| *s == expected));
			}
		}
		for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
			let (send, mut receive) = rtrb::RingBuffer::new(8);
			let mut capture = Capture::new(48_000, send);
			capture.process(&[invalid; 961], 1, &audio.gate);
			assert_eq!(receive.pop().unwrap(), [0.0; 960]);
			let (mut send, receive) = rtrb::RingBuffer::new(8);
			send.push([invalid; 960]).unwrap();
			let mut playback = Playback::new(48_000, receive, rtrb::RingBuffer::new(8).0);
			let mut rendered = [1.0_f32; 960];
			playback.render(&mut rendered, 1, &audio.gate);
			assert_eq!(rendered, [0.0; 960]);
		}
	}

	#[test]
	fn runtime_gain_updates_preserve_gates_and_discard_buffered_audio() {
		let audio = audio_without_devices();
		let (capture_send, mut captured) = rtrb::RingBuffer::new(8);
		let mut capture = Capture::new(48_000, capture_send);
		let (mut playback_send, playback_receive) = rtrb::RingBuffer::new(8);
		let mut playback = Playback::new(48_000, playback_receive, rtrb::RingBuffer::new(8).0);
		let mut rendered = [1.0_f32; 2];
		playback_send.push([0.25; 960]).unwrap();
		capture.process(&[0.25; 961], 1, &audio.gate);
		playback.render(&mut rendered, 1, &audio.gate);
		assert!(captured.pop().is_err());
		assert_eq!(rendered, [0.0; 2]);
		audio.set_ready(true);
		assert!(
			audio
				.gate
				.acknowledge(audio.gate.revision.load(Ordering::Acquire))
		);
		playback.render(&mut rendered, 1, &audio.gate);
		assert_eq!(rendered, [0.0; 2]);

		// The same callback state observes new controls; no stream needs recreation.
		playback.reset();
		playback_send.push([0.25; 960]).unwrap();
		playback.render(&mut rendered, 1, &audio.gate);
		assert_eq!(rendered, [0.0, 0.25]);
		audio.set_gain(200, 0);
		capture.process(&[0.25; 961], 1, &audio.gate);
		assert_eq!(captured.pop().unwrap(), [0.25; 960]);
		playback.render(&mut rendered, 1, &audio.gate);
		assert_eq!(rendered, [0.0; 2]);
		audio.set_gain(0, 200);
		capture.process(&[0.25; 960], 1, &audio.gate);
		let frame = captured.pop().unwrap();
		assert_eq!(frame, [0.25; 960]); // Microphone gain now follows AEC on the worker.
		playback.render(&mut rendered, 1, &audio.gate);
		assert_eq!(rendered, [0.5; 2]);

		audio.set_controls(true, false);
		capture.process(&[0.75; 961], 1, &audio.gate);
		assert!(captured.pop().is_err());
		playback.render(&mut rendered, 1, &audio.gate);
		assert_eq!(rendered, [0.5; 2]);
		audio.set_controls(false, true);
		playback_send.push([0.75; 960]).unwrap();
		capture.process(&[0.75; 961], 1, &audio.gate);
		playback.render(&mut rendered, 1, &audio.gate);
		assert!(captured.pop().is_err());
		assert_eq!(rendered, [0.0; 2]);
		audio.set_controls(false, false);
		audio.set_gain(100, 100);
		capture.process(&[0.25; 961], 1, &audio.gate);
		assert_eq!(captured.pop().unwrap(), [0.25; 960]);
		playback.render(&mut rendered, 1, &audio.gate);
		assert_eq!(rendered, [0.0; 2]);
		audio.gate.stopped.store(true, Ordering::Release);
		capture.process(&[0.75; 961], 1, &audio.gate);
		playback_send.push([0.75; 960]).unwrap();
		playback.render(&mut rendered, 1, &audio.gate);
		assert!(captured.pop().is_err());
		assert_eq!(rendered, [0.0; 2]);
	}

	#[test]
	fn resampling_buffers_and_capture_gate_are_bounded_without_devices() {
		let gate = Gate::default();
		assert!(!gate.capture());
		gate.ready.store(true, Ordering::Release);
		assert!(gate.acknowledge(gate.revision.load(Ordering::Acquire)));
		assert!(gate.capture());
		gate.muted.store(true, Ordering::Release);
		assert!(!gate.capture());
		assert!(gate.playback());
		gate.deafened.store(true, Ordering::Release);
		assert!(!gate.playback());
		for rate in [44_100, 48_000, 96_000] {
			let (send, mut receive) = rtrb::RingBuffer::new(8);
			let mut capture = Capture::new(rate, send);
			let mut count = 0;
			for _ in 0..rate {
				capture.sample(0.25);
				while let Ok(frame) = receive.pop() {
					assert!(frame.iter().all(|s| (*s - 0.25).abs() < 0.0001));
					count += 960;
				}
			}
			assert!((47_040..=48_000).contains(&count));
		}
		let (mut send, receive) = rtrb::RingBuffer::new(8);
		for _ in 0..8 {
			send.push([0.5; 960]).unwrap();
		}
		assert!(send.push([0.5; 960]).is_err());
		let mut playback = Playback::new(44_100, receive, rtrb::RingBuffer::new(8).0);
		for _ in 0..7000 {
			assert!(playback.sample().is_finite());
		}
		playback.reset();
		assert_eq!(playback.sample(), 0.0);
	}

	#[test]
	fn stream_errors_distinguish_transient_glitches_from_fatal_disconnects() {
		for non_fatal in [
			cpal::ErrorKind::Xrun,
			cpal::ErrorKind::RealtimeDenied,
			cpal::ErrorKind::DeviceChanged,
		] {
			assert!(!is_fatal_error(&cpal::Error::from(non_fatal)));
		}
		for fatal in [
			cpal::ErrorKind::DeviceNotAvailable,
			cpal::ErrorKind::StreamInvalidated,
			cpal::ErrorKind::PermissionDenied,
			cpal::ErrorKind::DeviceBusy,
		] {
			assert!(is_fatal_error(&cpal::Error::from(fatal)));
		}
	}

	#[test]
	fn microphone_unavailability_clears_on_recovery() {
		let audio = audio_without_devices();
		audio.set_ready(true);
		let revision = audio.gate.revision.load(Ordering::Acquire);
		assert!(audio.gate.acknowledge(revision));
		assert!(!audio.microphone_unavailable());
		// Simulating fatal input error marks input_failed_revision.
		audio
			.gate
			.input_failed_revision
			.store(revision, Ordering::Release);
		assert!(audio.microphone_unavailable());
		// Simulating successful auto-recovery clears input_failed_revision.
		audio.gate.input_failed_revision.store(0, Ordering::Release);
		assert!(!audio.microphone_unavailable());
	}
}
