//! Array inspection and comparison functions.

use crate::{ArrayKey, BytecodeClass, Value};
use std::collections::{HashMap, HashSet};
use super::common::*;

/// Query array data with count semantics.
pub(super) fn count(
    args: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>,
) -> Result<Value, String> {
    require_count("count", args, 1, 2)?;
    /// Count nested array entries recursively for `count($array, COUNT_RECURSIVE)`.
    fn deep_count(value: &Value) -> i64 {
        let value = value.dereference();
        match &value {
            Value::Array(items) => items.iter().map(|(_, child)| match child.dereference() {
                Value::Array(nested) => nested.len() as i64 + deep_count(child),
                _ => 0,
            }).sum(),
            _ => 0,
        }
    }
    let mode = args.get(1).map_or(0, |value| value.number() as i64);
    if !matches!(mode, 0 | 1) {
        return Err("ValueError:count(): Argument #2 ($mode) must be either COUNT_NORMAL or COUNT_RECURSIVE".into());
    }
    let value = args[0].dereference();
    let result = match &value {
        Value::Array(items) if mode == 1 => items.len() as i64 + deep_count(&args[0]),
        Value::Array(items) => items.len() as i64,
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            if !implements_countable(&class_name, classes, &mut HashSet::new()) {
                return Err(format!("TypeError:count(): Argument #1 ($value) must be of type Countable|array, {class_name} given"));
            }
            let callback = Value::array(vec![
                (ArrayKey::Integer(0), Value::Object(object.clone())),
                (ArrayKey::Integer(1), Value::String("count".into())),
            ]);
            let result = invoke(&callback, Vec::new())?;
            match result.dereference() {
                Value::Integer(count) => count,
                _ => return Err("TypeError:Countable::count() must return int".into()),
            }
        }
        _ => return Err(format!("TypeError:count(): Argument #1 ($value) must be of type Countable|array, {} given", value.type_name())),
    };
    Ok(Value::Integer(result))
}

pub(crate) fn is_countable(value: &Value, classes: &HashMap<String, BytecodeClass>) -> bool {
    match value.dereference() {
        Value::Array(_) => true,
        Value::Object(object) => implements_countable(&object.borrow().class_name, classes, &mut HashSet::new()),
        _ => false,
    }
}

fn implements_countable(class_name: &str, classes: &HashMap<String, BytecodeClass>, visited: &mut HashSet<String>) -> bool {
    let normalized = class_name.to_ascii_lowercase();
    if !visited.insert(normalized.clone()) { return false; }
    let Some(class) = classes.get(&normalized) else { return false; };
    class.interfaces.iter().any(|interface| {
        interface.eq_ignore_ascii_case("Countable") || implements_countable(interface, classes, visited)
    }) || class.parent.as_deref().is_some_and(|parent| implements_countable(parent, classes, visited))
}

/// Query array data with array_keys semantics.
pub(super) fn array_keys(args: &[Value]) -> Result<Value, String> {
    require_count("array_keys", args, 1, 3)?;
    let items = entries(&args[0], "array_keys")?;
    let filtered = args.len() >= 2;
    let strict = args.get(2).is_some_and(boolean);
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in items {
        if !filtered || equal(value, &args[1], strict) { append(&mut result, key_value(key)); }
    }
    Ok(array_value(result))
}

/// Query array data with array_values semantics.
pub(super) fn array_values(args: &[Value]) -> Result<Value, String> {
    require_count("array_values", args, 1, 1)?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (_, value) in entries(&args[0], "array_values")? { append(&mut result, value.clone()); }
    Ok(array_value(result))
}

/// Query array data with key_exists semantics.
pub(super) fn key_exists(args: &[Value]) -> Result<Value, String> {
    require_count("array_key_exists", args, 2, 2)?;
    let key = args[0].array_key()?;
    Ok(Value::Bool(entries(&args[1], "array_key_exists")?.iter().any(|(candidate, _)| *candidate == key)))
}

/// Query array data with array_is_list semantics.
pub(super) fn array_is_list(args: &[Value]) -> Result<Value, String> {
    require_count("array_is_list", args, 1, 1)?;
    let items = entries(&args[0], "array_is_list")?;
    Ok(Value::Bool(items.iter().enumerate().all(|(index, (key, _))| *key == ArrayKey::Integer(index as i64))))
}

/// Query array data with in_array semantics.
pub(super) fn in_array(args: &[Value]) -> Result<Value, String> {
    require_count("in_array", args, 2, 3)?;
    let strict = args.get(2).is_some_and(boolean);
    let found = entries(&args[1], "in_array")?.iter().any(|(_, value)| equal(value, &args[0], strict));
    Ok(Value::Bool(found))
}

/// Query array data with array_search semantics.
pub(super) fn array_search(args: &[Value]) -> Result<Value, String> {
    require_count("array_search", args, 2, 3)?;
    let strict = args.get(2).is_some_and(boolean);
    for (key, value) in entries(&args[1], "array_search")? {
        if equal(value, &args[0], strict) { return Ok(key_value(key)); }
    }
    Ok(Value::Bool(false))
}

/// Query array data with array_sum semantics.
pub(super) fn array_sum(args: &[Value]) -> Result<Value, String> {
    require_count("array_sum", args, 1, 1)?;
    let mut integer_sum = 0i64;
    let mut float_sum = 0.0;
    let mut is_integer = true;
    for (_, value) in entries(&args[0], "array_sum")? {
        match value.dereference() {
            Value::Integer(value) if is_integer => match integer_sum.checked_add(value) {
                Some(sum) => integer_sum = sum,
                None => { float_sum = integer_sum as f64 + value as f64; is_integer = false; }
            },
            value => {
                if is_integer { float_sum = integer_sum as f64; is_integer = false; }
                float_sum += value.number();
            }
        }
    }
    Ok(if is_integer { Value::Integer(integer_sum) } else { Value::Number(float_sum) })
}

/// Query array data with array_product semantics.
pub(super) fn array_product(args: &[Value]) -> Result<Value, String> {
    require_count("array_product", args, 1, 1)?;
    let mut integer_product = 1i64;
    let mut float_product = 1.0;
    let mut is_integer = true;
    for (_, value) in entries(&args[0], "array_product")? {
        match value.dereference() {
            Value::Integer(value) if is_integer => match integer_product.checked_mul(value) {
                Some(product) => integer_product = product,
                None => { float_product = integer_product as f64 * value as f64; is_integer = false; }
            },
            value => {
                if is_integer { float_product = integer_product as f64; is_integer = false; }
                float_product *= value.number();
            }
        }
    }
    Ok(if is_integer { Value::Integer(integer_product) } else { Value::Number(float_product) })
}

/// Query array data with array_edge_key semantics.
pub(super) fn array_edge_key(args: &[Value], last: bool) -> Result<Value, String> {
    let name = if last { "array_key_last" } else { "array_key_first" };
    require_count(name, args, 1, 1)?;
    let items = entries(&args[0], name)?;
    Ok(items.get(if last { items.len().saturating_sub(1) } else { 0 }).map(|(key, _)| key_value(key)).unwrap_or(Value::Null))
}

/// Query array data with array_edge_value semantics.
pub(super) fn array_edge_value(args: &[Value], last: bool) -> Result<Value, String> {
    let name = if last { "array_last" } else { "array_first" };
    require_count(name, args, 1, 1)?;
    let items = entries(&args[0], name)?;
    Ok(items.get(if last { items.len().saturating_sub(1) } else { 0 }).map(|(_, value)| value.clone()).unwrap_or(Value::Null))
}

