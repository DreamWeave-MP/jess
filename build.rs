//! Compiles the audio device's C (`src/audio/backend/device.c`, over the vendored miniaudio) when
//! the `audio-device` feature is on, and lists the guide's pages for `jess docs` to carry.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    guide();
    #[cfg(feature = "audio-device")]
    device();
}

/// Writes `guide.rs`: every Markdown page under `site/content/docs`, by its path without the
/// extension, with forward slashes, in order. A tree without the site (there is none to find)
/// carries no pages.
fn guide() {
    fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|kind| kind == "md") {
                found.push(path);
            }
        }
    }

    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("site/content/docs");
    println!("cargo::rerun-if-changed={}", root.display());
    let mut found = Vec::new();
    walk(&root, &mut found);
    found.sort();
    let pages: Vec<String> = found
        .iter()
        .filter_map(|path| {
            let name: Vec<String> = path
                .strip_prefix(&root)
                .ok()?
                .with_extension("")
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            Some(format!(
                "    ({:?}, include_str!({:?})),\n",
                name.join("/"),
                path.display().to_string()
            ))
        })
        .collect();
    let source = format!(
        "/// The guide's pages: each one's name, and its source.\nconst PAGES: &[(&str, &str)] = &[\n{}];\n",
        pages.concat()
    );
    let out = Path::new(&std::env::var("OUT_DIR").unwrap_or_default()).join("guide.rs");
    std::fs::write(out, source).expect("the build's output folder takes a file");
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
