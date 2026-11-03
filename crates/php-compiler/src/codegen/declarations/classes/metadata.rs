//! Class property and constant metadata validation.

use php_runtime::{ArrayKey, MemberVisibility, PhpArray, Value};
use php_syntax::{BinaryOp, Expr, Visibility};

/// Compile a supported constant expression into an immutable runtime value.
pub(super) fn constant_value(expression: &Expr) -> Result<Value, String> {
    constant_value_in_class(expression, None, None)
}

pub(super) fn constant_value_in_class(expression: &Expr, class: Option<&php_syntax::ClassDecl>, program: Option<&php_syntax::Program>) -> Result<Value, String> {
    match expression {
        Expr::Integer(value) => Ok(Value::Integer(*value)),
        Expr::Number(value) => Ok(Value::Number(*value)),
        Expr::String(value) => Ok(Value::String(value.clone())),
        Expr::Bool(value) => Ok(Value::Bool(*value)),
        Expr::Null => Ok(Value::Null),
        Expr::Constant(name) => php_runtime::predefined_constant(name)
            .or_else(|| name.rsplit_once('\\').and_then(|(_, unqualified)| php_runtime::predefined_constant(unqualified)))
            .ok_or_else(|| format!("Undefined constant '{name}' in class constant expression")),
        Expr::Negate(value) => match constant_value(value)? {
            Value::Integer(number) => number.checked_neg().map(Value::Integer).ok_or_else(|| "Integer overflow in class constant".into()),
            Value::Number(number) => Ok(Value::Number(-number)),
            _ => Err("Unary minus in a class constant requires a numeric literal".into()),
        },
        Expr::Binary(BinaryOp::Concatenate, left, right) => {
            let left = constant_value_in_class(left, class, program)?;
            let right = constant_value_in_class(right, class, program)?;
            let as_string = |value: Value| match value {
                Value::String(value) => Ok(value),
                Value::Integer(value) => Ok(value.to_string()),
                Value::Number(value) => Ok(value.to_string()),
                Value::Bool(value) => Ok(if value { "1".to_owned() } else { String::new() }),
                Value::Null => Ok(String::new()),
                _ => Err("class property default concatenation requires scalar constants".to_owned()),
            };
            Ok(Value::String(format!("{}{}", as_string(left)?, as_string(right)?)))
        }
        Expr::Binary(BinaryOp::BitOr, left, right) => {
            let integer = |expression: &Expr| match constant_value_in_class(expression, class, program)? {
                Value::Integer(value) => Ok(value),
                _ => Err("bitwise OR in an attribute argument requires integer constants".to_string()),
            };
            let left = integer(left)?;
            let right = integer(right)?;
            Ok(Value::Integer(left | right))
        }
        Expr::Binary(operation, left, right) if matches!(operation,
            BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo | BinaryOp::Power
            | BinaryOp::BitAnd | BinaryOp::BitXor | BinaryOp::ShiftLeft | BinaryOp::ShiftRight) => {
            let left = constant_value_in_class(left, class, program)?;
            let right = constant_value_in_class(right, class, program)?;
            let integer = |value: Value| match value {
                Value::Integer(number) => Ok(number),
                _ => Err("class constant arithmetic requires integer literals".to_owned()),
            };
            if matches!(operation, BinaryOp::BitAnd | BinaryOp::BitXor | BinaryOp::ShiftLeft | BinaryOp::ShiftRight | BinaryOp::Modulo) {
                let left = integer(left)?;
                let right = integer(right)?;
                return match operation {
                    BinaryOp::BitAnd => Ok(Value::Integer(left & right)),
                    BinaryOp::BitXor => Ok(Value::Integer(left ^ right)),
                    BinaryOp::ShiftLeft if (0..64).contains(&right) => Ok(Value::Integer(left.wrapping_shl(right as u32))),
                    BinaryOp::ShiftRight if (0..64).contains(&right) => Ok(Value::Integer(left >> right)),
                    BinaryOp::Modulo if right != 0 => Ok(Value::Integer(if left == i64::MIN && right == -1 { 0 } else { left % right })),
                    BinaryOp::ShiftLeft | BinaryOp::ShiftRight => Err("Invalid shift in class constant expression".into()),
                    BinaryOp::Modulo => Err("Modulo by zero in class constant expression".into()),
                    _ => unreachable!(),
                };
            }
            if let (Value::Integer(left), Value::Integer(right)) = (&left, &right) {
                let result = match operation {
                    BinaryOp::Add => left.checked_add(*right),
                    BinaryOp::Subtract => left.checked_sub(*right),
                    BinaryOp::Multiply => left.checked_mul(*right),
                    BinaryOp::Power if (0..=u32::MAX as i64).contains(right) => left.checked_pow(*right as u32),
                    _ => None,
                };
                if let Some(result) = result { return Ok(Value::Integer(result)); }
            }
            let number = |value: Value| match value {
                Value::Integer(value) => Ok(value as f64), Value::Number(value) => Ok(value),
                _ => Err("class constant arithmetic requires numeric literals".to_owned()),
            };
            let left = number(left)?; let right = number(right)?;
            match operation {
                BinaryOp::Divide if right != 0.0 => Ok(Value::Number(left / right)),
                BinaryOp::Divide => Err("Division by zero in class constant expression".into()),
                BinaryOp::Power => Ok(Value::Number(left.powf(right))),
                _ => Err("Invalid class constant arithmetic expression".into()),
            }
        }
        Expr::Array(entries) => {
            let mut values = PhpArray::from_entries(Vec::new());
            for entry in entries {
                if entry.unpack || entry.by_reference {
                    return Err("class property default arrays cannot unpack or contain references".into());
                }
                let value = constant_value_in_class(&entry.value, class, program)?;
                if let Some(entry_key) = &entry.key {
                    let key = match constant_value_in_class(entry_key, class, program)? {
                        Value::Integer(index) => ArrayKey::Integer(index),
                        Value::String(key) => ArrayKey::String(key),
                        Value::Bool(value) => ArrayKey::Integer(i64::from(value)),
                        Value::Null => ArrayKey::String(String::new()),
                        _ => return Err("class property default array keys must be integers or strings".into()),
                    };
                    values.set(key, value);
                } else {
                    values.append(value)?;
                    continue;
                }
            }
            Ok(Value::Array(values))
        }
        Expr::ClassConstant(owner, name) => {
            if let Some(value) = php_runtime::pdo_class_constant(owner, name) { return Ok(value); }
            if let Some(value) = php_runtime::intl_class_constant(owner, name) { return Ok(value); }
            if let Some(value) = symfony_request_class_constant(owner, name) { return Ok(value); }
            if owner.trim_start_matches('\\').rsplit('\\').next().is_some_and(|owner| owner.eq_ignore_ascii_case("Attribute")) {
                let value = match name.to_ascii_uppercase().as_str() {
                    "TARGET_CLASS" => 1, "TARGET_FUNCTION" => 2, "TARGET_METHOD" => 4,
                    "TARGET_PROPERTY" => 8, "TARGET_CLASS_CONSTANT" => 16,
                    "TARGET_PARAMETER" => 32, "TARGET_ALL" => 63, "IS_REPEATABLE" => 64,
                    _ => return Err(format!("Undefined class constant Attribute::{name}")),
                };
                return Ok(Value::Integer(value));
            }
            let current = class.filter(|class| owner.eq_ignore_ascii_case("self") || owner.eq_ignore_ascii_case(&class.name))
                .or_else(|| program.and_then(|program| program.classes.iter().find(|class| class.name.eq_ignore_ascii_case(owner))));
            let Some(target) = current else {
                if let Some(interface) = program.and_then(|program| program.interfaces.iter().find(|interface| interface.name.eq_ignore_ascii_case(owner))) {
                    let constant = interface.constants.iter().find(|constant| constant.name == *name)
                        .ok_or_else(|| format!("Undefined class constant {owner}::{name}"))?;
                    return constant_value_in_class(&constant.value, None, program);
                }
                return Err(format!("Cannot resolve class constant {owner}::{name}"));
            };
            let constant = target.constants.iter().find(|constant| constant.name == *name)
                .or_else(|| program.and_then(|program| target.interfaces.iter().find_map(|interface_name| {
                    program.interfaces.iter().find(|interface| interface.name.eq_ignore_ascii_case(interface_name))?
                        .constants.iter().find(|constant| constant.name == *name)
                })))
                .or_else(|| program.and_then(|program| target.parent.as_ref().and_then(|parent| program.classes.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(parent))?.constants.iter().find(|constant| constant.name == *name))))
                .ok_or_else(|| format!("Undefined class constant {owner}::{name}"))?;
            if constant.is_enum_case {
                Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                    class_name: target.name.clone(), properties: std::collections::HashMap::from([("name".into(), Value::String(name.clone()))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))))
            } else { constant_value_in_class(&constant.value, Some(target), program) }
        }
        other => Err(format!("class property defaults must be constant literals in this version: {other:?}")),
    }
}

/// Symfony HttpFoundation's Request constants are inherited by Laravel's
/// Illuminate request wrapper. Composer class files are not always part of
/// the statically collected program, so retain these stable public constants
/// when evaluating class metadata before runtime autoloading.
fn symfony_request_class_constant(owner: &str, name: &str) -> Option<Value> {
    let owner = owner.trim_start_matches('\\').to_ascii_lowercase();
    if !matches!(owner.as_str(), "symfony\\component\\httpfoundation\\request" | "illuminate\\http\\request") {
        return None;
    }
    let value = match name.to_ascii_uppercase().as_str() {
        "HEADER_FORWARDED" => 0b000001,
        "HEADER_X_FORWARDED_FOR" => 0b000010,
        "HEADER_X_FORWARDED_HOST" => 0b000100,
        "HEADER_X_FORWARDED_PROTO" => 0b001000,
        "HEADER_X_FORWARDED_PORT" => 0b010000,
        "HEADER_X_FORWARDED_PREFIX" => 0b100000,
        "HEADER_X_FORWARDED_AWS_ELB" => 0b0011010,
        "HEADER_X_FORWARDED_TRAEFIK" => 0b0111110,
        _ => return None,
    };
    Some(Value::Integer(value))
}

/// Translate syntax visibility into the runtime representation.
pub(super) fn compile_visibility(visibility: Visibility) -> MemberVisibility {
    match visibility {
        Visibility::Public => MemberVisibility::Public,
        Visibility::Protected => MemberVisibility::Protected,
        Visibility::Private => MemberVisibility::Private,
    }
}

/// Evaluate a property default with access to the declaring class's constants and enum cases.
pub(super) fn class_property_default(
    property: &php_syntax::PropertyDecl,
    class: &php_syntax::ClassDecl,
    program: &php_syntax::Program,
) -> Result<Option<Value>, String> {
    fn find_class_constant<'a>(class: &'a php_syntax::ClassDecl, name: &str, program: &'a php_syntax::Program, depth: usize) -> Option<&'a php_syntax::ClassConstantDecl> {
        if depth > 64 { return None; }
        class.constants.iter().find(|constant| constant.name == name)
            .or_else(|| class.interfaces.iter().find_map(|interface_name| {
                program.interfaces.iter().find(|interface| interface.name.eq_ignore_ascii_case(interface_name))?
                    .constants.iter().find(|constant| constant.name == name)
            }))
            .or_else(|| class.parent.as_ref().and_then(|parent| find_class_constant(program.classes.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(parent))?, name, program, depth + 1)))
    }
    fn evaluate(expression: &Expr, class: &php_syntax::ClassDecl, program: &php_syntax::Program, depth: usize) -> Result<Value, String> {
        if depth > 64 { return Err("class property default nesting is too deep".into()); }
        match expression {
            Expr::ClassConstant(owner, name) if name.eq_ignore_ascii_case("class") => Ok(Value::String(owner.trim_start_matches('\\').to_string())),
            Expr::ClassConstant(owner, name) if php_runtime::pdo_class_constant(owner, name).is_some() => Ok(php_runtime::pdo_class_constant(owner, name).expect("checked PDO constant")),
            Expr::ClassConstant(owner, name) if php_runtime::intl_class_constant(owner, name).is_some() => Ok(php_runtime::intl_class_constant(owner, name).expect("checked Intl constant")),
            Expr::ClassConstant(owner, name) if symfony_request_class_constant(owner, name).is_some() => Ok(symfony_request_class_constant(owner, name).expect("checked Symfony Request constant")),
            Expr::ClassConstant(owner, name) if owner.trim_start_matches('\\').eq_ignore_ascii_case("SortDirection") => {
                if !["Ascending", "Descending"].iter().any(|case| case.eq_ignore_ascii_case(name)) { return Err(format!("Undefined class constant SortDirection::{name}")); }
                let case = if name.eq_ignore_ascii_case("Ascending") { "Ascending" } else { "Descending" };
                Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                    class_name: "SortDirection".into(), properties: std::collections::HashMap::from([("name".into(), Value::String(case.into()))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))))
            }
            Expr::ClassConstant(owner, name) if owner.eq_ignore_ascii_case("self") || owner.eq_ignore_ascii_case(&class.name) => {
                let constant = find_class_constant(class, name, program, 0)
                    .ok_or_else(|| format!("Undefined class constant {}::{name}", class.name))?;
                if constant.is_enum_case {
                    return Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                        class_name: class.name.clone(),
                        properties: std::collections::HashMap::from([("name".into(), Value::String(name.clone()))]),
                        property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                    }))));
                }
                evaluate(&constant.value, class, program, depth + 1)
            }
            Expr::ClassConstant(owner, name) => {
                let Some(target) = program.classes.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(owner)) else {
                    if let Some(interface) = program.interfaces.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(owner)) {
                        let constant = interface.constants.iter().find(|constant| constant.name == *name)
                            .ok_or_else(|| format!("Undefined class constant {owner}::{name}"))?;
                        return evaluate(&constant.value, class, program, depth + 1);
                    }
                    return Err(format!("Cannot resolve class constant {owner}::{name} in property default"));
                };
                let constant = find_class_constant(target, name, program, 0).ok_or_else(|| format!("Undefined class constant {owner}::{name}"))?;
                if constant.is_enum_case {
                    return Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                        class_name: target.name.clone(),
                        properties: std::collections::HashMap::from([("name".into(), Value::String(name.clone()))]),
                        property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                    }))));
                }
                evaluate(&constant.value, target, program, depth + 1)
            }
            Expr::Array(entries) => {
                let mut array = PhpArray::from_entries(Vec::new());
                for entry in entries {
                    if entry.unpack || entry.by_reference { return Err("class property default arrays cannot unpack or contain references".into()); }
                    let value = evaluate(&entry.value, class, program, depth + 1)?;
                    if let Some(key) = &entry.key {
                        let key = match evaluate(key, class, program, depth + 1)? {
                            Value::Integer(index) => ArrayKey::Integer(index), Value::String(key) => ArrayKey::String(key),
                            Value::Bool(value) => ArrayKey::Integer(i64::from(value)), Value::Null => ArrayKey::String(String::new()),
                            _ => return Err("class property default array keys must be integers or strings".into()),
                        };
                        array.set(key, value);
                    } else { array.append(value)?; }
                }
                Ok(Value::Array(array))
            }
            Expr::Binary(BinaryOp::Concatenate, left, right) => {
                let left = evaluate(left, class, program, depth + 1)?;
                let right = evaluate(right, class, program, depth + 1)?;
                let string = |value: Value| match value {
                    Value::String(value) => Ok(value), Value::Integer(value) => Ok(value.to_string()),
                    Value::Number(value) => Ok(value.to_string()), Value::Bool(value) => Ok(if value { "1".into() } else { String::new() }),
                    Value::Null => Ok(String::new()), _ => Err("class property default concatenation requires scalar constants".to_owned()),
                };
                Ok(Value::String(format!("{}{}", string(left)?, string(right)?)))
            }
            other => constant_value(other),
        }
    }
    if property.is_readonly && property.type_name.is_none() { return Err(format!("Readonly property ${} must have a type", property.name)); }
    if property.is_readonly && property.is_static { return Err(format!("Readonly property ${} cannot be static", property.name)); }
    if property.is_readonly && property.default.is_some() { return Err(format!("Readonly property ${} cannot have a default value", property.name)); }
    let default = property.default.as_ref().map(|expression| evaluate(expression, class, program, 0)).transpose()?;
    if let (Some(type_name), Some(value)) = (&property.type_name, &default) {
        if !matches_type(value, type_name) { return Err(format!("Default value for ${} does not match type {type_name}: {value:?}", property.name)); }
    }
    if property.type_name.is_none() && !property.is_readonly { Ok(Some(default.unwrap_or(Value::Null))) } else { Ok(default) }
}

/// Check a scalar literal against a property type during compilation.
pub(super) fn matches_type(value: &Value, type_name: &str) -> bool {
    if type_name.contains('|') { return type_name.split('|').any(|part| matches_type(value, part)); }
    if type_name.contains('&') { return type_name.split('&').all(|part| matches_type(value, part)); }
    if matches!(value, Value::Null) && (type_name.starts_with('?') || type_name.split(['|', '&']).any(|part| part.trim().eq_ignore_ascii_case("null"))) { return true; }
    match type_name.trim_start_matches('?') {
        "mixed" => true,
        "null" => matches!(value, Value::Null),
        "int" | "integer" => matches!(value, Value::Integer(_)) || matches!(value, Value::Number(number) if number.is_finite() && number.fract() == 0.0),
        "float" | "double" => matches!(value, Value::Number(_) | Value::Integer(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "array" => matches!(value, Value::Array(_)),
        expected => match value {
            Value::Object(object) => object.borrow().class_name.trim_start_matches('\\').eq_ignore_ascii_case(expected.trim_start_matches('\\')),
            _ => false,
        },
    }
}

