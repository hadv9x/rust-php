//! Array inspection and comparison functions.

use super::common::*;
use crate::{ArrayKey, BytecodeClass, DiagnosticLevel, RuntimeDiagnostic, Value};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Query array data with count semantics.
pub(super) fn count(
    args: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Result<Value, String> {
    require_count("count", args, 1, 2)?;
    /// Count nested array entries recursively for `count($array, COUNT_RECURSIVE)`.
    fn signature(value: &Value, output: &mut String) {
        match value {
            Value::Null => output.push('n'),
            Value::Bool(value) => output.push_str(if *value { "b1;" } else { "b0;" }),
            Value::Integer(value) => output.push_str(&format!("i{value};")),
            Value::Number(value) => output.push_str(&format!("f{:016x};", value.to_bits())),
            Value::String(value) => output.push_str(&format!("s{}:{value};", value.len())),
            Value::Reference(reference) => {
                output.push_str(&format!("r{:p};", Rc::as_ptr(reference)))
            }
            Value::Object(object) => output.push_str(&format!("o{:p};", Rc::as_ptr(object))),
            Value::Closure(closure) => output.push_str(&format!("c{:p};", Rc::as_ptr(closure))),
            Value::Generator(generator) => {
                output.push_str(&format!("g{:p};", Rc::as_ptr(generator)))
            }
            Value::Stream(stream) => output.push_str(&format!("t{:p};", Rc::as_ptr(stream))),
            Value::Array(array) => {
                output.push('[');
                for (key, value) in array.iter() {
                    match key {
                        ArrayKey::Integer(key) => output.push_str(&format!("k{key};")),
                        ArrayKey::String(key) => output.push_str(&format!("K{}:{key};", key.len())),
                    }
                    signature(value, output);
                }
                output.push(']');
            }
        }
    }
    fn deep_count(
        value: &Value,
        reference_edge: bool,
        ancestors: &mut Vec<String>,
        diagnostics: &mut Vec<RuntimeDiagnostic>,
        recursion_warned: &mut bool,
    ) -> i64 {
        let dereferenced = value.dereference();
        let Value::Array(items) = &dereferenced else {
            return 0;
        };
        let mut shape = String::new();
        signature(&dereferenced, &mut shape);
        if reference_edge && ancestors.contains(&shape) {
            if !*recursion_warned {
                diagnostics.push(RuntimeDiagnostic {
                    level: DiagnosticLevel::Warning,
                    message: "count(): Recursion detected".into(),
                    source_file: None,
                    source_line: None,
                    output_offset: 0,
                });
                *recursion_warned = true;
            }
            return 0;
        }
        ancestors.push(shape);
        let count = items.len() as i64
            + items
                .iter()
                .filter_map(|(_, child)| {
                    matches!(child.dereference(), Value::Array(_)).then(|| {
                        deep_count(
                            child,
                            matches!(child, Value::Reference(_)),
                            ancestors,
                            diagnostics,
                            recursion_warned,
                        )
                    })
                })
                .sum::<i64>();
        ancestors.pop();
        count
    }
    let mode = args.get(1).map_or(0, |value| value.number() as i64);
    if !matches!(mode, 0 | 1) {
        return Err("ValueError:count(): Argument #2 ($mode) must be either COUNT_NORMAL or COUNT_RECURSIVE".into());
    }
    let value = args[0].dereference();
    let result = match &value {
        Value::Array(_) if mode == 1 => {
            deep_count(&args[0], false, &mut Vec::new(), diagnostics, &mut false)
        }
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
        _ => {
            return Err(format!(
                "TypeError:count(): Argument #1 ($value) must be of type Countable|array, {} given",
                value.type_name()
            ))
        }
    };
    Ok(Value::Integer(result))
}

pub(crate) fn is_countable(value: &Value, classes: &HashMap<String, BytecodeClass>) -> bool {
    match value.dereference() {
        Value::Array(_) => true,
        Value::Object(object) => {
            implements_countable(&object.borrow().class_name, classes, &mut HashSet::new())
        }
        _ => false,
    }
}

fn implements_countable(
    class_name: &str,
    classes: &HashMap<String, BytecodeClass>,
    visited: &mut HashSet<String>,
) -> bool {
    let normalized = class_name.to_ascii_lowercase();
    if !visited.insert(normalized.clone()) {
        return false;
    }
    let Some(class) = classes.get(&normalized) else {
        return false;
    };
    class.interfaces.iter().any(|interface| {
        interface.eq_ignore_ascii_case("Countable")
            || implements_countable(interface, classes, visited)
    }) || class
        .parent
        .as_deref()
        .is_some_and(|parent| implements_countable(parent, classes, visited))
}

/// Query array data with array_keys semantics.
pub(super) fn array_keys(args: &[Value]) -> Result<Value, String> {
    require_count("array_keys", args, 1, 3)?;
    let items = entries(&args[0], "array_keys")?;
    let filtered = args.len() >= 2;
    let strict = args.get(2).is_some_and(boolean);
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in items {
        if !filtered || equal(value, &args[1], strict) {
            append(&mut result, key_value(key));
        }
    }
    Ok(array_value(result))
}

/// Query array data with array_values semantics.
pub(super) fn array_values(args: &[Value]) -> Result<Value, String> {
    require_count("array_values", args, 1, 1)?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (_, value) in entries(&args[0], "array_values")? {
        append(&mut result, value.clone());
    }
    Ok(array_value(result))
}

/// Query array data with key_exists semantics.
pub(super) fn key_exists(
    args: &[Value],
    diagnostics: &mut Vec<crate::RuntimeDiagnostic>,
) -> Result<Value, String> {
    require_count("array_key_exists", args, 2, 2)?;
    let key = array_key(&args[0], diagnostics)?;
    Ok(Value::Bool(
        entries(&args[1], "array_key_exists")?
            .iter()
            .any(|(candidate, _)| *candidate == key),
    ))
}

/// Query array data with array_is_list semantics.
pub(super) fn array_is_list(args: &[Value]) -> Result<Value, String> {
    require_count("array_is_list", args, 1, 1)?;
    let items = entries(&args[0], "array_is_list")?;
    Ok(Value::Bool(items.iter().enumerate().all(
        |(index, (key, _))| *key == ArrayKey::Integer(index as i64),
    )))
}

/// Query array data with in_array semantics.
pub(super) fn in_array(args: &[Value]) -> Result<Value, String> {
    require_count("in_array", args, 2, 3)?;
    let strict = args.get(2).is_some_and(boolean);
    let found = entries(&args[1], "in_array")?
        .iter()
        .any(|(_, value)| equal(value, &args[0], strict));
    Ok(Value::Bool(found))
}

/// Query array data with array_search semantics.
pub(super) fn array_search(args: &[Value]) -> Result<Value, String> {
    require_count("array_search", args, 2, 3)?;
    let strict = args.get(2).is_some_and(boolean);
    for (key, value) in entries(&args[1], "array_search")? {
        if equal(value, &args[0], strict) {
            return Ok(key_value(key));
        }
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
                None => {
                    float_sum = integer_sum as f64 + value as f64;
                    is_integer = false;
                }
            },
            value => {
                if is_integer {
                    float_sum = integer_sum as f64;
                    is_integer = false;
                }
                float_sum += value.number();
            }
        }
    }
    Ok(if is_integer {
        Value::Integer(integer_sum)
    } else {
        Value::Number(float_sum)
    })
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
                None => {
                    float_product = integer_product as f64 * value as f64;
                    is_integer = false;
                }
            },
            value => {
                if is_integer {
                    float_product = integer_product as f64;
                    is_integer = false;
                }
                float_product *= value.number();
            }
        }
    }
    Ok(if is_integer {
        Value::Integer(integer_product)
    } else {
        Value::Number(float_product)
    })
}

/// Query array data with array_edge_key semantics.
pub(super) fn array_edge_key(args: &[Value], last: bool) -> Result<Value, String> {
    let name = if last {
        "array_key_last"
    } else {
        "array_key_first"
    };
    require_count(name, args, 1, 1)?;
    let items = entries(&args[0], name)?;
    Ok(items
        .get(if last {
            items.len().saturating_sub(1)
        } else {
            0
        })
        .map(|(key, _)| key_value(key))
        .unwrap_or(Value::Null))
}

/// Query array data with array_edge_value semantics.
pub(super) fn array_edge_value(args: &[Value], last: bool) -> Result<Value, String> {
    let name = if last { "array_last" } else { "array_first" };
    require_count(name, args, 1, 1)?;
    let items = entries(&args[0], name)?;
    Ok(items
        .get(if last {
            items.len().saturating_sub(1)
        } else {
            0
        })
        .map(|(_, value)| value.clone())
        .unwrap_or(Value::Null))
}
