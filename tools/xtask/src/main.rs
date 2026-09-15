use std::{
	path::PathBuf,
	process::{Command, ExitCode, Stdio},
};
fn run(args: &[&str]) -> Result<(), String> {
	run_tool("cargo", args)
}
fn run_tool(program: &str, args: &[&str]) -> Result<(), String> {
	if Command::new(program)
		.args(args)
		.status()
		.map_err(|e| e.to_string())?
		.success()
	{
		Ok(())
	} else {
		Err(format!("{program} {} failed", args.join(" ")))
	}
}
fn rust_host() -> Result<String, String> {
	let compiler = Command::new("rustc")
		.args(["--version", "--verbose"])
		.output()
		.map_err(|e| e.to_string())?;
	let compiler = String::from_utf8_lossy(&compiler.stdout);
	compiler
		.lines()
		.find_map(|line| line.strip_prefix("host: "))
		.map(str::to_owned)
		.ok_or_else(|| "Rust host target unavailable".into())
}
fn policy() -> Result<(), String> {
	let host = rust_host()?;
	let output = Command::new("cargo")
		.args([
			"metadata",
			"--format-version=1",
			"--locked",
			"--offline",
			"--filter-platform",
			&host,
		])
		.output()
		.map_err(|e| e.to_string())?;
	if !output.status.success() {
		return Err("cargo metadata failed".into());
	}
	let metadata: serde_json::Value =
		serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
	for node in metadata["resolve"]["nodes"]
		.as_array()
		.ok_or("No dependency graph")?
	{
		let id = node["id"].as_str().unwrap_or("");
		let features = node["features"].as_array().ok_or("Missing features")?;
		for feature in features {
			let feature = feature.as_str().unwrap_or("");
			if ((id.contains("#eframe@") || id.contains("#egui@")) && feature == "persistence")
				|| (id.contains("#eframe@") && feature == "glow")
				|| (id.contains("#reqwest@") && feature == "cookies")
			{
				return Err(format!("Forbidden runtime feature: {id} / {feature}"));
			}
		}
	}
	let main = std::fs::read_to_string("apps/desktop/src/main.rs").map_err(|e| e.to_string())?;
	let compact = main.split_whitespace().collect::<String>();
	if !compact.contains("persist_window:false")
		|| !compact.contains("fnpersist_egui_memory(&self)->bool{false}")
	{
		return Err("Native persistence controls changed".into());
	}
	let platform =
		std::fs::read_to_string("crates/platform/src/lib.rs").map_err(|e| e.to_string())?;
	if !platform.contains(".with_incognito(true)") {
		return Err("Authentication webview must be ephemeral".into());
	}
	println!(
		"Policy checks passed: no eframe persistence, one renderer, no REST cookie jar, ephemeral login requested."
	);
	Ok(())
}
fn licenses() -> Result<(), String> {
	let version = Command::new("cargo-deny")
		.arg("--version")
		.output()
		.map_err(|_| "Install the checker: cargo install cargo-deny --version 0.20.2 --locked")?;
	if !version.status.success()
		|| String::from_utf8_lossy(&version.stdout).trim() != "cargo-deny 0.20.2"
	{
		return Err("License checks require cargo-deny 0.20.2; install the pinned version".into());
	}
	run(&["deny", "--locked", "--offline", "check", "licenses"])?;
	run(&[
		"deny",
		"--locked",
		"--offline",
		"--manifest-path",
		"fuzz/Cargo.toml",
		"--config",
		"deny.toml",
		"check",
		"licenses",
	])
}
fn fuzz() -> Result<(), String> {
	let version = Command::new("cargo-fuzz")
		.arg("--version")
		.output()
		.map_err(|_| "Install the checker: cargo install cargo-fuzz --version 0.13.2 --locked")?;
	if !version.status.success()
		|| String::from_utf8_lossy(&version.stdout).trim() != "cargo-fuzz 0.13.2"
	{
		return Err("Fuzz smoke requires cargo-fuzz 0.13.2".into());
	}
	let lock = std::fs::read("fuzz/Cargo.lock").map_err(|e| e.to_string())?;
	run(&[
		"fetch",
		"--locked",
		"--offline",
		"--manifest-path",
		"fuzz/Cargo.toml",
	])?;
	let root = PathBuf::from("target/fuzz-smoke");
	std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
	let root = root.canonicalize().map_err(|e| e.to_string())?;
	let nonce = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_err(|e| e.to_string())?
		.as_nanos();
	let corpus = root.join(format!("corpus-{}-{nonce}", std::process::id()));
	std::fs::create_dir(&corpus).map_err(|e| e.to_string())?;
	let result = (|| {
		for (target, max_len) in [("decode", "4194306"), ("state_transitions", "16384")] {
			let seeds = corpus.join(target);
			copy_directory(&PathBuf::from("fuzz/seeds").join(target), &seeds)?;
			let artifact = root.join(format!("{target}.crash"));
			let status = Command::new("cargo")
				.args(["+nightly-2026-09-09", "fuzz", "run", target])
				.arg(&seeds)
				.args([
					"--",
					"-max_total_time=30",
					"-runs=1000000",
					"-timeout=5",
					"-rss_limit_mb=512",
				])
				.arg(format!("-max_len={max_len}"))
				.arg(format!("-exact_artifact_path={}", artifact.display()))
				.env("CARGO_NET_OFFLINE", "true")
				.status()
				.map_err(|e| e.to_string())?;
			if !status.success() {
				return Err(format!("{target} fuzz smoke failed"));
			}
		}
		Ok(())
	})();
	// Delete only this invocation's generated corpus, never the committed seeds.
	let resolved = corpus.canonicalize().map_err(|e| e.to_string())?;
	if resolved.parent() != Some(root.as_path()) {
		return Err("Unexpected fuzz corpus location".into());
	}
	std::fs::remove_dir_all(resolved).map_err(|e| e.to_string())?;
	if std::fs::read("fuzz/Cargo.lock").map_err(|e| e.to_string())? != lock {
		return Err("Fuzzing changed fuzz/Cargo.lock; review the dependency resolution".into());
	}
	result
}
fn copy_directory(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
	std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
	for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
		let entry = entry.map_err(|e| e.to_string())?;
		// PR screenshots are development evidence, not installed application assets.
		if entry.file_name() == "pr-evidence" {
			continue;
		}
		let target = destination.join(entry.file_name());
		if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
			copy_directory(&entry.path(), &target)?;
		} else {
			std::fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
		}
	}
	Ok(())
}
#[allow(dead_code)]
fn package_windows(root: &std::path::Path) -> Result<(), String> {
	let nsis_candidates = [
		PathBuf::from("makensis"),
		PathBuf::from("makensis.exe"),
		PathBuf::from(r"C:\Program Files (x86)\NSIS\makensis.exe"),
		PathBuf::from(r"C:\Program Files\NSIS\makensis.exe"),
	];

	let makensis = nsis_candidates.iter().find(|cmd| {
		Command::new(cmd)
			.arg("/VERSION")
			.stdout(Stdio::null())
			.stderr(Stdio::null())
			.status()
			.is_ok_and(|s| s.success())
	});

	if let Some(makensis) = makensis {
		let installer_dir = PathBuf::from("dist-installer");
		std::fs::create_dir_all(&installer_dir).map_err(|e| e.to_string())?;
		let version = env!("CARGO_PKG_VERSION");
		let status = Command::new(makensis)
			.args([
				"-NOCD",
				&format!("-DVERSION={version}"),
				&format!("-DDIST_DIR={}", root.display()),
				&format!("-DOUTPUT_DIR={}", installer_dir.display()),
				"packaging/windows/installer.nsi",
			])
			.status()
			.map_err(|e| e.to_string())?;
		if !status.success() {
			return Err("NSIS installer compilation failed".into());
		}
		println!(
			"Windows installer created: {}",
			installer_dir
				.join(format!("serein-{version}-setup.exe"))
				.display()
		);
	} else {
		println!(
			"makensis not found; skipping Windows installer binary creation (installer script available at packaging/windows/installer.nsi)"
		);
	}
	Ok(())
}
fn package() -> Result<(), String> {
	let options: Vec<String> = std::env::args().skip(2).collect();
	let format = match options.as_slice() {
		[] => "deb",
		[flag, value]
			if flag == "--format"
				&& matches!(value.as_str(), "deb" | "rpm" | "arch" | "dir" | "appimage")
				&& cfg!(target_os = "linux") =>
		{
			value.as_str()
		}
		_ => {
			return Err(
				"Use cargo xtask package [--format deb|rpm|arch|dir|appimage (Linux only)]".into(),
			);
		}
	};
	let arguments = [
		"build",
		"--release",
		"--locked",
		"-p",
		"serein",
		"--no-default-features",
	];
	run(&arguments)?;
	let root = PathBuf::from("dist");
	std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
	let exe = if cfg!(windows) {
		"serein.exe"
	} else {
		"serein"
	};
	let destination = if cfg!(target_os = "macos") {
		let app = root.join("Serein.app/Contents");
		std::fs::create_dir_all(app.join("MacOS")).map_err(|e| e.to_string())?;
		std::fs::copy("packaging/macos/Info.plist", app.join("Info.plist"))
			.map_err(|e| e.to_string())?;
		app.join("MacOS").join(exe)
	} else {
		root.join(exe)
	};
	if cfg!(windows) {
		std::fs::copy(
			"packaging/windows/install-notifications.ps1",
			root.join("install-notifications.ps1"),
		)
		.map_err(|e| e.to_string())?;
	}
	let source = std::env::var_os("CARGO_TARGET_DIR")
		.map_or_else(|| PathBuf::from("target"), PathBuf::from)
		.join("release")
		.join(exe);
	if cfg!(target_os = "macos") {
		// macOS caches code signatures by inode. Replace the executable rather
		// than overwrite a previously launched, signed file in place.
		let staging = destination.with_extension("staging");
		match std::fs::remove_file(&staging) {
			Ok(()) => {}
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
			Err(error) => return Err(error.to_string()),
		}
		std::fs::copy(&source, &staging).map_err(|e| e.to_string())?;
		std::fs::rename(&staging, &destination).map_err(|e| e.to_string())?;
	} else {
		std::fs::copy(&source, &destination).map_err(|e| e.to_string())?;
	}
	for file in [
		"README.md",
		"LICENSE-MIT",
		"LICENSE-APACHE",
		"THIRD_PARTY_NOTICES.md",
	] {
		std::fs::copy(file, root.join(file)).map_err(|e| e.to_string())?;
	}
	let resources = if cfg!(target_os = "macos") {
		root.join("Serein.app/Contents/Resources")
	} else {
		root.clone()
	};
	if cfg!(target_os = "macos") {
		let status = std::process::Command::new("sh")
			.arg("packaging/macos/compile-icon.sh")
			.arg(&resources)
			.status()
			.map_err(|e| e.to_string())?;
		if !status.success() {
			return Err("macOS app icon compilation failed".into());
		}
	}
	std::fs::create_dir_all(resources.join("licenses")).map_err(|e| e.to_string())?;
	for file in [
		"NotoSansCJK-LICENSE.txt",
		"NotoSansArabic-OFL.txt",
		"Inter-OFL.txt",
	] {
		std::fs::copy(
			PathBuf::from("assets/fonts").join(file),
			resources.join("licenses").join(file),
		)
		.map_err(|e| e.to_string())?;
	}
	std::fs::copy(
		"assets/twemoji/LICENSE-GRAPHICS",
		resources.join("licenses/Twemoji-CC-BY-4.0.txt"),
	)
	.map_err(|e| e.to_string())?;
	std::fs::copy(
		"assets/twemoji/LICENSE-UNICODE",
		resources.join("licenses/Unicode-LICENSE.txt"),
	)
	.map_err(|e| e.to_string())?;
	std::fs::copy(
		"assets/icons/LICENSE",
		resources.join("licenses/Phosphor-Icons-MIT.txt"),
	)
	.map_err(|e| e.to_string())?;
	std::fs::copy(
		"assets/icons/LICENSE-SIMPLE-ICONS",
		resources.join("licenses/Simple-Icons-CC0.txt"),
	)
	.map_err(|e| e.to_string())?;
	copy_directory(
		std::path::Path::new("assets/licenses/files"),
		&resources.join("licenses/files"),
	)?;
	copy_directory(
		std::path::Path::new("assets/licenses/notifications"),
		&resources.join("licenses/notifications"),
	)?;
	copy_directory(
		std::path::Path::new("assets/licenses/login"),
		&resources.join("licenses/login"),
	)?;
	copy_directory(
		std::path::Path::new("assets/licenses/voice"),
		&resources.join("licenses/voice"),
	)?;
	copy_directory(
		std::path::Path::new("assets/licenses/audio"),
		&resources.join("licenses/audio"),
	)?;
	copy_directory(
		std::path::Path::new("assets/licenses/dependencies"),
		&resources.join("licenses/dependencies"),
	)?;
	for file in [
		"README.md",
		"LICENSE-MIT",
		"LICENSE-APACHE",
		"THIRD_PARTY_NOTICES.md",
	] {
		std::fs::copy(file, resources.join(file)).map_err(|e| e.to_string())?;
	}
	if cfg!(target_os = "macos") {
		// Seal only after every bundle resource has been staged. Ad-hoc signing
		// needs no identity and makes no Developer ID or notarization claim.
		let bundle = root.join("Serein.app");
		let bundle = bundle.to_str().ok_or("Invalid bundle path")?;
		run_tool("codesign", &["--force", "--sign", "-", bundle])?;
		run_tool("codesign", &["--verify", "--strict", bundle])?;
	}
	if cfg!(target_os = "linux") {
		let mut arguments = vec![
			if format == "appimage" {
				"packaging/appimage/build.py"
			} else {
				"packaging/linux/package.py"
			},
			root.to_str().ok_or("Invalid package path")?,
			env!("CARGO_PKG_VERSION"),
		];
		if format != "appimage" {
			arguments.extend(["--format", format]);
		}
		run_tool("python3", &arguments)?;
	}
	if cfg!(windows) {
		package_windows(&root)?;
	}
	println!(
		"{} package executable: {} ({} bytes)",
		if cfg!(target_os = "macos") {
			"Locally ad-hoc signed (not notarized)"
		} else {
			"Unsigned"
		},
		destination.display(),
		std::fs::metadata(&destination)
			.map_err(|e| e.to_string())?
			.len()
	);
	Ok(())
}
fn enter_workspace() -> Result<(), String> {
	// Shared target caches can reuse an executable built in a different worktree.
	// Resolve the caller's workspace at runtime instead of embedding a checkout path.
	let output = Command::new("cargo")
		.args(["locate-project", "--workspace", "--message-format", "plain"])
		.output()
		.map_err(|e| e.to_string())?;
	if !output.status.success() {
		return Err("Run xtask from inside the intended Cargo workspace".into());
	}
	let manifest = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
	let manifest = std::path::Path::new(manifest.trim());
	let root = manifest
		.parent()
		.ok_or("Workspace manifest has no parent")?;
	std::env::set_current_dir(root).map_err(|e| e.to_string())
}
fn main() -> ExitCode {
	let result = enter_workspace().and_then(|()| {
		match std::env::args().nth(1).as_deref().unwrap_or("check") {
			"check" => run(&["fmt", "--all", "--", "--check"])
				.and_then(|_| {
					run(&[
						"clippy",
						"--workspace",
						"--all-targets",
						"--locked",
						"--",
						"-D",
						"warnings",
					])
				})
				.and_then(|_| run(&["test", "--workspace", "--locked"]))
				.and_then(|_| run(&["check", "-p", "serein", "--no-default-features", "--locked"]))
				.and_then(|_| policy()),
			"policy" => policy(),
			"licenses" => licenses(),
			"fuzz" => fuzz(),
			"package" => package(),
			_ => Err("Use cargo xtask [check|policy|licenses|fuzz|package]".into()),
		}
	});
	match result {
		Ok(()) => ExitCode::SUCCESS,
		Err(error) => {
			eprintln!("{error}");
			ExitCode::FAILURE
		}
	}
}
