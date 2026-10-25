//! `@jess/shader` (`jess.shader`): typed shader construction, a thin Luau handle on
//! [`jess::shader`](crate::shader)'s [`ProgramBuilder`].
//!
//! A shader module is an ordinary Luau module that runs once and returns a [`LuaShaderProgram`]
//! from `shader.program { attributes, instance, pass, material, draw, vertex, fragment }`. Its
//! vertex and fragment functions don't run on the GPU or per frame: each is called once, during
//! `shader.program`, with an input handle whose names are the declarations: `input.attributes` and
//! `input.instance` in the vertex function, `input.pass`, `input.material`, and `input.draw` in
//! both, and in the fragment function the varyings the vertex function returned. Every read and
//! every operation appends one typed expression to the program and hands back a symbolic value
//! ([`LuaShaderExpr`]); operators and swizzles on those values do the same. A bare Luau number in a
//! shader position is a `Float`.
//!
//! Every expression records where the Luau source wrote it (`file:line`, plus the module that
//! called `shader.program`), so a mistake fails at the call that made it, in Jess terms:
//! `shaders/lit.jsl:12 (shaders/lit.jsl): dot(Vec3f, Vec4f) is invalid: expected two vectors of
//! the same type`. Nothing here takes WGSL or exposes a GPU API.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use crate::shader::{
    AttributeId, DataType, Element, Field, Frequency, Place, PlaceRoot, Program, ProgramBuilder, Rate, ResourceId,
    ResourceKind, Scalar, Site, SourceOrigin, Step, StructId, Swizzle, Type, ValidationError, Value,
};
use l3i::Runtime;
use l3i::bind::{ArgView, Call, StackResults};
use l3i::debug::DebugScope;
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Frame, Scope, ValueView};
use l3i::userdata::{Owned, Userdata};
use l3i::value::{Function, Table};

use crate::lua::plan::Functions;
use crate::lua::{Jess, error, kind, push_table};

/// What `shader.program` takes.
const PROGRAM_KEYS: &[&str] = &["attributes", "draw", "fragment", "instance", "material", "pass", "vertex"];

/// The names a stage input answers with a namespace of declarations, by the attribute rate they
/// read.
const NAMESPACES: [(&str, Rate); 2] = [("attributes", Rate::Vertex), ("instance", Rate::Instance)];

/// The names a stage input answers with what a frequency binds, which are also the spec's keys.
const GROUPS: [(&str, Frequency); 3] = [
    ("pass", Frequency::Pass),
    ("material", Frequency::Material),
    ("draw", Frequency::Draw),
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

/// What one name a frequency binds is.
#[derive(Clone, Debug)]
#[allow(variant_size_differences)] // a handful per program, alive while it is built
enum Member {
    /// A uniform member, at its position among the frequency's members.
    Uniform { position: u32, descriptor: Descriptor },
    /// A texture and the sampler declared with it.
    Texture { texture: ResourceId, sampler: ResourceId },
    /// A sampler of its own.
    Sampler(ResourceId),
}

/// The names a program declares, each read once and handed out as the same value after that.
#[derive(Debug)]
struct Info {
    /// The module that called `shader.program`, on every origin.
    owner: Arc<str>,
    /// The last chunk an origin named, so a run of calls from one file shares the allocation.
    last_file: Option<Arc<str>>,
    /// The declared attributes of each rate, by name, with the value of the first read, where it
    /// was written.
    attributes: [BTreeMap<String, (AttributeId, Option<Value>)>; 2],
    /// What each frequency binds, by name.
    groups: [BTreeMap<String, Member>; 3],
    /// The structs declared so far, by the descriptor they came from.
    structs: HashMap<*const StructType, StructId>,
    /// Every uniform value read without an index, so reading one twice gives the same value.
    loads: HashMap<Place, Value>,
    /// What the vertex function returned, as the fragment stage reads it.
    varyings: BTreeMap<String, Value>,
}

impl Info {
    fn new(owner: Arc<str>) -> Self {
        Self {
            owner,
            last_file: None,
            attributes: [BTreeMap::new(), BTreeMap::new()],
            groups: [BTreeMap::new(), BTreeMap::new(), BTreeMap::new()],
            structs: HashMap::new(),
            loads: HashMap::new(),
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

    /// Every name an input of `stage` answers to, sorted, for the error naming one it doesn't.
    fn names(&self, stage: Stage) -> Vec<&str> {
        let mut names: Vec<&str> = GROUPS.iter().map(|(name, _)| *name).collect();
        match stage {
            Stage::Vertex => names.extend(NAMESPACES.iter().map(|(name, _)| *name)),
            Stage::Fragment => names.extend(self.varyings.keys().map(String::as_str)),
        }
        names.sort_unstable();
        names
    }
}

/// Why a vertex output can't be named `name`: the input already answers to it.
fn output_name_taken(name: &str) -> l3i::Result<()> {
    let reserved = NAMESPACES
        .iter()
        .map(|(namespace, _)| *namespace)
        .chain(GROUPS.iter().map(|(group, _)| *group));
    if reserved.into_iter().any(|reserved| reserved == name) {
        return Err(error(format!(
            "vertex output \"{name}\" has the name of a namespace of the input, which the input would hide"
        )));
    }
    Ok(())
}

/// What a shader type describes: a value, a texture or sampler, a struct, or an array.
#[derive(Clone, Debug)]
enum Descriptor {
    Value(Type),
    Resource(ResourceKind),
    Struct(Rc<StructType>),
    Array(ArrayElement, u32),
}

/// What an array of a [`Descriptor::Array`] holds.
#[derive(Clone, Debug)]
#[allow(variant_size_differences)] // a type beside a pointer: a few bytes either way
enum ArrayElement {
    Value(Type),
    Struct(Rc<StructType>),
}

impl ArrayElement {
    fn descriptor(&self) -> Descriptor {
        match self {
            ArrayElement::Value(ty) => Descriptor::Value(*ty),
            ArrayElement::Struct(structure) => Descriptor::Struct(structure.clone()),
        }
    }
}

/// A struct `shader.struct` made: its name and fields, sorted by name. Each program that uses it
/// declares it once.
#[derive(Debug)]
struct StructType {
    name: String,
    fields: Vec<(String, Descriptor)>,
}

impl Descriptor {
    /// The type's name, as errors and reflection print it: `Vec4f`, `Texture2D`, `Light`,
    /// `Light[8]`.
    fn name(&self) -> String {
        match self {
            Descriptor::Value(ty) => ty.name().to_owned(),
            Descriptor::Resource(kind) => kind.name().to_owned(),
            Descriptor::Struct(structure) => structure.name.clone(),
            Descriptor::Array(element, length) => format!("{}[{length}]", element.descriptor().name()),
        }
    }
}

/// A shader type: a declaration in a program's attributes and bindings, and, for the vectors and
/// scalars, a constructor.
#[derive(Clone, Debug)]
pub struct LuaShaderType {
    descriptor: Descriptor,
    shader: Rc<Shader>,
}

// SAFETY: `Rc`s to plain Rust state; the destructor never touches the Lua API.
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

/// A stage function's input: the namespaces of declarations and, in the fragment function, the
/// varyings.
#[derive(Clone, Debug)]
pub struct LuaShaderInput {
    build: Rc<RefCell<Build>>,
    stage: Stage,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderInput {
    const NAME: &'static str = "jess.shader.Input";
}

/// `input.attributes` or `input.instance`: the declared attributes of one rate, by name.
#[derive(Clone, Debug)]
pub struct LuaShaderAttributes {
    build: Rc<RefCell<Build>>,
    rate: Rate,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderAttributes {
    const NAME: &'static str = "jess.shader.Attributes";
}

/// Somewhere in what a frequency binds that isn't a value yet: `input.material`, a struct member,
/// an array. Indexing it goes one step further in; a step that reaches a value reads it.
#[derive(Clone, Debug)]
pub struct LuaShaderPlace {
    build: Rc<RefCell<Build>>,
    frequency: Frequency,
    place: Place,
    /// The values of the place's index steps, in order.
    indices: Vec<Value>,
    /// What the place is: `None` for the frequency's bindings themselves.
    at: Option<Descriptor>,
}

// SAFETY: `Rc`s to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderPlace {
    const NAME: &'static str = "jess.shader.Place";
}

/// A declared texture, with the sampler declared with it: what `shader.sample` reads.
#[derive(Clone, Debug)]
pub struct LuaShaderTexture {
    build: Rc<RefCell<Build>>,
    texture: ResourceId,
    sampler: ResourceId,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderTexture {
    const NAME: &'static str = "jess.shader.Texture";
}

/// A sampler declared on its own, for `shader.sample`'s three-argument form.
#[derive(Clone, Debug)]
pub struct LuaShaderSampler {
    build: Rc<RefCell<Build>>,
    sampler: ResourceId,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaShaderSampler {
    const NAME: &'static str = "jess.shader.Sampler";
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
    /// What each frequency binds, sorted by name, with its type's name: a sampler declared with a
    /// texture is part of the texture.
    groups: [Vec<(String, String)>; 3],
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
    operand_of(site, operation, view, Scalar::Float)
}

/// [`operand`], with a number as a literal of `scalar`: an integer constructor's numbers are its
/// integers, and must be whole numbers in its range.
fn operand_of(site: &mut Site<'_>, operation: &str, view: ValueView<'_>, scalar: Scalar) -> l3i::Result<Value> {
    if let Some(expression) = l3i::userdata::receiver::<LuaShaderExpr>(view) {
        return Ok(expression.value);
    }
    if !(view.is_number() || view.is_integer()) {
        return Err(error(format!(
            "{operation}: expected a shader value or a number, not {}",
            shader_kind(view)
        )));
    }
    let number = view.read::<f64>()?;
    let whole = |low: f64, high: f64, name: &str| {
        if number.fract() == 0.0 && (low..=high).contains(&number) {
            return Ok(number);
        }
        Err(error(format!(
            "{operation}: {number} is not {name}: expected a whole number from {low} to {high}"
        )))
    };
    match scalar {
        // In range and whole, checked above.
        #[allow(clippy::cast_possible_truncation)]
        Scalar::Int => Ok(site.int(whole(f64::from(i32::MIN), f64::from(i32::MAX), "an Int")? as i32)),
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Scalar::UInt => Ok(site.uint(whole(0.0, f64::from(u32::MAX), "a UInt")? as u32)),
        #[allow(clippy::cast_possible_truncation)] // Shader floats are f32.
        Scalar::Float | Scalar::Bool => site.float(number as f32).map_err(invalid),
    }
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
        .ok_or_else(|| {
            let other = [left, right]
                .into_iter()
                .find(|side| !(side.is_number() || side.is_integer()));
            let found = other.map_or_else(|| "two numbers".to_owned(), shader_kind);
            error(format!("{}: expected a shader value, not {found}", operation.name()))
        })?;
    binary(call, &build, operation, left, right)
}

/// What a Luau value is, as an error names it: a shader handle by what it is and what to do with
/// it instead, anything else by its Luau kind.
fn shader_kind(view: ValueView<'_>) -> String {
    if let Some(expression) = l3i::userdata::receiver::<LuaShaderExpr>(view) {
        return format!("a {} value", expression.value.ty());
    }
    if l3i::userdata::receiver::<LuaShaderTexture>(view).is_some() {
        return "a texture, which is sampled with shader.sample".to_owned();
    }
    if l3i::userdata::receiver::<LuaShaderSampler>(view).is_some() {
        return "a sampler, which shader.sample takes".to_owned();
    }
    if let Some(place) = l3i::userdata::receiver::<LuaShaderPlace>(view) {
        return match &place.at {
            None => format!("the {}: read one of its members", place.frequency),
            Some(Descriptor::Array(..)) => format!("{}, an array: read one of its elements", place_name(place)),
            Some(_) => format!("{}, a struct: read one of its fields", place_name(place)),
        };
    }
    kind(view).to_owned()
}

/// `shader.sample(texture, uv)` with the sampler declared with the texture, or
/// `shader.sample(texture, sampler, uv)` with a sampler declared on its own.
fn sample(
    call: &Call<'_>,
    texture: ValueView<'_>,
    second: ValueView<'_>,
    third: Option<ValueView<'_>>,
) -> l3i::Result<Owned<LuaShaderExpr>> {
    let Some(texture) = l3i::userdata::receiver::<LuaShaderTexture>(texture) else {
        return Err(error(format!(
            "shader.sample: expected a texture, such as input.material.diffuse, not {}",
            shader_kind(texture)
        )));
    };
    let (sampler, coordinates) = match third {
        None => (texture.sampler, second),
        Some(coordinates) => {
            let Some(sampler) = l3i::userdata::receiver::<LuaShaderSampler>(second) else {
                return Err(error(format!(
                    "shader.sample: expected a sampler, not {}",
                    shader_kind(second)
                )));
            };
            if !Rc::ptr_eq(&sampler.build, &texture.build) {
                return Err(error("shader.sample: the sampler belongs to another shader program"));
            }
            (sampler.sampler, coordinates)
        }
    };
    let build = texture.build.clone();
    let value = at_site(call, &build, |site, _| {
        let coordinates = operand(site, "sample", coordinates)?;
        site.sample(texture.texture, sampler, coordinates).map_err(invalid)
    })?;
    Ok(expression(&build, value))
}

/// The `jess.shader` members that are types, as `shader.<name>`.
const TYPE_DESCRIPTORS: [(&str, Type); 14] = [
    ("float", Type::Float),
    ("int", Type::Int),
    ("uint", Type::UInt),
    ("vec2", Type::Vec2f),
    ("vec3", Type::Vec3f),
    ("vec4", Type::Vec4f),
    ("vec2i", Type::Vec2i),
    ("vec3i", Type::Vec3i),
    ("vec4i", Type::Vec4i),
    ("vec2u", Type::Vec2u),
    ("vec3u", Type::Vec3u),
    ("vec4u", Type::Vec4u),
    ("mat3", Type::Mat3f),
    ("mat4", Type::Mat4f),
];

/// The `jess.shader` members that declare textures and samplers.
const RESOURCE_DESCRIPTORS: [(&str, ResourceKind); 2] =
    [("texture2d", ResourceKind::Texture2D), ("sampler", ResourceKind::Sampler)];

fn descriptor_name(ty: Type) -> &'static str {
    TYPE_DESCRIPTORS
        .iter()
        .find(|(_, descriptor)| *descriptor == ty)
        .map_or("type", |(name, _)| name)
}

/// `shader.vec4(...)` and the other value types called: a vector from values and numbers, or
/// `shader.float(number)`, a `Float` literal. The other types say they can't be constructed.
fn construct(this: &LuaShaderType, call: &Call<'_>, args: ArgView<'_>) -> l3i::Result<Owned<LuaShaderExpr>> {
    let Descriptor::Value(ty) = this.descriptor else {
        return Err(error(format!(
            "{} can't be constructed: it is declared, and read from the input",
            this.descriptor.name()
        )));
    };
    let views: Vec<ValueView<'_>> = (0..args.len()).map(|index| args.get(index)).collect();
    let build = this.shader.build_for(descriptor_name(ty), &views)?;
    let operation = format!("shader.{}", descriptor_name(ty));
    let scalar = ty.scalar();
    let value = at_site(call, &build, |site, _| {
        if ty.is_numeric_scalar()
            && let [only] = views.as_slice()
            && (only.is_number() || only.is_integer())
        {
            return operand_of(site, &operation, *only, scalar);
        }
        let components = views
            .iter()
            .map(|view| operand_of(site, &operation, *view, scalar))
            .collect::<l3i::Result<Vec<Value>>>()?;
        site.construct(ty, &components).map_err(invalid)
    })?;
    Ok(expression(&build, value))
}

/// A field's, member's, or element's type: a value, a struct, or (`arrays`) an array, not a
/// texture or sampler.
fn data_descriptor(value: ValueView<'_>, what: &str, arrays: bool) -> l3i::Result<Descriptor> {
    let Some(ty) = l3i::userdata::receiver::<LuaShaderType>(value) else {
        return Err(error(format!(
            "{what} must be a shader type such as shader.vec4, not {}",
            kind(value)
        )));
    };
    match &ty.descriptor {
        Descriptor::Resource(resource) => Err(error(format!(
            "{what} is a {}: textures and samplers are bound, not held in a struct or array",
            resource.name()
        ))),
        Descriptor::Array(..) if !arrays => Err(error(format!(
            "{what} is an array: an array holds values or structs, not arrays"
        ))),
        descriptor => Ok(descriptor.clone()),
    }
}

/// `shader.struct(name, fields)`: a struct type, which a program declares where it uses it.
fn structure(shader: &Rc<Shader>, call: &Call<'_>, name: &str, fields: ValueView<'_>) -> l3i::Result<Owned<LuaShaderType>> {
    if !fields.is_table() {
        return Err(error(format!(
            "shader.struct takes a name and a table of fields, not {}",
            kind(fields)
        )));
    }
    let mut declared = Vec::new();
    call.with_frame(|frame| {
        fields.as_table()?.for_each(frame, |_, key, value| {
            if !key.is_string() {
                return Err(error(format!("{name}'s fields are named by strings, not {}", kind(key))));
            }
            let field = key.read::<&str>()?;
            let descriptor = data_descriptor(value, &format!("{name}.{field}"), true)?;
            declared.push((field.to_owned(), descriptor));
            Ok(())
        })
    })?;
    declared.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Owned(LuaShaderType {
        descriptor: Descriptor::Struct(Rc::new(StructType {
            name: name.to_owned(),
            fields: declared,
        })),
        shader: shader.clone(),
    }))
}

/// `shader.array(element, length)`: an array type of `length` values or structs.
fn array(shader: &Rc<Shader>, element: ValueView<'_>, length: ValueView<'_>) -> l3i::Result<Owned<LuaShaderType>> {
    let element = match data_descriptor(element, "shader.array's element", false)? {
        Descriptor::Value(ty) => ArrayElement::Value(ty),
        Descriptor::Struct(structure) => ArrayElement::Struct(structure),
        Descriptor::Resource(_) | Descriptor::Array(..) => unreachable!("data_descriptor refused it"),
    };
    let count = if length.is_number() || length.is_integer() {
        length.read::<f64>()?
    } else {
        -1.0
    };
    if count.fract() != 0.0 || !(1.0..=f64::from(u32::MAX)).contains(&count) {
        return Err(error(format!(
            "shader.array's length must be a whole number of at least 1, not {}",
            if length.is_number() || length.is_integer() {
                count.to_string()
            } else {
                kind(length).to_owned()
            }
        )));
    }
    // Whole and in range, checked above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let length = count as u32;
    Ok(Owned(LuaShaderType {
        descriptor: Descriptor::Array(element, length),
        shader: shader.clone(),
    }))
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

/// `input.<name>` in a stage function: `input.attributes` and `input.instance`, `input.pass`,
/// `input.material`, and `input.draw`, or a varying.
fn input(this: &LuaShaderInput, call: &Call<'_>, key: ValueView<'_>) -> l3i::Result<StackResults> {
    if !key.is_string() {
        return Err(error(format!(
            "the {} input is indexed by name, not {}",
            this.stage.name(),
            kind(key)
        )));
    }
    let name = key.read::<&str>()?;
    let stage = this.stage;
    if let Some((_, rate)) = NAMESPACES.iter().find(|(namespace, _)| *namespace == name) {
        if stage == Stage::Fragment {
            return Err(error(format!(
                "the {} are only readable in the vertex function; return what the fragment function \
                 needs from them, to pass it as a varying",
                rate_phrase(*rate)
            )));
        }
        call.push(&Owned(LuaShaderAttributes {
            build: this.build.clone(),
            rate: *rate,
        }))?;
        return Ok(StackResults);
    }
    if let Some((_, frequency)) = GROUPS.iter().find(|(group, _)| *group == name) {
        let place = Place::new(PlaceRoot::Uniform(*frequency), &[]).expect("a root has no steps");
        call.push(&Owned(LuaShaderPlace {
            build: this.build.clone(),
            frequency: *frequency,
            place,
            indices: Vec::new(),
            at: None,
        }))?;
        return Ok(StackResults);
    }
    let value = at_site(call, &this.build, |_, info| {
        if stage == Stage::Fragment
            && let Some(value) = info.varyings.get(name)
        {
            return Ok(*value);
        }
        Err(error(format!(
            "the {} input has no \"{name}\": it has {}",
            stage.name(),
            info.names(stage).join(", ")
        )))
    })?;
    call.push(&expression(&this.build, value))?;
    Ok(StackResults)
}

/// What a place is, as errors name it: `the material`, `Light`, `Light[8]`.
fn place_name(this: &LuaShaderPlace) -> String {
    this.at
        .as_ref()
        .map_or_else(|| format!("the {}", this.frequency), Descriptor::name)
}

/// `place.<name>` or `place[index]`: one step further in. A step that reaches a value reads it; a
/// texture or sampler is its handle.
fn place_index(this: &LuaShaderPlace, call: &Call<'_>, key: ValueView<'_>) -> l3i::Result<StackResults> {
    let (step, index, descriptor) = match &this.at {
        None => {
            let name = place_key(this, key)?;
            let member = at_site(call, &this.build, |_, info| {
                let members = &info.groups[this.frequency as usize];
                members.get(name).cloned().ok_or_else(|| {
                    let names: Vec<&str> = members.keys().map(String::as_str).collect();
                    if names.is_empty() {
                        return error(format!(
                            "the {} has no \"{name}\": the program binds nothing there",
                            this.frequency
                        ));
                    }
                    error(format!(
                        "the {} has no \"{name}\": it has {}",
                        this.frequency,
                        names.join(", ")
                    ))
                })
            })?;
            match member {
                Member::Uniform { position, descriptor } => (Step::Field(position), None, descriptor),
                Member::Texture { texture, sampler } => {
                    call.push(&Owned(LuaShaderTexture {
                        build: this.build.clone(),
                        texture,
                        sampler,
                    }))?;
                    return Ok(StackResults);
                }
                Member::Sampler(sampler) => {
                    call.push(&Owned(LuaShaderSampler {
                        build: this.build.clone(),
                        sampler,
                    }))?;
                    return Ok(StackResults);
                }
            }
        }
        Some(Descriptor::Struct(structure)) => {
            let name = place_key(this, key)?;
            let Some(position) = structure.fields.iter().position(|(field, _)| field == name) else {
                let names: Vec<&str> = structure.fields.iter().map(|(field, _)| field.as_str()).collect();
                return Err(error(format!(
                    "{} has no field \"{name}\": it has {}",
                    structure.name,
                    names.join(", ")
                )));
            };
            // At most `u32::MAX` fields: they are a Luau table's.
            #[allow(clippy::cast_possible_truncation)]
            let position = position as u32;
            (Step::Field(position), None, structure.fields[position as usize].1.clone())
        }
        Some(Descriptor::Array(element, length)) => {
            let index = at_site(call, &this.build, |site, _| element_index(site, this, key, *length))?;
            (Step::Index(index.expression()), Some(index), element.descriptor())
        }
        Some(Descriptor::Value(_) | Descriptor::Resource(_)) => unreachable!("a place is never a value or resource"),
    };
    let Some(place) = this.place.then(step) else {
        return Err(error(
            "this place is too deep to read: a value is at most 8 fields and indices from its binding, \
             and at most 4 of them indices",
        ));
    };
    let mut indices = this.indices.clone();
    indices.extend(index);
    let Descriptor::Value(_) = descriptor else {
        call.push(&Owned(LuaShaderPlace {
            build: this.build.clone(),
            frequency: this.frequency,
            place,
            indices,
            at: Some(descriptor),
        }))?;
        return Ok(StackResults);
    };
    let value = at_site(call, &this.build, |site, info| {
        if indices.is_empty()
            && let Some(value) = info.loads.get(&place)
        {
            return Ok(*value);
        }
        let value = site.load(place, &indices).map_err(invalid)?;
        if indices.is_empty() {
            info.loads.insert(place, value);
        }
        Ok(value)
    })?;
    call.push(&expression(&this.build, value))?;
    Ok(StackResults)
}

/// The name a struct or a frequency's bindings are indexed by.
fn place_key<'a>(this: &LuaShaderPlace, key: ValueView<'a>) -> l3i::Result<&'a str> {
    if !key.is_string() {
        return Err(error(format!("{} is indexed by name, not {}", place_name(this), kind(key))));
    }
    key.read::<&str>()
}

/// An array's index: a shader value, or a number, a `UInt` that must be one of its elements.
fn element_index(site: &mut Site<'_>, this: &LuaShaderPlace, key: ValueView<'_>, length: u32) -> l3i::Result<Value> {
    if let Some(expression) = l3i::userdata::receiver::<LuaShaderExpr>(key) {
        return Ok(expression.value);
    }
    let name = place_name(this);
    if !(key.is_number() || key.is_integer()) {
        return Err(error(format!(
            "{name} is indexed by a number or a shader value, not {}",
            kind(key)
        )));
    }
    let number = key.read::<f64>()?;
    if number.fract() != 0.0 || !(0.0..f64::from(length)).contains(&number) {
        return Err(error(format!(
            "{name} has no element {number}: its elements are 0 to {}",
            length - 1
        )));
    }
    // Whole and in range, checked above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(site.uint(number as u32))
}

/// `#place`: an array's length.
fn place_length(this: &LuaShaderPlace) -> l3i::Result<u32> {
    match &this.at {
        Some(Descriptor::Array(_, length)) => Ok(*length),
        _ => Err(error(format!("{} has no length: only an array does", place_name(this)))),
    }
}

/// `input.attributes.<name>` and `input.instance.<name>`: a declared attribute.
fn attribute(this: &LuaShaderAttributes, call: &Call<'_>, key: ValueView<'_>) -> l3i::Result<Owned<LuaShaderExpr>> {
    let namespace = rate_phrase(this.rate);
    if !key.is_string() {
        return Err(error(format!("the {namespace} are indexed by name, not {}", kind(key))));
    }
    let name = key.read::<&str>()?;
    let value = at_site(call, &this.build, |site, info| {
        let declared = &mut info.attributes[this.rate as usize];
        let Some((attribute, read)) = declared.get_mut(name) else {
            let names: Vec<&str> = declared.keys().map(String::as_str).collect();
            if names.is_empty() {
                return Err(error(format!(
                    "the {namespace} have no \"{name}\": the program declares none"
                )));
            }
            return Err(error(format!(
                "the {namespace} have no \"{name}\": they have {}",
                names.join(", ")
            )));
        };
        Ok(*read.get_or_insert_with(|| site.read_attribute(*attribute)))
    })?;
    Ok(expression(&this.build, value))
}

/// What errors call the attributes of `rate`.
fn rate_phrase(rate: Rate) -> &'static str {
    match rate {
        Rate::Vertex => "attributes",
        Rate::Instance => "instance attributes",
    }
}

/// The namespace that reads attributes of `rate`.
fn namespace_name(rate: Rate) -> &'static str {
    NAMESPACES
        .iter()
        .find(|(_, of)| *of == rate)
        .map_or("attributes", |(name, _)| name)
}

/// A program spec's `attributes` or `instance` table: names to value types, sorted by name.
fn declarations(frame: &Frame<'_>, table: ValueView<'_>, what: &str) -> l3i::Result<Vec<(String, Type)>> {
    let declared = bindings(frame, table, what)?;
    declared
        .into_iter()
        .map(|(name, descriptor)| match descriptor {
            Descriptor::Value(ty) => Ok((name, ty)),
            other => Err(error(format!(
                "{what}.{name} must be a value type such as shader.vec3, not {}",
                other.name()
            ))),
        })
        .collect()
}

/// A program spec's `pass`, `material`, or `draw` table, or its attributes: names to types,
/// sorted by name.
fn bindings(frame: &Frame<'_>, table: ValueView<'_>, what: &str) -> l3i::Result<Vec<(String, Descriptor)>> {
    if table.is_nil() {
        return Ok(Vec::new());
    }
    if !table.is_table() {
        return Err(error(format!("{what} must be a table of shader types, not {}", kind(table))));
    }
    let mut declared = Vec::new();
    table.as_table()?.for_each(frame, |_, key, value| {
        if !key.is_string() {
            return Err(error(format!("the names in {what} must be strings, not {}", kind(key))));
        }
        let name = key.read::<&str>()?;
        let Some(ty) = l3i::userdata::receiver::<LuaShaderType>(value) else {
            return Err(error(format!(
                "{what}.{name} must be a shader type such as shader.vec4, not {}",
                kind(value)
            )));
        };
        declared.push((name.to_owned(), ty.descriptor.clone()));
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

/// What `shader.program` was given, read before anything is built.
struct Spec {
    /// The `attributes` and `instance` declarations, by rate.
    attributes: [Vec<(String, Type)>; 2],
    /// The `pass`, `material`, and `draw` declarations, by frequency.
    groups: [Vec<(String, Descriptor)>; 3],
    vertex: Function,
    fragment: Function,
}

/// The struct a descriptor came from, declared in the program if it isn't already, with every
/// struct it holds declared before it.
fn declare_struct(
    site: &mut Site<'_>,
    structs: &mut HashMap<*const StructType, StructId>,
    structure: &Rc<StructType>,
) -> l3i::Result<StructId> {
    if let Some(id) = structs.get(&Rc::as_ptr(structure)) {
        return Ok(*id);
    }
    let mut fields = Vec::with_capacity(structure.fields.len());
    for (name, descriptor) in &structure.fields {
        fields.push(Field::new(name, declare_type(site, structs, descriptor)?));
    }
    let id = site.structure(&structure.name, fields).map_err(invalid)?;
    structs.insert(Rc::as_ptr(structure), id);
    Ok(id)
}

/// The type a value, struct, or array descriptor is in the program, declaring its structs.
fn declare_type(
    site: &mut Site<'_>,
    structs: &mut HashMap<*const StructType, StructId>,
    descriptor: &Descriptor,
) -> l3i::Result<DataType> {
    let ty = match descriptor {
        Descriptor::Value(ty) => DataType::Value(*ty),
        Descriptor::Struct(structure) => DataType::Struct(declare_struct(site, structs, structure)?),
        Descriptor::Array(element, length) => {
            let element = match element {
                ArrayElement::Value(ty) => Element::Value(*ty),
                ArrayElement::Struct(structure) => Element::Struct(declare_struct(site, structs, structure)?),
            };
            DataType::Array {
                element,
                length: *length,
            }
        }
        Descriptor::Resource(_) => unreachable!("a texture or sampler is a resource, not a type"),
    };
    Ok(ty)
}

/// Declares what each frequency binds, sorted by name: a value, struct, or array is a uniform
/// member, a texture is a texture and the sampler of the same name, and a sampler is itself.
fn declare_groups(site: &mut Site<'_>, info: &mut Info, groups: [Vec<(String, Descriptor)>; 3]) -> l3i::Result<()> {
    for (frequency, declared) in Frequency::ALL.into_iter().zip(groups) {
        let mut position = 0;
        for (name, descriptor) in declared {
            let member = match descriptor {
                Descriptor::Resource(ResourceKind::Texture2D) => {
                    let texture = site.resource(frequency, &name, ResourceKind::Texture2D).map_err(invalid)?;
                    let sampler = site.resource(frequency, &name, ResourceKind::Sampler).map_err(invalid)?;
                    Member::Texture { texture, sampler }
                }
                Descriptor::Resource(ResourceKind::Sampler) => {
                    Member::Sampler(site.resource(frequency, &name, ResourceKind::Sampler).map_err(invalid)?)
                }
                descriptor => {
                    let ty = declare_type(site, &mut info.structs, &descriptor)?;
                    site.uniform(frequency, &name, ty).map_err(invalid)?;
                    position += 1;
                    Member::Uniform {
                        position: position - 1,
                        descriptor,
                    }
                }
            };
            info.groups[frequency as usize].insert(name, member);
        }
    }
    Ok(())
}

/// Builds the program: declares, runs the vertex function and passes what it returned, runs the
/// fragment function and outputs what it returned, then validates.
fn construct_program(call: &Call<'_>, build: &Rc<RefCell<Build>>, spec: Spec) -> l3i::Result<Finished> {
    let Spec {
        attributes,
        groups,
        vertex,
        fragment,
    } = spec;
    let declared_at = {
        let Build { builder, info } = &mut *build.borrow_mut();
        let origin = info.origin(call);
        let builder = builder.as_mut().expect("a program under construction has its builder");
        let mut site = builder.at(origin.clone());
        for (rate, declared) in Rate::ALL.into_iter().zip(attributes) {
            for (name, ty) in declared {
                let attribute = site.attribute(rate, &name, ty).map_err(invalid)?;
                info.attributes[rate as usize].insert(name, (attribute, None));
            }
        }
        declare_groups(&mut site, info, groups)?;
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
            output_name_taken(&name)?;
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

    let (builder, owner, groups) = {
        let Build { builder, info } = &mut *build.borrow_mut();
        let mut taken = builder.take().expect("a program under construction has its builder");
        let at = origin_of(&taken, output, &declared_at);
        taken.at(at).output(output).map_err(invalid)?;
        let groups = info.groups.clone().map(|members| {
            let reflect = |(name, member): (String, Member)| {
                let ty = match member {
                    Member::Uniform { descriptor, .. } => descriptor.name(),
                    Member::Texture { .. } => ResourceKind::Texture2D.name().to_owned(),
                    Member::Sampler(_) => ResourceKind::Sampler.name().to_owned(),
                };
                (name, ty)
            };
            members.into_iter().map(reflect).collect()
        });
        (taken, info.owner.clone(), groups)
    };
    let program = builder.finish().map_err(|problems| {
        let mut message = format!("{owner}: the shader program is invalid");
        for problem in problems {
            message.push('\n');
            message.push_str(&problem.to_string());
        }
        error(message)
    })?;
    Ok(Finished { program, owner, groups })
}

/// `shader.program { attributes, instance, pass, material, draw, vertex, fragment }`.
fn program(shader: &Rc<Shader>, call: &Call<'_>, spec: ValueView<'_>) -> l3i::Result<Owned<LuaShaderProgram>> {
    if !spec.is_table() {
        return Err(error(format!("shader.program takes a table, not {}", kind(spec))));
    }
    let owner: Arc<str> = Arc::from(call_site(call).0);
    let spec = call.with_frame(|frame| {
        crate::lua::check_options(frame, spec, "jess.shader.program", PROGRAM_KEYS)?;
        let table = spec.as_table()?;
        let attributes = [
            declarations(frame, table.raw_get(frame, "attributes")?, "attributes")?,
            declarations(frame, table.raw_get(frame, "instance")?, "instance")?,
        ];
        let [pass, material, draw] = GROUPS.map(|(name, _)| name);
        let groups = [
            bindings(frame, table.raw_get(frame, pass)?, pass)?,
            bindings(frame, table.raw_get(frame, material)?, material)?,
            bindings(frame, table.raw_get(frame, draw)?, draw)?,
        ];
        Ok(Spec {
            attributes,
            groups,
            vertex: stage_function(table.raw_get(frame, "vertex")?, "vertex")?,
            fragment: stage_function(table.raw_get(frame, "fragment")?, "fragment")?,
        })
    })?;
    let build = Rc::new(RefCell::new(Build {
        builder: Some(ProgramBuilder::new()),
        info: Info::new(owner),
    }));
    shader.current.borrow_mut().push(build.clone());
    let finished = construct_program(call, &build, spec);
    shader.current.borrow_mut().pop();
    // Whatever happened, the values handed out during construction belong to no program now.
    build.borrow_mut().builder = None;
    Ok(Owned(LuaShaderProgram(Rc::new(finished?))))
}

/// Pushes `{ name, type }` for each declaration.
fn push_declarations(call: &Call<'_>, declarations: &[crate::shader::Declaration]) -> l3i::Result<StackResults> {
    l3i::stack::push_list(call, declarations.iter(), |frame, declaration| {
        push_table(frame, 0, 2, |frame, table| {
            table.raw_set_value(frame, "name", &declaration.name)?;
            table.raw_set_value(frame, "type", declaration.ty.name())
        })
    })?;
    Ok(StackResults)
}

/// Pushes `{ name, type }` for each `(name, type)`.
fn push_named(call: &Call<'_>, named: &[(String, String)]) -> l3i::Result<StackResults> {
    l3i::stack::push_list(call, named.iter(), |frame, (name, ty)| {
        push_table(frame, 0, 2, |frame, table| {
            table.raw_set_value(frame, "name", name)?;
            table.raw_set_value(frame, "type", ty)
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
    let values = TYPE_DESCRIPTORS.map(|(name, ty)| (name, Descriptor::Value(ty)));
    let resources = RESOURCE_DESCRIPTORS.map(|(name, kind)| (name, Descriptor::Resource(kind)));
    for (name, descriptor) in values.into_iter().chain(resources) {
        module.set(
            name,
            &Owned(LuaShaderType {
                descriptor,
                shader: shader.clone(),
            }),
        )?;
    }
    let state = shader.clone();
    module.function("program", move |call: &Call<'_>, spec: ValueView<'_>| {
        program(&state, call, spec)
    })?;
    let state = shader.clone();
    module.function("struct", move |call: &Call<'_>, name: &str, fields: ValueView<'_>| {
        structure(&state, call, name, fields)
    })?;
    let state = shader.clone();
    module.function("array", move |element: ValueView<'_>, length: ValueView<'_>| {
        array(&state, element, length)
    })?;
    module.function(
        "sample",
        move |call: &Call<'_>, texture: ValueView<'_>, second: ValueView<'_>, third: Option<ValueView<'_>>| {
            sample(call, texture, second, third)
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
        .doc("A shader type: an attribute, a binding, a struct field, or an array element, and a value constructor.");
    crate::lua::plan::metamethod(&mut ty, "__call", construct);
    crate::lua::plan::metamethod(&mut ty, "__tostring", |this: &LuaShaderType| this.descriptor.name());

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
        .doc("A stage function's input: its namespaces of declarations, and the stage's own values.");
    crate::lua::plan::metamethod(&mut input_type, "__index", input);
    crate::lua::plan::metamethod(&mut input_type, "__tostring", |this: &LuaShaderInput| {
        format!("ShaderInput({})", this.stage.name())
    });

    let mut attributes = d.userdata::<LuaShaderAttributes>(LuaShaderAttributes::NAME);
    attributes
        .tag(TagPolicy::Never)
        .doc("The declared attributes of one rate, by name: `input.attributes` or `input.instance`.");
    crate::lua::plan::metamethod(&mut attributes, "__index", attribute);
    crate::lua::plan::metamethod(&mut attributes, "__tostring", |this: &LuaShaderAttributes| {
        format!("ShaderAttributes({})", namespace_name(this.rate))
    });

    describe_bindings(d);
    describe_program(d);
}

/// Gives a handle that isn't a value the arithmetic metamethods, so `texture * 2` fails naming
/// what the handle is instead of with Luau's own error.
fn refuse_arithmetic<T: Userdata>(ty: &mut UserdataBuilder<'_, T>) {
    let operations = [
        ("__add", Binary::Add),
        ("__sub", Binary::Subtract),
        ("__mul", Binary::Multiply),
        ("__div", Binary::Divide),
    ];
    for (name, operation) in operations {
        crate::lua::plan::metamethod(ty, name, move |call: &Call<'_>, left: ValueView<'_>, right: ValueView<'_>| {
            arithmetic(call, operation, left, right)
        });
    }
}

/// Declares places, textures, and samplers.
fn describe_bindings(d: &mut ExtensionDescriptor) {
    let mut place = d.userdata::<LuaShaderPlace>(LuaShaderPlace::NAME);
    place
        .tag(TagPolicy::Never)
        .doc("What a frequency binds, a struct, or an array, on the way to a value: `input.material`.");
    crate::lua::plan::metamethod(&mut place, "__index", place_index);
    refuse_arithmetic(&mut place);
    // Luau passes the operand of `#` twice.
    crate::lua::plan::metamethod(&mut place, "__len", |this: &LuaShaderPlace, _: Option<ValueView<'_>>| {
        place_length(this)
    });
    crate::lua::plan::metamethod(&mut place, "__tostring", |this: &LuaShaderPlace| {
        let at = this
            .at
            .as_ref()
            .map_or_else(|| this.frequency.name().to_owned(), Descriptor::name);
        format!("ShaderPlace({at})")
    });

    let mut texture = d.userdata::<LuaShaderTexture>(LuaShaderTexture::NAME);
    texture.tag(TagPolicy::Never).doc("A declared texture, for `shader.sample`.");
    refuse_arithmetic(&mut texture);
    crate::lua::plan::metamethod(&mut texture, "__tostring", |_: &LuaShaderTexture| {
        format!("ShaderTexture({})", ResourceKind::Texture2D.name())
    });

    let mut sampler = d.userdata::<LuaShaderSampler>(LuaShaderSampler::NAME);
    sampler
        .tag(TagPolicy::Never)
        .doc("A sampler declared on its own, for `shader.sample`.");
    refuse_arithmetic(&mut sampler);
    crate::lua::plan::metamethod(&mut sampler, "__tostring", |_: &LuaShaderSampler| {
        format!("ShaderSampler({})", ResourceKind::Sampler.name())
    });
}

/// Declares the finished program and what it reflects.
fn describe_program(d: &mut ExtensionDescriptor) {
    let mut program = d.userdata::<LuaShaderProgram>(LuaShaderProgram::NAME);
    program.tag(TagPolicy::Never).doc("A finished, validated shader program.");
    for rate in Rate::ALL {
        program
            .getter(namespace_name(rate), move |this: &LuaShaderProgram, call: &Call<'_>| {
                let attributes: Vec<(String, String)> = this
                    .0
                    .program
                    .attributes()
                    .iter()
                    .filter(|attribute| attribute.rate == rate)
                    .map(|attribute| (attribute.name.clone(), attribute.ty.name().to_owned()))
                    .collect();
                push_named(call, &attributes)
            })
            .untyped();
    }
    for (name, frequency) in GROUPS {
        program
            .getter(name, move |this: &LuaShaderProgram, call: &Call<'_>| {
                push_named(call, &this.0.groups[frequency as usize])
            })
            .untyped();
    }
    program
        .getter("varyings", |this: &LuaShaderProgram, call: &Call<'_>| {
            push_declarations(call, this.0.program.varyings())
        })
        .untyped();
    crate::lua::plan::metamethod(&mut program, "__tostring", |this: &LuaShaderProgram| {
        let textures = this
            .0
            .groups
            .iter()
            .flatten()
            .filter(|(_, ty)| ty == ResourceKind::Texture2D.name())
            .count();
        format!(
            "ShaderProgram({}, {}, {}, {})",
            plural(this.0.program.attributes().len(), "attribute"),
            plural(this.0.program.uniforms().len(), "uniform"),
            plural(textures, "texture"),
            plural(this.0.program.varyings().len(), "varying")
        )
    });
}
