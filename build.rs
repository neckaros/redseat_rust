fn main() {
    // Need this for CoreML. See: https://ort.pyke.io/perf/execution-providers#coreml
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" && std::env::var_os("DOCS_RS").is_none() {
        pkg_config::Config::new()
            .cargo_metadata(false)
            .statik(true)
            .atleast_version("1.23.3")
            .probe("libheif")
            .unwrap_or_else(|error| {
                panic!(
                    "static libheif 1.23.3 or newer is required; install it with scripts/install-libheif.sh: {error}"
                )
            });
    }

    // The ONNX Runtime of the `gpu` feature comes with a library of its own. Loaded
    // on first use rather than at startup, the server starts without it and embeds
    // on the CPU: see `gpu_library_present` in src/tools/embedding/gemma.rs.
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if std::env::var_os("CARGO_FEATURE_GPU").is_some()
        && target_os == "windows"
        && target_env == "msvc"
    {
        println!("cargo:rustc-link-arg-bin=redseat-rust=/DELAYLOAD:webgpu_dawn.dll");
        println!("cargo:rustc-link-arg-bin=redseat-rust=delayimp.lib");
    }
}
