fn main() {
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
		println!("cargo:rustc-link-arg=-Wl,--exclude-libs,ALL");
	}

	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
		windows_icon();
	}
	println!("cargo:rerun-if-changed=../../packaging/macos/Info.plist");
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
		// ScreenCaptureKit's Swift bridge uses the system Swift runtime.
		println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
		// cargo run has no app bundle: embed the same microphone usage description.
		let plist = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
			.join("../../packaging/macos/Info.plist");
		println!(
			"cargo:rustc-link-arg-bin=serein=-Wl,-sectcreate,__TEXT,__info_plist,{}",
			plist.display()
		);
	}
}

fn windows_icon() {
	use std::{path::PathBuf, process::Command};
	println!("cargo:rerun-if-changed=../../packaging/windows/Serein.ico");
	println!("cargo:rerun-if-changed=../../packaging/windows/serein.rc");
	let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
		.join("../../packaging/windows");
	let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
	let (mut compiler, resource) = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
	{
		let host = std::env::var("HOST").unwrap();
		let sdk = find_msvc_tools::find_windows_sdk(host.split('-').next().unwrap())
			.expect("Windows SDK is required to embed the application icon");
		let rc = sdk
			.path()
			.map(|path| path.join("rc.exe"))
			.find(|path| path.is_file())
			.expect("Windows SDK rc.exe");
		let resource = out.join("serein.res");
		let mut compiler = Command::new(rc);
		compiler
			.arg("/nologo")
			.arg("/fo")
			.arg(&resource)
			.arg("serein.rc");
		(compiler, resource)
	} else {
		let resource = out.join("serein-icon.o");
		let mut compiler = Command::new("windres");
		compiler.args(["-i", "serein.rc", "-o"]).arg(&resource);
		(compiler, resource)
	};
	assert!(
		compiler
			.current_dir(root)
			.status()
			.expect("run icon resource compiler")
			.success(),
		"application icon resource compilation failed"
	);
	println!("cargo:rustc-link-arg-bin=serein={}", resource.display());
}
