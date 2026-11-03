//! Top-level function, class, interface, and trait compilation passes.

use php_runtime::{bytecode::Op, ArrayKey, BytecodeProgram, Value};
use php_syntax::{BinaryOp, Expr, Program};

use super::CodeGenerator;

mod classes;
mod functions;

pub(super) fn strict_mode_for_body(body: &[php_syntax::Stmt], fallback: bool) -> bool {
    match body.first() {
        Some(php_syntax::Stmt::SourceFile(_, strict_types, _)) => *strict_types,
        _ => fallback,
    }
}

pub(super) fn constant_declarations(program: &Program) -> Result<Vec<(String, Value)>, String> {
    let mut declared = std::collections::HashSet::new();
    let mut values = Vec::with_capacity(program.constants.len());
    for constant in &program.constants {
        if !declared.insert(constant.name.clone()) {
            return Err(format!("Constant '{}' is already declared", constant.name));
        }
        let value = constant_parameter_value(&constant.value, Some(program), None, None)?;
        values.push((constant.name.clone(), value));
    }
    Ok(values)
}

/// Lower a parameter's supported constant expression into its runtime value.
pub(super) fn parameter_defaults(
    defaults: &[Option<Expr>], program: Option<&Program>, class_context: Option<&str>, parent_context: Option<&str>,
) -> Result<Vec<Option<Value>>, String> {
    defaults.iter().map(|default| default.as_ref().map(|expression| constant_parameter_value(expression, program, class_context, parent_context)).transpose()).collect()
}

/// Evaluate the constant subset allowed in parameter defaults at compile time.
pub(super) fn constant_parameter_value(expression: &Expr, program: Option<&Program>, class_context: Option<&str>, parent_context: Option<&str>) -> Result<Value, String> {
    constant_parameter_value_inner(expression, program, class_context, parent_context, &mut Vec::new())
}

fn constant_parameter_value_inner(
    expression: &Expr, program: Option<&Program>, class_context: Option<&str>, parent_context: Option<&str>, resolving: &mut Vec<String>,
) -> Result<Value, String> {
    match expression {
        Expr::Null => Ok(Value::Null),
        Expr::Bool(value) => Ok(Value::Bool(*value)),
        Expr::Integer(value) => Ok(Value::Integer(*value)),
        Expr::Number(value) => Ok(Value::Number(*value)),
        Expr::String(value) => Ok(Value::String(value.clone())),
        Expr::Negate(value) => match constant_parameter_value_inner(value, program, class_context, parent_context, resolving)? {
            Value::Integer(number) => number.checked_neg().map(Value::Integer).ok_or_else(|| "Integer overflow in parameter default".into()),
            Value::Number(number) => Ok(Value::Number(-number)),
            _ => Err("Unary minus in a parameter default requires a numeric constant".into()),
        },
        Expr::Binary(operation, left, right) => constant_binary(*operation, left, right, program, class_context, parent_context, resolving),
        Expr::ClassConstant(class_name, constant_name) => resolve_class_constant(class_name, constant_name, program, class_context, parent_context, resolving),
        Expr::Constant(name) => {
            let program = program.ok_or("Constant references require a declaration context")?;
            let Some(declaration) = program.constants.iter().find(|constant| constant.name == *name)
                .or_else(|| name.rsplit_once('\\').and_then(|(_, unqualified)| program.constants.iter().find(|constant| constant.name == unqualified))) else {
                    return php_runtime::predefined_constant(name)
                        .or_else(|| name.rsplit_once('\\').and_then(|(_, unqualified)| php_runtime::predefined_constant(unqualified)))
                        .ok_or_else(|| format!("Undefined constant '{name}' in constant expression"));
                };
            if resolving.iter().any(|active| active == &declaration.name) {
                return Err(format!("Circular constant reference involving '{}'", declaration.name));
            }
            let name = declaration.name.clone();
            resolving.push(name);
            let result = constant_parameter_value_inner(&declaration.value, Some(program), class_context, parent_context, resolving);
            resolving.pop();
            result
        }
        Expr::Array(entries) => {
            let mut result = Vec::<(ArrayKey, Value)>::new();
            let mut next_index = 0i64;
            for entry in entries {
                if entry.unpack || entry.by_reference {
                    return Err("Unpacking and references are not allowed in parameter default arrays".into());
                }
                let key = if let Some(key) = &entry.key {
                    match constant_parameter_value_inner(key, program, class_context, parent_context, resolving)? {
                        Value::Integer(index) => ArrayKey::Integer(index),
                        Value::String(key) => constant_array_key(key),
                        Value::Bool(value) => ArrayKey::Integer(i64::from(value)),
                        Value::Null => ArrayKey::String(String::new()),
                        _ => return Err("Parameter default array keys must be constant integers or strings".into()),
                    }
                } else {
                    let key = ArrayKey::Integer(next_index);
                    next_index = next_index.checked_add(1).ok_or("Parameter default array index overflow")?;
                    key
                };
                if let ArrayKey::Integer(index) = key {
                    if index >= next_index {
                        next_index = index.checked_add(1).ok_or("Parameter default array index overflow")?;
                    }
                    let value = constant_parameter_value_inner(&entry.value, program, class_context, parent_context, resolving)?;
                    if let Some((_, existing)) = result.iter_mut().find(|(candidate, _)| *candidate == ArrayKey::Integer(index)) {
                        *existing = value;
                    } else {
                        result.push((ArrayKey::Integer(index), value));
                    }
                } else {
                    let value = constant_parameter_value_inner(&entry.value, program, class_context, parent_context, resolving)?;
                    if let Some((_, existing)) = result.iter_mut().find(|(candidate, _)| *candidate == key) {
                        *existing = value;
                    } else {
                        result.push((key, value));
                    }
                }
            }
            Ok(Value::from_array_entries(result))
        }
        Expr::NewObject(class_name, arguments, _) if arguments.is_empty() => {
            let mut properties = std::collections::HashMap::new();
            properties.insert("\0php-rust-parameter-default-object".to_string(), Value::Bool(true));
            Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                class_name: class_name.clone(), properties, property_order: Vec::new(),
                active_property_hooks: Default::default(), exception: None,
            }))))
        }
        other => Err(format!("Parameter defaults must be constant expressions or array literals: {other:?}")),
    }
}

/// Evaluate the arithmetic/concatenation portion of PHP constant expressions.
fn constant_binary(operation: BinaryOp, left: &Expr, right: &Expr, program: Option<&Program>, class_context: Option<&str>, parent_context: Option<&str>, resolving: &mut Vec<String>) -> Result<Value, String> {
    let left = constant_parameter_value_inner(left, program, class_context, parent_context, resolving)?;
    let right = constant_parameter_value_inner(right, program, class_context, parent_context, resolving)?;
    if operation == BinaryOp::Concatenate {
        return Ok(Value::String(format!("{}{}", constant_string(&left)?, constant_string(&right)?)));
    }
    if matches!(operation, BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::ShiftLeft | BinaryOp::ShiftRight) {
        let lhs = constant_integer(&left)?;
        let rhs = constant_integer(&right)?;
        return match operation {
            BinaryOp::BitAnd => Ok(Value::Integer(lhs & rhs)),
            BinaryOp::BitOr => Ok(Value::Integer(lhs | rhs)),
            BinaryOp::BitXor => Ok(Value::Integer(lhs ^ rhs)),
            BinaryOp::ShiftLeft if (0..64).contains(&rhs) => Ok(Value::Integer(lhs.wrapping_shl(rhs as u32))),
            BinaryOp::ShiftRight if (0..64).contains(&rhs) => Ok(Value::Integer(lhs >> rhs)),
            BinaryOp::ShiftLeft | BinaryOp::ShiftRight => Err("Invalid shift amount in parameter default".into()),
            _ => unreachable!(),
        };
    }
    if !matches!(operation, BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply | BinaryOp::Divide | BinaryOp::Modulo | BinaryOp::Power) {
        return Err(format!("Operator {operation:?} is not supported in parameter default constant expressions"));
    }
    if operation == BinaryOp::Divide || operation == BinaryOp::Power {
        if operation == BinaryOp::Power {
            if let (Value::Integer(base), Value::Integer(exponent)) = (&left, &right) {
                if (0..=u32::MAX as i64).contains(exponent) {
                    if let Some(result) = base.checked_pow(*exponent as u32) { return Ok(Value::Integer(result)); }
                }
            }
        }
        let lhs = constant_number(&left)?;
        let rhs = constant_number(&right)?;
        if operation == BinaryOp::Divide && rhs == 0.0 { return Err("Division by zero in parameter default".into()); }
        return Ok(Value::Number(if operation == BinaryOp::Divide { lhs / rhs } else { lhs.powf(rhs) }));
    }
    if operation == BinaryOp::Modulo {
        let lhs = constant_integer(&left)?;
        let rhs = constant_integer(&right)?;
        if rhs == 0 { return Err("Modulo by zero in parameter default".into()); }
        return Ok(Value::Integer(if lhs == i64::MIN && rhs == -1 { 0 } else { lhs % rhs }));
    }
    if let (Value::Integer(lhs), Value::Integer(rhs)) = (&left, &right) {
        let integer = match operation {
            BinaryOp::Add => lhs.checked_add(*rhs),
            BinaryOp::Subtract => lhs.checked_sub(*rhs),
            BinaryOp::Multiply => lhs.checked_mul(*rhs),
            _ => unreachable!(),
        };
        if let Some(value) = integer { return Ok(Value::Integer(value)); }
    }
    let lhs = constant_number(&left)?;
    let rhs = constant_number(&right)?;
    Ok(Value::Number(match operation {
        BinaryOp::Add => lhs + rhs,
        BinaryOp::Subtract => lhs - rhs,
        BinaryOp::Multiply => lhs * rhs,
        _ => unreachable!(),
    }))
}

/// Resolve a class/interface constant in a declaration-time parameter default.
fn resolve_class_constant(
    class_name: &str, constant_name: &str, program: Option<&Program>, class_context: Option<&str>, parent_context: Option<&str>,
    resolving: &mut Vec<String>,
) -> Result<Value, String> {
    let program = program.ok_or("Class constants in parameter defaults require a declaration context")?;
    let resolved_name = match class_name.to_ascii_lowercase().as_str() {
        "self" => class_context.ok_or("self:: parameter defaults require a class context")?,
        "static" => return Err("static:: is not allowed in a parameter default constant expression".into()),
        "parent" => parent_context.ok_or("parent:: parameter defaults require a parent class")?,
        _ => class_name,
    };
    if constant_name.eq_ignore_ascii_case("class") { return Ok(Value::String(resolved_name.trim_start_matches('\\').to_string())); }
    if let Some(value) = php_runtime::pdo_class_constant(resolved_name, constant_name) { return Ok(value); }
    if let Some(value) = php_runtime::intl_class_constant(resolved_name, constant_name) { return Ok(value); }
    if resolved_name.eq_ignore_ascii_case("RoundingMode") {
        const CASES: &[&str] = &["HalfAwayFromZero", "HalfTowardsZero", "HalfEven", "HalfOdd", "TowardsZero", "AwayFromZero", "NegativeInfinity", "PositiveInfinity"];
        if let Some(case) = CASES.iter().find(|case| case.eq_ignore_ascii_case(constant_name)) {
            return Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                class_name: "RoundingMode".into(),
                properties: std::collections::HashMap::from([("name".into(), Value::String((*case).into()))]),
                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
            }))));
        }
        return Err(format!("Undefined class constant RoundingMode::{constant_name}"));
    }
    if resolved_name.eq_ignore_ascii_case("SortDirection") {
        if ["Ascending", "Descending"].iter().any(|case| case.eq_ignore_ascii_case(constant_name)) {
            let case = if constant_name.eq_ignore_ascii_case("Ascending") { "Ascending" } else { "Descending" };
            return Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                class_name: "SortDirection".into(), properties: std::collections::HashMap::from([("name".into(), Value::String(case.into()))]),
                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
            }))));
        }
        return Err(format!("Undefined class constant SortDirection::{constant_name}"));
    }
    if let Some(value) = builtin_class_constant(resolved_name, constant_name) { return Ok(Value::Integer(value)); }
    let mut class = program.classes.iter().find(|class| class.name.eq_ignore_ascii_case(resolved_name));
    while let Some(declaration) = class {
        if let Some(constant) = declaration.constants.iter().find(|constant| constant.name == constant_name) {
            if constant.visibility != php_syntax::Visibility::Public {
                let is_declaring_scope = class_context.is_some_and(|scope| scope.eq_ignore_ascii_case(&declaration.name));
                let can_access_protected = constant.visibility == php_syntax::Visibility::Protected
                    && class_context.is_some_and(|scope| declaration_is_subclass(program, scope, &declaration.name));
                if !is_declaring_scope && !can_access_protected {
                    return Err(format!("Cannot access {} class constant {}::{constant_name}",
                        if constant.visibility == php_syntax::Visibility::Private { "private" } else { "protected" }, declaration.name));
                }
            }
            let key = format!("{}::{constant_name}", declaration.name);
            if resolving.iter().any(|active| active == &key) { return Err(format!("Circular class constant reference involving {key}")); }
            resolving.push(key);
            let result = constant_parameter_value_inner(&constant.value, Some(program), Some(&declaration.name), declaration.parent.as_deref(), resolving);
            resolving.pop();
            return result;
        }
        for interface_name in &declaration.interfaces {
            if let Some(interface) = program.interfaces.iter().find(|interface| interface.name.eq_ignore_ascii_case(interface_name)) {
                if let Some(constant) = interface.constants.iter().find(|constant| constant.name == constant_name) {
                    let key = format!("{}::{constant_name}", interface.name);
                    if resolving.iter().any(|active| active == &key) { return Err(format!("Circular class constant reference involving {key}")); }
                    resolving.push(key);
                    let result = constant_parameter_value_inner(&constant.value, Some(program), Some(&interface.name), None, resolving);
                    resolving.pop();
                    return result;
                }
            }
        }
        class = declaration.parent.as_deref().and_then(|parent| program.classes.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(parent)));
    }
    if let Some(declaration) = program.classes.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(resolved_name)) {
        for interface_name in &declaration.interfaces {
            if let Some(interface) = program.interfaces.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(interface_name)) {
                if let Some(constant) = interface.constants.iter().find(|constant| constant.name == constant_name) {
                    let key = format!("{}::{constant_name}", interface.name);
                    if resolving.iter().any(|active| active == &key) { return Err(format!("Circular class constant reference involving {key}")); }
                    resolving.push(key);
                    let result = constant_parameter_value_inner(&constant.value, Some(program), Some(&interface.name), None, resolving);
                    resolving.pop();
                    return result;
                }
            }
        }
    }
    for interface in &program.interfaces {
        if !interface.name.eq_ignore_ascii_case(resolved_name) { continue; }
        let constant = interface.constants.iter().find(|constant| constant.name == constant_name)
            .ok_or_else(|| format!("Undefined class constant {}::{constant_name}", interface.name))?;
        let key = format!("{}::{constant_name}", interface.name);
        if resolving.iter().any(|active| active == &key) { return Err(format!("Circular class constant reference involving {key}")); }
        resolving.push(key);
        let result = constant_parameter_value_inner(&constant.value, Some(program), class_context, parent_context, resolving);
        resolving.pop();
        return result;
    }
    Err(format!("Unknown class or interface '{resolved_name}' in parameter default"))
}

fn builtin_class_constant(class_name: &str, constant_name: &str) -> Option<i64> {
    let class = class_name.rsplit('\\').next()?.to_ascii_lowercase();
    let constant = constant_name.to_ascii_uppercase();
    let value = match (class.as_str(), constant.as_str()) {
        ("cachingiterator", "CALL_TOSTRING") => 1,
        ("cachingiterator", "TOSTRING_USE_CURRENT") => 2,
        ("cachingiterator", "TOSTRING_USE_KEY") => 4,
        ("cachingiterator", "TOSTRING_USE_INNER") => 8,
        ("cachingiterator", "FULL_CACHE") => 256,
        ("recursiveiteratoriterator", "LEAVES_ONLY") => 0,
        ("recursiveiteratoriterator", "SELF_FIRST") => 1,
        ("recursiveiteratoriterator", "CHILD_FIRST") => 2,
        ("recursiveiteratoriterator", "CATCH_GET_CHILD") => 16,
        ("recursivedirectoryiterator", "CURRENT_AS_FILEINFO" | "KEY_AS_PATHNAME") => 0,
        ("recursivedirectoryiterator", "CURRENT_AS_SELF") => 16,
        ("recursivedirectoryiterator", "CURRENT_AS_PATHNAME") => 32,
        ("recursivedirectoryiterator", "KEY_AS_FILENAME") => 256,
        ("recursivedirectoryiterator", "FOLLOW_SYMLINKS") => 512,
        ("recursivedirectoryiterator", "NEW_CURRENT_AND_KEY") => 256,
        ("recursivedirectoryiterator", "SKIP_DOTS") => 4096,
        ("recursivedirectoryiterator", "UNIX_PATHS") => 8192,
        ("arrayiterator", "STD_PROP_LIST") => 1,
        ("arrayiterator", "ARRAY_AS_PROPS") => 2,
        _ => return None,
    };
    Some(value)
}

fn declaration_is_subclass(program: &Program, candidate: &str, parent: &str) -> bool {
    let mut current = program.classes.iter().find(|class| class.name.eq_ignore_ascii_case(candidate)).and_then(|class| class.parent.as_deref());
    while let Some(name) = current {
        if name.eq_ignore_ascii_case(parent) { return true; }
        current = program.classes.iter().find(|class| class.name.eq_ignore_ascii_case(name)).and_then(|class| class.parent.as_deref());
    }
    false
}

fn constant_integer(value: &Value) -> Result<i64, String> {
    match value {
        Value::Integer(value) => Ok(*value),
        Value::Number(value) if value.is_finite() && *value >= i64::MIN as f64 && *value <= i64::MAX as f64 => Ok(*value as i64),
        Value::Bool(value) => Ok(i64::from(*value)),
        Value::Null => Ok(0),
        Value::String(value) => value.parse().map_err(|_| "Invalid numeric constant in parameter default".into()),
        _ => Err("Parameter arithmetic requires scalar numeric constants".into()),
    }
}

fn constant_number(value: &Value) -> Result<f64, String> {
    match value {
        Value::Integer(value) => Ok(*value as f64),
        Value::Number(value) => Ok(*value),
        Value::Bool(value) => Ok(if *value { 1.0 } else { 0.0 }),
        Value::Null => Ok(0.0),
        Value::String(value) => value.parse().map_err(|_| "Invalid numeric constant in parameter default".into()),
        _ => Err("Parameter arithmetic requires scalar numeric constants".into()),
    }
}

fn constant_string(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Integer(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(if *value { "1".into() } else { String::new() }),
        Value::Null => Ok(String::new()),
        _ => Err("Parameter default concatenation requires scalar constants".into()),
    }
}

/// Apply PHP's canonical decimal-integer string key conversion to constant array keys.
fn constant_array_key(key: String) -> ArrayKey {
    if let Ok(index) = key.parse::<i64>() {
        if index.to_string() == key { return ArrayKey::Integer(index); }
    }
    ArrayKey::String(key)
}

impl CodeGenerator<'_> {
    /// Build executable code and resolve class/interface/trait relationships.
    pub(super) fn generate(mut self, program: &Program) -> Result<BytecodeProgram, String> {
        for (name, value) in constant_declarations(program)? {
            self.instructions.push(Op::Const(value));
            self.instructions.push(Op::DeclareConstant(name));
        }
        self.statements(&program.statements);
        self.resolve_gotos();
        if let Some(error) = self.errors.first() { return Err(error.clone()); }
        let functions = functions::compile(&program.functions, program)?;
        let classes = classes::compile(program)?;
        Ok(BytecodeProgram::new(self.instructions, functions, classes))
    }
}

