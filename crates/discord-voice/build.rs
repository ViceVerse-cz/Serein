fn main() {
	println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
	println!("cargo:rerun-if-changed=src/video_encode_ffmpeg.c");
	println!("cargo:rerun-if-changed=src/video_encode_ffmpeg.h");
	println!("cargo:rerun-if-changed=src/video_gpu.c");
	println!("cargo:rerun-if-changed=src/video_gpu.h");
	for file in [
		"video_query.c",
		"video_query_nvenc.c",
		"video_query_qsv.c",
		"video_query_amf.cpp",
		"video_query_videotoolbox.c",
	] {
		println!("cargo:rerun-if-changed=src/{file}");
	}
	let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
	if target == "linux" {
		// Test executables also link static OpenH264 from hashed Rust rlibs.
		// They must not export it into host GStreamer's different native ABI.
		println!("cargo:rustc-link-arg=-Wl,--exclude-libs,ALL");
		println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
		println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib/serein");
	} else if target == "macos" {
		println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
	}
	let prefix = std::env::var_os("FFMPEG_DIR").map(std::path::PathBuf::from);
	let mut native = cc::Build::new();
	native
		.file("src/video_encode_ffmpeg.c")
		.file("src/video_gpu.c")
		.file("src/video_query.c")
		.file("src/video_query_nvenc.c")
		.file("src/video_query_qsv.c")
		.file("src/video_query_videotoolbox.c")
		.static_crt(target == "windows")
		.std("c11");
	if let Some(prefix) = &prefix {
		assert!(
			prefix.join("include/libavcodec/avcodec.h").is_file(),
			"Build the LGPL FFmpeg libraries with python3 scripts/build-ffmpeg.py, then set FFMPEG_DIR to its prefix"
		);
		native.include(prefix.join("include"));
		query_dependencies(
			&target,
			&[prefix.join("include")],
			Some(prefix),
			&mut native,
		);
		native.compile("serein_avc");
		println!(
			"cargo:rustc-link-search=native={}",
			prefix.join("lib").display()
		);
		println!("cargo:rustc-link-lib=dylib=avcodec-serein");
		println!("cargo:rustc-link-lib=dylib=avutil-serein");
		if matches!(target.as_str(), "linux" | "macos") {
			println!(
				"cargo:rustc-link-arg=-Wl,-rpath,{}",
				prefix.join("lib").display()
			);
		}
	} else {
		let codec = pkg_config::Config::new()
			.atleast_version("61")
			.cargo_metadata(false)
			.probe("libavcodec-serein")
			.expect("FFmpeg 7+ development libraries required; see scripts/build-ffmpeg.py");
		for include in &codec.include_paths {
			native.include(include);
		}
		query_dependencies(&target, &codec.include_paths, None, &mut native);
		native.compile("serein_avc");
		pkg_config::Config::new()
			.atleast_version("61")
			.probe("libavcodec-serein")
			.unwrap();
		pkg_config::Config::new()
			.atleast_version("59")
			.probe("libavutil-serein")
			.unwrap();
	}
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
		// Dependency link-args do not propagate to this crate's test executables.
		println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
		println!("cargo:rustc-link-lib=framework=VideoToolbox");
		println!("cargo:rustc-link-lib=framework=CoreFoundation");
	}
}

fn query_dependencies(
	target: &str,
	includes: &[std::path::PathBuf],
	prefix: Option<&std::path::PathBuf>,
	native: &mut cc::Build,
) {
	if !matches!(target, "linux" | "windows") {
		return;
	}
	let has = |header: &str| {
		includes
			.iter()
			.any(|include| include.join(header).is_file())
	};
	if has("ffnvcodec/nvEncodeAPI.h") && has("ffnvcodec/dynlink_cuda.h") {
		native.define("SEREIN_HAVE_NVENC_QUERY", "1");
	}
	if has("AMF/core/Factory.h") {
		native.define("SEREIN_HAVE_AMF_QUERY", "1");
		let mut amf = cc::Build::new();
		amf.cpp(true)
			.file("src/video_query_amf.cpp")
			.std("c++11")
			.static_crt(target == "windows");
		for include in includes {
			amf.include(include);
		}
		if target == "linux" && has("vulkan/vulkan.h") {
			native.define("SEREIN_HAVE_VULKAN_GPU", "1");
			amf.define("SEREIN_HAVE_VULKAN_GPU", "1");
		}
		amf.compile("serein_amf_query");
	}
	if has("vpl/mfxdispatcher.h") {
		if let Some(prefix) = prefix {
			assert!(
				prefix
					.join("lib")
					.join(if target == "windows" {
						"vpl.lib"
					} else {
						"libvpl.a"
					})
					.is_file(),
				"The oneVPL headers require the matching static dispatcher library"
			);
			println!("cargo:rustc-link-lib=static=vpl");
			if target == "windows" {
				for library in ["advapi32", "ole32", "uuid"] {
					println!("cargo:rustc-link-lib={library}");
				}
			} else {
				println!("cargo:rustc-link-lib=stdc++");
			}
		} else {
			pkg_config::Config::new()
				.statik(true)
				.probe("vpl")
				.expect("Driver queries require the matching oneVPL dispatcher");
		}
		native.define("SEREIN_HAVE_QSV_QUERY", "1");
	}
	if target == "linux" {
		println!("cargo:rustc-link-lib=dl");
	}
}
