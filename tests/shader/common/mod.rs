//! Shared checks and helpers: every lowered module goes through naga, every layout jess reports
//! is the one naga computes, and goldens compare or bless.

use std::path::PathBuf;

use jess::shader::layout::StructLayout;
use jess::shader::wgsl::WgslModule;
use jess::shader::{DataType, Frequency, Place, PlaceRoot, Program, Site, Step, Type, Uniform, Value};

/// Parses and validates `module`'s WGSL with naga, and checks every struct and uniform block
/// layout the lowering reported against the one naga computes.
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
    for group in &module.layout.groups {
        if let Some(block) = &group.uniform {
            let name = match group.frequency {
                Frequency::Pass => "PassUniforms",
                Frequency::Material => "MaterialUniforms",
                Frequency::Draw => "DrawUniforms",
            };
            assert_layout(&parsed, |candidate| candidate == name, name, block);
        }
    }
    for (index, layout) in module.layout.structs.iter().enumerate() {
        let prefix = format!("S{index}_");
        assert_layout(&parsed, |candidate| candidate.starts_with(&prefix), &prefix, layout);
    }
}

fn assert_layout(module: &naga::Module, named: impl Fn(&str) -> bool, name: &str, layout: &StructLayout) {
    let (_, ty) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref().is_some_and(&named))
        .unwrap_or_else(|| panic!("no struct {name}"));
    let naga::TypeInner::Struct { members, span } = &ty.inner else {
        panic!("{name} is not a struct");
    };
    assert_eq!(*span, layout.size, "{name} size");
    let offsets: Vec<u32> = members.iter().map(|member| member.offset).collect();
    let reported: Vec<u32> = layout
        .order
        .iter()
        .map(|position| layout.members[*position as usize].offset)
        .collect();
    assert_eq!(offsets, reported, "{name} offsets");
}

/// The place of `frequency`'s uniform member `name`, declared as `ty` if the program doesn't
/// have it yet.
pub fn uniform(program: &mut Program, frequency: Frequency, name: &str, ty: Type) -> Place {
    let existing = program.uniforms_of(frequency).position(|(_, uniform)| uniform.name == name);
    let position = existing.unwrap_or_else(|| {
        program.uniform(Uniform::new(frequency, name, DataType::Value(ty)));
        program.uniforms_of(frequency).count() - 1
    });
    member(frequency, position)
}

/// The value of `frequency`'s uniform member `name`, declared through `site` as `ty` if the
/// program doesn't have it yet.
pub fn read(site: &mut Site<'_>, frequency: Frequency, name: &str, ty: Type) -> Value {
    let existing = site
        .program()
        .uniforms_of(frequency)
        .position(|(_, uniform)| uniform.name == name);
    let position = existing.unwrap_or_else(|| {
        site.uniform(frequency, name, DataType::Value(ty)).unwrap();
        site.program().uniforms_of(frequency).count() - 1
    });
    site.load(member(frequency, position), &[]).unwrap()
}

/// The place of `frequency`'s member at `position`.
pub fn member(frequency: Frequency, position: usize) -> Place {
    let step = Step::Field(u32::try_from(position).unwrap());
    Place::new(PlaceRoot::Uniform(frequency), &[step]).unwrap()
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
