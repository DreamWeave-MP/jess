//! A shader program: its declarations, its expression arena, and what each stage outputs.

use crate::shader::expression::{Expression, ExpressionId, SourceOrigin, VaryingId};
use crate::shader::flow::{Local, LocalId, Stage, Statement};
use crate::shader::interface::{
    Attribute, AttributeId, Frequency, Resource, ResourceId, StorageBuffer, StorageId, StructDecl, StructId, Uniform,
    UniformId,
};
use crate::shader::layout::ProgramLayout;
use crate::shader::types::Type;

/// A named, typed varying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    /// The name the author gave it. The lowering derives every backend name from it.
    pub name: String,
    /// Its type.
    pub ty: Type,
    /// Where the author declared it, if known.
    pub origin: Option<SourceOrigin>,
}

/// A vertex stage and a fragment stage over one expression arena.
///
/// The vertex stage reads the declared attributes and writes the clip position and any varyings;
/// the fragment stage reads those varyings and writes color outputs. Uniform members, textures, and
/// samplers are bound per [`Frequency`], and attributes come from vertex buffers, all where
/// [`layout`](crate::shader::layout) says: the author never picks a bind group, binding, or
/// location.
///
/// Expressions are appended and never removed, so an [`ExpressionId`] stays valid for the
/// program's lifetime; [`fold_constants`](Program::fold_constants) rewrites nodes in place.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub(crate) expressions: Vec<Expression>,
    /// Sparse and sorted by id: native construction rarely has origins, and only errors read them.
    pub(crate) origins: Vec<(ExpressionId, SourceOrigin)>,
    pub(crate) structs: Vec<StructDecl>,
    pub(crate) attributes: Vec<Attribute>,
    pub(crate) uniforms: Vec<Uniform>,
    pub(crate) resources: Vec<Resource>,
    pub(crate) storage: Vec<StorageBuffer>,
    pub(crate) varyings: Vec<Declaration>,
    pub(crate) position: Option<ExpressionId>,
    pub(crate) varying_writes: Vec<(VaryingId, ExpressionId)>,
    pub(crate) fragment_outputs: Vec<ExpressionId>,
    pub(crate) locals: Vec<Local>,
    /// Each stage's statements, by [`Stage`] order.
    pub(crate) bodies: [Vec<Statement>; Stage::ALL.len()],
    /// A compute program's workgroup size; `None` for a vertex and fragment program.
    pub(crate) workgroup_size: Option<[u32; 3]>,
}

impl Program {
    /// An empty program of a vertex and a fragment stage.
    #[must_use]
    pub fn new() -> Program {
        Program::default()
    }

    /// An empty compute program: one stage, run once per invocation, in workgroups of
    /// `workgroup_size` invocations.
    #[must_use]
    pub fn compute(workgroup_size: [u32; 3]) -> Program {
        Program {
            workgroup_size: Some(workgroup_size),
            ..Program::default()
        }
    }

    /// A compute program's workgroup size; `None` for a vertex and fragment program.
    #[must_use]
    pub fn workgroup_size(&self) -> Option<[u32; 3]> {
        self.workgroup_size
    }

    /// Declares a struct. Its fields may only name structs declared before it.
    pub fn structure(&mut self, declaration: StructDecl) -> StructId {
        StructId::at(push(&mut self.structs, declaration))
    }

    /// Declares a vertex attribute.
    pub fn attribute(&mut self, attribute: Attribute) -> AttributeId {
        AttributeId::at(push(&mut self.attributes, attribute))
    }

    /// Declares a member of a frequency's uniform block.
    pub fn uniform(&mut self, uniform: Uniform) -> UniformId {
        UniformId::at(push(&mut self.uniforms, uniform))
    }

    /// Declares a texture or sampler a frequency binds.
    pub fn resource(&mut self, resource: Resource) -> ResourceId {
        ResourceId::at(push(&mut self.resources, resource))
    }

    /// Declares a value the vertex stage passes to the fragment stage: `Int`, `UInt` (both
    /// passed flat, not interpolated), `Float`, or a vector.
    pub fn varying(&mut self, name: &str, ty: Type) -> VaryingId {
        VaryingId::at(push_declaration(&mut self.varyings, name, ty, None))
    }

    /// [`varying`](Program::varying), recording where it was declared.
    pub fn varying_at(&mut self, name: &str, ty: Type, origin: SourceOrigin) -> VaryingId {
        VaryingId::at(push_declaration(&mut self.varyings, name, ty, Some(origin)))
    }

    /// Appends an expression.
    ///
    /// # Panics
    ///
    /// If the arena already holds `u32::MAX` expressions.
    pub fn push(&mut self, expression: Expression) -> ExpressionId {
        let id = ExpressionId(u32::try_from(self.expressions.len()).expect("more than u32::MAX shader expressions"));
        self.expressions.push(expression);
        id
    }

    /// [`push`](Program::push), recording where the expression was written.
    ///
    /// # Panics
    ///
    /// If the arena already holds `u32::MAX` expressions.
    pub fn push_at(&mut self, expression: Expression, origin: SourceOrigin) -> ExpressionId {
        let id = self.push(expression);
        self.origins.push((id, origin));
        id
    }

    /// Sets the vertex stage's clip-space position, a `Vec4f`.
    pub fn set_position(&mut self, expression: ExpressionId) {
        self.position = Some(expression);
    }

    /// Makes the vertex stage write `expression` to `varying`.
    pub fn write_varying(&mut self, varying: VaryingId, expression: ExpressionId) {
        self.varying_writes.push((varying, expression));
    }

    /// Adds a fragment output, a `Vec4f`, at the next color location; returns the location.
    ///
    /// # Panics
    ///
    /// If the program already has `u32::MAX` fragment outputs.
    pub fn push_fragment_output(&mut self, expression: ExpressionId) -> u32 {
        let location = u32::try_from(self.fragment_outputs.len()).expect("more than u32::MAX fragment outputs");
        self.fragment_outputs.push(expression);
        location
    }

    /// Declares a storage buffer.
    pub fn storage_buffer(&mut self, buffer: StorageBuffer) -> StorageId {
        StorageId::at(push(&mut self.storage, buffer))
    }

    /// The declared storage buffers, in order.
    #[must_use]
    pub fn storage_buffers(&self) -> &[StorageBuffer] {
        &self.storage
    }

    /// Declares a local of a stage body, for a [`Statement::Store`] or a [`Statement::Loop`].
    pub fn local(&mut self, local: Local) -> LocalId {
        LocalId::at(push(&mut self.locals, local))
    }

    /// `stage`'s statements, run before it writes its outputs.
    pub fn body_mut(&mut self, stage: Stage) -> &mut Vec<Statement> {
        &mut self.bodies[stage as usize]
    }

    /// `stage`'s statements.
    #[must_use]
    pub fn body(&self, stage: Stage) -> &[Statement] {
        &self.bodies[stage as usize]
    }

    /// The declared locals, in order.
    #[must_use]
    pub fn locals(&self) -> &[Local] {
        &self.locals
    }

    /// The declared structs, in order.
    #[must_use]
    pub fn structs(&self) -> &[StructDecl] {
        &self.structs
    }

    /// The declared vertex attributes, of both rates, in order.
    #[must_use]
    pub fn attributes(&self) -> &[Attribute] {
        &self.attributes
    }

    /// The declared uniform members, of every frequency, in order.
    #[must_use]
    pub fn uniforms(&self) -> &[Uniform] {
        &self.uniforms
    }

    /// The uniform members `frequency` binds, with their ids, in order.
    pub fn uniforms_of(&self, frequency: Frequency) -> impl Iterator<Item = (UniformId, &Uniform)> {
        self.uniforms
            .iter()
            .enumerate()
            .filter(move |(_, uniform)| uniform.frequency == frequency)
            .map(|(index, uniform)| (UniformId::at(index), uniform))
    }

    /// The declared textures and samplers, of every frequency, in order.
    #[must_use]
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    /// Where everything the program declares lives: see [`layout`](crate::shader::layout).
    #[must_use]
    pub fn layout(&self) -> ProgramLayout {
        ProgramLayout::of(self)
    }

    /// Every expression, in arena order.
    #[must_use]
    pub fn expressions(&self) -> &[Expression] {
        &self.expressions
    }

    /// The expression `id` names, if it is in this program.
    #[must_use]
    pub fn expression(&self, id: ExpressionId) -> Option<&Expression> {
        self.expressions.get(id.index())
    }

    /// Where the expression `id` was written, if recorded.
    #[must_use]
    pub fn origin(&self, id: ExpressionId) -> Option<&SourceOrigin> {
        let position = self.origins.binary_search_by_key(&id, |(origin_id, _)| *origin_id).ok()?;
        Some(&self.origins[position].1)
    }

    /// The declared varyings, in order.
    #[must_use]
    pub fn varyings(&self) -> &[Declaration] {
        &self.varyings
    }

    /// The vertex stage's clip-space position, if set.
    #[must_use]
    pub fn position(&self) -> Option<ExpressionId> {
        self.position
    }

    /// What the vertex stage writes to each varying, in the order they were written.
    #[must_use]
    pub fn varying_writes(&self) -> &[(VaryingId, ExpressionId)] {
        &self.varying_writes
    }

    /// The fragment outputs, by color location.
    #[must_use]
    pub fn fragment_outputs(&self) -> &[ExpressionId] {
        &self.fragment_outputs
    }
}

fn push_declaration(declarations: &mut Vec<Declaration>, name: &str, ty: Type, origin: Option<SourceOrigin>) -> usize {
    let declaration = Declaration {
        name: name.to_owned(),
        ty,
        origin,
    };
    push(declarations, declaration)
}

fn push<T>(declarations: &mut Vec<T>, declaration: T) -> usize {
    let index = declarations.len();
    assert!(u32::try_from(index).is_ok(), "more than u32::MAX shader declarations");
    declarations.push(declaration);
    index
}
