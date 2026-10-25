//! A typed shader IR that lowers to WGSL.
//!
//! A [`Program`] is an arena of [`Expression`]s over a declared interface, plus a vertex stage
//! (clip position and varyings) and a fragment stage (color outputs). The interface is everything
//! the program reads, named by what it is and how often it changes: vertex attributes per vertex
//! or per instance, and per [`Frequency`] the uniform members, textures, and samplers each pass,
//! material, or draw binds. Nothing is built in; [`layout`] decides where all of it lives, and
//! [`wgsl::lower`] writes that out. [`Program::validate`] reports mistakes in Jess terms, with the
//! author's [`SourceOrigin`] when one was recorded, before any backend sees the program.
//! [`Program::fold_constants`] folds constant arithmetic and trivial identities, and nothing more.
//!
//! WGSL is backend output, not the interface: nothing here takes WGSL or exposes a GPU API.
//!
//! ```
//! use jess::shader::{
//!     Attribute, BinaryOperator, DataType, Expression, Frequency, Literal, Place, PlaceRoot, Program, Rate, Step,
//!     Type, Uniform, wgsl,
//! };
//!
//! let mut program = Program::new();
//! let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec3f));
//! program.uniform(Uniform::new(Frequency::Pass, "projection", DataType::Value(Type::Mat4f)));
//! program.uniform(Uniform::new(Frequency::Material, "color", DataType::Value(Type::Vec4f)));
//! let member = |frequency| Place::new(PlaceRoot::Uniform(frequency), &[Step::Field(0)]).unwrap();
//!
//! let position = program.push(Expression::Attribute(position));
//! let one = program.push(Expression::Literal(Literal::Float(1.0)));
//! let local = program.push(Expression::construct(Type::Vec4f, &[position, one]).unwrap());
//! let projection = program.push(Expression::Load(member(Frequency::Pass)));
//! let clip = program.push(Expression::Binary(BinaryOperator::Multiply, projection, local));
//! program.set_position(clip);
//!
//! let color = program.push(Expression::Load(member(Frequency::Material)));
//! program.push_fragment_output(color);
//!
//! program.fold_constants()?;
//! let module = wgsl::lower(&program)?;
//! assert!(module.source.contains("let e1 = u_pass.f_projection * e0;"));
//! # Ok::<(), Vec<jess::shader::ValidationError>>(())
//! ```
//!
//! A frontend that builds a program one call at a time uses [`ProgramBuilder`] instead. Its
//! [`Site`] types every operation as it is pushed, by the rules validation applies, and reports
//! each mistake with the origin the site was given; its [`Value`]s carry their type and cannot
//! cross into another builder. The same program:
//!
//! ```
//! use jess::shader::{DataType, Frequency, Place, PlaceRoot, ProgramBuilder, Rate, Step, Type, wgsl};
//!
//! let mut builder = ProgramBuilder::new();
//! let mut site = builder.site();
//! let position = site.attribute(Rate::Vertex, "position", Type::Vec3f)?;
//! let position = site.read_attribute(position);
//! site.uniform(Frequency::Pass, "projection", DataType::Value(Type::Mat4f))?;
//! site.uniform(Frequency::Material, "color", DataType::Value(Type::Vec4f))?;
//! let member = |frequency| Place::new(PlaceRoot::Uniform(frequency), &[Step::Field(0)]).unwrap();
//!
//! let one = site.float(1.0)?;
//! let local = site.construct(Type::Vec4f, &[position, one])?;
//! let projection = site.load(member(Frequency::Pass), &[])?;
//! let clip = site.multiply(projection, local)?;
//! assert_eq!(clip.ty(), Type::Vec4f);
//! site.set_position(clip)?;
//! let color = site.load(member(Frequency::Material), &[])?;
//! site.output(color)?;
//!
//! let program = builder.finish().map_err(|mut errors| errors.remove(0))?;
//! let module = wgsl::lower(&program).map_err(|mut errors| errors.remove(0))?;
//! assert!(module.source.contains("let e1 = u_pass.f_projection * e0;"));
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
mod flow;
mod fold;
mod interface;
pub mod layout;
mod program;
mod types;
mod validate;
pub mod wgsl;

pub use build::{ProgramBuilder, Site, Value};
pub use expression::{
    BinaryOperator, Builtin, Component, Expression, ExpressionId, Function, Literal, MAX_PLACE_INDICES, MAX_PLACE_STEPS,
    Operands, Place, PlaceRoot, SampleLevel, SourceOrigin, Step, Swizzle, TextureQuery, UnaryOperator, VaryingId,
};
pub use flow::{Local, LocalId, LocalKind, Stage, Statement};
pub use interface::{
    Access, Attribute, AttributeId, DataType, Element, Field, Frequency, Rate, Resource, ResourceId, ResourceKind,
    StorageBuffer, StorageId, StructDecl, StructId, Uniform, UniformId,
};
pub use program::{Declaration, Program};
pub use types::{Scalar, Type};
pub use validate::{
    DeclarationKind, MAX_ATTRIBUTE_LOCATIONS, MAX_FRAGMENT_OUTPUTS, PlaceProblem, ValidationError, ValidationErrorKind,
};
