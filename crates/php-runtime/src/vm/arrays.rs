//! Virtual-machine instructions for reading and mutating arrays.

use std::collections::HashMap;

use super::{
    read_variable, write_array_root, write_variable, GlobalVariables, StaticProperties,
    VariableBinding,
};
use crate::{
    bytecode::{ArrayEntryKind, CompoundOperation, DestructureTarget, Op},
    BytecodeClass, BytecodeFunction, Value,
};

enum PendingArrayEntry {
    Packed(Value),
    Keyed(crate::ArrayKey, Value),
    Unpack(Value),
}

fn array_key_with_diagnostic(
    value: Value,
    globals: &mut GlobalVariables,
) -> Result<crate::ArrayKey, String> {
    if let Value::Number(number) = value.dereference() {
        let diagnostic = if number.is_nan() {
            Some((
                crate::DiagnosticLevel::Warning,
                "unexpected NAN value was coerced to int".to_string(),
            ))
        } else if !number.is_finite()
            || number >= 9_223_372_036_854_775_808.0
            || number < -9_223_372_036_854_775_808.0
        {
            Some((
                crate::DiagnosticLevel::Warning,
                crate::array_builtins::non_representable_float_to_int_message(number),
            ))
        } else if number.fract() != 0.0 {
            Some((
                crate::DiagnosticLevel::Deprecated,
                format!(
                    "Implicit conversion from float {} to int loses precision",
                    number
                ),
            ))
        } else {
            None
        };
        if let Some((level, message)) = diagnostic {
            globals.report_diagnostic(crate::RuntimeDiagnostic {
                level,
                message,
                source_file: None,
                source_line: None,
                output_offset: 0,
            });
        }
    }
    value.array_key()
}

fn array_key_for_isset(value: Value) -> Result<crate::ArrayKey, String> {
    value.array_key().map_err(|_| {
        format!(
            "TypeError:Cannot access offset of type {} in isset or empty",
            invalid_offset_type_name(&value)
        )
    })
}

fn invalid_offset_type_name(value: &Value) -> String {
    match value.dereference() {
        Value::Object(object) => object.borrow().class_name.clone(),
        value => value.type_name().to_string(),
    }
}

fn unset_array_key(value: Value, container: &Value) -> Result<crate::ArrayKey, String> {
    value.array_key().map_err(|_| {
        format!(
            "Error:Cannot unset offset of type {} on {}",
            invalid_offset_type_name(&value),
            container.type_name(),
        )
    })
}

/// Execute array storage instructions; return false when the opcode belongs to another VM group.
pub(super) fn execute_instruction(
    instruction: &Op,
    stack: &mut Vec<Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<bool, String> {
    match instruction {
        Op::MakeArray(has_keys) => {
            let mut entries = Vec::with_capacity(has_keys.len());
            // Operands were emitted left-to-right, so pop in reverse then restore insertion order.
            for kind in has_keys.iter().rev() {
                match kind {
                    ArrayEntryKind::Keyed => {
                        let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
                        let key = array_key_with_diagnostic(
                            stack.pop().ok_or("Invalid bytecode stack")?,
                            globals,
                        )?;
                        entries.push(PendingArrayEntry::Keyed(key, value));
                    }
                    ArrayEntryKind::Packed => {
                        entries.push(PendingArrayEntry::Packed(
                            stack.pop().ok_or("Invalid bytecode stack")?.dereference(),
                        ));
                    }
                    ArrayEntryKind::KeyedReference => {
                        let value = stack.pop().ok_or("Invalid bytecode stack")?;
                        let key = array_key_with_diagnostic(
                            stack.pop().ok_or("Invalid bytecode stack")?,
                            globals,
                        )?;
                        entries.push(PendingArrayEntry::Keyed(key, value));
                    }
                    ArrayEntryKind::PackedReference => {
                        entries.push(PendingArrayEntry::Packed(
                            stack.pop().ok_or("Invalid bytecode stack")?,
                        ));
                    }
                    ArrayEntryKind::Unpack => {
                        entries.push(PendingArrayEntry::Unpack(
                            stack.pop().ok_or("Invalid bytecode stack")?,
                        ));
                    }
                }
            }
            entries.reverse();
            let mut array = Value::array(Vec::new());
            for entry in entries {
                array = match entry {
                    PendingArrayEntry::Packed(value) => array.array_append(value)?,
                    PendingArrayEntry::Keyed(key, value) => array.array_set(key, value)?,
                    PendingArrayEntry::Unpack(value) => {
                        let Value::Array(unpacked) = value else {
                            return Err("Only arrays can be unpacked into arrays".into());
                        };
                        for (key, value) in unpacked {
                            array = match key {
                                crate::ArrayKey::Integer(_) => array.array_append(value)?,
                                key => array.array_set(key, value)?,
                            };
                        }
                        array
                    }
                };
            }
            stack.push(array);
        }
        Op::ArrayGet | Op::ArrayGetSilent | Op::ArrayGetIsset => {
            let raw_key = stack.pop().ok_or("Invalid bytecode stack")?;
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            let key = if matches!(instruction, Op::ArrayGetIsset) {
                match array_key_for_isset(raw_key.clone()) {
                    Ok(key) => key,
                    Err(_) if matches!(array.dereference(), Value::String(_)) => {
                        globals.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Warning,
                            message: format!(
                                "Cannot access offset of type {} on string",
                                invalid_offset_type_name(&raw_key)
                            ),
                            source_file: None,
                            source_line: None,
                            output_offset: 0,
                        });
                        stack.push(Value::Null);
                        return Ok(true);
                    }
                    Err(error) => return Err(error),
                }
            } else {
                array_key_with_diagnostic(raw_key, globals)?
            };
            if matches!(instruction, Op::ArrayGet) {
                super::warn_undefined_array_key(&array, std::slice::from_ref(&key), globals);
            }
            stack.push(array.array_get(&key)?);
        }
        Op::AssignArrayPath(variable, key_count) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys
                .into_iter()
                .map(|key| array_key_with_diagnostic(key, globals))
                .collect::<Result<Vec<_>, _>>()?;
            let root = stack.pop().ok_or("Invalid bytecode stack")?;
            let previous = root.array_get_path(&keys)?;
            let updated = root.array_set_path(&keys, value.clone())?;
            write_array_root(
                variable.clone(),
                updated,
                variables,
                bindings,
                globals,
                global_scope,
            )?;
            super::objects::run_destructors_removed_from_value(
                previous,
                &value,
                functions,
                classes,
                static_properties,
                globals,
                output,
                steps,
            )?;
            stack.push(value);
        }
        Op::AssignCompoundPath(variable, key_count, compound_operation) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            if stack.len() < key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys
                .into_iter()
                .map(|key| array_key_with_diagnostic(key, globals))
                .collect::<Result<Vec<_>, _>>()?;
            let root = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = root.array_get_path(&keys)?;
            let operation = match compound_operation {
                crate::bytecode::CompoundOperation::Add => Op::Add,
                crate::bytecode::CompoundOperation::Subtract => Op::Sub,
                crate::bytecode::CompoundOperation::Multiply => Op::Mul,
                crate::bytecode::CompoundOperation::Power => Op::Pow,
                crate::bytecode::CompoundOperation::Divide => Op::Div,
                crate::bytecode::CompoundOperation::Modulo => Op::Mod,
                crate::bytecode::CompoundOperation::Concatenate => Op::Concat,
                CompoundOperation::BitAnd => Op::BitAnd,
                CompoundOperation::BitXor => Op::BitXor,
                CompoundOperation::BitOr => Op::BitOr,
                CompoundOperation::ShiftLeft => Op::ShiftLeft,
                CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let assigned = super::operators::apply_binary(&operation, current, value)?;
            let updated = root.array_set_path(&keys, assigned.clone())?;
            write_array_root(
                variable.clone(),
                updated,
                variables,
                bindings,
                globals,
                global_scope,
            )?;
            stack.push(assigned);
        }
        Op::CompoundAssignPath(key_count, operation) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            if stack.len() < key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys
                .into_iter()
                .map(|key| array_key_with_diagnostic(key, globals))
                .collect::<Result<Vec<_>, _>>()?;
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = array.array_get_path(&keys)?;
            let operation = match operation {
                crate::bytecode::CompoundOperation::Add => Op::Add,
                crate::bytecode::CompoundOperation::Subtract => Op::Sub,
                crate::bytecode::CompoundOperation::Multiply => Op::Mul,
                crate::bytecode::CompoundOperation::Power => Op::Pow,
                crate::bytecode::CompoundOperation::Divide => Op::Div,
                crate::bytecode::CompoundOperation::Modulo => Op::Mod,
                crate::bytecode::CompoundOperation::Concatenate => Op::Concat,
                CompoundOperation::BitAnd => Op::BitAnd,
                CompoundOperation::BitXor => Op::BitXor,
                CompoundOperation::BitOr => Op::BitOr,
                CompoundOperation::ShiftLeft => Op::ShiftLeft,
                CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let updated = super::operators::apply_binary(&operation, current, value)?;
            stack.push(array.array_set_path(&keys, updated)?);
        }
        Op::ListDestructure(
            targets,
            target_key_count,
            access_scope,
            in_constructor,
            source_variable,
        ) => {
            if stack.len() < target_key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let target_keys = stack.split_off(stack.len() - target_key_count);
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let mut target_keys = target_keys.into_iter();
            destructure_value(
                &value,
                targets,
                &mut target_keys,
                variables,
                bindings,
                globals,
                global_scope,
                access_scope.as_deref(),
                *in_constructor,
                source_variable.as_deref(),
                &value,
                &[],
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            if target_keys.next().is_some() {
                return Err("Unused destructuring target key".into());
            }
        }
        Op::ListDestructureKeep(
            targets,
            target_key_count,
            access_scope,
            in_constructor,
            source_variable,
        ) => {
            if stack.len() < target_key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let target_keys = stack.split_off(stack.len() - target_key_count);
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let mut target_keys = target_keys.into_iter();
            destructure_value(
                &value,
                targets,
                &mut target_keys,
                variables,
                bindings,
                globals,
                global_scope,
                access_scope.as_deref(),
                *in_constructor,
                source_variable.as_deref(),
                &value,
                &[],
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            if target_keys.next().is_some() {
                return Err("Unused destructuring target key".into());
            }
            stack.push(value);
        }
        Op::ArraySetPath(key_count) | Op::ArrayAppendPath(key_count) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys
                .into_iter()
                .map(|key| array_key_with_diagnostic(key, globals))
                .collect::<Result<Vec<_>, _>>()?;
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            let result = if matches!(instruction, Op::ArraySetPath(_)) {
                array.array_set_path(&keys, value)?
            } else {
                array.array_append_path(&keys, value)?
            };
            stack.push(result);
        }
        Op::ArraySetMixedPath(path) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let key_count = path.iter().filter(|append| !**append).count();
            if stack.len() < key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let mut keys = raw_keys
                .into_iter()
                .map(|key| array_key_with_diagnostic(key, globals))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter();
            let segments = path
                .iter()
                .map(|append| {
                    if *append {
                        Ok(None)
                    } else {
                        keys.next()
                            .map(Some)
                            .ok_or_else(|| "Invalid array path key stack".to_string())
                    }
                })
                .collect::<Result<Vec<_>, String>>()?;
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(array.array_set_mixed_path(&segments, value)?);
        }
        Op::AssignArrayAppendPath(variable, key_count) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys
                .into_iter()
                .map(|key| array_key_with_diagnostic(key, globals))
                .collect::<Result<Vec<_>, _>>()?;
            let root = stack.pop().ok_or("Invalid bytecode stack")?;
            let updated = root.array_append_path(&keys, value.clone())?;
            write_array_root(
                variable.clone(),
                updated,
                variables,
                bindings,
                globals,
                global_scope,
            )?;
            stack.push(value);
        }
        Op::ArrayAppend => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(array.array_append(value)?);
        }
        Op::UnsetArrayPath(name, key_count) => {
            if stack.len() < *key_count {
                return Err("Invalid bytecode stack".into());
            }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let current = read_variable(name, variables, bindings, globals, global_scope);
            let keys = raw_keys
                .into_iter()
                .map(|key| unset_array_key(key, &current))
                .collect::<Result<Vec<_>, _>>()?;
            let (updated, removed) = current.clone().array_unset_path_with_removed(&keys)?;
            write_array_root(
                name.clone(),
                updated,
                variables,
                bindings,
                globals,
                global_scope,
            )?;
            drop(current);
            if let Some(value) = removed {
                super::objects::run_destructors_removed_from_value(
                    value,
                    &Value::Null,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                )?;
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn destructure_value(
    value: &Value,
    targets: &[DestructureTarget],
    target_keys: &mut impl Iterator<Item = Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    access_scope: Option<&str>,
    in_constructor: bool,
    source_variable: Option<&str>,
    source_root: &Value,
    source_path: &[crate::ArrayKey],
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let source = match value.dereference() {
        Value::Array(_) | Value::Null => value.dereference(),
        Value::Object(object) => {
            return Err(format!(
                "Error:Cannot use object of type {} as array",
                object.borrow().class_name
            ))
        }
        Value::Closure(_) => return Err("Error:Cannot use object of type Closure as array".into()),
        Value::Generator(_) => {
            return Err("Error:Cannot use object of type Generator as array".into())
        }
        Value::Integer(_)
        | Value::Number(_)
        | Value::String(_)
        | Value::Bool(_)
        | Value::Stream(_) => {
            let kind = value.type_name();
            globals.report_diagnostic(crate::RuntimeDiagnostic {
                level: crate::DiagnosticLevel::Warning,
                message: format!("Cannot use {kind} as array"),
                source_file: None,
                source_line: None,
                output_offset: 0,
            });
            Value::Null
        }
        Value::Reference(_) => unreachable!("dereference removes reference wrappers"),
    };
    let mut next_implicit_index = 0i64;
    for target in targets {
        let key = if target.computed_key {
            array_key_with_diagnostic(
                target_keys
                    .next()
                    .ok_or("Invalid destructuring source key stack")?,
                globals,
            )?
        } else {
            target
                .key
                .clone()
                .unwrap_or(crate::ArrayKey::Integer(next_implicit_index))
        };
        if let crate::ArrayKey::Integer(index) = key {
            if index >= next_implicit_index {
                next_implicit_index = index.saturating_add(1);
            }
        }
        let has_target = target.variable.is_some()
            || target.array_variable.is_some()
            || target.property.is_some()
            || target.nested.is_some();
        if has_target
            && matches!(source.dereference(), Value::Array(_))
            && source
                .first_missing_array_path_key(std::slice::from_ref(&key))
                .is_some()
        {
            let rendered = match &key {
                crate::ArrayKey::Integer(index) => index.to_string(),
                crate::ArrayKey::String(key) => format!("\"{key}\""),
            };
            globals.report_diagnostic(crate::RuntimeDiagnostic {
                level: crate::DiagnosticLevel::Warning,
                message: format!("Undefined array key {rendered}"),
                source_file: None,
                source_line: None,
                output_offset: 0,
            });
        }
        let item = source.array_get(&key)?;
        if let Some(name) = &target.variable {
            let previous = read_variable(name, variables, bindings, globals, global_scope);
            if target.by_reference {
                let mut path = source_path.to_vec();
                path.push(key.clone());
                if let Some(source_variable) = source_variable {
                    super::assign_array_reference(
                        name,
                        source_variable,
                        path,
                        variables,
                        bindings,
                        globals,
                        global_scope,
                        frame_id,
                    )?;
                } else {
                    let (_, reference) = source_root.clone().ensure_array_reference(&path)?;
                    match bindings.get(name).cloned() {
                        Some(VariableBinding::Global(global))
                        | Some(VariableBinding::Static(global)) => {
                            let key = globals.store_reference(reference);
                            globals.bind_alias(global, key);
                        }
                        Some(_) => {
                            let key = globals.store_reference(reference);
                            bindings.insert(name.clone(), VariableBinding::Reference(key));
                            variables.remove(name);
                        }
                        None if global_scope => {
                            let key = globals.store_reference(reference);
                            globals.bind_alias(name.clone(), key);
                        }
                        None => {
                            variables.remove(name);
                            variables.insert(name.clone(), Value::Reference(reference));
                        }
                    }
                }
            } else {
                write_variable(
                    name.clone(),
                    item.clone(),
                    variables,
                    bindings,
                    globals,
                    global_scope,
                )?;
            }
            super::objects::run_destructors_removed_from_value(
                previous,
                &item,
                functions,
                classes,
                static_properties,
                globals,
                output,
                steps,
            )?;
        }
        if let Some(name) = &target.array_variable {
            let keys = (0..target.array_key_count)
                .map(|_| {
                    target_keys
                        .next()
                        .ok_or_else(|| "Invalid destructuring target key stack".to_string())
                        .and_then(|key| array_key_with_diagnostic(key, globals))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let current = read_variable(name, variables, bindings, globals, global_scope);
            let previous = if target.array_append {
                None
            } else {
                Some(current.array_get_path(&keys)?)
            };
            if target.by_reference {
                let mut source_keys = source_path.to_vec();
                source_keys.push(key.clone());
                if source_variable.is_some() && !target.array_append {
                    super::bind_array_reference(
                        name,
                        keys,
                        source_variable.unwrap(),
                        source_keys,
                        variables,
                        bindings,
                        globals,
                        global_scope,
                        frame_id,
                    )?;
                } else {
                    let (_, reference) =
                        source_root.clone().ensure_array_reference(&source_keys)?;
                    let updated = current.clone().bind_array_reference(&keys, reference)?;
                    write_array_root(
                        name.clone(),
                        updated,
                        variables,
                        bindings,
                        globals,
                        global_scope,
                    )?;
                }
            } else {
                let updated = if target.array_append {
                    current.clone().array_append_path(&keys, item.clone())?
                } else {
                    current.clone().array_set_path(&keys, item.clone())?
                };
                write_array_root(
                    name.clone(),
                    updated,
                    variables,
                    bindings,
                    globals,
                    global_scope,
                )?;
            }
            drop(current);
            if let Some(previous) = previous {
                super::objects::run_destructors_removed_from_value(
                    previous,
                    &item,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                )?;
            }
        }
        if let Some(property) = &target.property {
            if !target.property_receiver {
                return Err("Invalid destructuring property receiver".into());
            }
            let keys = (0..target.property_array_key_count)
                .map(|_| {
                    target_keys
                        .next()
                        .ok_or_else(|| "Invalid destructuring property key stack".to_string())
                        .and_then(|key| array_key_with_diagnostic(key, globals))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let receiver = target_keys
                .next()
                .ok_or("Invalid destructuring property receiver stack")?;
            let previous = if target.by_reference {
                let mut source_keys = source_path.to_vec();
                source_keys.push(key.clone());
                let reference = if let Some(source_variable) = source_variable {
                    super::reference_cell_for_path(
                        source_variable,
                        &source_keys,
                        variables,
                        bindings,
                        globals,
                        global_scope,
                        frame_id,
                    )?
                    .1
                } else {
                    source_root.clone().ensure_array_reference(&source_keys)?.1
                };
                if keys.is_empty() {
                    super::objects::bind_property_reference(
                        &receiver,
                        property,
                        reference,
                        access_scope,
                        classes,
                    )?
                } else {
                    let current = super::objects::read_property(
                        &receiver,
                        property,
                        access_scope,
                        functions,
                        classes,
                        static_properties,
                        globals,
                        output,
                        steps,
                    )?;
                    let previous = current.array_get_path(&keys)?;
                    let updated = current.clone().bind_array_reference(&keys, reference)?;
                    drop(current);
                    super::objects::assign_property(
                        &receiver,
                        property,
                        updated,
                        access_scope,
                        in_constructor,
                        functions,
                        classes,
                        static_properties,
                        globals,
                        output,
                        steps,
                    )?;
                    Some(previous)
                }
            } else if keys.is_empty() {
                let previous = super::objects::existing_property_for_assignment(
                    &receiver,
                    property,
                    access_scope,
                    classes,
                );
                super::objects::assign_property(
                    &receiver,
                    property,
                    item.clone(),
                    access_scope,
                    in_constructor,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                )?;
                previous
            } else {
                let current = super::objects::read_property(
                    &receiver,
                    property,
                    access_scope,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                )?;
                let previous = current.array_get_path(&keys)?;
                let updated = current.clone().array_set_path(&keys, item.clone())?;
                drop(current);
                super::objects::assign_property(
                    &receiver,
                    property,
                    updated,
                    access_scope,
                    in_constructor,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                )?;
                Some(previous)
            };
            if let Some(previous) = previous {
                super::objects::run_destructors_removed_from_value(
                    previous,
                    &item,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                )?;
            }
        }
        if let Some(nested) = &target.nested {
            let mut nested_path = source_path.to_vec();
            nested_path.push(key);
            destructure_value(
                &item,
                nested,
                target_keys,
                variables,
                bindings,
                globals,
                global_scope,
                access_scope,
                in_constructor,
                source_variable,
                source_root,
                &nested_path,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
        }
    }
    Ok(())
}
