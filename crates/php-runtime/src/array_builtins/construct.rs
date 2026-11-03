//! Array construction and key/value conversion functions.

use crate::{ArrayKey, BytecodeClass, DiagnosticLevel, MemberVisibility, RuntimeDiagnostic, Value};
use std::collections::HashMap;
use super::common::*;

/// Extract a selected column from each row, optionally indexing by another column.
pub(super) fn array_column(args: &[Value], classes: &HashMap<String, BytecodeClass>) -> Result<Value, String> {
    // Extract a field from each row, optionally using a second field as the result key.
    require_count("array_column", args, 2, 3)?;
    let rows = entries(&args[0], "array_column")?;
    let column_key = match &args[1] {
        Value::Null => None,
        key => Some(key.array_key()?),
    };
    let index_key = match args.get(2) {
        None | Some(Value::Null) => None,
        Some(key) => Some(key.array_key()?),
    };
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (_, row) in rows {
        let row = row.dereference();
        let (fields, is_object) = match &row {
            Value::Array(fields) => (fields.iter().map(|(key, value)| (key.clone(), value.clone())).collect::<Vec<_>>(), false),
            Value::Object(object) => {
                let object = object.borrow();
                let fields = object.properties.iter()
                    .filter(|(name, _)| !name.starts_with('\0') && object_property_is_public(&object.class_name, name, classes))
                    .map(|(name, value)| (ArrayKey::String(name.clone()), value.clone()))
                    .collect::<Vec<_>>();
                (fields, true)
            }
            _ => continue,
        };
        let column = match &column_key {
            None => row.clone(),
            Some(key) => match row_field(&fields, key, is_object) {
                Some((_, value)) => value.clone(),
                None => continue,
            },
        };
        let output_key = if let Some(key) = &index_key {
            match row_field(&fields, key, is_object) {
                Some((_, value)) => Some(value.array_key()?),
                None => None,
            }
        } else {
            None
        };
        match output_key {
            Some(key) => insert(&mut result, key, column),
            None => append(&mut result, column),
        }
    }
    Ok(array_value(result))
}

fn object_property_is_public(class_name: &str, property: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    let Some(class) = classes.get(&class_name.to_ascii_lowercase()) else { return true; };
    if let Some(metadata) = class.properties.get(property) {
        return metadata.visibility == MemberVisibility::Public;
    }
    if let Some(parent) = &class.parent {
        return object_property_is_public(parent, property, classes);
    }
    true // Dynamic properties are public.
}

fn row_field<'a>(fields: &'a [(ArrayKey, Value)], key: &ArrayKey, is_object: bool) -> Option<(&'a ArrayKey, &'a Value)> {
    fields.iter().find(|(candidate, _)| {
        candidate == key || (is_object && matches!((candidate, key), (ArrayKey::String(name), ArrayKey::Integer(index)) if name == &index.to_string()))
    }).map(|(candidate, value)| (candidate, value))
}

/// Pair entries from key and value arrays to build a new array.
pub(super) fn array_combine(args: &[Value]) -> Result<Value, String> {
    require_count("array_combine", args, 2, 2)?;
    let keys = entries(&args[0], "array_combine")?;
    let values = entries(&args[1], "array_combine")?;
    if keys.len() != values.len() { return Err("array_combine(): arrays must have the same number of elements".into()); }
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for ((_, key), (_, value)) in keys.iter().zip(values) { insert(&mut result, key.array_key()?, value.clone()); }
    Ok(array_value(result))
}

/// Swap values and keys where each value is a valid array key.
pub(super) fn array_flip(args: &[Value]) -> Result<Value, String> {
    require_count("array_flip", args, 1, 1)?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in entries(&args[0], "array_flip")? {
        if matches!(value, Value::String(_) | Value::Integer(_) | Value::Number(_)) {
            insert(&mut result, value.array_key()?, key_value(key));
        }
    }
    Ok(array_value(result))
}

/// Change string keys to the requested case.
pub(super) fn array_change_key_case(args: &[Value]) -> Result<Value, String> {
    require_count("array_change_key_case", args, 1, 2)?;
    let upper = args.get(1).is_some_and(|mode| integer(mode) == 1);
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in entries(&args[0], "array_change_key_case")? {
        let key = match key {
            ArrayKey::Integer(_) => key.clone(),
            ArrayKey::String(text) if upper => ArrayKey::String(text.to_uppercase()),
            ArrayKey::String(text) => ArrayKey::String(text.to_lowercase()),
        };
        insert(&mut result, key, value.clone());
    }
    Ok(array_value(result))
}

/// Count occurrences of each supported scalar value.
pub(super) fn array_count_values(args: &[Value]) -> Result<Value, String> {
    require_count("array_count_values", args, 1, 1)?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (_, value) in entries(&args[0], "array_count_values")? {
        if matches!(value, Value::String(_) | Value::Integer(_) | Value::Number(_)) {
            let key = value.array_key()?;
            let count = result.iter().find(|(candidate, _)| *candidate == key).map(|(_, count): &(_, Value)| count.number()).unwrap_or(0.0);
            insert(&mut result, key, Value::Integer(count as i64 + 1));
        }
    }
    Ok(array_value(result))
}

/// Create an array with consecutive integer keys and one repeated value.
pub(super) fn array_fill(args: &[Value]) -> Result<Value, String> {
    require_count("array_fill", args, 3, 3)?;
    let start = integer(&args[0]);
    let count = integer(&args[1]);
    if count < 0 { return Err("array_fill(): num must be greater than or equal to 0".into()); }
    if count > 1_000_000 { return Err("array_fill(): requested size is too large".into()); }
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for i in 0..count { insert(&mut result, ArrayKey::Integer(start + i), args[2].clone()); }
    Ok(array_value(result))
}

/// Create an array using each supplied key and one repeated value.
pub(super) fn array_fill_keys(args: &[Value]) -> Result<Value, String> {
    require_count("array_fill_keys", args, 2, 2)?;
    let keys = entries(&args[0], "array_fill_keys")?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (_, key) in keys { insert(&mut result, key.array_key()?, args[1].clone()); }
    Ok(array_value(result))
}

/// Build an inclusive ascending or descending numeric or single-byte character range.
pub(super) fn range(args: &[Value], diagnostics: &mut Vec<RuntimeDiagnostic>) -> Result<Value, String> {
    require_count("range", args, 2, 3)?;
    for (index, value) in args.iter().take(2).enumerate() {
        if !matches!(value.dereference(), Value::String(_) | Value::Integer(_) | Value::Number(_)) {
            return Err(format!("range(): Argument #{} (${}) must be of type string|int|float, {} given", index + 1, if index == 0 { "start" } else { "end" }, range_type_name(value)));
        }
    }
    let default_step = Value::Integer(1);
    let step = args.get(2).unwrap_or(&default_step);
    if !matches!(step.dereference(), Value::Integer(_) | Value::Number(_)) {
        return Err(format!("range(): Argument #3 ($step) must be of type int|float, {} given", range_type_name(step)));
    }
    let step_number = range_float(step);
    if !step_number.is_finite() { return Err("ValueError:range(): Argument #3 ($step) must be finite".into()); }
    if step_number == 0.0 { return Err("ValueError:range(): Argument #3 ($step) must not be 0".into()); }
    for (index, value) in args.iter().take(2).enumerate() {
        let Value::String(text) = value.dereference() else { continue; };
        let parameter = if index == 0 { "start" } else { "end" };
        if text.is_empty() {
            diagnostics.push(RuntimeDiagnostic { level: DiagnosticLevel::Warning, message: format!("range(): Argument #{} (${parameter}) must not be empty, casted to 0", index + 1), source_file: None, source_line: None });
            continue;
        }
        if range_numeric_string(&text).is_none() {
            let other_is_string = matches!(args[1 - index].dereference(), Value::String(_));
            let message = if other_is_string && text.len() > 1 {
                format!("range(): Argument #{} (${parameter}) must be a single byte, subsequent bytes are ignored", index + 1)
            } else if !other_is_string {
                format!("range(): Argument #{} (${parameter}) must be numeric when the other boundary is numeric", index + 1)
            } else {
                continue;
            };
            diagnostics.push(RuntimeDiagnostic { level: DiagnosticLevel::Warning, message, source_file: None, source_line: None });
        }
    }
    if matches!(step.dereference(), Value::Number(_))
        && args.iter().take(2).any(|value| matches!(value.dereference(), Value::String(text) if range_numeric_string(&text).is_none())) {
        diagnostics.push(RuntimeDiagnostic { level: DiagnosticLevel::Warning, message: "range(): Argument #3 ($step) must be of type int when generating an array of characters".into(), source_file: None, source_line: None });
    }
    let step_integer = match step.dereference() {
        Value::Integer(value) => Some(value),
        Value::Number(value) if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 => Some(value as i64),
        Value::String(value) => value.trim().parse::<i64>().ok().or_else(|| value.trim().parse::<f64>().ok()
            .filter(|number| number.is_finite() && number.fract() == 0.0 && *number >= i64::MIN as f64 && *number <= i64::MAX as f64)
            .map(|number| number as i64)),
        _ => None,
    };
    let (start_string, end_string) = match (&args[0], &args[1]) {
        (Value::String(start), Value::String(end)) => (Some(start.as_str()), Some(end.as_str())),
        _ => (None, None),
    };
    let start_numeric = start_string.and_then(range_numeric_string);
    let end_numeric = end_string.and_then(range_numeric_string);
    let float_numeric_string = |value: Option<(i64, bool)>| value.is_some_and(|(_, is_float)| is_float);
    let use_float = matches!((&args[0], &args[1]), (Value::Number(_), _) | (_, Value::Number(_)))
        || float_numeric_string(start_numeric) || float_numeric_string(end_numeric)
        || step_integer.is_none();

    // PHP 8.3 treats a pair of non-numeric strings as a sequence of bytes. Numeric strings
    // remain numeric, while a mixed numeric/non-numeric pair uses the string byte sequence.
    if let (Some(start), Some(end), Some(integer_step)) = (start_string, end_string, step_integer) {
        if !use_float && (start_numeric.is_none() || end_numeric.is_none()) && !start.is_empty() && !end.is_empty() {
            let start_byte = range_byte(start)?;
            let end_byte = range_byte(end)?;
            let descending = start_byte > end_byte;
            if integer_step < 0 && !descending { return Err("ValueError:range(): Argument #3 ($step) must be greater than 0 for increasing ranges".into()); }
            let step = integer_step.unsigned_abs();
            let distance = u64::from(start_byte.abs_diff(end_byte));
            let count = distance / step as u64 + 1;
            if count > 1_000_000 { return Err("range(): requested range is too large".into()); }
            let direction = if descending { -1i64 } else { 1i64 };
            let values = (0..count).map(|index| {
                let byte = i128::from(start_byte) + i128::from(direction) * i128::from(step) * index as i128;
                (ArrayKey::Integer(index as i64), Value::String((byte as u8 as char).to_string()))
            }).collect();
            return Ok(array_value(values));
        }
    }

    if use_float {
        let start = range_float(&args[0]);
        let end = range_float(&args[1]);
        if !start.is_finite() || !end.is_finite() { return Err("ValueError:range(): Arguments #1 ($start) and #2 ($end) must be finite".into()); }
        let descending = start > end;
        if step_number < 0.0 && !descending { return Err("ValueError:range(): Argument #3 ($step) must be greater than 0 for increasing ranges".into()); }
        let step = step_number.abs();
        let count = ((end - start).abs() / step).floor() as usize + 1;
        if count > 1_000_000 { return Err("range(): requested range is too large".into()); }
        let direction = if descending { -1.0 } else { 1.0 };
        let values = (0..count).map(|index| (ArrayKey::Integer(index as i64), Value::Number(start + direction * step * index as f64))).collect();
        return Ok(array_value(values));
    }

    let start = range_integer(&args[0]);
    let end = range_integer(&args[1]);
    let integer_step = step_integer.unwrap_or(step_number as i64);
    let descending = start > end;
    if integer_step < 0 && !descending { return Err("ValueError:range(): Argument #3 ($step) must be greater than 0 for increasing ranges".into()); }
    let step = integer_step.unsigned_abs();
    let distance = (i128::from(end) - i128::from(start)).unsigned_abs();
    let count = distance / step as u128 + 1;
    if count > 1_000_000 { return Err("range(): requested range is too large".into()); }
    let direction = if descending { -1i128 } else { 1i128 };
    let values = (0..count as usize).map(|index| {
        let value = i128::from(start) + direction * i128::from(step) * index as i128;
        (ArrayKey::Integer(index as i64), Value::Integer(value as i64))
    }).collect();
    Ok(array_value(values))
}

fn range_type_name(value: &Value) -> &'static str {
    match value.dereference() {
        Value::Null => "null", Value::Bool(_) => "bool", Value::Integer(_) => "int",
        Value::Number(_) => "float", Value::String(_) => "string", Value::Array(_) => "array",
        Value::Object(_) | Value::Closure(_) | Value::Generator(_) => "object", Value::Stream(_) => "resource",
        Value::Reference(_) => "unknown",
    }
}

fn range_numeric_string(value: &str) -> Option<(i64, bool)> {
    let value = value.trim();
    if value.is_empty() { return None; }
    value.parse::<i64>().ok().map(|number| (number, false))
        .or_else(|| value.parse::<f64>().ok().filter(|number| number.is_finite()).map(|number| (number as i64, true)))
}

fn range_byte(value: &str) -> Result<u8, String> {
    let byte = value.as_bytes().first().copied().ok_or("range(): empty string endpoint is not supported without PHP diagnostics")?;
    if !byte.is_ascii() { return Err("range(): non-ASCII byte strings are not represented by this runtime".into()); }
    Ok(byte)
}

fn range_float(value: &Value) -> f64 {
    match value.dereference() {
        Value::String(text) => text.trim().parse::<f64>().unwrap_or_else(|_| {
            let text = text.trim();
            (1..=text.len()).rev().find_map(|length| text.get(..length)?.parse::<f64>().ok()).unwrap_or(0.0)
        }),
        value => value.number(),
    }
}

fn range_integer(value: &Value) -> i64 {
    match value.dereference() {
        Value::Integer(value) => value,
        Value::Number(value) => value as i64,
        Value::String(text) => range_float(&Value::String(text)).trunc() as i64,
        Value::Bool(value) => i64::from(value),
        Value::Null => 0,
        _ => value.number() as i64,
    }
}

