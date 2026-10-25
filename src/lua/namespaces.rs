//! Inventory of the generic runtime userdata and their registered members.

use l3i::extension::MemberKind;

/// The getters and methods one handle type registers, besides metamethods.
#[derive(Debug, Default)]
pub struct Namespace {
    /// The type's name in the Luau definitions: `ShaderExpr`, `InputGraph`, ...
    pub name: &'static str,
    /// Names read through a getter or a plain field.
    pub fields: Vec<String>,
    /// Names of methods and functions.
    pub methods: Vec<String>,
    /// Metamethods and meta fields: `__index`, `__len`, ...
    pub metamethods: Vec<String>,
}

/// Every jess userdata type, by its `Userdata::NAME` and the name the definitions use.
const TYPES: [(&str, &str); 11] = [
    ("jess.shader.Type", "ShaderType"),
    ("jess.shader.Expr", "ShaderExpr"),
    ("jess.shader.Input", "ShaderInput"),
    ("jess.shader.Attributes", "ShaderAttributes"),
    ("jess.shader.Builtins", "ShaderBuiltins"),
    ("jess.shader.Place", "ShaderPlace"),
    ("jess.shader.Texture", "ShaderTexture"),
    ("jess.shader.Sampler", "ShaderSampler"),
    ("jess.shader.Program", "ShaderProgram"),
    ("jess.input.Graph", "InputGraph"),
    ("jess.input.Runtime", "InputRuntime"),
];

/// Every handle type scripts index, with the names it registers.
///
/// # Panics
///
/// Panics if the runtime plan doesn't finalize or a runtime can't be made from it.
#[must_use]
pub fn namespaces() -> Vec<Namespace> {
    userdata()
}

/// Every userdata type scripts get, with the names it registers: the handles [`namespaces`]
/// lists, and the values they hand out.
///
/// # Panics
///
/// Panics if the runtime plan doesn't finalize or a runtime can't be made from it.
#[must_use]
pub fn userdata() -> Vec<Namespace> {
    // The method tables are bound per runtime, so one has to exist for their names to be known.
    let runtime = crate::lua::runtime(crate::lua::Options::default()).expect("a jess runtime");
    let plan = runtime.plan().expect("a planned runtime");
    TYPES
        .iter()
        .map(|(key, name)| {
            let resolved = plan.userdata_by_key(key).unwrap_or_else(|| panic!("{key} is in the plan"));
            let surface = crate::lua::plan::surface_of(key);
            let mut methods: Vec<String> = resolved
                .members
                .iter()
                .filter(|member| member.kind == MemberKind::Method)
                .map(|member| member.name.clone())
                .collect();
            methods.sort();
            methods.dedup();
            let mut fields: Vec<String> = resolved
                .members
                .iter()
                .filter(|member| matches!(member.kind, MemberKind::Getter | MemberKind::Field))
                .map(|member| member.name.clone())
                .collect();
            fields.sort();
            fields.dedup();
            Namespace {
                name,
                fields,
                methods,
                metamethods: surface.metamethods.iter().cloned().collect(),
            }
        })
        .collect()
}
