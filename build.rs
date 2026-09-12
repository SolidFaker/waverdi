//! Detect the Synopsys FSDB Reader SDK (FFR) and build the C++ bridge.
//!
//! When `VERDI_HOME` points at a Verdi installation containing
//! `share/FsdbReader`, the bridge is compiled and linked against `libnffr`;
//! `cfg(fsdb_sdk)` is set so `src/fsdb.rs` is included. On every other
//! machine the build stays pure Rust and `.fsdb` files report a hint.

use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(fsdb_sdk)");
    println!("cargo::rerun-if-changed=csrc/ffr_bridge.cpp");
    println!("cargo::rerun-if-env-changed=VERDI_HOME");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }

    let Some(verdi_home) = std::env::var_os("VERDI_HOME") else {
        return;
    };
    let verdi_home = PathBuf::from(verdi_home);
    let sdk = verdi_home.join("share/FsdbReader");
    let lib_dir = sdk.join("linux64");
    if !sdk.join("ffrAPI.h").is_file() || !lib_dir.join("libnffr.so").is_file() {
        println!(
            "cargo::warning=VERDI_HOME is set but {} does not contain the FSDB Reader SDK; \
             building without FSDB support",
            sdk.display()
        );
        return;
    }

    cc::Build::new()
        .cpp(true)
        .file("csrc/ffr_bridge.cpp")
        .include(&sdk)
        .flag_if_supported("-std=c++14")
        .flag_if_supported("-w")
        .compile("waverdi_ffr_bridge");

    println!("cargo::rustc-link-search=native={}", lib_dir.display());
    println!("cargo::rustc-link-lib=dylib=nffr");
    // libnffr depends on symbols from libnsys; the bridge references one so
    // the linker keeps it despite --as-needed defaults.
    println!("cargo::rustc-link-lib=dylib=nsys");
    println!("cargo::rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    println!("cargo::rustc-cfg=fsdb_sdk");
}
