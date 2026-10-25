//! Lowering a validated [`Program`] to one WGSL module.
//!
//! Every backend name comes from here and follows one scheme: `u_pass`, `u_material`, and
//! `u_draw` the uniform blocks (of structs `PassUniforms`, `MaterialUniforms`, `DrawUniforms`),
//! `SN_Name` the declared structs (N their declaration position), `f_` struct fields and uniform
//! members, `t_frequency_name`/`s_frequency_name` textures and samplers, `a_` per-vertex and `i_`
//! per-instance attributes (a matrix attribute's columns are `mN_name`, N its location), `v_`
//! varyings, `colorN` fragment outputs, and `eN` temporaries numbered from 0 in each entry point,
//! in arena order. Every non-leaf expression a stage reads becomes one `let`; leaves (literals,
//! attributes, varyings, and uniform values reached without an index) are written in place.
//!
//! Bindings and layouts are [`layout`](crate::shader::layout)'s, written out: each frequency's
//! uniform block at binding 0 of its group, its resources after it, every struct and block member
//! in memory order, with `@align` and `@size` wherever Jess's rule differs from WGSL's own.

use std::fmt::Write as _;

use crate::shader::expression::{
    Builtin, Expression, ExpressionId, Function, Literal, Place, PlaceRoot, SampleLevel, Step, TextureQuery,
};
use crate::shader::flow::{Stage, Statement};
use crate::shader::interface::{Access, AttributeId, DataType, Element, Frequency, Rate, ResourceId, ResourceKind};
use crate::shader::layout::{MemberLayout, ProgramLayout, StructLayout, value_extent};
use crate::shader::program::Program;
use crate::shader::types::Type;
use crate::shader::validate::{Analysis, COMPUTE, FRAGMENT, VERTEX, ValidationError, analyze};

/// The vertex entry point's name.
pub const VERTEX_ENTRY_POINT: &str = "vertex_main";
/// The fragment entry point's name.
pub const FRAGMENT_ENTRY_POINT: &str = "fragment_main";

/// The compute entry point's name.
pub const COMPUTE_ENTRY_POINT: &str = "compute_main";

/// A lowered program: the WGSL, and every binding, layout, and location the renderer must match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgslModule {
    /// The WGSL source, with entry points [`VERTEX_ENTRY_POINT`] and [`FRAGMENT_ENTRY_POINT`], or
    /// [`COMPUTE_ENTRY_POINT`] for a compute program.
    pub source: String,
    /// Where everything the program declares lives: uniform blocks, structs, bindings, and vertex
    /// buffers.
    pub layout: ProgramLayout,
    /// What the fragment stage writes to each color target, by location: one type per target.
    pub outputs: Vec<Type>,
}

/// Validates `program` and lowers it as it is; run
/// [`fold_constants`](Program::fold_constants) first to fold. The output depends only on the
/// program, so equal programs lower to byte-identical WGSL.
///
/// # Errors
///
/// The program's validation errors, as [`Program::validate`] reports them.
pub fn lower(program: &Program) -> Result<WgslModule, Vec<ValidationError>> {
    let analysis = analyze(program)?;
    // Validation typed every output.
    let outputs = program
        .fragment_outputs
        .iter()
        .map(|output| analysis.types[output.index()].unwrap_or(Type::Vec4f))
        .collect();
    let layout = program.layout();
    let mut lowering = Lowering {
        program,
        analysis: &analysis,
        layout: &layout,
        source: String::with_capacity(1024 + program.expressions.len() * 48),
        temporaries: vec![0; program.expressions.len()],
        next: 0,
    };

    lowering.structs();
    lowering.bindings();
    if let Some(size) = program.workgroup_size {
        lowering.compute_entry_point(size);
    } else {
        lowering.vertex_input_struct();
        lowering.vertex_output_struct();
        lowering.fragment_output_struct();
        lowering.vertex_entry_point();
        lowering.fragment_entry_point();
    }

    Ok(WgslModule {
        source: lowering.source,
        layout,
        outputs,
    })
}

/// One level of a body's indentation.
const INDENT: &str = "    ";

/// Writes to the output. Writing to a `String` cannot fail.
macro_rules! emit {
    ($lowering:expr, $($format:tt)*) => {
        $lowering.source.write_fmt(format_args!($($format)*)).unwrap_or(())
    };
}

struct Lowering<'a> {
    program: &'a Program,
    analysis: &'a Analysis,
    layout: &'a ProgramLayout,
    source: String,
    /// Each non-leaf expression's temporary number in the entry point being written.
    temporaries: Vec<u32>,
    /// The next temporary number.
    next: u32,
}

impl Lowering<'_> {
    /// Every declared struct, in declaration order, which is dependency order.
    fn structs(&mut self) {
        let (program, layout) = (self.program, self.layout);
        for (index, declaration) in program.structs.iter().enumerate() {
            emit!(self, "struct {} {{\n", struct_name(program, index));
            let members = declaration.fields.iter().map(|field| (field.name.as_str(), field.ty));
            self.members(&members.collect::<Vec<_>>(), &layout.structs[index]);
            emit!(self, "}}\n\n");
        }
    }

    /// The members of a struct or uniform block, in memory order.
    fn members(&mut self, members: &[(&str, DataType)], layout: &StructLayout) {
        for &position in &layout.order {
            let (name, ty) = members[position as usize];
            let placed = layout.members[position as usize];
            emit!(self, "    ");
            self.member_attributes(ty, placed);
            emit!(self, "f_{name}: {},\n", self.data_type(ty));
        }
    }

    /// `@align` and `@size` where Jess's layout differs from what WGSL would pick by itself.
    fn member_attributes(&mut self, ty: DataType, placed: MemberLayout) {
        let natural = match ty {
            DataType::Value(_) => return,
            DataType::Struct(id) => self.layout.structure(id).map(|layout| (layout.align, layout.size)),
            DataType::Array { element, length } => {
                let align = match element {
                    Element::Value(ty) => value_extent(ty).map(|extent| extent.align),
                    Element::Struct(id) => self.layout.structure(id).map(|layout| layout.align),
                };
                align.map(|align| (align, self.layout.element_stride(element) * length))
            }
        };
        let Some((align, size)) = natural else {
            return;
        };
        if placed.align != align {
            emit!(self, "@align({}) ", placed.align);
        }
        if placed.size != size {
            emit!(self, "@size({}) ", placed.size);
        }
    }

    fn data_type(&self, ty: DataType) -> String {
        match ty {
            DataType::Value(ty) => wgsl_type(ty).to_owned(),
            DataType::Struct(id) => struct_name(self.program, id.index()),
            DataType::Array { element, length } => {
                let element = match element {
                    Element::Value(ty) => wgsl_type(ty).to_owned(),
                    Element::Struct(id) => struct_name(self.program, id.index()),
                };
                format!("array<{element}, {length}>")
            }
        }
    }

    /// Each frequency's uniform block and resources, at the bindings the layout gave them.
    fn bindings(&mut self) {
        let (program, layout) = (self.program, self.layout);
        for group in &layout.groups {
            let frequency = group.frequency;
            if let Some(block) = &group.uniform {
                emit!(self, "struct {} {{\n", block_name(frequency));
                let members = program
                    .uniforms_of(frequency)
                    .map(|(_, uniform)| (uniform.name.as_str(), uniform.ty));
                self.members(&members.collect::<Vec<_>>(), block);
                emit!(
                    self,
                    "}}\n\n@group({}) @binding(0) var<uniform> u_{frequency}: {};\n\n",
                    frequency.group(),
                    block_name(frequency)
                );
            }
            for binding in &group.resources {
                let resource = &program.resources[binding.resource.index()];
                let ty = match resource.kind {
                    ResourceKind::Texture2D => "texture_2d<f32>",
                    ResourceKind::Texture2Di => "texture_2d<i32>",
                    ResourceKind::Texture2Du => "texture_2d<u32>",
                    ResourceKind::Texture2DArray => "texture_2d_array<f32>",
                    ResourceKind::TextureCube => "texture_cube<f32>",
                    ResourceKind::Texture3D => "texture_3d<f32>",
                    ResourceKind::DepthTexture2D => "texture_depth_2d",
                    ResourceKind::DepthTexture2DArray => "texture_depth_2d_array",
                    ResourceKind::DepthTextureCube => "texture_depth_cube",
                    ResourceKind::Sampler => "sampler",
                    ResourceKind::ComparisonSampler => "sampler_comparison",
                };
                let prefix = resource_prefix(resource.kind);
                emit!(
                    self,
                    "@group({}) @binding({}) var {prefix}_{frequency}_{}: {ty};\n",
                    binding.group,
                    binding.binding,
                    resource.name
                );
            }
            if !group.resources.is_empty() {
                emit!(self, "\n");
            }
            for binding in &group.storage {
                let buffer = &program.storage[binding.storage.index()];
                let access = match buffer.access {
                    Access::Read => "read",
                    Access::ReadWrite | Access::Atomic => "read_write",
                };
                let element = match buffer.element {
                    Element::Value(ty) if buffer.access == Access::Atomic => format!("atomic<{}>", wgsl_type(ty)),
                    Element::Value(ty) => wgsl_type(ty).to_owned(),
                    Element::Struct(id) => struct_name(program, id.index()),
                };
                emit!(
                    self,
                    "@group({}) @binding({}) var<storage, {access}> b_{frequency}_{}: array<{element}>;\n",
                    binding.group,
                    binding.binding,
                    buffer.name
                );
            }
            if !group.storage.is_empty() {
                emit!(self, "\n");
            }
        }
    }

    fn vertex_input_struct(&mut self) {
        if self.program.attributes.is_empty() {
            return;
        }
        emit!(self, "struct VertexInput {{\n");
        let layout = self.layout;
        for attribute in layout.vertex_buffers.iter().flat_map(|buffer| &buffer.attributes) {
            let declaration = &self.program.attributes[attribute.attribute.index()];
            let Some(column) = declaration.ty.column() else {
                let name = attribute_name(declaration.rate, &declaration.name);
                emit!(
                    self,
                    "    @location({}) {name}: {},\n",
                    attribute.location,
                    wgsl_type(declaration.ty)
                );
                continue;
            };
            for location in attribute.location..attribute.location + attribute.locations {
                emit!(
                    self,
                    "    @location({location}) m{location}_{}: {},\n",
                    declaration.name,
                    wgsl_type(column)
                );
            }
        }
        emit!(self, "}}\n\n");
    }

    /// The varyings the vertex stage writes, in declaration order, with what it writes.
    fn written_varyings(&self) -> Vec<Option<ExpressionId>> {
        let mut written = vec![None; self.program.varyings.len()];
        for &(varying, expression) in &self.program.varying_writes {
            written[varying.index()] = Some(expression);
        }
        written
    }

    fn vertex_output_struct(&mut self) {
        emit!(self, "struct VertexOutput {{\n    @builtin(position) position: vec4<f32>,\n");
        let mut location = 0;
        for (declaration, written) in self.program.varyings.iter().zip(self.written_varyings()) {
            if written.is_none() {
                continue;
            }
            let interpolation = if declaration.ty.float_components() == 0 {
                " @interpolate(flat)"
            } else {
                ""
            };
            emit!(
                self,
                "    @location({location}){interpolation} v_{}: {},\n",
                declaration.name,
                wgsl_type(declaration.ty)
            );
            location += 1;
        }
        emit!(self, "}}\n\n");
    }

    fn fragment_output_struct(&mut self) {
        if self.program.fragment_outputs.is_empty() {
            return;
        }
        emit!(self, "struct FragmentOutput {{\n");
        for (location, &output) in self.program.fragment_outputs.iter().enumerate() {
            let ty = wgsl_type(self.type_of(output));
            emit!(self, "    @location({location}) color{location}: {ty},\n");
        }
        emit!(self, "}}\n\n");
    }

    fn vertex_entry_point(&mut self) {
        let mut parameters = Vec::new();
        if !self.program.attributes.is_empty() {
            parameters.push("input: VertexInput".to_owned());
        }
        parameters.extend(self.builtin_parameters(VERTEX));
        let parameters = parameters.join(", ");
        emit!(self, "@vertex\nfn {VERTEX_ENTRY_POINT}({parameters}) -> VertexOutput {{\n");
        self.stage_body(Stage::Vertex, VERTEX);
        emit!(self, "    var output: VertexOutput;\n    output.position = ");
        let program = self.program;
        // Validation guarantees the position exists.
        if let Some(position) = program.position {
            self.operand(position);
        }
        emit!(self, ";\n");
        for (declaration, written) in program.varyings.iter().zip(self.written_varyings()) {
            let Some(expression) = written else {
                continue;
            };
            emit!(self, "    output.v_{} = ", declaration.name);
            self.operand(expression);
            emit!(self, ";\n");
        }
        emit!(self, "    return output;\n}}\n\n");
    }

    fn fragment_entry_point(&mut self) {
        let outputs = self.program.fragment_outputs.len();
        let result = if outputs == 0 { "" } else { " -> FragmentOutput" };
        let mut parameters = vec!["input: VertexOutput".to_owned()];
        parameters.extend(self.builtin_parameters(FRAGMENT));
        let parameters = parameters.join(", ");
        emit!(self, "@fragment\nfn {FRAGMENT_ENTRY_POINT}({parameters}){result} {{\n");
        self.stage_body(Stage::Fragment, FRAGMENT);
        if outputs > 0 {
            emit!(self, "    var output: FragmentOutput;\n");
            for location in 0..outputs {
                emit!(self, "    output.color{location} = ");
                self.operand(self.program.fragment_outputs[location]);
                emit!(self, ";\n");
            }
            emit!(self, "    return output;\n");
        }
        emit!(self, "}}\n");
    }

    fn compute_entry_point(&mut self, [x, y, z]: [u32; 3]) {
        let parameters = self.builtin_parameters(COMPUTE).join(", ");
        emit!(
            self,
            "@compute @workgroup_size({x}, {y}, {z})\nfn {COMPUTE_ENTRY_POINT}({parameters}) {{\n"
        );
        self.stage_body(Stage::Compute, COMPUTE);
        emit!(self, "}}\n");
    }

    /// The entry point parameters of the built-in values `stage` reads, in [`Builtin::ALL`]'s
    /// order. The fragment position is the vertex output's, so it needs none.
    fn builtin_parameters(&self, stage: u8) -> Vec<String> {
        Builtin::ALL
            .into_iter()
            .filter(|builtin| *builtin != Builtin::Position)
            .filter(|builtin| {
                self.program.expressions.iter().enumerate().any(|(index, expression)| {
                    *expression == Expression::Builtin(*builtin) && self.analysis.stages[index] & stage != 0
                })
            })
            .map(|builtin| {
                let name = builtin_name(builtin);
                format!("@builtin({name}) {name}: {}", wgsl_type(builtin.ty()))
            })
            .collect()
    }

    /// A stage's locals, then a `let eN` per non-leaf expression it reads that no statement
    /// places, in arena order (which is dependency order), then its statements.
    fn stage_body(&mut self, stage: Stage, bit: u8) {
        self.next = 0;
        let program = self.program;
        for (index, local) in program.locals.iter().enumerate() {
            if local.stage == stage {
                emit!(self, "    var l{index}: {};\n", wgsl_type(local.ty));
            }
        }
        for index in 0..program.expressions.len() {
            if self.analysis.placed[index].is_none() {
                self.temporary(ExpressionId::at(index), bit, 1);
            }
        }
        self.statements(program.body(stage), bit, 1);
    }

    /// `let eN = ...;` for `id`, if `bit`'s stage reads it and it isn't a leaf, written where
    /// it is read.
    fn temporary(&mut self, id: ExpressionId, bit: u8, depth: usize) {
        if self.analysis.stages[id.index()] & bit == 0 || self.program.expressions[id.index()].is_leaf() {
            return;
        }
        self.temporaries[id.index()] = self.next;
        emit!(self, "{}let e{} = ", INDENT.repeat(depth), self.next);
        self.operation(id);
        emit!(self, ";\n");
        self.next += 1;
    }

    fn statements(&mut self, block: &[Statement], bit: u8, depth: usize) {
        let indent = INDENT.repeat(depth);
        for statement in block {
            match statement {
                Statement::Emit(id) => self.temporary(*id, bit, depth),
                Statement::Store { local, value, .. } => {
                    emit!(self, "{indent}l{} = ", local.index());
                    self.operand(*value);
                    emit!(self, ";\n");
                }
                Statement::Write { place, value, .. } if self.atomic(place) => {
                    emit!(self, "{indent}atomicStore(&");
                    self.place(place);
                    emit!(self, ", ");
                    self.operand(*value);
                    emit!(self, ");\n");
                }
                Statement::Write { place, value, .. } => {
                    emit!(self, "{indent}");
                    self.place(place);
                    emit!(self, " = ");
                    self.operand(*value);
                    emit!(self, ";\n");
                }
                Statement::Atomic {
                    op,
                    place,
                    value,
                    result,
                    ..
                } => {
                    emit!(self, "{indent}l{} = {}(&", result.index(), op.wgsl());
                    self.place(place);
                    emit!(self, ", ");
                    self.operand(*value);
                    emit!(self, ");\n");
                }
                Statement::If {
                    condition,
                    accept,
                    reject,
                    ..
                } => {
                    emit!(self, "{indent}if ");
                    self.operand(*condition);
                    emit!(self, " {{\n");
                    self.statements(accept, bit, depth + 1);
                    if !reject.is_empty() {
                        emit!(self, "{indent}}} else {{\n");
                        self.statements(reject, bit, depth + 1);
                    }
                    emit!(self, "{indent}}}\n");
                }
                Statement::Loop { index, count, body, .. } => {
                    // Validation made the index and count one integer type.
                    let (zero, one) = if self.program.locals[index.index()].ty == Type::Int {
                        ("0i", "1i")
                    } else {
                        ("0u", "1u")
                    };
                    let index = format!("l{}", index.index());
                    emit!(self, "{indent}for ({index} = {zero}; {index} < ");
                    self.operand(*count);
                    emit!(self, "; {index} = {index} + {one}) {{\n");
                    self.statements(body, bit, depth + 1);
                    emit!(self, "{indent}}}\n");
                }
                Statement::Discard { .. } => emit!(self, "{indent}discard;\n"),
            }
        }
    }

    fn type_of(&self, id: ExpressionId) -> Type {
        // Validation typed every expression a stage reads.
        self.analysis.types[id.index()].unwrap_or(Type::Bool)
    }

    fn operation(&mut self, id: ExpressionId) {
        let expression = self.program.expressions[id.index()];
        match expression {
            // WGSL takes a texture function's operands in Jess's order: coordinates, layer, level.
            Expression::Sample {
                texture, sampler, level, ..
            } => {
                let function = match level {
                    SampleLevel::Implicit => "textureSample",
                    SampleLevel::Bias(_) => "textureSampleBias",
                    SampleLevel::Explicit(_) => "textureSampleLevel",
                };
                self.texture_call(function, &[texture, sampler], &expression);
            }
            Expression::SampleCompare {
                texture,
                sampler,
                level_zero,
                ..
            } => {
                let function = if level_zero {
                    "textureSampleCompareLevel"
                } else {
                    "textureSampleCompare"
                };
                self.texture_call(function, &[texture, sampler], &expression);
            }
            Expression::TextureLoad { texture, .. } => self.texture_call("textureLoad", &[texture], &expression),
            Expression::TextureQuery { texture, query } => {
                let function = match query {
                    TextureQuery::Dimensions(_) => "textureDimensions",
                    TextureQuery::Levels => "textureNumLevels",
                    TextureQuery::Layers => "textureNumLayers",
                };
                self.texture_call(function, &[texture], &expression);
            }
            Expression::Load(place) if self.atomic(&place) => {
                emit!(self, "atomicLoad(&");
                self.place(&place);
                emit!(self, ")");
            }
            Expression::Load(place) => self.place(&place),
            Expression::Local(local) => emit!(self, "l{}", local.index()),
            Expression::StorageLength(storage) => {
                let buffer = &self.program.storage[storage.index()];
                emit!(self, "arrayLength(&b_{}_{})", buffer.frequency, buffer.name);
            }
            // Parenthesized: `--1.0` is WGSL's decrement, not the negation of `-1.0`.
            Expression::Unary(operator, value) => {
                emit!(self, "{}(", operator.symbol());
                self.operand(value);
                emit!(self, ")");
            }
            Expression::Binary(operator, left, right) => {
                self.operand(left);
                emit!(self, " {} ", operator.symbol());
                self.operand(right);
            }
            Expression::Select {
                condition,
                accept,
                reject,
            } => self.call("select", &[reject, accept, condition], None),
            Expression::Call { function, arguments } => {
                // WGSL takes no scalar beside a vector in these, which Jess does.
                let splat = matches!(
                    function,
                    Function::Min
                        | Function::Max
                        | Function::Clamp
                        | Function::Pow
                        | Function::Atan2
                        | Function::Step
                        | Function::Smoothstep
                )
                .then(|| self.type_of(id));
                self.call(function.name(), &arguments, splat);
            }
            Expression::Construct { target, components } => self.call(wgsl_type(target), &components, None),
            Expression::Convert { target, value } => self.call(wgsl_type(target), &[value], None),
            Expression::Swizzle { vector, swizzle } => {
                self.operand(vector);
                emit!(self, ".{swizzle}");
            }
            Expression::Literal(_) | Expression::Attribute(_) | Expression::Builtin(_) | Expression::Varying(_) => {
                self.operand(id);
            }
        }
    }

    /// `function(arguments)`; with `splat_to`, a `Float` argument to a vector-typed call is
    /// widened, because WGSL's `min`, `max`, and `clamp` take no mixed scalar arguments.
    fn call(&mut self, function: &str, arguments: &[ExpressionId], splat_to: Option<Type>) {
        emit!(self, "{function}(");
        for (position, &argument) in arguments.iter().enumerate() {
            if position > 0 {
                emit!(self, ", ");
            }
            let splat = splat_to.filter(|ty| ty.component() == Some(self.type_of(argument)));
            match splat {
                Some(ty) => {
                    emit!(self, "{}(", wgsl_type(ty));
                    self.operand(argument);
                    emit!(self, ")");
                }
                None => self.operand(argument),
            }
        }
        emit!(self, ")");
    }

    fn operand(&mut self, id: ExpressionId) {
        let program = self.program;
        match program.expressions[id.index()] {
            Expression::Literal(literal) => self.literal(literal),
            Expression::Load(place) if place.indices().is_empty() => self.place(&place),
            Expression::Attribute(attribute) => self.attribute(attribute),
            Expression::Builtin(builtin) => emit!(self, "{}", builtin_name(builtin)),
            Expression::Varying(varying) => emit!(self, "input.v_{}", program.varyings[varying.index()].name),
            _ => emit!(self, "e{}", self.temporaries[id.index()]),
        }
    }

    /// Whether `place` is in an atomic storage buffer, which only atomic operations reach.
    fn atomic(&self, place: &Place) -> bool {
        match place.root {
            PlaceRoot::Storage(storage) => self.program.storage[storage.index()].access == Access::Atomic,
            PlaceRoot::Uniform(_) => false,
        }
    }

    /// A uniform or storage value: its block or buffer, then a field or an index per step.
    fn place(&mut self, place: &Place) {
        let program = self.program;
        // A uniform block is its members; a storage buffer is an array of its elements.
        let (frequency, mut current) = match place.root {
            PlaceRoot::Uniform(frequency) => {
                emit!(self, "u_{frequency}");
                (frequency, None)
            }
            PlaceRoot::Storage(storage) => {
                let buffer = &program.storage[storage.index()];
                emit!(self, "b_{}_{}", buffer.frequency, buffer.name);
                let array = DataType::Array {
                    element: buffer.element,
                    length: 0,
                };
                (buffer.frequency, Some(array))
            }
        };
        for step in place.steps() {
            match *step {
                Step::Field(position) => {
                    // Validation resolved every step: the first names a member, the rest struct fields.
                    let (name, ty) = match current {
                        None => program
                            .uniforms_of(frequency)
                            .nth(position as usize)
                            .map(|(_, uniform)| (uniform.name.as_str(), uniform.ty)),
                        Some(DataType::Struct(id)) => program.structs[id.index()]
                            .fields
                            .get(position as usize)
                            .map(|field| (field.name.as_str(), field.ty)),
                        Some(_) => None,
                    }
                    .unwrap_or(("", DataType::Value(Type::Bool)));
                    emit!(self, ".f_{name}");
                    current = Some(ty);
                }
                Step::Index(index) => {
                    emit!(self, "[");
                    self.operand(index);
                    emit!(self, "]");
                    if let Some(DataType::Array { element, .. }) = current {
                        current = Some(element.into());
                    }
                }
            }
        }
    }

    /// `function(resources, operands)`: a texture function of the resources, then the
    /// expression's value operands.
    fn texture_call(&mut self, function: &str, resources: &[ResourceId], expression: &Expression) {
        emit!(self, "{function}(");
        for (position, resource) in resources.iter().enumerate() {
            if position > 0 {
                emit!(self, ", ");
            }
            self.resource(*resource);
        }
        for operand in expression.operands().iter() {
            emit!(self, ", ");
            self.operand(*operand);
        }
        emit!(self, ")");
    }

    fn resource(&mut self, resource: ResourceId) {
        let declaration = &self.program.resources[resource.index()];
        let prefix = resource_prefix(declaration.kind);
        emit!(self, "{prefix}_{}_{}", declaration.frequency, declaration.name);
    }

    /// An attribute read: the input field, or a matrix rebuilt from its columns.
    fn attribute(&mut self, attribute: AttributeId) {
        let declaration = &self.program.attributes[attribute.index()];
        let Some(placed) = declaration.ty.column().and_then(|_| self.layout.attribute(attribute)) else {
            emit!(self, "input.{}", attribute_name(declaration.rate, &declaration.name));
            return;
        };
        emit!(self, "{}(", wgsl_type(declaration.ty));
        for location in placed.location..placed.location + placed.locations {
            if location > placed.location {
                emit!(self, ", ");
            }
            emit!(self, "input.m{location}_{}", declaration.name);
        }
        emit!(self, ")");
    }

    fn literal(&mut self, literal: Literal) {
        match literal {
            Literal::Bool(value) => emit!(self, "{value}"),
            // `-2147483648i` is `-(2147483648i)`, and 2147483648 is not an `i32`.
            Literal::Int(i32::MIN) => emit!(self, "i32(-2147483648)"),
            Literal::Int(value) => emit!(self, "{value}i"),
            Literal::UInt(value) => emit!(self, "{value}u"),
            Literal::Float(value) => emit!(self, "{value:?}"),
            Literal::Vec2f(_) | Literal::Vec3f(_) | Literal::Vec4f(_) => {
                emit!(self, "{}(", wgsl_type(literal.ty()));
                for (position, value) in literal.floats().iter().enumerate() {
                    if position > 0 {
                        emit!(self, ", ");
                    }
                    emit!(self, "{value:?}");
                }
                emit!(self, ")");
            }
        }
    }
}

/// The WGSL name of the struct declared at `index`: `S3_Light`.
fn struct_name(program: &Program, index: usize) -> String {
    format!("S{index}_{}", program.structs[index].name)
}

/// The WGSL struct a frequency's uniform block is.
fn block_name(frequency: Frequency) -> &'static str {
    match frequency {
        Frequency::Pass => "PassUniforms",
        Frequency::Material => "MaterialUniforms",
        Frequency::Draw => "DrawUniforms",
    }
}

/// A scalar or vector attribute's input field: `a_name` per vertex, `i_name` per instance.
fn attribute_name(rate: Rate, name: &str) -> String {
    match rate {
        Rate::Vertex => format!("a_{name}"),
        Rate::Instance => format!("i_{name}"),
    }
}

fn wgsl_type(ty: Type) -> &'static str {
    match ty {
        Type::Bool => "bool",
        Type::Int => "i32",
        Type::UInt => "u32",
        Type::Float => "f32",
        Type::Vec2f => "vec2<f32>",
        Type::Vec3f => "vec3<f32>",
        Type::Vec4f => "vec4<f32>",
        Type::Vec2i => "vec2<i32>",
        Type::Vec3i => "vec3<i32>",
        Type::Vec4i => "vec4<i32>",
        Type::Vec2u => "vec2<u32>",
        Type::Vec3u => "vec3<u32>",
        Type::Vec4u => "vec4<u32>",
        Type::Mat3f => "mat3x3<f32>",
        Type::Mat4f => "mat4x4<f32>",
    }
}

/// What a lowered stage calls a built-in value: WGSL's own name, and the vertex output's position.
fn builtin_name(builtin: Builtin) -> &'static str {
    match builtin {
        Builtin::VertexIndex => "vertex_index",
        Builtin::InstanceIndex => "instance_index",
        Builtin::Position => "input.position",
        Builtin::FrontFacing => "front_facing",
        Builtin::GlobalInvocationId => "global_invocation_id",
        Builtin::LocalInvocationId => "local_invocation_id",
        Builtin::LocalInvocationIndex => "local_invocation_index",
        Builtin::WorkgroupId => "workgroup_id",
        Builtin::NumWorkgroups => "num_workgroups",
    }
}

/// The first letter of a resource's name: `t` for a texture, `s` for a sampler.
fn resource_prefix(kind: ResourceKind) -> char {
    if kind.is_texture() { 't' } else { 's' }
}
