//! Compiles the audio device's C (`src/audio/backend/device.c`, over the vendored miniaudio) when
//! the `audio-device` feature is on. Without it, jess has no C of its own to build.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "audio-device")]
    device();
}

#[cfg(feature = "audio-device")]
fn device() {
    println!("cargo::rerun-if-changed=src/audio/backend/device.c");
    println!("cargo::rerun-if-changed=vendor/miniaudio/miniaudio.h");
    cc::Build::new()
        .file("src/audio/backend/device.c")
        .include("vendor/miniaudio")
        // miniaudio is vendored as published: its own warnings are upstream's to fix.
        .warnings(false)
        .compile("jess_audio_device");
    // miniaudio loads the platform's audio libraries at run time; what it needs linked is the
    // threading, maths, and loader underneath.
    let target = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let libraries: &[&str] = match target.as_str() {
        "linux" => &["dl", "pthread", "m"],
        // Bionic has threads in its C library.
        "android" => &["dl", "m"],
        "macos" | "ios" | "freebsd" | "netbsd" | "openbsd" | "dragonfly" => &["pthread", "m"],
        _ => &[],
    };
    for library in libraries {
        println!("cargo::rustc-link-lib={library}");
    }
}
