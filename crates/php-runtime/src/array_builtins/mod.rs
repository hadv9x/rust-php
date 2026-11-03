//! PHP array builtins, grouped by operation family.

mod construct;
mod query;
mod transform;
mod callbacks;
mod common;
mod cursor;
mod mutation;
mod sorting;

use crate::{BytecodeClass, RuntimeDiagnostic, Value};
use std::collections::HashMap;

pub(crate) fn natural_string_order(left: &str, right: &str, insensitive: bool) -> std::cmp::Ordering {
    sorting::natural_order(left, right, insensitive)
}

/// Compare values using PHP's regular array-sort comparison rules.
pub(crate) fn regular_value_order(left: &Value, right: &Value) -> Result<std::cmp::Ordering, String> {
    sorting::compare_values(left, right, 0)
}

/// Test the runtime's public array-function inventory without invoking a function.
pub(crate) const fn supported_names() -> &'static [&'static str] {
    &[
        "array_all", "array_any", "array_change_key_case", "array_chunk", "array_column", "array_combine",
        "array_count_values", "array_diff", "array_diff_assoc", "array_diff_key", "array_diff_uassoc",
        "array_diff_ukey", "array_fill", "array_fill_keys", "array_filter", "array_find", "array_find_key",
        "array_first", "array_flip", "array_intersect", "array_intersect_assoc", "array_intersect_key",
        "array_intersect_uassoc", "array_intersect_ukey", "array_is_list", "array_key_exists", "array_key_first",
        "array_key_last", "array_keys", "array_last", "array_map", "array_merge", "array_merge_recursive",
        "array_multisort", "array_pad", "array_pop", "array_product", "array_push", "array_rand", "array_reduce",
        "array_replace", "array_replace_recursive", "array_reverse", "array_search", "array_shift", "array_slice",
        "array_splice", "array_sum", "array_udiff", "array_udiff_assoc", "array_udiff_uassoc", "array_udiff_ukey",
        "array_uintersect", "array_uintersect_assoc", "array_uintersect_uassoc", "array_uintersect_ukey",
        "array_unshift", "array_unique", "array_values", "arsort", "asort", "count", "current", "each", "end",
        "in_array", "iterator_count", "iterator_to_array", "key", "key_exists", "krsort", "ksort", "natcasesort", "next", "natsort", "pos", "prev", "range",
        "reset", "rsort", "shuffle", "sizeof", "sort", "uasort", "uksort", "usort", "array_walk",
        "array_walk_recursive",
    ]
}

pub(crate) fn is_supported(name: &str) -> bool {
    supported_names().iter().any(|function| function.eq_ignore_ascii_case(name))
}

pub(crate) fn is_countable(value: &Value, classes: &HashMap<String, BytecodeClass>) -> bool {
    query::is_countable(value, classes)
}

/// Dispatch supported array functions and return by-reference mutations separately.
/// The VM writes each updated array back to its originating PHP variable.
pub(crate) fn call(
    name: &str,
    args: Vec<Value>,
    classes: &HashMap<String, BytecodeClass>,
    invoke: &mut dyn FnMut(&Value, Vec<Value>) -> Result<Value, String>,
    invoke_walk: &mut dyn FnMut(&Value, Value, Value, Option<Value>) -> Result<Value, String>,
    stringify: &mut dyn FnMut(&Value) -> Result<String, String>,
    diagnostics: &mut Vec<RuntimeDiagnostic>,
) -> Result<(Value, Vec<(usize, Value)>), String> {
    if matches!(name, "current" | "pos" | "key") { return cursor::call(if name == "pos" { "current" } else { name }, &args); }
    if matches!(name, "next" | "prev" | "reset" | "end" | "each") { return cursor::call(name, &args); }
    if name == "array_walk" {
        let (value, mutation) = callbacks::array_walk(&args, invoke_walk)?;
        return Ok((value, mutation.into_iter().collect()));
    }
    if name == "array_walk_recursive" {
        let (value, mutation) = callbacks::array_walk_recursive(&args, invoke_walk)?;
        return Ok((value, mutation.into_iter().collect()));
    }
    if matches!(name, "usort" | "uasort" | "uksort") {
        let (value, mutation) = sorting::user_sort(name, &args, name == "uksort", name != "usort", invoke)?;
        return Ok((value, mutation.into_iter().collect()));
    }
    if name == "array_multisort" {
        return sorting::multisort(&args);
    }
    if name == "array_unique" {
        return Ok((transform::array_unique(&args, stringify)?, Vec::new()));
    }
    let callback_diff = match name {
        "array_diff_uassoc" => Some((false, false, false, true, false)),
        "array_diff_ukey" => Some((false, true, false, true, false)),
        "array_intersect_uassoc" => Some((true, false, false, true, false)),
        "array_intersect_ukey" => Some((true, true, false, true, false)),
        "array_udiff" | "array_uintersect" => Some((name == "array_uintersect", false, true, false, false)),
        "array_udiff_assoc" | "array_uintersect_assoc" => Some((name == "array_uintersect_assoc", false, true, false, true)),
        "array_udiff_uassoc" | "array_udiff_ukey" => Some((false, false, true, true, false)),
        "array_uintersect_uassoc" | "array_uintersect_ukey" => Some((true, false, true, true, false)),
        _ => None,
    };
    if let Some((intersect, ignore_values, compare_values, compare_keys, exact_keys)) = callback_diff {
        return Ok((transform::array_diff_with_callbacks(name, &args, intersect, ignore_values, compare_values, compare_keys, exact_keys, invoke)?, Vec::new()));
    }
    let mutation = match name {
        "array_pop" => Some(mutation::array_pop(&args)?),
        "array_shift" => Some(mutation::array_shift(&args)?),
        "array_splice" => Some(mutation::array_splice(&args)?),
        "array_push" => Some(mutation::array_push(&args)?),
        "array_unshift" => Some(mutation::array_unshift(&args)?),
        "shuffle" => Some(mutation::shuffle(&args)?),
        "natsort" => Some(sorting::natural_sort(&args, false, stringify)?),
        "natcasesort" => Some(sorting::natural_sort(&args, true, stringify)?),
        "sort" => Some(sorting::sort(&args, sorting::SortMode::ValuesAscending, stringify)?),
        "rsort" => Some(sorting::sort(&args, sorting::SortMode::ValuesDescending, stringify)?),
        "asort" => Some(sorting::sort(&args, sorting::SortMode::ValuesAscendingPreserveKeys, stringify)?),
        "arsort" => Some(sorting::sort(&args, sorting::SortMode::ValuesDescendingPreserveKeys, stringify)?),
        "ksort" => Some(sorting::sort(&args, sorting::SortMode::KeysAscending, stringify)?),
        "krsort" => Some(sorting::sort(&args, sorting::SortMode::KeysDescending, stringify)?),
        _ => None,
    };
    if let Some((value, mutation)) = mutation { return Ok((value, mutation.into_iter().collect())); }

    let result = match name {
        "count" | "sizeof" => query::count(&args, classes, invoke),
        "array_keys" => query::array_keys(&args),
        "array_values" => query::array_values(&args),
        "array_key_exists" | "key_exists" => query::key_exists(&args),
        "array_is_list" => query::array_is_list(&args),
        "in_array" => query::in_array(&args),
        "array_merge" => transform::array_merge(&args),
        "array_merge_recursive" => transform::array_merge_recursive(&args),
        "array_replace" => transform::array_replace(&args),
        "array_replace_recursive" => transform::array_replace_recursive(&args),
        "array_reverse" => transform::array_reverse(&args),
        "array_filter" => callbacks::array_filter(&args, invoke),
        "array_all" => callbacks::array_predicate(&args, callbacks::Predicate::All, invoke),
        "array_any" => callbacks::array_predicate(&args, callbacks::Predicate::Any, invoke),
        "array_find" => callbacks::array_find(&args, false, invoke),
        "array_find_key" => callbacks::array_find(&args, true, invoke),
        "array_map" => callbacks::array_map(&args, invoke),
        "array_reduce" => callbacks::array_reduce(&args, invoke),
        "array_column" => construct::array_column(&args, classes),
        "array_search" => query::array_search(&args),
        "array_rand" => mutation::array_rand(&args),
        "array_sum" => query::array_sum(&args),
        "array_product" => query::array_product(&args),
        "array_slice" => transform::array_slice(&args),
        "array_chunk" => transform::array_chunk(&args),
        "array_pad" => transform::array_pad(&args),
        "array_combine" => construct::array_combine(&args),
        "array_flip" => construct::array_flip(&args),
        "array_change_key_case" => construct::array_change_key_case(&args),
        "array_count_values" => construct::array_count_values(&args),
        "array_fill" => construct::array_fill(&args),
        "array_fill_keys" => construct::array_fill_keys(&args),
        "range" => construct::range(&args, diagnostics),
        "array_diff" => transform::array_diff(&args, transform::DiffMode::Values, false),
        "array_intersect" => transform::array_diff(&args, transform::DiffMode::Values, true),
        "array_diff_assoc" => transform::array_diff(&args, transform::DiffMode::ValuesAndKeys, false),
        "array_intersect_assoc" => transform::array_diff(&args, transform::DiffMode::ValuesAndKeys, true),
        "array_diff_key" => transform::array_diff(&args, transform::DiffMode::Keys, false),
        "array_intersect_key" => transform::array_diff(&args, transform::DiffMode::Keys, true),
        "array_key_first" => query::array_edge_key(&args, false),
        "array_key_last" => query::array_edge_key(&args, true),
        "array_first" => query::array_edge_value(&args, false),
        "array_last" => query::array_edge_value(&args, true),
        _ => Err(format!("Call to undefined function {name}()")),
    }?;
    Ok((result, Vec::new()))
}

/// Invoke the small set of scalar builtins accepted as array callbacks.
pub(crate) fn call_callback_builtin(name: &str, arguments: Vec<Value>) -> Result<Value, String> {
    callbacks::call_callback_builtin(name, arguments)
}
