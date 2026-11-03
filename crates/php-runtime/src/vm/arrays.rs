//! Virtual-machine instructions for reading and mutating arrays.

use std::collections::HashMap;

use crate::{bytecode::{ArrayEntryKind, CompoundOperation, DestructureTarget, Op}, BytecodeClass, BytecodeFunction, Value};
use super::{read_variable, write_array_root, write_variable, GlobalVariables, StaticProperties, VariableBinding};

enum PendingArrayEntry {
    Packed(Value),
    Keyed(crate::ArrayKey, Value),
    Unpack(Value),
}

/// Execute array storage instructions; return false when the opcode belongs to another VM group.
pub(super) fn execute_instruction(
    instruction: &Op,
    stack: &mut Vec<Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
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
                        let key = stack.pop().ok_or("Invalid bytecode stack")?.array_key()?;
                        entries.push(PendingArrayEntry::Keyed(key, value));
                    }
                    ArrayEntryKind::Packed => {
                        entries.push(PendingArrayEntry::Packed(stack.pop().ok_or("Invalid bytecode stack")?.dereference()));
                    }
                    ArrayEntryKind::KeyedReference => {
                        let value = stack.pop().ok_or("Invalid bytecode stack")?;
                        let key = stack.pop().ok_or("Invalid bytecode stack")?.array_key()?;
                        entries.push(PendingArrayEntry::Keyed(key, value));
                    }
                    ArrayEntryKind::PackedReference => {
                        entries.push(PendingArrayEntry::Packed(stack.pop().ok_or("Invalid bytecode stack")?));
                    }
                    ArrayEntryKind::Unpack => {
                        entries.push(PendingArrayEntry::Unpack(stack.pop().ok_or("Invalid bytecode stack")?));
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
                        let Value::Array(unpacked) = value else { return Err("Only arrays can be unpacked into arrays".into()); };
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
        Op::ArrayGet | Op::ArrayGetSilent => {
            let key = stack.pop().ok_or("Invalid bytecode stack")?.array_key()?;
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            if matches!(instruction, Op::ArrayGet) {
                super::warn_undefined_array_key(&array, std::slice::from_ref(&key), globals);
            }
            stack.push(array.array_get(&key)?);
        }
        Op::AssignArrayPath(variable, key_count) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let root = stack.pop().ok_or("Invalid bytecode stack")?;
            let previous = root.array_get_path(&keys)?;
            let updated = root.array_set_path(&keys, value.clone())?;
            write_array_root(variable.clone(), updated, variables, bindings, globals, global_scope)?;
            super::objects::run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, globals, output, steps)?;
            stack.push(value);
        }
        Op::AssignCompoundPath(variable, key_count, compound_operation) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
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
            write_array_root(variable.clone(), updated, variables, bindings, globals, global_scope)?;
            stack.push(assigned);
        }
        Op::CompoundAssignPath(key_count, operation) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
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
        Op::ListDestructure(targets, target_key_count, access_scope, in_constructor) => {
            if stack.len() < target_key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let target_keys = stack.split_off(stack.len() - target_key_count);
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let mut target_keys = target_keys.into_iter();
            destructure_value(&value, targets, &mut target_keys, variables, bindings, globals, global_scope, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, output, steps)?;
            if target_keys.next().is_some() { return Err("Unused destructuring target key".into()); }
        }
        Op::ListDestructureKeep(targets, target_key_count, access_scope, in_constructor) => {
            if stack.len() < target_key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let target_keys = stack.split_off(stack.len() - target_key_count);
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let mut target_keys = target_keys.into_iter();
            destructure_value(&value, targets, &mut target_keys, variables, bindings, globals, global_scope, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, output, steps)?;
            if target_keys.next().is_some() { return Err("Unused destructuring target key".into()); }
            stack.push(value);
        }
        Op::ArraySetPath(key_count) | Op::ArrayAppendPath(key_count) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
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
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let mut keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?.into_iter();
            let segments = path.iter().map(|append| {
                if *append { Ok(None) } else { keys.next().map(Some).ok_or_else(|| "Invalid array path key stack".to_string()) }
            }).collect::<Result<Vec<_>, String>>()?;
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(array.array_set_mixed_path(&segments, value)?);
        }
        Op::AssignArrayAppendPath(variable, key_count) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let root = stack.pop().ok_or("Invalid bytecode stack")?;
            let updated = root.array_append_path(&keys, value.clone())?;
            write_array_root(variable.clone(), updated, variables, bindings, globals, global_scope)?;
            stack.push(value);
        }
        Op::ArrayAppend => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let array = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(array.array_append(value)?);
        }
        Op::UnsetArrayPath(name, key_count) => {
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let current = read_variable(name, variables, bindings, globals, global_scope);
            let (updated, removed) = current.clone().array_unset_path_with_removed(&keys)?;
            write_array_root(name.clone(), updated, variables, bindings, globals, global_scope)?;
            drop(current);
            if let Some(value) = removed {
                super::objects::run_destructors_removed_from_value(value, &Value::Null, functions, classes, static_properties, globals, output, steps)?;
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
    bindings: &HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    access_scope: Option<&str>,
    in_constructor: bool,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let mut next_implicit_index = 0i64;
    for target in targets {
        let key = if target.computed_key {
            target_keys.next().ok_or("Invalid destructuring source key stack")?.array_key()?
        } else {
            target.key.clone().unwrap_or(crate::ArrayKey::Integer(next_implicit_index))
        };
        if let crate::ArrayKey::Integer(index) = key {
            if index >= next_implicit_index { next_implicit_index = index.saturating_add(1); }
        }
        let item = value.array_get(&key)?;
        if let Some(name) = &target.variable {
            write_variable(name.clone(), item.clone(), variables, bindings, globals, global_scope)?;
        }
        if let Some(name) = &target.array_variable {
            let keys = (0..target.array_key_count)
                .map(|_| target_keys.next().ok_or("Invalid destructuring target key stack")?.array_key())
                .collect::<Result<Vec<_>, _>>()?;
            let current = read_variable(name, variables, bindings, globals, global_scope);
            let updated = if target.array_append {
                current.array_append_path(&keys, item.clone())?
            } else {
                current.array_set_path(&keys, item.clone())?
            };
            write_array_root(name.clone(), updated, variables, bindings, globals, global_scope)?;
        }
        if let Some(property) = &target.property {
            if !target.property_receiver { return Err("Invalid destructuring property receiver".into()); }
            let keys = (0..target.property_array_key_count)
                .map(|_| target_keys.next().ok_or("Invalid destructuring property key stack")?.array_key())
                .collect::<Result<Vec<_>, _>>()?;
            let receiver = target_keys.next().ok_or("Invalid destructuring property receiver stack")?;
            let assigned = if keys.is_empty() { item.clone() } else {
                let current = super::objects::read_property(&receiver, property, access_scope, functions, classes, static_properties, globals, output, steps)?;
                current.array_set_path(&keys, item.clone())?
            };
            super::objects::assign_property(&receiver, property, assigned, access_scope, in_constructor, functions, classes, static_properties, globals, output, steps)?;
        }
        if let Some(nested) = &target.nested {
            destructure_value(&item, nested, target_keys, variables, bindings, globals, global_scope, access_scope, in_constructor, functions, classes, static_properties, output, steps)?;
        }
    }
    Ok(())
}

