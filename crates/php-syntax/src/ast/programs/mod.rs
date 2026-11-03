//! Root syntax tree node for a parsed PHP program.

use super::super::{ClassDecl, Expr, FunctionDecl, InterfaceDecl, Stmt, TraitDecl};

/// Parsed PHP source before semantic validation and bytecode generation.
#[derive(Clone, Debug)]
/// Top-level statements, functions, classes, interfaces, and traits.
pub struct Program {
    /// Scalar argument calls made from this source unit use strict typing when enabled.
    pub strict_types: bool,
    pub statements: Vec<Stmt>,
    pub functions: Vec<FunctionDecl>,
    pub classes: Vec<ClassDecl>,
    pub interfaces: Vec<InterfaceDecl>,
    pub traits: Vec<TraitDecl>,
    pub constants: Vec<ConstantDecl>,
}

/// One file-scope or namespace-scope constant declaration.
#[derive(Clone, Debug)]
pub struct ConstantDecl {
    pub name: String,
    pub value: Expr,
}
