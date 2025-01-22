use std::env;

fn main() {
	// Need this for CoreML. See: https://ort.pyke.io/perf/execution-providers#coreml
    println!("cargo:rerun-if-changed=build.rs");


	#[cfg(target_os = "macos")]
	println!("cargo:rustc-link-arg=-fapple-link-rtlib");
	#[cfg(target_os = "macos")]
    env::set_var("PKG_CONFIG_PATH", "/Users/arnaudjezequel/Downloads/libheif-master/build");
}