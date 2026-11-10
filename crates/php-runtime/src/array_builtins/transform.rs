//! Array merge, replace, slicing, and set-operation functions.

use super::common::*;
use crate::{ArrayKey, Value};

/// Merge arrays, appending numeric keys and replacing matching string keys.
pub(super) fn array_merge(args: &[Value]) -> Result<Value, String> {
    // PHP merge appends numeric-keyed values and overwrites matching string keys from the right.
    require_count("array_merge", args, 0, usize::MAX)?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for input in args {
        for (key, value) in entries(input, "array_merge")? {
            match key {
                ArrayKey::Integer(_) => append(&mut result, value.clone()),
                ArrayKey::String(_) => insert(&mut result, key.clone(), value.clone()),
            }
        }
    }
    Ok(array_value(result))
}

/// Merge colliding nested values recursively; scalar collisions become a two-item array.
pub(super) fn merge_recursive_values(left: Value, right: Value) -> Value {
    match (left, right) {
        (Value::Array(mut left), Value::Array(right)) => {
            for (key, value) in right {
                if let Some((_, previous)) = left.iter_mut().find(|(existing, _)| *existing == key)
                {
                    *previous = merge_recursive_values(previous.clone(), value);
                } else {
                    left.entries.push((key, value));
                }
            }
            Value::Array(left)
        }
        (left, right) => Value::array(vec![
            (ArrayKey::Integer(0), left),
            (ArrayKey::Integer(1), right),
        ]),
    }
}

/// Merge arrays while recursively combining duplicate string keys.
pub(super) fn array_merge_recursive(args: &[Value]) -> Result<Value, String> {
    require_count("array_merge_recursive", args, 0, usize::MAX)?;
    let mut result = Vec::new();
    for input in args {
        for (key, value) in entries(input, "array_merge_recursive")? {
            match key {
                ArrayKey::Integer(_) => append(&mut result, value.clone()),
                ArrayKey::String(_) => {
                    if let Some((_, previous)) =
                        result.iter_mut().find(|(existing, _)| existing == key)
                    {
                        *previous = merge_recursive_values(previous.clone(), value.clone());
                    } else {
                        result.push((key.clone(), value.clone()));
                    }
                }
            }
        }
    }
    Ok(Value::array(result))
}

/// Replace matching nested entries and preserve values absent from the replacement.
pub(super) fn replace_recursive(left: &mut Vec<(ArrayKey, Value)>, right: &[(ArrayKey, Value)]) {
    for (key, value) in right {
        if let Some((_, previous)) = left.iter_mut().find(|(existing, _)| existing == key) {
            if let (Value::Array(previous), Value::Array(replacement)) = (&mut *previous, value) {
                replace_recursive(&mut previous.entries, &replacement.entries);
            } else {
                *previous = value.clone();
            }
        } else {
            left.push((key.clone(), value.clone()));
        }
    }
}

/// Replace matching entries recursively across the input arrays.
pub(super) fn array_replace_recursive(args: &[Value]) -> Result<Value, String> {
    require_count("array_replace_recursive", args, 1, usize::MAX)?;
    let mut result = entries(&args[0], "array_replace_recursive")?.to_vec();
    for input in &args[1..] {
        replace_recursive(&mut result, entries(input, "array_replace_recursive")?);
    }
    Ok(Value::array(result))
}

/// Replace entries from left to right while retaining their keys.
pub(super) fn array_replace(args: &[Value]) -> Result<Value, String> {
    require_count("array_replace", args, 1, usize::MAX)?;
    let mut result = entries(&args[0], "array_replace")?.to_vec();
    for input in &args[1..] {
        for (key, value) in entries(input, "array_replace")? {
            insert(&mut result, key.clone(), value.clone());
        }
    }
    Ok(array_value(result))
}

/// Reverse entry order and optionally retain numeric keys.
pub(super) fn array_reverse(args: &[Value]) -> Result<Value, String> {
    require_count("array_reverse", args, 1, 2)?;
    let preserve = args.get(1).is_some_and(boolean);
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in entries(&args[0], "array_reverse")?.iter().rev() {
        if preserve || matches!(key, ArrayKey::String(_)) {
            insert(&mut result, key.clone(), value.clone());
        } else {
            append(&mut result, value.clone());
        }
    }
    Ok(array_value(result))
}

/// Remove later values equal to an earlier value while preserving its key.
pub(super) fn array_unique(
    args: &[Value],
    stringify: &mut dyn FnMut(&Value) -> Result<String, String>,
) -> Result<Value, String> {
    require_count("array_unique", args, 1, 2)?;
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    let flags = args.get(1).map(integer).unwrap_or(2);
    for (key, value) in entries(&args[0], "array_unique")? {
        let mut duplicate = false;
        for (_, existing) in &result {
            if super::sorting::compare_for_sort(existing, value, flags, stringify)?
                == std::cmp::Ordering::Equal
            {
                duplicate = true;
                break;
            }
        }
        if !duplicate {
            insert(&mut result, key.clone(), value.clone());
        }
    }
    Ok(array_value(result))
}

/// Return a portion of an array using offset and optional length.
pub(super) fn array_slice(args: &[Value]) -> Result<Value, String> {
    require_count("array_slice", args, 2, 4)?;
    let items = entries(&args[0], "array_slice")?;
    let len = items.len() as i64;
    let offset = integer(&args[1]);
    let start = if offset < 0 {
        (len + offset).max(0)
    } else {
        offset.min(len)
    } as usize;
    let end = match args.get(2) {
        None | Some(Value::Null) => len as usize,
        Some(value) if integer(value) < 0 => (len + integer(value)).max(start as i64) as usize,
        Some(value) => (start as i64 + integer(value)).min(len).max(start as i64) as usize,
    };
    let preserve = args.get(3).is_some_and(boolean);
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    for (key, value) in &items[start..end] {
        if preserve || matches!(key, ArrayKey::String(_)) {
            insert(&mut result, key.clone(), value.clone());
        } else {
            append(&mut result, value.clone());
        }
    }
    Ok(array_value(result))
}

/// Split an array into chunks of the requested size.
pub(super) fn array_chunk(args: &[Value]) -> Result<Value, String> {
    require_count("array_chunk", args, 2, 3)?;
    let items = entries(&args[0], "array_chunk")?;
    let size = integer(&args[1]);
    if size < 1 {
        return Err(
            "ValueError:array_chunk(): Argument #2 ($length) must be greater than 0".into(),
        );
    }
    let preserve = args.get(2).is_some_and(boolean);
    let mut chunks = Vec::new();
    for slice in items.chunks(size as usize) {
        let mut chunk = Vec::new();
        for (key, value) in slice {
            if preserve {
                insert(&mut chunk, key.clone(), value.clone());
            } else {
                append(&mut chunk, value.clone());
            }
        }
        append(&mut chunks, array_value(chunk));
    }
    Ok(array_value(chunks))
}

/// Pad an array to the requested size using the supplied value.
pub(super) fn array_pad(args: &[Value]) -> Result<Value, String> {
    require_count("array_pad", args, 3, 3)?;
    let source = entries(&args[0], "array_pad")?;
    let target = integer(&args[1]);
    if target.unsigned_abs() > 1_000_000 {
        return Err("array_pad(): requested size is too large".into());
    }
    let amount = target.unsigned_abs() as usize;
    if amount <= source.len() {
        return Ok(args[0].clone());
    }
    let count = amount - source.len();
    let mut result: Vec<(ArrayKey, Value)> = Vec::new();
    if target < 0 {
        for _ in 0..count {
            append(&mut result, args[2].clone());
        }
    }
    for (key, value) in source {
        match key {
            ArrayKey::Integer(_) => append(&mut result, value.clone()),
            ArrayKey::String(_) => insert(&mut result, key.clone(), value.clone()),
        }
    }
    if target >= 0 {
        for _ in 0..count {
            append(&mut result, args[2].clone());
        }
    }
    Ok(array_value(result))
}

#[derive(Clone, Copy)]
/// The value/key comparison dimensions for array diff and intersection.
pub(super) enum DiffMode {
    Values,
    ValuesAndKeys,
    Keys,
}

/// Compute array difference or intersection with the selected key/value comparison mode.
pub(super) fn array_diff(
    args: &[Value],
    mode: DiffMode,
    intersect: bool,
    stringify: &mut dyn FnMut(&Value) -> Result<String, String>,
) -> Result<Value, String> {
    // All diff/intersect variants preserve keys from the first input array.
    let name = match (mode, intersect) {
        (DiffMode::Values, false) => "array_diff",
        (DiffMode::Values, true) => "array_intersect",
        (DiffMode::ValuesAndKeys, false) => "array_diff_assoc",
        (DiffMode::ValuesAndKeys, true) => "array_intersect_assoc",
        (DiffMode::Keys, false) => "array_diff_key",
        (DiffMode::Keys, true) => "array_intersect_key",
    };
    require_count(name, args, 2, usize::MAX)?;
    let arrays = args
        .iter()
        .map(|value| entries(value, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = Vec::new();
    for (key, value) in arrays[0] {
        let mut found_in_all = true;
        for array in &arrays[1..] {
            let mut found = false;
            for (candidate_key, candidate_value) in *array {
                let values_match = match mode {
                    DiffMode::Values | DiffMode::ValuesAndKeys => {
                        stringify(value)? == stringify(candidate_value)?
                    }
                    DiffMode::Keys => true,
                };
                let keys_match = match mode {
                    DiffMode::Values => true,
                    DiffMode::ValuesAndKeys | DiffMode::Keys => key == candidate_key,
                };
                if values_match && keys_match {
                    found = true;
                    break;
                }
            }
            if !found {
                found_in_all = false;
                break;
            }
        }
        if found_in_all == intersect {
            insert(&mut result, key.clone(), value.clone());
        }
    }
    Ok(array_value(result))
}

/// Compute the callback-based PHP diff/intersection variants.
pub(super) fn array_diff_with_callbacks(
    name: &str,
    args: &[Value],
    intersect: bool,
    ignore_values: bool,
    compare_values: bool,
    compare_keys: bool,
    exact_keys: bool,
    invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>,
    stringify: &mut dyn FnMut(&Value) -> Result<String, String>,
) -> Result<Value, String> {
    let callback_count = usize::from(compare_values) + usize::from(compare_keys);
    require_count(name, args, 2 + callback_count, usize::MAX)?;
    let arrays_count = args.len() - callback_count;
    let arrays = args[..arrays_count]
        .iter()
        .map(|value| entries(value, name))
        .collect::<Result<Vec<_>, _>>()?;
    let value_callback = compare_values.then(|| &args[arrays_count]);
    let key_callback = compare_keys.then(|| &args[arrays_count + usize::from(compare_values)]);
    if let Some(callback) = value_callback {
        super::callbacks::callback_name(callback, name)?;
    }
    if let Some(callback) = key_callback {
        super::callbacks::callback_name(callback, name)?;
    }

    let mut result = Vec::new();
    for (key, value) in arrays[0] {
        let mut found_in_all = true;
        for array in &arrays[1..] {
            let mut found = false;
            for (candidate_key, candidate_value) in *array {
                let value_matches = if ignore_values {
                    true
                } else if let Some(callback) = value_callback {
                    invoke(callback, vec![value.clone(), candidate_value.clone()])?.number() == 0.0
                } else {
                    stringify(value)? == stringify(candidate_value)?
                };
                if !value_matches {
                    continue;
                }
                let key_matches = if let Some(callback) = key_callback {
                    invoke(callback, vec![key_value(key), key_value(candidate_key)])?.number()
                        == 0.0
                } else if exact_keys {
                    key == candidate_key
                } else {
                    true
                };
                if key_matches {
                    found = true;
                    break;
                }
            }
            if !found {
                found_in_all = false;
                break;
            }
        }
        if found_in_all == intersect {
            insert(&mut result, key.clone(), value.clone());
        }
    }
    Ok(array_value(result))
}
