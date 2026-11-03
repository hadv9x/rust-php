//! Class, property, constant, and trait-adaptation syntax nodes.

use super::super::{Expr, FunctionDecl, Stmt, Visibility};

/// Instance or static property declaration with its default and access level.
#[derive(Clone, Debug)]
/// A declared instance or static property and its metadata.
pub struct PropertyDecl {
    pub name: String,
    pub attributes: Vec<AttributeDecl>,
    pub default: Option<Expr>,
    pub is_static: bool,
    pub is_readonly: bool,
    pub visibility: Visibility,
    /// Optional PHP 8.4 asymmetric setter visibility.
    pub set_visibility: Option<Visibility>,
    pub type_name: Option<String>,
    /// Optional PHP 8.4 property getter body.
    pub get_hook: Option<Vec<Stmt>>,
    /// Optional PHP 8.4 property setter body and its parameter name.
    pub set_hook: Option<PropertySetHook>,
}

/// A PHP 8.4 `set` property hook body.
#[derive(Clone, Debug)]
pub struct PropertySetHook {
    pub parameter: String,
    pub body: Vec<Stmt>,
}

/// Scalar class constant with PHP visibility and optional final modifier.
#[derive(Clone, Debug)]
/// A class constant expression and its visibility/final metadata.
pub struct ClassConstantDecl {
    pub name: String,
    pub value: Expr,
    pub type_name: Option<String>,
    pub visibility: Visibility,
    pub is_final: bool,
    /// Marks an enum case rather than an ordinary class constant.
    pub is_enum_case: bool,
    /// Whether the enum case has an explicit backed value expression.
    pub enum_case_has_value: bool,
}

/// A trait method conflict rule or alias declared in a class `use` block.
#[derive(Clone, Debug)]
/// A trait method conflict-resolution or aliasing rule.
pub struct TraitAdaptation {
    pub trait_name: String,
    pub method_name: String,
    pub excluded_traits: Vec<String>,
    pub alias: Option<String>,
    pub visibility: Option<Visibility>,
}

/// A PHP attribute attached to a class-like declaration.
#[derive(Clone, Debug)]
pub struct AttributeDecl {
    pub name: String,
    pub arguments: Vec<Expr>,
}

/// Class metadata retained until inheritance and trait composition are resolved.
#[derive(Clone, Debug)]
/// A class declaration including inheritance and composed members.
pub struct ClassDecl {
    pub name: String,
    pub attributes: Vec<AttributeDecl>,
    pub is_enum: bool,
    pub enum_backing_type: Option<String>,
    pub is_abstract: bool,
    pub is_final: bool,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
    pub traits: Vec<String>,
    pub trait_adaptations: Vec<TraitAdaptation>,
    pub properties: Vec<PropertyDecl>,
    pub constants: Vec<ClassConstantDecl>,
    pub methods: Vec<FunctionDecl>,
}
