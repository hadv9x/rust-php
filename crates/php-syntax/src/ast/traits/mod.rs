//! Trait declaration syntax nodes.

use super::super::{ClassConstantDecl, FunctionDecl, PropertyDecl, TraitAdaptation};

/// Reusable methods and properties copied into consuming classes.
#[derive(Clone, Debug)]
/// A trait declaration containing reusable properties and methods.
pub struct TraitDecl {
    pub name: String,
    pub attributes: Vec<super::AttributeDecl>,
    pub traits: Vec<String>,
    pub trait_adaptations: Vec<TraitAdaptation>,
    pub properties: Vec<PropertyDecl>,
    pub constants: Vec<ClassConstantDecl>,
    pub methods: Vec<FunctionDecl>,
}
