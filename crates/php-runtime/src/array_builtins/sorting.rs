//! Array ordering and sorting builtins.

use crate::{ArrayKey, Value};
use std::ffi::CString;
use super::common::*;

#[derive(Clone, Copy)]
/// The ordering and key-preservation policy used by array sorting.
pub(super) enum SortMode { ValuesAscending, ValuesDescending, ValuesAscendingPreserveKeys, ValuesDescendingPreserveKeys, KeysAscending, KeysDescending }

/// Compare strings in natural order with optional case-insensitive matching.
pub(super) fn natural_order(left: &str, right: &str, insensitive: bool) -> std::cmp::Ordering {
    let left = if insensitive { ascii_case_fold(left) } else { left.as_bytes().to_vec() };
    let right = if insensitive { ascii_case_fold(right) } else { right.as_bytes().to_vec() };
    let (a, b) = (left.as_slice(), right.as_slice());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (start_i, start_j) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() { i += 1; }
            while j < b.len() && b[j].is_ascii_digit() { j += 1; }
            // Ignore leading zeroes for numeric magnitude, then use run length as a tie-break.
            let mut digits_i = &a[start_i..i];
            let mut digits_j = &b[start_j..j];
            while digits_i.first() == Some(&b'0') && digits_i.len() > 1 { digits_i = &digits_i[1..]; }
            while digits_j.first() == Some(&b'0') && digits_j.len() > 1 { digits_j = &digits_j[1..]; }
            let order = digits_i.len().cmp(&digits_j.len()).then_with(|| digits_i.cmp(digits_j));
            if order != std::cmp::Ordering::Equal { return order; }
        } else {
            let order = a[i].cmp(&b[j]);
            if order != std::cmp::Ordering::Equal { return order; }
            i += 1;
            j += 1;
        }
    }
    a.len().cmp(&b.len())
}

/// PHP's case-insensitive sort flag folds ASCII letters only (PHP 8.2+).
fn ascii_case_fold(value: &str) -> Vec<u8> {
    value.bytes().map(|byte| if byte.is_ascii_uppercase() { byte + (b'a' - b'A') } else { byte }).collect()
}

/// Sort an array using natural string ordering.
pub(super) fn natural_sort(args: &[Value], insensitive: bool, stringify: &mut dyn FnMut(&Value) -> Result<String, String>) -> Result<(Value, Option<(usize, Value)>), String> {
    let name = if insensitive { "natcasesort" } else { "natsort" };
    let mut items = mutable_array(name, args, 1, 1)?;
    let mut compare_error = None;
    items.sort_by(|a, b| {
        let left = stringify(&a.1);
        let right = stringify(&b.1);
        match (left, right) {
            (Ok(left), Ok(right)) => natural_order(&left, &right, insensitive),
            (Err(error), _) | (_, Err(error)) => { compare_error = Some(error); std::cmp::Ordering::Equal }
        }
    });
    if let Some(error) = compare_error { return Err(error); }
    Ok((Value::Bool(true), Some((0, Value::array(items)))))
}

/// Compare values using the requested PHP sort flag.
struct CompareState {
    compared_objects: std::collections::HashSet<(usize, usize)>,
    compared_references: std::collections::HashSet<(usize, usize)>,
    recursive: bool,
}

pub(super) fn compare_values(left: &Value, right: &Value, flags: i64) -> Result<std::cmp::Ordering, String> {
    let mut state = CompareState { compared_objects: std::collections::HashSet::new(), compared_references: std::collections::HashSet::new(), recursive: false };
    let ordering = compare_regular(left, right, flags, &mut state, 0);
    if state.recursive { Err("Error:Nesting level too deep - recursive dependency?".into()) } else { Ok(ordering) }
}

pub(super) fn compare_for_sort(left: &Value, right: &Value, flags: i64, stringify: &mut dyn FnMut(&Value) -> Result<String, String>) -> Result<std::cmp::Ordering, String> {
    match flags & 7 {
        2 => {
            let left = stringify(left)?;
            let right = stringify(right)?;
            Ok(if flags & 8 != 0 { ascii_case_fold(&left).cmp(&ascii_case_fold(&right)) } else { left.cmp(&right) })
        }
        5 => Ok(locale_string_order(&stringify(left)?, &stringify(right)?)),
        6 => Ok(natural_order(&stringify(left)?, &stringify(right)?, flags & 8 != 0)),
        _ => compare_values(left, right, flags),
    }
}

/// Compare NUL-free strings using the process's active LC_COLLATE locale.
/// PHP's locale sort mode delegates to the active C locale; retain byte order
/// for embedded-NUL strings because `strcoll` accepts NUL-terminated inputs.
fn locale_string_order(left: &str, right: &str) -> std::cmp::Ordering {
    let (Ok(left_c), Ok(right_c)) = (CString::new(left), CString::new(right)) else {
        return left.as_bytes().cmp(right.as_bytes());
    };
    let comparison = unsafe { libc::strcoll(left_c.as_ptr(), right_c.as_ptr()) };
    comparison.cmp(&0)
}

fn compare_regular(left: &Value, right: &Value, flags: i64, state: &mut CompareState, depth: usize) -> std::cmp::Ordering {
    match flags & 7 {
        1 => left.number().partial_cmp(&right.number()).unwrap_or(std::cmp::Ordering::Equal),
        2 if flags & 8 != 0 => ascii_case_fold(&left.show()).cmp(&ascii_case_fold(&right.show())),
        2 => left.show().cmp(&right.show()),
        5 => locale_string_order(&left.show(), &right.show()),
        6 if flags & 8 != 0 => natural_order(&left.show(), &right.show(), true),
        6 => natural_order(&left.show(), &right.show(), false),
        _ => {
            if let (Value::Reference(a), Value::Reference(b)) = (left, right) {
                let identity = (std::rc::Rc::as_ptr(a) as usize, std::rc::Rc::as_ptr(b) as usize);
                if !state.compared_references.insert(identity) { state.recursive = true; return std::cmp::Ordering::Equal; }
                let ordering = compare_regular(&a.borrow(), &b.borrow(), flags, state, depth + 1);
                state.compared_references.remove(&identity);
                return ordering;
            }
            let left = left.dereference();
            let right = right.dereference();
            match (&left, &right) {
            (Value::Bool(_) | Value::Null, _) | (_, Value::Bool(_) | Value::Null) => left.truthy().cmp(&right.truthy()),
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Integer(a), Value::Number(b)) => (*a as f64).partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Number(a), Value::Integer(b)) => a.partial_cmp(&(*b as f64)).unwrap_or(std::cmp::Ordering::Equal),
            (Value::Number(a), Value::Number(b)) => a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal),
            (Value::String(a), Value::String(b)) => match (numeric_string(a), numeric_string(b)) {
                (Some(a), Some(b)) => compare_numeric_strings(a, b),
                _ => a.cmp(b),
            },
            (Value::Integer(a), Value::String(b)) => numeric_string(b).map_or_else(|| a.to_string().cmp(b), |b| compare_integer_to_numeric_string(*a, b)),
            (Value::String(a), Value::Integer(b)) => numeric_string(a).map_or_else(|| a.cmp(&b.to_string()), |a| compare_integer_to_numeric_string(*b, a).reverse()),
            (Value::Number(a), Value::String(b)) => numeric_string(b).map_or_else(|| a.to_string().cmp(b), |b| compare_float_to_numeric_string(*a, b)),
            (Value::String(a), Value::Number(b)) => numeric_string(a).map_or_else(|| a.cmp(&b.to_string()), |a| compare_float_to_numeric_string(*b, a).reverse()),
            (Value::Array(a), Value::Array(b)) => {
                if depth >= 512 { state.recursive = true; return std::cmp::Ordering::Equal; }
                let mut result = a.len().cmp(&b.len());
                if result == std::cmp::Ordering::Equal {
                    for (key, value) in a.iter() {
                        let Some((_, other)) = b.iter().find(|(other_key, _)| other_key == key) else {
                            return std::cmp::Ordering::Greater;
                        };
                        result = compare_regular(value, other, flags, state, depth + 1);
                        if result != std::cmp::Ordering::Equal { break; }
                    }
                }
                result
            }
            (Value::Object(a), Value::Object(b)) => {
                if std::rc::Rc::ptr_eq(a, b) { return std::cmp::Ordering::Equal; }
                let (left_class, left_properties, left_order) = { let object = a.borrow(); (object.class_name.to_ascii_lowercase(), object.properties.clone(), object.property_order.clone()) };
                let (right_class, right_properties, right_order) = { let object = b.borrow(); (object.class_name.to_ascii_lowercase(), object.properties.clone(), object.property_order.clone()) };
                // PHP treats objects from unrelated classes as incomparable. For stable
                // builtin sorts and extrema, keep their encounter order instead of
                // inventing a lexical order from implementation class names.
                if left_class != right_class { return std::cmp::Ordering::Equal; }
                let identity = (std::rc::Rc::as_ptr(a) as usize, std::rc::Rc::as_ptr(b) as usize);
                if !state.compared_objects.insert(identity) { state.recursive = true; return std::cmp::Ordering::Equal; }
                let ordered_keys = |properties: &std::collections::HashMap<String, Value>, order: &[String]| {
                    let mut keys = order.iter().filter(|key| properties.contains_key(*key)).cloned().collect::<Vec<_>>();
                    let mut remaining = properties.keys().filter(|key| !keys.contains(key)).cloned().collect::<Vec<_>>();
                    remaining.sort();
                    keys.extend(remaining);
                    keys
                };
                let left_keys = ordered_keys(&left_properties, &left_order);
                let right_keys = ordered_keys(&right_properties, &right_order);
                let mut result = left_keys.len().cmp(&right_keys.len());
                if result == std::cmp::Ordering::Equal {
                    for left_key in &left_keys {
                        let Some(right_value) = right_properties.get(left_key) else {
                            result = std::cmp::Ordering::Greater;
                            break;
                        };
                        result = compare_regular(&left_properties[left_key], right_value, flags, state, depth + 1);
                        if result != std::cmp::Ordering::Equal { break; }
                    }
                }
                state.compared_objects.remove(&identity);
                result
            }
            (Value::Object(_), _) => std::cmp::Ordering::Greater,
            (_, Value::Object(_)) => std::cmp::Ordering::Less,
            (Value::Array(_), _) => std::cmp::Ordering::Greater,
            (_, Value::Array(_)) => std::cmp::Ordering::Less,
            _ => left.show().cmp(&right.show()),
            }
        }
    }
}

#[derive(Clone, Copy)]
enum NumericString { Integer(i64), Number(f64) }

fn numeric_string(value: &str) -> Option<NumericString> {
    // PHP numeric strings accept ASCII whitespace around the number, not the
    // broader Unicode whitespace set used by Rust's str::trim().
    let value = value.trim_matches(|character| matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{000c}' | '\u{000b}'));
    if value.is_empty() { return None; }
    if let Ok(integer) = value.parse::<i64>() { return Some(NumericString::Integer(integer)); }
    value.parse::<f64>().ok().filter(|number| number.is_finite()).map(NumericString::Number)
}

fn compare_numeric_strings(left: NumericString, right: NumericString) -> std::cmp::Ordering {
    match (left, right) {
        (NumericString::Integer(left), NumericString::Integer(right)) => left.cmp(&right),
        (NumericString::Integer(left), NumericString::Number(right)) => (left as f64).partial_cmp(&right).unwrap_or(std::cmp::Ordering::Equal),
        (NumericString::Number(left), NumericString::Integer(right)) => left.partial_cmp(&(right as f64)).unwrap_or(std::cmp::Ordering::Equal),
        (NumericString::Number(left), NumericString::Number(right)) => left.partial_cmp(&right).unwrap_or(std::cmp::Ordering::Equal),
    }
}

fn compare_integer_to_numeric_string(integer: i64, value: NumericString) -> std::cmp::Ordering {
    compare_numeric_strings(NumericString::Integer(integer), value)
}

fn compare_float_to_numeric_string(number: f64, value: NumericString) -> std::cmp::Ordering {
    compare_numeric_strings(NumericString::Number(number), value)
}

/// Sort one or more arrays by lexicographic columns and return every updated input.
pub(super) fn multisort(args: &[Value], stringify: &mut dyn FnMut(&Value) -> Result<String, String>) -> Result<(Value, Vec<(usize, Value)>), String> {
    require_count("array_multisort", args, 1, usize::MAX)?;
    let mut columns = Vec::<(usize, Vec<(ArrayKey, Value)>, bool, i64)>::new();
    let mut cursor = 0;
    while cursor < args.len() {
        let Value::Array(array) = &args[cursor] else {
            return Err(format!("TypeError:array_multisort(): Argument #{} must be an array or a sort flag", cursor + 1));
        };
        let argument_index = cursor;
        let items = array.iter().cloned().collect::<Vec<_>>();
        cursor += 1;
        let mut descending = false;
        let mut direction_seen = false;
        let mut flags = 0_i64;
        let mut flags_seen = false;
        while let Some(argument) = args.get(cursor) {
            if !matches!(argument, Value::Integer(_) | Value::Number(_)) { break; }
            let flag = argument.number() as i64;
            if matches!(flag, 3 | 4) && direction_seen {
                return Err(format!("TypeError:array_multisort(): Argument #{} has the same sort order as a previous argument", cursor + 1));
            } else if matches!(flag, 3 | 4) {
                descending = flag == 3;
                direction_seen = true;
            } else if matches!(flag, 0 | 1 | 2 | 5 | 6 | 10 | 14) && flags_seen {
                return Err(format!("TypeError:array_multisort(): Argument #{} has the same sort flag as a previous argument", cursor + 1));
            } else if matches!(flag, 0 | 1 | 2 | 5 | 6 | 10 | 14) {
                flags = flag;
                flags_seen = true;
            } else {
                return Err(format!("ValueError:array_multisort(): Argument #{} contains an unknown sort flag", cursor + 1));
            }
            cursor += 1;
        }
        columns.push((argument_index, items, descending, flags));
    }
    let row_count = columns[0].1.len();
    if columns.iter().any(|(_, values, _, _)| values.len() != row_count) {
        return Err("ValueError:array_multisort(): Array sizes are inconsistent".into());
    }

    let mut order = (0..row_count).collect::<Vec<_>>();
    for index in 1..order.len() {
        let mut current = index;
        while current > 0 {
            let mut comparison = std::cmp::Ordering::Equal;
            for (_, values, descending, flags) in &columns {
                comparison = compare_for_sort(&values[order[current - 1]].1, &values[order[current]].1, *flags, stringify)?;
                if *descending { comparison = comparison.reverse(); }
                if comparison != std::cmp::Ordering::Equal { break; }
            }
            if comparison != std::cmp::Ordering::Greater { break; }
            order.swap(current - 1, current);
            current -= 1;
        }
    }

    let mutations = columns.into_iter().map(|(argument_index, values, _, _)| {
        let mut sorted = Vec::with_capacity(row_count);
        for (new_index, old_index) in order.iter().copied().enumerate() {
            let (key, value) = &values[old_index];
            let key = match key {
                ArrayKey::String(_) => key.clone(),
                ArrayKey::Integer(_) => ArrayKey::Integer(new_index as i64),
            };
            sorted.push((key, value.clone()));
        }
        (argument_index, Value::array(sorted))
    }).collect();
    Ok((Value::Bool(true), mutations))
}

/// Sort values or keys and apply the selected key-preservation policy.
pub(super) fn sort(args: &[Value], mode: SortMode, stringify: &mut dyn FnMut(&Value) -> Result<String, String>) -> Result<(Value, Option<(usize, Value)>), String> {
    // Preserve keys only for asort/arsort; sort/rsort intentionally rebuild packed numeric keys.
    let name = match mode {
        SortMode::ValuesAscending => "sort", SortMode::ValuesDescending => "rsort",
        SortMode::ValuesAscendingPreserveKeys => "asort", SortMode::ValuesDescendingPreserveKeys => "arsort",
        SortMode::KeysAscending => "ksort", SortMode::KeysDescending => "krsort",
    };
    let mut items = mutable_array(name, args, 1, 2)?;
    let flags = args.get(1).map(integer).unwrap_or(0);
    let descending = matches!(mode, SortMode::ValuesDescending | SortMode::ValuesDescendingPreserveKeys | SortMode::KeysDescending);
    let mut compare_error = None;
    if matches!(mode, SortMode::KeysAscending | SortMode::KeysDescending) {
        items.sort_by(|a,b| {
            let ordering = match compare_for_sort(&key_value(&a.0), &key_value(&b.0), flags, stringify) { Ok(ordering) => ordering, Err(error) => { compare_error = Some(error); std::cmp::Ordering::Equal } };
            if descending { ordering.reverse() } else { ordering }
        });
    } else {
        items.sort_by(|a,b| {
            let ordering = match compare_for_sort(&a.1, &b.1, flags, stringify) { Ok(ordering) => ordering, Err(error) => { compare_error = Some(error); std::cmp::Ordering::Equal } };
            if descending { ordering.reverse() } else { ordering }
        });
    }
    if let Some(error) = compare_error { return Err(error); }
    if matches!(mode, SortMode::ValuesAscending | SortMode::ValuesDescending) {
        let values = items.into_iter().map(|(_, value)| value).collect::<Vec<_>>();
        items = values.into_iter().map(|value| (ArrayKey::Integer(0), value)).collect();
        for (index, (key, _)) in items.iter_mut().enumerate() { *key = ArrayKey::Integer(index as i64); }
    }
    Ok((Value::Bool(true), Some((0, Value::array(items)))))
}

/// Sort by a PHP comparator, preserving keys for `uasort`/`uksort` and rebuilding them for `usort`.
pub(super) fn user_sort(
    name: &str,
    args: &[Value],
    compare_keys: bool,
    preserve_keys: bool,
    invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>,
) -> Result<(Value, Option<(usize, Value)>), String> {
    require_count(name, args, 2, 2)?;
    let callback = super::callbacks::callback_name(&args[1], name)?;
    let mut items = entries(&args[0], name)?.to_vec();

    // Use a fallible stable insertion sort because the comparator can execute PHP and fail.
    for index in 1..items.len() {
        let mut current = index;
        while current > 0 {
            let left = if compare_keys { key_value(&items[current - 1].0) } else { items[current - 1].1.clone() };
            let right = if compare_keys { key_value(&items[current].0) } else { items[current].1.clone() };
            if invoke(callback, vec![left, right])?.number() <= 0.0 { break; }
            items.swap(current - 1, current);
            current -= 1;
        }
    }

    if !preserve_keys {
        for (index, (key, _)) in items.iter_mut().enumerate() {
            *key = ArrayKey::Integer(index as i64);
        }
    }
    Ok((Value::Bool(true), Some((0, Value::array(items)))))
}

#[cfg(target_os = "linux")]
#[test]
fn locale_string_order_uses_thread_local_lc_collate() {
    let locale_name = CString::new("en_US.utf8").unwrap();
    let locale = unsafe { libc::newlocale(libc::LC_COLLATE_MASK, locale_name.as_ptr(), std::ptr::null_mut()) };
    if locale.is_null() { return; }
    let previous = unsafe { libc::uselocale(locale) };
    assert_eq!(locale_string_order("z", "ä"), std::cmp::Ordering::Greater);
    unsafe {
        libc::uselocale(previous);
        libc::freelocale(locale);
    }
}

