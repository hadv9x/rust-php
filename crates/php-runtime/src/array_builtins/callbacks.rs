//! Array builtins that invoke user-provided callbacks.

use crate::{ArrayKey, Value};
use super::common::*;

/// Visit array values and write callback modifications back through the first argument.
pub(super) fn array_walk(
    args: &[Value],
    invoke: &mut dyn FnMut(&Value, Value, Value, Option<Value>) -> Result<Value, String>,
) -> Result<(Value, Option<(usize, Value)>), String> {
    require_count("array_walk", args, 2, 3)?;
    let callback = callback_name(&args[1], "array_walk")?;
    let user_data = args.get(2).cloned();
    let mut result = entries(&args[0], "array_walk")?.to_vec();
    for (key, value) in &mut result {
        *value = invoke(callback, value.clone(), key_value(key), user_data.clone())?;
    }
    Ok((Value::Bool(true), Some((0, array_value(result)))))
}

/// Recursively visit leaf values and write callback modifications back through the input array.
pub(super) fn array_walk_recursive(
    args: &[Value],
    invoke: &mut dyn FnMut(&Value, Value, Value, Option<Value>) -> Result<Value, String>,
) -> Result<(Value, Option<(usize, Value)>), String> {
    require_count("array_walk_recursive", args, 2, 3)?;
    let callback = callback_name(&args[1], "array_walk_recursive")?;
    let user_data = args.get(2).cloned();
    let mut result = entries(&args[0], "array_walk_recursive")?.to_vec();
    walk_recursive_entries(&mut result, callback, user_data, invoke)?;
    Ok((Value::Bool(true), Some((0, array_value(result)))))
}

fn walk_recursive_entries(
    entries: &mut [(ArrayKey, Value)],
    callback: &Value,
    user_data: Option<Value>,
    invoke: &mut dyn FnMut(&Value, Value, Value, Option<Value>) -> Result<Value, String>,
) -> Result<(), String> {
    for (key, value) in entries {
        match value {
            Value::Array(nested) => {
                walk_recursive_entries(nested, callback, user_data.clone(), invoke)?;
            }
            Value::Reference(reference) => {
                let current = reference.borrow().clone();
                if let Value::Array(mut nested) = current {
                    walk_recursive_entries(&mut nested, callback, user_data.clone(), invoke)?;
                    *reference.borrow_mut() = Value::Array(nested);
                } else {
                    let updated = invoke(callback, current, key_value(key), user_data.clone())?;
                    *reference.borrow_mut() = updated;
                }
            }
            _ => {
                *value = invoke(callback, value.clone(), key_value(key), user_data.clone())?;
            }
        }
    }
    Ok(())
}

/// Dispatch a supported builtin by its string callback name.
pub(super) fn call_callback_builtin(name: &str, arguments: Vec<Value>) -> Result<Value, String> {
    let normalized_name = name.to_ascii_lowercase();
    if matches!(normalized_name.as_str(), "str_replace" | "str_ireplace") {
        return crate::string_builtins::replace(name, &arguments).map(|(value, _)| value);
    }
    if crate::string_builtins::is_supported(&normalized_name) {
        return crate::string_builtins::call(name, &arguments);
    }
    if crate::math_builtins::is_supported(&normalized_name) {
        return crate::math_builtins::call(name, &arguments);
    }
    let value = arguments.first().cloned().unwrap_or(Value::Null);
    let result = match name.to_ascii_lowercase().as_str() {
        "intval" => {
            let base = arguments.get(1).map_or(10, |value| value.number() as u32);
            if base != 0 && !(2..=36).contains(&base) {
                return Err("ValueError:intval(): Argument #2 ($base) must be between 2 and 36".into());
            }
            let integer = match &value {
                Value::String(text) => crate::vm::intval_string(text, base),
                _ => value.number().trunc() as i64,
            };
            Value::Integer(integer)
        }
        "floatval" => Value::Number(value.number()),
        "strval" => Value::String(value.show()),
        "boolval" => Value::Bool(value.truthy()),
        "gettype" => Value::String(match value {
            Value::Null => "NULL",
            Value::Bool(_) => "boolean",
            Value::Integer(_) => "integer",
            Value::Number(_) => "double",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Stream(_) => "resource",
            Value::Object(_) | Value::Closure(_) | Value::Generator(_) => "object",
            Value::Reference(_) => unreachable!(),
        }.into()),
        "get_debug_type" => Value::String(match &value {
            Value::Null => "null".into(),
            Value::Bool(_) => "bool".into(),
            Value::Integer(_) => "int".into(),
            Value::Number(_) => "float".into(),
            Value::String(_) => "string".into(),
            Value::Array(_) => "array".into(),
            Value::Object(object) => object.borrow().class_name.clone(),
            Value::Closure(_) => "Closure".into(),
            Value::Generator(_) => "Generator".into(),
            Value::Stream(_) => "resource (stream)".into(),
            Value::Reference(_) => unreachable!(),
        }),
        "is_string" => Value::Bool(matches!(value, Value::String(_))),
        "is_int" | "is_integer" => Value::Bool(matches!(value, Value::Integer(_))),
        "is_long" => Value::Bool(matches!(value, Value::Integer(_))),
        "is_float" | "is_double" | "is_real" => Value::Bool(matches!(value, Value::Number(_))),
        "is_array" => Value::Bool(matches!(value, Value::Array(_))),
        "is_bool" => Value::Bool(matches!(value, Value::Bool(_))),
        "is_null" => Value::Bool(matches!(value, Value::Null)),
        "is_object" => Value::Bool(matches!(value, Value::Object(_) | Value::Closure(_) | Value::Generator(_))),
        "is_resource" => Value::Bool(matches!(&value, Value::Stream(stream) if stream.borrow().is_open())),
        "is_scalar" => Value::Bool(matches!(value, Value::Bool(_) | Value::Integer(_) | Value::Number(_) | Value::String(_))),
        "is_numeric" => Value::Bool(matches!(value, Value::Integer(_) | Value::Number(_)) || matches!(value, Value::String(s) if {
            let text = s.trim();
            !text.is_empty() && !matches!(text.to_ascii_lowercase().as_str(), "nan" | "+nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity") && text.parse::<f64>().is_ok()
        })),
        "sum" | "add" => Value::Number(value.number() + arguments.get(1).cloned().unwrap_or(Value::Null).number()),
        "multiply" => Value::Number(value.number() * arguments.get(1).cloned().unwrap_or(Value::Null).number()),
        "concat" => Value::String(value.show() + &arguments.get(1).cloned().unwrap_or(Value::Null).show()),
        "min" => { let other = arguments.get(1).cloned().unwrap_or(Value::Null); if value.number() <= other.number() { value } else { other } },
        "max" => { let other = arguments.get(1).cloned().unwrap_or(Value::Null); if value.number() >= other.number() { value } else { other } },
        _ => return Err(format!("Unsupported array callback: {name}")),
    };
    Ok(result)
}

/// Keep entries for which the callback returns a truthy value.
pub(super) fn array_filter(args: &[Value], invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>) -> Result<Value, String> {
    // Filtering preserves original keys; a missing callback uses PHP-like truthiness.
    require_count("array_filter", args, 1, 3)?;
    let items = entries(&args[0], "array_filter")?;
    let callback = match args.get(1) {
        None | Some(Value::Null) => None,
        Some(Value::String(_) | Value::Closure(_) | Value::Array(_) | Value::Object(_)) => args.get(1),
        _ => return Err("array_filter(): callback must be a valid callback or null".into()),
    };
    let mode = args.get(2).map(integer).unwrap_or(0);
    if callback.is_some() && !matches!(mode, 0..=2) {
        return Err("array_filter(): Argument #3 ($mode) must be 0, ARRAY_FILTER_USE_KEY, or ARRAY_FILTER_USE_BOTH".into());
    }
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in items {
        let keep = match callback {
            None => value.truthy(),
            Some(callback) => {
                let params = match mode {
                    1 => vec![value.clone(), key_value(key)],
                    2 => vec![key_value(key)],
                    _ => vec![value.clone()],
                };
                invoke(callback, params)?.truthy()
            }
        };
        if keep { insert(&mut result, key.clone(), value.clone()); }
    }
    Ok(array_value(result))
}

/// Apply a callback to each corresponding value from the input arrays.
pub(super) fn array_map(args: &[Value], invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>) -> Result<Value, String> {
    // The callback receives corresponding values from each input array at each numeric position.
    require_count("array_map", args, 2, usize::MAX)?;
    let callback = match &args[0] {
        Value::Null => None,
        Value::String(_) | Value::Closure(_) | Value::Array(_) | Value::Object(_) => Some(&args[0]),
        _ => return Err("array_map(): callback must be a valid callback or null".into()),
    };
    let inputs = args[1..].iter().map(|value| entries(value, "array_map")).collect::<Result<Vec<_>, _>>()?;
    let length = inputs.iter().map(|array| array.len()).max().unwrap_or(0);
    if callback.is_none() && inputs.len() == 1 { return Ok(args[1].clone()); }

    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for index in 0..length {
        let values = inputs.iter().map(|array| array.get(index).map(|(_, value)| value.clone()).unwrap_or(Value::Null)).collect::<Vec<_>>();
        let output = match callback {
            Some(callback) => invoke(callback, values)?,
            None => Value::array(values.into_iter().enumerate().map(|(i, value)| (ArrayKey::Integer(i as i64), value)).collect()),
        };
        if inputs.len() == 1 {
            if let Some((key, _)) = inputs[0].get(index) {
                insert(&mut result, key.clone(), output);
                continue;
            }
        }
        append(&mut result, output);
    }
    Ok(array_value(result))
}

/// Fold the entries into one accumulator by repeatedly invoking the callback.
pub(super) fn array_reduce(args: &[Value], invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>) -> Result<Value, String> {
    // Fold in insertion order, passing the accumulator before the current item to the callback.
    require_count("array_reduce", args, 2, 3)?;
    let callback = callback_name(&args[1], "array_reduce")?;
    let mut accumulator = args.get(2).cloned().unwrap_or(Value::Null);
    for (_, value) in entries(&args[0], "array_reduce")? {
        accumulator = invoke(callback, vec![accumulator, value.clone()])?;
    }
    Ok(accumulator)
}

/// The all/any reduction performed by callback array functions.
pub(super) enum Predicate { All, Any }

/// Validate and extract the supported string callback name.
pub(super) fn callback_name<'a>(value: &'a Value, function: &str) -> Result<&'a Value, String> {
    match value {
        Value::String(_) | Value::Closure(_) | Value::Array(_) | Value::Object(_) => Ok(value),
        _ => Err(format!("{function}(): callback must be a valid callback")),
    }
}

/// Evaluate array_all or array_any, stopping when its result is known.
pub(super) fn array_predicate(args: &[Value], predicate: Predicate, invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>) -> Result<Value, String> {
    // Short-circuit as soon as the requested all/any condition is determined.
    let name = match predicate { Predicate::All => "array_all", Predicate::Any => "array_any" };
    require_count(name, args, 2, 2)?;
    let callback_name = callback_name(&args[1], name)?;
    let items = entries(&args[0], name)?;
    for (key, value) in items {
        let answer = invoke(callback_name, vec![value.clone(), key_value(key)])?.truthy();
        if matches!(predicate, Predicate::All) && !answer { return Ok(Value::Bool(false)); }
        if matches!(predicate, Predicate::Any) && answer { return Ok(Value::Bool(true)); }
    }
    Ok(Value::Bool(matches!(predicate, Predicate::All)))
}

/// Return the first callback-matching value or its key.
pub(super) fn array_find(args: &[Value], return_key: bool, invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>) -> Result<Value, String> {
    let name = if return_key { "array_find_key" } else { "array_find" };
    require_count(name, args, 2, 2)?;
    let callback_name = callback_name(&args[1], name)?;
    for (key, value) in entries(&args[0], name)? {
        if invoke(callback_name, vec![value.clone(), key_value(key)])?.truthy() {
            return Ok(if return_key { key_value(key) } else { value.clone() });
        }
    }
    Ok(Value::Null)
}

