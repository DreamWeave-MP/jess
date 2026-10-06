//! A shader program: its declarations, its expression arena, and what each stage outputs.

use crate::shader::expression::{Expression, ExpressionId, ParameterId, SourceOrigin, VaryingId, VertexInputId};
use crate::shader::types::Type;

/// A named, typed parameter, vertex input, or varying.
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
/// The vertex stage writes the clip position and any varyings; the fragment stage reads those
/// varyings and writes color outputs. Parameters, textures, samplers, and semantics are resources
/// the lowering binds: the author never picks a bind group or binding.
///
/// Expressions are appended and never removed, so an [`ExpressionId`] stays valid for the
/// program's lifetime; [`fold_constants`](Program::fold_constants) rewrites nodes in place.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub(crate) expressions: Vec<Expression>,
    /// Sparse and sorted by id: native construction rarely has origins, and only errors read them.
    pub(crate) origins: Vec<(ExpressionId, SourceOrigin)>,
    pub(crate) parameters: Vec<Declaration>,
    pub(crate) vertex_inputs: Vec<Declaration>,
    pub(crate) varyings: Vec<Declaration>,
    pub(crate) position: Option<ExpressionId>,
    pub(crate) varying_writes: Vec<(VaryingId, ExpressionId)>,
    pub(crate) fragment_outputs: Vec<ExpressionId>,
}

impl Program {
    /// An empty program.
    #[must_use]
    pub fn new() -> Program {
        Program::default()
    }

    /// Declares a parameter: a value (`Int`, `UInt`, `Float`, a vector, or a matrix), a
    /// `Texture2D`, or a `Sampler`.
    pub fn parameter(&mut self, name: &str, ty: Type) -> ParameterId {
        ParameterId::at(push_declaration(&mut self.parameters, name, ty, None))
    }

    /// [`parameter`](Program::parameter), recording where it was declared.
    pub fn parameter_at(&mut self, name: &str, ty: Type, origin: SourceOrigin) -> ParameterId {
        ParameterId::at(push_declaration(&mut self.parameters, name, ty, Some(origin)))
    }

    /// Declares a per-vertex attribute: `Int`, `UInt`, `Float`, or a vector.
    pub fn vertex_input(&mut self, name: &str, ty: Type) -> VertexInputId {
        VertexInputId::at(push_declaration(&mut self.vertex_inputs, name, ty, None))
    }

    /// [`vertex_input`](Program::vertex_input), recording where it was declared.
    pub fn vertex_input_at(&mut self, name: &str, ty: Type, origin: SourceOrigin) -> VertexInputId {
        VertexInputId::at(push_declaration(&mut self.vertex_inputs, name, ty, Some(origin)))
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

    /// The declared parameters, in order.
    #[must_use]
    pub fn parameters(&self) -> &[Declaration] {
        &self.parameters
    }

    /// The declared vertex inputs, in order.
    #[must_use]
    pub fn vertex_inputs(&self) -> &[Declaration] {
        &self.vertex_inputs
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
    let index = declarations.len();
    assert!(u32::try_from(index).is_ok(), "more than u32::MAX shader declarations");
    declarations.push(Declaration {
        name: name.to_owned(),
        ty,
        origin,
    });
    index
}
