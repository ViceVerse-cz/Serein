//! Prints what the folder opener and the process probe report in this environment.

fn main() {
	let path = std::env::args()
		.nth(1)
		.unwrap_or_else(|| std::env::temp_dir().display().to_string());
	let target = std::path::Path::new(&path);
	println!("open_folder({path}) -> {:?}", platform::open_folder(target));
	println!("spawn_probe() -> {:?}", platform::spawn_probe());
}
