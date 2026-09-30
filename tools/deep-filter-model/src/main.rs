//! Offline conversion of the upstream DeepFilterNet3 ONNX release into the Tract NNEF
//! archive embedded by `vendor/deep-filter`: `cargo run -p deep-filter-model`.
//!
//! The app links only Tract core, NNEF and pulse operators. Parsing ONNX, type inference
//! and pulsification happen here, once, with the exact graph setup upstream performs at
//! load time. Output is deterministic, so `embedded_nnef_matches_upstream_onnx` proves the
//! committed archive is exactly this conversion of the committed upstream model.
use std::io::{Cursor, Read};

use anyhow::{Context, Result};
use flate2::{Compression, GzBuilder, read::GzDecoder, write::GzEncoder};
use ini::Ini;
use tar::{Archive, Builder, Header};
use tract_nnef::internal::*;
use tract_onnx::{prelude::*, tract_hir::shapefactoid};
use tract_onnx_opl::WithOnnx;
use tract_pulse::model::{PulsedModel, PulsedModelExt};
use tract_pulse_opl::{WithPulse, ops::Delay};

/// Upstream `v0.5.6` model archive; converted, never embedded.
const UPSTREAM_ONNX: &str = concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../vendor/deep-filter/models/DeepFilterNet3_onnx.tar.gz"
);
/// Embedded runtime archive produced by [`nnef_archive_from_onnx`].
const EMBEDDED_NNEF: &str = concat!(
	env!("CARGO_MANIFEST_DIR"),
	"/../../vendor/deep-filter/models/DeepFilterNet3_nnef.tar.gz"
);

fn main() -> Result<()> {
	let archive = nnef_archive_from_onnx(std::fs::File::open(UPSTREAM_ONNX)?)?;
	std::fs::write(EMBEDDED_NNEF, &archive)?;
	println!("Wrote {} bytes to {EMBEDDED_NNEF}", archive.len());
	Ok(())
}
/// MS-DOS epoch, matching Tract's deterministic NNEF writer.
const MTIME: u64 = 315_532_800;

/// Tract 0.22.4 can load `tract_pulse_delay` but ships no serializer for it.
fn ser_delay(ast: &mut IntoAst, node: &TypedNode, op: &Delay) -> TractResult<Option<Arc<RValue>>> {
	let wire = ast.mapping[&node.inputs[0]].clone();
	Ok(Some(invocation(
		"tract_pulse_delay",
		&[wire],
		&[
			("axis", numeric(op.axis)),
			("delay", numeric(op.delay)),
			("overlap", numeric(op.overlap)),
		],
	)))
}

/// Convert upstream's ONNX `tar.gz` into the embedded NNEF `tar.gz`, byte-for-byte reproducibly.
fn nnef_archive_from_onnx(onnx_targz: impl Read) -> Result<Vec<u8>> {
	let mut graphs = [Vec::new(), Vec::new(), Vec::new()];
	let mut config = Vec::new();
	for entry in Archive::new(GzDecoder::new(onnx_targz)).entries()? {
		let mut file = entry?;
		let path = file.path()?.into_owned();
		let target = if path.ends_with("enc.onnx") {
			&mut graphs[0]
		} else if path.ends_with("erb_dec.onnx") {
			&mut graphs[1]
		} else if path.ends_with("df_dec.onnx") {
			&mut graphs[2]
		} else if path.ends_with("config.ini") {
			&mut config
		} else {
			continue;
		};
		file.read_to_end(target)?;
	}
	let ini = Ini::read_from(&mut Cursor::new(&config)).context("Reading model config")?;
	let net = ini
		.section(Some("deepfilternet"))
		.context("Missing [deepfilternet]")?;
	let df = ini.section(Some("df")).context("Missing [df]")?;
	let [enc, erb_dec, df_dec] = &graphs;
	let models = [
		("enc", encoder(enc, df)?),
		("erb_dec", erb_decoder(erb_dec, net, df)?),
		("df_dec", df_decoder(df_dec, net, df)?),
	];

	let mut delay = Registry::new("tract_pulse");
	delay.register_dumper(ser_delay);
	let mut nnef = tract_nnef::nnef()
		.with_tract_core()
		.with_pulse()
		.with_onnx();
	nnef.registries.insert(0, delay);

	let gz = GzBuilder::new()
		.mtime(0)
		.write(Vec::new(), Compression::best());
	let mut archive = Builder::new(gz);
	let mut append = |name: String, bytes: &[u8]| -> Result<()> {
		let mut header = Header::new_gnu();
		header.set_path(name)?;
		header.set_size(bytes.len() as u64);
		header.set_mode(0o644);
		header.set_mtime(MTIME);
		header.set_cksum();
		Ok(archive.append(&header, bytes)?)
	};
	for (name, model) in &models {
		let graph = nnef.write_to_tar_with_config(model, Vec::new(), false, true)?;
		append(format!("{name}.nnef.tar"), &graph)?;
	}
	append("config.ini".into(), &config)?;
	let gz: GzEncoder<Vec<u8>> = archive.into_inner()?;
	Ok(gz.finish()?)
}

fn onnx(graph: &[u8]) -> Result<(InferenceModel, Symbol)> {
	let model = tract_onnx::onnx()
		.with_ignore_output_shapes(true)
		.model_for_read(&mut Cursor::new(graph))?;
	let s = model.symbols.sym("S");
	Ok((model, s))
}

/// Upstream's load-time setup, minus `into_optimized`, which stays at runtime because
/// optimized graphs contain CPU-specific kernels.
fn pulse(model: InferenceModel, s: &Symbol) -> Result<TypedModel> {
	let mut model = model.into_typed()?;
	model.declutter()?;
	PulsedModel::new(&model, s.clone(), &1.to_dim())?.into_typed()
}

fn encoder(graph: &[u8], df: &ini::Properties) -> Result<TypedModel> {
	let (model, s) = onnx(graph)?;
	let nb_erb = df.get("nb_erb").context("nb_erb")?.parse::<usize>()?;
	let nb_df = df.get("nb_df").context("nb_df")?.parse::<usize>()?;
	let mut model = model
		.with_input_fact(
			0,
			InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, 1, s, nb_erb)),
		)?
		.with_input_fact(
			1,
			InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, 2, s, nb_df)),
		)?
		.with_input_names(["feat_erb", "feat_spec"])?
		.with_output_names(["e0", "e1", "e2", "e3", "emb", "c0", "lsnr"])?;
	model.analyse(true)?;
	pulse(model, &s)
}

fn erb_decoder(graph: &[u8], net: &ini::Properties, df: &ini::Properties) -> Result<TypedModel> {
	let (model, s) = onnx(graph)?;
	let nb_erb = df.get("nb_erb").context("nb_erb")?.parse::<usize>()?;
	let width = net.get("conv_ch").context("conv_ch")?.parse::<usize>()?;
	let (hidden, e3, e1) = (width * nb_erb / 4, nb_erb / 4, nb_erb / 2);
	let fact = |shape| InferenceFact::dt_shape(f32::datum_type(), shape);
	let mut model = model
		.with_input_fact(0, fact(shapefactoid!(1, s, hidden)))?
		.with_input_fact(1, fact(shapefactoid!(1, width, s, e3)))?
		.with_input_fact(2, fact(shapefactoid!(1, width, s, e3)))?
		.with_input_fact(3, fact(shapefactoid!(1, width, s, e1)))?
		.with_input_fact(4, fact(shapefactoid!(1, width, s, nb_erb)))?
		.with_input_names(["emb", "e3", "e2", "e1", "e0"])?;
	model.analyse(true)?;
	// Mono only: upstream's multichannel mask reduction is an identity here, so none is wired.
	pulse(model, &s)?.with_output_names(["m"])
}

fn df_decoder(graph: &[u8], net: &ini::Properties, df: &ini::Properties) -> Result<TypedModel> {
	let (model, s) = onnx(graph)?;
	let nb_erb = df.get("nb_erb").context("nb_erb")?.parse::<usize>()?;
	let nb_df = df.get("nb_df").context("nb_df")?.parse::<usize>()?;
	let width = net.get("conv_ch").context("conv_ch")?.parse::<usize>()?;
	let hidden = width * nb_erb / 4;
	let mut model = model
		.with_input_fact(
			0,
			InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, s, hidden)),
		)?
		.with_input_fact(
			1,
			InferenceFact::dt_shape(f32::datum_type(), shapefactoid!(1, width, s, nb_df)),
		)?
		.with_input_names(["emb", "c0"])?
		.with_output_names(["coefs"])?;
	model.analyse(true)?;
	pulse(model, &s)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn embedded_nnef_matches_upstream_onnx() {
		let converted =
			nnef_archive_from_onnx(std::fs::File::open(UPSTREAM_ONNX).unwrap()).unwrap();
		assert!(
			converted == std::fs::read(EMBEDDED_NNEF).unwrap(),
			"embedded NNEF is stale; regenerate it with `cargo run -p deep-filter-model`"
		);
	}
}
