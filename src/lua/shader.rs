//! `@jess/shader` (`jess.shader`): typed shader construction, a thin Luau handle on
//! [`jess::shader`](crate::shader)'s [`ProgramBuilder`].
//!
//! A shader module is an ordinary Luau module that runs once and returns a [`LuaShaderProgram`]
//! from `shader.program { parameters, textures, vertex, fragment }`. Its vertex and fragment
//! functions don't run on the GPU or per frame: each is called once, during `shader.program`,
//! with an input handle whose names are the declared parameters and textures, the semantic
//! inputs (`model`, `view`, `projection`, ...), and, per stage, the vertex attributes or the
//! varyings the vertex function returned. Every read and every operation appends one typed
//! expression to the program and hands back a symbolic value ([`LuaShaderExpr`]); operators and
//! swizzles on those values do the same. A bare Luau number in a shader position is a `Float`.
//!
//! Every expression records where the Luau source wrote it (`file:line`, plus the module that
//! called `shader.program`), so a mistake fails at the call that made it, in Jess terms:
//! `shaders/lit.jsl:12 (shaders/lit.jsl): dot(Vec3f, Vec4f) is invalid: expected two float
//! vectors of the same size`. Nothing here takes WGSL or exposes a GPU API.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use crate::shader::{Program, ProgramBuilder, Semantic, Site, SourceOrigin, Swizzle, Type, ValidationError, Value};
use l3i::Runtime;
use l3i::bind::{ArgView, Call, StackResults};
use l3i::debug::DebugScope;
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Frame, Scope, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table};

use crate::lua::plan::Functions;
use crate::lua::{Jess, error, kind, lua, push_table};

/// What `shader.program` takes.
const PROGRAM_KEYS: &[&str] = &["fragment", "parameters", "textures", "vertex"];

/// The per-vertex attributes a vertex function reads from its input, by name.
const VERTEX_ATTRIBUTES: [(&str, Type); 4] = [
    ("position", Type::Vec3f),
    ("normal", Type::Vec3f),
    ("uv", Type::Vec2f),
    ("color", Type::Vec4f),
];

/// The shader module's state in one runtime: the programs under construction, innermost last,
/// so a call with no shader value among its arguments (`shader.vec4(1, 1, 1, 1)`) knows which
/// program it belongs to.
#[derive(Debug)]
struct Shader {
    current: RefCell<Vec<Rc<RefCell<Build>>>>,
}

impl Shader {
    /// The program a call builds into: the one under construction, else the program of the
    /// first shader value among `operands`, which has finished and says so when used.
    fn build_for(&self, operation: &str, operands: &[ValueView<'_>]) -> l3i::Result<Rc<RefCell<Build>>> {
        if let Some(current) = self.current.borrow().last() {
            return Ok(current.clone());
        }
        operands
            .iter()
            .find_map(|operand| l3i::userdata::receiver::<LuaShaderExpr>(*operand))
            .map(|expression| expression.build.clone())
            .ok_or_else(|| {
                error(format!(
                    "shader.{operation} is only usable inside shader.program's vertex or fragment function"
                ))
            })
    }
}

/// The chunk and current line of the Luau function that called the running binding. A binding
/// called through `pcall`, or as a metamethod another native function triggered, has that
/// function as its caller, so this walks up to the nearest Luau frame.
fn call_site(call: &Call<'_>) -> (String, u32) {
    let mut level = 1;
    while let Some(site) = call.call_site(level) {
        if site.source != "[C]" {
            return (site.source, u32::try_from(site.line).unwrap_or(0));
        }
        level += 1;
    }
    ("?".to_owned(), 0)
}

/// Which stage function is running, and so which names its input has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Vertex,
    Fragment,
}

impl Stage {
    const fn name(self) -> &'static str {
        match self {
            Self::Vertex => "vertex",
            Self::Fragment => "fragment",
        }
    }
}

/// One program under construction: the builder, gone once the program is finished, and what the
/// input handles resolve names to.
#[derive(Debug)]
struct Build {
    builder: Option<ProgramBuilder>,
    info: Info,
}

/// The names a program declares, each read once and handed out as the same value after that.
#[derive(Debug)]
struct Info {
    /// The module that called `shader.program`, on every origin.
    owner: Arc<str>,
    /// The last chunk an origin named, so a run of calls from one file shares the allocation.
    last_file: Option<Arc<str>>,
    parameters: BTreeMap<String, Value>,
    textures: BTreeMap<String, Value>,
    /// The sampler declared with each texture, by the texture's value.
    samplers: HashMap<Value, Value>,
    vertex_inputs: HashMap<&'static str, Value>,
    semantics: HashMap<Semantic, Value>,
    /// What the vertex function returned, as the fragment stage reads it.
    varyings: BTreeMap<String, Value>,
}

impl Info {
    fn new(owner: Arc<str>) -> Self {
        Self {
            owner,
            last_file: None,
            parameters: BTreeMap::new(),
            textures: BTreeMap::new(),
            samplers: HashMap::new(),
            vertex_inputs: HashMap::new(),
            semantics: HashMap::new(),
            varyings: BTreeMap::new(),
        }
    }

    /// Where the Luau function that called the running binding is: its chunk and current line.
    /// Luau records lines, not columns, so the column is 0 and prints as nothing.
    fn origin(&mut self, call: &Call<'_>) -> SourceOrigin {
        let (file, line) = call_site(call);
        let file = match &self.last_file {
            Some(last) if **last == *file => last.clone(),
            _ => {
                let file: Arc<str> = Arc::from(file);
                self.last_file = Some(file.clone());
                file
            }
        };
        SourceOrigin {
            owner: self.owner.clone(),
            file,
            line,
            column: 0,
        }
    }

    /// Why `name` can't be declared as `what`: it's the name of an input every program has (a
    /// semantic, or a vertex attribute when `stage` is the vertex stage; a varying may share an
    /// attribute's name, since the fragment stage never sees attributes), or of something this
    /// program declared already.
    fn name_taken(&self, name: &str, what: &str, stage: Stage) -> l3i::Result<()> {
        let taken = if Semantic::ALL.iter().any(|semantic| semantic.name() == name) {
            "semantic input"
        } else if stage == Stage::Vertex && VERTEX_ATTRIBUTES.iter().any(|(attribute, _)| *attribute == name) {
            "vertex attribute"
        } else if self.parameters.contains_key(name) {
            "parameter"
        } else if self.textures.contains_key(name) {
            "texture"
        } else {
            return Ok(());
        };
        Err(error(format!(
            "{what} \"{name}\" has the name of a {taken}, which the input would hide"
        )))
    }

    /// Every name an input of `stage` answers to, sorted, for the error naming one it doesn't.
    fn names(&self, stage: Stage) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .parameters
            .keys()
            .chain(self.textures.keys())
            .map(String::as_str)
            .collect();
        match stage {
            Stage::Vertex => names.extend(VERTEX_ATTRIBUTES.iter().map(|(name, _)| *name)),
            Stage::Fragment => names.extend(self.varyings.keys().map(String::as_str)),
        }
        names.extend(Semantic::ALL.iter().map(|semantic| semantic.name()));
        names.sort_unstable();
        names
    }
}

/// A shader type: a declaration in `shader.program`'s `parameters` and `textures`, and, for the
/// vector types and `float`, a constructor.
#[derive(Clone, Debug)]
pub struct LuaShaderType {
    ty: Type,
    shader: Rc<Shader>,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderType {
    const NAME: &'static str = "jess.shader.Type";
}

/// A symbolic, typed value in one shader program under construction. Operators (`+ - * /`) and
/// swizzles (`.xyz`) append expressions to that program; nothing is computed.
#[derive(Clone, Debug)]
pub struct LuaShaderExpr {
    build: Rc<RefCell<Build>>,
    value: Value,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderExpr {
    const NAME: &'static str = "jess.shader.Expr";
}

/// A stage function's input: every name is a declared parameter or texture, a semantic input,
/// or (per stage) a vertex attribute or a varying.
#[derive(Clone, Debug)]
pub struct LuaShaderInput {
    build: Rc<RefCell<Build>>,
    stage: Stage,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderInput {
    const NAME: &'static str = "jess.shader.Input";
}

/// A finished, validated shader program: what a shader module returns.
#[derive(Clone, Debug)]
pub struct LuaShaderProgram(Rc<Finished>);

// SAFETY: an `Rc` to plain Rust data; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderProgram {
    const NAME: &'static str = "jess.shader.Program";
}

#[derive(Debug)]
struct Finished {
    program: Program,
    owner: Arc<str>,
    /// The texture names, in declaration order.
    textures: Vec<String>,
}

impl LuaShaderProgram {
    /// The validated program, ready to fold and lower. `shader.program` validates but doesn't fold
    /// constants: folding is the consumer's decision.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.0.program
    }

    /// The module that built the program: the chunk that called `shader.program`.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.0.owner
    }
}

/// Runs `body` on the program's builder, positioned where the calling Luau function is.
fn at_site<R>(
    call: &Call<'_>,
    build: &Rc<RefCell<Build>>,
    body: impl FnOnce(&mut Site<'_>, &mut Info) -> l3i::Result<R>,
) -> l3i::Result<R> {
    let Build { builder, info } = &mut *build.borrow_mut();
    let builder = builder.as_mut().ok_or_else(|| {
        error("this shader program is finished: its values belong to a shader.program call that has returned")
    })?;
    let origin = info.origin(call);
    body(&mut builder.at(origin), info)
}

/// A shader value that raised `problem`.
fn invalid(problem: ValidationError) -> l3i::Error {
    error(problem)
}

/// An operand of `operation`: a shader value's expression (a value from another program is
/// refused by the site), or a number, which is a `Float` literal at the operation's origin.
fn operand(site: &mut Site<'_>, operation: &str, view: ValueView<'_>) -> l3i::Result<Value> {
    if let Some(expression) = l3i::userdata::receiver::<LuaShaderExpr>(view) {
        return Ok(expression.value);
    }
    if view.is_number() || view.is_integer() {
        #[allow(clippy::cast_possible_truncation)] // Shader floats are f32.
        let number = view.read::<f64>()? as f32;
        return site.float(number).map_err(invalid);
    }
    Err(error(format!(
        "{operation}: expected a shader value or a number, not {}",
        kind(view)
    )))
}

fn expression(build: &Rc<RefCell<Build>>, value: Value) -> Owned<LuaShaderExpr> {
    Owned(LuaShaderExpr {
        build: build.clone(),
        value,
    })
}

/// The two-operand operations: the arithmetic metamethods and the module's functions.
#[derive(Clone, Copy, Debug)]
enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Min,
    Max,
    Dot,
    Cross,
}

impl Binary {
    const fn name(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Subtract => "-",
            Self::Multiply => "*",
            Self::Divide => "/",
            Self::Min => "min",
            Self::Max => "max",
            Self::Dot => "dot",
            Self::Cross => "cross",
        }
    }

    fn apply(self, site: &mut Site<'_>, left: Value, right: Value) -> Result<Value, ValidationError> {
        match self {
            Self::Add => site.add(left, right),
            Self::Subtract => site.subtract(left, right),
            Self::Multiply => site.multiply(left, right),
            Self::Divide => site.divide(left, right),
            Self::Min => site.min(left, right),
            Self::Max => site.max(left, right),
            Self::Dot => site.dot(left, right),
            Self::Cross => site.cross(left, right),
        }
    }
}

/// The one-operand operations.
#[derive(Clone, Copy, Debug)]
enum Unary {
    Normalize,
    Length,
    Floor,
}

impl Unary {
    const fn name(self) -> &'static str {
        match self {
            Self::Normalize => "normalize",
            Self::Length => "length",
            Self::Floor => "floor",
        }
    }

    fn apply(self, site: &mut Site<'_>, value: Value) -> Result<Value, ValidationError> {
        match self {
            Self::Normalize => site.normalize(value),
            Self::Length => site.length(value),
            Self::Floor => site.floor(value),
        }
    }
}

/// The three-operand operations.
#[derive(Clone, Copy, Debug)]
enum Ternary {
    Clamp,
    Mix,
}

impl Ternary {
    const fn name(self) -> &'static str {
        match self {
            Self::Clamp => "clamp",
            Self::Mix => "mix",
        }
    }

    fn apply(self, site: &mut Site<'_>, first: Value, second: Value, third: Value) -> Result<Value, ValidationError> {
        match self {
            Self::Clamp => site.clamp(first, second, third),
            Self::Mix => site.mix(first, second, third),
        }
    }
}

fn binary(
    call: &Call<'_>,
    build: &Rc<RefCell<Build>>,
    operation: Binary,
    left: ValueView<'_>,
    right: ValueView<'_>,
) -> l3i::Result<Owned<LuaShaderExpr>> {
    let value = at_site(call, build, |site, _| {
        let left = operand(site, operation.name(), left)?;
        let right = operand(site, operation.name(), right)?;
        operation.apply(site, left, right).map_err(invalid)
    })?;
    Ok(expression(build, value))
}

fn ternary(
    call: &Call<'_>,
    build: &Rc<RefCell<Build>>,
    operation: Ternary,
    first: ValueView<'_>,
    second: ValueView<'_>,
    third: ValueView<'_>,
) -> l3i::Result<Owned<LuaShaderExpr>> {
    let value = at_site(call, build, |site, _| {
        let first = operand(site, operation.name(), first)?;
        let second = operand(site, operation.name(), second)?;
        let third = operand(site, operation.name(), third)?;
        operation.apply(site, first, second, third).map_err(invalid)
    })?;
    Ok(expression(build, value))
}

fn unary(
    call: &Call<'_>,
    build: &Rc<RefCell<Build>>,
    operation: Unary,
    value: ValueView<'_>,
) -> l3i::Result<Owned<LuaShaderExpr>> {
    let value = at_site(call, build, |site, _| {
        let value = operand(site, operation.name(), value)?;
        operation.apply(site, value).map_err(invalid)
    })?;
    Ok(expression(build, value))
}

/// An arithmetic metamethod: the program is the first shader value's, whichever side it's on.
fn arithmetic(
    call: &Call<'_>,
    operation: Binary,
    left: ValueView<'_>,
    right: ValueView<'_>,
) -> l3i::Result<Owned<LuaShaderExpr>> {
    let build = [left, right]
        .into_iter()
        .find_map(|side| l3i::userdata::receiver::<LuaShaderExpr>(side))
        .map(|expression| expression.build.clone())
        .ok_or_else(|| error(format!("{}: expected a shader value", operation.name())))?;
    binary(call, &build, operation, left, right)
}

/// `shader.sample(texture, uv)` with the sampler declared with the texture, or
/// `shader.sample(texture, sampler, uv)`.
fn sample(
    call: &Call<'_>,
    build: &Rc<RefCell<Build>>,
    texture: ValueView<'_>,
    second: ValueView<'_>,
    third: Option<ValueView<'_>>,
) -> l3i::Result<Owned<LuaShaderExpr>> {
    let value = at_site(call, build, |site, info| {
        let texture = operand(site, "sample", texture)?;
        let (sampler, coordinates) = if let Some(coordinates) = third {
            (operand(site, "sample", second)?, coordinates)
        } else {
            let sampler = *info.samplers.get(&texture).ok_or_else(|| {
                error(
                    "sample(texture, uv) takes a texture from this program's textures, whose sampler comes with it; \
                     anything else needs its sampler: sample(texture, sampler, uv)",
                )
            })?;
            (sampler, second)
        };
        let coordinates = operand(site, "sample", coordinates)?;
        site.sample(texture, sampler, coordinates).map_err(invalid)
    })?;
    Ok(expression(build, value))
}

/// The `jess.shader` member a type is, as `shader.<name>`.
const TYPE_DESCRIPTORS: [(&str, Type); 8] = [
    ("float", Type::Float),
    ("vec2", Type::Vec2f),
    ("vec3", Type::Vec3f),
    ("vec4", Type::Vec4f),
    ("mat3", Type::Mat3f),
    ("mat4", Type::Mat4f),
    ("texture2d", Type::Texture2D),
    ("sampler", Type::Sampler),
];

fn descriptor_name(ty: Type) -> &'static str {
    TYPE_DESCRIPTORS
        .iter()
        .find(|(_, descriptor)| *descriptor == ty)
        .map_or("type", |(name, _)| name)
}

/// `shader.vec4(...)` and the other type descriptors called: a vector from values and numbers,
/// or `shader.float(number)`, a `Float` literal. The other types say they can't be constructed.
fn construct(this: &LuaShaderType, call: &Call<'_>, args: ArgView<'_>) -> l3i::Result<Owned<LuaShaderExpr>> {
    let views: Vec<ValueView<'_>> = (0..args.len()).map(|index| args.get(index)).collect();
    let operation = this.ty.name();
    let build = this.shader.build_for(descriptor_name(this.ty), &views)?;
    let value = at_site(call, &build, |site, _| {
        if this.ty == Type::Float
            && let [only] = views.as_slice()
            && (only.is_number() || only.is_integer())
        {
            return operand(site, operation, *only);
        }
        let components = views
            .iter()
            .map(|view| operand(site, operation, *view))
            .collect::<l3i::Result<Vec<Value>>>()?;
        site.construct(this.ty, &components).map_err(invalid)
    })?;
    Ok(expression(&build, value))
}

/// `value.xyz`: a swizzle, the only names a shader value has.
fn swizzle(this: &LuaShaderExpr, call: &Call<'_>, key: ValueView<'_>) -> l3i::Result<Owned<LuaShaderExpr>> {
    if !key.is_string() {
        return Err(error(format!(
            "a shader value is indexed by a swizzle such as .xyz, not {}",
            kind(key)
        )));
    }
    let name = key.read::<&str>()?;
    let swizzle = Swizzle::parse(name).ok_or_else(|| {
        error(format!(
            "a shader value has no field '{name}': its components are read by a swizzle of one to four of x, y, z, \
             and w, such as .xyz"
        ))
    })?;
    let value = at_site(call, &this.build, |site, _| {
        site.swizzle(this.value, swizzle).map_err(invalid)
    })?;
    Ok(expression(&this.build, value))
}

/// `input.<name>` in a stage function.
fn input(this: &LuaShaderInput, call: &Call<'_>, key: ValueView<'_>) -> l3i::Result<Owned<LuaShaderExpr>> {
    if !key.is_string() {
        return Err(error(format!(
            "the {} input is indexed by name, not {}",
            this.stage.name(),
            kind(key)
        )));
    }
    let name = key.read::<&str>()?;
    let stage = this.stage;
    let value = at_site(call, &this.build, |site, info| {
        if let Some(value) = info.parameters.get(name).or_else(|| info.textures.get(name)) {
            return Ok(*value);
        }
        match stage {
            Stage::Vertex => {
                if let Some((attribute, ty)) = VERTEX_ATTRIBUTES.iter().find(|(attribute, _)| *attribute == name) {
                    if let Some(value) = info.vertex_inputs.get(attribute) {
                        return Ok(*value);
                    }
                    let value = site.vertex_input(attribute, *ty).map_err(invalid)?;
                    info.vertex_inputs.insert(attribute, value);
                    return Ok(value);
                }
            }
            Stage::Fragment => {
                if let Some(value) = info.varyings.get(name) {
                    return Ok(*value);
                }
                if VERTEX_ATTRIBUTES.iter().any(|(attribute, _)| *attribute == name) {
                    return Err(error(format!(
                        "vertex input \"{name}\" is only readable in the vertex function; return it from there to \
                         pass it to the fragment function as a varying"
                    )));
                }
            }
        }
        if let Some(semantic) = Semantic::ALL.iter().copied().find(|semantic| semantic.name() == name) {
            if let Some(value) = info.semantics.get(&semantic) {
                return Ok(*value);
            }
            let value = site.semantic(semantic);
            info.semantics.insert(semantic, value);
            return Ok(value);
        }
        Err(error(format!(
            "the {} input has no \"{name}\": it has {}",
            stage.name(),
            info.names(stage).join(", ")
        )))
    })?;
    Ok(expression(&this.build, value))
}

/// The `parameters` or `textures` table of a program spec: names to types, sorted by name.
fn declarations(frame: &Frame<'_>, table: ValueView<'_>, what: &str) -> l3i::Result<Vec<(String, Type)>> {
    if table.is_nil() {
        return Ok(Vec::new());
    }
    if !table.is_table() {
        return Err(error(format!("{what} must be a table of shader types, not {}", kind(table))));
    }
    let mut declared = Vec::new();
    table.as_table()?.for_each(frame, |_, key, value| {
        if !key.is_string() {
            return Err(error(format!("{what} are named by strings, not {}", kind(key))));
        }
        let name = key.read::<&str>()?;
        let Some(ty) = l3i::userdata::receiver::<LuaShaderType>(value) else {
            return Err(error(format!(
                "{what}.{name} must be a shader type such as shader.vec4, not {}",
                kind(value)
            )));
        };
        declared.push((name.to_owned(), ty.ty));
        Ok(())
    })?;
    declared.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(declared)
}

/// A program spec's `vertex` or `fragment` function.
fn stage_function(function: ValueView<'_>, what: &str) -> l3i::Result<Function> {
    if !function.is_function() {
        return Err(error(format!(
            "shader.program needs a {what} function, not {}",
            kind(function)
        )));
    }
    Function::from_value(l3i::value::Value::store(function)?)
}

/// What the vertex function returned: its `position`, and the values to pass to the fragment
/// stage, sorted by name so the varying interface doesn't depend on the table's order.
fn vertex_outputs(frame: &Frame<'_>, outputs: ValueView<'_>) -> l3i::Result<(Value, Vec<(String, Value)>)> {
    if !outputs.is_table() {
        return Err(error(format!(
            "the vertex function returned {}: expected a table with position and the values to pass to the \
             fragment function",
            kind(outputs)
        )));
    }
    let mut position = None;
    let mut varyings = Vec::new();
    outputs.as_table()?.for_each(frame, |_, key, value| {
        if !key.is_string() {
            return Err(error(format!("vertex outputs are named by strings, not {}", kind(key))));
        }
        let name = key.read::<&str>()?;
        let Some(expression) = l3i::userdata::receiver::<LuaShaderExpr>(value) else {
            return Err(error(format!(
                "vertex output \"{name}\" is {}: expected a shader value",
                kind(value)
            )));
        };
        if name == "position" {
            position = Some(expression.value);
        } else {
            varyings.push((name.to_owned(), expression.value));
        }
        Ok(())
    })?;
    let position = position.ok_or_else(|| error("the vertex function's table has no position"))?;
    varyings.sort_by(|left, right| left.0.cmp(&right.0));
    Ok((position, varyings))
}

/// Where `value` was written, for the declarations and outputs made from it, else `fallback`.
fn origin_of(builder: &ProgramBuilder, value: Value, fallback: &SourceOrigin) -> SourceOrigin {
    builder
        .program()
        .origin(value.expression())
        .cloned()
        .unwrap_or_else(|| fallback.clone())
}

/// Builds the program: declares, runs the vertex function and passes what it returned, runs the
/// fragment function and outputs what it returned, then validates.
fn construct_program(
    call: &Call<'_>,
    build: &Rc<RefCell<Build>>,
    parameters: Vec<(String, Type)>,
    textures: Vec<(String, Type)>,
    vertex: &Function,
    fragment: &Function,
) -> l3i::Result<Finished> {
    let declared_at = {
        let Build { builder, info } = &mut *build.borrow_mut();
        let origin = info.origin(call);
        let builder = builder.as_mut().expect("a program under construction has its builder");
        let mut site = builder.at(origin.clone());
        for (name, ty) in parameters {
            if ty == Type::Texture2D {
                return Err(error(format!(
                    "parameter \"{name}\" is a Texture2D: textures are declared in textures"
                )));
            }
            info.name_taken(&name, "parameter", Stage::Vertex)?;
            let value = site.parameter(&name, ty).map_err(invalid)?;
            info.parameters.insert(name, value);
        }
        for (name, ty) in textures {
            if ty != Type::Texture2D {
                return Err(error(format!(
                    "texture \"{name}\" is {ty}: textures are shader.texture2d; values go in parameters"
                )));
            }
            info.name_taken(&name, "texture", Stage::Vertex)?;
            let texture = site.parameter(&name, Type::Texture2D).map_err(invalid)?;
            let sampler = site.parameter(&name, Type::Sampler).map_err(invalid)?;
            info.samplers.insert(texture, sampler);
            info.textures.insert(name, texture);
        }
        origin
    };

    let input = LuaShaderInput {
        build: build.clone(),
        stage: Stage::Vertex,
    };
    let (position, outputs) = vertex
        .invoke_with(call, (Owned(input),), vertex_outputs)
        .map_err(crate::lua::callback_error)?;
    {
        let Build { builder, info } = &mut *build.borrow_mut();
        let builder = builder.as_mut().expect("a program under construction has its builder");
        let at = origin_of(builder, position, &declared_at);
        builder.at(at).set_position(position).map_err(invalid)?;
        for (name, value) in outputs {
            info.name_taken(&name, "vertex output", Stage::Fragment)?;
            let at = origin_of(builder, value, &declared_at);
            let read = builder.at(at).pass(&name, value).map_err(invalid)?;
            info.varyings.insert(name, read);
        }
    }

    let input = LuaShaderInput {
        build: build.clone(),
        stage: Stage::Fragment,
    };
    let output = fragment
        .invoke_with(call, (Owned(input),), |_, output| {
            l3i::userdata::receiver::<LuaShaderExpr>(output)
                .map(|expression| expression.value)
                .ok_or_else(|| {
                    error(format!(
                        "the fragment function returned {}: expected a shader value, the Vec4f color",
                        kind(output)
                    ))
                })
        })
        .map_err(crate::lua::callback_error)?;

    let (builder, owner, textures) = {
        let Build { builder, info } = &mut *build.borrow_mut();
        let taken = builder.take().expect("a program under construction has its builder");
        let mut taken = taken;
        let at = origin_of(&taken, output, &declared_at);
        taken.at(at).output(output).map_err(invalid)?;
        (taken, info.owner.clone(), info.textures.keys().cloned().collect())
    };
    let program = builder.finish().map_err(|problems| {
        let mut message = format!("{owner}: the shader program is invalid");
        for problem in problems {
            message.push('\n');
            message.push_str(&problem.to_string());
        }
        error(message)
    })?;
    Ok(Finished {
        program,
        owner,
        textures,
    })
}

/// `shader.program { parameters, textures, vertex, fragment }`.
fn program(shader: &Rc<Shader>, call: &Call<'_>, spec: ValueView<'_>) -> l3i::Result<Owned<LuaShaderProgram>> {
    if !spec.is_table() {
        return Err(error(format!("shader.program takes a table, not {}", kind(spec))));
    }
    let owner: Arc<str> = Arc::from(call_site(call).0);
    let (parameters, textures, vertex, fragment) = call.with_frame(|frame| {
        crate::lua::check_options(frame, spec, "jess.shader.program", PROGRAM_KEYS)?;
        let table = spec.as_table()?;
        let parameters = declarations(frame, table.raw_get(frame, "parameters")?, "parameters")?;
        let textures = declarations(frame, table.raw_get(frame, "textures")?, "textures")?;
        let vertex = stage_function(table.raw_get(frame, "vertex")?, "vertex")?;
        let fragment = stage_function(table.raw_get(frame, "fragment")?, "fragment")?;
        Ok((parameters, textures, vertex, fragment))
    })?;
    let build = Rc::new(RefCell::new(Build {
        builder: Some(ProgramBuilder::new()),
        info: Info::new(owner),
    }));
    shader.current.borrow_mut().push(build.clone());
    let finished = construct_program(call, &build, parameters, textures, &vertex, &fragment);
    shader.current.borrow_mut().pop();
    // Whatever happened, the values handed out during construction belong to no program now.
    build.borrow_mut().builder = None;
    Ok(Owned(LuaShaderProgram(Rc::new(finished?))))
}

/// Pushes `{ name, type }` for each declaration.
fn push_declarations(call: &Call<'_>, declarations: &[crate::shader::Declaration]) -> l3i::Result<StackResults> {
    lua::push_list(call, declarations.iter(), |frame, declaration| {
        push_table(frame, 0, 2, |frame, table| {
            table.raw_set_value(frame, "name", &declaration.name)?;
            table.raw_set_value(frame, "type", declaration.ty.name())
        })
    })?;
    Ok(StackResults)
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

pub(crate) fn module(runtime: &Runtime, _jess: &Rc<Jess>) -> l3i::Result<Table> {
    let shader = Rc::new(Shader {
        current: RefCell::new(Vec::new()),
    });
    let module = Functions::module(runtime, "shader")?;
    for (name, ty) in TYPE_DESCRIPTORS {
        module.set(
            name,
            &Owned(LuaShaderType {
                ty,
                shader: shader.clone(),
            }),
        )?;
    }
    let state = shader.clone();
    module.function("program", move |call: &Call<'_>, spec: ValueView<'_>| {
        program(&state, call, spec)
    })?;
    let state = shader.clone();
    module.function(
        "sample",
        move |call: &Call<'_>, texture: ValueView<'_>, second: ValueView<'_>, third: Option<ValueView<'_>>| {
            let build = state.build_for("sample", &[texture, second])?;
            sample(call, &build, texture, second, third)
        },
    )?;
    for operation in [Binary::Min, Binary::Max, Binary::Dot, Binary::Cross] {
        let state = shader.clone();
        module.function(
            operation.name(),
            move |call: &Call<'_>, left: ValueView<'_>, right: ValueView<'_>| {
                let build = state.build_for(operation.name(), &[left, right])?;
                binary(call, &build, operation, left, right)
            },
        )?;
    }
    for operation in [Unary::Normalize, Unary::Length, Unary::Floor] {
        let state = shader.clone();
        module.function(operation.name(), move |call: &Call<'_>, value: ValueView<'_>| {
            let build = state.build_for(operation.name(), &[value])?;
            unary(call, &build, operation, value)
        })?;
    }
    for operation in [Ternary::Clamp, Ternary::Mix] {
        let state = shader.clone();
        module.function(
            operation.name(),
            move |call: &Call<'_>, first: ValueView<'_>, second: ValueView<'_>, third: ValueView<'_>| {
                let build = state.build_for(operation.name(), &[first, second, third])?;
                ternary(call, &build, operation, first, second, third)
            },
        )?;
    }
    module.finish()
}

/// Declares the shader types.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut ty = d.userdata::<LuaShaderType>(LuaShaderType::NAME);
    ty.tag(TagPolicy::Never)
        .doc("A shader type: a parameter or texture declaration, and a vector constructor.");
    crate::lua::plan::metamethod(&mut ty, "__call", construct);
    crate::lua::plan::metamethod(&mut ty, "__tostring", |this: &LuaShaderType| this.ty.name().to_owned());

    let mut expression = d.userdata::<LuaShaderExpr>(LuaShaderExpr::NAME);
    expression
        .tag(TagPolicy::Required)
        .doc("A symbolic value in the shader program under construction.");
    crate::lua::plan::metamethod(&mut expression, "__index", swizzle);
    crate::lua::plan::metamethod(
        &mut expression,
        "__add",
        |call: &Call<'_>, left: ValueView<'_>, right: ValueView<'_>| arithmetic(call, Binary::Add, left, right),
    );
    crate::lua::plan::metamethod(
        &mut expression,
        "__sub",
        |call: &Call<'_>, left: ValueView<'_>, right: ValueView<'_>| arithmetic(call, Binary::Subtract, left, right),
    );
    crate::lua::plan::metamethod(
        &mut expression,
        "__mul",
        |call: &Call<'_>, left: ValueView<'_>, right: ValueView<'_>| arithmetic(call, Binary::Multiply, left, right),
    );
    crate::lua::plan::metamethod(
        &mut expression,
        "__div",
        |call: &Call<'_>, left: ValueView<'_>, right: ValueView<'_>| arithmetic(call, Binary::Divide, left, right),
    );
    crate::lua::plan::metamethod(&mut expression, "__eq", |this: &LuaShaderExpr, other: ValueView<'_>| {
        l3i::userdata::receiver::<LuaShaderExpr>(other)
            .is_some_and(|other| Rc::ptr_eq(&this.build, &other.build) && this.value == other.value)
    });
    crate::lua::plan::metamethod(&mut expression, "__tostring", |this: &LuaShaderExpr| {
        format!("ShaderExpr({})", this.value.ty())
    });

    let mut input_type = d.userdata::<LuaShaderInput>(LuaShaderInput::NAME);
    input_type
        .tag(TagPolicy::Never)
        .doc("A stage function's input: parameters, textures, semantics, and the stage's own values.");
    crate::lua::plan::metamethod(&mut input_type, "__index", input);
    crate::lua::plan::metamethod(&mut input_type, "__tostring", |this: &LuaShaderInput| {
        format!("ShaderInput({})", this.stage.name())
    });

    let mut program = d.userdata::<LuaShaderProgram>(LuaShaderProgram::NAME);
    program.tag(TagPolicy::Never).doc("A finished, validated shader program.");
    program
        .getter("parameters", |this: &LuaShaderProgram, call: &Call<'_>| {
            let parameters: Vec<crate::shader::Declaration> = this
                .0
                .program
                .parameters()
                .iter()
                .filter(|declaration| !declaration.ty.is_resource())
                .cloned()
                .collect();
            push_declarations(call, &parameters)
        })
        .untyped();
    program
        .getter(
            "textures",
            |this: &LuaShaderProgram, call: &Call<'_>| -> l3i::Result<StackResults> {
                lua::push_list(call, this.0.textures.iter(), |frame, name| {
                    frame.push_string(name);
                    Ok(())
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    program
        .getter("varyings", |this: &LuaShaderProgram, call: &Call<'_>| {
            push_declarations(call, this.0.program.varyings())
        })
        .untyped();
    crate::lua::plan::metamethod(&mut program, "__tostring", |this: &LuaShaderProgram| {
        let parameters = this
            .0
            .program
            .parameters()
            .iter()
            .filter(|declaration| !declaration.ty.is_resource())
            .count();
        format!(
            "ShaderProgram({}, {}, {})",
            plural(parameters, "parameter"),
            plural(this.0.textures.len(), "texture"),
            plural(this.0.program.varyings().len(), "varying")
        )
    });
}
