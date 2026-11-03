//! Virtual-machine instructions for loops, jumps, and unset operations.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{bytecode::Op, ArrayKey, PhpGenerator, Value};
use super::{read_variable, remove_variable, write_variable, GlobalVariables, VariableBinding};

/// Active foreach iterators keyed by the bytecode instruction that created them.
pub(super) type Iterators = HashMap<usize, ForeachIterator>;

#[derive(Debug)]
pub(super) enum ForeachIterator {
    Array(Vec<(ArrayKey, Value)>, usize),
    Generator(Rc<RefCell<PhpGenerator>>, bool),
    UserIterator { object: Value, advance_pending: bool },
}

pub(super) fn call_iterator_method(
    object: Value,
    method: &str,
    stack: &mut Vec<Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    functions: &HashMap<String, crate::BytecodeFunction>,
    classes: &HashMap<String, crate::BytecodeClass>,
    static_properties: &mut super::StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    use crate::bytecode::Op;
    stack.push(object);
    let instruction = Op::CallMethod(method.into(), 0, Vec::new(), Vec::new(), Vec::new(), None, false);
    super::functions::execute_instruction(&instruction, stack, variables, bindings, globals, global_scope,
        frame_id, functions, classes, static_properties, output, steps)?;
    stack.pop().ok_or_else(|| "Iterator method returned no value".into())
}

/// Execute branches, foreach state, and local-variable removal instructions.
pub(super) fn execute_instruction(
    instruction: &Op,
    stack: &mut Vec<Value>,
    iterators: &mut Iterators,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    program_counter: &mut usize,
    functions: &HashMap<String, crate::BytecodeFunction>,
    classes: &HashMap<String, crate::BytecodeClass>,
    static_properties: &mut super::StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<bool, String> {
    match instruction {
        Op::IterStart(id) => {
            let mut value = stack.pop().ok_or("Invalid bytecode stack")?;
            let mut aggregate_depth = 0usize;
            while let Value::Object(object) = value.dereference() {
                let class_name = object.borrow().class_name.clone();
                if !super::is_a_type(&class_name.to_ascii_lowercase(), "iteratoraggregate", classes, &mut std::collections::HashSet::new()) { break; }
                if aggregate_depth >= 32 { return Err("IteratorAggregate nesting is too deep".into()); }
                value = call_iterator_method(Value::Object(object), "getiterator", stack, variables, bindings, globals,
                    global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                aggregate_depth += 1;
            }
            if aggregate_depth > 0 && matches!(value.dereference(), Value::Array(_)) {
                return Err("IteratorAggregate::getIterator() must return a Traversable object".into());
            }
            let iterator = match value {
                Value::Array(entries) => ForeachIterator::Array(entries.into_iter().collect(), 0),
                Value::Generator(generator) => {
                    let has_current = super::rewind_generator(
                        &generator, functions, classes, static_properties, globals, output, steps,
                    )?;
                    ForeachIterator::Generator(generator, has_current)
                }
                Value::Object(object) if object.borrow().class_name.eq_ignore_ascii_case("PDOStatement") => {
                    let rows = super::functions::pdo_iterator_rows(&Value::Object(object), variables, bindings, globals,
                        global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                    ForeachIterator::Array(rows, 0)
                }
                Value::Object(object) => {
                    let class_name = object.borrow().class_name.clone();
                    let normalized_class = class_name.to_ascii_lowercase();
                    if !super::is_a_type(&normalized_class, "traversable", classes, &mut std::collections::HashSet::new()) {
                        return Err(format!("foreach(): object of type {class_name} is not Traversable"));
                    }
                    if super::is_a_type(&normalized_class, "iteratoraggregate", classes, &mut std::collections::HashSet::new()) {
                        let iterator = call_iterator_method(Value::Object(object), "getiterator", stack, variables, bindings, globals,
                            global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                        match iterator {
                            Value::Array(_) => return Err("IteratorAggregate::getIterator() must return a Traversable object".into()),
                            Value::Generator(generator) => {
                                let has_current = super::rewind_generator(&generator, functions, classes, static_properties, globals, output, steps)?;
                                ForeachIterator::Generator(generator, has_current)
                            }
                            Value::Object(iterator_object) if iterator_object.borrow().class_name.eq_ignore_ascii_case("PDOStatement") => {
                                let rows = super::functions::pdo_iterator_rows(&Value::Object(iterator_object), variables, bindings, globals,
                                    global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                                ForeachIterator::Array(rows, 0)
                            }
                            Value::Object(iterator_object) => {
                                let iterator_class = iterator_object.borrow().class_name.clone();
                                let required = ["rewind", "valid", "current", "key", "next"];
                                if !super::is_a_type(&iterator_class.to_ascii_lowercase(), "iterator", classes, &mut std::collections::HashSet::new())
                                    || required.iter().any(|method| super::objects::find_method(&iterator_class, method, classes).is_none()) {
                                    return Err("IteratorAggregate::getIterator() must return a Traversable object".into());
                                }
                                let iterator = Value::Object(iterator_object);
                                let _ = call_iterator_method(iterator.clone(), "rewind", stack, variables, bindings, globals,
                                    global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                                ForeachIterator::UserIterator { object: iterator, advance_pending: false }
                            }
                            _ => return Err("IteratorAggregate::getIterator() must return a Traversable object".into()),
                        }
                    } else {
                        if !super::is_a_type(&normalized_class, "iterator", classes, &mut std::collections::HashSet::new()) {
                            return Err(format!("Object of type {class_name} is not an Iterator"));
                        }
                        let required = ["rewind", "valid", "current", "key", "next"];
                        if required.iter().any(|method| super::objects::find_method(&class_name, method, classes).is_none()) {
                            return Err(format!("Object of type {class_name} does not provide the Iterator methods"));
                        }
                    let iterator = Value::Object(object);
                    let _ = call_iterator_method(iterator.clone(), "rewind", stack, variables, bindings, globals,
                        global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                    ForeachIterator::UserIterator { object: iterator, advance_pending: false }
                    }
                }
                Value::Null => ForeachIterator::Array(Vec::new(), 0),
                _ => return Err("foreach expects an array".into()),
            };
            iterators.insert(*id, iterator);
        }
        Op::IterStartReference(id, variable, key_count) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let offsets = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let root = stack.pop().ok_or("Invalid bytecode stack")?;
            let value = root.array_get_path(&offsets)?;
            let initial_entries = match value {
                Value::Array(entries) => entries.into_iter().collect::<Vec<_>>(),
                Value::Null => Vec::new(),
                _ => return Err("foreach expects an array".into()),
            };
            for (key, _) in &initial_entries {
                let mut path = offsets.clone();
                path.push(key.clone());
                let root = super::read_variable(variable, variables, bindings, globals, global_scope);
                let (updated, _) = root.ensure_array_reference(&path)?;
                write_variable(variable.clone(), updated, variables, bindings, globals, global_scope)?;
            }
            let entries = match super::read_variable(variable, variables, bindings, globals, global_scope).array_get_path(&offsets)? {
                Value::Array(entries) => entries.into_iter().collect(),
                Value::Null => Vec::new(),
                _ => return Err("foreach expects an array".into()),
            };
            iterators.insert(*id, ForeachIterator::Array(entries, 0));
        }
        Op::IterNext(id, key_name, value_name, by_reference, target) => {
            let current = match iterators.get_mut(id).ok_or("Invalid foreach iterator")? {
                ForeachIterator::Array(entries, index) if *index < entries.len() => {
                    let (key, value) = entries[*index].clone();
                    *index += 1;
                    Some((array_key_value(key), value))
                }
                ForeachIterator::Array(_, _) => None,
                ForeachIterator::Generator(generator, use_current) if *use_current => {
                    *use_current = false;
                    super::current_generator_entry(generator)?
                }
                ForeachIterator::Generator(generator, _) => super::advance_generator(
                    generator, functions, classes, static_properties, globals, output, steps, Value::Null,
                )?,
                ForeachIterator::UserIterator { object, advance_pending } => {
                    if *advance_pending {
                        let _ = call_iterator_method(object.clone(), "next", stack, variables, bindings, globals,
                            global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                        *advance_pending = false;
                    }
                    let valid = call_iterator_method(object.clone(), "valid", stack, variables, bindings, globals,
                        global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                    if !valid.truthy() { None }
                    else {
                        let value = call_iterator_method(object.clone(), "current", stack, variables, bindings, globals,
                            global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                        let key = call_iterator_method(object.clone(), "key", stack, variables, bindings, globals,
                            global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                        *advance_pending = true;
                        Some((key, value))
                    }
                }
            };
            if let Some((key, value)) = current {
                if *by_reference && matches!(iterators.get(id), Some(ForeachIterator::Generator(_, _))) {
                    return Err("Cannot iterate a Generator by reference".into());
                }
                if let Some(name) = key_name {
                    write_variable(name.clone(), key, variables, bindings, globals, global_scope)?;
                }
                if *by_reference {
                    let Value::Reference(reference) = &value else { return Err("Invalid by-reference foreach entry".into()); };
                    let storage_key = globals.store_reference(reference.clone());
                    match bindings.get(value_name).cloned() {
                        Some(VariableBinding::Global(name)) | Some(VariableBinding::Static(name)) => globals.bind_alias(name, storage_key),
                        None if global_scope => globals.bind_alias(value_name.clone(), storage_key),
                        _ => {
                            variables.remove(value_name);
                            bindings.insert(value_name.clone(), VariableBinding::Reference(storage_key));
                        }
                    }
                } else {
                    write_variable(value_name.clone(), value.dereference(), variables, bindings, globals, global_scope)?;
                }
            } else {
                *program_counter = *target;
            }
        }
        Op::UnsetVariable(name) => {
            let old_value = read_variable(name, variables, bindings, globals, global_scope).dereference();
            remove_variable(name, variables, bindings, globals, global_scope)?;
            super::objects::run_destructors_removed_from_value(old_value, &Value::Null, functions, classes, static_properties, globals, output, steps)?;
        }
        Op::Jump(target) => *program_counter = *target,
        Op::DynamicLoopJump { targets, is_break: _ } => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let level = value.number();
            if !level.is_finite() || level < 1.0 || level.fract() != 0.0 || level > targets.len() as f64 {
                return Err("break/continue level must be a positive integer not exceeding the active loop/switch depth".into());
            }
            let target = targets[level as usize - 1];
            if target == usize::MAX { return Err("Invalid break/continue target".into()); }
            *program_counter = target;
        }
        Op::JumpFalse(target) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            if !value.truthy() { *program_counter = *target; }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn array_key_value(key: ArrayKey) -> Value {
    match key {
        ArrayKey::Integer(number) => Value::Integer(number),
        ArrayKey::String(text) => Value::String(text),
    }
}
