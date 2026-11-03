//! Function, instance-method, static-method, and magic-call dispatch.

use std::{collections::{HashMap, HashSet}, path::PathBuf, rc::Rc};

use crate::{bytecode::{Op, ReferenceArgument, ReferenceDescriptor}, ArrayKey, BytecodeClass, BytecodeFunction, MemberVisibility, PhpArray, PhpClosure, Value};
use super::objects::{ensure_access, find_method, find_property_in_scope, instance_property_storage_key};
use super::{complete_arguments, execute_code, function_scope, read_variable, reference_storage_key, validate_arguments, validate_return_type, write_array_root, write_variable, GlobalVariables, StaticProperties, VariableBinding};

fn enum_scalar_equal(left: &Value, right: &Value) -> bool {
    match (left.dereference(), right.dereference()) {
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        _ => false,
    }
}

fn reflection_attribute_arguments(arguments: &[(Option<String>, Value)]) -> Value {
    let mut positional = 0i64;
    Value::from_array_entries(arguments.iter().map(|(name, value)| {
        let key = match name {
            Some(name) => ArrayKey::String(name.clone()),
            None => { let key = ArrayKey::Integer(positional); positional += 1; key }
        };
        (key, value.clone())
    }).collect())
}

fn instantiate_reflection_attribute(
    properties: &HashMap<String, Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let class_name = properties.get("\0attribute_name").ok_or("Invalid ReflectionAttribute")?.show();
    let declaration = classes.get(&class_name.to_ascii_lowercase()).ok_or_else(|| format!("Attribute class {class_name} does not exist"))?;
    let marker = declaration.attributes.iter().find(|attribute| attribute.name.eq_ignore_ascii_case("Attribute"))
        .ok_or_else(|| format!("Attempting to use non-attribute class {class_name} as an attribute"))?;
    let flags = marker.arguments.first().and_then(|(name, value)| name.is_none().then_some(value.number() as i64)).unwrap_or(63);
    let target_type = properties.get("\0target_type").map_or(1, |value| value.number() as i64);
    if flags & target_type == 0 { return Err(format!("Attribute class {class_name} cannot target this declaration")); }
    let target_class = properties.get("\0target_class").ok_or("Invalid ReflectionAttribute target")?.show();
    let repeated = properties.get("\0is_repeated").is_some_and(Value::truthy) || if target_type == 2 {
        functions.get(&target_class.to_ascii_lowercase()).is_some_and(|target| target.attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name)).count() > 1)
    } else { classes.get(&target_class.to_ascii_lowercase()).is_some_and(|target| {
        if target_type == 4 {
            let method = properties.get("\0target_member").map(Value::show).unwrap_or_default();
            target.method_attributes.get(&method.to_ascii_lowercase()).is_some_and(|attributes| attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name)).count() > 1)
        } else if target_type == 8 {
            let property = properties.get("\0target_member").map(Value::show).unwrap_or_default();
            target.property_attributes.get(&property).is_some_and(|attributes| attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name)).count() > 1)
        } else if target_type == 32 {
            let method = properties.get("\0target_member").map(Value::show).unwrap_or_default();
            let position = properties.get("\0target_position").map_or(usize::MAX, |value| value.number() as usize);
            target.parameter_attributes.get(&method.to_ascii_lowercase()).and_then(|items| items.get(position)).is_some_and(|attributes| attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name)).count() > 1)
        } else {
            target.attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name)).count() > 1
        }
    }) };
    if repeated && flags & 64 == 0 { return Err(format!("Attribute {class_name} must not be repeated")); }
    let argument_array = properties.get("\0attribute_arguments").ok_or("Invalid ReflectionAttribute arguments")?;
    let Value::Array(entries) = argument_array.dereference() else { return Err("Invalid ReflectionAttribute arguments".into()); };
    let mut arguments = Vec::with_capacity(entries.len());
    let mut argument_names = Vec::with_capacity(entries.len());
    for (key, value) in entries.iter() {
        argument_names.push(match key { ArrayKey::Integer(_) => None, ArrayKey::String(name) => Some(name.clone()) });
        arguments.push(value.clone());
    }
    let mut stack = arguments;
    let instruction = Op::NewObject(class_name, stack.len(), vec![None; stack.len()], argument_names, vec![false; stack.len()], None, Some(1), false);
    let mut program_counter = 0;
    super::objects::execute_instruction(
        &instruction, &mut stack, variables, bindings, global_variables, global_scope,
        frame_id, "<main>", functions, classes, static_properties, output, steps, &mut program_counter,
    )?;
    stack.pop().ok_or_else(|| "Invalid ReflectionAttribute construction result".into())
}

pub(super) fn take_call_arguments(
    stack: &mut Vec<Value>,
    descriptors: &[Option<ReferenceDescriptor>],
    argument_names: &[Option<String>],
    unpack: &[bool],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(Vec<Value>, Vec<Option<ReferenceArgument>>, Vec<Option<String>>), String> {
    let width = descriptors.iter().map(|descriptor| descriptor.as_ref().map_or(1, |target| 1 + target.array_key_count)).sum::<usize>();
    if stack.len() < width { return Err("Invalid bytecode stack".into()); }
    let operands = stack.split_off(stack.len() - width);
    let mut operands = operands.into_iter();
    let mut arguments = Vec::with_capacity(descriptors.len());
    let mut references = Vec::with_capacity(descriptors.len());
    let mut names = Vec::with_capacity(descriptors.len());
    for (index, descriptor) in descriptors.iter().enumerate() {
        if let Some(descriptor) = descriptor {
            let root_or_value = operands.next().ok_or("Invalid bytecode stack")?;
            if descriptor.direct_reference {
                let Value::Reference(reference) = root_or_value else { return Err("Argument must be passed by reference".into()); };
                let keys = (0..descriptor.array_key_count)
                    .map(|_| operands.next().ok_or("Invalid bytecode stack")?.array_key())
                    .collect::<Result<Vec<_>, _>>()?;
                let direct_reference = if keys.is_empty() {
                    reference
                } else {
                    let (updated, element_reference) = reference.borrow().clone().ensure_array_reference(&keys)?;
                    *reference.borrow_mut() = updated;
                    element_reference
                };
                arguments.push(direct_reference.borrow().dereference());
                references.push(Some(ReferenceArgument { variable: String::new(), array_keys: Vec::new(), object_property: None, direct_reference: Some(direct_reference) }));
                names.push(argument_names.get(index).cloned().flatten());
                continue;
            }
            let keys = (0..descriptor.array_key_count)
                .map(|_| operands.next().ok_or("Invalid bytecode stack")?.array_key())
                .collect::<Result<Vec<_>, _>>()?;
            let (value, object_property) = if let Some(property) = &descriptor.object_property {
                let Value::Object(object) = &root_or_value else { return Err("Attempt to pass a property on a non-object by reference".into()); };
                let class_name = object.borrow().class_name.clone();
                let metadata = find_property_in_scope(&class_name, property, descriptor.access_scope.as_deref(), classes);
                if let Some(metadata) = metadata {
                    ensure_access(metadata.visibility, Some(&metadata.declaring_class), descriptor.access_scope.as_deref(), classes, property)?;
                    if metadata.is_readonly {
                        return Err(format!("Cannot indirectly modify readonly property {class_name}::${property}"));
                    }
                    if metadata.type_name.is_some() {
                        return Err(format!("Typed property {class_name}::${property} must not be referenced"));
                    }
                }
                let storage_key = metadata.map_or_else(|| property.clone(), |metadata| instance_property_storage_key(metadata, property));
                let current = object.borrow().properties.get(&storage_key).cloned().unwrap_or(Value::Null);
                (if keys.is_empty() { current } else { current.array_get_path(&keys)? }, Some((root_or_value.clone(), storage_key)))
            } else {
                let resolved = if keys.is_empty() { root_or_value } else { root_or_value.array_get_path(&keys)? };
                (resolved, None)
            };
            arguments.push(value);
            references.push(Some(ReferenceArgument { variable: descriptor.variable.clone(), array_keys: keys, object_property, direct_reference: None }));
        } else {
            let value = operands.next().ok_or("Invalid bytecode stack")?;
            if unpack.get(index).copied().unwrap_or(false) {
                let Value::Array(array) = value.dereference() else { return Err("Only arrays can be unpacked into arguments".into()); };
                for (key, value) in array.iter() {
                    let name = match key { ArrayKey::Integer(_) => None, ArrayKey::String(name) => Some(name.clone()) };
                    if let Value::Reference(reference) = value {
                        arguments.push(reference.borrow().dereference());
                        references.push(Some(ReferenceArgument {
                            variable: String::new(), array_keys: Vec::new(), object_property: None,
                            direct_reference: Some(reference.clone()),
                        }));
                    } else {
                        arguments.push(value.dereference());
                        references.push(None);
                    }
                    names.push(name);
                }
                continue;
            }
            arguments.push(value);
            references.push(None);
        }
        names.push(argument_names.get(index).cloned().flatten());
    }
    Ok((arguments, references, names))
}

fn collect_traversable(
    value: Value,
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
    depth: usize,
    ignore_iterator_keys: bool,
    skip_invalid_iterator_keys: bool,
) -> Result<Vec<(ArrayKey, Value)>, String> {
    if depth > 32 { return Err("IteratorAggregate nesting is too deep".into()); }
    match value.dereference() {
        Value::Array(entries) => Ok(entries.into_iter().collect()),
        Value::Generator(generator) => {
            let mut rows = Vec::new();
            let mut current = if super::rewind_generator(&generator, functions, classes, static_properties, globals, output, steps)? {
                super::current_generator_entry(&generator)?
            } else { None };
            while let Some((key, value)) = current {
                if ignore_iterator_keys {
                    rows.push((ArrayKey::Integer(rows.len() as i64), value));
                } else {
                    match key.array_key() {
                        Ok(key) => rows.push((key, value)),
                        Err(_) if skip_invalid_iterator_keys => globals.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Warning,
                            message: "Illegal offset type".into(),
                            source_file: None,
                            source_line: None,
                        }),
                        Err(error) => return Err(error),
                    }
                }
                current = super::advance_generator(&generator, functions, classes, static_properties, globals, output, steps, Value::Null)?;
            }
            Ok(rows)
        }
        Value::Object(object) if object.borrow().class_name.eq_ignore_ascii_case("PDOStatement") => {
            pdo_iterator_rows(&Value::Object(object), variables, bindings, globals, global_scope, frame_id,
                functions, classes, static_properties, output, steps)
        }
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            let normalized = class_name.to_ascii_lowercase();
            if super::is_a_type(&normalized, "iteratoraggregate", classes, &mut std::collections::HashSet::new()) {
                let mut call_stack = Vec::new();
                let iterator = super::control::call_iterator_method(Value::Object(object), "getiterator", &mut call_stack,
                    variables, bindings, globals, global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                if matches!(iterator.dereference(), Value::Array(_)) {
                    return Err("IteratorAggregate::getIterator() must return a Traversable object".into());
                }
                return collect_traversable(iterator, variables, bindings, globals, global_scope, frame_id,
                    functions, classes, static_properties, output, steps, depth + 1, ignore_iterator_keys, skip_invalid_iterator_keys);
            }
            if !super::is_a_type(&normalized, "iterator", classes, &mut std::collections::HashSet::new()) {
                return Err(format!("Object of type {class_name} is not Traversable"));
            }
            let required = ["rewind", "valid", "current", "key", "next"];
            if required.iter().any(|method| super::objects::find_method(&class_name, method, classes).is_none()) {
                return Err(format!("Object of type {class_name} does not provide the Iterator methods"));
            }
            let iterator = Value::Object(object);
            let mut call_stack = Vec::new();
            let call = |method: &str, call_stack: &mut Vec<Value>, variables: &mut HashMap<String, Value>,
                bindings: &mut HashMap<String, VariableBinding>, globals: &mut GlobalVariables,
                static_properties: &mut StaticProperties, output: &mut String, steps: &mut u64|
                super::control::call_iterator_method(iterator.clone(), method, call_stack, variables, bindings,
                    globals, global_scope, frame_id, functions, classes, static_properties, output, steps);
            let _ = call("rewind", &mut call_stack, variables, bindings, globals, static_properties, output, steps)?;
            let mut rows = Vec::new();
            loop {
                if !call("valid", &mut call_stack, variables, bindings, globals, static_properties, output, steps)?.truthy() { break; }
                let value = call("current", &mut call_stack, variables, bindings, globals, static_properties, output, steps)?;
                let key = if ignore_iterator_keys {
                    Some(ArrayKey::Integer(rows.len() as i64))
                } else {
                    let value = call("key", &mut call_stack, variables, bindings, globals, static_properties, output, steps)?;
                    match value.array_key() {
                        Ok(key) => Some(key),
                        Err(error) if skip_invalid_iterator_keys => {
                            globals.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Warning,
                                message: "Illegal offset type".into(),
                                source_file: None,
                                source_line: None,
                            });
                            None
                        }
                        Err(error) => return Err(error),
                    }
                };
                if let Some(key) = key { rows.push((key, value)); }
                let _ = call("next", &mut call_stack, variables, bindings, globals, static_properties, output, steps)?;
            }
            Ok(rows)
        }
        _ => Err("Value is not Traversable|array".into()),
    }
}

pub(super) fn callback_call(callback: Value, arguments: Value, strict_types: bool) -> Result<(Op, Vec<Value>), String> {
    match callback {
        Value::String(name) => {
            if let Some((class_name, method)) = name.split_once("::") {
                Ok((
                    Op::CallStaticMethod(class_name.to_ascii_lowercase(), method.to_ascii_lowercase(), 1,
                        vec![None], vec![None], vec![true], None, strict_types),
                    vec![arguments],
                ))
            } else {
                Ok((Op::Call(name.to_ascii_lowercase(), vec![None], vec![None], vec![true], strict_types), vec![arguments]))
            }
        }
        Value::Closure(closure) => Ok((Op::CallValue(1, vec![None], vec![None], vec![true], strict_types), vec![Value::Closure(closure), arguments])),
        Value::Array(callable) => Ok((Op::CallValue(1, vec![None], vec![None], vec![true], strict_types), vec![Value::Array(callable), arguments])),
        Value::Object(object) => Ok((
            Op::CallMethod("__invoke".into(), 1, vec![None], vec![None], vec![true], None, strict_types),
            vec![Value::Object(object), arguments],
        )),
        _ => Err("Callback must be callable".into()),
    }
}

/// Execute user/builtin function calls and instance/static/parent method calls.
pub(super) fn execute_instruction(
    instruction: &Op,
    stack: &mut Vec<Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<bool, String> {
    match instruction {
        Op::ExpandUnpack => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let entries = collect_traversable(value, variables, bindings, global_variables, global_scope, frame_id,
                functions, classes, static_properties, output, steps, 0, false, false).map_err(|error| {
                    if error == "Value is not Traversable|array" { "Only arrays and Traversable objects can be unpacked".into() } else { error }
                })?;
            stack.push(Value::from_array_entries(entries));
        }
        Op::Call(name, descriptors, argument_names, unpack, strict_types) => {
            let (arguments, by_reference, argument_names) = take_call_arguments(stack, descriptors, argument_names, unpack, classes)?;
            let normalized_name = name.to_ascii_lowercase();
            if functions.contains_key(&normalized_name) {
                let result = invoke_with_references(&normalized_name, arguments, &by_reference, &argument_names, *strict_types, variables, bindings, global_variables, global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                stack.push(result);
            } else {
                // Unqualified names inside a namespace may fall back to a global function or builtin.
                let fallback_name = name.rsplit('\\').next().unwrap_or(name);
                if functions.contains_key(fallback_name) {
                    let result = invoke_with_references(fallback_name, arguments, &by_reference, &argument_names, *strict_types, variables, bindings, global_variables, global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                    stack.push(result);
                    return Ok(true);
                }
                if matches!(fallback_name.to_ascii_lowercase().as_str(), "func_get_args" | "func_get_arg" | "func_num_args") {
                    let (arguments, _) = super::builtin_arguments::bind(fallback_name, arguments, &by_reference, &argument_names)?;
                    let call_arguments = current_function_arguments(variables, bindings, global_variables, global_scope)?;
                    let value = match fallback_name.to_ascii_lowercase().as_str() {
                        "func_num_args" if arguments.is_empty() => Value::Integer(call_arguments.len() as i64),
                        "func_get_args" if arguments.is_empty() => Value::from_array_entries(call_arguments.into_iter().enumerate()
                            .map(|(index, value)| (ArrayKey::Integer(index as i64), value)).collect()),
                        "func_get_arg" if arguments.len() == 1 => {
                            let index = arguments[0].number() as i64;
                            if index < 0 { Value::Bool(false) } else { call_arguments.get(index as usize).cloned().unwrap_or(Value::Bool(false)) }
                        }
                        "func_get_arg" => return Err("func_get_arg() expects exactly 1 argument".into()),
                        "func_get_args" | "func_num_args" => return Err(format!("{}() expects no arguments", fallback_name)),
                        _ => unreachable!(),
                    };
                    stack.push(value);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("call_user_func") {
                    let mut callback = None;
                    let mut forwarded = Vec::<(Value, Option<String>)>::new();
                    let mut positional_seen = false;
                    let mut named_seen = false;
                    for (index, value) in arguments.into_iter().enumerate() {
                        let name = argument_names.get(index).cloned().flatten();
                        if name.as_deref() == Some("callback") {
                            if callback.replace(value).is_some() {
                                return Err("call_user_func(): argument '$callback' is passed more than once".into());
                            }
                            named_seen = true;
                        } else if let Some(name) = name {
                            named_seen = true;
                            forwarded.push((value, Some(name)));
                        } else {
                            if named_seen { return Err("call_user_func(): positional argument follows a named argument".into()); }
                            if !positional_seen {
                                positional_seen = true;
                                callback = Some(value);
                            } else {
                                forwarded.push((value, None));
                            }
                        }
                    }
                    let callback = callback.ok_or("call_user_func(): argument '$callback' is required")?;
                    let mut next_index = 0i64;
                    let entries = forwarded.into_iter().map(|(value, name)| {
                        let key = if let Some(name) = name { ArrayKey::String(name) } else {
                            let key = ArrayKey::Integer(next_index);
                            next_index += 1;
                            key
                        };
                        let reference = Rc::new(std::cell::RefCell::new(value.dereference()));
                        (key, Value::Reference(reference))
                    }).collect();
                    let arguments = Value::from_array_entries(entries);
                    let (instruction, mut callback_stack) = callback_call(callback, arguments, false)?;
                    execute_instruction(
                        &instruction, &mut callback_stack, variables, bindings, global_variables, global_scope,
                        frame_id, functions, classes, static_properties, output, steps,
                    )?;
                    stack.push(callback_stack.pop().ok_or("Invalid callback result stack")?);
                    return Ok(true);
                }
                let csv_escape_omitted = fallback_name.eq_ignore_ascii_case("str_getcsv")
                    && arguments.len() < 4
                    && !argument_names.iter().flatten().any(|name| name.eq_ignore_ascii_case("escape"));
                let similar_text_percent_provided = fallback_name.eq_ignore_ascii_case("similar_text")
                    && (arguments.len() >= 3 || argument_names.iter().flatten().any(|name| name.eq_ignore_ascii_case("percent")));
                if fallback_name.eq_ignore_ascii_case("strtok") && arguments.len() == 1 && !argument_names.iter().any(Option::is_some) {
                    let mut arguments = arguments;
                    coerce_stringable_builtin_args("strtok", &mut arguments, crate::string_builtins::string_argument_positions("strtok"), functions, classes, static_properties, global_variables, output, steps)?;
                    stack.push(strtok_with_state(&arguments, global_variables)?);
                    return Ok(true);
                }
                let (mut arguments, by_reference) = super::builtin_arguments::bind(
                    fallback_name, arguments, &by_reference, &argument_names,
                )?;
                if fallback_name.eq_ignore_ascii_case("similar_text") {
                    coerce_stringable_builtin_args("similar_text", &mut arguments, crate::string_builtins::string_argument_positions("similar_text"), functions, classes, static_properties, global_variables, output, steps)?;
                    let (matches, percent) = crate::string_builtins::similar_text_with_percent(&arguments)?;
                    if similar_text_percent_provided {
                        let target = by_reference.get(2).and_then(Option::as_ref)
                            .ok_or("TypeError:similar_text(): Argument #3 ($percent) cannot be passed by reference")?;
                        write_reference_argument(target, Value::Number(percent), variables, bindings, global_variables, global_scope)?;
                    }
                    stack.push(matches);
                    return Ok(true);
                }
                if csv_escape_omitted {
                    global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                        level: crate::DiagnosticLevel::Deprecated,
                        message: "str_getcsv(): the $escape parameter must be provided as its default value will change".into(),
                        source_file: None,
                        source_line: None,
                    });
                }
                if fallback_name.eq_ignore_ascii_case("error_reporting") {
                    if arguments.len() > 1 { return Err("ArgumentCountError:error_reporting() expects at most 1 argument".into()); }
                    let previous = global_variables.error_reporting.unwrap_or(32767);
                    if let Some(level) = arguments.first() { global_variables.error_reporting = Some(level.number() as i64); }
                    stack.push(Value::Integer(previous));
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("error_get_last") {
                    if !arguments.is_empty() { return Err("ArgumentCountError:error_get_last() expects exactly 0 arguments".into()); }
                    let value = global_variables.last_error.as_ref().map_or(Value::Null, |diagnostic| {
                        let mut entries = vec![
                            (crate::ArrayKey::String("type".into()), Value::Integer(match diagnostic.level {
                                crate::DiagnosticLevel::Warning => 2,
                                crate::DiagnosticLevel::Deprecated => 8192,
                            })),
                            (crate::ArrayKey::String("message".into()), Value::String(diagnostic.message.clone())),
                            (crate::ArrayKey::String("file".into()), Value::String(diagnostic.source_file.clone().unwrap_or_else(|| "Command line code".into()))),
                            (crate::ArrayKey::String("line".into()), Value::Integer(diagnostic.source_line.unwrap_or(0) as i64)),
                        ];
                        Value::from_array_entries(std::mem::take(&mut entries))
                    });
                    stack.push(value);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("error_clear_last") {
                    if !arguments.is_empty() { return Err("ArgumentCountError:error_clear_last() expects exactly 0 arguments".into()); }
                    global_variables.last_error = None;
                    stack.push(Value::Null);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("strtok") {
                    coerce_stringable_builtin_args("strtok", &mut arguments, crate::string_builtins::string_argument_positions("strtok"), functions, classes, static_properties, global_variables, output, steps)?;
                    stack.push(strtok_with_state(&arguments, global_variables)?);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("iterator_count") {
                    stack.push(iterator_count(&arguments, variables, bindings, global_variables, global_scope, frame_id,
                        functions, classes, static_properties, output, steps)?);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("iterator_to_array") {
                    stack.push(iterator_to_array(&arguments, variables, bindings, global_variables, global_scope, frame_id,
                        functions, classes, static_properties, output, steps)?);
                    return Ok(true);
                }
                match fallback_name.to_ascii_lowercase().as_str() {
                    "abs" | "ceil" | "floor" | "round" | "sqrt" | "fdiv" | "fmod" | "intdiv" | "is_finite" | "is_infinite" | "is_nan" | "exp" | "expm1" | "log" | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "atan2" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh" | "deg2rad" | "rad2deg" | "hypot" | "max" | "min" | "base_convert" | "decbin" | "dechex" | "decoct" => {
                        stack.push(crate::math_builtins::call(fallback_name, &arguments)?);
                    }
                    "mongo_find" | "mongo_aggregate" | "mongo_count_documents" | "mongo_estimated_document_count" | "mongo_insert_one" | "mongo_insert_many" | "mongo_replace_one" | "mongo_update_one" | "mongo_update_many" | "mongo_delete_one" | "mongo_delete_many" | "mongo_begin_transaction" | "mongo_commit" | "mongo_rollback" => {
                        stack.push(crate::mongo_php::call(&fallback_name.to_ascii_lowercase(), &arguments)?);
                    }
                    "function_exists" => {
                        let name = arguments.first().ok_or("function_exists() expects exactly 1 argument")?.show();
                        stack.push(Value::Bool(runtime_function_exists(&name, functions)));
                    }
                    "php_sapi_name" => {
                        if !arguments.is_empty() { return Err("php_sapi_name() expects exactly 0 arguments".into()); }
                        stack.push(global_variables.get("\0sapi_name").cloned().unwrap_or_else(|| Value::String("cli".into())));
                    }
                    "var_dump" => {
                        if arguments.is_empty() { return Err("var_dump() expects at least 1 argument, 0 given".into()); }
                        let mut seen = HashSet::new();
                        for value in &arguments {
                            let dump = format_dump_value(value, 0, &mut seen, functions, classes, static_properties, global_variables, output, steps)?;
                            output.push_str(&dump);
                        }
                        stack.push(Value::Null);
                    }
                    "intval" | "floatval" | "strval" | "boolval" | "gettype" | "get_debug_type" | "is_int" | "is_integer" | "is_long" | "is_float" | "is_double" | "is_real"
                    | "is_string" | "is_array" | "is_countable" | "is_iterable" | "is_bool" | "is_null" | "is_object" | "is_resource" | "is_scalar" | "is_numeric" => {
                        if fallback_name.eq_ignore_ascii_case("strval") {
                            let value = arguments.first().ok_or("strval() expects exactly 1 argument")?.dereference();
                            let string = super::objects::stringify(&value, functions, classes, static_properties, global_variables, output, steps)?;
                            stack.push(Value::String(string));
                        } else {
                            stack.push(type_builtin(&fallback_name.to_ascii_lowercase(), &arguments, classes)?);
                        }
                    }
                    "class_exists" | "interface_exists" | "trait_exists" => {
                        let name = arguments.first().ok_or("class_exists() expects at least 1 argument")?.show();
                        let key = name.trim_start_matches('\\').to_ascii_lowercase();
                        let built_in_class = matches!(key.as_str(), "generator" | "pdo" | "pdostatement" | "reflectionclass" | "reflectionenum" | "reflectionenumunitcase" | "reflectionenumbackedcase" | "reflectionnamedtype" | "reflectiontype" | "reflectionfunction" | "reflectionmethod" | "reflectionproperty" | "reflectionattribute" | "reflectionparameter" | "attribute" | "mongodb\\client" | "mongodb\\database" | "mongodb\\collection" | "mongodb\\cursor" | "mongodb\\insertresult" | "mongodb\\insertmanyresult" | "mongodb\\updateresult" | "mongodb\\deleteresult");
                        let wants_interface = fallback_name.eq_ignore_ascii_case("interface_exists");
                        let wants_trait = fallback_name.eq_ignore_ascii_case("trait_exists");
                        let exists = if wants_trait {
                            classes.get(&key).is_some_and(|class| class.is_trait)
                        } else if wants_interface {
                            crate::exception_builtins::is_interface(&key)
                        } else {
                            crate::exception_builtins::is_class(&key) || built_in_class
                        } || classes.get(&key).is_some_and(|class| if wants_interface { class.is_interface } else { !class.is_interface && !class.is_trait });
                        stack.push(Value::Bool(exists));
                    }
                    "get_class" | "get_parent_class" => {
                        let wants_parent = fallback_name.eq_ignore_ascii_case("get_parent_class");
                        let target = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null));
                        let class_key = match target.map(Value::dereference) {
                            Some(Value::Object(object)) => Some(object.borrow().class_name.to_ascii_lowercase()),
                            Some(Value::Generator(_)) => Some("generator".into()),
                            Some(Value::String(name)) if wants_parent => Some(name.trim_start_matches('\\').to_ascii_lowercase()),
                            Some(_) => return Err(if wants_parent {
                                "get_parent_class(): Argument #1 must be an object or a class name".into()
                            } else {
                                "get_class(): Argument #1 must be an object".into()
                            }),
                            None => match variables.get("this") {
                                Some(Value::Object(object)) => Some(object.borrow().class_name.to_ascii_lowercase()),
                                _ => variables.get("static_class").map(Value::show).map(|name| name.to_ascii_lowercase()),
                            },
                        };
                        let result = class_key.and_then(|key| {
                            if let Some(class) = classes.get(&key) {
                                if wants_parent {
                                    class.parent.as_ref().and_then(|parent| classes.get(parent)).map(|parent| parent.name.clone())
                                } else {
                                    Some(class.name.clone())
                                }
                            } else if !wants_parent {
                                match key.as_str() {
                                    "pdo" => Some("PDO".into()),
                                    "pdostatement" => Some("PDOStatement".into()),
                                    "generator" => Some("Generator".into()),
                                    "stdclass" => Some("stdClass".into()),
                                    _ => crate::exception_builtins::canonical_name(&key).map(str::to_string),
                                }
                            } else if crate::exception_builtins::is_class(&key) {
                                crate::exception_builtins::parent_class(&key).map(str::to_string)
                            } else { None }
                        });
                        stack.push(result.map(Value::String).unwrap_or(Value::Bool(false)));
                    }
                    "get_called_class" => {
                        if !arguments.is_empty() {
                            return Err(format!("get_called_class() expects exactly 0 arguments, {} given", arguments.len()));
                        }
                        let called_class = variables.get("static_class").map(Value::show)
                            .ok_or_else(|| "Error:get_called_class() must be called from within a class".to_string())?;
                        stack.push(Value::String(classes.get(&called_class.to_ascii_lowercase()).map(|class| class.name.clone()).unwrap_or(called_class)));
                    }
                    "is_a" | "is_subclass_of" => {
                        let is_subclass = fallback_name.eq_ignore_ascii_case("is_subclass_of");
                        let function_name = if is_subclass { "is_subclass_of" } else { "is_a" };
                        let value = arguments.first().ok_or_else(|| format!("{function_name}() expects at least 2 arguments"))?.dereference();
                        let expected = arguments.get(1).ok_or_else(|| format!("{function_name}() expects at least 2 arguments"))?.show().trim_start_matches('\\').to_ascii_lowercase();
                        let allow_string = arguments.get(2).is_some_and(Value::truthy);
                        let actual = match value {
                            Value::Object(object) => Some(object.borrow().class_name.to_ascii_lowercase()),
                            Value::Generator(_) => Some("generator".into()),
                            Value::String(name) if allow_string => Some(name.trim_start_matches('\\').to_ascii_lowercase()),
                            _ => None,
                        };
                        let matches = actual.is_some_and(|actual| {
                            let same_class = actual.eq_ignore_ascii_case(&expected);
                            if is_subclass && same_class { return false; }
                            super::is_a_type(&actual, &expected, classes, &mut std::collections::HashSet::new())
                                || !is_subclass && matches!(actual.as_str(), "generator" | "pdo" | "pdostatement" | "stdclass") && same_class
                        });
                        stack.push(Value::Bool(matches));
                    }
                    "method_exists" => {
                        let target = arguments.first().ok_or("method_exists() expects exactly 2 arguments")?;
                        let method = arguments.get(1).ok_or("method_exists() expects exactly 2 arguments")?.show().to_ascii_lowercase();
                        let class_name = match target.dereference() {
                            Value::Object(object) => object.borrow().class_name.clone(),
                            Value::Generator(_) => "generator".into(),
                            Value::String(name) => name.trim_start_matches('\\').to_ascii_lowercase(),
                            _ => String::new(),
                        };
                        let generator_method = matches!(method.as_str(), "current" | "getreturn" | "key" | "next" | "rewind" | "send" | "throw" | "valid");
                        let reflection_method = match class_name.to_ascii_lowercase().as_str() {
                            "reflectionclass" => matches!(method.as_str(), "getname" | "getattributes" | "getmethod" | "getproperty" | "getparentclass" | "isenum" | "isinterface" | "isabstract" | "isfinal" | "issubclassof" | "implementsinterface"),
                            "reflectionenum" => matches!(method.as_str(), "getname" | "getattributes" | "getmethod" | "getcases" | "isbacked" | "getbackingtype"),
                            "reflectionnamedtype" => matches!(method.as_str(), "getname" | "isbuiltin" | "allowsnull"),
                            "reflectionenumunitcase" => matches!(method.as_str(), "getname" | "getvalue"),
                            "reflectionenumbackedcase" => matches!(method.as_str(), "getname" | "getvalue" | "getbackingvalue"),
                            "reflectionmethod" => matches!(method.as_str(), "getname" | "getattributes" | "getparameters"),
                            "reflectionparameter" => matches!(method.as_str(), "getname" | "getposition" | "getattributes"),
                            "reflectionfunction" => matches!(method.as_str(), "getname" | "getattributes" | "getparameters"),
                            "reflectionproperty" => matches!(method.as_str(), "getname" | "getattributes"),
                            "reflectionattribute" => matches!(method.as_str(), "getname" | "getarguments" | "gettarget" | "isrepeated" | "newinstance"),
                            _ => false,
                        };
                        let enum_interface_method = match class_name.to_ascii_lowercase().as_str() {
                            "unitenum" => method == "cases",
                            "backedenum" => matches!(method.as_str(), "from" | "tryfrom"),
                            _ => false,
                        };
                        let inherited_throwable_method = !class_name.is_empty()
                            && !crate::exception_builtins::is_interface(&class_name)
                            && super::is_a_type(&class_name.to_ascii_lowercase(), "throwable", classes, &mut std::collections::HashSet::new())
                            && crate::exception_builtins::method_exists("Exception", &method);
                        stack.push(Value::Bool(reflection_method || enum_interface_method || generator_method && class_name == "generator"
                            || crate::exception_builtins::method_exists(&class_name, &method)
                            || inherited_throwable_method
                            || !class_name.is_empty() && (find_method(&class_name, &method, classes).is_some() || crate::pdo::method_exists(&class_name, &method) || crate::mongo_php::method_exists(&class_name, &method))));
                    }
                    "property_exists" => {
                        let target = arguments.first().ok_or("property_exists() expects exactly 2 arguments")?.dereference();
                        let property = arguments.get(1).ok_or("property_exists() expects exactly 2 arguments")?.show();
                        let exists = match target {
                            Value::Object(object) => {
                                let class_name = object.borrow().class_name.to_ascii_lowercase();
                                declared_property_exists(&class_name, &property, classes, &mut std::collections::HashSet::new())
                                    || object.borrow().properties.contains_key(&property)
                            }
                            Value::Generator(_) => false,
                            Value::String(class_name) => declared_property_exists(&class_name, &property, classes, &mut std::collections::HashSet::new()),
                            _ => return Err("property_exists(): Argument #1 must be an object or a valid class name".into()),
                        };
                        stack.push(Value::Bool(exists));
                    }
                    "is_callable" => {
                        let value = arguments.first().ok_or("is_callable() expects exactly 1 argument")?;
                        let syntax_only = arguments.get(1).is_some_and(Value::truthy);
                        let (callable, name) = callable_status(value, syntax_only, functions, classes);
                        if let Some(Some(target)) = by_reference.get(2) {
                            write_reference_argument(target, Value::String(name), variables, bindings, global_variables, global_scope)?;
                        }
                        stack.push(Value::Bool(callable));
                    }
                    "call_user_func_array" => {
                        let callback = arguments.first().ok_or("call_user_func_array() expects exactly 2 arguments")?.clone();
                        let callback_arguments = arguments.get(1).ok_or("call_user_func_array() expects exactly 2 arguments")?.clone();
                        if !matches!(callback_arguments.dereference(), Value::Array(_)) {
                            return Err("call_user_func_array(): Argument #2 ($args) must be of type array".into());
                        }
                        let (nested_instruction, mut nested_stack) = callback_call(callback, callback_arguments, false)
                            .map_err(|_| "call_user_func_array(): Argument #1 ($callback) must be a valid callback")?;
                        execute_instruction(
                            &nested_instruction, &mut nested_stack, variables, bindings, global_variables,
                            global_scope, frame_id, functions, classes, static_properties, output, steps,
                        )?;
                        stack.push(nested_stack.pop().ok_or("Invalid callback result stack")?);
                    }
                    "header" => {
                        if arguments.is_empty() {
                            return Err("header() expects at least 1 argument".into());
                        }
                        let line = arguments[0].show();
                        if line.contains('\r') || line.contains('\n') {
                            return Err("header(): invalid header value".into());
                        }
                        if let Some((name, value)) = line.split_once(':') {
                            let name = name.trim();
                            let value = value.trim().to_string();
                            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) {
                                return Err("header(): invalid header name".into());
                            }
                            let replace = arguments.get(1).map(Value::truthy).unwrap_or(true);
                            if replace {
                                global_variables.http_headers.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
                            }
                            global_variables.http_headers.push((name.to_string(), value));
                        } else if let Some(value) = line.strip_prefix("HTTP/") {
                            if let Some(code) = value.split_whitespace().nth(1).and_then(|v| v.parse::<u16>().ok()).filter(|v| (100..=599).contains(v)) {
                                global_variables.http_status = Some(code);
                            }
                        } else {
                            return Err("header(): malformed header line".into());
                        }
                        if let Some(code) = arguments.get(2).and_then(|value| value.show().parse::<u16>().ok()).filter(|v| (100..=599).contains(v)) {
                            global_variables.http_status = Some(code);
                        }
                        stack.push(Value::Null);
                    }
                    "http_response_code" => {
                        let old = global_variables.http_status.unwrap_or(200);
                        if let Some(value) = arguments.first().filter(|value| !matches!(value, Value::Null)) {
                            let code = value.show().parse::<u16>().map_err(|_| "http_response_code(): invalid status code")?;
                            if !(100..=599).contains(&code) {
                                return Err("http_response_code(): status code must be between 100 and 599".into());
                            }
                            global_variables.http_status = Some(code);
                        }
                        stack.push(Value::Integer(i64::from(old)));
                    }
                    "header_remove" => {
                        if let Some(name) = arguments.first().filter(|value| !matches!(value, Value::Null)) {
                            let name = name.show();
                            global_variables.http_headers.retain(|(key, _)| !key.eq_ignore_ascii_case(&name));
                        } else {
                            global_variables.http_headers.clear();
                        }
                        stack.push(Value::Null);
                    }
                    "headers_list" => {
                        let entries = global_variables.http_headers.iter().enumerate().map(|(index, (name, value))| {
                            (ArrayKey::Integer(index as i64), Value::String(format!("{name}: {value}")))
                        }).collect();
                        stack.push(Value::from_array_entries(entries));
                    }
                    "file_exists" | "is_file" | "is_dir" | "is_link" | "is_readable" | "is_writable" | "is_executable" | "filesize" | "realpath" => {
                        let path = arguments.first().ok_or_else(|| format!("{}() expects a filename", fallback_name))?.show();
                        let path_ref = filesystem_path(global_variables, std::path::Path::new(&path), false);
                        let result = match path_ref.as_deref() {
                            None => Value::Bool(false),
                            Some(path_ref) => match fallback_name.to_ascii_lowercase().as_str() {
                            "file_exists" => Value::Bool(path_ref.exists()),
                            "is_file" => Value::Bool(std::fs::metadata(path_ref).is_ok_and(|metadata| metadata.is_file())),
                            "is_dir" => Value::Bool(std::fs::metadata(path_ref).is_ok_and(|metadata| metadata.is_dir())),
                            "is_link" => Value::Bool(std::fs::symlink_metadata(path_ref).is_ok_and(|metadata| metadata.file_type().is_symlink())),
                            "is_readable" => Value::Bool(filesystem_access(path_ref, FilesystemAccess::Read)),
                            "is_writable" => Value::Bool(filesystem_access(path_ref, FilesystemAccess::Write)),
                            "is_executable" => Value::Bool(filesystem_access(path_ref, FilesystemAccess::Execute)),
                            "filesize" => std::fs::metadata(path_ref).ok().and_then(|metadata| i64::try_from(metadata.len()).ok())
                                .map(Value::Integer).unwrap_or(Value::Bool(false)),
                            "realpath" => Value::String(path_ref.to_string_lossy().into_owned()),
                            _ => unreachable!(),
                            },
                        };
                        stack.push(result);
                    }
                    "getcwd" => {
                        if !arguments.is_empty() { return Err("getcwd() expects exactly 0 arguments".into()); }
                        stack.push(std::env::current_dir().ok().map(|path| Value::String(path.to_string_lossy().into_owned())).unwrap_or(Value::Bool(false)));
                    }
                    "sys_get_temp_dir" => {
                        if !arguments.is_empty() { return Err("sys_get_temp_dir() expects exactly 0 arguments".into()); }
                        stack.push(Value::String(std::env::temp_dir().to_string_lossy().into_owned()));
                    }
                    "gettimeofday" => {
                        if arguments.len() > 1 { return Err(format!("gettimeofday() expects at most 1 argument, {} given", arguments.len())); }
                        stack.push(gettimeofday_value(arguments.first().is_some_and(Value::truthy))?);
                    }
                    "time" => {
                        if !arguments.is_empty() { return Err("time() expects exactly 0 arguments".into()); }
                        stack.push(Value::Integer(unix_time().0 as i64));
                    }
                    "random_int" => {
                        if arguments.len() != 2 { return Err(format!("random_int() expects exactly 2 arguments, {} given", arguments.len())); }
                        let min = random_int_bound("random_int", &arguments[0], 1)?;
                        let max = random_int_bound("random_int", &arguments[1], 2)?;
                        if min > max { return Err("ValueError:random_int(): Argument #2 ($max) must be greater than or equal to Argument #1 ($min)".into()); }
                        stack.push(Value::Integer(secure_random_integer(min, max)?));
                    }
                    "rand" | "mt_rand" => {
                        if arguments.len() > 2 { return Err(format!("{fallback_name}() expects at most 2 arguments, {} given", arguments.len())); }
                        let min = arguments.first().map(|value| random_int_bound(&fallback_name, value, 1)).transpose()?.unwrap_or(0);
                        let max = arguments.get(1).map(|value| random_int_bound(&fallback_name, value, 2)).transpose()?.unwrap_or(2_147_483_647);
                        if min > max { return Err(format!("ValueError:{fallback_name}(): Argument #2 ($max) must be greater than or equal to Argument #1 ($min)")); }
                        stack.push(Value::Integer(secure_random_integer(min, max)?));
                    }
                    "getrandmax" | "mt_getrandmax" => {
                        if !arguments.is_empty() { return Err(format!("{fallback_name}() expects exactly 0 arguments, {} given", arguments.len())); }
                        stack.push(Value::Integer(2_147_483_647));
                    }
                    "date" | "gmdate" => {
                        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("{fallback_name}() expects 1 or 2 arguments, {} given", arguments.len())); }
                        let format = arguments[0].show();
                        let timestamp = match arguments.get(1) {
                            None | Some(Value::Null) => unix_time().0 as i64,
                            Some(value) => date_timestamp(value)?,
                        };
                        stack.push(Value::String(format_php_date(&format, timestamp, fallback_name.eq_ignore_ascii_case("gmdate"))?));
                    }
                    "getdate" => {
                        if arguments.len() > 1 { return Err(format!("getdate() expects at most 1 argument, {} given", arguments.len())); }
                        let timestamp = match arguments.first() { None | Some(Value::Null) => unix_time().0 as i64, Some(value) => date_timestamp(value)? };
                        stack.push(getdate_value(timestamp)?);
                    }
                    "localtime" => {
                        if arguments.len() > 2 { return Err(format!("localtime() expects at most 2 arguments, {} given", arguments.len())); }
                        let timestamp = match arguments.first() { None | Some(Value::Null) => unix_time().0 as i64, Some(value) => date_timestamp(value)? };
                        stack.push(localtime_value(timestamp, arguments.get(1).is_some_and(Value::truthy))?);
                    }
                    "strtotime" => {
                        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("strtotime() expects 1 or 2 arguments, {} given", arguments.len())); }
                        let datetime = arguments[0].show();
                        let base = match arguments.get(1) { None | Some(Value::Null) => unix_time().0 as i64, Some(value) => date_timestamp(value)? };
                        stack.push(parse_php_strtotime(&datetime, base).map(Value::Integer).unwrap_or(Value::Bool(false)));
                    }
                    "idate" => {
                        if arguments.is_empty() || arguments.len() > 2 { return Err(format!("idate() expects 1 or 2 arguments, {} given", arguments.len())); }
                        let format = arguments[0].show();
                        if format.chars().count() != 1 { stack.push(Value::Bool(false)); }
                        else {
                            let timestamp = match arguments.get(1) { None | Some(Value::Null) => unix_time().0 as i64, Some(value) => date_timestamp(value)? };
                            let formatted = format_php_date(&format, timestamp, false)?;
                            stack.push(formatted.parse::<i64>().map(Value::Integer).unwrap_or(Value::Bool(false)));
                        }
                    }
                    "checkdate" => {
                        if arguments.len() != 3 { return Err(format!("checkdate() expects exactly 3 arguments, {} given", arguments.len())); }
                        let month = date_component(&arguments[0]);
                        let day = date_component(&arguments[1]);
                        let year = date_component(&arguments[2]);
                        let valid = month.is_some_and(|month| (1..=12).contains(&month))
                            && day.is_some_and(|day| day >= 1)
                            && year.is_some_and(|year| (1..=32767).contains(&year));
                        let valid = valid && month.zip(day).zip(year).is_some_and(|((month, day), year)| {
                            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                            let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                            day <= days[(month - 1) as usize]
                        });
                        stack.push(Value::Bool(valid));
                    }
                    "mktime" | "gmmktime" => {
                        if arguments.len() > 6 { return Err(format!("{fallback_name}() expects at most 6 arguments, {} given", arguments.len())); }
                        let utc = fallback_name.eq_ignore_ascii_case("gmmktime");
                        let now = unix_time().0 as i64;
                        let current = date_tm(now, utc)?;
                        let defaults = [current.tm_hour as i64, current.tm_min as i64, current.tm_sec as i64,
                            (current.tm_mon + 1) as i64, current.tm_mday as i64, (current.tm_year + 1900) as i64];
                        let mut parts = defaults;
                        for (index, value) in arguments.iter().enumerate() {
                            if !matches!(value.dereference(), Value::Null) { parts[index] = date_component(value).ok_or_else(|| format!("TypeError:{fallback_name}(): Argument #{} must be of type ?int", index + 1))?; }
                        }
                        let year = match parts[5] { 0..=69 => parts[5] + 2000, 70..=100 => parts[5] + 1900, _ => parts[5] };
                        let mut tm = libc::tm {
                            tm_sec: parts[2] as libc::c_int, tm_min: parts[1] as libc::c_int, tm_hour: parts[0] as libc::c_int,
                            tm_mday: parts[4] as libc::c_int, tm_mon: (parts[3] - 1) as libc::c_int,
                            tm_year: (year - 1900) as libc::c_int, tm_wday: 0, tm_yday: 0, tm_isdst: if utc { 0 } else { -1 },
                            #[cfg(target_os = "linux")]
                            tm_gmtoff: 0,
                            #[cfg(target_os = "linux")]
                            tm_zone: std::ptr::null(),
                        };
                        let timestamp = unsafe { if utc { libc::timegm(&mut tm) } else { libc::mktime(&mut tm) } };
                        stack.push(Value::Integer(timestamp as i64));
                    }
                    "microtime" => {
                        let (seconds, microseconds) = unix_time();
                        if arguments.first().is_some_and(Value::truthy) {
                            stack.push(Value::Number(seconds as f64 + microseconds as f64 / 1_000_000.0));
                        } else {
                            stack.push(Value::String(format!("0.{microseconds:06}00 {seconds}")));
                        }
                    }
                    "hrtime" => {
                        let (seconds, nanoseconds) = high_resolution_time();
                        if arguments.first().is_some_and(Value::truthy) {
                            stack.push(i64::try_from(seconds.saturating_mul(1_000_000_000).saturating_add(nanoseconds as u64))
                                .map(Value::Integer).unwrap_or(Value::Number(seconds as f64 * 1_000_000_000.0 + nanoseconds as f64)));
                        } else {
                            stack.push(Value::from_array_entries(vec![
                                (ArrayKey::Integer(0), Value::Integer(seconds as i64)),
                                (ArrayKey::Integer(1), Value::Integer(nanoseconds as i64)),
                            ]));
                        }
                    }
                    "file_get_contents" => {
                        let path = arguments.first().ok_or("file_get_contents() expects a filename")?.show();
                        let contents = if path == "php://input" {
                            Some(global_variables.get("\0php_input").cloned().unwrap_or_else(|| Value::String(String::new())).show().into_bytes())
                        } else {
                            filesystem_path(global_variables, std::path::Path::new(&path), false).and_then(|path| std::fs::read(path).ok())
                        };
                        if let Some(bytes) = contents {
                            let requested_offset = arguments.get(3).map(|value| value.number() as i64).unwrap_or(0);
                            let start = if requested_offset < 0 { (bytes.len() as i64).saturating_add(requested_offset).max(0) as usize } else { requested_offset as usize };
                            if start > bytes.len() {
                                stack.push(Value::Bool(false));
                            } else {
                                let available = bytes.len() - start;
                                let length = arguments.get(4).filter(|value| !matches!(value.dereference(), Value::Null))
                                    .map(|value| value.number().max(0.0) as usize).unwrap_or(available).min(available);
                                stack.push(Value::String(String::from_utf8_lossy(&bytes[start..start + length]).into_owned()));
                            }
                        } else {
                            stack.push(Value::Bool(false));
                        }
                    }
                    "file" => {
                        let path = arguments.first().ok_or("file() expects a filename")?.show();
                        let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(0);
                        if let Some(path) = filesystem_path(global_variables, std::path::Path::new(&path), false) {
                        if let Ok(contents) = std::fs::read(path) {
                            let contents = String::from_utf8_lossy(&contents);
                            let mut lines = Vec::new();
                            if !contents.is_empty() {
                                for line in contents.split_inclusive('\n') {
                                    let mut line = line.to_owned();
                                    let empty = line.trim_end_matches(['\r', '\n']).is_empty();
                                    if flags & 4 != 0 && empty { continue; }
                                    if flags & 2 != 0 {
                                        if line.ends_with('\n') { line.pop(); }
                                        if line.ends_with('\r') { line.pop(); }
                                    }
                                    lines.push((ArrayKey::Integer(lines.len() as i64), Value::String(line)));
                                }
                            }
                            stack.push(Value::from_array_entries(lines));
                        } else {
                            stack.push(Value::Bool(false));
                        }
                        } else { stack.push(Value::Bool(false)); }
                    }
                    "file_put_contents" => {
                        let path = arguments.first().ok_or("file_put_contents() expects a filename")?.show();
                        let Some(data) = arguments.get(1) else { return Err("file_put_contents() expects data".into()); };
                        if let Some(bytes) = file_put_contents_bytes(data) {
                            let flags = arguments.get(2).map(|value| value.number() as i64).unwrap_or(0);
                            let result = filesystem_path(global_variables, std::path::Path::new(&path), true)
                                .and_then(|path| write_file_contents(&path, &bytes, flags).ok())
                                .and_then(|written| i64::try_from(written).ok()).map(Value::Integer).unwrap_or(Value::Bool(false));
                            stack.push(result);
                        } else {
                            stack.push(Value::Bool(false));
                        }
                    }
                    "tempnam" => {
                        let directory = arguments.first().ok_or("tempnam() expects a directory")?.show();
                        let prefix = arguments.get(1).ok_or("tempnam() expects a prefix")?.show();
                        let directory = std::path::Path::new(&directory);
                        let system_temp = std::env::temp_dir();
                        let directory = if directory.is_dir() {
                            filesystem_path(global_variables, directory, false)
                        } else if global_variables.get("\0filesystem_root").is_some() {
                            None
                        } else {
                            Some(system_temp)
                        };
                        stack.push(directory.and_then(|directory| create_temporary_file(&directory, &prefix).ok()).map(|path| Value::String(path.to_string_lossy().into_owned())).unwrap_or(Value::Bool(false)));
                    }
                    "unlink" => {
                        let path = arguments.first().ok_or("unlink() expects a filename")?.show();
                        let removed = filesystem_path(global_variables, std::path::Path::new(&path), false)
                            .is_some_and(|path| std::fs::remove_file(path).is_ok());
                        stack.push(Value::Bool(removed));
                    }
                    "fopen" => {
                        let path = arguments.first().ok_or("fopen() expects at least 2 arguments")?.show();
                        let mode = arguments.get(1).ok_or("fopen() expects at least 2 arguments")?.show();
                        let stream = if path == "php://temp" || path == "php://memory" {
                            let Some(first) = mode.chars().next() else { return Err("fopen(): invalid stream mode".into()); };
                            if !matches!(first, 'r' | 'w' | 'a' | 'x' | 'c') || mode.chars().skip(1).any(|character| !matches!(character, '+' | 'b' | 't')) { return Err("fopen(): invalid stream mode".into()); }
                            let readable = first == 'r' || mode.contains('+');
                            let writable = first != 'r' || mode.contains('+');
                            crate::PhpStream::temporary(readable, writable).ok()
                        } else {
                            let writable = mode.chars().next().is_some_and(|mode| mode != 'r') || mode.contains('+');
                            filesystem_path(global_variables, std::path::Path::new(&path), writable)
                                .and_then(|path| crate::PhpStream::open_file(path, &mode).ok())
                        };
                        stack.push(stream.map(|stream| Value::Stream(std::rc::Rc::new(std::cell::RefCell::new(stream)))).unwrap_or(Value::Bool(false)));
                    }
                    "fwrite" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference) else { return Err("fwrite() expects a stream resource".into()); };
                        let bytes = arguments.get(1).ok_or("fwrite() expects at least 2 arguments")?.show().into_bytes();
                        let length = arguments.get(2).filter(|value| !matches!(value.dereference(), Value::Null)).map(|value| value.number().max(0.0) as usize).unwrap_or(bytes.len());
                        let written = stream.borrow_mut().write(&bytes[..bytes.len().min(length)]).map_err(|error| format!("fwrite(): {error}"))?;
                        stack.push(Value::Integer(written as i64));
                    }
                    "fread" | "stream_get_contents" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference) else { return Err(format!("{}() expects a stream resource", fallback_name)); };
                        if fallback_name.eq_ignore_ascii_case("fread") && arguments.get(1).is_none() { return Err("fread() expects a length".into()); }
                        let offset = if fallback_name.eq_ignore_ascii_case("stream_get_contents") { arguments.get(2).map(|value| value.number() as i64).unwrap_or(-1) } else { -1 };
                        if offset >= 0 { stream.borrow_mut().seek(std::io::SeekFrom::Current(offset)).map_err(|error| format!("stream_get_contents(): {error}"))?; }
                        let length = arguments.get(1).map(|value| value.number() as i64).filter(|length| *length >= 0).map(|length| length as usize);
                        let bytes = stream.borrow_mut().read(length).map_err(|error| format!("{}(): {error}", fallback_name))?;
                        stack.push(Value::String(String::from_utf8_lossy(&bytes).into_owned()));
                    }
                    "rewind" | "fseek" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference) else { return Err(format!("{}() expects a stream resource", fallback_name)); };
                        let result = if fallback_name.eq_ignore_ascii_case("rewind") {
                            stream.borrow_mut().seek(std::io::SeekFrom::Start(0))
                        } else {
                            let offset = arguments.get(1).map(|value| value.number() as i64).ok_or("fseek() expects an offset")?;
                            match arguments.get(2).map(|value| value.number() as i64).unwrap_or(0) {
                                0 if offset >= 0 => stream.borrow_mut().seek(std::io::SeekFrom::Start(offset as u64)),
                                1 => stream.borrow_mut().seek(std::io::SeekFrom::Current(offset)),
                                2 => stream.borrow_mut().seek(std::io::SeekFrom::End(offset)),
                                _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid whence or negative absolute offset")),
                            }
                        };
                        if fallback_name.eq_ignore_ascii_case("rewind") { stack.push(Value::Bool(result.is_ok())); }
                        else { stack.push(Value::Integer(if result.is_ok() { 0 } else { -1 })); }
                    }
                    "ftell" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference) else { return Err("ftell() expects a stream resource".into()); };
                        let position = stream.borrow_mut().seek(std::io::SeekFrom::Current(0)).map_err(|error| format!("ftell(): {error}"))?;
                        stack.push(Value::Integer(position as i64));
                    }
                    "feof" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference) else { return Err("feof() expects a stream resource".into()); };
                        let mut stream = stream.borrow_mut();
                        let position = stream.seek(std::io::SeekFrom::Current(0)).map_err(|error| format!("feof(): {error}"))?;
                        let end = stream.seek(std::io::SeekFrom::End(0)).map_err(|error| format!("feof(): {error}"))?;
                        stream.seek(std::io::SeekFrom::Start(position)).map_err(|error| format!("feof(): {error}"))?;
                        stack.push(Value::Bool(position >= end));
                    }
                    "fclose" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference) else { return Err("fclose() expects a stream resource".into()); };
                        stack.push(Value::Bool(stream.borrow_mut().close().is_ok()));
                    }
                    "json_encode" => {
                        let value = arguments.first().ok_or("json_encode() expects exactly 1 argument")?;
                        let flags = arguments.get(1).map(|value| value.number() as i64).unwrap_or(0);
                        let depth = arguments.get(2).map(|value| value.number() as i64).unwrap_or(512);
                        let throw_on_error = flags & 4194304 != 0;
                        if depth > i32::MAX as i64 || depth < i32::MIN as i64 {
                            return Err(format!("ValueError:json_encode(): Argument #3 ($depth) must be between 1 and {}", i32::MAX));
                        }
                        if depth < 1 {
                            return Err("ValueError:json_encode(): Argument #3 ($depth) must be greater than 0".into());
                        }
                        if flags & !(4095 | 1048576 | 2097152 | 4194304) != 0 {
                            let code = 4;
                            if throw_on_error && flags & 512 == 0 {
                                return Err(json_exception_marker(global_variables, *steps, code, json_error_message(code), classes)?);
                            }
                            global_variables.json_last_error = code;
                            stack.push(Value::Bool(false));
                        } else {
                            let mut serialize = |object: &Value| -> Result<Option<Value>, String> {
                                let Value::Object(instance) = object else { return Ok(None); };
                                let class_name = instance.borrow().class_name.clone();
                                let Some(method) = super::objects::find_method(&class_name, "jsonserialize", classes) else {
                                    return Ok(None);
                                };
                                if method.is_static || !method.parameters.is_empty() || method.visibility != crate::MemberVisibility::Public {
                                    return Err(format!("{class_name}::jsonSerialize() must be a public non-static method without parameters"));
                                }
                                let mut locals = HashMap::from([
                                    ("this".to_string(), object.clone()),
                                    ("static_class".to_string(), Value::String(class_name.clone())),
                                ]);
                                let result = execute_code(&method.instructions, functions, classes, static_properties,
                                    &mut locals, global_variables, false, &super::function_scope(method), HashMap::new(), output, steps)?
                                    .unwrap_or(Value::Null);
                                Ok(Some(super::validate_return_type_in_scope(result, method, classes, Some(&class_name))?))
                            };
                            match crate::json_builtins::encode_with_serializer(value, depth as usize, crate::json_builtins::EncodeOptions {
                                pretty: flags & 128 != 0,
                                unescaped_slashes: flags & 64 != 0,
                                unescaped_unicode: flags & 256 != 0,
                                unescaped_line_terminators: flags & 2048 != 0,
                                force_object: flags & 16 != 0,
                                hex_flags: flags & 15,
                                preserve_zero_fraction: flags & 1024 != 0,
                                numeric_check: flags & 32 != 0,
                                partial_output: flags & 512 != 0,
                            }, classes, &mut serialize) {
                                Ok((json, error_code)) => {
                                    if !throw_on_error || flags & 512 != 0 || error_code != 0 { global_variables.json_last_error = error_code; }
                                    stack.push(Value::String(json));
                                }
                                Err(error) => {
                                    let code = json_error_code(&error);
                                    if throw_on_error && flags & 512 == 0 {
                                        return Err(json_exception_marker(global_variables, *steps, code, json_error_message(code), classes)?);
                                    }
                                    global_variables.json_last_error = code;
                                    stack.push(Value::Bool(false));
                                }
                            }
                        }
                    }
                    "json_decode" => {
                        let source = arguments.first().ok_or("json_decode() expects exactly 1 argument")?.show();
                        let depth = arguments.get(2).map(|value| value.number() as i64).unwrap_or(512);
                        let flags = arguments.get(3).map(|value| value.number() as i64).unwrap_or(0);
                        let associative = arguments.get(1).map_or(flags & 1 != 0, |value| {
                            matches!(value.dereference(), Value::Null) && flags & 1 != 0 || value.truthy()
                        });
                        let throw_on_error = flags & 4194304 != 0;
                        if depth < 1 || depth > i32::MAX as i64 {
                            return Err(format!("ValueError:json_decode(): Argument #3 ($depth) must be between 1 and {}", i32::MAX));
                        }
                        if flags & !(3 | 1048576 | 2097152 | 4194304) != 0 {
                            let code = 4;
                            if throw_on_error {
                                return Err(json_exception_marker(global_variables, *steps, code, json_error_message(code), classes)?);
                            }
                            global_variables.json_last_error = code;
                            stack.push(Value::Null);
                        } else {
                            match crate::json_builtins::decode(&source, associative, depth as usize, flags & 2 != 0) {
                                Ok(value) => { if !throw_on_error { global_variables.json_last_error = 0; } stack.push(value); }
                                Err(error) => {
                                    let code = json_error_code(&error);
                                    if throw_on_error {
                                        return Err(json_exception_marker(global_variables, *steps, code, json_error_message(code), classes)?);
                                    }
                                    global_variables.json_last_error = code;
                                    stack.push(Value::Null);
                                }
                            }
                        }
                    }
                    "json_validate" => {
                        let value = arguments.first().ok_or("json_validate() expects exactly 1 argument")?;
                        let source = match value.dereference() {
                            Value::String(source) => source,
                            Value::Integer(number) => number.to_string(),
                            Value::Number(number) => number.to_string(),
                            Value::Bool(value) => if value { "1".into() } else { String::new() },
                            Value::Null => String::new(),
                            other => return Err(format!("TypeError:json_validate(): Argument #1 ($json) must be of type string, {} given", other.type_name().to_ascii_lowercase())),
                        };
                        let depth = arguments.get(1).map(|value| value.number() as i64).unwrap_or(512);
                        let flags = arguments.get(2).map(|value| value.number() as i64).unwrap_or(0);
                        if depth < 1 || depth > i32::MAX as i64 {
                            return Err(format!("ValueError:json_validate(): Argument #2 ($depth) must be between 1 and {}", i32::MAX));
                        }
                        if flags & !1048576 != 0 {
                            return Err("ValueError:json_validate(): Argument #3 ($flags) must be a valid flag (allowed flags: JSON_INVALID_UTF8_IGNORE)".into());
                        }
                        match crate::json_builtins::decode(&source, true, depth as usize, false) {
                            Ok(_) => { global_variables.json_last_error = 0; stack.push(Value::Bool(true)); }
                            Err(error) => {
                                global_variables.json_last_error = json_error_code(&error);
                                stack.push(Value::Bool(false));
                            }
                        }
                    }
                    "json_last_error" => {
                        if !arguments.is_empty() { return Err("json_last_error() expects exactly 0 arguments".into()); }
                        stack.push(Value::Integer(global_variables.json_last_error));
                    }
                    "json_last_error_msg" => {
                        if !arguments.is_empty() { return Err("json_last_error_msg() expects exactly 0 arguments".into()); }
                        stack.push(Value::String(json_error_message(global_variables.json_last_error).into()));
                    }
                    "is_uploaded_file" => {
                        let path = arguments.first().ok_or("is_uploaded_file() expects a filename")?.show();
                        let allowed = global_variables.get("\0upload_paths").map(Value::show).unwrap_or_default();
                        stack.push(Value::Bool(allowed.lines().any(|candidate| candidate == path) && PathBuf::from(path).is_file()));
                    }
                    "move_uploaded_file" => {
                        let source = arguments.first().ok_or("move_uploaded_file() expects a source filename")?.show();
                        let destination = arguments.get(1).ok_or("move_uploaded_file() expects a destination filename")?.show();
                        let allowed = global_variables.get("\0upload_paths").map(Value::show).unwrap_or_default();
                        let root = global_variables.get("\0document_root").map(Value::show).unwrap_or_default();
                        let source_path = PathBuf::from(&source);
                        let root_path = PathBuf::from(root);
                        let destination_path = PathBuf::from(destination);
                        let destination_path = if destination_path.is_absolute() { destination_path } else { root_path.join(destination_path) };
                        let moved = if allowed.lines().any(|candidate| candidate == source)
                            && source_path.is_file()
                            && destination_path.file_name().is_some()
                        {
                            destination_path.parent().and_then(|parent| parent.canonicalize().ok()).filter(|parent| parent.starts_with(&root_path))
                                .map(|parent| parent.join(destination_path.file_name().unwrap()))
                                .is_some_and(|destination| std::fs::rename(&source_path, &destination).is_ok())
                        } else { false };
                        if moved {
                            let remaining = allowed.lines().filter(|candidate| *candidate != source).collect::<Vec<_>>().join("\n");
                            global_variables.set("\0upload_paths".into(), Value::String(remaining));
                        }
                        stack.push(Value::Bool(moved));
                    }
                    "session_start" => {
                        global_variables.session_started = true;
                        stack.push(Value::Bool(true));
                    }
                    "session_destroy" => {
                        global_variables.session_started = true;
                        global_variables.session_destroyed = true;
                        global_variables.set("_SESSION".into(), Value::from_array_entries(Vec::new()));
                        stack.push(Value::Bool(true));
                    }
                    "session_unset" => {
                        global_variables.set("_SESSION".into(), Value::from_array_entries(Vec::new()));
                        stack.push(Value::Null);
                    }
                    "session_status" => {
                        stack.push(Value::Integer(if global_variables.session_started { 2 } else { 1 }));
                    }
                    "session_id" => {
                        let current = global_variables.get("\0session_id").map(Value::show).unwrap_or_default();
                        if let Some(id) = arguments.first().filter(|value| !matches!(value, Value::Null)) {
                            let id = id.show();
                            if id.len() > 128 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte)) {
                                return Err("session_id(): invalid session identifier".into());
                            }
                            global_variables.set("\0session_id".into(), Value::String(id));
                        }
                        stack.push(Value::String(current));
                    }
                    "basename" | "dirname" | "pathinfo" | "strip_tags" | "strtolower" | "strtoupper" | "strlen" | "ucfirst" | "lcfirst" | "strrev" | "str_shuffle" | "str_rot13" | "addslashes" | "addcslashes" | "stripslashes" | "stripcslashes" | "bin2hex" | "hex2bin" | "chr" | "count_chars" | "ord" | "number_format" | "str_increment" | "str_decrement" | "strspn" | "strcspn" | "strpbrk" | "str_word_count" | "strcmp" | "strcasecmp" | "strncmp" | "strncasecmp" | "strnatcmp" | "strnatcasecmp" | "strtr" | "ucwords" | "trim" | "ltrim" | "rtrim" | "str_repeat" | "str_pad" | "str_split" | "strpos" | "stripos" | "strrpos" | "strripos" | "substr_count" | "substr_compare" | "strtok" | "levenshtein" | "substr" | "substr_replace" | "explode" | "implode" | "str_getcsv" | "htmlspecialchars" | "htmlspecialchars_decode" | "nl2br" | "wordwrap" | "str_contains" | "str_starts_with" | "str_ends_with" | "strstr" | "strchr" | "stristr" => {
                        coerce_stringable_builtin_args(fallback_name, &mut arguments, crate::string_builtins::string_argument_positions(fallback_name), functions, classes, static_properties, global_variables, output, steps)?;
                        let result = match fallback_name.to_ascii_lowercase().as_str() {
                            "hex2bin" => hex2bin_with_diagnostic(&arguments, global_variables)?,
                            "chr" => chr_with_diagnostic(&arguments, global_variables)?,
                            _ => crate::string_builtins::call(fallback_name, &arguments)?,
                        };
                        stack.push(result);
                    }
                    "str_replace" | "str_ireplace" => {
                        coerce_stringable_builtin_args(fallback_name, &mut arguments, crate::string_builtins::string_argument_positions(fallback_name), functions, classes, static_properties, global_variables, output, steps)?;
                        let (value, count) = crate::string_builtins::replace(fallback_name, &arguments)?;
                        if let Some(Some(target)) = by_reference.get(3) {
                            write_reference_argument(target, Value::Integer(count as i64), variables, bindings, global_variables, global_scope)?;
                        }
                        stack.push(value);
                    }
                    "define" => {
                        if arguments.len() < 2 || arguments.len() > 3 { return Err("define() expects between 2 and 3 arguments".into()); }
                        let name = arguments[0].show();
                        if name.is_empty() || name.contains('\0') { return Err("define(): constant name must be a non-empty string".into()); }
                        if arguments.get(2).is_some_and(Value::truthy) { return Err("define(): case-insensitive constants are not supported".into()); }
                        let name = name.trim_start_matches('\\').to_string();
                        if super::constant_exists(&name, global_variables) {
                            stack.push(Value::Bool(false));
                        } else {
                            global_variables.constants.insert(name, arguments[1].clone());
                            stack.push(Value::Bool(true));
                        }
                    }
                    "defined" => {
                        let name = arguments.first().ok_or("defined() expects exactly 1 argument")?.show();
                        stack.push(Value::Bool(super::constant_exists(&name, global_variables)));
                    }
                    "constant" => {
                        let name = arguments.first().ok_or("constant() expects exactly 1 argument")?.show();
                        stack.push(super::read_constant(&name, global_variables)?);
                    }
                    "compact" => {
                        stack.push(super::symbol_table::compact(&arguments, variables, bindings, global_variables, global_scope)?);
                    }
                    "get_defined_vars" => {
                        if !arguments.is_empty() { return Err("get_defined_vars() expects exactly 0 arguments".into()); }
                        stack.push(super::symbol_table::get_defined_vars(variables, bindings, global_variables, global_scope));
                    }
                    "extract" => {
                        stack.push(super::symbol_table::extract(
                            &arguments, variables, bindings, global_variables, global_scope, frame_id,
                            by_reference.first().and_then(Option::as_ref),
                        )?);
                    }
                    _ => {
                let array_arguments = arguments.iter().map(Value::dereference).collect::<Vec<_>>();
                let mut array_diagnostics = Vec::new();
                let normalized_array_name = fallback_name.to_ascii_lowercase();
                let callback_indices = match normalized_array_name.as_str() {
                    "array_map" => vec![0],
                    "array_all" | "array_any" | "array_filter" | "array_find" | "array_find_key" | "array_reduce" | "usort" | "uasort" | "uksort" => vec![1],
                    "array_diff_uassoc" | "array_diff_ukey" | "array_intersect_uassoc" | "array_intersect_ukey" => vec![array_arguments.len().saturating_sub(1)],
                    "array_udiff" | "array_udiff_assoc" | "array_uintersect" | "array_uintersect_assoc" => vec![array_arguments.len().saturating_sub(1)],
                    "array_udiff_uassoc" | "array_udiff_ukey" | "array_uintersect_uassoc" | "array_uintersect_ukey" => vec![array_arguments.len().saturating_sub(2), array_arguments.len().saturating_sub(1)],
                    _ => Vec::new(),
                };
                if callback_indices.iter().any(|index| array_arguments.get(*index).is_some_and(|callback| {
                    callable_parameter_references(callback, functions, classes).iter().any(|by_ref| *by_ref)
                })) {
                    array_diagnostics.push(crate::RuntimeDiagnostic {
                        level: crate::DiagnosticLevel::Warning,
                        message: format!("{fallback_name}(): callback expects a parameter to be passed by reference"),
                        source_file: None, source_line: None,
                    });
                }
                if fallback_name.eq_ignore_ascii_case("array_walk") || fallback_name.eq_ignore_ascii_case("array_walk_recursive") {
                    if let Some(callback) = array_arguments.get(1) {
                        let refs = callable_parameter_references(callback, functions, classes);
                        let userdata_passed = array_arguments.len() > 2;
                        if refs.get(1).copied().unwrap_or(false) || (userdata_passed && refs.get(2).copied().unwrap_or(false)) {
                            array_diagnostics.push(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Warning,
                                message: format!("{fallback_name}(): callback expects the key or userdata parameter to be passed by reference"),
                                source_file: None, source_line: None,
                            });
                        }
                    }
                }
                let dispatch_result = if fallback_name.eq_ignore_ascii_case("array_walk") || fallback_name.eq_ignore_ascii_case("array_walk_recursive") {
                    let mut no_positional_callback = |_: &Value, _: Vec<Value>| Err(format!("Unexpected positional callback for {fallback_name}"));
                    let mut walk_index = 0usize;
                    let mut walk_invoker = |callback: &Value, value: Value, key: Value, user_data: Option<Value>| {
                        let temp_name = format!("\0array_walk:{}:{}", frame_id, walk_index);
                        walk_index += 1;
                        invoke_array_walk_callback(
                            callback, value, key, user_data, temp_name, fallback_name, frame_id, functions, classes,
                            static_properties, global_variables, output, steps,
                        )
                    };
                    let mut stringify = |value: &Value| Ok(value.show());
                    crate::array_builtins::call(fallback_name, array_arguments, classes, &mut no_positional_callback, &mut walk_invoker, &mut stringify, &mut array_diagnostics)
                } else {
                    let mut unused_walk = |_: &Value, _: Value, _: Value, _: Option<Value>| Err("Unexpected array walk callback".into());
                    if matches!(fallback_name, "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" | "natsort" | "natcasesort" | "array_unique") {
                        let mut unused_callback = |_: &Value, _: Vec<Value>| Err("Unexpected positional callback for sorting".into());
                        let mut stringify = |value: &Value| super::objects::stringify(
                            value, functions, classes, static_properties, global_variables, output, steps,
                        );
                        crate::array_builtins::call(fallback_name, array_arguments, classes, &mut unused_callback, &mut unused_walk, &mut stringify, &mut array_diagnostics)
                    } else {
                        let mut callback_invoker = |callback: &Value, callback_args: Vec<Value>| {
                            invoke_array_callback(
                                callback, callback_args, variables, bindings, global_scope, frame_id, functions, classes,
                                static_properties, global_variables, output, steps,
                            )
                        };
                        let mut stringify = |value: &Value| Ok(value.show());
                        crate::array_builtins::call(fallback_name, array_arguments, classes, &mut callback_invoker, &mut unused_walk, &mut stringify, &mut array_diagnostics)
                    }
                };
                for diagnostic in array_diagnostics { global_variables.report_diagnostic(diagnostic); }
                let (result, mutation) = dispatch_result?;
                for (argument_index, updated_value) in mutation {
                    let Some(Some(target)) = by_reference.get(argument_index) else {
                        let parameter = match normalized_array_name.as_str() {
                            "array_pop" | "array_shift" | "array_push" | "array_unshift" | "array_splice"
                            | "shuffle" | "natsort" | "natcasesort" | "sort" | "rsort" | "asort" | "arsort"
                            | "ksort" | "krsort" | "usort" | "uasort" | "uksort" | "array_multisort" => "array",
                            _ => "",
                        };
                        let parameter = if parameter.is_empty() { String::new() } else { format!(" (${parameter})") };
                        return Err(format!("{fallback_name}(): Argument #{}{parameter} cannot be passed by reference", argument_index + 1));
                    };
                    write_reference_argument(target, updated_value, variables, bindings, global_variables, global_scope)?;
                }
                stack.push(result);
                    }
                }
            }
        }
        Op::MakeClosure(function, captures) => {
            let mut captured_values = HashMap::new();
            let mut reference_captures = HashMap::new();
            let mut capture_order = Vec::new();
            for (name, by_reference) in captures {
                capture_order.push(name.clone());
                if *by_reference {
                    let key = reference_storage_key(name, variables, bindings, global_variables, global_scope, frame_id);
                    reference_captures.insert(name.clone(), key);
                } else {
                    captured_values.insert(name.clone(), read_variable(name, variables, bindings, global_variables, global_scope));
                }
            }
            stack.push(Value::Closure(Rc::new(std::cell::RefCell::new(PhpClosure {
                function: (**function).clone(), captures: captured_values, capture_order, reference_captures, callable_target: None,
            }))));
        }
        Op::MakeFunctionCallable(name) => {
            stack.push(make_callable_closure(Value::String(name.clone()), functions, classes)?);
        }
        Op::MakeStaticCallable(class, method) => {
            let target = Value::String(format!("{class}::{method}"));
            stack.push(make_callable_closure(target, functions, classes)?);
        }
        Op::MakeLateStaticCallable(method) => {
            let class = variables.get("static_class").map(Value::show)
                .or_else(|| variables.get("this").and_then(|value| match value.dereference() {
                    Value::Object(object) => Some(object.borrow().class_name.clone()),
                    _ => None,
                }))
                .ok_or("Cannot create a static callable outside a class context")?;
            let target = Value::String(format!("{class}::{method}"));
            stack.push(make_callable_closure(target, functions, classes)?);
        }
        Op::MakeMethodCallable(method) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let target = Value::from_array_entries(vec![
                (ArrayKey::Integer(0), receiver),
                (ArrayKey::Integer(1), Value::String(method.clone())),
            ]);
            stack.push(make_callable_closure(target, functions, classes)?);
        }
        Op::CallValue(_argument_count, reference_names, argument_names, unpack, strict_types) => {
            let (arguments, reference_names, argument_names) = take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            if stack.is_empty() { return Err("Invalid bytecode stack".into()); }
            let callable = stack.pop().ok_or("Invalid bytecode stack")?;
            let result = match &callable {
                Value::Closure(closure) => invoke_closure(closure, arguments, &reference_names, &argument_names, variables, bindings, global_scope, frame_id, functions, classes, static_properties, global_variables, output, steps, *strict_types)?,
                Value::Array(_) => invoke_array_callable(&callable, arguments, &reference_names, &argument_names, variables, bindings, global_scope, frame_id, functions, classes, static_properties, global_variables, output, steps, *strict_types)?,
                Value::Object(_) => invoke_object_callable(&callable, arguments, &reference_names, &argument_names, variables, bindings, global_scope, frame_id, functions, classes, static_properties, global_variables, output, steps, *strict_types)?,
                Value::String(name) => {
                    let packed_arguments = pack_callable_arguments(
                        arguments, &reference_names, &argument_names, variables, bindings,
                        global_variables, global_scope, frame_id,
                    )?;
                    let (instruction, mut callback_stack) = callback_call(Value::String(name.clone()), packed_arguments, *strict_types)?;
                    execute_instruction(
                        &instruction, &mut callback_stack, variables, bindings, global_variables, global_scope,
                        frame_id, functions, classes, static_properties, output, steps,
                    )?;
                    callback_stack.pop().ok_or("Invalid first-class callable result stack")?
                }
                _ => return Err("Value is not callable".into()),
            };
            stack.push(result);
        }
        Op::CallMethod(method, _argument_count, reference_names, argument_names, unpack, access_scope, strict_types) => {
            let (mut arguments, mut reference_names, argument_names) = take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let method = if method == "\0dynamic" {
                stack.pop().ok_or("Missing dynamic method name")?.show().to_ascii_lowercase()
            } else { method.clone() };
            if stack.is_empty() { return Err("Invalid bytecode stack".into()); }
            let object = stack.pop().ok_or("Invalid bytecode stack")?;
            if let Value::Generator(generator) = &object {
                if matches!(method.as_str(), "current" | "key" | "valid" | "rewind" | "next" | "send" | "throw" | "getreturn") {
                    let method_signature = format!("generator::{method}");
                    arguments = super::builtin_arguments::bind(&method_signature, arguments, &reference_names, &argument_names)?.0;
                }
                stack.push(super::call_generator_method(
                    generator, &method, arguments, functions, classes, static_properties,
                    global_variables, output, steps,
                )?);
                return Ok(true);
            }
            let Value::Object(object) = object else { return Err("Call to method on non-object".into()); };
            let class_name = object.borrow().class_name.clone();
            if class_name.eq_ignore_ascii_case("ReflectionClass") || class_name.eq_ignore_ascii_case("ReflectionEnum") || class_name.eq_ignore_ascii_case("ReflectionEnumUnitCase") || class_name.eq_ignore_ascii_case("ReflectionEnumBackedCase") || class_name.eq_ignore_ascii_case("ReflectionNamedType") || class_name.eq_ignore_ascii_case("ReflectionMethod") || class_name.eq_ignore_ascii_case("ReflectionProperty") || class_name.eq_ignore_ascii_case("ReflectionAttribute") || class_name.eq_ignore_ascii_case("ReflectionParameter") || class_name.eq_ignore_ascii_case("ReflectionFunction") {
                let signature = format!("{}::{method}", class_name.to_ascii_lowercase());
                arguments = super::builtin_arguments::bind(&signature, arguments, &reference_names, &argument_names)?.0;
                let internal = object.borrow().properties.clone();
                let result = match (class_name.to_ascii_lowercase().as_str(), method.as_str()) {
                    ("reflectionfunction", "getname") => internal.get("\0function_name").cloned().ok_or("Invalid ReflectionFunction")?,
                    ("reflectionfunction", "getattributes") => {
                        let function_name = internal.get("\0function_name").ok_or("Invalid ReflectionFunction")?.show();
                        let function = functions.get(&function_name.to_ascii_lowercase()).ok_or("Reflected function no longer exists")?;
                        let filter = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null)).map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 { return Err("ReflectionFunction::getAttributes(): unsupported flags".into()); }
                        let entries = function.attributes.iter().filter(|attribute| filter.as_ref().is_none_or(|name| attribute.name.eq_ignore_ascii_case(name) || (flags & 2 != 0 && super::is_a_type(&attribute.name, name, classes, &mut HashSet::new())))).enumerate().map(|(index, attribute)| (ArrayKey::Integer(index as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionAttribute".into(),
                            properties: HashMap::from([("\0attribute_name".into(), Value::String(attribute.name.clone())), ("\0attribute_arguments".into(), reflection_attribute_arguments(&attribute.arguments)), ("\0target_class".into(), Value::String(function_name.clone())), ("\0target_type".into(), Value::Integer(2))]),
                            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                        }))))).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionfunction", "getparameters") => {
                        let function = match internal.get("\0closure").map(Value::dereference) {
                            Some(Value::Closure(closure)) => Some(closure.borrow().function.clone()),
                            _ => internal.get("\0function_name").and_then(|name| functions.get(&name.show().to_ascii_lowercase())).cloned(),
                        }.ok_or("Reflected function no longer exists")?;
                        let entries = function.parameters.iter().enumerate().map(|(position, name)| {
                            let attributes = function.parameter_attributes.get(position).map(Vec::as_slice).unwrap_or_default();
                            let reflected = attributes.iter().map(|attribute| {
                                let duplicate_count = attributes.iter().filter(|candidate| candidate.name.eq_ignore_ascii_case(&attribute.name)).count();
                                Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                    class_name: "ReflectionAttribute".into(),
                                    properties: HashMap::from([
                                        ("\0attribute_name".into(), Value::String(attribute.name.clone())),
                                        ("\0attribute_arguments".into(), reflection_attribute_arguments(&attribute.arguments)),
                                        ("\0target_class".into(), Value::String(function.display_name.clone())),
                                        ("\0target_function".into(), Value::String(function.display_name.clone())),
                                        ("\0target_type".into(), Value::Integer(32)),
                                        ("\0target_position".into(), Value::Integer(position as i64)),
                                        ("\0is_repeated".into(), Value::Bool(duplicate_count > 1)),
                                    ]),
                                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                                })))
                            }).enumerate().map(|(index, attribute)| (ArrayKey::Integer(index as i64), attribute)).collect();
                            (ArrayKey::Integer(position as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionParameter".into(),
                                properties: HashMap::from([
                                    ("\0parameter_name".into(), Value::String(name.clone())),
                                    ("\0parameter_position".into(), Value::Integer(position as i64)),
                                    ("\0parameter_attributes".into(), Value::from_array_entries(reflected)),
                                ]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            }))))
                        }).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getname") => internal.get("\0case_name").cloned().ok_or("Invalid ReflectionEnumUnitCase")?,
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getvalue") => internal.get("\0case_value").cloned().ok_or("Invalid ReflectionEnumUnitCase")?,
                    ("reflectionenumbackedcase", "getbackingvalue") => {
                        let case = internal.get("\0case_value").ok_or("Invalid ReflectionEnumBackedCase")?;
                        match case.dereference() {
                            Value::Object(case) => case.borrow().properties.get("value").cloned().ok_or("Invalid ReflectionEnumBackedCase")?,
                            _ => return Err("Invalid ReflectionEnumBackedCase".into()),
                        }
                    }
                    ("reflectionnamedtype", "getname") => internal.get("\0type_name").cloned().ok_or("Invalid ReflectionNamedType")?,
                    ("reflectionnamedtype", "isbuiltin") => Value::Bool(internal.get("\0is_builtin").is_some_and(Value::truthy)),
                    ("reflectionnamedtype", "allowsnull") => Value::Bool(internal.get("\0allows_null").is_some_and(Value::truthy)),
                    ("reflectionenum", "isbacked") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionEnum target")?.show();
                        Value::Bool(classes.get(&target.to_ascii_lowercase()).is_some_and(|class| class.enum_backing_type.is_some()))
                    }
                    ("reflectionenum", "getbackingtype") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionEnum target")?.show();
                        let class = classes.get(&target.to_ascii_lowercase()).filter(|class| class.is_enum).ok_or("Invalid ReflectionEnum target")?;
                        match &class.enum_backing_type {
                            None => Value::Null,
                            Some(name) => Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionNamedType".into(),
                                properties: HashMap::from([
                                    ("\0type_name".into(), Value::String(name.clone())),
                                    ("\0is_builtin".into(), Value::Bool(true)),
                                    ("\0allows_null".into(), Value::Bool(false)),
                                ]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            }))),
                        }
                    }
                    ("reflectionenum", "getcases") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionEnum target")?.show();
                        let class = classes.get(&target.to_ascii_lowercase()).filter(|class| class.is_enum).ok_or("Invalid ReflectionEnum target")?;
                        Value::from_array_entries(class.enum_cases.iter().enumerate().filter_map(|(index, name)| {
                            let case = class.constants.get(name)?.value.clone();
                            let reflected = crate::PhpObject {
                                class_name: if class.enum_backing_type.is_some() { "ReflectionEnumBackedCase" } else { "ReflectionEnumUnitCase" }.into(),
                                properties: HashMap::from([("\0case_name".into(), Value::String(name.clone())), ("\0case_value".into(), case)]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            };
                            Some((ArrayKey::Integer(index as i64), Value::Object(Rc::new(std::cell::RefCell::new(reflected)))))
                        }).collect())
                    }
                    ("reflectionclass", "isenum") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        Value::Bool(classes.get(&target.to_ascii_lowercase()).is_some_and(|class| class.is_enum))
                    }
                    ("reflectionclass", "isinterface") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        Value::Bool(classes.get(&target.to_ascii_lowercase()).is_some_and(|class| class.is_interface))
                    }
                    ("reflectionclass", "isabstract") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        Value::Bool(classes.get(&target.to_ascii_lowercase()).is_some_and(|class| class.is_abstract))
                    }
                    ("reflectionclass", "isfinal") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        Value::Bool(classes.get(&target.to_ascii_lowercase()).is_some_and(|class| class.is_final))
                    }
                    ("reflectionclass", "getparentclass") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        match classes.get(&target.to_ascii_lowercase()).and_then(|class| class.parent.as_ref()).and_then(|parent| {
                            classes.get(&parent.to_ascii_lowercase()).map(|class| class.name.clone())
                        }) {
                            Some(parent) => Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionClass".into(),
                                properties: HashMap::from([("\0target_class".into(), Value::String(parent.clone()))]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            }))),
                            None => Value::Bool(false),
                        }
                    }
                    ("reflectionclass", "issubclassof" | "implementsinterface") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        let related = arguments.first().ok_or_else(|| format!("ReflectionClass::{method}() expects exactly 1 argument"))?.show();
                        let same = target.eq_ignore_ascii_case(&related);
                        let matches = super::is_a_type(&target.to_ascii_lowercase(), &related.to_ascii_lowercase(), classes, &mut HashSet::new());
                        Value::Bool(matches && (method == "implementsinterface" || !same))
                    }
                    ("reflectionclass" | "reflectionenum", "getname") => internal.get("\0target_class").cloned().ok_or("Invalid ReflectionClass target")?,
                    ("reflectionclass" | "reflectionenum", "getattributes") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        let class = classes.get(&target.to_ascii_lowercase()).ok_or("Reflected class no longer exists")?;
                        let filter = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null)).map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 { return Err("ReflectionClass::getAttributes(): unsupported flags".into()); }
                        let entries = class.attributes.iter().filter(|attribute| filter.as_ref().is_none_or(|name| {
                            attribute.name.eq_ignore_ascii_case(name) || (flags & 2 != 0 && super::is_a_type(&attribute.name, name, classes, &mut HashSet::new()))
                        })).enumerate().map(|(index, attribute)| {
                            let arguments = reflection_attribute_arguments(&attribute.arguments);
                            (ArrayKey::Integer(index as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionAttribute".into(),
                                properties: HashMap::from([("\0attribute_name".into(), Value::String(attribute.name.clone())), ("\0attribute_arguments".into(), arguments), ("\0target_class".into(), Value::String(class.name.clone())), ("\0target_type".into(), Value::Integer(1))]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            }))))
                        }).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionclass" | "reflectionenum", "getmethod") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        let name = arguments.first().ok_or("ReflectionClass::getMethod() expects exactly 1 argument")?.show();
                        let method = find_method(&target, &name.to_ascii_lowercase(), classes).ok_or_else(|| format!("Method {target}::{name}() does not exist"))?;
                        let declaring_class = method.declaring_class.clone().unwrap_or_else(|| target.to_ascii_lowercase());
                        let display_name = classes.get(&declaring_class.to_ascii_lowercase()).and_then(|class| class.method_display_names.get(&method.name.to_ascii_lowercase())).cloned().unwrap_or(name);
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionMethod".into(),
                            properties: HashMap::from([("\0target_class".into(), Value::String(declaring_class)), ("\0method_name".into(), Value::String(display_name))]),
                            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                        })))
                    }
                    ("reflectionclass" | "reflectionenum", "getproperty") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionClass target")?.show();
                        let name = arguments.first().ok_or("ReflectionClass::getProperty() expects exactly 1 argument")?.show();
                        let mut current = Some(target.as_str());
                        let mut declaring = None;
                        while let Some(class_name) = current {
                            let Some(class) = classes.get(&class_name.to_ascii_lowercase()) else { break; };
                            if class.properties.contains_key(&name) || class.static_properties.contains_key(&name) { declaring = Some(class.name.clone()); break; }
                            current = class.parent.as_deref();
                        }
                        let declaring = declaring.ok_or_else(|| format!("Property {target}::${name} does not exist"))?;
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionProperty".into(),
                            properties: HashMap::from([("\0target_class".into(), Value::String(declaring)), ("\0property_name".into(), Value::String(name))]),
                            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                        })))
                    }
                    ("reflectionmethod", "getname") => internal.get("\0method_name").cloned().ok_or("Invalid ReflectionMethod")?,
                    ("reflectionmethod", "getparameters") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionMethod target")?.show();
                        let method_name = internal.get("\0method_name").ok_or("Invalid ReflectionMethod name")?.show();
                        let function = find_method(&target, &method_name.to_ascii_lowercase(), classes).ok_or("Reflected method no longer exists")?;
                        let entries = function.parameters.iter().enumerate().map(|(position, name)| (ArrayKey::Integer(position as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionParameter".into(),
                            properties: HashMap::from([("\0target_class".into(), Value::String(target.clone())), ("\0method_name".into(), Value::String(method_name.to_ascii_lowercase())), ("\0parameter_name".into(), Value::String(name.clone())), ("\0parameter_position".into(), Value::Integer(position as i64))]),
                            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                        }))))).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionparameter", "getname") => internal.get("\0parameter_name").cloned().ok_or("Invalid ReflectionParameter")?,
                    ("reflectionparameter", "getposition") => internal.get("\0parameter_position").cloned().ok_or("Invalid ReflectionParameter")?,
                    ("reflectionparameter", "getattributes") => {
                        if let Some(Value::Array(attributes)) = internal.get("\0parameter_attributes").map(Value::dereference) {
                            let filter = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null)).map(Value::show);
                            let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                            if flags & !2 != 0 { return Err("ReflectionParameter::getAttributes(): unsupported flags".into()); }
                            let filtered = attributes.iter().filter(|(_, attribute)| {
                                let Value::Object(attribute) = attribute.dereference() else { return false; };
                                let attribute = attribute.borrow();
                                filter.as_ref().is_none_or(|name| attribute.properties.get("\0attribute_name").is_some_and(|value| {
                                    let actual = value.show();
                                    actual.eq_ignore_ascii_case(name) || flags & 2 != 0 && super::is_a_type(&actual.to_ascii_lowercase(), &name.to_ascii_lowercase(), classes, &mut HashSet::new())
                                }))
                            }).cloned().collect();
                            return { stack.push(Value::from_array_entries(filtered)); Ok(true) };
                        }
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionParameter target")?.show();
                        let method_name = internal.get("\0method_name").ok_or("Invalid ReflectionParameter method")?.show();
                        let position = internal.get("\0parameter_position").map_or(usize::MAX, |value| value.number() as usize);
                        let class = classes.get(&target.to_ascii_lowercase()).ok_or("Reflected class no longer exists")?;
                        let attributes = class.parameter_attributes.get(&method_name.to_ascii_lowercase()).and_then(|items| items.get(position)).map(Vec::as_slice).unwrap_or_default();
                        let filter = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null)).map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 { return Err("ReflectionParameter::getAttributes(): unsupported flags".into()); }
                        let entries = attributes.iter().filter(|attribute| filter.as_ref().is_none_or(|name| attribute.name.eq_ignore_ascii_case(name) || (flags & 2 != 0 && super::is_a_type(&attribute.name, name, classes, &mut HashSet::new())))).enumerate().map(|(index, attribute)| (ArrayKey::Integer(index as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionAttribute".into(),
                            properties: HashMap::from([("\0attribute_name".into(), Value::String(attribute.name.clone())), ("\0attribute_arguments".into(), reflection_attribute_arguments(&attribute.arguments)), ("\0target_class".into(), Value::String(class.name.clone())), ("\0target_type".into(), Value::Integer(32)), ("\0target_member".into(), Value::String(method_name.to_ascii_lowercase())), ("\0target_position".into(), Value::Integer(position as i64))]),
                            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                        }))))).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionmethod", "getattributes") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionMethod target")?.show();
                        let method_name = internal.get("\0method_name").ok_or("Invalid ReflectionMethod name")?.show();
                        let class = classes.get(&target.to_ascii_lowercase()).ok_or("Reflected class no longer exists")?;
                        let attributes = class.method_attributes.get(&method_name.to_ascii_lowercase()).map(Vec::as_slice).unwrap_or_default();
                        let filter = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null)).map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 { return Err("ReflectionMethod::getAttributes(): unsupported flags".into()); }
                        let entries = attributes.iter().filter(|attribute| filter.as_ref().is_none_or(|name| {
                            attribute.name.eq_ignore_ascii_case(name) || (flags & 2 != 0 && super::is_a_type(&attribute.name, name, classes, &mut HashSet::new()))
                        })).enumerate().map(|(index, attribute)| {
                            (ArrayKey::Integer(index as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionAttribute".into(),
                                properties: HashMap::from([("\0attribute_name".into(), Value::String(attribute.name.clone())), ("\0attribute_arguments".into(), reflection_attribute_arguments(&attribute.arguments)), ("\0target_class".into(), Value::String(class.name.clone())), ("\0target_type".into(), Value::Integer(4)), ("\0target_member".into(), Value::String(method_name.to_ascii_lowercase()))]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            }))))
                        }).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionproperty", "getname") => internal.get("\0property_name").cloned().ok_or("Invalid ReflectionProperty")?,
                    ("reflectionproperty", "getattributes") => {
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionProperty target")?.show();
                        let property = internal.get("\0property_name").ok_or("Invalid ReflectionProperty name")?.show();
                        let class = classes.get(&target.to_ascii_lowercase()).ok_or("Reflected class no longer exists")?;
                        let attributes = class.property_attributes.get(&property).map(Vec::as_slice).unwrap_or_default();
                        let filter = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null)).map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 { return Err("ReflectionProperty::getAttributes(): unsupported flags".into()); }
                        let entries = attributes.iter().filter(|attribute| filter.as_ref().is_none_or(|name| {
                            attribute.name.eq_ignore_ascii_case(name) || (flags & 2 != 0 && super::is_a_type(&attribute.name, name, classes, &mut HashSet::new()))
                        })).enumerate().map(|(index, attribute)| {
                            (ArrayKey::Integer(index as i64), Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionAttribute".into(),
                                properties: HashMap::from([("\0attribute_name".into(), Value::String(attribute.name.clone())), ("\0attribute_arguments".into(), reflection_attribute_arguments(&attribute.arguments)), ("\0target_class".into(), Value::String(class.name.clone())), ("\0target_type".into(), Value::Integer(8)), ("\0target_member".into(), Value::String(property.clone()))]),
                                property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                            }))))
                        }).collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionattribute", "getname") => internal.get("\0attribute_name").cloned().ok_or("Invalid ReflectionAttribute")?,
                    ("reflectionattribute", "getarguments") => internal.get("\0attribute_arguments").cloned().ok_or("Invalid ReflectionAttribute")?,
                    ("reflectionattribute", "gettarget") => internal.get("\0target_type").cloned().unwrap_or(Value::Integer(1)),
                    ("reflectionattribute", "isrepeated") => {
                        if let Some(repeated) = internal.get("\0is_repeated") { Value::Bool(repeated.truthy()) } else {
                        let name = internal.get("\0attribute_name").ok_or("Invalid ReflectionAttribute")?.show();
                        let target = internal.get("\0target_class").ok_or("Invalid ReflectionAttribute target")?.show();
                        let target_type = internal.get("\0target_type").map_or(1, |value| value.number() as i64);
                        let count = if target_type == 2 {
                            functions.get(&target.to_ascii_lowercase()).map_or(0, |function| function.attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&name)).count())
                        } else { classes.get(&target.to_ascii_lowercase()).map_or(0, |class| {
                            if target_type == 4 {
                                let method = internal.get("\0target_member").map(Value::show).unwrap_or_default();
                                class.method_attributes.get(&method.to_ascii_lowercase()).map_or(0, |attributes| attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&name)).count())
                            } else if target_type == 8 {
                                let property = internal.get("\0target_member").map(Value::show).unwrap_or_default();
                                class.property_attributes.get(&property).map_or(0, |attributes| attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&name)).count())
                            } else if target_type == 32 {
                                let method = internal.get("\0target_member").map(Value::show).unwrap_or_default();
                                let position = internal.get("\0target_position").map_or(usize::MAX, |value| value.number() as usize);
                                class.parameter_attributes.get(&method.to_ascii_lowercase()).and_then(|items| items.get(position)).map_or(0, |attributes| attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&name)).count())
                            } else { class.attributes.iter().filter(|attribute| attribute.name.eq_ignore_ascii_case(&name)).count() }
                        }) };
                        Value::Bool(count > 1)
                        }
                    }
                    ("reflectionattribute", "newinstance") => instantiate_reflection_attribute(
                        &internal, variables, bindings, global_variables, global_scope, frame_id,
                        functions, classes, static_properties, output, steps,
                    )?,
                    _ => return Err(format!("Call to undefined method {class_name}::{method}()")),
                };
                stack.push(result);
                return Ok(true);
            }
            if object.borrow().exception.is_some() && find_method(&class_name, &method, classes).is_none() {
                super::builtin_arguments::bind(
                    &format!("exception::{method}"), arguments.clone(), &reference_names, &argument_names,
                )?;
                stack.push(crate::exception_builtins::call_method(&Value::Object(object), &method, &arguments)?);
                return Ok(true);
            }
            if class_name.eq_ignore_ascii_case("pdo") || class_name.eq_ignore_ascii_case("pdostatement") {
                let method_signature = format!("{}::{method}", class_name.to_ascii_lowercase());
                let bound = super::builtin_arguments::bind(&method_signature, arguments, &reference_names, &argument_names)?;
                arguments = bound.0;
                reference_names = bound.1;
                if class_name.eq_ignore_ascii_case("pdostatement") && matches!(method.to_ascii_lowercase().as_str(), "bindparam" | "bindcolumn") {
                    let target = reference_names.get(1).and_then(Option::as_ref)
                        .ok_or_else(|| format!("PDOStatement::{method}(): argument #2 ($var) must be passed by reference"))?;
                    let cell = reference_cell_for_argument(target, variables, bindings, global_variables, global_scope, frame_id)?;
                    if arguments.len() < 2 { return Err(format!("PDOStatement::{method}() expects at least 2 arguments")); }
                    arguments[1] = Value::Reference(cell);
                }
                let mut instantiate_row = |row: &Value, fetched_class: &str, constructor_args: &[Value]| {
                    if fetched_class == "\0PDO_FETCH_FUNC" {
                        let packed = Value::from_array_entries(constructor_args.iter().enumerate()
                            .map(|(index, value)| (ArrayKey::Integer(index as i64), value.clone())).collect());
                        let (instruction, mut callback_stack) = callback_call(row.clone(), packed, false)?;
                        execute_instruction(&instruction, &mut callback_stack, variables, bindings, global_variables,
                            global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                        return callback_stack.pop().ok_or_else(|| "Invalid PDO::FETCH_FUNC callback result stack".to_string());
                    }
                    let (fetched_class, props_late) = fetched_class.strip_prefix("\0PDO_FETCH_PROPS_LATE:")
                        .map_or((fetched_class, false), |name| (name, true));
                    instantiate_pdo_row(row, fetched_class, constructor_args, props_late, variables, bindings, global_variables,
                        global_scope, frame_id, functions, classes, static_properties, output, steps)
                };
                stack.push(crate::pdo::call_method(&object, &method, &arguments, &mut instantiate_row)?);
                return Ok(true);
            }
            if class_name.to_ascii_lowercase().starts_with("mongodb\\") {
                let method_signature = format!("{}::{method}", class_name.to_ascii_lowercase());
                arguments = super::builtin_arguments::bind(&method_signature, arguments, &reference_names, &argument_names)?.0;
                stack.push(crate::mongo_php::call_method(&object, &method, &arguments)?);
                return Ok(true);
            }
            let function = match find_method(&class_name, &method, classes) {
                Some(function) if ensure_access(function.visibility, function.declaring_class.as_deref(), access_scope.as_deref(), classes, &method).is_ok() => function,
                _ => {
                    let magic = find_method(&class_name, "__call", classes).ok_or_else(|| format!("Call to undefined method {class_name}::{method}()"))?;
                    let result = invoke_magic_method(&Value::Object(object.clone()), magic, &method, arguments, &argument_names, functions, classes, static_properties, global_variables, output, steps)?;
                    stack.push(result);
                    return Ok(true);
                }
            };
            if function.is_abstract { return Err(format!("Cannot call abstract method {class_name}::{method}()")); }
            if function.is_static { return Err(format!("Non-static method {class_name}::{method}() cannot be called statically")); }
            let (mut locals, parameter_bindings) = prepare_reference_arguments(arguments, &reference_names, function, &argument_names, &format!("{class_name}::{method}()"), variables, bindings, global_variables, global_scope, frame_id, classes, *strict_types)?;
            let concrete_class = object.borrow().class_name.clone();
            locals.insert("this".to_string(), Value::Object(object));
            locals.insert("static_class".to_string(), Value::String(concrete_class.clone()));
            if super::is_generator_function(function) {
                stack.push(super::new_generator(function.clone(), locals, parameter_bindings, *steps, classes)?);
                return Ok(true);
            }
            let result = execute_code(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(function), parameter_bindings, output, steps)?.unwrap_or(Value::Null);
            let result = super::validate_return_type_in_scope(result, function, classes, Some(&concrete_class))?;
            stack.push(result);
        }
        Op::CallParentMethod(parent, method, _argument_count, reference_names, argument_names, unpack, access_scope, strict_types) => {
            let (arguments, reference_names, argument_names) = take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let object = variables.get("this").cloned().ok_or("parent call outside an instance method")?;
            let Value::Object(object) = object else { return Err("Invalid $this object".into()); };
            let concrete_class = object.borrow().class_name.clone();
            if method.eq_ignore_ascii_case("__construct")
                && find_method(parent, method, classes).is_none()
                && super::is_a_type(parent, "throwable", classes, &mut std::collections::HashSet::new()) {
                crate::exception_builtins::initialize_object(&Value::Object(object), parent, &arguments, classes)?;
                stack.push(Value::Null);
                return Ok(true);
            }
            let function = find_method(parent, method, classes).ok_or_else(|| format!("Call to undefined parent method {parent}::{method}()"))?;
            if function.is_abstract { return Err(format!("Cannot call abstract method {parent}::{method}()")); }
            ensure_access(function.visibility, function.declaring_class.as_deref(), access_scope.as_deref(), classes, method)?;
            let (mut locals, parameter_bindings) = prepare_reference_arguments(arguments, &reference_names, function, &argument_names, &format!("{parent}::{method}()"), variables, bindings, global_variables, global_scope, frame_id, classes, *strict_types)?;
            locals.insert("this".to_string(), Value::Object(object));
            locals.insert("static_class".to_string(), Value::String(concrete_class.clone()));
            if super::is_generator_function(function) {
                stack.push(super::new_generator(function.clone(), locals, parameter_bindings, *steps, classes)?);
                return Ok(true);
            }
            let result = execute_code(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(function), parameter_bindings, output, steps)?.unwrap_or(Value::Null);
            let result = super::validate_return_type_in_scope(result, function, classes, Some(&concrete_class))?;
            stack.push(result);
        }
        Op::CallStaticMethod(class_name, method, _argument_count, reference_names, argument_names, unpack, access_scope, strict_types) => {
            let (mut arguments, reference_names, argument_names) = take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let method = if method == "\0dynamic" { stack.pop().ok_or("Missing dynamic static method name")?.show().to_ascii_lowercase() } else { method.clone() };
            let class_name = if class_name == "\0dynamic" {
                match stack.pop().ok_or_else(|| "Missing dynamic class name".to_string())? { Value::String(class) => class.to_ascii_lowercase(), value => value.show().to_ascii_lowercase() }
            } else { class_name.clone() };
            let resolved_class = if class_name == "static" {
                match variables.get("static_class") { Some(Value::String(class)) => class.clone(), _ => return Err("static:: used outside a class method".into()) }
            } else { class_name.clone() };
            let class_name = resolved_class.as_str();
            if let Some(class) = classes.get(&class_name.to_ascii_lowercase()).filter(|class| class.is_enum) {
                if method.eq_ignore_ascii_case("cases") || method.eq_ignore_ascii_case("from") || method.eq_ignore_ascii_case("tryfrom") {
                    arguments = super::builtin_arguments::bind(&format!("enum::{method}"), arguments, &reference_names, &argument_names)?.0;
                }
                if method.eq_ignore_ascii_case("cases") {
                    if !arguments.is_empty() { return Err(format!("{}::cases() expects exactly 0 arguments", class.name)); }
                    let cases = class.enum_cases.iter().enumerate().map(|(index, case)| (
                        ArrayKey::Integer(index as i64),
                        class.constants.get(case).map(|constant| constant.value.clone()).unwrap_or(Value::Null),
                    )).collect();
                    stack.push(Value::array(cases));
                    return Ok(true);
                }
                if method.eq_ignore_ascii_case("from") || method.eq_ignore_ascii_case("tryfrom") {
                    let Some(backing_type) = class.enum_backing_type.as_deref() else {
                        return Err(format!("Call to undefined method {}::{method}()", class.name));
                    };
                    if arguments.len() != 1 { return Err(format!("{}::{method}() expects exactly 1 argument", class.name)); }
                    let input = arguments[0].dereference();
                    let scalar = match (backing_type, input) {
                        ("int", Value::Integer(value)) => Value::Integer(value),
                        ("int", Value::Number(value)) if value.is_finite() => Value::Integer(value as i64),
                        ("int", Value::String(value)) => Value::Integer(value.trim().parse::<i64>()
                            .map_err(|_| format!("{}::{method}(): Argument #1 must be a valid integer", class.name))?),
                        ("string", Value::String(value)) => Value::String(value),
                        ("string", Value::Integer(value)) => Value::String(value.to_string()),
                        ("string", Value::Number(value)) if value.is_finite() => Value::String(value.to_string()),
                        (_, value) => {
                            let kind = match value { Value::Null => "null", Value::Bool(_) => "bool", Value::Array(_) => "array", Value::Object(_) => "object", Value::Closure(_) => "Closure", Value::Generator(_) => "Generator", Value::Stream(_) => "resource", Value::Integer(_) => "int", Value::Number(_) => "float", Value::String(_) => "string", Value::Reference(_) => "reference" };
                            return Err(format!("{}::{method}(): Argument #1 must be of type int|string, {kind} given", class.name));
                        }
                    };
                    let found = class.enum_cases.iter().filter_map(|case| class.constants.get(case))
                        .find(|constant| matches!(&constant.value, Value::Object(object)
                            if object.borrow().properties.get("value").is_some_and(|value| enum_scalar_equal(value, &scalar))));
                    if let Some(case) = found {
                        stack.push(case.value.clone());
                    } else if method.eq_ignore_ascii_case("tryfrom") {
                        stack.push(Value::Null);
                    } else {
                        return Err(format!("ValueError: {:?} is not a valid backing value for enum {}", scalar.show(), class.name));
                    }
                    return Ok(true);
                }
            }
            if find_method(class_name, &method, classes).is_none() {
                if class_name.eq_ignore_ascii_case("PDO") && matches!(method.to_ascii_lowercase().as_str(), "connect" | "getavailabledrivers") {
                    arguments = super::builtin_arguments::bind(&format!("pdo::{method}"), arguments, &reference_names, &argument_names)?.0;
                }
                if let Some(result) = crate::pdo::call_static_method(class_name, &method, &arguments) {
                    stack.push(result?);
                    return Ok(true);
                }
            }
            let function = match find_method(class_name, &method, classes) {
                Some(function) if ensure_access(function.visibility, function.declaring_class.as_deref(), access_scope.as_deref(), classes, &method).is_ok() => function,
                _ => {
                    let magic = find_method(class_name, "__callstatic", classes).ok_or_else(|| format!("Call to undefined method {class_name}::{method}()"))?;
                    let result = invoke_magic_method(&Value::Null, magic, &method, arguments, &argument_names, functions, classes, static_properties, global_variables, output, steps)?;
                    stack.push(result);
                    return Ok(true);
                }
            };
            if function.is_abstract { return Err(format!("Cannot call abstract method {class_name}::{method}()")); }
            if !function.is_static { return Err(format!("Non-static method {class_name}::{method}() cannot be called statically")); }
            let (mut locals, parameter_bindings) = prepare_reference_arguments(arguments, &reference_names, function, &argument_names, &format!("{class_name}::{method}()"), variables, bindings, global_variables, global_scope, frame_id, classes, *strict_types)?;
            locals.insert("static_class".to_string(), Value::String(class_name.to_string()));
            if super::is_generator_function(function) {
                stack.push(super::new_generator(function.clone(), locals, parameter_bindings, *steps, classes)?);
                return Ok(true);
            }
            let result = execute_code(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(function), parameter_bindings, output, steps)?.unwrap_or(Value::Null);
            let result = super::validate_return_type_in_scope(result, function, classes, Some(class_name))?;
            stack.push(result);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn make_callable_closure(
    target: Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    let (callable, display_name) = callable_status(&target, false, functions, classes);
    if !callable {
        return Err(format!("Failed to create closure from callable {}", display_name));
    }
    let function = BytecodeFunction::new(
        "{first-class-callable}".into(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(),
        None, false, Vec::new(), false, false, false, MemberVisibility::Public, None,
    );
    Ok(Value::Closure(Rc::new(std::cell::RefCell::new(PhpClosure {
        function,
        captures: HashMap::new(),
        capture_order: Vec::new(),
        reference_captures: HashMap::new(),
        callable_target: Some(target),
    }))))
}

fn invoke_array_walk_callback(
    callback: &Value,
    value: Value,
    key: Value,
    user_data: Option<Value>,
    value_variable: String,
    builtin_name: &str,
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let mut variables = HashMap::from([(value_variable.clone(), value)]);
    let mut bindings = HashMap::new();
    let mut arguments = vec![Value::Null, key];
    if let Some(user_data) = user_data { arguments.push(user_data); }
    let callback_references = callable_parameter_references(callback, functions, classes);
    let reference_names = arguments.iter().enumerate().map(|(index, argument)| {
        let variable = if index == 0 {
            Some(value_variable.clone())
        } else if callback_references.get(index).copied().unwrap_or(false) {
            let name = format!("\0array_walk_arg:{}:{}", frame_id, index);
            variables.insert(name.clone(), argument.clone());
            Some(name)
        } else { None };
        variable.map(|variable| ReferenceArgument { variable, array_keys: Vec::new(), object_property: None, direct_reference: None })
    }).collect::<Vec<_>>();
    let argument_names = vec![None; arguments.len()];
    match callback {
        Value::String(name) if functions.contains_key(&name.to_ascii_lowercase()) => {
            invoke_with_references(
                name, arguments, &reference_names, &argument_names, false, &mut variables, &mut bindings,
                global_variables, false, frame_id, functions, classes, static_properties, output, steps,
            )?;
        }
        Value::String(name) => { invoke_callable(name, arguments, functions, classes, static_properties, global_variables, output, steps)?; }
        Value::Closure(closure) => {
            invoke_closure(
                closure, arguments, &reference_names, &argument_names, &mut variables, &mut bindings,
                false, frame_id, functions, classes, static_properties, global_variables, output, steps, false,
            )?;
        }
        Value::Array(_) => {
            invoke_array_callable(
                callback, arguments, &reference_names, &argument_names, &mut variables, &mut bindings, false, frame_id,
                functions, classes, static_properties, global_variables, output, steps, false,
            )?;
        }
        Value::Object(_) => {
            invoke_object_callable(
                callback, arguments, &reference_names, &argument_names, &mut variables, &mut bindings, false, frame_id,
                functions, classes, static_properties, global_variables, output, steps, false,
            )?;
        }
        _ => return Err(format!("{builtin_name}(): callback must be a valid callback")),
    }
    Ok(read_variable(&value_variable, &variables, &bindings, global_variables, false))
}

fn visible_argument_count(function: &BytecodeFunction, argument_count: usize, argument_names: &[Option<String>]) -> usize {
    if !argument_names.iter().any(Option::is_some) { return argument_count; }
    let fixed_count = function.parameter_variadic.iter().position(|variadic| *variadic).unwrap_or(function.parameters.len());
    let mut positional_count = 0usize;
    let mut visible_count = 0usize;
    for name in argument_names {
        if let Some(name) = name {
            if let Some(index) = function.parameters[..fixed_count].iter().position(|parameter| parameter == name) {
                visible_count = visible_count.max(index + 1);
            }
        } else {
            positional_count += 1;
            visible_count = visible_count.max(positional_count);
        }
    }
    visible_count
}

fn current_function_arguments(
    variables: &HashMap<String, Value>, bindings: &HashMap<String, VariableBinding>,
    globals: &GlobalVariables, global_scope: bool,
) -> Result<Vec<Value>, String> {
    let count = variables.get("\0func_argument_count").map(Value::number)
        .ok_or("func_*() called from the global scope")? as usize;
    let fixed_count = variables.get("\0func_fixed_parameter_count").map(Value::number).unwrap_or(0.0) as usize;
    let parameter_names = variables.get("\0func_parameter_names").ok_or("func_*() called from the global scope")?;
    let variadic_name = variables.get("\0func_variadic_parameter").map(Value::show).filter(|name| !name.is_empty());
    let mut arguments = Vec::with_capacity(count);
    for index in 0..count {
        if index < fixed_count {
            let parameter_name = parameter_names.array_get(&ArrayKey::Integer(index as i64))?.show();
            arguments.push(read_variable(&parameter_name, variables, bindings, globals, global_scope));
        } else if let Some(variadic_name) = &variadic_name {
            let variadic = read_variable(variadic_name, variables, bindings, globals, global_scope);
            arguments.push(variadic.array_get(&ArrayKey::Integer((index - fixed_count) as i64))?);
        } else {
            let extras = variables.get("\0func_extra_arguments").cloned().unwrap_or(Value::Null);
            arguments.push(extras.array_get(&ArrayKey::Integer((index - fixed_count) as i64))?);
        }
    }
    Ok(arguments)
}

fn normalize_named_arguments(
    arguments: Vec<Value>, reference_names: &[Option<ReferenceArgument>], argument_names: &[Option<String>],
    function: &BytecodeFunction, call_name: &str, caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>, global_variables: &mut GlobalVariables,
    global_scope: bool, frame_id: u64,
) -> Result<(Vec<Value>, Vec<Option<ReferenceArgument>>, bool), String> {
    let variadic_index = function.parameter_variadic.iter().position(|variadic| *variadic);
    let reference_variadic = variadic_index.is_some_and(|index| function.parameter_by_reference.get(index).copied().unwrap_or(false));
    if !argument_names.iter().any(Option::is_some) && !reference_variadic {
        return Ok((arguments, reference_names.to_vec(), false));
    }
    let fixed_count = variadic_index.unwrap_or(function.parameters.len());
    let mut slots = vec![None::<(Value, Option<ReferenceArgument>)>; fixed_count];
    let mut variadic_entries = Vec::<(ArrayKey, Value, Option<ReferenceArgument>)>::new();
    let mut next_positional = 0;
    let mut next_variadic_index = 0i64;
    let mut named_seen = false;
    for (index, value) in arguments.into_iter().enumerate() {
        let reference = reference_names.get(index).cloned().flatten();
        if let Some(name) = argument_names.get(index).cloned().flatten() {
            named_seen = true;
            if let Some(parameter_index) = function.parameters[..fixed_count].iter().position(|parameter| parameter == &name) {
                if slots[parameter_index].is_some() {
                    return Err(format!("{call_name}: parameter ${name} was passed more than once"));
                }
                slots[parameter_index] = Some((value, reference));
            } else if variadic_index.is_some() {
                if variadic_entries.iter().any(|(key, _, _)| key == &ArrayKey::String(name.clone())) {
                    return Err(format!("{call_name}: named parameter ${name} was passed more than once"));
                }
                variadic_entries.push((ArrayKey::String(name), value, reference));
            } else {
                return Err(format!("{call_name}: unknown named parameter ${name}"));
            }
        } else {
            if named_seen { return Err(format!("{call_name}: positional argument follows a named argument")); }
            if next_positional < fixed_count {
                slots[next_positional] = Some((value, reference));
                next_positional += 1;
            } else if variadic_index.is_some() {
                variadic_entries.push((ArrayKey::Integer(next_variadic_index), value, reference));
                next_variadic_index += 1;
            } else {
                variadic_entries.push((ArrayKey::Integer(next_variadic_index), value, reference));
                next_variadic_index += 1;
            }
        }
    }
    let mut ordered_values = Vec::new();
    let mut ordered_references = Vec::new();
    for (index, slot) in slots.into_iter().enumerate() {
        if let Some((value, reference)) = slot {
            ordered_values.push(value);
            ordered_references.push(reference);
        } else if let Some(Some(default)) = function.parameter_defaults.get(index) {
            ordered_values.push(default.materialize_parameter_default());
            ordered_references.push(None);
        } else {
            return Err(format!("{call_name}: required parameter ${} was not passed", function.parameters[index]));
        }
    }
    if variadic_index.is_some() {
        let mut packed = crate::PhpArray::from_entries(Vec::new());
        for (key, value, reference) in variadic_entries {
            if reference_variadic {
                let Some(reference) = reference else {
                    return Err(format!("Argument to {call_name} must be a variable passed by reference"));
                };
                let cell = reference_cell_for_argument(
                    &reference, caller_variables, caller_bindings, global_variables, global_scope, frame_id,
                )?;
                packed.set_reference(key, cell);
            } else {
                packed.set(key, value);
            }
        }
        ordered_values.push(Value::Array(packed));
        ordered_references.push(None);
        return Ok((ordered_values, ordered_references, true));
    }
    for (_, value, reference) in variadic_entries {
        ordered_values.push(value);
        ordered_references.push(reference);
    }
    Ok((ordered_values, ordered_references, false))
}

fn reference_cell_for_argument(
    target: &ReferenceArgument, variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>, globals: &mut GlobalVariables,
    global_scope: bool, frame_id: u64,
) -> Result<Rc<std::cell::RefCell<Value>>, String> {
    if let Some(reference) = &target.direct_reference { return Ok(reference.clone()); }
    if let Some((receiver, property)) = &target.object_property {
        let Value::Object(object) = receiver else { return Err("Only object properties can be passed by reference".into()); };
        let mut object = object.borrow_mut();
        let current = object.properties.get(property).cloned().unwrap_or(Value::Null);
        if target.array_keys.is_empty() {
            if let Value::Reference(reference) = &current { return Ok(reference.clone()); }
            let reference = Rc::new(std::cell::RefCell::new(current.dereference()));
            object.properties.insert(property.clone(), Value::Reference(reference.clone()));
            return Ok(reference);
        }
        let (updated, reference) = current.ensure_array_reference(&target.array_keys)?;
        object.properties.insert(property.clone(), updated);
        return Ok(reference);
    }
    let (_, reference) = super::reference_cell_for_path(
        &target.variable, &target.array_keys, variables, bindings, globals, global_scope, frame_id,
    )?;
    Ok(reference)
}

fn write_reference_argument(
    target: &ReferenceArgument,
    value: Value,
    variables: &mut HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
) -> Result<(), String> {
    if let Some(reference) = &target.direct_reference {
        *reference.borrow_mut() = value.dereference();
        return Ok(());
    }
    if let Some((receiver, property)) = &target.object_property {
        let Value::Object(object) = receiver else { return Err("Only objects can have properties".into()); };
        let mut object = object.borrow_mut();
        let current = object.properties.get(property).cloned().unwrap_or(Value::Null);
        let updated = if target.array_keys.is_empty() { value } else { current.array_set_path(&target.array_keys, value)? };
        object.properties.insert(property.clone(), updated);
        return Ok(());
    }
    if target.array_keys.is_empty() {
        return write_variable(target.variable.clone(), value, variables, bindings, globals, global_scope);
    }
    let current = read_variable(&target.variable, variables, bindings, globals, global_scope);
    let updated = current.array_set_path(&target.array_keys, value)?;
    write_array_root(target.variable.clone(), updated, variables, bindings, globals, global_scope)
}

pub(super) fn pdo_iterator_rows(
    value: &Value, variables: &mut HashMap<String, Value>, bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables, global_scope: bool, frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>, classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties, output: &mut String, steps: &mut u64,
) -> Result<Vec<(ArrayKey, Value)>, String> {
    let mut instantiate_row = |row: &Value, class_name: &str, constructor_args: &[Value]| {
        if class_name == "\0PDO_FETCH_FUNC" {
            let packed = Value::from_array_entries(constructor_args.iter().enumerate()
                .map(|(index, value)| (ArrayKey::Integer(index as i64), value.clone())).collect());
            let (instruction, mut callback_stack) = callback_call(row.clone(), packed, false)?;
            execute_instruction(&instruction, &mut callback_stack, variables, bindings, globals, global_scope,
                frame_id, functions, classes, static_properties, output, steps)?;
            return callback_stack.pop().ok_or_else(|| "Invalid PDO::FETCH_FUNC callback result stack".into());
        }
        let (class_name, props_late) = class_name.strip_prefix("\0PDO_FETCH_PROPS_LATE:")
            .map_or((class_name, false), |name| (name, true));
        instantiate_pdo_row(row, class_name, constructor_args, props_late, variables, bindings, globals,
            global_scope, frame_id, functions, classes, static_properties, output, steps)
    };
    crate::pdo::statement_iterator_rows(value, &mut instantiate_row)
}

fn iterator_count(
    arguments: &[Value], variables: &mut HashMap<String, Value>, bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables, global_scope: bool, frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>, classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties, output: &mut String, steps: &mut u64,
) -> Result<Value, String> {
    if arguments.len() != 1 { return Err(format!("iterator_count() expects exactly 1 argument, {} given", arguments.len())); }
    let count = collect_traversable(arguments[0].clone(), variables, bindings, globals, global_scope, frame_id,
        functions, classes, static_properties, output, steps, 0, true, false).map_err(|error| {
            if error == "Value is not Traversable|array" { "iterator_count(): Argument #1 ($iterator) must be of type Traversable|array".into() } else { error }
        })?.len();
    i64::try_from(count).map(Value::Integer).map_err(|_| "iterator_count(): iterator is too large".into())
}

fn iterator_to_array(
    arguments: &[Value], variables: &mut HashMap<String, Value>, bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables, global_scope: bool, frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>, classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties, output: &mut String, steps: &mut u64,
) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!("iterator_to_array() expects 1 to 2 arguments, {} given", arguments.len()));
    }
    let preserve_keys = arguments.get(1).is_none_or(Value::truthy);
    let mut result = Vec::<(ArrayKey, Value)>::new();
    let mut insert_entry = |key: ArrayKey, value: Value| {
        let key = if preserve_keys { key } else { ArrayKey::Integer(result.len() as i64) };
        if let Some((_, existing)) = result.iter_mut().find(|(existing_key, _)| *existing_key == key) {
            *existing = value.dereference();
        } else {
            result.push((key, value.dereference()));
        }
    };
    for (key, value) in collect_traversable(arguments[0].clone(), variables, bindings, globals, global_scope, frame_id,
        functions, classes, static_properties, output, steps, 0, !preserve_keys, preserve_keys).map_err(|error| {
            if error == "Value is not Traversable|array" { "iterator_to_array(): Argument #1 ($iterator) must be of type Traversable|array".into() } else { error }
        })? {
        insert_entry(key, value);
    }
    Ok(Value::from_array_entries(result))
}

fn json_error_code(message: &str) -> i64 {
    if message.contains("Maximum stack depth") { 1 }
    else if message.contains("Unescaped control character") { 3 }
    else if message.contains("Unicode surrogate") { 10 }
    else if message.contains("Malformed UTF-8") { 5 }
    else if message.contains("Recursion") { 6 }
    else if message.contains("Inf and NaN") { 7 }
    else if message.contains("Non-backed enums") { 11 }
    else if message.contains("Invalid property name") || message.contains("decoded property name is invalid") { 9 }
    else if message.contains("Closures") || message.contains("Generators") { 8 }
    else { 4 }
}

fn json_exception_marker(
    global_variables: &mut GlobalVariables,
    step: u64,
    code: i64,
    message: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<String, String> {
    let exception = crate::exception_builtins::construct(
        "JsonException", &[Value::String(message.to_string()), Value::Integer(code)], classes,
    )?;
    global_variables.store_thrown(step, exception);
    Ok(format!("{}{}", super::THROWN_VALUE_PREFIX, step))
}

fn json_error_message(code: i64) -> &'static str {
    match code {
        0 => "No error",
        1 => "Maximum stack depth exceeded",
        2 => "State mismatch (invalid or malformed JSON)",
        3 => "Control character error, possibly incorrectly encoded",
        4 => "Syntax error",
        5 => "Malformed UTF-8 characters, possibly incorrectly encoded",
        6 => "Recursion detected",
        7 => "Inf and NaN cannot be JSON encoded",
        8 => "Type is not supported",
        9 => "The decoded property name is invalid",
        10 => "Single unpaired UTF-16 surrogate in unicode escape",
        11 => "Non-backed enums have no default serialization",
        _ => "Unknown error",
    }
}

#[derive(Clone, Copy)]
enum FilesystemAccess { Read, Write, Execute }

/// Resolve PHP filesystem paths against the server's application root when
/// the request installed a sandbox root. CLI executions do not set one.
fn filesystem_path(global_variables: &GlobalVariables, path: &std::path::Path, allow_create: bool) -> Option<PathBuf> {
    let Some(root) = global_variables.get("\0filesystem_root") else { return Some(path.to_path_buf()); };
    let root = std::path::PathBuf::from(root.show()).canonicalize().ok()?;
    let candidate = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().ok()?.join(path) };
    if let Ok(canonical) = candidate.canonicalize() {
        return canonical.starts_with(&root).then_some(canonical);
    }
    if !allow_create || std::fs::symlink_metadata(&candidate).is_ok() { return None; }
    let name = candidate.file_name()?;
    let parent = candidate.parent()?.canonicalize().ok()?;
    parent.starts_with(&root).then(|| parent.join(name))
}

#[cfg(unix)]
fn filesystem_access(path: &std::path::Path, access: FilesystemAccess) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else { return false; };
    let mode = match access { FilesystemAccess::Read => libc::R_OK, FilesystemAccess::Write => libc::W_OK, FilesystemAccess::Execute => libc::X_OK };
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}

#[cfg(not(unix))]
fn filesystem_access(path: &std::path::Path, access: FilesystemAccess) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return false; };
    match access {
        FilesystemAccess::Read => true,
        FilesystemAccess::Write => !metadata.permissions().readonly(),
        FilesystemAccess::Execute => false,
    }
}

fn file_put_contents_bytes(value: &Value) -> Option<Vec<u8>> {
    match value.dereference() {
        Value::Stream(stream) => stream.borrow_mut().read(None).ok(),
        Value::Array(array) => {
            let mut bytes = Vec::new();
            for (_, item) in &array.entries { bytes.extend_from_slice(item.show().as_bytes()); }
            Some(bytes)
        }
        Value::Object(_) | Value::Closure(_) | Value::Generator(_) => None,
        Value::Null => Some(Vec::new()),
        scalar => Some(scalar.show().into_bytes()),
    }
}

fn unix_time() -> (u64, u32) {
    let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (elapsed.as_secs(), elapsed.subsec_micros())
}

fn gettimeofday_value(as_float: bool) -> Result<Value, String> {
    let (seconds, microseconds) = unix_time();
    if as_float {
        return Ok(Value::Number(seconds as f64 + microseconds as f64 / 1_000_000.0));
    }
    let tm = date_tm(seconds as i64, false)?;
    let minutes_west = {
        #[cfg(target_os = "linux")]
        { -(tm.tm_gmtoff as i64) / 60 }
        #[cfg(not(target_os = "linux"))]
        { 0i64 }
    };
    Ok(Value::from_array_entries(vec![
        (ArrayKey::String("sec".into()), Value::Integer(seconds as i64)),
        (ArrayKey::String("usec".into()), Value::Integer(microseconds as i64)),
        (ArrayKey::String("minuteswest".into()), Value::Integer(minutes_west)),
        (ArrayKey::String("dsttime".into()), Value::Integer(i64::from(tm.tm_isdst > 0))),
    ]))
}

fn random_int_bound(function: &str, value: &Value, argument: usize) -> Result<i64, String> {
    let value = value.dereference();
    match value {
        Value::Integer(number) => Ok(number),
        Value::Number(number) if number.is_finite() && number >= i64::MIN as f64 && number < -(i64::MIN as f64) => Ok(number.trunc() as i64),
        Value::String(text) => {
            if let Ok(number) = text.trim().parse::<i64>() { return Ok(number); }
            let number = text.trim().parse::<f64>().ok().filter(|number| number.is_finite() && *number >= i64::MIN as f64 && *number < -(i64::MIN as f64));
            number.map(|number| number.trunc() as i64).ok_or_else(|| format!("TypeError:{function}(): Argument #{argument} must be a valid integer"))
        }
        Value::Bool(value) => Ok(i64::from(value)),
        Value::Null => Ok(0),
        value => Err(format!("TypeError:{function}(): Argument #{argument} must be of type int, {} given", value.type_name())),
    }
}

fn date_timestamp(value: &Value) -> Result<i64, String> {
    match value.dereference() {
        Value::Integer(value) => Ok(value),
        Value::Number(value) if value.is_finite() && value >= i64::MIN as f64 && value < -(i64::MIN as f64) => Ok(value.trunc() as i64),
        Value::String(value) => value.trim().parse::<i64>().or_else(|_| value.trim().parse::<f64>().map(|number| number.trunc() as i64))
            .map_err(|_| "TypeError:date(): Argument #2 ($timestamp) must be of type ?int".into()),
        Value::Bool(value) => Ok(i64::from(value)),
        Value::Null => Ok(unix_time().0 as i64),
        value => Err(format!("TypeError:date(): Argument #2 ($timestamp) must be of type ?int, {} given", value.type_name())),
    }
}

fn secure_random_integer(min: i64, max: i64) -> Result<i64, String> {
    let width = (i128::from(max) - i128::from(min) + 1) as u128;
    let sample_space = u64::MAX as u128 + 1;
    let accepted_limit = sample_space / width * width;
    loop {
        let mut bytes = [0u8; 8];
        std::fs::File::open("/dev/urandom").and_then(|mut source| std::io::Read::read_exact(&mut source, &mut bytes))
            .map_err(|error| format!("secure random source failed: {error}"))?;
        let sample = u64::from_ne_bytes(bytes) as u128;
        if sample < accepted_limit {
            return Ok((i128::from(min) + (sample % width) as i128) as i64);
        }
    }
}

fn date_component(value: &Value) -> Option<i64> {
    match value.dereference() {
        Value::Integer(value) => Some(value),
        Value::Number(value) if value.is_finite() => Some(value.trunc() as i64),
        Value::String(value) => value.trim().parse::<i64>().ok().or_else(|| value.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| n.trunc() as i64)),
        Value::Bool(value) => Some(i64::from(value)),
        Value::Null => Some(0),
        _ => None,
    }
}

fn date_tm(timestamp: i64, utc: bool) -> Result<libc::tm, String> {
    let timestamp = timestamp as libc::time_t;
    let mut broken_down = std::mem::MaybeUninit::<libc::tm>::zeroed();
    let result = unsafe {
        if utc { libc::gmtime_r(&timestamp, broken_down.as_mut_ptr()) }
        else { libc::localtime_r(&timestamp, broken_down.as_mut_ptr()) }
    };
    if result.is_null() { return Err("date/time value is out of range".into()); }
    Ok(unsafe { broken_down.assume_init() })
}

fn make_date_timestamp(parts: [i64; 6], utc: bool) -> Option<i64> {
    let year = match parts[5] { 0..=69 => parts[5] + 2000, 70..=100 => parts[5] + 1900, _ => parts[5] };
    let mut tm = unsafe { std::mem::MaybeUninit::<libc::tm>::zeroed().assume_init() };
    tm.tm_sec = parts[2] as libc::c_int;
    tm.tm_min = parts[1] as libc::c_int;
    tm.tm_hour = parts[0] as libc::c_int;
    tm.tm_mday = parts[4] as libc::c_int;
    tm.tm_mon = (parts[3] - 1) as libc::c_int;
    tm.tm_year = (year - 1900) as libc::c_int;
    tm.tm_isdst = if utc { 0 } else { -1 };
    let timestamp = unsafe { if utc { libc::timegm(&mut tm) } else { libc::mktime(&mut tm) } };
    Some(timestamp as i64)
}

fn parse_php_strtotime(source: &str, base: i64) -> Option<i64> {
    let input = source.trim();
    let normalized = input.to_ascii_lowercase();
    if normalized == "now" { return Some(base); }
    let current = date_tm(base, false).ok()?;
    let mut parts = [current.tm_hour as i64, current.tm_min as i64, current.tm_sec as i64,
        (current.tm_mon + 1) as i64, current.tm_mday as i64, (current.tm_year + 1900) as i64];
    match normalized.as_str() {
        "today" | "tomorrow" | "yesterday" => {
            parts[0] = 0; parts[1] = 0; parts[2] = 0;
            if normalized == "tomorrow" { parts[4] += 1; }
            if normalized == "yesterday" { parts[4] -= 1; }
            return make_date_timestamp(parts, false);
        }
        _ => {}
    }
    if let Some((amount, unit)) = input.split_once(char::is_whitespace) {
        let amount = amount.parse::<i64>().ok()?;
        if !matches!(unit.trim().split_whitespace().count(), 1) { return None; }
        let unit = unit.trim().trim_end_matches('s').to_ascii_lowercase();
        match unit.as_str() {
            "second" => parts[2] = parts[2].checked_add(amount)?,
            "minute" => parts[1] = parts[1].checked_add(amount)?,
            "hour" => parts[0] = parts[0].checked_add(amount)?,
            "day" => parts[4] = parts[4].checked_add(amount)?,
            "week" => parts[4] = parts[4].checked_add(amount.checked_mul(7)?)?,
            "month" => parts[3] = parts[3].checked_add(amount)?,
            "year" => parts[5] = parts[5].checked_add(amount)?,
            _ => return None,
        }
        return make_date_timestamp(parts, false);
    }

    let (date_part, time_part) = if let Some((date, time)) = input.split_once('T').or_else(|| input.split_once('t')) {
        (date, Some(time))
    } else if let Some((date, time)) = input.split_once(' ') { (date, Some(time.trim())) }
    else { (input, None) };
    let mut date_fields = date_part.split('-');
    let year = date_fields.next()?.parse::<i64>().ok()?;
    let month = date_fields.next()?.parse::<i64>().ok()?;
    let day = date_fields.next()?.parse::<i64>().ok()?;
    if date_fields.next().is_some() || !(1..=32767).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) { return None; }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if day > month_days[(month - 1) as usize] { return None; }
    parts[3] = month; parts[4] = day; parts[5] = year;
    let mut utc = false;
    let mut offset_seconds = 0i64;
    if let Some(mut time) = time_part {
        if time.ends_with('Z') || time.ends_with('z') { utc = true; time = &time[..time.len() - 1]; }
        if let Some(zone_start) = time.char_indices().skip(5).find_map(|(index, ch)| matches!(ch, '+' | '-').then_some(index)) {
            let zone = &time[zone_start..];
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let mut offset = zone[1..].split(':');
            let hours = offset.next()?.parse::<i64>().ok()?;
            let minutes = offset.next()?.parse::<i64>().ok()?;
            if offset.next().is_some() || hours > 23 || minutes > 59 { return None; }
            offset_seconds = sign * (hours * 3600 + minutes * 60);
            utc = true;
            time = &time[..zone_start];
        }
        let mut clock = time.split(':');
        parts[0] = clock.next()?.parse().ok()?;
        parts[1] = clock.next()?.parse().ok()?;
        if let Some(second) = clock.next() { parts[2] = second.parse().ok()?; }
        if clock.next().is_some() || !(0..=24).contains(&parts[0]) || !(0..=59).contains(&parts[1]) || !(0..=60).contains(&parts[2]) { return None; }
    } else { parts[0] = 0; parts[1] = 0; parts[2] = 0; }
    let timestamp = make_date_timestamp(parts, utc)?;
    Some(timestamp - offset_seconds)
}

fn getdate_value(timestamp: i64) -> Result<Value, String> {
    let timestamp = timestamp as libc::time_t;
    let mut broken_down = std::mem::MaybeUninit::<libc::tm>::zeroed();
    let result = unsafe { libc::localtime_r(&timestamp, broken_down.as_mut_ptr()) };
    if result.is_null() { return Err("getdate(): timestamp is out of range".into()); }
    let tm = unsafe { broken_down.assume_init() };
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let mut entries = vec![
        (ArrayKey::Integer(0), Value::Integer(timestamp as i64)),
        (ArrayKey::String("seconds".into()), Value::Integer(tm.tm_sec as i64)),
        (ArrayKey::String("minutes".into()), Value::Integer(tm.tm_min as i64)),
        (ArrayKey::String("hours".into()), Value::Integer(tm.tm_hour as i64)),
        (ArrayKey::String("mday".into()), Value::Integer(tm.tm_mday as i64)),
        (ArrayKey::String("wday".into()), Value::Integer(tm.tm_wday as i64)),
        (ArrayKey::String("mon".into()), Value::Integer((tm.tm_mon + 1) as i64)),
        (ArrayKey::String("year".into()), Value::Integer((tm.tm_year + 1900) as i64)),
        (ArrayKey::String("yday".into()), Value::Integer(tm.tm_yday as i64)),
        (ArrayKey::String("weekday".into()), Value::String(DAYS[tm.tm_wday.clamp(0, 6) as usize].into())),
        (ArrayKey::String("month".into()), Value::String(MONTHS[tm.tm_mon.clamp(0, 11) as usize].into())),
    ];
    entries.shrink_to_fit();
    Ok(Value::from_array_entries(entries))
}

fn localtime_value(timestamp: i64, associative: bool) -> Result<Value, String> {
    let tm = date_tm(timestamp, false)?;
    let fields = [
        ("tm_sec", tm.tm_sec), ("tm_min", tm.tm_min), ("tm_hour", tm.tm_hour),
        ("tm_mday", tm.tm_mday), ("tm_mon", tm.tm_mon), ("tm_year", tm.tm_year),
        ("tm_wday", tm.tm_wday), ("tm_yday", tm.tm_yday), ("tm_isdst", tm.tm_isdst),
    ];
    Ok(Value::from_array_entries(fields.iter().enumerate().map(|(index, (name, value))| {
        let key = if associative { ArrayKey::String((*name).into()) } else { ArrayKey::Integer(index as i64) };
        (key, Value::Integer(*value as i64))
    }).collect()))
}

fn high_resolution_time() -> (u64, u32) {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let elapsed = ORIGIN.get_or_init(std::time::Instant::now).elapsed();
    (elapsed.as_secs(), elapsed.subsec_nanos())
}

fn format_php_date(format: &str, timestamp: i64, utc: bool) -> Result<String, String> {
    let timestamp = timestamp as libc::time_t;
    let mut broken_down = std::mem::MaybeUninit::<libc::tm>::zeroed();
    let result = unsafe {
        if utc { libc::gmtime_r(&timestamp, broken_down.as_mut_ptr()) }
        else { libc::localtime_r(&timestamp, broken_down.as_mut_ptr()) }
    };
    if result.is_null() { return Err("date(): timestamp is out of range".into()); }
    let tm = unsafe { broken_down.assume_init() };
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let year = tm.tm_year + 1900;
    let month = tm.tm_mon + 1;
    let day = tm.tm_mday;
    let hour = tm.tm_hour;
    let weekday = tm.tm_wday.clamp(0, 6) as usize;
    let month_index = tm.tm_mon.clamp(0, 11) as usize;
    let offset = {
        #[cfg(target_os = "linux")]
        { tm.tm_gmtoff as i64 }
        #[cfg(not(target_os = "linux"))]
        { 0i64 }
    };
    let absolute_offset = offset.unsigned_abs();
    let suffix = match day % 100 { 11..=13 => "th", _ => match day % 10 { 1 => "st", 2 => "nd", 3 => "rd", _ => "th" } };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let iso_week = date_strftime("%V", &tm);
    let iso_year = date_strftime("%G", &tm);
    let mut output = String::new();
    let mut characters = format.chars().peekable();
    while let Some(token) = characters.next() {
        if token == '\\' {
            output.push(characters.next().unwrap_or('\\'));
            continue;
        }
        match token {
            'd' => output.push_str(&format!("{day:02}")), 'D' => output.push_str(&DAYS[weekday][..3]),
            'j' => output.push_str(&day.to_string()), 'l' => output.push_str(DAYS[weekday]),
            'N' => output.push_str(&(if weekday == 0 { 7 } else { weekday }).to_string()),
            'S' => output.push_str(suffix), 'w' => output.push_str(&weekday.to_string()), 'z' => output.push_str(&tm.tm_yday.to_string()),
            'W' => output.push_str(iso_week.trim()), 'F' => output.push_str(MONTHS[month_index]), 'm' => output.push_str(&format!("{month:02}")),
            'M' => output.push_str(&MONTHS[month_index][..3]), 'n' => output.push_str(&month.to_string()), 't' => output.push_str(&month_days[month_index].to_string()),
            'L' => output.push(if leap { '1' } else { '0' }), 'o' => output.push_str(iso_year.trim()), 'Y' => output.push_str(&year.to_string()),
            'y' => output.push_str(&format!("{:02}", year.rem_euclid(100))), 'a' => output.push_str(if hour < 12 { "am" } else { "pm" }),
            'A' => output.push_str(if hour < 12 { "AM" } else { "PM" }), 'g' | 'h' => {
                let twelve = match hour % 12 { 0 => 12, value => value };
                if token == 'h' { output.push_str(&format!("{twelve:02}")); } else { output.push_str(&twelve.to_string()); }
            }
            'G' => output.push_str(&hour.to_string()), 'H' => output.push_str(&format!("{hour:02}")),
            'i' => output.push_str(&format!("{:02}", tm.tm_min)), 's' => output.push_str(&format!("{:02}", tm.tm_sec)),
            'u' => output.push_str("000000"), 'v' => output.push_str("000"),
            'e' => {
                #[cfg(target_os = "linux")]
                { if !utc { let zone = unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }.to_string_lossy(); output.push_str(&zone); } else { output.push_str("UTC"); } }
                #[cfg(not(target_os = "linux"))]
                { output.push_str("UTC"); }
            }
            'I' => output.push(if tm.tm_isdst > 0 { '1' } else { '0' }),
            'O' | 'P' => {
                let sign = if offset < 0 { '-' } else { '+' };
                if token == 'P' { output.push_str(&format!("{sign}{:02}:{:02}", absolute_offset / 3600, absolute_offset % 3600 / 60)); }
                else { output.push_str(&format!("{sign}{:02}{:02}", absolute_offset / 3600, absolute_offset % 3600 / 60)); }
            }
            'p' => { if offset == 0 { output.push('Z'); } else { output.push_str(&format_php_date("P", timestamp as i64, utc)?); } }
            'T' => {
                #[cfg(target_os = "linux")]
                { output.push_str(&unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }.to_string_lossy()); }
                #[cfg(not(target_os = "linux"))]
                { output.push_str("UTC"); }
            }
            'Z' => output.push_str(&offset.to_string()),
            'B' => { let beats = (timestamp.rem_euclid(86_400) + 3_600).rem_euclid(86_400) * 1_000 / 86_400; output.push_str(&format!("{beats:03}")); }
            'c' => output.push_str(&format_php_date("Y-m-d\\TH:i:sP", timestamp as i64, utc)?),
            'r' => output.push_str(&format_php_date("D, d M Y H:i:s O", timestamp as i64, utc)?),
            'U' => output.push_str(&(timestamp as i64).to_string()),
            other => output.push(other),
        }
    }
    Ok(output)
}

fn date_strftime(format: &str, tm: &libc::tm) -> String {
    let Ok(format) = std::ffi::CString::new(format) else { return String::new(); };
    let mut buffer = [0u8; 32];
    let length = unsafe { libc::strftime(buffer.as_mut_ptr().cast(), buffer.len(), format.as_ptr(), tm) };
    String::from_utf8_lossy(&buffer[..length]).into_owned()
}

fn write_file_contents(path: &std::path::Path, bytes: &[u8], flags: i64) -> std::io::Result<usize> {
    use std::io::{Seek, SeekFrom, Write};
    let append = flags & 8 != 0;
    let lock = flags & 2 != 0;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).append(append);
    let mut file = options.open(path)?;
    #[cfg(unix)]
    if lock && unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_EX) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let result = (|| {
        if !append {
            file.set_len(0)?;
            file.seek(SeekFrom::Start(0))?;
        }
        file.write_all(bytes)?;
        Ok(bytes.len())
    })();
    #[cfg(unix)]
    if lock { unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_UN); } }
    result
}

fn create_temporary_file(directory: &std::path::Path, prefix: &str) -> std::io::Result<std::path::PathBuf> {
    static NEXT_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let prefix = prefix.chars().take(32).map(|character| if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') { character } else { '_' }).collect::<String>();
    loop {
        let id = NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = directory.join(format!("{prefix}{}-{id}.tmp", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

fn coerce_stringable_builtin_args(
    function_name: &str,
    arguments: &mut [Value],
    positions: &[usize],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    for &position in positions {
        let Some(value) = arguments.get_mut(position) else { continue; };
        if matches!(value.dereference(), Value::Object(_)) {
            *value = super::objects::stringify(value, functions, classes, static_properties, global_variables, output, steps)
                .map(Value::String)
                .map_err(|error| format!("{function_name}(): {error}"))?;
        }
    }
    Ok(())
}

fn format_dump_value(
    value: &Value,
    depth: usize,
    seen: &mut HashSet<usize>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<String, String> {
    let indent = "  ".repeat(depth);
    let nested_indent = "  ".repeat(depth + 1);
    if let Value::Reference(reference) = value {
        let identity = Rc::as_ptr(reference) as usize;
        if !seen.insert(identity) { return Ok("*RECURSION*\n".into()); }
        let dump = format_dump_value(&reference.borrow(), depth, seen, functions, classes, static_properties, global_variables, output, steps);
        seen.remove(&identity);
        return dump;
    }
    let value = value.dereference();
    Ok(match value {
        Value::Null => "NULL\n".into(),
        Value::Bool(value) => format!("bool({value})\n"),
        Value::Integer(value) => format!("int({value})\n"),
        Value::Number(value) => format!("float({})\n", Value::Number(value).show()),
        Value::String(value) => format!("string({}) \"{}\"\n", value.len(), value.escape_default()),
        Value::Array(array) => {
            if depth >= 64 { return Err("var_dump(): nesting level too deep".into()); }
            let mut dump = format!("array({}) {{\n", array.len());
            for (key, item) in array.iter() {
                let key = match key {
                    ArrayKey::Integer(key) => format!("[{key}]"),
                    ArrayKey::String(key) => format!("[\"{}\"]", key.escape_default()),
                };
                dump.push_str(&format!("{nested_indent}{key}=>\n"));
                dump.push_str(&format_dump_value(item, depth + 1, seen, functions, classes, static_properties, global_variables, output, steps)?);
            }
            dump.push_str(&format!("{indent}}}\n"));
            dump
        }
        Value::Object(object) => {
            let identity = Rc::as_ptr(&object) as usize;
            if !seen.insert(identity) { return Ok("*RECURSION*\n".into()); }
            if depth >= 64 { return Err("var_dump(): nesting level too deep".into()); }
            let (class_name, raw_properties, property_order) = {
                let object = object.borrow();
                (object.class_name.clone(), object.properties.clone(), object.property_order.clone())
            };
            let properties = if let Some(method) = super::objects::find_method(&class_name, "__debuginfo", classes) {
                let mut locals = HashMap::from([("this".into(), Value::Object(object.clone()))]);
                let returned = execute_code(&method.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(method), HashMap::new(), output, steps)?
                    .unwrap_or(Value::Null);
                match returned.dereference() {
                    Value::Array(properties) => Value::Array(properties),
                    other => return Err(format!("TypeError:{class_name}::__debugInfo() must return an array, {} returned", other.type_name())),
                }
            } else {
                let mut names = property_order;
                let mut remaining = raw_properties.keys().filter(|name| !name.starts_with('\0') && !names.contains(name)).cloned().collect::<Vec<_>>();
                remaining.sort();
                names.extend(remaining);
                Value::from_array_entries(names.into_iter().filter_map(|name| raw_properties.get(&name).cloned().map(|value| (ArrayKey::String(name), value))).collect())
            };
            let Value::Array(properties) = properties.dereference() else { unreachable!() };
            let mut dump = format!("object({class_name}) {{\n");
            for (key, item) in properties.iter() {
                let key = match key {
                    ArrayKey::Integer(key) => format!("[{key}]"),
                    ArrayKey::String(key) => format!("[\"{}\"]", key.escape_default()),
                };
                dump.push_str(&format!("{nested_indent}{key}=>\n"));
                dump.push_str(&format_dump_value(item, depth + 1, seen, functions, classes, static_properties, global_variables, output, steps)?);
            }
            dump.push_str(&format!("{indent}}}\n"));
            seen.remove(&identity);
            dump
        }
        Value::Closure(_) => "object(Closure)\n".into(),
        Value::Generator(_) => "object(Generator)\n".into(),
        Value::Stream(_) => "resource(stream) of type (stream)\n".into(),
        Value::Reference(_) => unreachable!(),
    })
}

fn runtime_function_exists(name: &str, functions: &HashMap<String, BytecodeFunction>) -> bool {
    let name = name.trim_start_matches('\\').to_ascii_lowercase();
    functions.contains_key(&name) || crate::array_builtins::is_supported(&name) || crate::math_builtins::is_supported(&name) || crate::string_builtins::is_supported(&name) || matches!(name.as_str(),
        "call_user_func" | "call_user_func_array" | "func_get_args" | "func_get_arg" | "func_num_args" | "var_dump" |
        "function_exists" | "php_sapi_name" | "error_reporting" | "error_get_last" | "error_clear_last" | "class_exists" | "interface_exists" | "trait_exists" | "method_exists" | "property_exists" | "get_class" | "get_parent_class" | "get_called_class" | "is_a" | "is_subclass_of" | "is_callable" |
        "mongo_find" | "mongo_aggregate" | "mongo_count_documents" | "mongo_estimated_document_count" | "mongo_insert_one" | "mongo_insert_many" | "mongo_replace_one" | "mongo_update_one" | "mongo_update_many" | "mongo_delete_one" | "mongo_delete_many" | "mongo_begin_transaction" | "mongo_commit" | "mongo_rollback" |
        "header" | "http_response_code" | "header_remove" | "headers_list" | "file_exists" | "is_file" | "is_dir" | "is_link" | "is_readable" | "is_writable" | "is_executable" | "filesize" | "realpath" | "getcwd" | "sys_get_temp_dir" | "tempnam" | "unlink" | "file_get_contents" | "file_put_contents" | "file" | "fopen" | "fwrite" | "fread" | "stream_get_contents" | "rewind" | "fseek" | "ftell" | "feof" | "fclose" | "time" | "microtime" | "hrtime" | "random_int" | "rand" | "mt_rand" | "getrandmax" | "mt_getrandmax" | "date" | "gmdate" | "getdate" | "gettimeofday" | "idate" | "checkdate" | "mktime" | "gmmktime" | "localtime" | "strtotime" |
        "json_encode" | "json_decode" | "json_validate" | "json_last_error" | "json_last_error_msg" | "is_uploaded_file" | "move_uploaded_file" |
        "session_start" | "session_destroy" | "session_unset" | "session_status" | "session_id" |
        "strtolower" | "strtoupper" | "strlen" | "ucfirst" | "lcfirst" | "strrev" | "bin2hex" | "ord" | "number_format" | "str_increment" | "str_decrement" | "strspn" | "strcspn" | "strpbrk" | "str_word_count" | "strcmp" | "strcasecmp" | "strncmp" | "strncasecmp" | "strnatcmp" | "strnatcasecmp" | "strtr" | "strstr" | "strchr" | "stristr" | "ucwords" | "trim" | "ltrim" | "rtrim" | "str_repeat" | "str_pad" |
        "str_rot13" | "str_shuffle" | "addslashes" | "stripslashes" |
        "str_split" | "strpos" | "stripos" | "strrpos" | "strripos" | "substr_count" | "substr_compare" | "substr" | "explode" | "implode" | "htmlspecialchars" |
        "htmlspecialchars_decode" | "nl2br" | "str_contains" | "str_starts_with" | "str_ends_with" |
        "str_replace" | "str_ireplace" | "define" | "defined" | "constant" | "compact" | "extract" | "get_defined_vars" |
        "intval" | "floatval" | "strval" | "boolval" | "gettype" | "get_debug_type" | "is_int" | "is_integer" | "is_long" | "is_float" | "is_double" | "is_real" |
        "is_string" | "is_array" | "is_countable" | "is_iterable" | "is_bool" | "is_null" | "is_object" | "is_resource" | "is_scalar" | "is_numeric")
}

fn type_builtin(name: &str, arguments: &[Value], classes: &HashMap<String, BytecodeClass>) -> Result<Value, String> {
    let max_arguments = if name == "intval" { 2 } else { 1 };
    if arguments.is_empty() || arguments.len() > max_arguments {
        if name == "intval" {
            let (expectation, noun) = if arguments.is_empty() { ("at least 1", "argument") } else { ("at most 2", "arguments") };
            return Err(format!("ArgumentCountError:intval() expects {expectation} {noun}, {} given", arguments.len()));
        }
        return Err(format!("ArgumentCountError:{name}() expects exactly 1 argument, {} given", arguments.len()));
    }
    let value = &arguments[0];
    let value = value.dereference();
    let is_integer = matches!(&value, Value::Integer(_));
    let is_float = matches!(&value, Value::Number(_));
    let is_numeric = match &value {
        Value::Integer(_) | Value::Number(_) => true,
        Value::String(text) => {
            let text = text.trim();
            !text.is_empty()
                && !matches!(text.to_ascii_lowercase().as_str(), "nan" | "+nan" | "-nan" | "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity")
                && text.parse::<f64>().is_ok()
        }
        _ => false,
    };
    let result = match name {
        "intval" => {
            let integer = match &value {
                Value::String(text) => {
                    let base = arguments.get(1).map_or(10, |value| value.number() as u32);
                    if base != 0 && !(2..=36).contains(&base) {
                        return Err("ValueError:intval(): Argument #2 ($base) must be between 2 and 36".into());
                    }
                    intval_string(text, base)
                }
                _ => value.number().trunc() as i64,
            };
            Value::Integer(integer)
        }
        "floatval" => Value::Number(value.number()),
        "strval" => Value::String(value.show()),
        "boolval" => Value::Bool(value.truthy()),
        "gettype" => Value::String(match &value {
            Value::Null => "NULL",
            Value::Bool(_) => "boolean",
            Value::Integer(_) => "integer",
            Value::Number(_) => "double",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Stream(_) => "resource",
            Value::Object(_) | Value::Closure(_) | Value::Generator(_) => "object",
            Value::Reference(_) => unreachable!(),
        }.into()),
        "get_debug_type" => Value::String(match &value {
            Value::Null => "null".into(),
            Value::Bool(_) => "bool".into(),
            Value::Integer(_) => "int".into(),
            Value::Number(_) => "float".into(),
            Value::String(_) => "string".into(),
            Value::Array(_) => "array".into(),
            Value::Object(object) => object.borrow().class_name.clone(),
            Value::Closure(_) => "Closure".into(),
            Value::Generator(_) => "Generator".into(),
            Value::Stream(_) => "resource (stream)".into(),
            Value::Reference(_) => unreachable!(),
        }),
        "is_int" | "is_integer" | "is_long" => Value::Bool(is_integer),
        "is_float" | "is_double" | "is_real" => Value::Bool(is_float),
        "is_string" => Value::Bool(matches!(&value, Value::String(_))),
        "is_array" => Value::Bool(matches!(&value, Value::Array(_))),
        "is_countable" => Value::Bool(crate::array_builtins::is_countable(&value, classes)),
        "is_iterable" => Value::Bool(super::value_is_iterable(&value, classes)),
        "is_bool" => Value::Bool(matches!(&value, Value::Bool(_))),
        "is_null" => Value::Bool(matches!(&value, Value::Null)),
        "is_object" => Value::Bool(matches!(&value, Value::Object(_) | Value::Closure(_) | Value::Generator(_))),
        "is_resource" => Value::Bool(matches!(&value, Value::Stream(stream) if stream.borrow().is_open())),
        "is_scalar" => Value::Bool(matches!(&value, Value::Bool(_) | Value::Integer(_) | Value::Number(_) | Value::String(_))),
        "is_numeric" => Value::Bool(is_numeric),
        _ => unreachable!("type builtin dispatch is exhaustive"),
    };
    Ok(result)
}

pub(super) fn intval_string(input: &str, requested_base: u32) -> i64 {
    let text = input.trim_start_matches(|character| matches!(character, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\0'));
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (base, digits) = if requested_base == 0 {
        if let Some(rest) = unsigned.strip_prefix("0x").or_else(|| unsigned.strip_prefix("0X")) {
            (16, rest)
        } else if let Some(rest) = unsigned.strip_prefix("0b").or_else(|| unsigned.strip_prefix("0B")) {
            (2, rest)
        } else if unsigned.starts_with('0') {
            (8, unsigned)
        } else {
            (10, unsigned)
        }
    } else if requested_base == 16 {
        (16, unsigned.strip_prefix("0x").or_else(|| unsigned.strip_prefix("0X")).unwrap_or(unsigned))
    } else if requested_base == 2 {
        (2, unsigned.strip_prefix("0b").or_else(|| unsigned.strip_prefix("0B")).unwrap_or(unsigned))
    } else {
        (requested_base, unsigned)
    };

    // Decimal floats and exponents are converted as numbers before truncation.
    if base == 10 {
        let mut numeric_prefix = None;
        for (index, _) in digits.char_indices().skip(1).chain(std::iter::once((digits.len(), '\0'))) {
            let prefix = &digits[..index];
            if let Ok(number) = prefix.parse::<f64>() {
                numeric_prefix = Some(number);
            }
        }
        if let Some(number) = numeric_prefix {
            let signed = if negative { -number } else { number };
            return signed.trunc() as i64;
        }
    }

    let mut found = false;
    let mut magnitude = 0i128;
    for character in digits.chars() {
        let digit = match character {
            '0'..='9' => character as u32 - '0' as u32,
            'a'..='z' => character as u32 - 'a' as u32 + 10,
            'A'..='Z' => character as u32 - 'A' as u32 + 10,
            _ => break,
        };
        if digit >= base { break; }
        found = true;
        magnitude = magnitude.saturating_mul(base as i128).saturating_add(digit as i128);
    }
    if !found { return 0; }
    let signed = if negative { magnitude.saturating_neg() } else { magnitude };
    signed.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn declared_property_exists(
    class_name: &str,
    property: &str,
    classes: &HashMap<String, BytecodeClass>,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let key = class_name.trim_start_matches('\\').to_ascii_lowercase();
    if !visited.insert(key.clone()) { return false; }
    let Some(class) = classes.get(&key) else { return false; };
    class.properties.contains_key(property)
        || class.static_properties.contains_key(property)
        || class.parent.as_deref().is_some_and(|parent| declared_property_exists(parent, property, classes, visited))
}

fn callable_status(
    value: &Value,
    syntax_only: bool,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
) -> (bool, String) {
    match value.dereference() {
        Value::Closure(_) => (true, "Closure::__invoke".into()),
        Value::String(name) => {
            if let Some((class, method)) = name.split_once("::") {
                let class = class.trim_start_matches('\\').to_ascii_lowercase();
                let method = method.to_ascii_lowercase();
                let valid = !class.is_empty() && !method.is_empty();
                let callable = valid && (syntax_only || classes.get(&class).is_some_and(|_| {
                    find_method(&class, &method, classes).is_some_and(|function| function.is_static && function.visibility == MemberVisibility::Public)
                        || find_method(&class, "__callstatic", classes).is_some_and(|function| function.is_static && function.visibility == MemberVisibility::Public)
                }));
                (callable, format!("{class}::{method}"))
            } else {
                let valid = !name.trim().is_empty();
                (valid && (syntax_only || runtime_function_exists(&name, functions)), name.clone())
            }
        }
        Value::Array(array) => {
            let entries = array.iter().collect::<Vec<_>>();
            if entries.len() != 2 || entries[0].0 != ArrayKey::Integer(0) || entries[1].0 != ArrayKey::Integer(1) {
                return (false, String::new());
            }
            let Value::String(method) = entries[1].1.dereference() else { return (false, String::new()); };
            let method = method.to_ascii_lowercase();
            let class_name = match entries[0].1.dereference() {
                Value::Object(object) => object.borrow().class_name.to_ascii_lowercase(),
                Value::String(class_name) => class_name.trim_start_matches('\\').to_ascii_lowercase(),
                _ => return (false, String::new()),
            };
            let is_object = matches!(entries[0].1.dereference(), Value::Object(_));
            let callable = if syntax_only { true } else {
                let user_method = find_method(&class_name, &method, classes).is_some_and(|function| {
                    function.visibility == MemberVisibility::Public && function.is_static == !is_object
                });
                let magic_method = find_method(&class_name, if is_object { "__call" } else { "__callstatic" }, classes)
                    .is_some_and(|function| function.visibility == MemberVisibility::Public && function.is_static == !is_object);
                let native_exception_method = is_object
                    && super::is_a_type(&class_name, "throwable", classes, &mut std::collections::HashSet::new())
                    && crate::exception_builtins::method_exists("Exception", &method);
                user_method || magic_method || native_exception_method
            };
            (callable, format!("{class_name}::{method}"))
        }
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            let callable = syntax_only || find_method(&class_name, "__invoke", classes).is_some_and(|function| {
                !function.is_static && function.visibility == MemberVisibility::Public
            });
            (callable, format!("{class_name}::__invoke"))
        }
        _ => (false, String::new()),
    }
}

pub(super) fn prepare_reference_arguments(
    arguments: Vec<Value>, reference_names: &[Option<ReferenceArgument>], function: &BytecodeFunction,
    argument_names: &[Option<String>], call_name: &str, caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>, global_variables: &mut GlobalVariables,
    global_scope: bool, frame_id: u64, classes: &HashMap<String, BytecodeClass>, strict_types: bool,
) -> Result<(HashMap<String, Value>, HashMap<String, VariableBinding>), String> {
    let visible_argument_count = visible_argument_count(function, arguments.len(), argument_names);
    let (arguments, reference_names, variadic_is_packed) = normalize_named_arguments(
        arguments, reference_names, argument_names, function, call_name, caller_variables,
        caller_bindings, global_variables, global_scope, frame_id,
    )?;
    let fixed_parameter_count = function.parameter_variadic.iter().position(|variadic| *variadic).unwrap_or(function.parameters.len());
    let extra_arguments = if function.parameter_variadic.iter().any(|variadic| *variadic) {
        Vec::new()
    } else {
        arguments.iter().skip(fixed_parameter_count).cloned().collect::<Vec<_>>()
    };
    let mut arguments = if variadic_is_packed { arguments } else { complete_arguments(arguments, function, call_name)? }
        .into_iter().map(|value| value.dereference()).collect::<Vec<_>>();
    if strict_types {
        validate_strict_arguments(&mut arguments, function, classes)?;
    } else {
        coerce_numeric_arguments(&mut arguments, &reference_names, function);
        validate_arguments(&arguments, function, classes)?;
    }
    let mut locals = function.parameters.iter().cloned()
        .zip(arguments.into_iter().take(function.parameters.len())).collect::<HashMap<_, _>>();
    super::initialize_local_order(&mut locals, function.parameters.iter().cloned());
    locals.insert("\0func_argument_count".into(), Value::Number(visible_argument_count as f64));
    locals.insert("\0func_fixed_parameter_count".into(), Value::Number(
        function.parameter_variadic.iter().position(|variadic| *variadic).unwrap_or(function.parameters.len()) as f64,
    ));
    locals.insert("\0func_parameter_names".into(), Value::from_array_entries(function.parameters.iter().enumerate()
        .map(|(index, name)| (ArrayKey::Integer(index as i64), Value::String(name.clone()))).collect()));
    locals.insert("\0func_extra_arguments".into(), Value::from_array_entries(extra_arguments.into_iter().enumerate()
        .map(|(index, value)| (ArrayKey::Integer(index as i64), value.dereference())).collect()));
    locals.insert("\0func_variadic_parameter".into(), function.parameter_variadic.iter().position(|variadic| *variadic)
        .map(|index| Value::String(function.parameters[index].clone())).unwrap_or(Value::Null));
    let mut parameter_bindings = HashMap::new();
    let variadic_index = function.parameter_variadic.iter().position(|variadic| *variadic);
    for (index, is_reference) in function.parameter_by_reference.iter().enumerate() {
        if !is_reference { continue; }
        // By-reference variadics hold one live reference per packed element. The
        // parameter itself remains an ordinary local array containing those cells.
        if Some(index) == variadic_index { continue; }
        let Some(Some(source)) = reference_names.get(index) else {
            return Err(format!("Argument {} to {call_name} must be a variable passed by reference", index + 1));
        };
        let parameter = function.parameters[index].clone();
        locals.remove(&parameter);
        let binding = if let Some((receiver, property)) = &source.object_property {
            if let Value::Object(object) = receiver {
                if source.array_keys.is_empty() {
                    object.borrow_mut().properties.entry(property.clone()).or_insert(Value::Null);
                    VariableBinding::ObjectProperty { receiver: receiver.clone(), property: property.clone() }
                } else {
                    let mut object = object.borrow_mut();
                    let current = object.properties.get(property).cloned().unwrap_or(Value::Null);
                    let (updated, reference) = current.ensure_array_reference(&source.array_keys)?;
                    object.properties.insert(property.clone(), updated);
                    VariableBinding::Reference(global_variables.store_reference(reference))
                }
            } else {
                return Err("Only object properties can be passed by reference".into());
            }
        } else if let Some(reference) = &source.direct_reference {
            VariableBinding::Reference(global_variables.store_reference(reference.clone()))
        } else {
            if let Some(VariableBinding::ObjectProperty { .. }) = caller_bindings.get(&source.variable) {
                if source.array_keys.is_empty() {
                    caller_bindings.get(&source.variable).cloned().expect("checked property binding")
                } else {
                    return Err("Array-element references through object-property references are not supported".into());
                }
            } else {
                let (key, _) = super::reference_cell_for_path(
                &source.variable, &source.array_keys, caller_variables, caller_bindings,
                global_variables, global_scope, frame_id,
                )?;
                VariableBinding::Reference(key)
            }
        };
        parameter_bindings.insert(parameter, binding);
    }
    Ok((locals, parameter_bindings))
}

pub(super) fn instantiate_pdo_row(
    row: &Value, class_name: &str, constructor_args: &[Value], props_late: bool,
    variables: &mut HashMap<String, Value>, bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables, global_scope: bool, frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>, classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties, output: &mut String, steps: &mut u64,
) -> Result<Value, String> {
    let normalized = class_name.trim_start_matches('\\');
    let object = if normalized.eq_ignore_ascii_case("stdClass") {
        let object = Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
            class_name: "stdClass".into(), properties: HashMap::new(), property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
        })));
        if !props_late { hydrate_pdo_row(&object, row)?; }
        object
    } else {
        let class = classes.get(&normalized.to_ascii_lowercase())
            .ok_or_else(|| format!("Class '{normalized}' not found"))?;
        if class.is_interface { return Err(format!("Cannot instantiate interface {normalized}")); }
        if class.is_abstract { return Err(format!("Cannot instantiate abstract class {normalized}")); }
        let mut properties = HashMap::new();
        super::objects::collect_properties(&class.name, classes, &mut properties)?;
        let object = Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
            class_name: class.name.clone(), properties, property_order: { let mut order = Vec::new(); super::objects::collect_property_order(&class.name, classes, &mut order)?; order }, active_property_hooks: Default::default(), exception: None,
        })));
        global_variables.register_destructor_object(&object);
        hydrate_pdo_row(&object, row)?;
        if let Some(constructor) = find_method(&class.name, "__construct", classes) {
            if constructor.visibility != MemberVisibility::Public {
                return Err(format!("Call to {} {}::__construct() from scope PDO", match constructor.visibility { MemberVisibility::Private => "private", MemberVisibility::Protected => "protected", MemberVisibility::Public => "public" }, class.name));
            }
            let references = vec![None; constructor_args.len()];
            let names = vec![None; constructor_args.len()];
            let (mut locals, parameter_bindings) = prepare_reference_arguments(
                constructor_args.to_vec(), &references, constructor, &names,
                &format!("{}::__construct()", class.name), variables, bindings,
                global_variables, global_scope, frame_id, classes, false,
            )?;
            locals.insert("this".into(), object.clone());
            locals.insert("static_class".into(), Value::String(class.name.clone()));
            execute_code(&constructor.instructions, functions, classes, static_properties,
                &mut locals, global_variables, false, &function_scope(constructor),
                parameter_bindings, output, steps)?;
        } else if !constructor_args.is_empty() {
            return Err(format!("Class '{}' has no constructor", class.name));
        }
        if props_late { hydrate_pdo_row(&object, row)?; }
        object
    };
    Ok(object)
}

fn hydrate_pdo_row(object: &Value, row: &Value) -> Result<(), String> {
    let Value::Object(instance) = object else { return Ok(()); };
    if let Value::Array(fields) = row.dereference() {
        let mut instance = instance.borrow_mut();
        for (key, value) in fields.iter() {
            if let ArrayKey::String(name) = key {
                instance.properties.insert(name.clone(), value.dereference());
            }
        }
    }
    Ok(())
}

/// Apply weak-mode integer/float parameter coercion after named and default arguments are aligned.
fn coerce_numeric_arguments(
    arguments: &mut [Value],
    reference_names: &[Option<ReferenceArgument>],
    function: &BytecodeFunction,
) {
    let variadic_index = function.parameter_variadic.iter().position(|variadic| *variadic);
    for (index, argument) in arguments.iter_mut().enumerate() {
        if function.parameter_by_reference.get(index).copied().unwrap_or(false)
            && reference_names.get(index).is_some_and(Option::is_some)
        {
            continue;
        }
        let Some(type_name) = function.parameter_types.get(index).and_then(Option::as_deref) else { continue; };
        let nullable = type_name.starts_with('?');
        let type_name = type_name.trim_start_matches('?').to_ascii_lowercase();
        if nullable && matches!(argument, Value::Null) { continue; }
        if type_name.contains('|') {
            if Some(index) == variadic_index {
                if let Value::Array(values) = argument {
                    let mut converted = values.clone();
                    for (extra_index, (_, value)) in converted.iter_mut().enumerate() {
                        let reference_index = index + extra_index;
                        if function.parameter_by_reference.get(index).copied().unwrap_or(false)
                            && reference_names.get(reference_index).is_some_and(Option::is_some) { continue; }
                        *value = coerce_numeric_union(value.clone(), &type_name);
                    }
                    *argument = Value::Array(converted);
                }
            } else {
                *argument = coerce_numeric_union(argument.clone(), &type_name);
            }
            continue;
        }
        if Some(index) == variadic_index {
            let Value::Array(values) = argument else { continue; };
            let mut converted = values.clone();
            for (extra_index, (_, value)) in converted.iter_mut().enumerate() {
                let reference_index = index + extra_index;
                if function.parameter_by_reference.get(index).copied().unwrap_or(false)
                    && reference_names.get(reference_index).is_some_and(Option::is_some)
                {
                    continue;
                }
                *value = coerce_numeric_value(value.clone(), &type_name);
            }
            *argument = Value::Array(converted);
        } else {
            *argument = coerce_numeric_value(argument.clone(), &type_name);
        }
    }
}

fn validate_strict_arguments(
    arguments: &mut [Value],
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    let variadic_index = function.parameter_variadic.iter().position(|variadic| *variadic);
    for (index, (argument, expected)) in arguments.iter_mut().zip(&function.parameter_types).enumerate() {
        let Some(expected) = expected.as_deref() else { continue; };
        let expected_normalized = expected.trim_start_matches('?').to_ascii_lowercase();
        if expected_normalized == "float" || expected_normalized == "double" {
            if Some(index) != variadic_index {
                if let Value::Integer(value) = argument.dereference() { *argument = Value::Number(value as f64); }
            }
        }
        let parameter = function.parameters.get(index).map(String::as_str).unwrap_or("?");
        let label = function.declaring_class.as_deref().map_or_else(|| function.name.clone(), |class| format!("{class}::{}", function.name));
        if Some(index) == variadic_index {
            let Value::Array(values) = argument else { continue; };
            for (extra_index, (_, value)) in values.iter_mut().enumerate() {
                if expected_normalized == "float" || expected_normalized == "double" {
                    if let Value::Integer(number) = value.dereference() { *value = Value::Number(number as f64); }
                }
                if !strict_value_matches_type(value, expected, classes) {
                    return Err(format!("TypeError:{label}(): Argument #{} (${parameter}) must be of type {expected}, {} given", index + extra_index + 1, value.dereference().type_name()));
                }
            }
        } else if !strict_value_matches_type(argument, expected, classes) {
            return Err(format!("TypeError:{label}(): Argument #{} (${parameter}) must be of type {expected}, {} given", index + 1, argument.dereference().type_name()));
        }
    }
    Ok(())
}

fn strict_value_matches_type(value: &Value, type_name: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    let value = value.dereference();
    if type_name.contains('|') {
        return type_name.split('|').any(|member| strict_value_matches_type(&value, member.trim_matches(['(', ')']), classes));
    }
    if type_name.contains('&') { return super::value_matches_type(&value, type_name, classes); }
    let nullable = type_name.starts_with('?');
    let expected = type_name.trim_start_matches('?').trim().to_ascii_lowercase();
    if nullable && matches!(value, Value::Null) { return true; }
    match expected.as_str() {
        "int" | "integer" => matches!(value, Value::Integer(_)),
        "float" | "double" => matches!(value, Value::Number(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "true" => matches!(value, Value::Bool(true)),
        "false" => matches!(value, Value::Bool(false)),
        _ => super::value_matches_type(&value, type_name, classes),
    }
}

fn coerce_numeric_value(value: Value, type_name: &str) -> Value {
    match type_name {
        "int" | "integer" => match value {
            Value::Integer(_) => value,
            Value::Number(number) if number.is_finite() && number >= i64::MIN as f64 && number < -(i64::MIN as f64) => Value::Integer(number.trunc() as i64),
            Value::Bool(value) => Value::Integer(i64::from(value)),
            Value::String(text) => text.trim().parse::<i64>().map(Value::Integer).or_else(|_| {
                text.trim().parse::<f64>().ok().filter(|number| number.is_finite() && *number >= i64::MIN as f64 && *number < -(i64::MIN as f64))
                    .map(|number| Value::Integer(number.trunc() as i64)).ok_or(())
            }).unwrap_or(Value::String(text)),
            value => value,
        },
        "float" | "double" => match value {
            Value::Integer(value) => Value::Number(value as f64),
            Value::Number(_) => value,
            Value::Bool(value) => Value::Number(f64::from(value)),
            Value::String(text) => text.trim().parse::<f64>().ok().filter(|number| number.is_finite())
                .map(Value::Number).unwrap_or(Value::String(text)),
            value => value,
        },
        _ => value,
    }
}

fn coerce_numeric_union(value: Value, type_name: &str) -> Value {
    let has = |name: &str| type_name.split('|').any(|member| member == name || (name == "int" && member == "integer") || (name == "float" && member == "double"));
    match value {
        Value::Integer(number) if has("int") => Value::Integer(number),
        Value::Integer(number) if has("float") => Value::Number(number as f64),
        Value::Number(number) if has("float") => Value::Number(number),
        Value::Number(number) if has("int") && number.is_finite() && number.fract() == 0.0 => Value::Integer(number as i64),
        Value::String(text) if has("int") => text.trim().parse::<i64>().map(Value::Integer).unwrap_or_else(|_| {
            if has("float") { text.trim().parse::<f64>().map(Value::Number).unwrap_or(Value::String(text)) } else { Value::String(text) }
        }),
        Value::String(text) if has("float") => text.trim().parse::<f64>().map(Value::Number).unwrap_or(Value::String(text)),
        Value::String(text) if has("string") => Value::String(text),
        Value::Bool(value) if has("bool") || type_name.contains("boolean") => Value::Bool(value),
        Value::Bool(value) if has("int") => Value::Integer(i64::from(value)),
        Value::Bool(value) if has("float") => Value::Number(if value { 1.0 } else { 0.0 }),
        value => value,
    }
}

/// Package a dynamic method name and its arguments for __call/__callStatic.
fn invoke_magic_method(
    receiver: &Value,
    function: &BytecodeFunction,
    missing_method: &str,
    arguments: Vec<Value>,
    argument_names: &[Option<String>],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let mut packed = PhpArray::new();
    for (index, argument) in arguments.into_iter().enumerate() {
        if let Some(Some(name)) = argument_names.get(index) {
            packed.set(ArrayKey::String(name.clone()), argument);
        } else {
            packed.append(argument)?;
        }
    }
    let magic_arguments = vec![Value::String(missing_method.to_string()), Value::Array(packed)];
    validate_arguments(&magic_arguments, function, classes)?;
    let mut locals = HashMap::new();
    if !matches!(receiver, Value::Null) {
        locals.insert("this".to_string(), receiver.clone());
    }
    let class_name = function.declaring_class.clone().unwrap_or_default();
    let static_class = match receiver {
        Value::Object(object) => object.borrow().class_name.clone(),
        _ => class_name,
    };
    locals.insert("static_class".to_string(), Value::String(static_class.clone()));
    for (parameter, argument) in function.parameters.iter().cloned().zip(magic_arguments) { locals.insert(parameter, argument); }
    let result = execute_code(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(function), HashMap::new(), output, steps)?.unwrap_or(Value::Null);
    let result = super::validate_return_type_in_scope(result, function, classes, Some(&static_class))?;
    Ok(result)
}

/// Call a user-defined function or dispatch a supported callback builtin.
fn callable_parameter_references(
    callable: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
) -> Vec<bool> {
    let function = match callable {
        Value::Closure(closure) => {
            let closure = closure.borrow();
            if let Some(target) = &closure.callable_target {
                return callable_parameter_references(target, functions, classes);
            }
            Some(closure.function.clone())
        }
        Value::String(name) => functions.get(&name.to_ascii_lowercase()).cloned(),
        Value::Array(array) => {
            let entries = array.iter().collect::<Vec<_>>();
            if entries.len() != 2 { None } else {
                let method = match entries[1].1.dereference() {
                    Value::String(method) => method,
                    _ => return Vec::new(),
                };
                let class_name = match entries[0].1.dereference() {
                    Value::Object(object) => object.borrow().class_name.clone(),
                    Value::String(class) => class,
                    _ => return Vec::new(),
                };
                find_method(&class_name, &method, classes).cloned()
            }
        }
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            find_method(&class_name, "__invoke", classes).cloned()
        }
        _ => None,
    };
    function.map(|function| function.parameter_by_reference).unwrap_or_default()
}

fn invoke_array_callback(
    callback: &Value,
    arguments: Vec<Value>,
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    caller_global_scope: bool,
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let references = callable_parameter_references(callback, functions, classes);
    if !references.iter().any(|by_reference| *by_reference) {
        return match callback {
            Value::String(name) => invoke_callable(name, arguments, functions, classes, static_properties, global_variables, output, steps),
            Value::Closure(closure) => invoke_closure(closure, arguments, &[], &[], caller_variables, caller_bindings, caller_global_scope, frame_id, functions, classes, static_properties, global_variables, output, steps, false),
            Value::Array(_) => invoke_array_callable(callback, arguments, &[], &[], caller_variables, caller_bindings, caller_global_scope, frame_id, functions, classes, static_properties, global_variables, output, steps, false),
            Value::Object(_) => invoke_object_callable(callback, arguments, &[], &[], caller_variables, caller_bindings, caller_global_scope, frame_id, functions, classes, static_properties, global_variables, output, steps, false),
            _ => Err("Value is not callable".into()),
        };
    }
    // PHP warns for these callback APIs and binds by-reference parameters to
    // temporary callback arguments, leaving the source array unchanged.
    let values = arguments.into_iter().map(|value| value.dereference()).collect::<Vec<_>>();
    let mut local_variables = HashMap::new();
    let mut local_bindings = HashMap::new();
    let reference_arguments = values.iter().enumerate().map(|(index, value)| {
        let name = format!("\0callback_argument:{frame_id}:{index}");
        local_variables.insert(name.clone(), value.clone());
        references.get(index).copied().unwrap_or(false).then_some(ReferenceArgument {
            variable: name, array_keys: Vec::new(), object_property: None, direct_reference: None,
        })
    }).collect::<Vec<_>>();
    let names = vec![None; values.len()];
    match callback {
        Value::String(name) if functions.contains_key(&name.to_ascii_lowercase()) => invoke_with_references(
            name, values, &reference_arguments, &names, false, &mut local_variables, &mut local_bindings,
            global_variables, false, frame_id, functions, classes, static_properties, output, steps,
        ),
        Value::Closure(closure) => invoke_closure(
            closure, values, &reference_arguments, &names, &mut local_variables, &mut local_bindings,
            false, frame_id, functions, classes, static_properties, global_variables, output, steps, false,
        ),
        Value::Array(_) => invoke_array_callable(
            callback, values, &reference_arguments, &names, &mut local_variables, &mut local_bindings,
            false, frame_id, functions, classes, static_properties, global_variables, output, steps, false,
        ),
        Value::Object(_) => invoke_object_callable(
            callback, values, &reference_arguments, &names, &mut local_variables, &mut local_bindings,
            false, frame_id, functions, classes, static_properties, global_variables, output, steps, false,
        ),
        Value::String(name) => invoke_callable(name, values, functions, classes, static_properties, global_variables, output, steps),
        _ => Err("Value is not callable".into()),
    }
}

fn strtok_with_state(arguments: &[Value], global_variables: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!("strtok() expects 1 or 2 arguments, {} given", arguments.len()));
    }
    let (input, offset) = if arguments.len() == 2 {
        let input = arguments[0].show();
        global_variables.set("\0strtok_input".into(), Value::String(input.clone()));
        global_variables.set("\0strtok_offset".into(), Value::Integer(0));
        (input, 0usize)
    } else {
        let Some(Value::String(input)) = global_variables.get("\0strtok_input") else {
            return Ok(Value::Bool(false));
        };
        let input = input.clone();
        let offset = global_variables.get("\0strtok_offset").map(|value| value.number() as usize).unwrap_or(input.len());
        (input, offset)
    };
    let delimiters = arguments.last().expect("strtok requires a delimiter").show().into_bytes();
    let bytes = input.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor < bytes.len() && delimiters.contains(&bytes[cursor]) { cursor += 1; }
    if cursor == bytes.len() {
        global_variables.set("\0strtok_offset".into(), Value::Integer(bytes.len() as i64));
        return Ok(Value::Bool(false));
    }
    let start = cursor;
    while cursor < bytes.len() && !delimiters.contains(&bytes[cursor]) { cursor += 1; }
    global_variables.set("\0strtok_offset".into(), Value::Integer(cursor as i64));
    Ok(Value::String(String::from_utf8_lossy(&bytes[start..cursor]).into_owned()))
}

fn hex2bin_with_diagnostic(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    let result = crate::string_builtins::call("hex2bin", arguments)?;
    if matches!(result, Value::Bool(false)) {
        let input = arguments.first().map(Value::show).unwrap_or_default();
        let message = if input.len() % 2 != 0 {
            "hex2bin(): Hexadecimal input string must have an even length"
        } else {
            "hex2bin(): Input string must be hexadecimal string"
        };
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: message.into(), source_file: None, source_line: None,
        });
    }
    Ok(result)
}

fn chr_with_diagnostic(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    let codepoint = arguments.first().map(Value::number).unwrap_or(0.0) as i64;
    if !(0..=255).contains(&codepoint) {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Deprecated,
            message: "chr(): Providing a value not in-between 0 and 255 is deprecated, this is because a byte value must be in the [0, 255] interval. The value used will be constrained using % 256".into(),
            source_file: None, source_line: None,
        });
    }
    crate::string_builtins::call("chr", arguments)
}

pub(super) fn invoke_callable(
    name: &str,
    arguments: Vec<Value>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let normalized_name = name.to_ascii_lowercase();
    if let Some(function) = functions.get(&normalized_name) {
        if function.parameter_by_reference.iter().any(|by_reference| *by_reference) {
            return Err(format!("{name}() requires a variable for its by-reference parameter"));
        }
        let arguments = complete_arguments(arguments, function, &format!("{name}()"))?;
        validate_arguments(&arguments, function, classes)?;
        let local_variables = function.parameters.iter().cloned()
            .zip(arguments.into_iter().take(function.parameters.len())).collect::<HashMap<_, _>>();
        if super::is_generator_function(function) {
            return super::new_generator(function.clone(), local_variables, HashMap::new(), *steps, classes);
        }
        let mut local_variables = local_variables;
        let result = execute_code(&function.instructions, functions, classes, static_properties, &mut local_variables, global_variables, false, &function_scope(function), HashMap::new(), output, steps)?
            .unwrap_or(Value::Null);
        let result = validate_return_type(result, function, classes)?;
        return Ok(result);
    }
    if name.eq_ignore_ascii_case("strval") {
        let value = arguments.first().ok_or("strval() expects exactly 1 argument")?.dereference();
        return super::objects::stringify(&value, functions, classes, static_properties, global_variables, output, steps)
            .map(Value::String);
    }
    if name.eq_ignore_ascii_case("is_countable") {
        return type_builtin("is_countable", &arguments, classes);
    }
    if name.eq_ignore_ascii_case("is_iterable") {
        return type_builtin("is_iterable", &arguments, classes);
    }
    if name.eq_ignore_ascii_case("gettimeofday") {
        if arguments.len() > 1 { return Err(format!("gettimeofday() expects at most 1 argument, {} given", arguments.len())); }
        return gettimeofday_value(arguments.first().is_some_and(Value::truthy));
    }
    if name.eq_ignore_ascii_case("strtok") {
        let mut arguments = arguments;
        coerce_stringable_builtin_args("strtok", &mut arguments, crate::string_builtins::string_argument_positions("strtok"), functions, classes, static_properties, global_variables, output, steps)?;
        return strtok_with_state(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("hex2bin") {
        return hex2bin_with_diagnostic(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("chr") {
        return chr_with_diagnostic(&arguments, global_variables);
    }
    crate::array_builtins::call_callback_builtin(name, arguments)
}

/// Invoke a two-item PHP callable array containing an object or class name and method name.
fn invoke_array_callable(
    callable: &Value,
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    caller_global_scope: bool,
    caller_frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
    strict_types: bool,
) -> Result<Value, String> {
    let Value::Array(items) = callable else { return Err("Value is not callable".into()); };
    if items.len() != 2
        || items.get(0).map_or(true, |(key, _)| key != &ArrayKey::Integer(0))
        || items.get(1).map_or(true, |(key, _)| key != &ArrayKey::Integer(1))
    {
        return Err("Array callback must contain exactly two numeric entries".into());
    }
    let receiver = items[0].1.clone();
    let method = match &items[1].1 {
        Value::String(method) => method.to_ascii_lowercase(),
        _ => return Err("Array callback method name must be a string".into()),
    };
    let (class_name, function, receiver_object) = match receiver {
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            let function = find_method(&class_name, &method, classes)
                .ok_or_else(|| format!("Call to undefined method {class_name}::{method}()"))?;
            ensure_access(function.visibility, function.declaring_class.as_deref(), None, classes, &method)?;
            if function.is_static { return Err(format!("Non-static callback target {class_name}::{method}() is static")); }
            (class_name, function, Some(Value::Object(object)))
        }
        Value::String(class_name) => {
            let class_name = class_name.to_ascii_lowercase();
            let function = find_method(&class_name, &method, classes)
                .ok_or_else(|| format!("Call to undefined method {class_name}::{method}()"))?;
            ensure_access(function.visibility, function.declaring_class.as_deref(), None, classes, &method)?;
            if !function.is_static { return Err(format!("Non-static method {class_name}::{method}() cannot be called statically")); }
            (class_name, function, None)
        }
        _ => return Err("Array callback must start with an object or class name".into()),
    };
    let (mut locals, parameter_bindings) = prepare_reference_arguments(
        arguments, reference_names, function, &argument_names, &format!("{class_name}::{method}()"),
        caller_variables, caller_bindings, global_variables, caller_global_scope, caller_frame_id, classes, strict_types,
    )?;
    if let Some(receiver) = receiver_object { locals.insert("this".into(), receiver); }
    locals.insert("static_class".into(), Value::String(class_name.clone()));
    if super::is_generator_function(function) {
        return super::new_generator(function.clone(), locals, parameter_bindings, *steps, classes);
    }
    let result = execute_code(
        &function.instructions, functions, classes, static_properties, &mut locals,
        global_variables, false, &function_scope(function), parameter_bindings, output, steps,
    )?.unwrap_or(Value::Null);
    let result = super::validate_return_type_in_scope(result, function, classes, Some(&class_name))?;
    Ok(result)
}

fn invoke_object_callable(
    callable: &Value,
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    caller_global_scope: bool,
    caller_frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
    strict_types: bool,
) -> Result<Value, String> {
    if !matches!(callable.dereference(), Value::Object(_)) { return Err("Value is not callable".into()); }
    let target = Value::from_array_entries(vec![
        (ArrayKey::Integer(0), callable.dereference()),
        (ArrayKey::Integer(1), Value::String("__invoke".into())),
    ]);
    invoke_array_callable(
        &target, arguments, reference_names, argument_names, caller_variables, caller_bindings,
        caller_global_scope, caller_frame_id, functions, classes, static_properties, global_variables, output, steps, strict_types,
    )
}

/// Invoke a named function and copy modified by-reference parameters back to caller variables.
fn invoke_with_references(
    name: &str,
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
    strict_types: bool,
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    caller_global_scope: bool,
    caller_frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let normalized_name = name.to_ascii_lowercase();
    let function = functions.get(&normalized_name).ok_or_else(|| format!("Call to undefined function {name}()"))?;
    let (mut locals, parameter_bindings) = prepare_reference_arguments(
        arguments, reference_names, function, argument_names, &format!("{name}()"),
        caller_variables, caller_bindings, global_variables, caller_global_scope, caller_frame_id, classes, strict_types,
    )?;
    if super::is_generator_function(function) {
        return super::new_generator(function.clone(), locals, parameter_bindings, *steps, classes);
    }
    let result = execute_code(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(function), parameter_bindings, output, steps)?
        .unwrap_or(Value::Null);
    let result = validate_return_type(result, function, classes)?;
    Ok(result)
}

/// Execute a closure in a fresh local frame initialized from its captured values.
fn invoke_closure(
    closure: &Rc<std::cell::RefCell<PhpClosure>>,
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    caller_global_scope: bool,
    caller_frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
    strict_types: bool,
) -> Result<Value, String> {
    let (function, captures, capture_order, reference_captures, callable_target) = {
        let closure = closure.borrow();
        (closure.function.clone(), closure.captures.clone(), closure.capture_order.clone(), closure.reference_captures.clone(), closure.callable_target.clone())
    };
    if let Some(target) = callable_target {
        return match target {
            Value::Array(_) => invoke_array_callable(
                &target, arguments, reference_names, argument_names, caller_variables, caller_bindings,
                caller_global_scope, caller_frame_id, functions, classes, static_properties,
                global_variables, output, steps, strict_types,
            ),
            Value::String(name) => {
                let packed_arguments = pack_callable_arguments(
                    arguments, reference_names, argument_names, caller_variables, caller_bindings,
                    global_variables, caller_global_scope, caller_frame_id,
                )?;
                let (instruction, mut callback_stack) = callback_call(Value::String(name), packed_arguments, strict_types)?;
                execute_instruction(
                    &instruction, &mut callback_stack, caller_variables, caller_bindings, global_variables,
                    caller_global_scope, caller_frame_id, functions, classes, static_properties, output, steps,
                )?;
                callback_stack.pop().ok_or_else(|| "Invalid first-class callable result stack".into())
            }
            _ => Err("Invalid first-class callable target".into()),
        };
    }
    let function = &function;
    let (mut parameter_locals, parameter_bindings) = prepare_reference_arguments(
        arguments, reference_names, function, argument_names, "Closure()", caller_variables, caller_bindings,
        global_variables, caller_global_scope, caller_frame_id, classes, strict_types,
    )?;
    let mut locals = captures.clone();
    locals.extend(parameter_locals.drain());
    super::initialize_local_order(&mut locals, capture_order.into_iter().chain(function.parameters.iter().cloned()));
    let mut frame_bindings = reference_captures.into_iter()
        .map(|(name, key)| (name, VariableBinding::Reference(key)))
        .collect::<HashMap<_, _>>();
    frame_bindings.extend(parameter_bindings);
    if super::is_generator_function(function) {
        return super::new_generator(function.clone(), locals, frame_bindings, *steps, classes);
    }
    let closure_scope = format!("<closure:{:p}>", Rc::as_ptr(closure));
    let result = execute_code(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &closure_scope, frame_bindings, output, steps)?
        .unwrap_or(Value::Null);
    let result = validate_return_type(result, function, classes)?;
    let mut closure = closure.borrow_mut();
    for name in closure.captures.keys().cloned().collect::<Vec<_>>() {
        if let Some(value) = locals.get(&name) { closure.captures.insert(name, value.clone()); }
    }
    Ok(result)
}

fn pack_callable_arguments(
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<Value, String> {
    let mut next_index = 0i64;
    let mut entries = Vec::with_capacity(arguments.len());
    for (index, value) in arguments.into_iter().enumerate() {
        let key = if let Some(name) = argument_names.get(index).cloned().flatten() {
            ArrayKey::String(name)
        } else {
            let key = ArrayKey::Integer(next_index);
            next_index += 1;
            key
        };
        let value = match reference_names.get(index).and_then(Option::as_ref) {
            Some(reference) => Value::Reference(reference_cell_for_argument(
                reference, variables, bindings, globals, global_scope, frame_id,
            )?),
            None => value.dereference(),
        };
        entries.push((key, value));
    }
    Ok(Value::from_array_entries(entries))
}

#[cfg(test)]
mod builtin_inventory_tests {
    #[test]
    fn function_exists_reports_every_string_dispatch_builtin() {
        let functions = std::collections::HashMap::new();
        for name in crate::string_builtins::supported_names() {
            assert!(super::runtime_function_exists(name, &functions), "function_exists({name:?}) returned false");
        }
    }
}

