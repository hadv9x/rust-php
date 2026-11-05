//! Shared conversion and validation helpers for array builtins.

use crate::{ArrayKey, DiagnosticLevel, RuntimeDiagnostic, Value};

/// Apply PHP's array-key conversion and report precision loss for fractional floats.
pub(super) fn array_key(value: &Value, diagnostics: &mut Vec<RuntimeDiagnostic>) -> Result<ArrayKey, String> {
    if let Value::Number(number) = value.dereference() {
        if number.is_nan() {
            diagnostics.push(RuntimeDiagnostic {
                level: DiagnosticLevel::Warning,
                message: "unexpected NAN value was coerced to int".into(),
                source_file: None, source_line: None, output_offset: 0,
            });
        } else if !number.is_finite() || number >= 9_223_372_036_854_775_808.0 || number < -9_223_372_036_854_775_808.0 {
            diagnostics.push(RuntimeDiagnostic {
                level: DiagnosticLevel::Warning,
                message: crate::array_builtins::non_representable_float_to_int_message(number),
                source_file: None, source_line: None, output_offset: 0,
            });
        } else if number.fract() != 0.0 {
            diagnostics.push(RuntimeDiagnostic {
                level: DiagnosticLevel::Deprecated,
                message: format!("Implicit conversion from float {number} to int loses precision"),
                source_file: None, source_line: None, output_offset: 0,
            });
        }
    }
    value.array_key()
}

/// Return a mutable array copy after validating the first argument and arity.
pub(super) fn mutable_array(name: &str, args: &[Value], min: usize, max: usize) -> Result<Vec<(ArrayKey, Value)>, String> {
    require_count(name, args, min, max)?;
    Ok(entries(&args[0], name)?.to_vec())
}

/// Validate that a builtin call has an allowed number of arguments.
pub(super) fn require_count(name: &str, args: &[Value], min: usize, max: usize) -> Result<(), String> {
    if args.len() < min || args.len() > max {
        let expected = if min == max { min.to_string() } else { format!("{min} to {max}") };
        return Err(format!("{name}() expects {expected} arguments, {} given", args.len()));
    }
    Ok(())
}

/// Return an array value as ordered key/value entries or report the PHP function error.
pub(super) fn entries<'a>(value: &'a Value, function: &str) -> Result<&'a [(ArrayKey, Value)], String> {
    match value {
        Value::Array(entries) => Ok(entries),
        _ => Err(format!("{function}() expects parameter 1 to be an array, {} given", value.type_name())),
    }
}

/// Convert a runtime value to the integer form used by array builtins.
pub(super) fn integer(value: &Value) -> i64 { value.number() as i64 }

/// Convert a runtime value using PHP-like truthiness.
pub(super) fn boolean(value: &Value) -> bool { value.truthy() }

/// Convert an internal array key back into a PHP runtime value.
pub(super) fn key_value(key: &ArrayKey) -> Value {
    match key {
        ArrayKey::Integer(value) => Value::Integer(*value),
        ArrayKey::String(value) => Value::String(value.clone()),
    }
}

/// Insert or replace a key while preserving the array entry order.
pub(super) fn insert(array: &mut Vec<(ArrayKey, Value)>, key: ArrayKey, value: Value) {
    if let Some((_, previous)) = array.iter_mut().find(|(existing, _)| *existing == key) {
        *previous = value;
    } else {
        array.push((key, value));
    }
}

/// Append a value using the next integer key in the internal array representation.
pub(super) fn append(array: &mut Vec<(ArrayKey, Value)>, value: Value) {
    let next = array.iter().filter_map(|(key, _)| match key {
        ArrayKey::Integer(index) => Some(*index),
        ArrayKey::String(_) => None,
    }).filter(|index| *index >= 0).max().and_then(|index| index.checked_add(1)).unwrap_or(0);
    insert(array, ArrayKey::Integer(next), value);
}

/// Compare two runtime values without type coercion.
pub(super) fn strict_equal(left: &Value, right: &Value) -> bool {
    let left = left.dereference();
    let right = right.dereference();
    match (&left, &right) {
        (Value::Integer(a), Value::Integer(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Null, Value::Null) => true,
        (Value::Array(a), Value::Array(b)) => a.len() == b.len() && a.iter().all(|(key, value)| {
            b.iter().find(|(other_key, _)| other_key == key).is_some_and(|(_, other)| strict_equal(value, other))
        }),
        _ => false,
    }
}

/// Compare runtime values using strict or PHP-like loose comparison.
pub(super) fn equal(left: &Value, right: &Value, strict: bool) -> bool {
    if strict { return strict_equal(left, right); }
    let left = left.dereference();
    let right = right.dereference();
    match (&left, &right) {
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Array(_), Value::Array(_)) => strict_equal(&left, &right),
        _ => left.number() == right.number(),
    }
}

/// Build a PHP array runtime value from ordered entries.
pub(super) fn array_value(items: Vec<(ArrayKey, Value)>) -> Value { Value::array(items) }

