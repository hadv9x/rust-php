//! Function, method, parameter, and visibility syntax nodes.

use super::super::Stmt;

/// PHP member visibility, retained for compiler/runtime access checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// PHP visibility levels retained in declaration metadata.
pub enum Visibility {
    Public,
    Protected,
    Private,
}

/// Function or method declaration. Interface signatures have an empty body.
#[derive(Clone, Debug)]
/// A function or method signature and its parsed body.
pub struct FunctionDecl {
    pub name: String,
    pub attributes: Vec<super::AttributeDecl>,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    /// Whether calls to this function return the reference cell of an l-value.
    pub returns_by_reference: bool,
    pub visibility: Visibility,
    pub parameters: Vec<String>,
    /// Attribute groups attached to each parameter, aligned with `parameters`.
    pub parameter_attributes: Vec<Vec<super::AttributeDecl>>,
    pub parameter_types: Vec<Option<String>>,
    /// Marks parameters that PHP requires callers to pass as variables by reference.
    pub parameter_by_reference: Vec<bool>,
    /// Marks a final variadic parameter whose extra arguments are packed into an array.
    pub parameter_variadic: Vec<bool>,
    /// Compile-time literal defaults, aligned with `parameters`.
    pub parameter_defaults: Vec<Option<super::Expr>>,
    pub return_type: Option<String>,
    pub promoted_properties: Vec<super::PropertyDecl>,
    pub body: Vec<Stmt>,
}
