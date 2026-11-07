use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_swift_translate();
    }
    tauri_build::build()
}

/// 翻译 uses Apple's Translation framework, which only has a Swift interface:
/// swift/Translate.swift is built into a static library with C entry points.
fn build_swift_translate() {
    const SOURCE: &str = "swift/Translate.swift";
    println!("cargo:rerun-if-changed={SOURCE}");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        Ok(other) => panic!("unsupported target arch {other}"),
        Err(_) => panic!("CARGO_CFG_TARGET_ARCH not set"),
    };
    // Same floor as the app (tauri.conf.json minimumSystemVersion); the code
    // itself checks for macOS 26 at run time.
    let target = format!("{arch}-apple-macos14.0");
    let library = out_dir.join("libcptranslate.a");

    let status = Command::new("xcrun")
        .args(["swiftc", "-emit-library", "-static", "-parse-as-library", "-O"])
        .args(["-swift-version", "5", "-target", &target, "-module-name", "CPTranslate"])
        .arg("-module-cache-path")
        .arg(out_dir.join("swift-module-cache"))
        .arg(SOURCE)
        .arg("-o")
        .arg(&library)
        .status()
        .expect("failed to run swiftc (Xcode command line tools needed)");
    assert!(status.success(), "swiftc failed for {SOURCE}");

    let sdk = String::from_utf8(
        Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output()
            .expect("xcrun --show-sdk-path")
            .stdout,
    )
    .unwrap();
    let sdk = sdk.trim();

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=cptranslate");
    // The Swift runtime ships with macOS, in /usr/lib/swift.
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    // Not on macOS 14, so linked weakly: the app still starts there, and the
    // run-time check keeps it from being touched.
    for framework in ["Translation", "_Translation_SwiftUI"] {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,{framework}");
    }
}
