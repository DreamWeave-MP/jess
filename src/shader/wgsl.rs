//! Lowering a validated [`Program`] to one WGSL module.
//!
//! Every backend name comes from here and follows one scheme: `p_` parameters (fields of the
//! `parameters` uniform), `t_`/`s_` textures and samplers, `a_` vertex inputs, `v_` varyings,
//! `colorN` fragment outputs, `semantics.snake_case` semantics, and `eN` temporaries numbered
//! from 0 in each entry point, in arena order. Every non-leaf expression a stage reads becomes one
//! `let`; leaves (literals, parameters, inputs, semantics) are written in place.
//!
//! Bindings are Jess's: semantics are one uniform block at `@group(0) @binding(0)`; value
//! parameters are one uniform block at `@group(1) @binding(0)`, followed in `@group(1)` by every
//! texture and sampler parameter in declaration order.

use std::fmt::Write as _;

use crate::shader::expression::{Expression, ExpressionId, Literal, ParameterId, Semantic, VertexInputId};
use crate::shader::program::Program;
use crate::shader::types::Type;
use crate::shader::validate::{Analysis, FRAGMENT, VERTEX, ValidationError, analyze};

/// The vertex entry point's name.
pub const VERTEX_ENTRY_POINT: &str = "vertex_main";
/// The fragment entry point's name.
pub const FRAGMENT_ENTRY_POINT: &str = "fragment_main";
/// The bind group holding the semantics uniform block.
pub const SEMANTIC_GROUP: u32 = 0;
/// The bind group holding the parameters uniform block, textures, and samplers.
pub const PARAMETER_GROUP: u32 = 1;

/// One member of a uniform block, where the renderer writes its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniformMember<Source> {
    /// What the member holds.
    pub source: Source,
    /// Its type.
    pub ty: Type,
    /// Its byte offset in the block.
    pub offset: u32,
    /// Its size in bytes. A `Mat3f` is three columns of 16 bytes, the last 4 of each padding.
    pub size: u32,
}

/// A uniform buffer the lowering declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UniformBlock<Source> {
    /// Its bind group.
    pub group: u32,
    /// Its binding in the group.
    pub binding: u32,
    /// The buffer size it needs, in bytes.
    pub size: u32,
    /// Its members, in offset order.
    pub members: Vec<UniformMember<Source>>,
}

/// A texture or sampler parameter's binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBinding {
    /// Its bind group.
    pub group: u32,
    /// Its binding in the group.
    pub binding: u32,
    /// The parameter.
    pub parameter: ParameterId,
    /// `Texture2D` or `Sampler`.
    pub ty: Type,
}

/// A vertex input's attribute location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VertexAttribute {
    /// The `@location`.
    pub location: u32,
    /// The vertex input.
    pub input: VertexInputId,
    /// Its type.
    pub ty: Type,
}

/// A lowered program: the WGSL, and every binding and location the renderer must match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgslModule {
    /// The WGSL source, with entry points [`VERTEX_ENTRY_POINT`] and [`FRAGMENT_ENTRY_POINT`].
    pub source: String,
    /// The semantics the program reads, if any, in [`Semantic`] order.
    pub semantics: Option<UniformBlock<Semantic>>,
    /// The value parameters, if any, in declaration order.
    pub parameters: Option<UniformBlock<ParameterId>>,
    /// Every texture and sampler parameter, in declaration order.
    pub resources: Vec<ResourceBinding>,
    /// Every vertex input, at its declaration index.
    pub vertex_attributes: Vec<VertexAttribute>,
    /// How many color targets the fragment stage writes.
    pub fragment_outputs: u32,
}

/// A value type's uniform buffer `(alignment, size)` in bytes, by WGSL's layout rules; `None` for
/// `Bool` and resources, which cannot live in a uniform buffer.
#[must_use]
pub fn uniform_layout(ty: Type) -> Option<(u32, u32)> {
    match ty {
        Type::Int | Type::UInt | Type::Float => Some((4, 4)),
        Type::Vec2f | Type::Vec2i | Type::Vec2u => Some((8, 8)),
        Type::Vec3f | Type::Vec3i | Type::Vec3u => Some((16, 12)),
        Type::Vec4f | Type::Vec4i | Type::Vec4u => Some((16, 16)),
        Type::Mat3f => Some((16, 48)),
        Type::Mat4f => Some((16, 64)),
        Type::Bool | Type::Texture2D | Type::Sampler => None,
    }
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
    // Validation caps the outputs at `MAX_FRAGMENT_OUTPUTS`.
    #[allow(clippy::cast_possible_truncation)]
    let output_count = program.fragment_outputs.len() as u32;
    let mut lowering = Lowering {
        program,
        analysis: &analysis,
        source: String::with_capacity(1024 + program.expressions.len() * 48),
        temporaries: vec![0; program.expressions.len()],
    };

    let semantics = lowering.semantics_block();
    let parameters = lowering.parameters_block();
    let resources = lowering.resources(parameters.is_some());
    let vertex_attributes = lowering.vertex_input_struct();
    lowering.vertex_output_struct();
    lowering.fragment_output_struct();
    lowering.vertex_entry_point();
    lowering.fragment_entry_point();

    Ok(WgslModule {
        source: lowering.source,
        semantics,
        parameters,
        resources,
        vertex_attributes,
        fragment_outputs: output_count,
    })
}

/// Writes to the output. Writing to a `String` cannot fail.
macro_rules! emit {
    ($lowering:expr, $($format:tt)*) => {
        $lowering.source.write_fmt(format_args!($($format)*)).unwrap_or(())
    };
}

struct Lowering<'a> {
    program: &'a Program,
    analysis: &'a Analysis,
    source: String,
    /// Each non-leaf expression's temporary number in the entry point being written.
    temporaries: Vec<u32>,
}

impl Lowering<'_> {
    fn semantics_block(&mut self) -> Option<UniformBlock<Semantic>> {
        let mut used = [false; Semantic::ALL.len()];
        for (index, expression) in self.program.expressions.iter().enumerate() {
            if let Expression::Semantic(semantic) = expression
                && self.analysis.stages[index] != 0
            {
                used[*semantic as usize] = true;
            }
        }
        let semantics = Semantic::ALL.into_iter().filter(|semantic| used[*semantic as usize]);
        let block = uniform_block(SEMANTIC_GROUP, 0, semantics.map(|semantic| (semantic, semantic.ty())))?;

        emit!(self, "struct Semantics {{\n");
        for member in &block.members {
            emit!(self, "    {}: {},\n", semantic_field(member.source), wgsl_type(member.ty));
        }
        emit!(
            self,
            "}}\n\n@group({}) @binding({}) var<uniform> semantics: Semantics;\n\n",
            block.group,
            block.binding
        );
        Some(block)
    }

    fn parameters_block(&mut self) -> Option<UniformBlock<ParameterId>> {
        let values = self
            .program
            .parameters
            .iter()
            .enumerate()
            .filter(|(_, declaration)| !declaration.ty.is_resource());
        let block = uniform_block(
            PARAMETER_GROUP,
            0,
            values.map(|(index, declaration)| (ParameterId::at(index), declaration.ty)),
        )?;

        emit!(self, "struct Parameters {{\n");
        for member in &block.members {
            let name = &self.program.parameters[member.source.index()].name;
            emit!(self, "    p_{name}: {},\n", wgsl_type(member.ty));
        }
        emit!(
            self,
            "}}\n\n@group({}) @binding({}) var<uniform> parameters: Parameters;\n",
            block.group,
            block.binding
        );
        Some(block)
    }

    fn resources(&mut self, has_parameters: bool) -> Vec<ResourceBinding> {
        let mut binding = u32::from(has_parameters);
        let mut resources = Vec::new();
        for (index, declaration) in self.program.parameters.iter().enumerate() {
            if !declaration.ty.is_resource() {
                continue;
            }
            let prefix = if declaration.ty == Type::Texture2D { 't' } else { 's' };
            emit!(
                self,
                "@group({PARAMETER_GROUP}) @binding({binding}) var {prefix}_{}: {};\n",
                declaration.name,
                wgsl_type(declaration.ty)
            );
            resources.push(ResourceBinding {
                group: PARAMETER_GROUP,
                binding,
                parameter: ParameterId::at(index),
                ty: declaration.ty,
            });
            binding += 1;
        }
        if has_parameters || !resources.is_empty() {
            emit!(self, "\n");
        }
        resources
    }

    fn vertex_input_struct(&mut self) -> Vec<VertexAttribute> {
        if self.program.vertex_inputs.is_empty() {
            return Vec::new();
        }
        emit!(self, "struct VertexInput {{\n");
        let mut attributes = Vec::with_capacity(self.program.vertex_inputs.len());
        for (location, declaration) in (0..).zip(&self.program.vertex_inputs) {
            emit!(
                self,
                "    @location({location}) a_{}: {},\n",
                declaration.name,
                wgsl_type(declaration.ty)
            );
            attributes.push(VertexAttribute {
                location,
                input: VertexInputId(location),
                ty: declaration.ty,
            });
        }
        emit!(self, "}}\n\n");
        attributes
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
        for location in 0..self.program.fragment_outputs.len() {
            emit!(self, "    @location({location}) color{location}: vec4<f32>,\n");
        }
        emit!(self, "}}\n\n");
    }

    fn vertex_entry_point(&mut self) {
        let input = if self.program.vertex_inputs.is_empty() {
            ""
        } else {
            "input: VertexInput"
        };
        emit!(self, "@vertex\nfn {VERTEX_ENTRY_POINT}({input}) -> VertexOutput {{\n");
        self.temporaries_for(VERTEX);
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
        emit!(self, "@fragment\nfn {FRAGMENT_ENTRY_POINT}(input: VertexOutput){result} {{\n");
        self.temporaries_for(FRAGMENT);
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

    /// One `let eN` per non-leaf expression `stage` reads, in arena order, which is dependency
    /// order.
    fn temporaries_for(&mut self, stage: u8) {
        let mut next = 0;
        for index in 0..self.program.expressions.len() {
            let expression = &self.program.expressions[index];
            if self.analysis.stages[index] & stage == 0 || expression.is_leaf() {
                continue;
            }
            self.temporaries[index] = next;
            emit!(self, "    let e{next} = ");
            self.operation(ExpressionId::at(index));
            emit!(self, ";\n");
            next += 1;
        }
    }

    fn type_of(&self, id: ExpressionId) -> Type {
        // Validation typed every expression a stage reads.
        self.analysis.types[id.index()].unwrap_or(Type::Bool)
    }

    fn operation(&mut self, id: ExpressionId) {
        let expression = self.program.expressions[id.index()];
        match expression {
            Expression::Sample {
                texture,
                sampler,
                coordinates,
            } => {
                self.call("textureSample", &[texture, sampler, coordinates], None);
            }
            Expression::Add(left, right)
            | Expression::Subtract(left, right)
            | Expression::Multiply(left, right)
            | Expression::Divide(left, right) => {
                self.operand(left);
                emit!(self, " {} ", expression.operation());
                self.operand(right);
            }
            Expression::Min(first, second) => self.call("min", &[first, second], Some(self.type_of(id))),
            Expression::Max(first, second) => self.call("max", &[first, second], Some(self.type_of(id))),
            Expression::Clamp(value, low, high) => self.call("clamp", &[value, low, high], Some(self.type_of(id))),
            Expression::Dot(left, right) => self.call("dot", &[left, right], None),
            Expression::Cross(left, right) => self.call("cross", &[left, right], None),
            Expression::Normalize(value) => self.call("normalize", &[value], None),
            Expression::Length(value) => self.call("length", &[value], None),
            Expression::Floor(value) => self.call("floor", &[value], None),
            Expression::Mix(start, end, amount) => self.call("mix", &[start, end, amount], None),
            Expression::Construct { target, components } => self.call(wgsl_type(target), &components, None),
            Expression::Swizzle { vector, swizzle } => {
                self.operand(vector);
                emit!(self, ".{swizzle}");
            }
            Expression::Literal(_)
            | Expression::Parameter(_)
            | Expression::VertexInput(_)
            | Expression::Varying(_)
            | Expression::Semantic(_) => self.operand(id),
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
            Expression::Parameter(parameter) => {
                let declaration = &program.parameters[parameter.index()];
                match declaration.ty {
                    Type::Texture2D => emit!(self, "t_{}", declaration.name),
                    Type::Sampler => emit!(self, "s_{}", declaration.name),
                    _ => emit!(self, "parameters.p_{}", declaration.name),
                }
            }
            Expression::VertexInput(input) => emit!(self, "input.a_{}", program.vertex_inputs[input.index()].name),
            Expression::Varying(varying) => emit!(self, "input.v_{}", program.varyings[varying.index()].name),
            Expression::Semantic(semantic) => emit!(self, "semantics.{}", semantic_field(semantic)),
            _ => emit!(self, "e{}", self.temporaries[id.index()]),
        }
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

fn uniform_block<Source>(
    group: u32,
    binding: u32,
    members: impl Iterator<Item = (Source, Type)>,
) -> Option<UniformBlock<Source>> {
    let mut block = UniformBlock {
        group,
        binding,
        size: 0,
        members: Vec::new(),
    };
    let mut end = 0u32;
    let mut block_alignment = 1;
    for (source, ty) in members {
        // Validation rejects `Bool` parameters, the only value type without a layout.
        let (alignment, size) = uniform_layout(ty)?;
        let offset = end.next_multiple_of(alignment);
        block.members.push(UniformMember {
            source,
            ty,
            offset,
            size,
        });
        end = offset + size;
        block_alignment = block_alignment.max(alignment);
    }
    if block.members.is_empty() {
        return None;
    }
    block.size = end.next_multiple_of(block_alignment);
    Some(block)
}

fn semantic_field(semantic: Semantic) -> &'static str {
    match semantic {
        Semantic::Model => "model",
        Semantic::View => "view",
        Semantic::Projection => "projection",
        Semantic::CameraPosition => "camera_position",
        Semantic::ViewportSize => "viewport_size",
        Semantic::Time => "time",
        Semantic::SceneColor => "scene_color",
        Semantic::SceneDepth => "scene_depth",
        Semantic::PreviousPass => "previous_pass",
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
        Type::Texture2D => "texture_2d<f32>",
        Type::Sampler => "sampler",
    }
}
