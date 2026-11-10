//! Public lexer, parser, and syntax-tree interface.

mod ast;
mod diagnostics;
mod lexer;
mod parser;

pub use ast::{
    ArrayEntry, ArrayPathSegment, AssignTarget, AttributeDecl, BinaryOp, CastType,
    ClassConstantDecl, ClassDecl, ConstantDecl, DestructureKey, DestructureTarget, Expr,
    FirstClassCallableTarget, FunctionDecl, IncludeKind, IncrementTarget, InterfaceDecl,
    IssetPropertySegment, MatchArm, Program, PropertyDecl, StaticVariable, Stmt, TraitAdaptation,
    TraitDecl, Visibility,
};
/// Parse PHP source into the frontend AST, returning a source-located diagnostic on error.
pub use parser::{parse, parse_with_anonymous_class_id};

#[cfg(test)]
#[path = "../../../tests/crates/php-syntax/syntax.rs"]
mod tests;
