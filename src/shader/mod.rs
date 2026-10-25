//! A typed shader IR that lowers to WGSL.
//!
//! A [`Program`] is an arena of [`Expression`]s plus a vertex stage (clip position and varyings)
//! and a fragment stage (color outputs). Expressions name parameters, vertex inputs, varyings, and
//! [`Semantic`] inputs by what they mean; [`wgsl::lower`] picks every binding and location.
//! [`Program::validate`] reports mistakes in Jess terms, with the author's [`SourceOrigin`] when
//! one was recorded, before any backend sees the program. [`Program::fold_constants`] folds
//! constant arithmetic and trivial identities, and nothing more.
//!
//! WGSL is backend output, not the interface: nothing here takes WGSL or exposes a GPU API.
//!
//! ```
//! use jess::shader::{Expression, Literal, Program, Semantic, Type, wgsl};
//!
//! let mut program = Program::new();
//! let position = program.vertex_input("position", Type::Vec3f);
//! let color = program.parameter("color", Type::Vec4f);
//!
//! let position = program.push(Expression::VertexInput(position));
//! let one = program.push(Expression::Literal(Literal::Float(1.0)));
//! let local = program.push(Expression::construct(Type::Vec4f, &[position, one]).unwrap());
//! let projection = program.push(Expression::Semantic(Semantic::Projection));
//! let clip = program.push(Expression::Multiply(projection, local));
//! program.set_position(clip);
//!
//! let color = program.push(Expression::Parameter(color));
//! program.push_fragment_output(color);
//!
//! program.fold_constants()?;
//! let module = wgsl::lower(&program)?;
//! assert!(module.source.contains("let e1 = semantics.projection * e0;"));
//! # Ok::<(), Vec<jess::shader::ValidationError>>(())
//! ```
//!
//! A frontend that builds a program one call at a time uses [`ProgramBuilder`] instead. Its
//! [`Site`] types every operation as it is pushed, by the rules validation applies, and reports
//! each mistake with the origin the site was given; its [`Value`]s carry their type and cannot
//! cross into another builder. The same program:
//!
//! ```
//! use jess::shader::{ProgramBuilder, Semantic, Type, wgsl};
//!
//! let mut builder = ProgramBuilder::new();
//! let mut site = builder.site();
//! let position = site.vertex_input("position", Type::Vec3f)?;
//! let color = site.parameter("color", Type::Vec4f)?;
//!
//! let one = site.float(1.0)?;
//! let local = site.construct(Type::Vec4f, &[position, one])?;
//! let projection = site.semantic(Semantic::Projection);
//! let clip = site.multiply(projection, local)?;
//! assert_eq!(clip.ty(), Type::Vec4f);
//! site.set_position(clip)?;
//! site.output(color)?;
//!
//! let program = builder.finish().map_err(|mut errors| errors.remove(0))?;
//! let module = wgsl::lower(&program).map_err(|mut errors| errors.remove(0))?;
//! assert!(module.source.contains("let e1 = semantics.projection * e0;"));
//! # Ok::<(), jess::shader::ValidationError>(())
//! ```

#![warn(missing_docs)]

/// An index into one of a program's arenas or declaration lists, only ever made by the program.
macro_rules! arena_id {
    ($(#[$attribute:meta])* $name:ident, $position:literal) => {
        $(#[$attribute])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub(crate) u32);

        impl $name {
            #[doc = concat!("The ", $position, ".")]
            #[must_use]
            pub fn index(self) -> usize {
                self.0 as usize
            }

            /// The id at `index`, which the program's push already bounded to `u32`.
            #[allow(clippy::cast_possible_truncation)]
            pub(crate) fn at(index: usize) -> $name {
                $name(index as u32)
            }
        }
    };
}

mod build;
mod expression;
mod fold;
mod interface;
pub mod layout;
mod program;
mod types;
mod validate;
pub mod wgsl;

pub use build::{ProgramBuilder, Site, Value};
pub use expression::{
    Component, Expression, ExpressionId, Literal, Operands, ParameterId, Semantic, SourceOrigin, Swizzle, VaryingId,
    VertexInputId,
};
pub use interface::{
    Attribute, AttributeId, DataType, Element, Field, Frequency, Rate, Resource, ResourceId, ResourceKind, StructDecl,
    StructId, Uniform, UniformId,
};
pub use program::{Declaration, Program};
pub use types::{Scalar, Type};
pub use validate::{DeclarationKind, MAX_FRAGMENT_OUTPUTS, ValidationError, ValidationErrorKind};
