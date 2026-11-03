//! Inheritance, interface contract, and member lookup helpers.

use std::collections::HashMap;
use php_runtime::{BytecodeClass, BytecodeConstant, BytecodeFunction, BytecodeProperty, MemberVisibility};
use php_syntax::FunctionDecl;

/// Check PHP-style parameter contravariance and return covariance for overrides/contracts.
pub(super) fn signature_compatible(
    implementation_parameters: &[Option<String>],
    implementation_return: Option<&str>,
    contract_parameters: &[Option<String>],
    contract_return: Option<&str>,
    classes: &HashMap<String, &php_syntax::ClassDecl>,
    interfaces: &HashMap<String, &php_syntax::InterfaceDecl>,
) -> bool {
    if implementation_parameters.len() != contract_parameters.len() { return false; }
    let parameters_compatible = implementation_parameters.iter().zip(contract_parameters).all(|(implementation, contract)| {
        // The implementation may accept a broader set of values than the contract requires.
        is_subtype(contract.as_deref().unwrap_or("mixed"), implementation.as_deref().unwrap_or("mixed"), classes, interfaces)
    });
    let return_compatible = is_subtype(
        implementation_return.unwrap_or("mixed"),
        contract_return.unwrap_or("mixed"),
        classes,
        interfaces,
    );
    parameters_compatible && return_compatible
}

/// Determine whether every value of `actual` can be used where `expected` is required.
pub(super) fn is_subtype(
    actual: &str,
    expected: &str,
    classes: &HashMap<String, &php_syntax::ClassDecl>,
    interfaces: &HashMap<String, &php_syntax::InterfaceDecl>,
) -> bool {
    if actual.starts_with('?') {
        let expanded = format!("{}|null", actual.trim_start_matches('?'));
        return is_subtype(&expanded, expected, classes, interfaces);
    }
    if expected.starts_with('?') {
        let expanded = format!("{}|null", expected.trim_start_matches('?'));
        return is_subtype(actual, &expanded, classes, interfaces);
    }
    if actual.contains('|') {
        return actual.split('|').all(|member| is_subtype(member, expected, classes, interfaces));
    }
    if expected.contains('|') {
        return expected.split('|').any(|member| is_subtype(actual, member, classes, interfaces));
    }
    if expected.contains('&') {
        return expected.split('&').all(|member| is_subtype(actual, member, classes, interfaces));
    }
    if actual.contains('&') {
        return actual.split('&').any(|member| is_subtype(member, expected, classes, interfaces));
    }
    let (actual_name, expected_name) = (actual, expected);
    if actual_name.eq_ignore_ascii_case(expected_name) { return true; }
    let actual_lower = actual_name.to_ascii_lowercase();
    let expected_lower = expected_name.to_ascii_lowercase();
    if actual_lower == "never" || expected_lower == "mixed" { return true; }
    if actual_lower == "mixed" || expected_lower == "never" { return false; }
    if matches!(actual_lower.as_str(), "true" | "false") && expected_lower == "bool" { return true; }
    if expected_lower == "iterable" && matches!(actual_lower.as_str(), "array" | "traversable") { return true; }
    if expected_lower == "object" && !matches!(actual_lower.as_str(), "int" | "integer" | "float" | "double" | "string" | "bool" | "boolean" | "array" | "iterable" | "void" | "null" | "true" | "false") {
        return true;
    }
    if matches!(actual_lower.as_str(), "int" | "integer" | "float" | "double" | "string" | "bool" | "boolean" | "array" | "iterable" | "object" | "void" | "null" | "true" | "false")
        || matches!(expected_lower.as_str(), "int" | "integer" | "float" | "double" | "string" | "bool" | "boolean" | "array" | "iterable" | "object" | "void" | "null" | "true" | "false") {
        return false;
    }
    declaration_is_a(&actual_lower, &expected_lower, classes, interfaces, &mut std::collections::HashSet::new())
}

/// Check whether a declared class/interface is assignable to another nominal type.
pub(super) fn declaration_is_a(
    actual: &str,
    expected: &str,
    classes: &HashMap<String, &php_syntax::ClassDecl>,
    interfaces: &HashMap<String, &php_syntax::InterfaceDecl>,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    if actual == expected { return true; }
    if !visited.insert(actual.to_string()) { return false; }
    if let Some(class) = classes.get(actual) {
        if class.parent.as_deref().is_some_and(|parent| declaration_is_a(&parent.to_ascii_lowercase(), expected, classes, interfaces, visited)) { return true; }
        return class.interfaces.iter().any(|interface| declaration_is_a(&interface.to_ascii_lowercase(), expected, classes, interfaces, visited));
    }
    interfaces.get(actual).is_some_and(|interface| interface.parents.iter().any(|parent| declaration_is_a(&parent.to_ascii_lowercase(), expected, classes, interfaces, visited)))
}

/// Find an instance or static property through the parent-class chain.
pub(super) fn find_property<'a>(class_name: &str, property: &str, classes: &'a HashMap<String, BytecodeClass>) -> Option<(&'a BytecodeProperty, bool)> {
    let class = classes.get(class_name)?;
    if let Some(metadata) = class.properties.get(property) { return Some((metadata, false)); }
    if let Some(metadata) = class.static_properties.get(property) { return Some((metadata, true)); }
    class.parent.as_deref().and_then(|parent| find_property(parent, property, classes))
}

/// Find a class constant through the parent-class chain.
pub(super) fn find_constant<'a>(class_name: &str, constant: &str, classes: &'a HashMap<String, BytecodeClass>) -> Option<&'a BytecodeConstant> {
    let class = classes.get(class_name)?;
    class.constants.get(constant).or_else(|| class.parent.as_deref().and_then(|parent| find_constant(parent, constant, classes)))
}

/// Gather interfaces implemented by a class and each of its parents.
pub(super) fn collect_class_interfaces(
    class: &php_syntax::ClassDecl,
    declarations: &HashMap<String, &php_syntax::ClassDecl>,
    output: &mut Vec<String>,
) -> Result<(), String> {
    if let Some(parent_name) = &class.parent {
        if let Some(parent) = declarations.get(&parent_name.to_ascii_lowercase()) {
            collect_class_interfaces(parent, declarations, output)?;
        } else if !matches!(parent_name.to_ascii_lowercase().as_str(),
            "exception" | "error" | "typeerror" | "argumentcounterror" | "compileerror" | "parseerror"
            | "arithmeticerror" | "divisionbyzeroerror" | "assertionerror" | "unhandledmatcherror" | "valueerror") {
            return Err(format!("Class '{parent_name}' not found"));
        }
    }
    output.extend(class.interfaces.iter().cloned());
    Ok(())
}

/// Convert visibility to an order used for inheritance widening checks.
pub(super) fn visibility_rank(visibility: MemberVisibility) -> u8 {
    match visibility {
        MemberVisibility::Private => 0,
        MemberVisibility::Protected => 1,
        MemberVisibility::Public => 2,
    }
}

/// Build the effective method table, with child methods overriding parent methods.
pub(super) fn collect_class_methods<'a>(name: &str, classes: &'a HashMap<String, php_runtime::BytecodeClass>, output: &mut HashMap<String, &'a BytecodeFunction>) -> Result<(), String> {
    // Parent methods are collected first; child methods replace same-name entries.
    let class = classes.get(name).ok_or_else(|| format!("Class '{name}' not found"))?;
    if let Some(parent) = &class.parent { collect_class_methods(parent, classes, output)?; }
    output.extend(class.methods.iter().map(|(name, method)| (name.clone(), method)));
    Ok(())
}

/// Collect interface requirements, including methods inherited from parent interfaces.
pub(super) fn collect_interface_methods<'a>(
    interface: &'a php_syntax::InterfaceDecl,
    declarations: &HashMap<String, &'a php_syntax::InterfaceDecl>,
    output: &mut HashMap<String, &'a FunctionDecl>,
) -> Result<(), String> {
    // An interface inherits the combined method contract of all its parent interfaces.
    for parent in &interface.parents {
        let parent_decl = declarations.get(&parent.to_ascii_lowercase()).ok_or_else(|| format!("Interface '{parent}' not found"))?;
        collect_interface_methods(parent_decl, declarations, output)?;
    }
    for method in &interface.methods { output.insert(method.name.to_ascii_lowercase(), method); }
    Ok(())
}

