//! AST nodes grouped by language concept; parser and compiler import through this module.

mod classes;
mod expressions;
mod functions;
mod interfaces;
mod programs;
mod statements;
mod traits;

pub use classes::{AttributeDecl, ClassConstantDecl, ClassDecl, PropertyDecl, PropertySetHook, TraitAdaptation};
pub use expressions::{ArrayEntry, CastType, Expr, FirstClassCallableTarget, IncrementTarget, IssetPropertySegment, MatchArm};
pub use functions::{FunctionDecl, Visibility};
pub use interfaces::InterfaceDecl;
pub use programs::{ConstantDecl, Program};
pub use statements::{ArrayPathSegment, AssignTarget, BinaryOp, CatchClause, DestructureKey, DestructureTarget, IncludeKind, Stmt, StaticVariable, SwitchCase};
pub use traits::TraitDecl;
