//! Shared checks: every lowered module goes through naga, and goldens compare or bless.

#![allow(dead_code)]

use std::path::PathBuf;

use jess::shader::wgsl::{UniformBlock, WgslModule};

/// Parses and validates `module`'s WGSL with naga, and checks the uniform layouts the lowering
/// reported against the ones naga computes.
pub fn assert_valid_wgsl(module: &WgslModule) {
    let parsed = match naga::front::wgsl::parse_str(&module.source) {
        Ok(parsed) => parsed,
        Err(error) => panic!(
            "naga cannot parse the WGSL:\n{}\n{}",
            error.emit_to_string(&module.source),
            module.source
        ),
    };
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default());
    if let Err(error) = validator.validate(&parsed) {
        panic!("naga rejects the WGSL: {error:?}\n{}", module.source);
    }
    if let Some(semantics) = &module.semantics {
        assert_layout(&parsed, "Semantics", semantics);
    }
    if let Some(parameters) = &module.parameters {
        assert_layout(&parsed, "Parameters", parameters);
    }
}

fn assert_layout<Source>(module: &naga::Module, name: &str, block: &UniformBlock<Source>) {
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no struct {name}"));
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("{name} is not a struct");
    };
    assert_eq!(*span, block.size, "{name} size");
    let offsets: Vec<u32> = members.iter().map(|member| member.offset).collect();
    let reported: Vec<u32> = block.members.iter().map(|member| member.offset).collect();
    assert_eq!(offsets, reported, "{name} offsets");
}

/// Compares `source` with `tests/golden/{name}.wgsl`; with `JESS_BLESS` set, rewrites it instead.
pub fn assert_golden(name: &str, source: &str) {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "tests",
        "shader",
        "golden",
        &format!("{name}.wgsl"),
    ]
    .iter()
    .collect();
    if std::env::var_os("JESS_BLESS").is_some() {
        std::fs::write(&path, source).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(
        expected == source,
        "{name}.wgsl differs; run with JESS_BLESS=1 to accept:\n{source}"
    );
}
