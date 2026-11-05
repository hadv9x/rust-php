//! Array mutation, insertion, and removal builtins.

use crate::{ArrayKey, Value};
use super::common::*;

/// Remove the last entry and return its value plus updated input for variable writeback.
pub(super) fn array_pop(args: &[Value]) -> Result<(Value, Option<(usize, Value)>), String> {
    let mut items = mutable_array("array_pop", args, 1, 1)?;
    // PHP returns a value copy here, even when the removed slot is a reference.
    let result = items.pop().map(|(_, value)| value.dereference()).unwrap_or(Value::Null);
    Ok((result, Some((0, Value::array(items)))))
}

/// Randomize entries and return the updated input array for variable writeback.
pub(super) fn shuffle(args: &[Value]) -> Result<(Value, Option<(usize, Value)>), String> {
    let mut entropy = std::fs::File::open("/dev/urandom")
        .map_err(|error| format!("shuffle(): secure random source failed: {error}"))?;
    let mut items = mutable_array("shuffle", args, 1, 1)?;
    for i in (1..items.len()).rev() {
        let j = random_index(&mut entropy, i + 1)?;
        items.swap(i, j);
    }
    let values = items.into_iter().map(|(_, value)| value).collect::<Vec<_>>();
    let shuffled = values.into_iter().enumerate().map(|(i, value)| (ArrayKey::Integer(i as i64), value)).collect();
    Ok((Value::Bool(true), Some((0, Value::array(shuffled)))))
}

/// Choose one or more random keys from an array.
pub(super) fn array_rand(args: &[Value]) -> Result<Value, String> {
    require_count("array_rand", args, 1, 2)?;
    let items = entries(&args[0], "array_rand").map_err(|_| format!("TypeError:array_rand(): Argument #1 ($array) must be of type array, {} given", args[0].type_name()))?;
    let count = args.get(1).map(array_rand_count).transpose()?.unwrap_or(1);
    if count < 1 || count as usize > items.len() { return Err("ValueError:array_rand(): Argument #2 ($num) must be between 1 and the number of elements in the array".into()); }
    let mut entropy = std::fs::File::open("/dev/urandom")
        .map_err(|error| format!("array_rand(): secure random source failed: {error}"))?;
    let mut indices = (0..items.len()).collect::<Vec<_>>();
    for i in 0..count as usize {
        let j = i + random_index(&mut entropy, indices.len() - i)?;
        indices.swap(i, j);
    }
    if count == 1 { return Ok(key_value(&items[indices[0]].0)); }
    let mut keys = Vec::new();
    for index in indices.into_iter().take(count as usize) { append(&mut keys, key_value(&items[index].0)); }
    Ok(Value::array(keys))
}

fn array_rand_count(value: &Value) -> Result<i64, String> {
    match value.dereference() {
        Value::Integer(number) => Ok(number),
        Value::Number(number) if number.is_finite() => Ok(number.trunc() as i64),
        Value::String(text) => text.trim().parse::<i64>().or_else(|_| text.trim().parse::<f64>().map(|number| number.trunc() as i64))
            .map_err(|_| format!("TypeError:array_rand(): Argument #2 ($num) must be of type int, string given")),
        Value::Bool(value) => Ok(i64::from(value)),
        Value::Null => Ok(0),
        value => Err(format!("TypeError:array_rand(): Argument #2 ($num) must be of type int, {} given", value.type_name())),
    }
}

/// Draw an unbiased index using rejection sampling over OS-provided entropy.
fn random_index(random: &mut impl std::io::Read, upper_exclusive: usize) -> Result<usize, String> {
    let upper = u64::try_from(upper_exclusive).map_err(|_| "random array bound is too large".to_string())?;
    if upper == 0 { return Err("random array bound must be positive".into()); }
    let reject_below = upper.wrapping_neg() % upper;
    loop {
        let mut bytes = [0_u8; 8];
        random.read_exact(&mut bytes).map_err(|error| format!("secure random source failed: {error}"))?;
        let sample = u64::from_ne_bytes(bytes);
        if sample >= reject_below { return Ok((sample % upper) as usize); }
    }
}

#[cfg(test)]
mod random_tests {
    use super::random_index;
    use std::io::Cursor;

    #[test]
    fn random_index_rejects_the_biased_prefix() {
        let mut samples = Vec::new();
        samples.extend_from_slice(&0_u64.to_ne_bytes()); // rejected for an upper bound of ten
        samples.extend_from_slice(&17_u64.to_ne_bytes()); // maps to seven
        assert_eq!(random_index(&mut Cursor::new(samples), 10).unwrap(), 7);
    }

    #[test]
    fn random_index_rejects_zero_bounds() {
        assert!(random_index(&mut Cursor::new(Vec::<u8>::new()), 0).is_err());
    }
}

/// Remove a range, optionally insert replacement values, and return removed entries.
pub(super) fn array_splice(args: &[Value]) -> Result<(Value, Option<(usize, Value)>), String> {
    // Replacement reindexes the remaining numeric keys, while string keys stay attached to items.
    let mut source = mutable_array("array_splice", args, 2, 4)?;
    let len = source.len() as i64;
    let offset = integer(&args[1]);
    let start = if offset < 0 { (len + offset).max(0) } else { offset.min(len) } as usize;
    let remove_count = match args.get(2) {
        None | Some(Value::Null) => source.len() - start,
        Some(value) if integer(value) < 0 => (len - start as i64 + integer(value)).max(0) as usize,
        Some(value) => (integer(value) as usize).min(source.len() - start),
    };
    let removed = source.drain(start..start + remove_count).collect::<Vec<_>>();
    let replacement = args.get(3).map(splice_replacement_values).unwrap_or_default();
    let mut insertion = Vec::new();
    for value in replacement { append(&mut insertion, value); }
    source.splice(start..start, insertion);
    let mut reindexed = Vec::new();
    let mut next = 0_i64;
    for (key, value) in source {
        match key {
            ArrayKey::Integer(_) => { insert(&mut reindexed, ArrayKey::Integer(next), value); next += 1; }
            ArrayKey::String(_) => insert(&mut reindexed, key, value),
        }
    }
    let mut removed_result = Vec::new();
    for (key, value) in removed {
        match key {
            ArrayKey::Integer(_) => append(&mut removed_result, value),
            ArrayKey::String(_) => insert(&mut removed_result, key, value),
        }
    }
    Ok((Value::array(removed_result), Some((0, Value::array(reindexed)))))
}

fn splice_replacement_values(value: &Value) -> Vec<Value> {
    match value.dereference() {
        Value::Null => Vec::new(),
        Value::Array(items) => items.iter().map(|(_, value)| value.clone()).collect(),
        Value::Object(object) => {
            let object = object.borrow();
            let mut names = object.property_order.clone();
            let mut remaining = object.properties.keys().filter(|name| !names.contains(name)).cloned().collect::<Vec<_>>();
            remaining.sort();
            names.extend(remaining);
            names.into_iter().filter_map(|name| object.properties.get(&name).cloned()).collect()
        }
        Value::Closure(_) | Value::Generator(_) => Vec::new(),
        value => vec![value],
    }
}

/// Remove the first entry and reindex remaining numeric keys.
pub(super) fn array_shift(args: &[Value]) -> Result<(Value, Option<(usize, Value)>), String> {
    let mut items = mutable_array("array_shift", args, 1, 1)?;
    // Like array_pop(), array_shift() does not return the slot's reference cell.
    let result = if items.is_empty() { Value::Null } else { items.remove(0).1.dereference() };
    let mut next_index = 0_i64;
    for (key, _) in &mut items {
        if matches!(key, ArrayKey::Integer(_)) {
            *key = ArrayKey::Integer(next_index);
            next_index += 1;
        }
    }
    Ok((result, Some((0, Value::array(items)))))
}

/// Append values and return the changed array for variable writeback.
pub(super) fn array_push(args: &[Value]) -> Result<(Value, Option<(usize, Value)>), String> {
    let mut items = mutable_array("array_push", args, 2, usize::MAX)?;
    for value in &args[1..] { append(&mut items, value.clone()); }
    let length = items.len();
    Ok((Value::Integer(length as i64), Some((0, Value::array(items)))))
}

/// Prepend values and return the changed array for variable writeback.
pub(super) fn array_unshift(args: &[Value]) -> Result<(Value, Option<(usize, Value)>), String> {
    let mut items = mutable_array("array_unshift", args, 2, usize::MAX)?;
    let mut result = Vec::new();
    for value in &args[1..] { append(&mut result, value.clone()); }
    let mut next_index = result.len() as i64;
    for (key, value) in items.drain(..) {
        match key {
            ArrayKey::Integer(_) => insert(&mut result, ArrayKey::Integer(next_index), value),
            ArrayKey::String(_) => insert(&mut result, key, value),
        }
        if let Some((ArrayKey::Integer(index), _)) = result.last() { next_index = index + 1; }
    }
    let length = result.len();
    Ok((Value::Integer(length as i64), Some((0, Value::array(result)))))
}

