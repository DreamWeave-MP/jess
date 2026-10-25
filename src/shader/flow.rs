//! Control flow: the stages' bodies of statements, the locals they keep, and where each expression
//! is computed, so a value is read only where it exists and derivatives only where every fragment
//! agrees on the path.

use std::fmt;

use crate::shader::expression::{Expression, ExpressionId, Place, PlaceRoot, SourceOrigin};
use crate::shader::interface::Access;
use crate::shader::program::Program;
use crate::shader::types::Type;

arena_id!(
    /// A local a stage body declares: a [`Var`](LocalKind::Var) or a loop's index.
    LocalId,
    "local's position in declaration order"
);

/// A shader stage: the function a program runs per vertex or per fragment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Once per vertex.
    Vertex,
    /// Once per fragment.
    Fragment,
}

impl Stage {
    /// Every stage, in the order their bodies are kept.
    pub const ALL: [Stage; 2] = [Stage::Vertex, Stage::Fragment];

    /// The stage's name, as errors print it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Stage::Vertex => "vertex",
            Stage::Fragment => "fragment",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// What a local is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocalKind {
    /// A value a stage keeps and stores into: `shader.var`.
    Var,
    /// The index of one [`Statement::Loop`], counting from 0; it is never stored into.
    LoopIndex,
}

/// A local of one stage: function-scoped, zero until something stores into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    /// What it holds: a scalar or vector, or a matrix.
    pub ty: Type,
    /// The stage whose body declares it.
    pub stage: Stage,
    /// A var or a loop's index.
    pub kind: LocalKind,
    /// Where the author declared it, if known.
    pub origin: Option<SourceOrigin>,
}

/// One step of a stage body.
#[derive(Clone, Debug, PartialEq)]
pub enum Statement {
    /// Computes an expression here, if the stage reads it: its value exists from here to the end
    /// of this block. An expression no statement places is computed at the top of whichever stage
    /// reads it, and may read only what is placed nowhere.
    Emit(ExpressionId),
    /// Stores `value` into a [`LocalKind::Var`] of its type.
    Store {
        /// The var.
        local: LocalId,
        /// What it now holds.
        value: ExpressionId,
        /// Where the author stored it, if known.
        origin: Option<SourceOrigin>,
    },
    /// Writes `value` to a place in a [`ReadWrite`](crate::shader::Access::ReadWrite) storage
    /// buffer that reaches a value of its type.
    Write {
        /// Where.
        place: Place,
        /// What.
        value: ExpressionId,
        /// Where the author wrote it, if known.
        origin: Option<SourceOrigin>,
    },
    /// `accept` where the `Bool` `condition` holds, else `reject`.
    If {
        /// A `Bool`.
        condition: ExpressionId,
        /// What runs where it holds.
        accept: Vec<Statement>,
        /// What runs where it doesn't.
        reject: Vec<Statement>,
        /// Where the author wrote it, if known.
        origin: Option<SourceOrigin>,
    },
    /// `body` once for each `index` from 0 to one less than `count`, an `Int` or `UInt` computed
    /// before the first time. Nothing stores into the index, so the loop always ends.
    Loop {
        /// A [`LocalKind::LoopIndex`] of the count's type.
        index: LocalId,
        /// How many times.
        count: ExpressionId,
        /// What runs each time.
        body: Vec<Statement>,
        /// Where the author wrote it, if known.
        origin: Option<SourceOrigin>,
    },
    /// Throws the fragment away: it writes no output. Fragment stage only.
    Discard {
        /// Where the author wrote it, if known.
        origin: Option<SourceOrigin>,
    },
}

/// A use of a value by a statement or a stage's output, in a block.
#[derive(Clone, Debug)]
pub struct Use {
    pub(crate) expression: ExpressionId,
    pub(crate) stage: Stage,
    pub(crate) block: u32,
    pub(crate) origin: Option<SourceOrigin>,
}

/// Where everything in the stage bodies is: each expression's stage and block, each block's
/// parent, and every value the statements read.
#[derive(Debug, Default)]
pub struct Placements {
    /// Per expression: the stage and block that compute it, or `None` for one placed nowhere.
    pub(crate) placed: Vec<Option<(Stage, u32)>>,
    /// Per block: the block it is in. Blocks 0 and 1 are the vertex and fragment bodies.
    parents: Vec<Option<u32>>,
    /// What the statements read, where.
    pub(crate) uses: Vec<Use>,
    /// Expressions placed twice, or placed while out of the arena.
    pub(crate) misplaced: Vec<ExpressionId>,
}

impl Placements {
    /// Where each stage body's top level is.
    pub fn top(stage: Stage) -> u32 {
        stage as u32
    }

    /// Whether code in `inner` sees a value computed in `outer`: `outer` is `inner` or encloses
    /// it.
    pub fn sees(&self, outer: u32, inner: u32) -> bool {
        let mut block = Some(inner);
        while let Some(current) = block {
            if current == outer {
                return true;
            }
            block = self.parents[current as usize];
        }
        false
    }
}

/// Walks the stage bodies, numbering their blocks.
pub fn place(program: &Program) -> Placements {
    let mut placements = Placements {
        placed: vec![None; program.expressions.len()],
        parents: vec![None; Stage::ALL.len()],
        ..Placements::default()
    };
    for stage in Stage::ALL {
        place_block(
            &program.bodies[stage as usize],
            stage,
            Placements::top(stage),
            &mut placements,
        );
    }
    placements
}

fn place_block(block: &[Statement], stage: Stage, id: u32, placements: &mut Placements) {
    let use_here = |expression: ExpressionId, origin: &Option<SourceOrigin>, placements: &mut Placements| {
        placements.uses.push(Use {
            expression,
            stage,
            block: id,
            origin: origin.clone(),
        });
    };
    for statement in block {
        match statement {
            Statement::Emit(expression) => match placements.placed.get_mut(expression.index()) {
                Some(slot @ None) => *slot = Some((stage, id)),
                _ => placements.misplaced.push(*expression),
            },
            Statement::Store { value, origin, .. } => use_here(*value, origin, placements),
            Statement::Write { place, value, origin } => {
                for index in place.indices().iter() {
                    use_here(*index, origin, placements);
                }
                use_here(*value, origin, placements);
            }
            Statement::If {
                condition,
                accept,
                reject,
                origin,
            } => {
                use_here(*condition, origin, placements);
                for branch in [accept, reject] {
                    let child = new_block(placements, id);
                    place_block(branch, stage, child, placements);
                }
            }
            Statement::Loop { count, body, origin, .. } => {
                use_here(*count, origin, placements);
                let child = new_block(placements, id);
                place_block(body, stage, child, placements);
            }
            Statement::Discard { .. } => {}
        }
    }
}

fn new_block(placements: &mut Placements, parent: u32) -> u32 {
    #[allow(clippy::cast_possible_truncation)] // a block per statement, far below u32::MAX
    let id = placements.parents.len() as u32;
    placements.parents.push(Some(parent));
    id
}

/// Every statement of `body`, nested ones after the one that holds them, with whether every
/// invocation that reaches it agrees on the path there.
pub fn walk(body: &[Statement], uniform: &dyn Fn(ExpressionId) -> bool, visit: &mut dyn FnMut(&Statement, bool)) {
    walk_block(body, true, uniform, visit);
}

fn walk_block(
    block: &[Statement],
    agreed: bool,
    uniform: &dyn Fn(ExpressionId) -> bool,
    visit: &mut dyn FnMut(&Statement, bool),
) {
    for statement in block {
        visit(statement, agreed);
        match statement {
            Statement::If {
                condition,
                accept,
                reject,
                ..
            } => {
                let agreed = agreed && uniform(*condition);
                walk_block(accept, agreed, uniform, visit);
                walk_block(reject, agreed, uniform, visit);
            }
            Statement::Loop { count, body, .. } => walk_block(body, agreed && uniform(*count), uniform, visit),
            Statement::Emit(_) | Statement::Store { .. } | Statement::Write { .. } | Statement::Discard { .. } => {}
        }
    }
}

/// Which fragment expressions every fragment of a draw agrees on, given which locals they do:
/// constants, uniform values, and what is computed only from them. A varying, the fragment's
/// position or facing, and a local some fragment may store differently are not.
pub fn uniformity(program: &Program, reachable: &[bool], locals: &[bool]) -> Vec<bool> {
    let mut uniform = vec![true; program.expressions.len()];
    for (index, expression) in program.expressions.iter().enumerate() {
        if !reachable[index] {
            continue;
        }
        uniform[index] = match expression {
            Expression::Varying(_) | Expression::Attribute(_) => false,
            Expression::Builtin(builtin) => builtin.in_vertex(),
            Expression::Local(local) => locals.get(local.index()).copied().unwrap_or(false),
            // Another invocation may write a readWrite buffer between any two reads.
            Expression::Load(Place {
                root: PlaceRoot::Storage(storage),
                ..
            }) if program
                .storage
                .get(storage.index())
                .is_none_or(|buffer| buffer.access == Access::ReadWrite) =>
            {
                false
            }
            _ => expression
                .operands()
                .iter()
                .all(|operand| uniform.get(operand.index()).copied().unwrap_or(false)),
        };
    }
    uniform
}

/// Which locals every fragment agrees on: none that some fragment stores a differing value into,
/// or stores into on a path fragments disagree about, and no loop index whose count differs.
pub fn uniform_locals(program: &Program, reachable: &[bool]) -> (Vec<bool>, Vec<bool>) {
    let mut locals = vec![true; program.locals.len()];
    // Each pass can only turn locals non-uniform, so this ends within one pass per local.
    loop {
        let uniform = uniformity(program, reachable, &locals);
        let mut changed = false;
        let known = |expression: ExpressionId| uniform.get(expression.index()).copied().unwrap_or(false);
        walk(&program.bodies[Stage::Fragment as usize], &known, &mut |statement, agreed| {
            let (local, value) = match statement {
                Statement::Store { local, value, .. } => (*local, *value),
                Statement::Loop { index, count, .. } => (*index, *count),
                _ => return,
            };
            if let Some(slot) = locals.get_mut(local.index())
                && *slot
                && !(agreed && known(value))
            {
                *slot = false;
                changed = true;
            }
        });
        if !changed {
            return (uniform, locals);
        }
    }
}

/// What a derivative-taking expression is called, and what to do instead where fragments may
/// disagree on the path to it; `None` for every other expression.
pub fn derivative(expression: &Expression) -> Option<(&'static str, &'static str)> {
    let sample = "sample before the shader.when or shader.loop, or sample at a level with sampleLevel";
    match expression {
        Expression::Sample { level, .. } if level.needs_derivatives() => Some((level.operation(), sample)),
        Expression::SampleCompare { level_zero: false, .. } => Some((
            "sampleCompare",
            "compare before the shader.when or shader.loop, or compare in level 0 with sampleCompareLevel",
        )),
        Expression::Call { function, .. } if function.needs_derivatives() => {
            Some((function.name(), "take it before the shader.when or shader.loop"))
        }
        _ => None,
    }
}
