fn main() {
    // Need this for CoreML. See: https://ort.pyke.io/perf/execution-providers#coreml
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" && std::env::var_os("DOCS_RS").is_none() {
        pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version("1.23.3")
            .probe("libheif")
            .unwrap_or_else(|error| {
                panic!(
                    "libheif 1.23.3 or newer is required; install it with Homebrew or scripts/install-libheif.sh: {error}"
                )
            });
    }
}
