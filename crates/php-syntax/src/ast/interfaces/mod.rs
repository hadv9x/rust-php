//! Interface declarations and their contracts.

use super::super::{ClassConstantDecl, FunctionDecl};

/// Interface contract checked by the compiler.
#[derive(Clone, Debug)]
/// An interface declaration with parents, constants, and method contracts.
pub struct InterfaceDecl {
    pub name: String,
    pub attributes: Vec<super::AttributeDecl>,
    pub parents: Vec<String>,
    pub constants: Vec<ClassConstantDecl>,
    pub methods: Vec<FunctionDecl>,
}
