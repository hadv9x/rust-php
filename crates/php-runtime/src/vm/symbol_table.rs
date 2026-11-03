//! PHP helpers that read or populate the active variable symbol table.

use std::collections::HashMap;

use crate::{ArrayKey, Value};
use crate::bytecode::ReferenceArgument;
use super::{GlobalVariables, VariableBinding, variable_exists, read_variable, write_variable};

fn collect_compact_names(value: &Value, names: &mut Vec<String>) -> Result<(), String> {
    let value = value.dereference();
    match &value {
        Value::String(name) => names.push(name.clone()),
        Value::Array(items) => {
            for (_, value) in items.iter() { collect_compact_names(value, names)?; }
        }
        _ => return Err("compact(): Argument must be of type string|array".into()),
    }
    Ok(())
}

/// Build an array from variable names visible in the active local/global scope.
pub(super) fn compact(
    args: &[Value],
    variables: &HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &GlobalVariables,
    global_scope: bool,
) -> Result<Value, String> {
    if args.is_empty() { return Err("compact(): At least one variable name is required".into()); }
    let mut names = Vec::new();
    for value in args { collect_compact_names(value, &mut names)?; }
    let mut entries = Vec::new();
    for name in names {
        if variable_exists(&name, variables, bindings, globals, global_scope) {
            entries.push((ArrayKey::String(name.clone()), read_variable(&name, variables, bindings, globals, global_scope)));
        }
    }
    Ok(Value::array(entries))
}

/// Return the variables visible in the active scope, resolving global and reference bindings.
pub(super) fn get_defined_vars(
    variables: &HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &GlobalVariables,
    global_scope: bool,
) -> Value {
    let mut names = Vec::new();
    if global_scope {
        names.extend(globals.order.iter().filter(|name| !name.starts_with('\0')).cloned());
        let missing_globals = globals.values.keys().chain(globals.aliases.keys())
            .filter(|name| !name.starts_with('\0') && !names.contains(name))
            .cloned().collect::<std::collections::BTreeSet<_>>();
        names.extend(missing_globals);
        let remaining = variables.keys().chain(bindings.keys())
            .filter(|name| !name.starts_with('\0') && !names.contains(name))
            .cloned().collect::<std::collections::BTreeSet<_>>();
        names.extend(remaining);
    } else {
        if let Some(Value::Array(order)) = variables.get("\0local-symbol-order").map(Value::dereference) {
            for (_, value) in order.iter() {
                if let Value::String(name) = value.dereference() {
                    if !names.contains(&name) && (variables.contains_key(&name) || bindings.contains_key(&name)) {
                        names.push(name);
                    }
                }
            }
        }
        let remaining = variables.keys().chain(bindings.keys())
            .filter(|name| !name.starts_with('\0') && !names.contains(name))
            .cloned().collect::<std::collections::BTreeSet<_>>();
        names.extend(remaining);
    }
    let entries = names.into_iter()
        .map(|name| (
            ArrayKey::String(name.clone()),
            read_variable(&name, variables, bindings, globals, global_scope),
        ))
        .collect();
    Value::array(entries)
}

fn valid_variable_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Import string-keyed array entries into the current symbol table using supported EXTR modes.
pub(super) fn extract(
    args: &[Value],
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    source_reference: Option<&ReferenceArgument>,
) -> Result<Value, String> {
    if args.is_empty() || args.len() > 3 { return Err("extract() expects between 1 and 3 arguments".into()); }
    let Value::Array(source) = &args[0] else { return Err("extract(): Argument #1 ($array) must be of type array".into()); };
    let flags = args.get(1).map(|value| value.number() as i64).unwrap_or(0);
    let by_reference = flags & 0x100 != 0;
    let source_path = if by_reference {
        let source = source_reference.ok_or("extract(): EXTR_REFS requires the input array to be a variable")?;
        if source.object_property.is_some() { return Err("extract(): EXTR_REFS from an object property is not supported".into()); }
        let mut keys = match bindings.get(&source.variable) {
            Some(VariableBinding::ArrayElement { keys, .. }) => keys.clone(),
            _ => Vec::new(),
        };
        keys.extend(source.array_keys.iter().cloned());
        let storage_key = super::reference_storage_key(&source.variable, variables, bindings, globals, global_scope, frame_id);
        Some((storage_key, keys))
    } else { None };
    let mode = flags & 0xff;
    if !matches!(mode, 0 | 1 | 2 | 3 | 4 | 5 | 6) {
        return Err(format!("extract(): unsupported extraction mode {mode}"));
    }
    let prefix = match args.get(2) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(prefix)) => prefix.clone(),
        Some(_) => return Err("extract(): prefix must be a string".into()),
    };
    if matches!(mode, 2 | 3 | 4 | 5) && prefix.is_empty() {
        return Err("extract(): prefix is required for this extraction mode".into());
    }

    let mut extracted = 0;
    for (key, value) in source.iter() {
        let name = match key {
            ArrayKey::String(name) => name.clone(),
            ArrayKey::Integer(index) => index.to_string(),
        };
        let valid = valid_variable_name(&name);
        let exists = variable_exists(&name, variables, bindings, globals, global_scope);
        let target = match mode {
            0 => { if !valid { continue; } name.clone() }
            1 => { if !valid || exists { continue; } name.clone() }
            2 => if exists { format!("{prefix}_{name}") } else if valid { name.clone() } else { continue },
            3 => format!("{prefix}_{name}"),
            4 => if valid { name.clone() } else { format!("{prefix}_{name}") },
            5 => if exists { format!("{prefix}_{name}") } else { continue },
            6 => { if !exists { continue; } name.clone() }
            _ => unreachable!(),
        };
        if !valid_variable_name(&target) { continue; }
        if let Some((storage_key, parent_keys)) = &source_path {
            let mut keys = parent_keys.clone();
            keys.push(key.clone());
            bindings.insert(target.clone(), VariableBinding::ArrayElement { storage_key: storage_key.clone(), keys });
            variables.remove(&target);
        } else {
            write_variable(target, value.dereference(), variables, bindings, globals, global_scope)?;
        }
        extracted += 1;
    }
    Ok(Value::Integer(extracted as i64))
}

