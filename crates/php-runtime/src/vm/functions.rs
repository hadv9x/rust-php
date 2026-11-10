//! Function, instance-method, static-method, and magic-call dispatch.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use super::objects::{
    ensure_access, find_method, find_property_in_scope, instance_property_storage_key,
};
use super::{
    complete_arguments, execute_code, function_scope, read_variable, reference_storage_key,
    validate_arguments, validate_return_type, write_array_root, write_variable, GlobalVariables,
    StaticProperties, VariableBinding,
};
use crate::{
    bytecode::{Op, ReferenceArgument, ReferenceDescriptor},
    ArrayKey, BytecodeClass, BytecodeFunction, MemberVisibility, PhpArray, PhpClosure, Value,
};

fn enum_scalar_equal(left: &Value, right: &Value) -> bool {
    match (left.dereference(), right.dereference()) {
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        _ => false,
    }
}

fn reflection_string_argument(
    function: &str,
    value: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<String, String> {
    match value.dereference() {
        Value::String(_) | Value::Integer(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
            Ok(value.show())
        }
        Value::Object(_) => super::objects::stringify(
            value,
            functions,
            classes,
            static_properties,
            globals,
            output,
            steps,
        )
        .map_err(|error| format!("{function}(): {error}")),
        _ => Err(format!(
            "TypeError:{function}(): Argument #1 ($name) must be of type string, {} given",
            value.type_name()
        )),
    }
}

pub(super) fn reflection_attribute_arguments(arguments: &[(Option<String>, Value)]) -> Value {
    let mut positional = 0i64;
    Value::from_array_entries(
        arguments
            .iter()
            .map(|(name, value)| {
                let key = match name {
                    Some(name) => ArrayKey::String(name.clone()),
                    None => {
                        let key = ArrayKey::Integer(positional);
                        positional += 1;
                        key
                    }
                };
                (key, value.clone())
            })
            .collect(),
    )
}

fn reflection_constants(
    target: &str,
    classes: &HashMap<String, BytecodeClass>,
    filter: Option<i64>,
) -> Value {
    fn collect(
        name: &str,
        classes: &HashMap<String, BytecodeClass>,
        filter: Option<i64>,
        visited: &mut HashSet<String>,
        added: &mut HashSet<String>,
        entries: &mut Vec<(ArrayKey, Value)>,
    ) {
        let key = name.trim_start_matches('\\').to_ascii_lowercase();
        if !visited.insert(key.clone()) {
            return;
        }
        let Some(class) = classes.get(&key) else {
            return;
        };
        let mut order = class.constant_order.clone();
        let mut remaining = class
            .constants
            .keys()
            .filter(|constant| !order.contains(constant))
            .cloned()
            .collect::<Vec<_>>();
        remaining.sort();
        order.extend(remaining);
        for constant_name in order {
            let Some(constant) = class.constants.get(&constant_name) else {
                continue;
            };
            let visibility = match constant.visibility {
                MemberVisibility::Public => 1,
                MemberVisibility::Protected => 2,
                MemberVisibility::Private => 4,
            };
            if filter.is_some_and(|filter| filter & visibility == 0)
                || !added.insert(constant_name.clone())
            {
                continue;
            }
            entries.push((ArrayKey::String(constant_name), constant.value.clone()));
        }
        if let Some(parent) = &class.parent {
            collect(parent, classes, filter, visited, added, entries);
        }
        for interface in &class.interfaces {
            collect(interface, classes, filter, visited, added, entries);
        }
    }
    let mut entries = Vec::new();
    collect(
        target,
        classes,
        filter,
        &mut HashSet::new(),
        &mut HashSet::new(),
        &mut entries,
    );
    Value::from_array_entries(entries)
}

fn split_top_level_type(type_name: &str, separator: char) -> Vec<&str> {
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut parts = Vec::new();
    for (index, character) in type_name.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            value if value == separator && depth == 0 => {
                parts.push(type_name[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if !parts.is_empty() {
        parts.push(type_name[start..].trim());
    }
    parts
}

fn strip_type_parentheses(mut type_name: &str) -> &str {
    loop {
        if !type_name.starts_with('(') || !type_name.ends_with(')') {
            return type_name;
        }
        let mut depth = 0usize;
        let mut wraps_all = true;
        for (index, character) in type_name.char_indices() {
            if character == '(' {
                depth += 1;
            }
            if character == ')' {
                depth = depth.saturating_sub(1);
                if depth == 0 && index + 1 != type_name.len() {
                    wraps_all = false;
                    break;
                }
            }
        }
        if !wraps_all {
            return type_name;
        }
        type_name = &type_name[1..type_name.len() - 1];
    }
}

fn reflection_type_allows_null(type_name: &str) -> bool {
    let type_name = type_name.trim();
    if type_name.starts_with('?') {
        return true;
    }
    let type_name = strip_type_parentheses(type_name);
    let members = split_top_level_type(type_name, '|');
    if !members.is_empty() {
        return members.iter().any(|member| {
            member.eq_ignore_ascii_case("null") || reflection_type_allows_null(member)
        });
    }
    type_name.eq_ignore_ascii_case("null")
}

fn reflection_named_type(name: &str, allows_null: bool) -> Value {
    let normalized = name.to_ascii_lowercase();
    let builtin = matches!(
        normalized.as_str(),
        "int"
            | "float"
            | "string"
            | "bool"
            | "array"
            | "object"
            | "callable"
            | "iterable"
            | "void"
            | "never"
            | "mixed"
            | "null"
            | "false"
            | "true"
    );
    let allows_null = allows_null || matches!(normalized.as_str(), "mixed" | "null");
    Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
        class_name: "ReflectionNamedType".into(),
        properties: HashMap::from([
            ("\0type_name".into(), Value::String(name.to_string())),
            ("\0is_builtin".into(), Value::Bool(builtin)),
            ("\0allows_null".into(), Value::Bool(allows_null)),
        ]),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })))
}

fn reflection_type(type_name: &str) -> Value {
    let allows_null = reflection_type_allows_null(type_name);
    let mut normalized = type_name.trim();
    if let Some(nullable) = normalized.strip_prefix('?') {
        normalized = nullable;
    }
    normalized = strip_type_parentheses(normalized);
    let parts = split_top_level_type(normalized, '|');
    let (parts, separator) = if parts.is_empty() {
        let intersection = split_top_level_type(normalized, '&');
        if intersection.is_empty() {
            return reflection_named_type(normalized, allows_null);
        }
        (intersection, '&')
    } else {
        (parts, '|')
    };
    let types = parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            (
                ArrayKey::Integer(index as i64),
                reflection_type(strip_type_parentheses(part)),
            )
        })
        .collect();
    Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
        class_name: if separator == '|' {
            "ReflectionUnionType"
        } else {
            "ReflectionIntersectionType"
        }
        .into(),
        properties: HashMap::from([
            ("\0types".into(), Value::from_array_entries(types)),
            ("\0allows_null".into(), Value::Bool(allows_null)),
        ]),
        property_order: Vec::new(),
        active_property_hooks: Default::default(),
        exception: None,
    })))
}

fn reflection_method_modifiers(function: &BytecodeFunction) -> i64 {
    let visibility = match function.visibility {
        MemberVisibility::Public => 1,
        MemberVisibility::Protected => 2,
        MemberVisibility::Private => 4,
    };
    visibility
        | if function.is_static { 16 } else { 0 }
        | if function.is_final { 32 } else { 0 }
        | if function.is_abstract { 64 } else { 0 }
}

fn reflection_optional_int_argument(
    arguments: &[Value],
    class_name: &str,
    method: &str,
) -> Result<Option<i64>, String> {
    let Some(value) = arguments.first() else {
        return Ok(None);
    };
    match value.dereference() {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(value)),
        Value::Number(value) if value.is_finite() => Ok(Some(value.trunc() as i64)),
        Value::Bool(value) => Ok(Some(i64::from(value))),
        Value::String(value) => {
            let value = value.trim();
            value.parse::<i64>().map(Some).or_else(|_| value.parse::<f64>().ok()
                .filter(|value| value.is_finite())
                .map(|value| Some(value.trunc() as i64))
                .ok_or(()))
                .map_err(|_| format!("TypeError:{class_name}::{method}(): Argument #1 ($filter) must be of type ?int, string given"))
        }
        Value::Object(object) => Err(format!("TypeError:{class_name}::{method}(): Argument #1 ($filter) must be of type ?int, {} given", object.borrow().class_name)),
        Value::Closure(_) => Err(format!("TypeError:{class_name}::{method}(): Argument #1 ($filter) must be of type ?int, Closure given")),
        Value::Generator(_) => Err(format!("TypeError:{class_name}::{method}(): Argument #1 ($filter) must be of type ?int, Generator given")),
        value => Err(format!("TypeError:{class_name}::{method}(): Argument #1 ($filter) must be of type ?int, {} given", value.type_name())),
    }
}

pub(super) fn required_parameter_count(function: &BytecodeFunction) -> usize {
    function
        .parameters
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            function
                .parameter_defaults
                .get(*index)
                .is_none_or(Option::is_none)
                && !function
                    .parameter_variadic
                    .get(*index)
                    .copied()
                    .unwrap_or(false)
        })
        .map(|(index, _)| index + 1)
        .max()
        .unwrap_or(0)
}

fn reflection_interface_names(
    target: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> Vec<String> {
    fn collect(
        name: &str,
        classes: &HashMap<String, BytecodeClass>,
        names: &mut Vec<String>,
        seen: &mut HashSet<String>,
    ) {
        let Some(class) = classes.get(&name.to_ascii_lowercase()) else {
            return;
        };
        for interface in &class.interfaces {
            let key = interface.to_ascii_lowercase();
            if seen.insert(key.clone()) {
                names.push(
                    classes
                        .get(&key)
                        .map_or_else(|| interface.clone(), |decl| decl.name.clone()),
                );
                collect(interface, classes, names, seen);
            }
        }
    }
    let mut chain = Vec::new();
    let mut current = Some(target);
    while let Some(name) = current {
        let Some(class) = classes.get(&name.to_ascii_lowercase()) else {
            break;
        };
        chain.push(class.name.clone());
        current = class.parent.as_deref();
    }
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for name in chain.iter().rev() {
        collect(name, classes, &mut names, &mut seen);
    }
    names
}

fn reflection_method_prototype(
    target: &str,
    method: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> Option<(String, String)> {
    fn search(
        class_name: &str,
        method: &str,
        classes: &HashMap<String, BytecodeClass>,
        visited: &mut HashSet<String>,
    ) -> Option<(String, String)> {
        let key = class_name.to_ascii_lowercase();
        if !visited.insert(key.clone()) {
            return None;
        }
        let class = classes.get(&key)?;
        if class.methods.contains_key(method) {
            let display = class
                .method_display_names
                .get(method)
                .cloned()
                .unwrap_or_else(|| method.to_string());
            return Some((class.name.clone(), display));
        }
        if let Some(parent) = &class.parent {
            if let Some(prototype) = search(parent, method, classes, visited) {
                return Some(prototype);
            }
        }
        for interface in &class.interfaces {
            if let Some(prototype) = search(interface, method, classes, visited) {
                return Some(prototype);
            }
        }
        None
    }

    let target_key = target.to_ascii_lowercase();
    let class = classes.get(&target_key)?;
    let method = method.to_ascii_lowercase();
    if let Some(parent) = &class.parent {
        if let Some(prototype) = search(parent, &method, classes, &mut HashSet::new()) {
            return Some(prototype);
        }
    }
    for interface in &class.interfaces {
        if let Some(prototype) = search(interface, &method, classes, &mut HashSet::new()) {
            return Some(prototype);
        }
    }
    for (name, candidate) in classes {
        if candidate.is_interface
            && candidate.methods.contains_key(&method)
            && super::is_a_type(&target_key, name, classes, &mut HashSet::new())
        {
            let display = candidate
                .method_display_names
                .get(&method)
                .cloned()
                .unwrap_or_else(|| method.clone());
            return Some((candidate.name.clone(), display));
        }
    }
    None
}

fn reflection_trait_names(target: &str, classes: &HashMap<String, BytecodeClass>) -> Vec<String> {
    classes
        .get(&target.to_ascii_lowercase())
        .map(|class| class.traits.clone())
        .unwrap_or_default()
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
    let class_name = properties
        .get("\0attribute_name")
        .ok_or("Invalid ReflectionAttribute")?
        .show();
    let declaration = classes
        .get(&class_name.to_ascii_lowercase())
        .ok_or_else(|| format!("Attribute class {class_name} does not exist"))?;
    let marker = declaration
        .attributes
        .iter()
        .find(|attribute| attribute.name.eq_ignore_ascii_case("Attribute"))
        .ok_or_else(|| {
            format!("Attempting to use non-attribute class {class_name} as an attribute")
        })?;
    let flags = marker
        .arguments
        .iter()
        .find(|(name, _)| name.as_deref() == Some("flags"))
        .or_else(|| marker.arguments.iter().find(|(name, _)| name.is_none()))
        .map_or(127, |(_, value)| value.number() as i64);
    let target_type = properties
        .get("\0target_type")
        .map_or(1, |value| value.number() as i64);
    if flags & target_type == 0 {
        return Err(format!(
            "Attribute class {class_name} cannot target this declaration"
        ));
    }
    let target_class = properties
        .get("\0target_class")
        .ok_or("Invalid ReflectionAttribute target")?
        .show();
    let repeated = properties.get("\0is_repeated").is_some_and(Value::truthy)
        || if target_type == 2 {
            functions
                .get(&target_class.to_ascii_lowercase())
                .is_some_and(|target| {
                    target
                        .attributes
                        .iter()
                        .filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name))
                        .count()
                        > 1
                })
        } else if target_type == 64 {
            global_variables
                .constant_attributes
                .get(&target_class)
                .is_some_and(|attributes| {
                    attributes
                        .iter()
                        .filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name))
                        .count()
                        > 1
                })
        } else {
            classes
                .get(&target_class.to_ascii_lowercase())
                .is_some_and(|target| {
                    if target_type == 4 {
                        let method = properties
                            .get("\0target_member")
                            .map(Value::show)
                            .unwrap_or_default();
                        target
                            .method_attributes
                            .get(&method.to_ascii_lowercase())
                            .is_some_and(|attributes| {
                                attributes
                                    .iter()
                                    .filter(|attribute| {
                                        attribute.name.eq_ignore_ascii_case(&class_name)
                                    })
                                    .count()
                                    > 1
                            })
                    } else if target_type == 8 {
                        let property = properties
                            .get("\0target_member")
                            .map(Value::show)
                            .unwrap_or_default();
                        target
                            .property_attributes
                            .get(&property)
                            .is_some_and(|attributes| {
                                attributes
                                    .iter()
                                    .filter(|attribute| {
                                        attribute.name.eq_ignore_ascii_case(&class_name)
                                    })
                                    .count()
                                    > 1
                            })
                    } else if target_type == 16 {
                        let constant = properties
                            .get("\0target_member")
                            .map(Value::show)
                            .unwrap_or_default();
                        target.constants.get(&constant).is_some_and(|constant| {
                            constant
                                .attributes
                                .iter()
                                .filter(|attribute| {
                                    attribute.name.eq_ignore_ascii_case(&class_name)
                                })
                                .count()
                                > 1
                        })
                    } else if target_type == 32 {
                        let method = properties
                            .get("\0target_member")
                            .map(Value::show)
                            .unwrap_or_default();
                        let position = properties
                            .get("\0target_position")
                            .map_or(usize::MAX, |value| value.number() as usize);
                        target
                            .parameter_attributes
                            .get(&method.to_ascii_lowercase())
                            .and_then(|items| items.get(position))
                            .is_some_and(|attributes| {
                                attributes
                                    .iter()
                                    .filter(|attribute| {
                                        attribute.name.eq_ignore_ascii_case(&class_name)
                                    })
                                    .count()
                                    > 1
                            })
                    } else {
                        target
                            .attributes
                            .iter()
                            .filter(|attribute| attribute.name.eq_ignore_ascii_case(&class_name))
                            .count()
                            > 1
                    }
                })
        };
    if repeated && flags & 128 == 0 {
        return Err(format!("Attribute {class_name} must not be repeated"));
    }
    let argument_array = properties
        .get("\0attribute_arguments")
        .ok_or("Invalid ReflectionAttribute arguments")?;
    let Value::Array(entries) = argument_array.dereference() else {
        return Err("Invalid ReflectionAttribute arguments".into());
    };
    let mut arguments = Vec::with_capacity(entries.len());
    let mut argument_names = Vec::with_capacity(entries.len());
    for (key, value) in entries.iter() {
        argument_names.push(match key {
            ArrayKey::Integer(_) => None,
            ArrayKey::String(name) => Some(name.clone()),
        });
        arguments.push(value.clone());
    }
    let mut stack = arguments;
    let instruction = Op::NewObject(
        class_name,
        stack.len(),
        vec![None; stack.len()],
        argument_names,
        vec![false; stack.len()],
        None,
        Some(1),
        false,
    );
    let mut program_counter = 0;
    super::objects::execute_instruction(
        &instruction,
        &mut stack,
        variables,
        bindings,
        global_variables,
        global_scope,
        frame_id,
        "<main>",
        functions,
        classes,
        static_properties,
        output,
        steps,
        &mut program_counter,
    )?;
    stack
        .pop()
        .ok_or_else(|| "Invalid ReflectionAttribute construction result".into())
}

pub(super) fn take_call_arguments(
    stack: &mut Vec<Value>,
    descriptors: &[Option<ReferenceDescriptor>],
    argument_names: &[Option<String>],
    unpack: &[bool],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<
    (
        Vec<Value>,
        Vec<Option<ReferenceArgument>>,
        Vec<Option<String>>,
    ),
    String,
> {
    let width = descriptors
        .iter()
        .map(|descriptor| {
            descriptor
                .as_ref()
                .map_or(1, |target| 1 + target.array_key_count)
        })
        .sum::<usize>();
    if stack.len() < width {
        return Err("Invalid bytecode stack".into());
    }
    let operands = stack.split_off(stack.len() - width);
    let mut operands = operands.into_iter();
    let mut arguments = Vec::with_capacity(descriptors.len());
    let mut references = Vec::with_capacity(descriptors.len());
    let mut names = Vec::with_capacity(descriptors.len());
    for (index, descriptor) in descriptors.iter().enumerate() {
        if let Some(descriptor) = descriptor {
            let root_or_value = operands.next().ok_or("Invalid bytecode stack")?;
            if descriptor.direct_reference {
                let Value::Reference(reference) = root_or_value else {
                    return Err("Argument must be passed by reference".into());
                };
                let keys = (0..descriptor.array_key_count)
                    .map(|_| operands.next().ok_or("Invalid bytecode stack")?.array_key())
                    .collect::<Result<Vec<_>, _>>()?;
                let direct_reference = if keys.is_empty() {
                    reference
                } else {
                    let (updated, element_reference) =
                        reference.borrow().clone().ensure_array_reference(&keys)?;
                    *reference.borrow_mut() = updated;
                    element_reference
                };
                arguments.push(direct_reference.borrow().dereference());
                references.push(Some(ReferenceArgument {
                    variable: String::new(),
                    array_keys: Vec::new(),
                    object_property: None,
                    direct_reference: Some(direct_reference),
                }));
                names.push(argument_names.get(index).cloned().flatten());
                continue;
            }
            let keys = (0..descriptor.array_key_count)
                .map(|_| operands.next().ok_or("Invalid bytecode stack")?.array_key())
                .collect::<Result<Vec<_>, _>>()?;
            let (value, object_property) = if let Some(property) = &descriptor.object_property {
                let Value::Object(object) = &root_or_value else {
                    return Err("Attempt to pass a property on a non-object by reference".into());
                };
                let class_name = object.borrow().class_name.clone();
                let metadata = find_property_in_scope(
                    &class_name,
                    property,
                    descriptor.access_scope.as_deref(),
                    classes,
                );
                if let Some(metadata) = metadata {
                    ensure_access(
                        metadata.visibility,
                        Some(&metadata.declaring_class),
                        descriptor.access_scope.as_deref(),
                        classes,
                        property,
                    )?;
                    if metadata.is_readonly {
                        return Err(format!(
                            "Cannot indirectly modify readonly property {class_name}::${property}"
                        ));
                    }
                    if metadata.type_name.is_some() {
                        return Err(format!(
                            "Typed property {class_name}::${property} must not be referenced"
                        ));
                    }
                }
                let storage_key = metadata.map_or_else(
                    || property.clone(),
                    |metadata| instance_property_storage_key(metadata, property),
                );
                let current = object
                    .borrow()
                    .properties
                    .get(&storage_key)
                    .cloned()
                    .unwrap_or(Value::Null);
                (
                    if keys.is_empty() {
                        current
                    } else {
                        current.array_get_path(&keys)?
                    },
                    Some((root_or_value.clone(), storage_key)),
                )
            } else {
                let resolved = if keys.is_empty() {
                    root_or_value
                } else {
                    root_or_value.array_get_path(&keys)?
                };
                (resolved, None)
            };
            arguments.push(value);
            references.push(Some(ReferenceArgument {
                variable: descriptor.variable.clone(),
                array_keys: keys,
                object_property,
                direct_reference: None,
            }));
        } else {
            let value = operands.next().ok_or("Invalid bytecode stack")?;
            if unpack.get(index).copied().unwrap_or(false) {
                let Value::Array(array) = value.dereference() else {
                    return Err("Only arrays can be unpacked into arguments".into());
                };
                let from_traversable = array.was_traversable_unpack;
                for (key, value) in array.iter() {
                    let name = match key {
                        ArrayKey::Integer(_) => None,
                        ArrayKey::String(name) => Some(name.clone()),
                    };
                    if from_traversable {
                        let temporary =
                            std::rc::Rc::new(std::cell::RefCell::new(value.dereference()));
                        arguments.push(value.dereference());
                        references.push(Some(ReferenceArgument {
                            variable: TRAVERSABLE_UNPACK_MARKER.into(),
                            array_keys: Vec::new(),
                            object_property: None,
                            direct_reference: Some(temporary),
                        }));
                    } else if let Value::Reference(reference) = value {
                        arguments.push(reference.borrow().dereference());
                        references.push(Some(ReferenceArgument {
                            variable: String::new(),
                            array_keys: Vec::new(),
                            object_property: None,
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

const TRAVERSABLE_UNPACK_MARKER: &str = "\0php-rust-traversable-unpack";

fn is_traversable_unpack_reference(reference: &ReferenceArgument) -> bool {
    reference.variable == TRAVERSABLE_UNPACK_MARKER
}

fn builtin_traversable_reference_parameter(
    name: &str,
    index: usize,
    argument_name: Option<&str>,
    value: &Value,
) -> bool {
    let name = name
        .rsplit('\\')
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    let parameter_name = argument_name.map(str::to_ascii_lowercase);
    let array_parameter = match name.as_str() {
        "array_push"
        | "array_pop"
        | "array_shift"
        | "array_unshift"
        | "array_splice"
        | "array_walk"
        | "array_walk_recursive"
        | "shuffle"
        | "sort"
        | "rsort"
        | "asort"
        | "arsort"
        | "ksort"
        | "krsort"
        | "natsort"
        | "natcasesort"
        | "usort"
        | "uasort"
        | "uksort"
        | "reset"
        | "end"
        | "next"
        | "prev"
        | "each" => index == 0,
        _ => false,
    };
    if name == "similar_text" {
        return index == 2 || parameter_name.as_deref() == Some("percent");
    }
    if name == "array_multisort" {
        return matches!(value.dereference(), Value::Array(_));
    }
    array_parameter
        || (parameter_name.as_deref() == Some("array")
            && matches!(
                name.as_str(),
                "array_push"
                    | "array_pop"
                    | "array_shift"
                    | "array_unshift"
                    | "array_splice"
                    | "array_walk"
                    | "array_walk_recursive"
                    | "shuffle"
                    | "sort"
                    | "rsort"
                    | "asort"
                    | "arsort"
                    | "ksort"
                    | "krsort"
                    | "natsort"
                    | "natcasesort"
                    | "usort"
                    | "uasort"
                    | "uksort"
                    | "reset"
                    | "end"
                    | "next"
                    | "prev"
                    | "each"
            ))
}

fn warn_traversable_by_reference(
    global_variables: &mut GlobalVariables,
    argument_index: usize,
    call_name: &str,
) {
    global_variables.report_diagnostic(crate::RuntimeDiagnostic {
        level: crate::DiagnosticLevel::Warning,
        message: format!("Cannot pass by-reference argument {argument_index} of {call_name} by unpacking a Traversable, passing by-value instead"),
        source_file: None,
        source_line: None,
        output_offset: 0,
    });
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
    if depth > 32 {
        return Err("IteratorAggregate nesting is too deep".into());
    }
    match value.dereference() {
        Value::Array(entries) => Ok(entries.into_iter().collect()),
        Value::Generator(generator) => {
            let mut rows = Vec::new();
            let mut current = if super::rewind_generator(
                &generator,
                functions,
                classes,
                static_properties,
                globals,
                output,
                steps,
            )? {
                super::current_generator_entry(&generator)?
            } else {
                None
            };
            while let Some((key, value)) = current {
                if ignore_iterator_keys {
                    rows.push((ArrayKey::Integer(rows.len() as i64), value));
                } else {
                    match key.array_key() {
                        Ok(key) => rows.push((key, value)),
                        Err(_) if skip_invalid_iterator_keys => {
                            globals.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Warning,
                                message: "Illegal offset type".into(),
                                source_file: None,
                                source_line: None,
                                output_offset: 0,
                            })
                        }
                        Err(error) => return Err(error),
                    }
                }
                current = super::advance_generator(
                    &generator,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                    Value::Null,
                )?;
            }
            Ok(rows)
        }
        Value::Object(object)
            if object
                .borrow()
                .class_name
                .eq_ignore_ascii_case("PDOStatement") =>
        {
            pdo_iterator_rows(
                &Value::Object(object),
                variables,
                bindings,
                globals,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )
        }
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            let normalized = class_name.to_ascii_lowercase();
            if super::is_a_type(
                &normalized,
                "iteratoraggregate",
                classes,
                &mut std::collections::HashSet::new(),
            ) {
                let mut call_stack = Vec::new();
                let iterator = super::control::call_iterator_method(
                    Value::Object(object),
                    "getiterator",
                    &mut call_stack,
                    variables,
                    bindings,
                    globals,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )?;
                if matches!(iterator.dereference(), Value::Array(_)) {
                    return Err(
                        "IteratorAggregate::getIterator() must return a Traversable object".into(),
                    );
                }
                return collect_traversable(
                    iterator,
                    variables,
                    bindings,
                    globals,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                    depth + 1,
                    ignore_iterator_keys,
                    skip_invalid_iterator_keys,
                );
            }
            if !super::is_a_type(
                &normalized,
                "iterator",
                classes,
                &mut std::collections::HashSet::new(),
            ) {
                return Err(format!("Object of type {class_name} is not Traversable"));
            }
            let required = ["rewind", "valid", "current", "key", "next"];
            if required
                .iter()
                .any(|method| super::objects::find_method(&class_name, method, classes).is_none())
            {
                return Err(format!(
                    "Object of type {class_name} does not provide the Iterator methods"
                ));
            }
            let iterator = Value::Object(object);
            let mut call_stack = Vec::new();
            let call = |method: &str,
                        call_stack: &mut Vec<Value>,
                        variables: &mut HashMap<String, Value>,
                        bindings: &mut HashMap<String, VariableBinding>,
                        globals: &mut GlobalVariables,
                        static_properties: &mut StaticProperties,
                        output: &mut String,
                        steps: &mut u64| {
                super::control::call_iterator_method(
                    iterator.clone(),
                    method,
                    call_stack,
                    variables,
                    bindings,
                    globals,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )
            };
            let _ = call(
                "rewind",
                &mut call_stack,
                variables,
                bindings,
                globals,
                static_properties,
                output,
                steps,
            )?;
            let mut rows = Vec::new();
            loop {
                if !call(
                    "valid",
                    &mut call_stack,
                    variables,
                    bindings,
                    globals,
                    static_properties,
                    output,
                    steps,
                )?
                .truthy()
                {
                    break;
                }
                let value = call(
                    "current",
                    &mut call_stack,
                    variables,
                    bindings,
                    globals,
                    static_properties,
                    output,
                    steps,
                )?;
                let key = if ignore_iterator_keys {
                    Some(ArrayKey::Integer(rows.len() as i64))
                } else {
                    let value = call(
                        "key",
                        &mut call_stack,
                        variables,
                        bindings,
                        globals,
                        static_properties,
                        output,
                        steps,
                    )?;
                    match value.array_key() {
                        Ok(key) => Some(key),
                        Err(_) if skip_invalid_iterator_keys => {
                            globals.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Warning,
                                message: "Illegal offset type".into(),
                                source_file: None,
                                source_line: None,
                                output_offset: 0,
                            });
                            None
                        }
                        Err(error) => return Err(error),
                    }
                };
                if let Some(key) = key {
                    rows.push((key, value));
                }
                let _ = call(
                    "next",
                    &mut call_stack,
                    variables,
                    bindings,
                    globals,
                    static_properties,
                    output,
                    steps,
                )?;
            }
            Ok(rows)
        }
        _ => Err("Value is not Traversable|array".into()),
    }
}

pub(super) fn callback_call(
    callback: Value,
    arguments: Value,
    strict_types: bool,
) -> Result<(Op, Vec<Value>), String> {
    match callback {
        Value::String(name) => {
            if let Some((class_name, method)) = name.split_once("::") {
                Ok((
                    Op::CallStaticMethod(
                        class_name.to_ascii_lowercase(),
                        method.to_ascii_lowercase(),
                        1,
                        vec![None],
                        vec![None],
                        vec![true],
                        None,
                        strict_types,
                    ),
                    vec![arguments],
                ))
            } else {
                Ok((
                    Op::Call(
                        name.to_ascii_lowercase(),
                        vec![None],
                        vec![None],
                        vec![true],
                        strict_types,
                    ),
                    vec![arguments],
                ))
            }
        }
        Value::Closure(closure) => Ok((
            Op::CallValue(1, vec![None], vec![None], vec![true], strict_types),
            vec![Value::Closure(closure), arguments],
        )),
        Value::Array(callable) => Ok((
            Op::CallValue(1, vec![None], vec![None], vec![true], strict_types),
            vec![Value::Array(callable), arguments],
        )),
        Value::Object(object) => Ok((
            Op::CallMethod(
                "__invoke".into(),
                1,
                vec![None],
                vec![None],
                vec![true],
                None,
                strict_types,
            ),
            vec![Value::Object(object), arguments],
        )),
        _ => Err("Callback must be callable".into()),
    }
}

pub(super) fn callback_call_args(
    callback: Value,
    arguments: Vec<Value>,
    strict_types: bool,
) -> Result<(Op, Vec<Value>), String> {
    match callback {
        Value::String(name) => {
            if let Some((class_name, method)) = name.split_once("::") {
                Ok((
                    Op::CallStaticMethod(
                        class_name.to_ascii_lowercase(),
                        method.to_ascii_lowercase(),
                        arguments.len(),
                        vec![None; arguments.len()],
                        vec![None; arguments.len()],
                        vec![false; arguments.len()],
                        None,
                        strict_types,
                    ),
                    arguments,
                ))
            } else {
                Ok((
                    Op::Call(
                        name.to_ascii_lowercase(),
                        vec![None; arguments.len()],
                        vec![None; arguments.len()],
                        vec![false; arguments.len()],
                        strict_types,
                    ),
                    arguments,
                ))
            }
        }
        Value::Closure(closure) => Ok((
            Op::CallValue(
                arguments.len(),
                vec![None; arguments.len()],
                vec![None; arguments.len()],
                vec![false; arguments.len()],
                strict_types,
            ),
            std::iter::once(Value::Closure(closure))
                .chain(arguments)
                .collect(),
        )),
        Value::Array(callable) => Ok((
            Op::CallValue(
                arguments.len(),
                vec![None; arguments.len()],
                vec![None; arguments.len()],
                vec![false; arguments.len()],
                strict_types,
            ),
            std::iter::once(Value::Array(callable))
                .chain(arguments)
                .collect(),
        )),
        Value::Object(object) => Ok((
            Op::CallMethod(
                "__invoke".into(),
                arguments.len(),
                vec![None; arguments.len()],
                vec![None; arguments.len()],
                vec![false; arguments.len()],
                None,
                strict_types,
            ),
            std::iter::once(Value::Object(object))
                .chain(arguments)
                .collect(),
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
            let from_traversable = !matches!(value.dereference(), Value::Array(_));
            let entries = collect_traversable(
                value,
                variables,
                bindings,
                global_variables,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
                0,
                false,
                false,
            )
            .map_err(|error| {
                if error == "Value is not Traversable|array" {
                    "Only arrays and Traversable objects can be unpacked".into()
                } else {
                    error
                }
            })?;
            let mut unpacked = crate::PhpArray::from_entries(entries);
            unpacked.was_traversable_unpack = from_traversable;
            stack.push(Value::Array(unpacked));
        }
        Op::Call(name, descriptors, argument_names, unpack, strict_types) => {
            let (arguments, by_reference, argument_names) =
                take_call_arguments(stack, descriptors, argument_names, unpack, classes)?;
            let normalized_name = name.to_ascii_lowercase();
            if functions.contains_key(&normalized_name) {
                let result = invoke_with_references(
                    &normalized_name,
                    arguments,
                    &by_reference,
                    &argument_names,
                    *strict_types,
                    variables,
                    bindings,
                    global_variables,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )?;
                stack.push(result);
            } else {
                // Unqualified names inside a namespace may fall back to a global function or builtin.
                let fallback_name = name.rsplit('\\').next().unwrap_or(name);
                if fallback_name.eq_ignore_ascii_case("exit")
                    || fallback_name.eq_ignore_ascii_case("die")
                {
                    let status = if arguments.is_empty() {
                        Value::Integer(0)
                    } else {
                        let (arguments, _) = super::builtin_arguments::bind(
                            "exit",
                            arguments,
                            &by_reference,
                            &argument_names,
                        )?;
                        arguments
                            .into_iter()
                            .next()
                            .ok_or("Invalid exit argument binding")?
                    };
                    apply_exit_status(
                        status,
                        output,
                        global_variables,
                        *strict_types,
                        functions,
                        classes,
                        static_properties,
                        steps,
                    )?;
                    stack.push(Value::Null);
                    return Ok(true);
                }
                if functions.contains_key(fallback_name) {
                    let result = invoke_with_references(
                        fallback_name,
                        arguments,
                        &by_reference,
                        &argument_names,
                        *strict_types,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?;
                    stack.push(result);
                    return Ok(true);
                }
                for (index, reference) in by_reference.iter().enumerate() {
                    if reference
                        .as_ref()
                        .is_some_and(is_traversable_unpack_reference)
                        && builtin_traversable_reference_parameter(
                            fallback_name,
                            index,
                            argument_names.get(index).and_then(Option::as_deref),
                            &arguments[index],
                        )
                    {
                        warn_traversable_by_reference(global_variables, index + 1, fallback_name);
                    }
                }
                if matches!(
                    fallback_name.to_ascii_lowercase().as_str(),
                    "func_get_args" | "func_get_arg" | "func_num_args"
                ) {
                    let (arguments, _) = super::builtin_arguments::bind(
                        fallback_name,
                        arguments,
                        &by_reference,
                        &argument_names,
                    )?;
                    let call_arguments = current_function_arguments(
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                    )?;
                    let value = match fallback_name.to_ascii_lowercase().as_str() {
                        "func_num_args" if arguments.is_empty() => {
                            Value::Integer(call_arguments.len() as i64)
                        }
                        "func_get_args" if arguments.is_empty() => Value::from_array_entries(
                            call_arguments
                                .into_iter()
                                .enumerate()
                                .map(|(index, value)| (ArrayKey::Integer(index as i64), value))
                                .collect(),
                        ),
                        "func_get_arg" if arguments.len() == 1 => {
                            let index = arguments[0].number() as i64;
                            if index < 0 {
                                Value::Bool(false)
                            } else {
                                call_arguments
                                    .get(index as usize)
                                    .cloned()
                                    .unwrap_or(Value::Bool(false))
                            }
                        }
                        "func_get_arg" => {
                            return Err("func_get_arg() expects exactly 1 argument".into())
                        }
                        "func_get_args" | "func_num_args" => {
                            return Err(format!("{}() expects no arguments", fallback_name))
                        }
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
                            if named_seen {
                                return Err("call_user_func(): positional argument follows a named argument".into());
                            }
                            if !positional_seen {
                                positional_seen = true;
                                callback = Some(value);
                            } else {
                                forwarded.push((value, None));
                            }
                        }
                    }
                    let callback =
                        callback.ok_or("call_user_func(): argument '$callback' is required")?;
                    let mut next_index = 0i64;
                    let entries = forwarded
                        .into_iter()
                        .map(|(value, name)| {
                            let key = if let Some(name) = name {
                                ArrayKey::String(name)
                            } else {
                                let key = ArrayKey::Integer(next_index);
                                next_index += 1;
                                key
                            };
                            let reference = Rc::new(std::cell::RefCell::new(value.dereference()));
                            (key, Value::Reference(reference))
                        })
                        .collect();
                    let arguments = Value::from_array_entries(entries);
                    let (instruction, mut callback_stack) =
                        callback_call(callback, arguments, false)?;
                    execute_instruction(
                        &instruction,
                        &mut callback_stack,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?;
                    stack.push(
                        callback_stack
                            .pop()
                            .ok_or("Invalid callback result stack")?,
                    );
                    return Ok(true);
                }
                let csv_escape_omitted = fallback_name.eq_ignore_ascii_case("str_getcsv")
                    && arguments.len() < 4
                    && !argument_names
                        .iter()
                        .flatten()
                        .any(|name| name.eq_ignore_ascii_case("escape"));
                let similar_text_percent_provided = fallback_name
                    .eq_ignore_ascii_case("similar_text")
                    && (arguments.len() >= 3
                        || argument_names
                            .iter()
                            .flatten()
                            .any(|name| name.eq_ignore_ascii_case("percent")));
                if fallback_name.eq_ignore_ascii_case("strtok")
                    && arguments.len() == 1
                    && !argument_names.iter().any(Option::is_some)
                {
                    let mut arguments = arguments;
                    coerce_stringable_builtin_args(
                        "strtok",
                        &mut arguments,
                        crate::string_builtins::string_argument_positions("strtok"),
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                    stack.push(strtok_with_state(&arguments, global_variables)?);
                    return Ok(true);
                }
                let (mut arguments, by_reference) = super::builtin_arguments::bind(
                    fallback_name,
                    arguments,
                    &by_reference,
                    &argument_names,
                )?;
                if fallback_name.eq_ignore_ascii_case("similar_text") {
                    coerce_stringable_builtin_args(
                        "similar_text",
                        &mut arguments,
                        crate::string_builtins::string_argument_positions("similar_text"),
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                    let (matches, percent) =
                        crate::string_builtins::similar_text_with_percent(&arguments)?;
                    if similar_text_percent_provided {
                        let target = by_reference.get(2).and_then(Option::as_ref)
                            .ok_or("TypeError:similar_text(): Argument #3 ($percent) cannot be passed by reference")?;
                        write_reference_argument(
                            target,
                            Value::Number(percent),
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                        )?;
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
                        output_offset: 0,
                    });
                }
                if fallback_name.eq_ignore_ascii_case("error_reporting") {
                    if arguments.len() > 1 {
                        return Err(
                            "ArgumentCountError:error_reporting() expects at most 1 argument"
                                .into(),
                        );
                    }
                    let previous = global_variables.error_reporting.unwrap_or(32767);
                    if let Some(level) = arguments.first() {
                        global_variables.error_reporting = Some(level.number() as i64);
                    }
                    stack.push(Value::Integer(previous));
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("set_error_handler") {
                    if arguments.is_empty() || arguments.len() > 2 {
                        return Err(
                            "ArgumentCountError:set_error_handler() expects 1 or 2 arguments"
                                .into(),
                        );
                    }
                    let handler = if matches!(arguments[0].dereference(), Value::Null) {
                        None
                    } else {
                        if !callable_status(&arguments[0], false, functions, classes).0 {
                            return Err("TypeError:set_error_handler(): Argument #1 ($callback) must be a valid callback".into());
                        }
                        Some(arguments[0].clone())
                    };
                    let mask = arguments.get(1).map(Value::number).unwrap_or(32767.0) as i64;
                    let previous = global_variables
                        .error_handler
                        .clone()
                        .unwrap_or(Value::Null);
                    global_variables.error_handler_stack.push((
                        global_variables.error_handler.clone(),
                        global_variables.error_handler_mask,
                    ));
                    global_variables.error_handler = handler;
                    global_variables.error_handler_mask = mask;
                    stack.push(previous);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("restore_error_handler") {
                    if !arguments.is_empty() {
                        return Err("ArgumentCountError:restore_error_handler() expects exactly 0 arguments".into());
                    }
                    if let Some((previous, previous_mask)) =
                        global_variables.error_handler_stack.pop()
                    {
                        global_variables.error_handler = previous;
                        global_variables.error_handler_mask = previous_mask;
                    }
                    stack.push(Value::Bool(true));
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("trigger_error")
                    || fallback_name.eq_ignore_ascii_case("user_error")
                {
                    if arguments.is_empty() || arguments.len() > 2 {
                        return Err(
                            "ArgumentCountError:trigger_error() expects 1 or 2 arguments".into(),
                        );
                    }
                    let message = match arguments[0].dereference() {
                        Value::String(message) => message,
                        other => return Err(format!("TypeError:trigger_error(): Argument #1 ($message) must be of type string, {} given", other.type_name())),
                    };
                    let error_type = arguments.get(1).map(Value::number).unwrap_or(1024.0) as i64;
                    let level = match error_type {
                        512 => crate::DiagnosticLevel::UserWarning,
                        1024 => crate::DiagnosticLevel::UserNotice,
                        16384 => crate::DiagnosticLevel::UserDeprecated,
                        256 => {
                            global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Deprecated,
                                message: "Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead".into(),
                                source_file: None,
                                source_line: None,
                                output_offset: output.len(),
                            });
                            global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Fatal,
                                message,
                                source_file: None,
                                source_line: None,
                                output_offset: output.len(),
                            });
                            global_variables.exit_code = 255;
                            global_variables.exiting = true;
                            stack.push(Value::Bool(true));
                            return Ok(true);
                        }
                        _ => return Err("ValueError:trigger_error(): Argument #2 ($error_level) must be one of E_USER_ERROR, E_USER_WARNING, E_USER_NOTICE, or E_USER_DEPRECATED".into()),
                    };
                    global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                        level,
                        message,
                        source_file: None,
                        source_line: None,
                        output_offset: output.len(),
                    });
                    stack.push(Value::Bool(true));
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("error_get_last") {
                    if !arguments.is_empty() {
                        return Err(
                            "ArgumentCountError:error_get_last() expects exactly 0 arguments"
                                .into(),
                        );
                    }
                    let value =
                        global_variables
                            .last_error
                            .as_ref()
                            .map_or(Value::Null, |diagnostic| {
                                let mut entries = vec![
                                    (
                                        crate::ArrayKey::String("type".into()),
                                        Value::Integer(match diagnostic.level {
                                            crate::DiagnosticLevel::Warning => 2,
                                            crate::DiagnosticLevel::Deprecated => 8192,
                                            crate::DiagnosticLevel::Notice => 8,
                                            crate::DiagnosticLevel::UserWarning => 512,
                                            crate::DiagnosticLevel::UserDeprecated => 16384,
                                            crate::DiagnosticLevel::UserNotice => 1024,
                                            crate::DiagnosticLevel::Fatal => 256,
                                        }),
                                    ),
                                    (
                                        crate::ArrayKey::String("message".into()),
                                        Value::String(diagnostic.message.clone()),
                                    ),
                                    (
                                        crate::ArrayKey::String("file".into()),
                                        Value::String(
                                            diagnostic
                                                .source_file
                                                .clone()
                                                .unwrap_or_else(|| "Command line code".into()),
                                        ),
                                    ),
                                    (
                                        crate::ArrayKey::String("line".into()),
                                        Value::Integer(diagnostic.source_line.unwrap_or(0) as i64),
                                    ),
                                ];
                                Value::from_array_entries(std::mem::take(&mut entries))
                            });
                    stack.push(value);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("error_clear_last") {
                    if !arguments.is_empty() {
                        return Err(
                            "ArgumentCountError:error_clear_last() expects exactly 0 arguments"
                                .into(),
                        );
                    }
                    global_variables.last_error = None;
                    stack.push(Value::Null);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("strtok") {
                    coerce_stringable_builtin_args(
                        "strtok",
                        &mut arguments,
                        crate::string_builtins::string_argument_positions("strtok"),
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                    stack.push(strtok_with_state(&arguments, global_variables)?);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("iterator_count") {
                    stack.push(iterator_count(
                        &arguments,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?);
                    return Ok(true);
                }
                if fallback_name.eq_ignore_ascii_case("iterator_to_array") {
                    stack.push(iterator_to_array(
                        &arguments,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?);
                    return Ok(true);
                }
                match fallback_name.to_ascii_lowercase().as_str() {
                    "abs" | "ceil" | "floor" | "round" | "sqrt" | "fdiv" | "fmod" | "intdiv"
                    | "is_finite" | "is_infinite" | "is_nan" | "exp" | "expm1" | "log"
                    | "log10" | "log1p" | "sin" | "cos" | "tan" | "asin" | "acos" | "atan"
                    | "atan2" | "sinh" | "cosh" | "tanh" | "asinh" | "acosh" | "atanh"
                    | "deg2rad" | "rad2deg" | "hypot" | "max" | "min" | "base_convert"
                    | "decbin" | "dechex" | "decoct" => {
                        stack.push(crate::math_builtins::call(fallback_name, &arguments)?);
                    }
                    "mongo_find"
                    | "mongo_aggregate"
                    | "mongo_count_documents"
                    | "mongo_estimated_document_count"
                    | "mongo_insert_one"
                    | "mongo_insert_many"
                    | "mongo_replace_one"
                    | "mongo_update_one"
                    | "mongo_update_many"
                    | "mongo_delete_one"
                    | "mongo_delete_many"
                    | "mongo_begin_transaction"
                    | "mongo_commit"
                    | "mongo_rollback" => {
                        stack.push(crate::mongo_php::call(
                            &fallback_name.to_ascii_lowercase(),
                            &arguments,
                        )?);
                    }
                    "function_exists" => {
                        let name = arguments
                            .first()
                            .ok_or("function_exists() expects exactly 1 argument")?
                            .show();
                        stack.push(Value::Bool(runtime_function_exists(&name, functions)));
                    }
                    "spl_autoload_register" => {
                        let callback = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .cloned()
                            .unwrap_or_else(|| Value::String("spl_autoload".into()));
                        let (valid, _) = callable_status(&callback, false, functions, classes);
                        if !valid {
                            if arguments.get(1).is_some_and(Value::truthy) {
                                return Err("TypeError:spl_autoload_register(): Argument #1 ($callback) must be a valid callback".into());
                            }
                            stack.push(Value::Bool(false));
                            return Ok(true);
                        }
                        let identity = autoload_callback_identity(&callback);
                        if !global_variables
                            .autoload_callbacks
                            .iter()
                            .any(|existing| autoload_callback_identity(existing) == identity)
                        {
                            if arguments.get(2).is_some_and(Value::truthy) {
                                global_variables.autoload_callbacks.insert(0, callback);
                            } else {
                                global_variables.autoload_callbacks.push(callback);
                            }
                        }
                        stack.push(Value::Bool(true));
                    }
                    "spl_autoload_unregister" => {
                        let callback = arguments
                            .first()
                            .ok_or("spl_autoload_unregister() expects exactly 1 argument")?;
                        let identity = autoload_callback_identity(callback);
                        let before = global_variables.autoload_callbacks.len();
                        global_variables
                            .autoload_callbacks
                            .retain(|existing| autoload_callback_identity(existing) != identity);
                        stack.push(Value::Bool(
                            global_variables.autoload_callbacks.len() != before,
                        ));
                    }
                    "spl_autoload_functions" => {
                        if global_variables.autoload_callbacks.is_empty() {
                            stack.push(Value::Bool(false));
                        } else {
                            stack.push(Value::from_array_entries(
                                global_variables
                                    .autoload_callbacks
                                    .iter()
                                    .enumerate()
                                    .map(|(index, callback)| {
                                        (ArrayKey::Integer(index as i64), callback.clone())
                                    })
                                    .collect(),
                            ));
                        }
                    }
                    "spl_autoload_call" => {
                        let requested = arguments
                            .first()
                            .ok_or("spl_autoload_call() expects exactly 1 argument")?
                            .show();
                        invoke_registered_autoloaders(
                            &requested,
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                            functions,
                            classes,
                            static_properties,
                            output,
                            steps,
                        )?;
                        stack.push(Value::Null);
                    }
                    "spl_autoload" => {
                        let requested = arguments
                            .first()
                            .ok_or("spl_autoload() expects exactly 1 argument")?
                            .show();
                        let extensions = arguments
                            .get(1)
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        invoke_spl_autoload(
                            &requested,
                            extensions.as_deref(),
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                            functions,
                            classes,
                            static_properties,
                            output,
                            steps,
                        )?;
                        stack.push(Value::Null);
                    }
                    "spl_autoload_extensions" => {
                        if arguments.len() > 1 {
                            return Err(
                                "spl_autoload_extensions() expects at most 1 argument".into()
                            );
                        }
                        let previous = global_variables
                            .autoload_extensions
                            .clone()
                            .unwrap_or_else(|| ".inc,.php".into());
                        if let Some(extension) = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                        {
                            global_variables.autoload_extensions = Some(extension.show());
                        }
                        stack.push(Value::String(previous));
                    }
                    "gc_collect_cycles" => {
                        if !arguments.is_empty() {
                            return Err(format!(
                                "gc_collect_cycles() expects exactly 0 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let collected = super::objects::collect_object_cycles(
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        stack.push(Value::Integer(collected as i64));
                    }
                    "getenv" => stack.push(getenv_value(&arguments, global_variables)?),
                    "putenv" => stack.push(putenv_value(&arguments, global_variables)?),
                    "php_sapi_name" => {
                        if !arguments.is_empty() {
                            return Err("php_sapi_name() expects exactly 0 arguments".into());
                        }
                        stack.push(
                            global_variables
                                .get("\0sapi_name")
                                .cloned()
                                .unwrap_or_else(|| Value::String("cli".into())),
                        );
                    }
                    "var_dump" => {
                        if arguments.is_empty() {
                            return Err("var_dump() expects at least 1 argument, 0 given".into());
                        }
                        let mut seen = HashSet::new();
                        let mut object_ids = HashMap::new();
                        let mut next_object_id = 1usize;
                        for value in &arguments {
                            let dump = format_dump_value(
                                value,
                                0,
                                &mut seen,
                                &mut object_ids,
                                &mut next_object_id,
                                functions,
                                classes,
                                static_properties,
                                global_variables,
                                output,
                                steps,
                            )?;
                            output.push_str(&dump);
                            global_variables.output_offset = output.len();
                        }
                        stack.push(Value::Null);
                    }
                    "intval" | "floatval" | "strval" | "boolval" | "gettype" | "get_debug_type"
                    | "is_int" | "is_integer" | "is_long" | "is_float" | "is_double"
                    | "is_real" | "is_string" | "is_array" | "is_countable" | "is_iterable"
                    | "is_bool" | "is_null" | "is_object" | "is_resource" | "is_scalar"
                    | "is_numeric" => {
                        if fallback_name.eq_ignore_ascii_case("strval") {
                            let value = arguments
                                .first()
                                .ok_or("strval() expects exactly 1 argument")?
                                .dereference();
                            let string = super::objects::stringify(
                                &value,
                                functions,
                                classes,
                                static_properties,
                                global_variables,
                                output,
                                steps,
                            )?;
                            stack.push(Value::String(string));
                        } else {
                            stack.push(type_builtin(
                                &fallback_name.to_ascii_lowercase(),
                                &arguments,
                                classes,
                            )?);
                        }
                    }
                    "class_exists" | "interface_exists" | "trait_exists" => {
                        let name = arguments
                            .first()
                            .ok_or("class_exists() expects at least 1 argument")?
                            .show();
                        let key = name.trim_start_matches('\\').to_ascii_lowercase();
                        let built_in_class = matches!(
                            key.as_str(),
                            "generator"
                                | "pdo"
                                | "pdostatement"
                                | "reflectionclass"
                                | "reflectionobject"
                                | "reflectionclassconstant"
                                | "reflectionconstant"
                                | "reflectionenum"
                                | "reflectionenumunitcase"
                                | "reflectionenumbackedcase"
                                | "reflectionnamedtype"
                                | "reflectiontype"
                                | "reflectionuniontype"
                                | "reflectionintersectiontype"
                                | "reflectionfunction"
                                | "reflectionmethod"
                                | "reflectionproperty"
                                | "reflectionattribute"
                                | "reflectionparameter"
                                | "attribute"
                                | "mongodb\\client"
                                | "mongodb\\database"
                                | "mongodb\\collection"
                                | "mongodb\\cursor"
                                | "mongodb\\insertresult"
                                | "mongodb\\insertmanyresult"
                                | "mongodb\\updateresult"
                                | "mongodb\\deleteresult"
                        );
                        let wants_interface =
                            fallback_name.eq_ignore_ascii_case("interface_exists");
                        let wants_trait = fallback_name.eq_ignore_ascii_case("trait_exists");
                        let mut exists = if wants_trait {
                            classes.get(&key).is_some_and(|class| class.is_trait)
                        } else if wants_interface {
                            crate::exception_builtins::is_interface(&key)
                        } else {
                            crate::exception_builtins::is_class(&key) || built_in_class
                        } || classes.get(&key).is_some_and(|class| {
                            if wants_interface {
                                class.is_interface
                            } else {
                                !class.is_interface && !class.is_trait
                            }
                        });
                        exists |= global_variables
                            .dynamic_classes
                            .get(&key)
                            .is_some_and(|class| {
                                if wants_trait {
                                    class.is_trait
                                } else if wants_interface {
                                    class.is_interface
                                } else {
                                    !class.is_interface && !class.is_trait
                                }
                            });
                        if !exists && arguments.get(1).is_none_or(Value::truthy) {
                            invoke_registered_autoloaders(
                                &name,
                                variables,
                                bindings,
                                global_variables,
                                global_scope,
                                frame_id,
                                functions,
                                classes,
                                static_properties,
                                output,
                                steps,
                            )?;
                            exists = if wants_trait {
                                global_variables
                                    .dynamic_classes
                                    .get(&key)
                                    .is_some_and(|class| class.is_trait)
                            } else if wants_interface {
                                crate::exception_builtins::is_interface(&key)
                                    || global_variables
                                        .dynamic_classes
                                        .get(&key)
                                        .is_some_and(|class| class.is_interface)
                            } else {
                                crate::exception_builtins::is_class(&key)
                                    || built_in_class
                                    || global_variables
                                        .dynamic_classes
                                        .get(&key)
                                        .is_some_and(|class| !class.is_interface && !class.is_trait)
                            };
                        }
                        stack.push(Value::Bool(exists));
                    }
                    "get_class" | "get_parent_class" => {
                        let wants_parent = fallback_name.eq_ignore_ascii_case("get_parent_class");
                        let target = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null));
                        let class_key = match target.map(Value::dereference) {
                            Some(Value::Object(object)) => {
                                Some(object.borrow().class_name.to_ascii_lowercase())
                            }
                            Some(Value::Generator(_)) => Some("generator".into()),
                            Some(Value::String(name)) if wants_parent => {
                                Some(name.trim_start_matches('\\').to_ascii_lowercase())
                            }
                            Some(_) => {
                                return Err(if wants_parent {
                                    "get_parent_class(): Argument #1 must be an object or a class name".into()
                                } else {
                                    "get_class(): Argument #1 must be an object".into()
                                })
                            }
                            None => match variables.get("this") {
                                Some(Value::Object(object)) => {
                                    Some(object.borrow().class_name.to_ascii_lowercase())
                                }
                                _ => variables
                                    .get("\0static_class")
                                    .map(Value::show)
                                    .map(|name| name.to_ascii_lowercase()),
                            },
                        };
                        let result = class_key.and_then(|key| {
                            if let Some(class) = classes.get(&key) {
                                if wants_parent {
                                    class
                                        .parent
                                        .as_ref()
                                        .and_then(|parent| classes.get(parent))
                                        .map(|parent| parent.name.clone())
                                } else {
                                    Some(class.reflection_name.clone())
                                }
                            } else if !wants_parent {
                                match key.as_str() {
                                    "pdo" => Some("PDO".into()),
                                    "pdostatement" => Some("PDOStatement".into()),
                                    "generator" => Some("Generator".into()),
                                    "stdclass" => Some("stdClass".into()),
                                    "reflectionclass" => Some("ReflectionClass".into()),
                                    "reflectionobject" => Some("ReflectionObject".into()),
                                    "reflectionclassconstant" => {
                                        Some("ReflectionClassConstant".into())
                                    }
                                    "reflectionconstant" => Some("ReflectionConstant".into()),
                                    "reflectionenum" => Some("ReflectionEnum".into()),
                                    "reflectionenumunitcase" => {
                                        Some("ReflectionEnumUnitCase".into())
                                    }
                                    "reflectionenumbackedcase" => {
                                        Some("ReflectionEnumBackedCase".into())
                                    }
                                    "reflectionnamedtype" => Some("ReflectionNamedType".into()),
                                    "reflectionuniontype" => Some("ReflectionUnionType".into()),
                                    "reflectionintersectiontype" => {
                                        Some("ReflectionIntersectionType".into())
                                    }
                                    "reflectionfunction" => Some("ReflectionFunction".into()),
                                    "reflectionmethod" => Some("ReflectionMethod".into()),
                                    "reflectionproperty" => Some("ReflectionProperty".into()),
                                    "reflectionattribute" => Some("ReflectionAttribute".into()),
                                    "reflectionparameter" => Some("ReflectionParameter".into()),
                                    _ => crate::exception_builtins::canonical_name(&key)
                                        .map(str::to_string),
                                }
                            } else if wants_parent {
                                super::builtin_parent_class(&key)
                                    .map(str::to_string)
                                    .or_else(|| {
                                        crate::exception_builtins::parent_class(&key)
                                            .map(str::to_string)
                                    })
                            } else if crate::exception_builtins::is_class(&key) {
                                crate::exception_builtins::parent_class(&key).map(str::to_string)
                            } else {
                                None
                            }
                        });
                        stack.push(result.map(Value::String).unwrap_or(Value::Bool(false)));
                    }
                    "get_called_class" => {
                        if !arguments.is_empty() {
                            return Err(format!(
                                "get_called_class() expects exactly 0 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let called_class = variables
                            .get("\0static_class")
                            .map(Value::show)
                            .ok_or_else(|| {
                                "Error:get_called_class() must be called from within a class"
                                    .to_string()
                            })?;
                        stack.push(Value::String(
                            classes
                                .get(&called_class.to_ascii_lowercase())
                                .map(|class| class.name.clone())
                                .unwrap_or(called_class),
                        ));
                    }
                    "is_a" | "is_subclass_of" => {
                        let is_subclass = fallback_name.eq_ignore_ascii_case("is_subclass_of");
                        let function_name = if is_subclass {
                            "is_subclass_of"
                        } else {
                            "is_a"
                        };
                        let value = arguments
                            .first()
                            .ok_or_else(|| {
                                format!("{function_name}() expects at least 2 arguments")
                            })?
                            .dereference();
                        let expected = arguments
                            .get(1)
                            .ok_or_else(|| {
                                format!("{function_name}() expects at least 2 arguments")
                            })?
                            .show()
                            .trim_start_matches('\\')
                            .to_ascii_lowercase();
                        let allow_string =
                            arguments.get(2).map(Value::truthy).unwrap_or(is_subclass);
                        let actual = match value {
                            Value::Object(object) => {
                                Some(object.borrow().class_name.to_ascii_lowercase())
                            }
                            Value::Generator(_) => Some("generator".into()),
                            Value::String(name) if allow_string => {
                                Some(name.trim_start_matches('\\').to_ascii_lowercase())
                            }
                            _ => None,
                        };
                        let matches = actual.is_some_and(|actual| {
                            let same_class = actual.eq_ignore_ascii_case(&expected);
                            if is_subclass && same_class {
                                return false;
                            }
                            super::is_a_type(
                                &actual,
                                &expected,
                                classes,
                                &mut std::collections::HashSet::new(),
                            ) || !is_subclass
                                && matches!(
                                    actual.as_str(),
                                    "generator" | "pdo" | "pdostatement" | "stdclass"
                                )
                                && same_class
                        });
                        stack.push(Value::Bool(matches));
                    }
                    "method_exists" => {
                        let target = arguments
                            .first()
                            .ok_or("method_exists() expects exactly 2 arguments")?;
                        let method = arguments
                            .get(1)
                            .ok_or("method_exists() expects exactly 2 arguments")?
                            .show()
                            .to_ascii_lowercase();
                        let class_name = match target.dereference() {
                            Value::Object(object) => object.borrow().class_name.clone(),
                            Value::Generator(_) => "generator".into(),
                            Value::String(name) => {
                                name.trim_start_matches('\\').to_ascii_lowercase()
                            }
                            _ => String::new(),
                        };
                        let generator_method = matches!(
                            method.as_str(),
                            "current"
                                | "getreturn"
                                | "key"
                                | "next"
                                | "rewind"
                                | "send"
                                | "throw"
                                | "valid"
                        );
                        let reflection_method = match class_name.to_ascii_lowercase().as_str() {
                            "reflectionclass" | "reflectionobject" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getshortname"
                                    | "getnamespacename"
                                    | "innamespace"
                                    | "getattributes"
                                    | "getmethod"
                                    | "getmethods"
                                    | "getconstructor"
                                    | "getproperty"
                                    | "getproperties"
                                    | "getdefaultproperties"
                                    | "getstaticproperties"
                                    | "getparentclass"
                                    | "getinterfacenames"
                                    | "getinterfaces"
                                    | "gettraitnames"
                                    | "gettraits"
                                    | "gettraitaliases"
                                    | "hasmethod"
                                    | "hasproperty"
                                    | "getconstant"
                                    | "getconstants"
                                    | "getreflectionconstant"
                                    | "hasconstant"
                                    | "isenum"
                                    | "isinterface"
                                    | "isabstract"
                                    | "isinstantiable"
                                    | "isfinal"
                                    | "isreadonly"
                                    | "isanonymous"
                                    | "istrait"
                                    | "isinstance"
                                    | "iscloneable"
                                    | "isiterable"
                                    | "isiterateable"
                                    | "issubclassof"
                                    | "implementsinterface"
                            ),
                            "reflectionenum" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getshortname"
                                    | "getnamespacename"
                                    | "innamespace"
                                    | "getattributes"
                                    | "getmethod"
                                    | "getmethods"
                                    | "getconstructor"
                                    | "getproperty"
                                    | "getproperties"
                                    | "getdefaultproperties"
                                    | "getstaticproperties"
                                    | "getparentclass"
                                    | "getinterfacenames"
                                    | "getinterfaces"
                                    | "gettraitnames"
                                    | "gettraits"
                                    | "gettraitaliases"
                                    | "getcases"
                                    | "getcase"
                                    | "hascase"
                                    | "getconstant"
                                    | "getconstants"
                                    | "getreflectionconstant"
                                    | "hasconstant"
                                    | "isbacked"
                                    | "getbackingtype"
                                    | "isinstantiable"
                                    | "isreadonly"
                                    | "isanonymous"
                                    | "istrait"
                                    | "isinstance"
                                    | "iscloneable"
                                    | "isiterable"
                                    | "isiterateable"
                            ),
                            "reflectionconstant" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getvalue"
                                    | "getattributes"
                                    | "getnamespace"
                                    | "isdeprecated"
                                    | "isinternal"
                                    | "isuserdefined"
                            ),
                            "reflectionclassconstant" => matches!(
                                method.as_str(),
                                "getattributes"
                                    | "getdeclaringclass"
                                    | "getmodifiers"
                                    | "getname"
                                    | "gettype"
                                    | "getvalue"
                                    | "hastype"
                                    | "isenumcase"
                                    | "isfinal"
                                    | "isprivate"
                                    | "isprotected"
                                    | "ispublic"
                            ),
                            "reflectionuniontype" | "reflectionintersectiontype" => {
                                matches!(method.as_str(), "gettypes" | "allowsnull")
                            }
                            "reflectionnamedtype" => {
                                matches!(method.as_str(), "getname" | "isbuiltin" | "allowsnull")
                            }
                            "reflectionenumunitcase" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getvalue"
                                    | "getenum"
                                    | "getattributes"
                                    | "getdeclaringclass"
                                    | "getmodifiers"
                                    | "gettype"
                                    | "hastype"
                                    | "isenumcase"
                                    | "isfinal"
                                    | "isprivate"
                                    | "isprotected"
                                    | "ispublic"
                            ),
                            "reflectionenumbackedcase" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getvalue"
                                    | "getenum"
                                    | "getbackingvalue"
                                    | "getattributes"
                                    | "getdeclaringclass"
                                    | "getmodifiers"
                                    | "gettype"
                                    | "hastype"
                                    | "isenumcase"
                                    | "isfinal"
                                    | "isprivate"
                                    | "isprotected"
                                    | "ispublic"
                            ),
                            "reflectionmethod" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getdeclaringclass"
                                    | "getmodifiers"
                                    | "getattributes"
                                    | "getparameters"
                                    | "setaccessible"
                                    | "ispublic"
                                    | "isprotected"
                                    | "isprivate"
                                    | "isstatic"
                                    | "isabstract"
                                    | "isfinal"
                                    | "isconstructor"
                                    | "isdestructor"
                                    | "getnumberofparameters"
                                    | "getnumberofrequiredparameters"
                                    | "hasreturntype"
                                    | "getreturntype"
                                    | "invoke"
                                    | "invokeargs"
                                    | "getclosure"
                                    | "createfrommethodname"
                                    | "isvariadic"
                                    | "returnsreference"
                                    | "isclosure"
                                    | "hasprototype"
                                    | "getprototype"
                            ),
                            "reflectionparameter" => matches!(
                                method.as_str(),
                                "__construct"
                                    | "getname"
                                    | "getposition"
                                    | "getattributes"
                                    | "getdeclaringclass"
                                    | "getdeclaringfunction"
                                    | "ispassedbyreference"
                                    | "canbepassedbyvalue"
                                    | "isvariadic"
                                    | "isoptional"
                                    | "isdefaultvalueavailable"
                                    | "getdefaultvalue"
                                    | "isdefaultvalueconstant"
                                    | "getdefaultvalueconstantname"
                                    | "hastype"
                                    | "gettype"
                                    | "allowsnull"
                                    | "ispromoted"
                                    | "getclass"
                                    | "isarray"
                                    | "iscallable"
                            ),
                            "reflectionfunction" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getattributes"
                                    | "getparameters"
                                    | "getnumberofparameters"
                                    | "getnumberofrequiredparameters"
                                    | "hasreturntype"
                                    | "getreturntype"
                                    | "isvariadic"
                                    | "returnsreference"
                                    | "isclosure"
                                    | "isstatic"
                                    | "invoke"
                                    | "invokeargs"
                                    | "getclosure"
                            ),
                            "reflectionproperty" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getvalue"
                                    | "setvalue"
                                    | "setaccessible"
                                    | "getdeclaringclass"
                                    | "gettype"
                                    | "hastype"
                                    | "getattributes"
                                    | "getmodifiers"
                                    | "ispublic"
                                    | "isprotected"
                                    | "isprivate"
                                    | "isstatic"
                                    | "isreadonly"
                                    | "isprivateset"
                                    | "isprotectedset"
                                    | "ispromoted"
                                    | "isinitialized"
                                    | "hasdefaultvalue"
                                    | "getdefaultvalue"
                            ),
                            "reflectionattribute" => matches!(
                                method.as_str(),
                                "getname"
                                    | "getarguments"
                                    | "gettarget"
                                    | "isrepeated"
                                    | "newinstance"
                            ),
                            _ => false,
                        };
                        let enum_interface_method = match class_name.to_ascii_lowercase().as_str() {
                            "unitenum" => method == "cases",
                            "backedenum" => matches!(method.as_str(), "from" | "tryfrom"),
                            _ => false,
                        };
                        let inherited_throwable_method = !class_name.is_empty()
                            && !crate::exception_builtins::is_interface(&class_name)
                            && super::is_a_type(
                                &class_name.to_ascii_lowercase(),
                                "throwable",
                                classes,
                                &mut std::collections::HashSet::new(),
                            )
                            && crate::exception_builtins::method_exists("Exception", &method);
                        stack.push(Value::Bool(
                            reflection_method
                                || enum_interface_method
                                || generator_method && class_name == "generator"
                                || crate::exception_builtins::method_exists(&class_name, &method)
                                || inherited_throwable_method
                                || !class_name.is_empty()
                                    && (find_method(&class_name, &method, classes).is_some()
                                        || crate::pdo::method_exists(&class_name, &method)
                                        || crate::mongo_php::method_exists(&class_name, &method)),
                        ));
                    }
                    "property_exists" => {
                        let target = arguments
                            .first()
                            .ok_or("property_exists() expects exactly 2 arguments")?
                            .dereference();
                        let property = arguments
                            .get(1)
                            .ok_or("property_exists() expects exactly 2 arguments")?
                            .show();
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
                        let value = arguments
                            .first()
                            .ok_or("is_callable() expects exactly 1 argument")?;
                        let syntax_only = arguments.get(1).is_some_and(Value::truthy);
                        let (callable, name) =
                            callable_status(value, syntax_only, functions, classes);
                        if let Some(Some(target)) = by_reference.get(2) {
                            write_reference_argument(
                                target,
                                Value::String(name),
                                variables,
                                bindings,
                                global_variables,
                                global_scope,
                            )?;
                        }
                        stack.push(Value::Bool(callable));
                    }
                    "call_user_func_array" => {
                        let callback = arguments
                            .first()
                            .ok_or("call_user_func_array() expects exactly 2 arguments")?
                            .clone();
                        let callback_arguments = arguments
                            .get(1)
                            .ok_or("call_user_func_array() expects exactly 2 arguments")?
                            .clone();
                        if !matches!(callback_arguments.dereference(), Value::Array(_)) {
                            return Err(
                                "call_user_func_array(): Argument #2 ($args) must be of type array"
                                    .into(),
                            );
                        }
                        let (nested_instruction, mut nested_stack) = callback_call(callback, callback_arguments, false)
                            .map_err(|_| "call_user_func_array(): Argument #1 ($callback) must be a valid callback")?;
                        execute_instruction(
                            &nested_instruction,
                            &mut nested_stack,
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                            functions,
                            classes,
                            static_properties,
                            output,
                            steps,
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
                            if name.is_empty()
                                || !name.bytes().all(|b| {
                                    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
                                })
                            {
                                return Err("header(): invalid header name".into());
                            }
                            let replace = arguments.get(1).map(Value::truthy).unwrap_or(true);
                            if replace {
                                global_variables
                                    .http_headers
                                    .retain(|(key, _)| !key.eq_ignore_ascii_case(name));
                            }
                            global_variables
                                .http_headers
                                .push((name.to_string(), value));
                        } else if let Some(value) = line.strip_prefix("HTTP/") {
                            if let Some(code) = value
                                .split_whitespace()
                                .nth(1)
                                .and_then(|v| v.parse::<u16>().ok())
                                .filter(|v| (100..=599).contains(v))
                            {
                                global_variables.http_status = Some(code);
                            }
                        } else {
                            return Err("header(): malformed header line".into());
                        }
                        if let Some(code) = arguments
                            .get(2)
                            .and_then(|value| value.show().parse::<u16>().ok())
                            .filter(|v| (100..=599).contains(v))
                        {
                            global_variables.http_status = Some(code);
                        }
                        stack.push(Value::Null);
                    }
                    "http_response_code" => {
                        let old = global_variables.http_status.unwrap_or(200);
                        if let Some(value) = arguments
                            .first()
                            .filter(|value| !matches!(value, Value::Null))
                        {
                            let code = value
                                .show()
                                .parse::<u16>()
                                .map_err(|_| "http_response_code(): invalid status code")?;
                            if !(100..=599).contains(&code) {
                                return Err(
                                    "http_response_code(): status code must be between 100 and 599"
                                        .into(),
                                );
                            }
                            global_variables.http_status = Some(code);
                        }
                        stack.push(Value::Integer(i64::from(old)));
                    }
                    "header_remove" => {
                        if let Some(name) = arguments
                            .first()
                            .filter(|value| !matches!(value, Value::Null))
                        {
                            let name = name.show();
                            global_variables
                                .http_headers
                                .retain(|(key, _)| !key.eq_ignore_ascii_case(&name));
                        } else {
                            global_variables.http_headers.clear();
                        }
                        stack.push(Value::Null);
                    }
                    "headers_list" => {
                        let entries = global_variables
                            .http_headers
                            .iter()
                            .enumerate()
                            .map(|(index, (name, value))| {
                                (
                                    ArrayKey::Integer(index as i64),
                                    Value::String(format!("{name}: {value}")),
                                )
                            })
                            .collect();
                        stack.push(Value::from_array_entries(entries));
                    }
                    "scandir" => {
                        stack.push(scandir_builtin(&arguments, global_variables)?);
                    }
                    "opendir" => stack.push(opendir_builtin(&arguments, global_variables)?),
                    "readdir" => stack.push(readdir_builtin(&arguments, global_variables)?),
                    "closedir" => stack.push(closedir_builtin(&arguments, global_variables)?),
                    "rewinddir" => stack.push(rewinddir_builtin(&arguments, global_variables)?),
                    "stat" | "lstat" => {
                        if arguments.len() != 1 {
                            return Err(format!(
                                "ArgumentCountError:{}() expects exactly 1 argument, {} given",
                                fallback_name,
                                arguments.len()
                            ));
                        }
                        let filename = arguments[0].show();
                        stack.push(file_stat_value(fallback_name, &filename, global_variables));
                    }
                    "md5_file" => {
                        stack.push(md5_file_builtin(&arguments, global_variables)?);
                    }
                    "sha1_file" => {
                        stack.push(sha1_file_builtin(&arguments, global_variables)?);
                    }
                    "hash_file" => {
                        stack.push(hash_file_builtin(&arguments, global_variables)?);
                    }
                    "hash_hmac_file" => {
                        stack.push(hash_hmac_file_builtin(&arguments, global_variables)?);
                    }
                    "file_exists" | "is_file" | "is_dir" | "is_link" | "is_readable"
                    | "is_writable" | "is_executable" | "filesize" | "filemtime" | "fileatime"
                    | "filectime" | "fileinode" | "fileowner" | "filegroup" | "fileperms"
                    | "filetype" | "realpath" => {
                        if arguments.len() != 1 {
                            return Err(format!(
                                "ArgumentCountError:{}() expects exactly 1 argument, {} given",
                                fallback_name,
                                arguments.len()
                            ));
                        }
                        let path = arguments
                            .first()
                            .ok_or_else(|| format!("{}() expects a filename", fallback_name))?
                            .show();
                        let path_ref =
                            filesystem_path(global_variables, std::path::Path::new(&path), false);
                        let result = match path_ref.as_deref() {
                            None => Value::Bool(false),
                            Some(path_ref) => match fallback_name.to_ascii_lowercase().as_str() {
                                "file_exists" => Value::Bool(path_ref.exists()),
                                "is_file" => Value::Bool(
                                    std::fs::metadata(path_ref)
                                        .is_ok_and(|metadata| metadata.is_file()),
                                ),
                                "is_dir" => Value::Bool(
                                    std::fs::metadata(path_ref)
                                        .is_ok_and(|metadata| metadata.is_dir()),
                                ),
                                "is_link" => Value::Bool(
                                    std::fs::symlink_metadata(path_ref)
                                        .is_ok_and(|metadata| metadata.file_type().is_symlink()),
                                ),
                                "is_readable" => {
                                    Value::Bool(filesystem_access(path_ref, FilesystemAccess::Read))
                                }
                                "is_writable" => Value::Bool(filesystem_access(
                                    path_ref,
                                    FilesystemAccess::Write,
                                )),
                                "is_executable" => Value::Bool(filesystem_access(
                                    path_ref,
                                    FilesystemAccess::Execute,
                                )),
                                "filesize" => std::fs::metadata(path_ref)
                                    .ok()
                                    .and_then(|metadata| i64::try_from(metadata.len()).ok())
                                    .map(Value::Integer)
                                    .unwrap_or(Value::Bool(false)),
                                "filemtime" | "fileatime" => file_timestamp_value(
                                    fallback_name,
                                    &path,
                                    Some(path_ref),
                                    global_variables,
                                ),
                                "filectime" => file_stat_field_value(
                                    fallback_name,
                                    &path,
                                    "ctime",
                                    global_variables,
                                ),
                                "fileinode" => file_stat_field_value(
                                    fallback_name,
                                    &path,
                                    "ino",
                                    global_variables,
                                ),
                                "fileowner" => file_stat_field_value(
                                    fallback_name,
                                    &path,
                                    "uid",
                                    global_variables,
                                ),
                                "filegroup" => file_stat_field_value(
                                    fallback_name,
                                    &path,
                                    "gid",
                                    global_variables,
                                ),
                                "fileperms" => file_stat_field_value(
                                    fallback_name,
                                    &path,
                                    "mode",
                                    global_variables,
                                ),
                                "filetype" => {
                                    filetype_value(fallback_name, &path, global_variables)
                                }
                                "realpath" => {
                                    Value::String(path_ref.to_string_lossy().into_owned())
                                }
                                _ => unreachable!(),
                            },
                        };
                        stack.push(result);
                    }
                    "getcwd" => {
                        if !arguments.is_empty() {
                            return Err("getcwd() expects exactly 0 arguments".into());
                        }
                        stack.push(
                            std::env::current_dir()
                                .ok()
                                .map(|path| Value::String(path.to_string_lossy().into_owned()))
                                .unwrap_or(Value::Bool(false)),
                        );
                    }
                    "get_include_path" => {
                        if !arguments.is_empty() {
                            return Err("get_include_path() expects exactly 0 arguments".into());
                        }
                        stack.push(Value::String(global_variables.include_path().into()));
                    }
                    "set_include_path" => {
                        let path = arguments
                            .first()
                            .ok_or("set_include_path() expects exactly 1 argument")?
                            .show();
                        stack.push(Value::String(global_variables.set_include_path(path)));
                    }
                    "sys_get_temp_dir" => {
                        if !arguments.is_empty() {
                            return Err("sys_get_temp_dir() expects exactly 0 arguments".into());
                        }
                        stack.push(Value::String(
                            std::env::temp_dir().to_string_lossy().into_owned(),
                        ));
                    }
                    "gettimeofday" => {
                        if arguments.len() > 1 {
                            return Err(format!(
                                "gettimeofday() expects at most 1 argument, {} given",
                                arguments.len()
                            ));
                        }
                        stack.push(gettimeofday_value(
                            arguments.first().is_some_and(Value::truthy),
                        )?);
                    }
                    "time" => {
                        if !arguments.is_empty() {
                            return Err("time() expects exactly 0 arguments".into());
                        }
                        stack.push(Value::Integer(unix_time().0 as i64));
                    }
                    "random_int" => {
                        if arguments.len() != 2 {
                            return Err(format!(
                                "random_int() expects exactly 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let min = random_int_bound("random_int", &arguments[0], 1)?;
                        let max = random_int_bound("random_int", &arguments[1], 2)?;
                        if min > max {
                            return Err("ValueError:random_int(): Argument #2 ($max) must be greater than or equal to Argument #1 ($min)".into());
                        }
                        stack.push(Value::Integer(secure_random_integer(min, max)?));
                    }
                    "rand" | "mt_rand" => {
                        if arguments.len() > 2 {
                            return Err(format!(
                                "{fallback_name}() expects at most 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let min = arguments
                            .first()
                            .map(|value| random_int_bound(&fallback_name, value, 1))
                            .transpose()?
                            .unwrap_or(0);
                        let max = arguments
                            .get(1)
                            .map(|value| random_int_bound(&fallback_name, value, 2))
                            .transpose()?
                            .unwrap_or(2_147_483_647);
                        if min > max {
                            return Err(format!("ValueError:{fallback_name}(): Argument #2 ($max) must be greater than or equal to Argument #1 ($min)"));
                        }
                        stack.push(Value::Integer(secure_random_integer(min, max)?));
                    }
                    "getrandmax" | "mt_getrandmax" => {
                        if !arguments.is_empty() {
                            return Err(format!(
                                "{fallback_name}() expects exactly 0 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        stack.push(Value::Integer(2_147_483_647));
                    }
                    "date" | "gmdate" => {
                        if arguments.is_empty() || arguments.len() > 2 {
                            return Err(format!(
                                "{fallback_name}() expects 1 or 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let format = arguments[0].show();
                        let timestamp = match arguments.get(1) {
                            None | Some(Value::Null) => unix_time().0 as i64,
                            Some(value) => date_timestamp(value)?,
                        };
                        stack.push(Value::String(format_php_date(
                            &format,
                            timestamp,
                            fallback_name.eq_ignore_ascii_case("gmdate"),
                        )?));
                    }
                    "strftime" | "gmstrftime" => {
                        if arguments.is_empty() || arguments.len() > 2 {
                            return Err(format!(
                                "{fallback_name}() expects 1 or 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let function = fallback_name.to_ascii_lowercase();
                        global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Deprecated,
                            message: format!("Function {function}() is deprecated since 8.1, use IntlDateFormatter::format() instead"),
                            source_file: None,
                            source_line: None,
                            output_offset: output.len(),
                        });
                        let format = arguments[0].show();
                        let timestamp = match arguments.get(1) {
                            None | Some(Value::Null) => unix_time().0 as i64,
                            Some(value) => date_timestamp(value)?,
                        };
                        let utc = function == "gmstrftime";
                        stack.push(format_strftime(&format, timestamp, utc));
                    }
                    #[cfg(unix)]
                    "strptime" => {
                        if arguments.len() != 2 {
                            return Err(format!(
                                "strptime() expects exactly 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Deprecated,
                            message: "Function strptime() is deprecated since 8.2, use date_parse_from_format() (for locale-independent parsing), or IntlDateFormatter::parse() (for locale-dependent parsing) instead".into(),
                            source_file: None,
                            source_line: None,
                            output_offset: output.len(),
                        });
                        stack.push(parse_strptime(&arguments[0].show(), &arguments[1].show()));
                    }
                    "getdate" => {
                        if arguments.len() > 1 {
                            return Err(format!(
                                "getdate() expects at most 1 argument, {} given",
                                arguments.len()
                            ));
                        }
                        let timestamp = match arguments.first() {
                            None | Some(Value::Null) => unix_time().0 as i64,
                            Some(value) => date_timestamp(value)?,
                        };
                        stack.push(getdate_value(timestamp)?);
                    }
                    "localtime" => {
                        if arguments.len() > 2 {
                            return Err(format!(
                                "localtime() expects at most 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let timestamp = match arguments.first() {
                            None | Some(Value::Null) => unix_time().0 as i64,
                            Some(value) => date_timestamp(value)?,
                        };
                        stack.push(localtime_value(
                            timestamp,
                            arguments.get(1).is_some_and(Value::truthy),
                        )?);
                    }
                    "strtotime" => {
                        if arguments.is_empty() || arguments.len() > 2 {
                            return Err(format!(
                                "strtotime() expects 1 or 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let datetime = arguments[0].show();
                        let base = match arguments.get(1) {
                            None | Some(Value::Null) => unix_time().0 as i64,
                            Some(value) => date_timestamp(value)?,
                        };
                        stack.push(
                            parse_php_strtotime(&datetime, base)
                                .map(Value::Integer)
                                .unwrap_or(Value::Bool(false)),
                        );
                    }
                    "idate" => {
                        if arguments.is_empty() || arguments.len() > 2 {
                            return Err(format!(
                                "idate() expects 1 or 2 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let format = arguments[0].show();
                        if format.chars().count() != 1 {
                            stack.push(Value::Bool(false));
                        } else {
                            let timestamp = match arguments.get(1) {
                                None | Some(Value::Null) => unix_time().0 as i64,
                                Some(value) => date_timestamp(value)?,
                            };
                            let formatted = format_php_date(&format, timestamp, false)?;
                            stack.push(
                                formatted
                                    .parse::<i64>()
                                    .map(Value::Integer)
                                    .unwrap_or(Value::Bool(false)),
                            );
                        }
                    }
                    "checkdate" => {
                        if arguments.len() != 3 {
                            return Err(format!(
                                "checkdate() expects exactly 3 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let month = date_component(&arguments[0]);
                        let day = date_component(&arguments[1]);
                        let year = date_component(&arguments[2]);
                        let valid = month.is_some_and(|month| (1..=12).contains(&month))
                            && day.is_some_and(|day| day >= 1)
                            && year.is_some_and(|year| (1..=32767).contains(&year));
                        let valid = valid
                            && month
                                .zip(day)
                                .zip(year)
                                .is_some_and(|((month, day), year)| {
                                    let leap =
                                        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                                    let days = [
                                        31,
                                        if leap { 29 } else { 28 },
                                        31,
                                        30,
                                        31,
                                        30,
                                        31,
                                        31,
                                        30,
                                        31,
                                        30,
                                        31,
                                    ];
                                    day <= days[(month - 1) as usize]
                                });
                        stack.push(Value::Bool(valid));
                    }
                    "mktime" | "gmmktime" => {
                        if arguments.len() > 6 {
                            return Err(format!(
                                "{fallback_name}() expects at most 6 arguments, {} given",
                                arguments.len()
                            ));
                        }
                        let utc = fallback_name.eq_ignore_ascii_case("gmmktime");
                        let now = unix_time().0 as i64;
                        let current = date_tm(now, utc)?;
                        let defaults = [
                            current.tm_hour as i64,
                            current.tm_min as i64,
                            current.tm_sec as i64,
                            (current.tm_mon + 1) as i64,
                            current.tm_mday as i64,
                            (current.tm_year + 1900) as i64,
                        ];
                        let mut parts = defaults;
                        for (index, value) in arguments.iter().enumerate() {
                            if !matches!(value.dereference(), Value::Null) {
                                parts[index] = date_component(value).ok_or_else(|| format!("TypeError:{fallback_name}(): Argument #{} must be of type ?int", index + 1))?;
                            }
                        }
                        let year = match parts[5] {
                            0..=69 => parts[5] + 2000,
                            70..=100 => parts[5] + 1900,
                            _ => parts[5],
                        };
                        let mut tm = libc::tm {
                            tm_sec: parts[2] as libc::c_int,
                            tm_min: parts[1] as libc::c_int,
                            tm_hour: parts[0] as libc::c_int,
                            tm_mday: parts[4] as libc::c_int,
                            tm_mon: (parts[3] - 1) as libc::c_int,
                            tm_year: (year - 1900) as libc::c_int,
                            tm_wday: 0,
                            tm_yday: 0,
                            tm_isdst: if utc { 0 } else { -1 },
                            #[cfg(target_os = "linux")]
                            tm_gmtoff: 0,
                            #[cfg(target_os = "linux")]
                            tm_zone: std::ptr::null(),
                        };
                        let timestamp = unsafe {
                            if utc {
                                libc::timegm(&mut tm)
                            } else {
                                libc::mktime(&mut tm)
                            }
                        };
                        stack.push(Value::Integer(timestamp as i64));
                    }
                    "microtime" => {
                        let (seconds, microseconds) = unix_time();
                        if arguments.first().is_some_and(Value::truthy) {
                            stack.push(Value::Number(
                                seconds as f64 + microseconds as f64 / 1_000_000.0,
                            ));
                        } else {
                            stack.push(Value::String(format!("0.{microseconds:06}00 {seconds}")));
                        }
                    }
                    "hrtime" => {
                        let (seconds, nanoseconds) = high_resolution_time();
                        if arguments.first().is_some_and(Value::truthy) {
                            stack.push(
                                i64::try_from(
                                    seconds
                                        .saturating_mul(1_000_000_000)
                                        .saturating_add(nanoseconds as u64),
                                )
                                .map(Value::Integer)
                                .unwrap_or(Value::Number(
                                    seconds as f64 * 1_000_000_000.0 + nanoseconds as f64,
                                )),
                            );
                        } else {
                            stack.push(Value::from_array_entries(vec![
                                (ArrayKey::Integer(0), Value::Integer(seconds as i64)),
                                (ArrayKey::Integer(1), Value::Integer(nanoseconds as i64)),
                            ]));
                        }
                    }
                    "file_get_contents" => {
                        let path = arguments
                            .first()
                            .ok_or("file_get_contents() expects a filename")?
                            .show();
                        let use_include_path = arguments.get(1).is_some_and(Value::truthy);
                        let contents = if path == "php://input" {
                            Some(
                                global_variables
                                    .get("\0php_input")
                                    .cloned()
                                    .unwrap_or_else(|| Value::String(String::new()))
                                    .show()
                                    .into_bytes(),
                            )
                        } else {
                            filesystem_lookup_path(
                                global_variables,
                                std::path::Path::new(&path),
                                false,
                                use_include_path,
                            )
                            .and_then(|path| std::fs::read(path).ok())
                        };
                        if let Some(bytes) = contents {
                            let requested_offset = arguments
                                .get(3)
                                .map(|value| value.number() as i64)
                                .unwrap_or(0);
                            let start = if requested_offset < 0 {
                                (bytes.len() as i64).saturating_add(requested_offset).max(0)
                                    as usize
                            } else {
                                requested_offset as usize
                            };
                            if start > bytes.len() {
                                stack.push(Value::Bool(false));
                            } else {
                                let available = bytes.len() - start;
                                let length = arguments
                                    .get(4)
                                    .filter(|value| !matches!(value.dereference(), Value::Null))
                                    .map(|value| value.number().max(0.0) as usize)
                                    .unwrap_or(available)
                                    .min(available);
                                stack.push(Value::String(
                                    String::from_utf8_lossy(&bytes[start..start + length])
                                        .into_owned(),
                                ));
                            }
                        } else {
                            stack.push(Value::Bool(false));
                        }
                    }
                    "file" => {
                        let path = arguments.first().ok_or("file() expects a filename")?.show();
                        let flags = arguments
                            .get(1)
                            .map(|value| value.number() as i64)
                            .unwrap_or(0);
                        if let Some(path) = filesystem_lookup_path(
                            global_variables,
                            std::path::Path::new(&path),
                            false,
                            flags & 1 != 0,
                        ) {
                            if let Ok(contents) = std::fs::read(path) {
                                let contents = String::from_utf8_lossy(&contents);
                                let mut lines = Vec::new();
                                if !contents.is_empty() {
                                    for line in contents.split_inclusive('\n') {
                                        let mut line = line.to_owned();
                                        let empty = line.trim_end_matches(['\r', '\n']).is_empty();
                                        if flags & 4 != 0 && empty {
                                            continue;
                                        }
                                        if flags & 2 != 0 {
                                            if line.ends_with('\n') {
                                                line.pop();
                                            }
                                            if line.ends_with('\r') {
                                                line.pop();
                                            }
                                        }
                                        lines.push((
                                            ArrayKey::Integer(lines.len() as i64),
                                            Value::String(line),
                                        ));
                                    }
                                }
                                stack.push(Value::from_array_entries(lines));
                            } else {
                                stack.push(Value::Bool(false));
                            }
                        } else {
                            stack.push(Value::Bool(false));
                        }
                    }
                    "file_put_contents" => {
                        let path = arguments
                            .first()
                            .ok_or("file_put_contents() expects a filename")?
                            .show();
                        let Some(data) = arguments.get(1) else {
                            return Err("file_put_contents() expects data".into());
                        };
                        if let Some(bytes) = file_put_contents_bytes(data) {
                            let flags = arguments
                                .get(2)
                                .map(|value| value.number() as i64)
                                .unwrap_or(0);
                            let result = filesystem_path(
                                global_variables,
                                std::path::Path::new(&path),
                                true,
                            )
                            .and_then(|path| write_file_contents(&path, &bytes, flags).ok())
                            .and_then(|written| i64::try_from(written).ok())
                            .map(Value::Integer)
                            .unwrap_or(Value::Bool(false));
                            stack.push(result);
                        } else {
                            stack.push(Value::Bool(false));
                        }
                    }
                    "copy" => {
                        stack.push(copy_value(&arguments, global_variables, output)?);
                    }
                    "rename" => {
                        stack.push(rename_value(&arguments, global_variables, output)?);
                    }
                    "mkdir" => {
                        stack.push(mkdir_value(&arguments, global_variables, output)?);
                    }
                    "rmdir" => {
                        stack.push(rmdir_value(&arguments, global_variables, output)?);
                    }
                    "tempnam" => {
                        let directory = arguments
                            .first()
                            .ok_or("tempnam() expects a directory")?
                            .show();
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
                        stack.push(
                            directory
                                .and_then(|directory| {
                                    create_temporary_file(&directory, &prefix).ok()
                                })
                                .map(|path| Value::String(path.to_string_lossy().into_owned()))
                                .unwrap_or(Value::Bool(false)),
                        );
                    }
                    "unlink" => {
                        let path = arguments
                            .first()
                            .ok_or("unlink() expects a filename")?
                            .show();
                        let removed =
                            filesystem_path(global_variables, std::path::Path::new(&path), false)
                                .is_some_and(|path| std::fs::remove_file(path).is_ok());
                        stack.push(Value::Bool(removed));
                    }
                    "touch" => {
                        stack.push(touch_value(&arguments, global_variables, output)?);
                    }
                    "chmod" => {
                        stack.push(chmod_value(&arguments, global_variables, output)?);
                    }
                    "disk_free_space" | "diskfreespace" | "disk_total_space" => {
                        stack.push(disk_space_value(
                            fallback_name,
                            &arguments,
                            global_variables,
                            output,
                        )?);
                    }
                    "fopen" => {
                        let path = arguments
                            .first()
                            .ok_or("fopen() expects at least 2 arguments")?
                            .show();
                        let mode = arguments
                            .get(1)
                            .ok_or("fopen() expects at least 2 arguments")?
                            .show();
                        let use_include_path = arguments.get(2).is_some_and(Value::truthy);
                        let stream = if path == "php://temp" || path == "php://memory" {
                            let Some(first) = mode.chars().next() else {
                                return Err("fopen(): invalid stream mode".into());
                            };
                            if !matches!(first, 'r' | 'w' | 'a' | 'x' | 'c')
                                || mode
                                    .chars()
                                    .skip(1)
                                    .any(|character| !matches!(character, '+' | 'b' | 't'))
                            {
                                return Err("fopen(): invalid stream mode".into());
                            }
                            let readable = first == 'r' || mode.contains('+');
                            let writable = first != 'r' || mode.contains('+');
                            crate::PhpStream::temporary(readable, writable).ok()
                        } else {
                            let writable = mode.chars().next().is_some_and(|mode| mode != 'r')
                                || mode.contains('+');
                            filesystem_lookup_path(
                                global_variables,
                                std::path::Path::new(&path),
                                writable,
                                use_include_path,
                            )
                            .and_then(|path| crate::PhpStream::open_file(path, &mode).ok())
                        };
                        stack.push(
                            stream
                                .map(|stream| {
                                    Value::Stream(std::rc::Rc::new(std::cell::RefCell::new(stream)))
                                })
                                .unwrap_or(Value::Bool(false)),
                        );
                    }
                    "fwrite" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference)
                        else {
                            return Err("fwrite() expects a stream resource".into());
                        };
                        let bytes = arguments
                            .get(1)
                            .ok_or("fwrite() expects at least 2 arguments")?
                            .show()
                            .into_bytes();
                        let length = arguments
                            .get(2)
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(|value| value.number().max(0.0) as usize)
                            .unwrap_or(bytes.len());
                        let written = stream
                            .borrow_mut()
                            .write(&bytes[..bytes.len().min(length)])
                            .map_err(|error| format!("fwrite(): {error}"))?;
                        stack.push(Value::Integer(written as i64));
                    }
                    "fread" | "stream_get_contents" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference)
                        else {
                            return Err(format!("{}() expects a stream resource", fallback_name));
                        };
                        if fallback_name.eq_ignore_ascii_case("fread") && arguments.get(1).is_none()
                        {
                            return Err("fread() expects a length".into());
                        }
                        let offset = if fallback_name.eq_ignore_ascii_case("stream_get_contents") {
                            arguments
                                .get(2)
                                .map(|value| value.number() as i64)
                                .unwrap_or(-1)
                        } else {
                            -1
                        };
                        if offset >= 0 {
                            stream
                                .borrow_mut()
                                .seek(std::io::SeekFrom::Current(offset))
                                .map_err(|error| format!("stream_get_contents(): {error}"))?;
                        }
                        let length = arguments
                            .get(1)
                            .map(|value| value.number() as i64)
                            .filter(|length| *length >= 0)
                            .map(|length| length as usize);
                        let bytes = stream
                            .borrow_mut()
                            .read(length)
                            .map_err(|error| format!("{}(): {error}", fallback_name))?;
                        stack.push(Value::String(String::from_utf8_lossy(&bytes).into_owned()));
                    }
                    "rewind" | "fseek" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference)
                        else {
                            return Err(format!("{}() expects a stream resource", fallback_name));
                        };
                        let result = if fallback_name.eq_ignore_ascii_case("rewind") {
                            stream.borrow_mut().seek(std::io::SeekFrom::Start(0))
                        } else {
                            let offset = arguments
                                .get(1)
                                .map(|value| value.number() as i64)
                                .ok_or("fseek() expects an offset")?;
                            match arguments
                                .get(2)
                                .map(|value| value.number() as i64)
                                .unwrap_or(0)
                            {
                                0 if offset >= 0 => stream
                                    .borrow_mut()
                                    .seek(std::io::SeekFrom::Start(offset as u64)),
                                1 => stream.borrow_mut().seek(std::io::SeekFrom::Current(offset)),
                                2 => stream.borrow_mut().seek(std::io::SeekFrom::End(offset)),
                                _ => Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "invalid whence or negative absolute offset",
                                )),
                            }
                        };
                        if fallback_name.eq_ignore_ascii_case("rewind") {
                            stack.push(Value::Bool(result.is_ok()));
                        } else {
                            stack.push(Value::Integer(if result.is_ok() { 0 } else { -1 }));
                        }
                    }
                    "ftell" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference)
                        else {
                            return Err("ftell() expects a stream resource".into());
                        };
                        let position = stream
                            .borrow_mut()
                            .seek(std::io::SeekFrom::Current(0))
                            .map_err(|error| format!("ftell(): {error}"))?;
                        stack.push(Value::Integer(position as i64));
                    }
                    "feof" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference)
                        else {
                            return Err("feof() expects a stream resource".into());
                        };
                        let mut stream = stream.borrow_mut();
                        let position = stream
                            .seek(std::io::SeekFrom::Current(0))
                            .map_err(|error| format!("feof(): {error}"))?;
                        let end = stream
                            .seek(std::io::SeekFrom::End(0))
                            .map_err(|error| format!("feof(): {error}"))?;
                        stream
                            .seek(std::io::SeekFrom::Start(position))
                            .map_err(|error| format!("feof(): {error}"))?;
                        stack.push(Value::Bool(position >= end));
                    }
                    "fclose" => {
                        let Some(Value::Stream(stream)) = arguments.first().map(Value::dereference)
                        else {
                            return Err("fclose() expects a stream resource".into());
                        };
                        stack.push(Value::Bool(stream.borrow_mut().close().is_ok()));
                    }
                    "json_encode" => stack.push(json_encode_builtin(
                        &arguments,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?),
                    "json_decode" => stack.push(json_decode_builtin(
                        &arguments,
                        classes,
                        global_variables,
                        steps,
                    )?),
                    "json_validate" => {
                        stack.push(json_validate_builtin(&arguments, global_variables)?)
                    }
                    "json_last_error" => {
                        if !arguments.is_empty() {
                            return Err("json_last_error() expects exactly 0 arguments".into());
                        }
                        stack.push(Value::Integer(global_variables.json_last_error));
                    }
                    "json_last_error_msg" => {
                        if !arguments.is_empty() {
                            return Err("json_last_error_msg() expects exactly 0 arguments".into());
                        }
                        stack.push(Value::String(
                            json_error_message(global_variables.json_last_error).into(),
                        ));
                    }
                    "is_uploaded_file" => {
                        let path = arguments
                            .first()
                            .ok_or("is_uploaded_file() expects a filename")?
                            .show();
                        let allowed = global_variables
                            .get("\0upload_paths")
                            .map(Value::show)
                            .unwrap_or_default();
                        stack.push(Value::Bool(
                            allowed.lines().any(|candidate| candidate == path)
                                && PathBuf::from(path).is_file(),
                        ));
                    }
                    "move_uploaded_file" => {
                        let source = arguments
                            .first()
                            .ok_or("move_uploaded_file() expects a source filename")?
                            .show();
                        let destination = arguments
                            .get(1)
                            .ok_or("move_uploaded_file() expects a destination filename")?
                            .show();
                        let allowed = global_variables
                            .get("\0upload_paths")
                            .map(Value::show)
                            .unwrap_or_default();
                        let root = global_variables
                            .get("\0document_root")
                            .map(Value::show)
                            .unwrap_or_default();
                        let source_path = PathBuf::from(&source);
                        let root_path = PathBuf::from(root);
                        let destination_path = PathBuf::from(destination);
                        let destination_path = if destination_path.is_absolute() {
                            destination_path
                        } else {
                            root_path.join(destination_path)
                        };
                        let moved = if allowed.lines().any(|candidate| candidate == source)
                            && source_path.is_file()
                            && destination_path.file_name().is_some()
                        {
                            destination_path
                                .parent()
                                .and_then(|parent| parent.canonicalize().ok())
                                .filter(|parent| parent.starts_with(&root_path))
                                .map(|parent| parent.join(destination_path.file_name().unwrap()))
                                .is_some_and(|destination| {
                                    std::fs::rename(&source_path, &destination).is_ok()
                                })
                        } else {
                            false
                        };
                        if moved {
                            let remaining = allowed
                                .lines()
                                .filter(|candidate| *candidate != source)
                                .collect::<Vec<_>>()
                                .join("\n");
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
                        global_variables
                            .set("_SESSION".into(), Value::from_array_entries(Vec::new()));
                        stack.push(Value::Bool(true));
                    }
                    "session_unset" => {
                        global_variables
                            .set("_SESSION".into(), Value::from_array_entries(Vec::new()));
                        stack.push(Value::Null);
                    }
                    "session_status" => {
                        stack.push(Value::Integer(if global_variables.session_started {
                            2
                        } else {
                            1
                        }));
                    }
                    "session_id" => {
                        let current = global_variables
                            .get("\0session_id")
                            .map(Value::show)
                            .unwrap_or_default();
                        if let Some(id) = arguments
                            .first()
                            .filter(|value| !matches!(value, Value::Null))
                        {
                            let id = id.show();
                            if id.len() > 128
                                || !id.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || b"-_".contains(&byte)
                                })
                            {
                                return Err("session_id(): invalid session identifier".into());
                            }
                            global_variables.set("\0session_id".into(), Value::String(id));
                        }
                        stack.push(Value::String(current));
                    }
                    "parse_str" => {
                        let (mut arguments, by_reference) = super::builtin_arguments::bind(
                            fallback_name,
                            arguments,
                            &by_reference,
                            &argument_names,
                        )?;
                        if arguments.len() != 2 {
                            return Err(format!("ArgumentCountError:parse_str() expects exactly 2 arguments, {} given", arguments.len()));
                        }
                        coerce_stringable_builtin_args(
                            fallback_name,
                            &mut arguments,
                            crate::string_builtins::string_argument_positions(fallback_name),
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        let target = by_reference.get(1).and_then(Option::as_ref)
                            .ok_or("Error:parse_str(): Argument #2 ($result) cannot be passed by reference")?;
                        let parsed = crate::string_builtins::call(fallback_name, &arguments)?;
                        write_reference_argument(
                            target,
                            parsed,
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                        )?;
                        stack.push(Value::Null);
                    }
                    "printf" | "vprintf" | "vsprintf" => {
                        let (arguments, _) = super::builtin_arguments::bind(
                            fallback_name,
                            arguments,
                            &by_reference,
                            &argument_names,
                        )?;
                        let text = if fallback_name.eq_ignore_ascii_case("printf") {
                            if arguments.is_empty() {
                                return Err("ArgumentCountError:printf() expects at least 1 argument, 0 given".into());
                            }
                            crate::string_builtins::format_values(
                                &arguments[0].show(),
                                &arguments[1..],
                            )?
                        } else {
                            let formatter = if fallback_name.eq_ignore_ascii_case("vprintf") {
                                "vsprintf"
                            } else {
                                fallback_name
                            };
                            match crate::string_builtins::call(formatter, &arguments)? {
                                Value::String(text) => text,
                                _ => unreachable!("formatted string builtin returns a string"),
                            }
                        };
                        if fallback_name.eq_ignore_ascii_case("vsprintf") {
                            stack.push(Value::String(text));
                        } else {
                            let bytes = text.len();
                            output.push_str(&text);
                            stack.push(Value::Integer(bytes as i64));
                        }
                    }
                    "fprintf" => {
                        let (arguments, _) = super::builtin_arguments::bind(
                            fallback_name,
                            arguments,
                            &by_reference,
                            &argument_names,
                        )?;
                        if arguments.len() < 2 {
                            return Err(
                                "ArgumentCountError:fprintf() expects at least 2 arguments".into(),
                            );
                        }
                        let Value::Stream(stream) = arguments[0].dereference() else {
                            return Err("fprintf() expects a stream resource".into());
                        };
                        let text = crate::string_builtins::format_values(
                            &arguments[1].show(),
                            &arguments[2..],
                        )?;
                        let written = stream
                            .borrow_mut()
                            .write(text.as_bytes())
                            .map_err(|error| format!("fprintf(): {error}"))?;
                        stack.push(Value::Integer(written as i64));
                    }
                    "sprintf" => {
                        coerce_stringable_builtin_args(
                            fallback_name,
                            &mut arguments,
                            crate::string_builtins::string_argument_positions(fallback_name),
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        stack.push(crate::string_builtins::call(fallback_name, &arguments)?);
                    }
                    "basename"
                    | "dirname"
                    | "pathinfo"
                    | "strip_tags"
                    | "urlencode"
                    | "urldecode"
                    | "rawurlencode"
                    | "rawurldecode"
                    | "strtolower"
                    | "strtoupper"
                    | "strlen"
                    | "ucfirst"
                    | "lcfirst"
                    | "strrev"
                    | "str_shuffle"
                    | "str_rot13"
                    | "addslashes"
                    | "addcslashes"
                    | "stripslashes"
                    | "stripcslashes"
                    | "bin2hex"
                    | "hex2bin"
                    | "base64_encode"
                    | "base64_decode"
                    | "chr"
                    | "count_chars"
                    | "ord"
                    | "crc32"
                    | "md5"
                    | "sha1"
                    | "hash"
                    | "hash_algos"
                    | "hash_hmac_algos"
                    | "hash_pbkdf2"
                    | "hash_hkdf"
                    | "hash_equals"
                    | "hash_hmac"
                    | "number_format"
                    | "str_increment"
                    | "str_decrement"
                    | "strspn"
                    | "strcspn"
                    | "strpbrk"
                    | "str_word_count"
                    | "strcmp"
                    | "strcasecmp"
                    | "strncmp"
                    | "strncasecmp"
                    | "strnatcmp"
                    | "strnatcasecmp"
                    | "strtr"
                    | "ucwords"
                    | "trim"
                    | "ltrim"
                    | "rtrim"
                    | "str_repeat"
                    | "str_pad"
                    | "str_split"
                    | "strpos"
                    | "stripos"
                    | "strrpos"
                    | "strripos"
                    | "substr_count"
                    | "substr_compare"
                    | "strtok"
                    | "levenshtein"
                    | "substr"
                    | "substr_replace"
                    | "explode"
                    | "implode"
                    | "str_getcsv"
                    | "htmlspecialchars"
                    | "htmlentities"
                    | "html_entity_decode"
                    | "get_html_translation_table"
                    | "htmlspecialchars_decode"
                    | "nl2br"
                    | "wordwrap"
                    | "str_contains"
                    | "str_starts_with"
                    | "str_ends_with"
                    | "strstr"
                    | "strchr"
                    | "stristr" => {
                        coerce_stringable_builtin_args(
                            fallback_name,
                            &mut arguments,
                            crate::string_builtins::string_argument_positions(fallback_name),
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        let result = match fallback_name.to_ascii_lowercase().as_str() {
                            "hex2bin" => hex2bin_with_diagnostic(&arguments, global_variables)?,
                            "chr" => chr_with_diagnostic(&arguments, global_variables)?,
                            _ => crate::string_builtins::call(fallback_name, &arguments)?,
                        };
                        stack.push(result);
                    }
                    "str_replace" | "str_ireplace" => {
                        coerce_stringable_builtin_args(
                            fallback_name,
                            &mut arguments,
                            crate::string_builtins::string_argument_positions(fallback_name),
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        let (value, count) =
                            crate::string_builtins::replace(fallback_name, &arguments)?;
                        if let Some(Some(target)) = by_reference.get(3) {
                            write_reference_argument(
                                target,
                                Value::Integer(count as i64),
                                variables,
                                bindings,
                                global_variables,
                                global_scope,
                            )?;
                        }
                        stack.push(value);
                    }
                    "define" => {
                        if arguments.len() < 2 || arguments.len() > 3 {
                            return Err("define() expects between 2 and 3 arguments".into());
                        }
                        let name = arguments[0].show();
                        if name.is_empty() || name.contains('\0') {
                            return Err("define(): constant name must be a non-empty string".into());
                        }
                        if arguments.get(2).is_some_and(Value::truthy) {
                            return Err(
                                "define(): case-insensitive constants are not supported".into()
                            );
                        }
                        let name = name.trim_start_matches('\\').to_string();
                        if super::constant_exists(&name, global_variables) {
                            stack.push(Value::Bool(false));
                        } else {
                            global_variables
                                .constants
                                .insert(name, arguments[1].clone());
                            stack.push(Value::Bool(true));
                        }
                    }
                    "defined" => {
                        let name = arguments
                            .first()
                            .ok_or("defined() expects exactly 1 argument")?
                            .show();
                        stack.push(Value::Bool(super::constant_exists(&name, global_variables)));
                    }
                    "constant" => {
                        let name = arguments
                            .first()
                            .ok_or("constant() expects exactly 1 argument")?
                            .show();
                        stack.push(super::read_constant(&name, global_variables)?);
                    }
                    "compact" => {
                        stack.push(super::symbol_table::compact(
                            &arguments,
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                        )?);
                    }
                    "get_defined_vars" => {
                        if !arguments.is_empty() {
                            return Err("get_defined_vars() expects exactly 0 arguments".into());
                        }
                        stack.push(super::symbol_table::get_defined_vars(
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                        ));
                    }
                    "extract" => {
                        stack.push(super::symbol_table::extract(
                            &arguments,
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                            by_reference.first().and_then(Option::as_ref),
                        )?);
                    }
                    _ => {
                        let array_arguments =
                            arguments.iter().map(Value::dereference).collect::<Vec<_>>();
                        let mut array_diagnostics = Vec::new();
                        let normalized_array_name = fallback_name.to_ascii_lowercase();
                        let dispatch_result = if fallback_name.eq_ignore_ascii_case("array_walk")
                            || fallback_name.eq_ignore_ascii_case("array_walk_recursive")
                        {
                            let mut no_positional_callback = |_: &Value, _: Vec<Value>| {
                                Err(format!(
                                    "Unexpected positional callback for {fallback_name}"
                                ))
                            };
                            let mut walk_index = 0usize;
                            let mut walk_reference_warning_emitted = false;
                            let userdata_passed = array_arguments.len() > 2;
                            let mut walk_invoker =
                                |callback: &Value,
                                 value: Value,
                                 key: Value,
                                 user_data: Option<Value>| {
                                    if !walk_reference_warning_emitted {
                                        let references = callable_parameter_references(
                                            callback, functions, classes,
                                        );
                                        if references.get(1).copied().unwrap_or(false)
                                            || (userdata_passed
                                                && references.get(2).copied().unwrap_or(false))
                                        {
                                            global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                    level: crate::DiagnosticLevel::Warning,
                                    message: format!("{fallback_name}(): callback expects the key or userdata parameter to be passed by reference"),
                                    source_file: None, source_line: None, output_offset: 0,
                                });
                                            walk_reference_warning_emitted = true;
                                        }
                                    }
                                    let temp_name =
                                        format!("\0array_walk:{}:{}", frame_id, walk_index);
                                    walk_index += 1;
                                    invoke_array_walk_callback(
                                        callback,
                                        value,
                                        key,
                                        user_data,
                                        temp_name,
                                        fallback_name,
                                        frame_id,
                                        functions,
                                        classes,
                                        static_properties,
                                        global_variables,
                                        output,
                                        steps,
                                    )
                                };
                            let mut stringify = |value: &Value| Ok(value.show());
                            crate::array_builtins::call(
                                fallback_name,
                                array_arguments,
                                classes,
                                &mut no_positional_callback,
                                &mut walk_invoker,
                                &mut stringify,
                                &mut array_diagnostics,
                            )
                        } else {
                            let mut unused_walk =
                                |_: &Value, _: Value, _: Value, _: Option<Value>| {
                                    Err("Unexpected array walk callback".into())
                                };
                            if matches!(
                                fallback_name,
                                "sort"
                                    | "rsort"
                                    | "asort"
                                    | "arsort"
                                    | "ksort"
                                    | "krsort"
                                    | "natsort"
                                    | "natcasesort"
                                    | "array_unique"
                                    | "array_multisort"
                            ) {
                                let mut unused_callback = |_: &Value, _: Vec<Value>| {
                                    Err("Unexpected positional callback for sorting".into())
                                };
                                let mut stringify = |value: &Value| {
                                    super::objects::stringify(
                                        value,
                                        functions,
                                        classes,
                                        static_properties,
                                        global_variables,
                                        output,
                                        steps,
                                    )
                                };
                                crate::array_builtins::call(
                                    fallback_name,
                                    array_arguments,
                                    classes,
                                    &mut unused_callback,
                                    &mut unused_walk,
                                    &mut stringify,
                                    &mut array_diagnostics,
                                )
                            } else if matches!(
                                fallback_name,
                                "array_diff"
                                    | "array_intersect"
                                    | "array_diff_assoc"
                                    | "array_intersect_assoc"
                                    | "array_diff_key"
                                    | "array_intersect_key"
                            ) {
                                let mut unused_callback = |_: &Value, _: Vec<Value>| {
                                    Err("Unexpected positional callback for array diff/intersection".into())
                                };
                                let mut stringify = |value: &Value| {
                                    super::objects::stringify(
                                        value,
                                        functions,
                                        classes,
                                        static_properties,
                                        global_variables,
                                        output,
                                        steps,
                                    )
                                };
                                crate::array_builtins::call(
                                    fallback_name,
                                    array_arguments,
                                    classes,
                                    &mut unused_callback,
                                    &mut unused_walk,
                                    &mut stringify,
                                    &mut array_diagnostics,
                                )
                            } else {
                                let mut reference_warning_emitted = false;
                                let runtime_state = std::cell::RefCell::new((
                                    &mut *static_properties,
                                    &mut *global_variables,
                                    &mut *output,
                                    &mut *steps,
                                ));
                                let mut callback_invoker =
                                    |callback: &Value, callback_args: Vec<Value>| {
                                        let (static_properties, global_variables, output, steps) =
                                            &mut *runtime_state.borrow_mut();
                                        if !reference_warning_emitted
                                            && callable_parameter_references(
                                                callback, functions, classes,
                                            )
                                            .iter()
                                            .any(|by_ref| *by_ref)
                                        {
                                            global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                    level: crate::DiagnosticLevel::Warning,
                                    message: format!("{fallback_name}(): callback expects a parameter to be passed by reference"),
                                    source_file: None, source_line: None, output_offset: 0,
                                });
                                            reference_warning_emitted = true;
                                        }
                                        invoke_array_callback(
                                            callback,
                                            callback_args
                                                .into_iter()
                                                .map(|value| value.dereference())
                                                .collect(),
                                            variables,
                                            bindings,
                                            global_scope,
                                            frame_id,
                                            functions,
                                            classes,
                                            static_properties,
                                            global_variables,
                                            output,
                                            steps,
                                        )
                                    };
                                let mut stringify = |value: &Value| {
                                    let (static_properties, global_variables, output, steps) =
                                        &mut *runtime_state.borrow_mut();
                                    super::objects::stringify(
                                        value,
                                        functions,
                                        classes,
                                        static_properties,
                                        global_variables,
                                        output,
                                        steps,
                                    )
                                };
                                let result = crate::array_builtins::call(
                                    fallback_name,
                                    array_arguments,
                                    classes,
                                    &mut callback_invoker,
                                    &mut unused_walk,
                                    &mut stringify,
                                    &mut array_diagnostics,
                                );
                                drop(callback_invoker);
                                drop(runtime_state);
                                result
                            }
                        };
                        for diagnostic in array_diagnostics {
                            global_variables.report_diagnostic(diagnostic);
                        }
                        let (result, mutation) = dispatch_result?;
                        for (argument_index, updated_value) in mutation {
                            let Some(Some(target)) = by_reference.get(argument_index) else {
                                let parameter = match normalized_array_name.as_str() {
                                    "array_pop" | "array_shift" | "array_push"
                                    | "array_unshift" | "array_splice" | "shuffle" | "natsort"
                                    | "natcasesort" | "sort" | "rsort" | "asort" | "arsort"
                                    | "ksort" | "krsort" | "usort" | "uasort" | "uksort"
                                    | "array_multisort" => "array",
                                    _ => "",
                                };
                                let parameter = if parameter.is_empty() {
                                    String::new()
                                } else {
                                    format!(" (${parameter})")
                                };
                                return Err(format!("{fallback_name}(): Argument #{}{parameter} cannot be passed by reference", argument_index + 1));
                            };
                            write_reference_argument(
                                target,
                                updated_value,
                                variables,
                                bindings,
                                global_variables,
                                global_scope,
                            )?;
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
                    let key = reference_storage_key(
                        name,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    );
                    reference_captures.insert(name.clone(), key);
                } else {
                    captured_values.insert(
                        name.clone(),
                        read_variable(name, variables, bindings, global_variables, global_scope),
                    );
                }
            }
            stack.push(Value::Closure(Rc::new(std::cell::RefCell::new(
                PhpClosure {
                    function: (**function).clone(),
                    captures: captured_values,
                    capture_order,
                    reference_captures,
                    callable_target: None,
                    callable_access_scope: None,
                },
            ))));
        }
        Op::MakeFunctionCallable(name) => {
            stack.push(make_callable_closure(
                Value::String(name.clone()),
                functions,
                classes,
            )?);
        }
        Op::MakeStaticCallable(class, method) => {
            let target = Value::String(format!("{class}::{method}"));
            stack.push(make_callable_closure(target, functions, classes)?);
        }
        Op::MakeLateStaticCallable(method) => {
            let class = variables
                .get("\0static_class")
                .map(Value::show)
                .or_else(|| {
                    variables
                        .get("this")
                        .and_then(|value| match value.dereference() {
                            Value::Object(object) => Some(object.borrow().class_name.clone()),
                            _ => None,
                        })
                })
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
            let (arguments, reference_names, argument_names) =
                take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            if stack.is_empty() {
                return Err("Invalid bytecode stack".into());
            }
            let callable = stack.pop().ok_or("Invalid bytecode stack")?;
            let result = match &callable {
                Value::Closure(closure) => invoke_closure(
                    closure,
                    arguments,
                    &reference_names,
                    &argument_names,
                    variables,
                    bindings,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    *strict_types,
                )?,
                Value::Array(_) => invoke_array_callable(
                    &callable,
                    arguments,
                    &reference_names,
                    &argument_names,
                    variables,
                    bindings,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    *strict_types,
                )?,
                Value::Object(_) => invoke_object_callable(
                    &callable,
                    arguments,
                    &reference_names,
                    &argument_names,
                    variables,
                    bindings,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    *strict_types,
                )?,
                Value::String(name) => {
                    let packed_arguments = pack_callable_arguments(
                        arguments,
                        &reference_names,
                        &argument_names,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )?;
                    let (instruction, mut callback_stack) = callback_call(
                        Value::String(name.clone()),
                        packed_arguments,
                        *strict_types,
                    )?;
                    execute_instruction(
                        &instruction,
                        &mut callback_stack,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?;
                    callback_stack
                        .pop()
                        .ok_or("Invalid first-class callable result stack")?
                }
                _ => return Err("Value is not callable".into()),
            };
            stack.push(result);
        }
        Op::CallMethod(
            method,
            _argument_count,
            reference_names,
            argument_names,
            unpack,
            access_scope,
            strict_types,
        ) => {
            let (mut arguments, mut reference_names, argument_names) =
                take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let method_display = if method == "\0dynamic" {
                stack.pop().ok_or("Missing dynamic method name")?.show()
            } else {
                method.clone()
            };
            let method = method_display.to_ascii_lowercase();
            if stack.is_empty() {
                return Err("Invalid bytecode stack".into());
            }
            let object = stack.pop().ok_or("Invalid bytecode stack")?;
            if let Value::Generator(generator) = &object {
                if matches!(
                    method.as_str(),
                    "current"
                        | "key"
                        | "valid"
                        | "rewind"
                        | "next"
                        | "send"
                        | "throw"
                        | "getreturn"
                ) {
                    let method_signature = format!("generator::{method}");
                    arguments = super::builtin_arguments::bind(
                        &method_signature,
                        arguments,
                        &reference_names,
                        &argument_names,
                    )?
                    .0;
                }
                stack.push(super::call_generator_method(
                    generator,
                    &method,
                    arguments,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                )?);
                return Ok(true);
            }
            let Value::Object(object) = object else {
                let value = object.dereference();
                return Err(format!(
                    "Error:Call to a member function {method_display}() on {}",
                    value.type_name(),
                ));
            };
            let class_name = object.borrow().class_name.clone();
            if class_name.eq_ignore_ascii_case("ReflectionClass")
                || class_name.eq_ignore_ascii_case("ReflectionObject")
                || class_name.eq_ignore_ascii_case("ReflectionClassConstant")
                || class_name.eq_ignore_ascii_case("ReflectionConstant")
                || class_name.eq_ignore_ascii_case("ReflectionEnum")
                || class_name.eq_ignore_ascii_case("ReflectionEnumUnitCase")
                || class_name.eq_ignore_ascii_case("ReflectionEnumBackedCase")
                || class_name.eq_ignore_ascii_case("ReflectionNamedType")
                || class_name.eq_ignore_ascii_case("ReflectionUnionType")
                || class_name.eq_ignore_ascii_case("ReflectionIntersectionType")
                || class_name.eq_ignore_ascii_case("ReflectionMethod")
                || class_name.eq_ignore_ascii_case("ReflectionProperty")
                || class_name.eq_ignore_ascii_case("ReflectionAttribute")
                || class_name.eq_ignore_ascii_case("ReflectionParameter")
                || class_name.eq_ignore_ascii_case("ReflectionFunction")
            {
                let reflection_kind = if class_name.eq_ignore_ascii_case("ReflectionObject") {
                    "reflectionclass".to_string()
                } else {
                    class_name.to_ascii_lowercase()
                };
                let signature = format!("{reflection_kind}::{method}");
                arguments = super::builtin_arguments::bind(
                    &signature,
                    arguments,
                    &reference_names,
                    &argument_names,
                )?
                .0;
                let internal = object.borrow().properties.clone();
                let result = match (reflection_kind.as_str(), method.as_str()) {
                    ("reflectionfunction", "getname") => internal
                        .get("\0function_name")
                        .cloned()
                        .ok_or("Invalid ReflectionFunction")?,
                    ("reflectionfunction", "invoke" | "invokeargs") => {
                        let function_name = internal
                            .get("\0function_name")
                            .ok_or("Invalid ReflectionFunction")?
                            .show();
                        let (invoke_args, invoke_references, invoke_names) = if method == "invoke" {
                            (
                                arguments.clone(),
                                reference_names.clone(),
                                argument_names.clone(),
                            )
                        } else {
                            match arguments.first().map(Value::dereference) {
                                Some(Value::Array(values)) => {
                                    let mut values_out = Vec::with_capacity(values.len());
                                    let mut references_out = Vec::with_capacity(values.len());
                                    let mut names_out = Vec::with_capacity(values.len());
                                    for (key, value) in values.iter() {
                                        values_out.push(value.clone());
                                        references_out.push(match value {
                                            Value::Reference(reference) => Some(ReferenceArgument { variable: String::new(), array_keys: Vec::new(), object_property: None, direct_reference: Some(reference.clone()) }),
                                            _ => None,
                                        });
                                        names_out.push(match key { ArrayKey::String(name) => Some(name.clone()), ArrayKey::Integer(_) => None });
                                    }
                                    (values_out, references_out, names_out)
                                }
                                Some(value) => return Err(format!("TypeError:ReflectionFunction::invokeArgs(): Argument #1 ($args) must be of type array, {} given", value.type_name())),
                                None => return Err("ReflectionFunction::invokeArgs() expects exactly 1 argument".into()),
                            }
                        };
                        if let Some(Value::Closure(closure)) =
                            internal.get("\0closure").map(Value::dereference)
                        {
                            invoke_closure(
                                &closure,
                                invoke_args,
                                &invoke_references,
                                &invoke_names,
                                variables,
                                bindings,
                                global_scope,
                                frame_id,
                                functions,
                                classes,
                                static_properties,
                                global_variables,
                                output,
                                steps,
                                false,
                            )?
                        } else {
                            invoke_with_references(
                                &function_name,
                                invoke_args,
                                &invoke_references,
                                &invoke_names,
                                false,
                                variables,
                                bindings,
                                global_variables,
                                global_scope,
                                frame_id,
                                functions,
                                classes,
                                static_properties,
                                output,
                                steps,
                            )?
                        }
                    }
                    ("reflectionfunction", "getclosure") => {
                        if let Some(Value::Closure(closure)) =
                            internal.get("\0closure").map(Value::dereference)
                        {
                            Value::Closure(closure)
                        } else {
                            let function_name = internal
                                .get("\0function_name")
                                .cloned()
                                .ok_or("Invalid ReflectionFunction")?;
                            let wrapper = BytecodeFunction::new(
                                "{reflection-function-closure}".into(),
                                Vec::new(),
                                Vec::new(),
                                Vec::new(),
                                Vec::new(),
                                Vec::new(),
                                None,
                                false,
                                Vec::new(),
                                false,
                                false,
                                false,
                                MemberVisibility::Public,
                                None,
                            );
                            Value::Closure(Rc::new(std::cell::RefCell::new(PhpClosure {
                                function: wrapper,
                                captures: HashMap::new(),
                                capture_order: Vec::new(),
                                reference_captures: HashMap::new(),
                                callable_target: Some(function_name),
                                callable_access_scope: None,
                            })))
                        }
                    }
                    ("reflectionfunction", "getattributes") => {
                        let function_name = internal
                            .get("\0function_name")
                            .ok_or("Invalid ReflectionFunction")?
                            .show();
                        let function = match internal.get("\0closure").map(Value::dereference) {
                            Some(Value::Closure(closure)) => closure.borrow().function.clone(),
                            _ => functions
                                .get(&function_name.to_ascii_lowercase())
                                .cloned()
                                .ok_or("Reflected function no longer exists")?,
                        };
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionFunction::getAttributes(): unsupported flags".into()
                            );
                        }
                        let entries = function
                            .attributes
                            .iter()
                            .filter(|attribute| {
                                filter.as_ref().is_none_or(|name| {
                                    attribute.name.eq_ignore_ascii_case(name)
                                        || (flags & 2 != 0
                                            && super::is_a_type(
                                                &attribute.name,
                                                name,
                                                classes,
                                                &mut HashSet::new(),
                                            ))
                                })
                            })
                            .enumerate()
                            .map(|(index, attribute)| {
                                (
                                    ArrayKey::Integer(index as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionAttribute".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0attribute_name".into(),
                                                    Value::String(attribute.name.clone()),
                                                ),
                                                (
                                                    "\0attribute_arguments".into(),
                                                    reflection_attribute_arguments(
                                                        &attribute.arguments,
                                                    ),
                                                ),
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(function_name.clone()),
                                                ),
                                                ("\0target_type".into(), Value::Integer(2)),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionfunction", "getparameters") => {
                        let function = match internal.get("\0closure").map(Value::dereference) {
                            Some(Value::Closure(closure)) => {
                                Some(closure.borrow().function.clone())
                            }
                            _ => internal
                                .get("\0function_name")
                                .and_then(|name| functions.get(&name.show().to_ascii_lowercase()))
                                .cloned(),
                        }
                        .ok_or("Reflected function no longer exists")?;
                        let entries = function
                            .parameters
                            .iter()
                            .enumerate()
                            .map(|(position, name)| {
                                let attributes = function
                                    .parameter_attributes
                                    .get(position)
                                    .map(Vec::as_slice)
                                    .unwrap_or_default();
                                let reflected = attributes
                                    .iter()
                                    .map(|attribute| {
                                        let duplicate_count = attributes
                                            .iter()
                                            .filter(|candidate| {
                                                candidate.name.eq_ignore_ascii_case(&attribute.name)
                                            })
                                            .count();
                                        Value::Object(Rc::new(std::cell::RefCell::new(
                                            crate::PhpObject {
                                                class_name: "ReflectionAttribute".into(),
                                                properties: HashMap::from([
                                                    (
                                                        "\0attribute_name".into(),
                                                        Value::String(attribute.name.clone()),
                                                    ),
                                                    (
                                                        "\0attribute_arguments".into(),
                                                        reflection_attribute_arguments(
                                                            &attribute.arguments,
                                                        ),
                                                    ),
                                                    (
                                                        "\0target_class".into(),
                                                        Value::String(
                                                            function.display_name.clone(),
                                                        ),
                                                    ),
                                                    (
                                                        "\0target_function".into(),
                                                        Value::String(
                                                            function.display_name.clone(),
                                                        ),
                                                    ),
                                                    ("\0target_type".into(), Value::Integer(32)),
                                                    (
                                                        "\0target_position".into(),
                                                        Value::Integer(position as i64),
                                                    ),
                                                    (
                                                        "\0is_repeated".into(),
                                                        Value::Bool(duplicate_count > 1),
                                                    ),
                                                ]),
                                                property_order: Vec::new(),
                                                active_property_hooks: Default::default(),
                                                exception: None,
                                            },
                                        )))
                                    })
                                    .enumerate()
                                    .map(|(index, attribute)| {
                                        (ArrayKey::Integer(index as i64), attribute)
                                    })
                                    .collect();
                                (
                                    ArrayKey::Integer(position as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionParameter".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0parameter_name".into(),
                                                    Value::String(name.clone()),
                                                ),
                                                (
                                                    "\0parameter_position".into(),
                                                    Value::Integer(position as i64),
                                                ),
                                                (
                                                    "\0function_name".into(),
                                                    internal
                                                        .get("\0function_name")
                                                        .cloned()
                                                        .unwrap_or(Value::Null),
                                                ),
                                                (
                                                    "\0closure".into(),
                                                    internal
                                                        .get("\0closure")
                                                        .cloned()
                                                        .unwrap_or(Value::Null),
                                                ),
                                                (
                                                    "\0parameter_attributes".into(),
                                                    Value::from_array_entries(reflected),
                                                ),
                                                (
                                                    "\0parameter_type".into(),
                                                    function
                                                        .parameter_types
                                                        .get(position)
                                                        .and_then(Clone::clone)
                                                        .map(Value::String)
                                                        .unwrap_or(Value::Null),
                                                ),
                                                (
                                                    "\0parameter_by_reference".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_by_reference
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_variadic".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_variadic
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_promoted".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_promoted
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_has_default".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_defaults
                                                            .get(position)
                                                            .is_some_and(Option::is_some),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_optional".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_variadic
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false)
                                                            || (function
                                                                .parameter_defaults
                                                                .get(position)
                                                                .is_some_and(Option::is_some)
                                                                && position
                                                                    >= required_parameter_count(
                                                                        &function,
                                                                    )),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_default".into(),
                                                    function
                                                        .parameter_defaults
                                                        .get(position)
                                                        .and_then(Clone::clone)
                                                        .unwrap_or(Value::Null),
                                                ),
                                                (
                                                    "\0parameter_default_constant_name".into(),
                                                    function
                                                        .parameter_default_constant_names
                                                        .get(position)
                                                        .and_then(Clone::clone)
                                                        .map(Value::String)
                                                        .unwrap_or(Value::Null),
                                                ),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    (
                        "reflectionfunction",
                        "getnumberofparameters"
                        | "getnumberofrequiredparameters"
                        | "hasreturntype"
                        | "getreturntype",
                    ) => {
                        let function = match internal.get("\0closure").map(Value::dereference) {
                            Some(Value::Closure(closure)) => {
                                Some(closure.borrow().function.clone())
                            }
                            _ => internal
                                .get("\0function_name")
                                .and_then(|name| functions.get(&name.show().to_ascii_lowercase()))
                                .cloned(),
                        }
                        .ok_or("Reflected function no longer exists")?;
                        match method.as_str() {
                            "getnumberofparameters" => {
                                Value::Integer(function.parameters.len() as i64)
                            }
                            "getnumberofrequiredparameters" => Value::Integer(
                                function
                                    .parameters
                                    .iter()
                                    .enumerate()
                                    .filter(|(index, _)| {
                                        function
                                            .parameter_defaults
                                            .get(*index)
                                            .is_none_or(Option::is_none)
                                            && !function
                                                .parameter_variadic
                                                .get(*index)
                                                .copied()
                                                .unwrap_or(false)
                                    })
                                    .map(|(index, _)| index + 1)
                                    .max()
                                    .unwrap_or(0) as i64,
                            ),
                            "hasreturntype" => Value::Bool(function.return_type.is_some()),
                            "getreturntype" => function
                                .return_type
                                .as_deref()
                                .map(reflection_type)
                                .unwrap_or(Value::Null),
                            _ => unreachable!(),
                        }
                    }
                    (
                        "reflectionfunction",
                        "isvariadic" | "returnsreference" | "isclosure" | "isstatic",
                    ) => {
                        let function = match internal.get("\0closure").map(Value::dereference) {
                            Some(Value::Closure(closure)) => {
                                Some(closure.borrow().function.clone())
                            }
                            _ => internal
                                .get("\0function_name")
                                .and_then(|name| functions.get(&name.show().to_ascii_lowercase()))
                                .cloned(),
                        }
                        .ok_or("Reflected function no longer exists")?;
                        Value::Bool(match method.as_str() {
                            "isvariadic" => {
                                function.parameter_variadic.iter().any(|variadic| *variadic)
                            }
                            "returnsreference" => function.returns_by_reference,
                            "isclosure" => internal.get("\0closure").is_some(),
                            "isstatic" => function.is_static,
                            _ => false,
                        })
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getname") => internal
                        .get("\0case_name")
                        .cloned()
                        .ok_or("Invalid ReflectionEnumUnitCase")?,
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getvalue") => internal
                        .get("\0case_value")
                        .cloned()
                        .ok_or("Invalid ReflectionEnumUnitCase")?,
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getenum") => {
                        let target = internal
                            .get("\0target_class")
                            .cloned()
                            .ok_or("Invalid ReflectionEnumUnitCase")?;
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionEnum".into(),
                            properties: HashMap::from([("\0target_class".into(), target)]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    ("reflectionenumbackedcase", "getbackingvalue") => {
                        let case = internal
                            .get("\0case_value")
                            .ok_or("Invalid ReflectionEnumBackedCase")?;
                        match case.dereference() {
                            Value::Object(case) => case
                                .borrow()
                                .properties
                                .get("value")
                                .cloned()
                                .ok_or("Invalid ReflectionEnumBackedCase")?,
                            _ => return Err("Invalid ReflectionEnumBackedCase".into()),
                        }
                    }
                    (
                        "reflectionenumunitcase" | "reflectionenumbackedcase",
                        "getdeclaringclass",
                    ) => {
                        let declaring = internal
                            .get("\0declaring_class")
                            .ok_or("Invalid ReflectionEnumCase")?
                            .show();
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionClass".into(),
                            properties: HashMap::from([(
                                "\0target_class".into(),
                                Value::String(declaring),
                            )]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "gettype") => {
                        Value::Null
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "hastype") => {
                        Value::Bool(false)
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "isenumcase") => {
                        Value::Bool(true)
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getmodifiers") => {
                        Value::Integer(1)
                    }
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "ispublic") => {
                        Value::Bool(true)
                    }
                    (
                        "reflectionenumunitcase" | "reflectionenumbackedcase",
                        "isprotected" | "isprivate" | "isfinal",
                    ) => Value::Bool(false),
                    ("reflectionenumunitcase" | "reflectionenumbackedcase", "getattributes") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionEnumCase")?
                            .show();
                        let name = internal
                            .get("\0case_name")
                            .ok_or("Invalid ReflectionEnumCase")?
                            .show();
                        let constant = classes
                            .get(&target.to_ascii_lowercase())
                            .and_then(|class| class.constants.get(&name))
                            .ok_or("Reflected enum case no longer exists")?;
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionClassConstant::getAttributes(): unsupported flags"
                                    .into(),
                            );
                        }
                        Value::from_array_entries(
                            constant
                                .attributes
                                .iter()
                                .filter(|attribute| {
                                    filter.as_ref().is_none_or(|filter| {
                                        attribute.name.eq_ignore_ascii_case(filter)
                                            || (flags & 2 != 0
                                                && super::is_a_type(
                                                    &attribute.name,
                                                    filter,
                                                    classes,
                                                    &mut HashSet::new(),
                                                ))
                                    })
                                })
                                .enumerate()
                                .map(|(index, attribute)| {
                                    (
                                        ArrayKey::Integer(index as i64),
                                        Value::Object(Rc::new(std::cell::RefCell::new(
                                            crate::PhpObject {
                                                class_name: "ReflectionAttribute".into(),
                                                properties: HashMap::from([
                                                    (
                                                        "\0attribute_name".into(),
                                                        Value::String(attribute.name.clone()),
                                                    ),
                                                    (
                                                        "\0attribute_arguments".into(),
                                                        reflection_attribute_arguments(
                                                            &attribute.arguments,
                                                        ),
                                                    ),
                                                    (
                                                        "\0target_class".into(),
                                                        Value::String(target.clone()),
                                                    ),
                                                    ("\0target_type".into(), Value::Integer(16)),
                                                    (
                                                        "\0target_member".into(),
                                                        Value::String(name.clone()),
                                                    ),
                                                ]),
                                                property_order: Vec::new(),
                                                active_property_hooks: Default::default(),
                                                exception: None,
                                            },
                                        ))),
                                    )
                                })
                                .collect(),
                        )
                    }
                    ("reflectionnamedtype", "getname") => internal
                        .get("\0type_name")
                        .cloned()
                        .ok_or("Invalid ReflectionNamedType")?,
                    ("reflectionnamedtype", "isbuiltin") => {
                        Value::Bool(internal.get("\0is_builtin").is_some_and(Value::truthy))
                    }
                    ("reflectionnamedtype", "allowsnull") => {
                        Value::Bool(internal.get("\0allows_null").is_some_and(Value::truthy))
                    }
                    ("reflectionuniontype" | "reflectionintersectiontype", "gettypes") => internal
                        .get("\0types")
                        .cloned()
                        .ok_or("Invalid ReflectionType")?,
                    ("reflectionuniontype" | "reflectionintersectiontype", "allowsnull") => {
                        Value::Bool(internal.get("\0allows_null").is_some_and(Value::truthy))
                    }
                    ("reflectionenum", "isbacked") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionEnum target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.enum_backing_type.is_some()),
                        )
                    }
                    ("reflectionenum", "getbackingtype") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionEnum target")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .filter(|class| class.is_enum)
                            .ok_or("Invalid ReflectionEnum target")?;
                        match &class.enum_backing_type {
                            None => Value::Null,
                            Some(name) => {
                                Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                    class_name: "ReflectionNamedType".into(),
                                    properties: HashMap::from([
                                        ("\0type_name".into(), Value::String(name.clone())),
                                        ("\0is_builtin".into(), Value::Bool(true)),
                                        ("\0allows_null".into(), Value::Bool(false)),
                                    ]),
                                    property_order: Vec::new(),
                                    active_property_hooks: Default::default(),
                                    exception: None,
                                })))
                            }
                        }
                    }
                    ("reflectionenum", "getcases") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionEnum target")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .filter(|class| class.is_enum)
                            .ok_or("Invalid ReflectionEnum target")?;
                        Value::from_array_entries(
                            class
                                .enum_cases
                                .iter()
                                .enumerate()
                                .filter_map(|(index, name)| {
                                    let case = class.constants.get(name)?.value.clone();
                                    let reflected = crate::PhpObject {
                                        class_name: if class.enum_backing_type.is_some() {
                                            "ReflectionEnumBackedCase"
                                        } else {
                                            "ReflectionEnumUnitCase"
                                        }
                                        .into(),
                                        properties: HashMap::from([
                                            ("\0case_name".into(), Value::String(name.clone())),
                                            ("\0case_value".into(), case),
                                            (
                                                "\0target_class".into(),
                                                Value::String(class.name.clone()),
                                            ),
                                            ("\0constant_name".into(), Value::String(name.clone())),
                                            (
                                                "\0declaring_class".into(),
                                                Value::String(class.name.clone()),
                                            ),
                                        ]),
                                        property_order: Vec::new(),
                                        active_property_hooks: Default::default(),
                                        exception: None,
                                    };
                                    Some((
                                        ArrayKey::Integer(index as i64),
                                        Value::Object(Rc::new(std::cell::RefCell::new(reflected))),
                                    ))
                                })
                                .collect(),
                        )
                    }
                    ("reflectionenum", "hascase") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionEnum target")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .filter(|class| class.is_enum)
                            .ok_or("Invalid ReflectionEnum target")?;
                        if arguments.len() != 1 {
                            return Err(format!(
                                "ReflectionEnum::hasCase() expects exactly 1 argument, {} given",
                                arguments.len()
                            ));
                        }
                        let name = reflection_string_argument(
                            "ReflectionEnum::hasCase",
                            &arguments[0],
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        Value::Bool(class.enum_cases.iter().any(|case| case == &name))
                    }
                    ("reflectionenum", "getcase") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionEnum target")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .filter(|class| class.is_enum)
                            .ok_or("Invalid ReflectionEnum target")?;
                        if arguments.len() != 1 {
                            return Err(format!(
                                "ReflectionEnum::getCase() expects exactly 1 argument, {} given",
                                arguments.len()
                            ));
                        }
                        let name = reflection_string_argument(
                            "ReflectionEnum::getCase",
                            &arguments[0],
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        let case = class
                            .enum_cases
                            .iter()
                            .find(|case| case.as_str() == name)
                            .ok_or_else(|| {
                                format!("ReflectionException:Case {name} does not exist")
                            })?;
                        let value = class
                            .constants
                            .get(case)
                            .ok_or("Enum case metadata is missing")?
                            .value
                            .clone();
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: if class.enum_backing_type.is_some() {
                                "ReflectionEnumBackedCase"
                            } else {
                                "ReflectionEnumUnitCase"
                            }
                            .into(),
                            properties: HashMap::from([
                                ("\0case_name".into(), Value::String(case.clone())),
                                ("\0case_value".into(), value),
                                ("\0target_class".into(), Value::String(class.name.clone())),
                                ("\0constant_name".into(), Value::String(case.clone())),
                                (
                                    "\0declaring_class".into(),
                                    Value::String(class.name.clone()),
                                ),
                            ]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    ("reflectionclass", "isenum") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.is_enum),
                        )
                    }
                    ("reflectionclass", "isinterface") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.is_interface),
                        )
                    }
                    ("reflectionclass", "isabstract") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.is_abstract),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "isinstantiable") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let instantiable =
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| {
                                    !class.is_abstract
                                        && !class.is_interface
                                        && !class.is_trait
                                        && !class.is_enum
                                        && super::objects::find_method(
                                            &class.name,
                                            "__construct",
                                            classes,
                                        )
                                        .is_none_or(
                                            |constructor| {
                                                constructor.visibility
                                                    == crate::MemberVisibility::Public
                                            },
                                        )
                                });
                        Value::Bool(instantiable)
                    }
                    ("reflectionclass", "isfinal") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.is_final),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "isreadonly") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.is_readonly),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "isanonymous") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(target.starts_with("class@anonymous#"))
                    }
                    ("reflectionclass" | "reflectionenum", "isiterable" | "isiterateable") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(super::is_a_type(
                            &target.to_ascii_lowercase(),
                            "traversable",
                            classes,
                            &mut HashSet::new(),
                        ))
                    }
                    ("reflectionclass" | "reflectionenum", "getconstant") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments
                            .first()
                            .ok_or("ReflectionClass::getConstant() expects exactly 1 argument")?
                            .show();
                        if let Some((_, constant)) =
                            super::objects::find_class_constant(&target, &name, classes)
                        {
                            constant.value.clone()
                        } else {
                            global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Deprecated,
                                message: "ReflectionClass::getConstant() for a non-existent constant is deprecated, use ReflectionClass::hasConstant() to check if the constant exists".into(),
                                source_file: None,
                                source_line: None,
                                output_offset: 0,
                            });
                            Value::Bool(false)
                        }
                    }
                    ("reflectionclass" | "reflectionenum", "getreflectionconstant") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments.first().ok_or("ReflectionClass::getReflectionConstant() expects exactly 1 argument")?.show();
                        if let Some((_, constant)) =
                            super::objects::find_class_constant(&target, &name, classes)
                        {
                            Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionClassConstant".into(),
                                properties: HashMap::from([
                                    ("\0target_class".into(), Value::String(target)),
                                    ("\0constant_name".into(), Value::String(name)),
                                    (
                                        "\0declaring_class".into(),
                                        Value::String(
                                            classes.get(&constant.declaring_class).map_or_else(
                                                || constant.declaring_class.clone(),
                                                |class| class.name.clone(),
                                            ),
                                        ),
                                    ),
                                ]),
                                property_order: Vec::new(),
                                active_property_hooks: Default::default(),
                                exception: None,
                            })))
                        } else {
                            Value::Bool(false)
                        }
                    }
                    ("reflectionclassconstant", "getname") => internal
                        .get("\0constant_name")
                        .cloned()
                        .ok_or("Invalid ReflectionClassConstant")?,
                    ("reflectionclassconstant", "hastype") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        Value::Bool(
                            super::objects::find_class_constant(&target, &name, classes)
                                .is_some_and(|(_, constant)| constant.type_name.is_some()),
                        )
                    }
                    ("reflectionclassconstant", "isenumcase") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        Value::Bool(classes.get(&target.to_ascii_lowercase()).is_some_and(
                            |class| {
                                class.is_enum && class.enum_cases.iter().any(|case| case == &name)
                            },
                        ))
                    }
                    ("reflectionclassconstant", "gettype") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let type_name =
                            super::objects::find_class_constant(&target, &name, classes)
                                .and_then(|(_, constant)| constant.type_name.clone());
                        type_name.map_or(Value::Null, |type_name| reflection_type(&type_name))
                    }
                    ("reflectionclassconstant", "getvalue") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        super::objects::find_class_constant(&target, &name, classes)
                            .map(|(_, constant)| constant.value.clone())
                            .ok_or("Reflected class constant no longer exists")?
                    }
                    ("reflectionclassconstant", "getdeclaringclass") => {
                        let declaring = internal
                            .get("\0declaring_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionClass".into(),
                            properties: HashMap::from([(
                                "\0target_class".into(),
                                Value::String(declaring),
                            )]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    (
                        "reflectionclassconstant",
                        "getmodifiers" | "ispublic" | "isprotected" | "isprivate" | "isfinal",
                    ) => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let constant = super::objects::find_class_constant(&target, &name, classes)
                            .map(|(_, constant)| constant)
                            .ok_or("Reflected class constant no longer exists")?;
                        let visibility = match constant.visibility {
                            crate::MemberVisibility::Public => 1,
                            crate::MemberVisibility::Protected => 2,
                            crate::MemberVisibility::Private => 4,
                        };
                        if method == "getmodifiers" {
                            Value::Integer(visibility | if constant.is_final { 32 } else { 0 })
                        } else {
                            Value::Bool(match method.as_str() {
                                "ispublic" => visibility == 1,
                                "isprotected" => visibility == 2,
                                "isprivate" => visibility == 4,
                                _ => constant.is_final,
                            })
                        }
                    }
                    ("reflectionclassconstant", "getattributes") => {
                        let target = internal
                            .get("\0declaring_class")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionClassConstant")?
                            .show();
                        let constant = classes
                            .get(&target.to_ascii_lowercase())
                            .and_then(|class| class.constants.get(&name))
                            .ok_or("Reflected class constant no longer exists")?;
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionClassConstant::getAttributes(): unsupported flags"
                                    .into(),
                            );
                        }
                        Value::from_array_entries(
                            constant
                                .attributes
                                .iter()
                                .filter(|attribute| {
                                    filter.as_ref().is_none_or(|name| {
                                        attribute.name.eq_ignore_ascii_case(name)
                                            || (flags & 2 != 0
                                                && super::is_a_type(
                                                    &attribute.name,
                                                    name,
                                                    classes,
                                                    &mut HashSet::new(),
                                                ))
                                    })
                                })
                                .enumerate()
                                .map(|(index, attribute)| {
                                    let arguments =
                                        reflection_attribute_arguments(&attribute.arguments);
                                    (
                                        ArrayKey::Integer(index as i64),
                                        Value::Object(Rc::new(std::cell::RefCell::new(
                                            crate::PhpObject {
                                                class_name: "ReflectionAttribute".into(),
                                                properties: HashMap::from([
                                                    (
                                                        "\0attribute_name".into(),
                                                        Value::String(attribute.name.clone()),
                                                    ),
                                                    ("\0attribute_arguments".into(), arguments),
                                                    (
                                                        "\0target_class".into(),
                                                        Value::String(target.clone()),
                                                    ),
                                                    ("\0target_type".into(), Value::Integer(16)),
                                                    (
                                                        "\0target_member".into(),
                                                        Value::String(name.clone()),
                                                    ),
                                                ]),
                                                property_order: Vec::new(),
                                                active_property_hooks: Default::default(),
                                                exception: None,
                                            },
                                        ))),
                                    )
                                })
                                .collect(),
                        )
                    }
                    ("reflectionconstant", "getname") => internal
                        .get("\0constant_name")
                        .cloned()
                        .ok_or("Invalid ReflectionConstant")?,
                    ("reflectionconstant", "getvalue") => {
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionConstant")?
                            .show();
                        super::read_constant(&name, global_variables)?
                    }
                    ("reflectionconstant", "getnamespace") => {
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionConstant")?
                            .show();
                        Value::String(
                            name.rsplit_once('\\')
                                .map_or(String::new(), |(namespace, _)| namespace.to_string()),
                        )
                    }
                    ("reflectionconstant", "isdeprecated") => Value::Bool(false),
                    ("reflectionconstant", "isuserdefined") => {
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionConstant")?
                            .show();
                        Value::Bool(global_variables.constants.contains_key(&name))
                    }
                    ("reflectionconstant", "isinternal") => {
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionConstant")?
                            .show();
                        Value::Bool(!global_variables.constants.contains_key(&name))
                    }
                    ("reflectionconstant", "getattributes") => {
                        let name = internal
                            .get("\0constant_name")
                            .ok_or("Invalid ReflectionConstant")?
                            .show();
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionConstant::getAttributes(): unsupported flags".into()
                            );
                        }
                        Value::from_array_entries(
                            global_variables
                                .constant_attributes
                                .get(&name)
                                .into_iter()
                                .flatten()
                                .filter(|attribute| {
                                    filter.as_ref().is_none_or(|filter| {
                                        attribute.name.eq_ignore_ascii_case(filter)
                                            || (flags & 2 != 0
                                                && super::is_a_type(
                                                    &attribute.name,
                                                    filter,
                                                    classes,
                                                    &mut HashSet::new(),
                                                ))
                                    })
                                })
                                .enumerate()
                                .map(|(index, attribute)| {
                                    (
                                        ArrayKey::Integer(index as i64),
                                        Value::Object(Rc::new(std::cell::RefCell::new(
                                            crate::PhpObject {
                                                class_name: "ReflectionAttribute".into(),
                                                properties: HashMap::from([
                                                    (
                                                        "\0attribute_name".into(),
                                                        Value::String(attribute.name.clone()),
                                                    ),
                                                    (
                                                        "\0attribute_arguments".into(),
                                                        reflection_attribute_arguments(
                                                            &attribute.arguments,
                                                        ),
                                                    ),
                                                    (
                                                        "\0target_class".into(),
                                                        Value::String(name.clone()),
                                                    ),
                                                    ("\0target_type".into(), Value::Integer(64)),
                                                ]),
                                                property_order: Vec::new(),
                                                active_property_hooks: Default::default(),
                                                exception: None,
                                            },
                                        ))),
                                    )
                                })
                                .collect(),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "hasconstant") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments
                            .first()
                            .ok_or("ReflectionClass::hasConstant() expects exactly 1 argument")?
                            .show();
                        Value::Bool(
                            super::objects::find_class_constant(&target, &name, classes).is_some(),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "getconstants") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let filter =
                            arguments
                                .first()
                                .and_then(|value| match value.dereference() {
                                    Value::Null => None,
                                    Value::Integer(value) => Some(value),
                                    _ => None,
                                });
                        reflection_constants(&target, classes, filter)
                    }
                    ("reflectionclass" | "reflectionenum", "getdefaultproperties") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let mut chain = Vec::new();
                        let mut current = Some(target.as_str());
                        while let Some(name) = current {
                            let Some(class) = classes.get(&name.to_ascii_lowercase()) else {
                                break;
                            };
                            chain.push(class);
                            current = class.parent.as_deref();
                        }
                        let mut properties = PhpArray::new();
                        for (depth, class) in chain.iter().rev().enumerate() {
                            let is_target = depth + 1 == chain.len();
                            for name in &class.property_order {
                                let Some(metadata) = class.properties.get(name) else {
                                    continue;
                                };
                                if !is_target && metadata.visibility == MemberVisibility::Private {
                                    continue;
                                }
                                properties.set(
                                    ArrayKey::String(name.clone()),
                                    metadata.default.clone().unwrap_or(Value::Null),
                                );
                            }
                            let mut static_names =
                                class.static_properties.keys().cloned().collect::<Vec<_>>();
                            static_names.sort();
                            for name in static_names {
                                let metadata = &class.static_properties[&name];
                                if !is_target && metadata.visibility == MemberVisibility::Private {
                                    continue;
                                }
                                properties.set(
                                    ArrayKey::String(name),
                                    metadata.default.clone().unwrap_or(Value::Null),
                                );
                            }
                        }
                        Value::Array(properties)
                    }
                    ("reflectionclass" | "reflectionenum", "getstaticproperties") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let mut chain = Vec::new();
                        let mut current = Some(target.as_str());
                        while let Some(name) = current {
                            let Some(class) = classes.get(&name.to_ascii_lowercase()) else {
                                break;
                            };
                            chain.push(class);
                            current = class.parent.as_deref();
                        }
                        let mut properties = PhpArray::new();
                        for (depth, class) in chain.iter().rev().enumerate() {
                            let is_target = depth + 1 == chain.len();
                            let mut names =
                                class.static_properties.keys().cloned().collect::<Vec<_>>();
                            names.sort();
                            for name in names {
                                let metadata = &class.static_properties[&name];
                                if !is_target && metadata.visibility == MemberVisibility::Private {
                                    continue;
                                }
                                let storage_key = (class.name.to_ascii_lowercase(), name.clone());
                                let value =
                                    static_properties.get(&storage_key).cloned().unwrap_or_else(
                                        || metadata.default.clone().unwrap_or(Value::Null),
                                    );
                                properties.set(ArrayKey::String(name), value.dereference());
                            }
                        }
                        Value::Array(properties)
                    }
                    ("reflectionclass", "hasmethod") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments
                            .first()
                            .ok_or("ReflectionClass::hasMethod() expects exactly 1 argument")?
                            .show();
                        Value::Bool(
                            find_method(&target, &name.to_ascii_lowercase(), classes).is_some(),
                        )
                    }
                    ("reflectionclass", "hasproperty") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments
                            .first()
                            .ok_or("ReflectionClass::hasProperty() expects exactly 1 argument")?
                            .show();
                        let mut current = Some(target.as_str());
                        let target_key = target.to_ascii_lowercase();
                        let mut found = false;
                        while let Some(class_name) = current {
                            let Some(class) = classes.get(&class_name.to_ascii_lowercase()) else {
                                break;
                            };
                            let declared_here = class_name.eq_ignore_ascii_case(&target_key);
                            let visible_member = |property: &crate::BytecodeProperty| {
                                declared_here || property.visibility != MemberVisibility::Private
                            };
                            if class.properties.get(&name).is_some_and(visible_member)
                                || class
                                    .static_properties
                                    .get(&name)
                                    .is_some_and(visible_member)
                            {
                                found = true;
                                break;
                            }
                            current = class.parent.as_deref();
                        }
                        Value::Bool(found)
                    }
                    ("reflectionclass", "getmethods") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let reflection_class = if class_name.eq_ignore_ascii_case("ReflectionEnum")
                        {
                            "ReflectionEnum"
                        } else {
                            "ReflectionClass"
                        };
                        let filter = reflection_optional_int_argument(
                            &arguments,
                            reflection_class,
                            "getMethods",
                        )?;
                        let mut methods = Vec::new();
                        let mut seen = HashSet::new();
                        let mut current = Some(target.as_str());
                        while let Some(class_name) = current {
                            let Some(class) = classes.get(&class_name.to_ascii_lowercase()) else {
                                break;
                            };
                            for method_name in &class.method_order {
                                if !seen.insert(method_name.clone()) {
                                    continue;
                                }
                                let Some(function) = class.methods.get(method_name) else {
                                    continue;
                                };
                                if filter.is_some_and(|mask| {
                                    reflection_method_modifiers(function) & mask == 0
                                }) {
                                    continue;
                                }
                                let declaring_class =
                                    function.declaring_class.as_deref().unwrap_or(&class.name);
                                let display_name = classes
                                    .get(&declaring_class.to_ascii_lowercase())
                                    .and_then(|declaring| {
                                        declaring.method_display_names.get(method_name)
                                    })
                                    .cloned()
                                    .unwrap_or_else(|| function.display_name.clone());
                                methods.push((
                                    ArrayKey::Integer(methods.len() as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionMethod".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(declaring_class.to_string()),
                                                ),
                                                (
                                                    "\0method_name".into(),
                                                    Value::String(display_name),
                                                ),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                ));
                            }
                            current = class.parent.as_deref();
                        }
                        Value::from_array_entries(methods)
                    }
                    ("reflectionclass", "getparentclass") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        match classes
                            .get(&target.to_ascii_lowercase())
                            .and_then(|class| class.parent.as_ref())
                            .and_then(|parent| {
                                classes
                                    .get(&parent.to_ascii_lowercase())
                                    .map(|class| class.name.clone())
                            }) {
                            Some(parent) => {
                                Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                    class_name: "ReflectionClass".into(),
                                    properties: HashMap::from([(
                                        "\0target_class".into(),
                                        Value::String(parent.clone()),
                                    )]),
                                    property_order: Vec::new(),
                                    active_property_hooks: Default::default(),
                                    exception: None,
                                })))
                            }
                            None => Value::Bool(false),
                        }
                    }
                    ("reflectionclass", "getinterfacenames") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let names = reflection_interface_names(&target, classes);
                        Value::from_array_entries(
                            names
                                .into_iter()
                                .enumerate()
                                .map(|(i, name)| (ArrayKey::Integer(i as i64), Value::String(name)))
                                .collect(),
                        )
                    }
                    ("reflectionclass", "getinterfaces") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::from_array_entries(
                            reflection_interface_names(&target, classes)
                                .into_iter()
                                .map(|name| {
                                    let key = ArrayKey::String(name.clone());
                                    let reflected = Value::Object(Rc::new(
                                        std::cell::RefCell::new(crate::PhpObject {
                                            class_name: "ReflectionClass".into(),
                                            properties: HashMap::from([(
                                                "\0target_class".into(),
                                                Value::String(name),
                                            )]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        }),
                                    ));
                                    (key, reflected)
                                })
                                .collect(),
                        )
                    }
                    ("reflectionclass", "gettraitnames") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::from_array_entries(
                            reflection_trait_names(&target, classes)
                                .into_iter()
                                .enumerate()
                                .map(|(index, name)| {
                                    (ArrayKey::Integer(index as i64), Value::String(name))
                                })
                                .collect(),
                        )
                    }
                    ("reflectionclass", "gettraits") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::from_array_entries(
                            reflection_trait_names(&target, classes)
                                .into_iter()
                                .map(|name| {
                                    let reflected = Value::Object(Rc::new(
                                        std::cell::RefCell::new(crate::PhpObject {
                                            class_name: "ReflectionClass".into(),
                                            properties: HashMap::from([(
                                                "\0target_class".into(),
                                                Value::String(name.clone()),
                                            )]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        }),
                                    ));
                                    (ArrayKey::String(name), reflected)
                                })
                                .collect(),
                        )
                    }
                    ("reflectionclass", "gettraitaliases") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let aliases = classes
                            .get(&target.to_ascii_lowercase())
                            .map(|class| class.trait_aliases.as_slice())
                            .unwrap_or_default();
                        Value::from_array_entries(
                            aliases
                                .iter()
                                .map(|(alias, original)| {
                                    (
                                        ArrayKey::String(alias.clone()),
                                        Value::String(original.clone()),
                                    )
                                })
                                .collect(),
                        )
                    }
                    ("reflectionclass", "issubclassof" | "implementsinterface") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let related = arguments
                            .first()
                            .ok_or_else(|| {
                                format!("ReflectionClass::{method}() expects exactly 1 argument")
                            })?
                            .show();
                        let same = target.eq_ignore_ascii_case(&related);
                        let matches = super::is_a_type(
                            &target.to_ascii_lowercase(),
                            &related.to_ascii_lowercase(),
                            classes,
                            &mut HashSet::new(),
                        );
                        Value::Bool(matches && (method == "implementsinterface" || !same))
                    }
                    ("reflectionclass" | "reflectionenum", "getname") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        classes.get(&target.to_ascii_lowercase()).map_or_else(
                            || Value::String(target),
                            |class| Value::String(class.reflection_name.clone()),
                        )
                    }
                    (
                        "reflectionclass" | "reflectionenum",
                        "getshortname" | "getnamespacename" | "innamespace",
                    ) => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let reflected = classes.get(&target.to_ascii_lowercase());
                        if reflected.is_some_and(|class| class.name.starts_with("class@anonymous#"))
                        {
                            match method.as_str() {
                                "getshortname" => Value::String("class@anonymous".into()),
                                "getnamespacename" => Value::String(String::new()),
                                _ => Value::Bool(false),
                            }
                        } else {
                            let target = reflected
                                .map_or(target.as_str(), |class| class.reflection_name.as_str());
                            let target = target.trim_start_matches('\\');
                            let namespace = target
                                .rsplit_once('\\')
                                .map_or("", |(namespace, _)| namespace);
                            match method.as_str() {
                                "getshortname" => Value::String(
                                    target.rsplit('\\').next().unwrap_or(target).into(),
                                ),
                                "getnamespacename" => Value::String(namespace.into()),
                                _ => Value::Bool(!namespace.is_empty()),
                            }
                        }
                    }
                    ("reflectionclass" | "reflectionenum", "istrait") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        Value::Bool(
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| class.is_trait),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "isinstance") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let object = arguments.first().ok_or_else(|| {
                            format!("ReflectionClass::isInstance() expects exactly 1 argument")
                        })?;
                        let actual = match object.dereference() {
                            Value::Object(object) => object.borrow().class_name.clone(),
                            Value::Generator(_) => "Generator".into(),
                            other => return Err(format!("TypeError:ReflectionClass::isInstance(): Argument #1 ($object) must be of type object, {} given", other.type_name())),
                        };
                        Value::Bool(super::is_a_type(
                            &actual.to_ascii_lowercase(),
                            &target.to_ascii_lowercase(),
                            classes,
                            &mut HashSet::new(),
                        ))
                    }
                    ("reflectionclass" | "reflectionenum", "iscloneable") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let cloneable_kind =
                            classes
                                .get(&target.to_ascii_lowercase())
                                .is_some_and(|class| {
                                    !class.is_interface && !class.is_trait && !class.is_enum
                                });
                        Value::Bool(
                            cloneable_kind
                                && find_method(&target, "__clone", classes).is_none_or(|method| {
                                    method.visibility != crate::MemberVisibility::Private
                                }),
                        )
                    }
                    ("reflectionclass" | "reflectionenum", "getconstructor") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        if let Some(constructor) = find_method(&target, "__construct", classes) {
                            let declaring_class = constructor
                                .declaring_class
                                .clone()
                                .unwrap_or_else(|| target.to_ascii_lowercase());
                            let display_name = classes
                                .get(&declaring_class.to_ascii_lowercase())
                                .and_then(|class| class.method_display_names.get("__construct"))
                                .cloned()
                                .unwrap_or_else(|| "__construct".into());
                            Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionMethod".into(),
                                properties: HashMap::from([
                                    ("\0target_class".into(), Value::String(declaring_class)),
                                    ("\0method_name".into(), Value::String(display_name)),
                                ]),
                                property_order: Vec::new(),
                                active_property_hooks: Default::default(),
                                exception: None,
                            })))
                        } else {
                            Value::Null
                        }
                    }
                    ("reflectionclass" | "reflectionenum", "getattributes") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionClass::getAttributes(): unsupported flags".into()
                            );
                        }
                        let entries = class
                            .attributes
                            .iter()
                            .filter(|attribute| {
                                filter.as_ref().is_none_or(|name| {
                                    attribute.name.eq_ignore_ascii_case(name)
                                        || (flags & 2 != 0
                                            && super::is_a_type(
                                                &attribute.name,
                                                name,
                                                classes,
                                                &mut HashSet::new(),
                                            ))
                                })
                            })
                            .enumerate()
                            .map(|(index, attribute)| {
                                let arguments =
                                    reflection_attribute_arguments(&attribute.arguments);
                                (
                                    ArrayKey::Integer(index as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionAttribute".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0attribute_name".into(),
                                                    Value::String(attribute.name.clone()),
                                                ),
                                                ("\0attribute_arguments".into(), arguments),
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(class.name.clone()),
                                                ),
                                                ("\0target_type".into(), Value::Integer(1)),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionclass" | "reflectionenum", "getproperties") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let reflection_class = if class_name.eq_ignore_ascii_case("ReflectionEnum")
                        {
                            "ReflectionEnum"
                        } else {
                            "ReflectionClass"
                        };
                        let filter = reflection_optional_int_argument(
                            &arguments,
                            reflection_class,
                            "getProperties",
                        )?;
                        let mut properties = Vec::new();
                        let mut seen = HashSet::new();
                        let mut current = Some(target.as_str());
                        while let Some(class_name) = current {
                            let Some(class) = classes.get(&class_name.to_ascii_lowercase()) else {
                                break;
                            };
                            let declared_here = class_name.eq_ignore_ascii_case(&target);
                            let mut instance_names = class.property_order.clone();
                            let mut remaining = class
                                .properties
                                .keys()
                                .filter(|name| !instance_names.contains(name))
                                .cloned()
                                .collect::<Vec<_>>();
                            remaining.sort();
                            instance_names.extend(remaining);
                            for (is_static, names) in [
                                (false, instance_names),
                                (true, {
                                    let mut names =
                                        class.static_properties.keys().cloned().collect::<Vec<_>>();
                                    names.sort();
                                    names
                                }),
                            ] {
                                for name in names {
                                    let metadata = if is_static {
                                        &class.static_properties[&name]
                                    } else {
                                        &class.properties[&name]
                                    };
                                    if !declared_here
                                        && metadata.visibility == MemberVisibility::Private
                                    {
                                        continue;
                                    }
                                    if !seen.insert(name.clone()) {
                                        continue;
                                    }
                                    let visibility = match metadata.visibility {
                                        MemberVisibility::Public => 1,
                                        MemberVisibility::Protected => 2,
                                        MemberVisibility::Private => 4,
                                    };
                                    let set_modifier = match metadata.set_visibility {
                                        Some(MemberVisibility::Private) => 32,
                                        Some(MemberVisibility::Protected) => 64,
                                        _ => 0,
                                    };
                                    let modifiers = visibility
                                        | set_modifier
                                        | if is_static { 16 } else { 0 }
                                        | if metadata.is_readonly { 128 } else { 0 };
                                    if filter.is_some_and(|mask| modifiers & mask == 0) {
                                        continue;
                                    }
                                    properties.push((
                                        ArrayKey::Integer(properties.len() as i64),
                                        Value::Object(Rc::new(std::cell::RefCell::new(
                                            crate::PhpObject {
                                                class_name: "ReflectionProperty".into(),
                                                properties: HashMap::from([
                                                    (
                                                        "\0target_class".into(),
                                                        Value::String(class.name.clone()),
                                                    ),
                                                    ("\0property_name".into(), Value::String(name)),
                                                ]),
                                                property_order: Vec::new(),
                                                active_property_hooks: Default::default(),
                                                exception: None,
                                            },
                                        ))),
                                    ));
                                }
                            }
                            current = class.parent.as_deref();
                        }
                        Value::from_array_entries(properties)
                    }
                    ("reflectionclass" | "reflectionenum", "getmethod") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments
                            .first()
                            .ok_or("ReflectionClass::getMethod() expects exactly 1 argument")?
                            .show();
                        let method = find_method(&target, &name.to_ascii_lowercase(), classes)
                            .ok_or_else(|| {
                                format!(
                                    "ReflectionException:Method {target}::{name}() does not exist"
                                )
                            })?;
                        let declaring_class = method
                            .declaring_class
                            .clone()
                            .unwrap_or_else(|| target.to_ascii_lowercase());
                        let declaring_class = classes
                            .get(&declaring_class.to_ascii_lowercase())
                            .map_or(declaring_class, |class| class.name.clone());
                        let display_name = classes
                            .get(&declaring_class.to_ascii_lowercase())
                            .and_then(|class| {
                                class
                                    .method_display_names
                                    .get(&method.name.to_ascii_lowercase())
                            })
                            .cloned()
                            .unwrap_or(name);
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionMethod".into(),
                            properties: HashMap::from([
                                ("\0target_class".into(), Value::String(declaring_class)),
                                ("\0method_name".into(), Value::String(display_name)),
                            ]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    ("reflectionclass" | "reflectionenum", "getproperty") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionClass target")?
                            .show();
                        let name = arguments
                            .first()
                            .ok_or("ReflectionClass::getProperty() expects exactly 1 argument")?
                            .show();
                        let mut current = Some(target.as_str());
                        let mut declaring = None;
                        while let Some(class_name) = current {
                            let Some(class) = classes.get(&class_name.to_ascii_lowercase()) else {
                                break;
                            };
                            let candidate = class
                                .properties
                                .get(&name)
                                .or_else(|| class.static_properties.get(&name));
                            if candidate.is_some_and(|property| {
                                class_name.eq_ignore_ascii_case(&target)
                                    || property.visibility != MemberVisibility::Private
                            }) {
                                declaring = Some(class.name.clone());
                                break;
                            }
                            current = class.parent.as_deref();
                        }
                        let declaring = declaring.ok_or_else(|| {
                            format!("ReflectionException:Property {target}::${name} does not exist")
                        })?;
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionProperty".into(),
                            properties: HashMap::from([
                                ("\0target_class".into(), Value::String(declaring)),
                                ("\0property_name".into(), Value::String(name)),
                            ]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    ("reflectionmethod", "getname") => internal
                        .get("\0method_name")
                        .cloned()
                        .ok_or("Invalid ReflectionMethod")?,
                    ("reflectionmethod", "getclosure") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        let receiver = if function.is_static {
                            if let Some(argument) = arguments.first() {
                                if !matches!(argument.dereference(), Value::Null | Value::Object(_))
                                {
                                    return Err(format!("TypeError:ReflectionMethod::getClosure(): Argument #1 ($object) must be of type ?object, {} given", argument.type_name()));
                                }
                            }
                            Value::String(target.clone())
                        } else {
                            let receiver = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null))
                                .ok_or_else(|| format!("ReflectionException:Trying to get closure for non static method {target}::{method_name}() without an object"))?
                                .dereference();
                            let Value::Object(object) = &receiver else {
                                return Err(format!("ReflectionException:Given object is not an instance of class {target}"));
                            };
                            if !super::is_a_type(
                                &object.borrow().class_name.to_ascii_lowercase(),
                                &target.to_ascii_lowercase(),
                                classes,
                                &mut HashSet::new(),
                            ) {
                                return Err(format!("ReflectionException:Given object is not an instance of class {target}"));
                            }
                            receiver
                        };
                        let target_callable = Value::from_array_entries(vec![
                            (ArrayKey::Integer(0), receiver),
                            (ArrayKey::Integer(1), Value::String(method_name)),
                        ]);
                        let wrapper = BytecodeFunction::new(
                            "{reflection-method-closure}".into(),
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            Vec::new(),
                            None,
                            false,
                            Vec::new(),
                            false,
                            false,
                            false,
                            MemberVisibility::Public,
                            None,
                        );
                        Value::Closure(Rc::new(std::cell::RefCell::new(PhpClosure {
                            function: wrapper,
                            captures: HashMap::new(),
                            capture_order: Vec::new(),
                            reference_captures: HashMap::new(),
                            callable_target: Some(target_callable),
                            callable_access_scope: Some(
                                function
                                    .declaring_class
                                    .clone()
                                    .unwrap_or_else(|| target.clone()),
                            ),
                        })))
                    }
                    ("reflectionmethod", "invoke" | "invokeargs") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        let receiver = if function.is_static {
                            Value::String(target.clone())
                        } else {
                            let receiver = arguments.first().filter(|value| !matches!(value.dereference(), Value::Null))
                                .ok_or_else(|| format!("ReflectionException:Trying to invoke non static method {target}::{method_name}() without an object"))?
                                .dereference();
                            let Value::Object(object) = &receiver else {
                                return Err(format!("ReflectionException:Given object is not an instance of class {target}"));
                            };
                            if !super::is_a_type(
                                &object.borrow().class_name.to_ascii_lowercase(),
                                &target.to_ascii_lowercase(),
                                classes,
                                &mut HashSet::new(),
                            ) {
                                return Err(format!("ReflectionException:Given object is not an instance of class {target}"));
                            }
                            receiver
                        };
                        let (invoke_args, invoke_references, invoke_names) = if method == "invoke" {
                            (
                                arguments.iter().skip(1).cloned().collect::<Vec<_>>(),
                                reference_names.iter().skip(1).cloned().collect::<Vec<_>>(),
                                argument_names.iter().skip(1).cloned().collect::<Vec<_>>(),
                            )
                        } else {
                            match arguments.get(1).map(Value::dereference) {
                                Some(Value::Array(values)) => {
                                    let mut values_out = Vec::with_capacity(values.len());
                                    let mut references_out = Vec::with_capacity(values.len());
                                    let mut names_out = Vec::with_capacity(values.len());
                                    for (key, value) in values.iter() {
                                        values_out.push(value.clone());
                                        references_out.push(match value {
                                            Value::Reference(reference) => Some(ReferenceArgument {
                                                variable: String::new(), array_keys: Vec::new(), object_property: None,
                                                direct_reference: Some(reference.clone()),
                                            }),
                                            _ => None,
                                        });
                                        names_out.push(match key { ArrayKey::String(name) => Some(name.clone()), ArrayKey::Integer(_) => None });
                                    }
                                    (values_out, references_out, names_out)
                                }
                                Some(value) => return Err(format!("TypeError:ReflectionMethod::invokeArgs(): Argument #2 ($args) must be of type array, {} given", value.type_name())),
                                None => return Err("ReflectionMethod::invokeArgs() expects exactly 2 arguments".into()),
                            }
                        };
                        let callable = Value::from_array_entries(vec![
                            (ArrayKey::Integer(0), receiver),
                            (ArrayKey::Integer(1), Value::String(method_name)),
                        ]);
                        invoke_array_callable_scoped(
                            &callable,
                            invoke_args,
                            &invoke_references,
                            &invoke_names,
                            variables,
                            bindings,
                            global_scope,
                            frame_id,
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                            false,
                            function.declaring_class.as_deref(),
                        )?
                    }
                    (
                        "reflectionmethod",
                        "getnumberofparameters"
                        | "getnumberofrequiredparameters"
                        | "hasreturntype"
                        | "getreturntype",
                    ) => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        match method.as_str() {
                            "getnumberofparameters" => {
                                Value::Integer(function.parameters.len() as i64)
                            }
                            "getnumberofrequiredparameters" => Value::Integer(
                                function
                                    .parameters
                                    .iter()
                                    .enumerate()
                                    .filter(|(index, _)| {
                                        function
                                            .parameter_defaults
                                            .get(*index)
                                            .is_none_or(Option::is_none)
                                            && !function
                                                .parameter_variadic
                                                .get(*index)
                                                .copied()
                                                .unwrap_or(false)
                                    })
                                    .map(|(index, _)| index + 1)
                                    .max()
                                    .unwrap_or(0) as i64,
                            ),
                            "hasreturntype" => Value::Bool(function.return_type.is_some()),
                            "getreturntype" => function
                                .return_type
                                .as_deref()
                                .map(reflection_type)
                                .unwrap_or(Value::Null),
                            _ => unreachable!(),
                        }
                    }
                    ("reflectionmethod", "setaccessible") => {
                        let _accessible = arguments
                            .first()
                            .ok_or("ReflectionMethod::setAccessible() expects exactly 1 argument")?
                            .truthy();
                        global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Deprecated,
                            message: "Method ReflectionMethod::setAccessible() is deprecated since 8.5, as it has no effect since PHP 8.1".into(),
                            source_file: None, source_line: None, output_offset: 0,
                        });
                        Value::Null
                    }
                    ("reflectionmethod", "getdeclaringclass") => {
                        let declaring = internal
                            .get("\0target_class")
                            .cloned()
                            .ok_or("Invalid ReflectionMethod target")?;
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionClass".into(),
                            properties: HashMap::from([("\0target_class".into(), declaring)]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    ("reflectionmethod", "getmodifiers") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        Value::Integer(reflection_method_modifiers(function))
                    }
                    ("reflectionmethod", "hasprototype" | "getprototype") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let prototype = reflection_method_prototype(&target, &method_name, classes);
                        if method == "hasprototype" {
                            Value::Bool(prototype.is_some())
                        } else {
                            let (class, name) = prototype.ok_or_else(|| format!("ReflectionException:Method {target}::{method_name}() does not have a prototype"))?;
                            Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionMethod".into(),
                                properties: HashMap::from([
                                    ("\0target_class".into(), Value::String(class)),
                                    ("\0method_name".into(), Value::String(name)),
                                ]),
                                property_order: Vec::new(),
                                active_property_hooks: Default::default(),
                                exception: None,
                            })))
                        }
                    }
                    (
                        "reflectionmethod",
                        "ispublic" | "isprotected" | "isprivate" | "isstatic" | "isabstract"
                        | "isfinal" | "isconstructor" | "isdestructor",
                    ) => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        let matches = match method.as_str() {
                            "ispublic" => function.visibility == MemberVisibility::Public,
                            "isprotected" => function.visibility == MemberVisibility::Protected,
                            "isprivate" => function.visibility == MemberVisibility::Private,
                            "isstatic" => function.is_static,
                            "isabstract" => function.is_abstract,
                            "isfinal" => function.is_final,
                            "isconstructor" => method_name.eq_ignore_ascii_case("__construct"),
                            "isdestructor" => method_name.eq_ignore_ascii_case("__destruct"),
                            _ => false,
                        };
                        Value::Bool(matches)
                    }
                    ("reflectionmethod", "isvariadic" | "returnsreference" | "isclosure") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        Value::Bool(match method.as_str() {
                            "isvariadic" => {
                                function.parameter_variadic.iter().any(|variadic| *variadic)
                            }
                            "returnsreference" => function.returns_by_reference,
                            "isclosure" => false,
                            _ => false,
                        })
                    }
                    ("reflectionmethod", "getparameters") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let function =
                            find_method(&target, &method_name.to_ascii_lowercase(), classes)
                                .ok_or("Reflected method no longer exists")?;
                        let entries = function
                            .parameters
                            .iter()
                            .enumerate()
                            .map(|(position, name)| {
                                (
                                    ArrayKey::Integer(position as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionParameter".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(target.clone()),
                                                ),
                                                (
                                                    "\0method_name".into(),
                                                    Value::String(method_name.to_ascii_lowercase()),
                                                ),
                                                (
                                                    "\0parameter_name".into(),
                                                    Value::String(name.clone()),
                                                ),
                                                (
                                                    "\0parameter_position".into(),
                                                    Value::Integer(position as i64),
                                                ),
                                                (
                                                    "\0parameter_type".into(),
                                                    function
                                                        .parameter_types
                                                        .get(position)
                                                        .and_then(Clone::clone)
                                                        .map(Value::String)
                                                        .unwrap_or(Value::Null),
                                                ),
                                                (
                                                    "\0parameter_by_reference".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_by_reference
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_variadic".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_variadic
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_promoted".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_promoted
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_has_default".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_defaults
                                                            .get(position)
                                                            .is_some_and(Option::is_some),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_optional".into(),
                                                    Value::Bool(
                                                        function
                                                            .parameter_variadic
                                                            .get(position)
                                                            .copied()
                                                            .unwrap_or(false)
                                                            || (function
                                                                .parameter_defaults
                                                                .get(position)
                                                                .is_some_and(Option::is_some)
                                                                && position
                                                                    >= required_parameter_count(
                                                                        &function,
                                                                    )),
                                                    ),
                                                ),
                                                (
                                                    "\0parameter_default".into(),
                                                    function
                                                        .parameter_defaults
                                                        .get(position)
                                                        .and_then(Clone::clone)
                                                        .unwrap_or(Value::Null),
                                                ),
                                                (
                                                    "\0parameter_default_constant_name".into(),
                                                    function
                                                        .parameter_default_constant_names
                                                        .get(position)
                                                        .and_then(Clone::clone)
                                                        .map(Value::String)
                                                        .unwrap_or(Value::Null),
                                                ),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionparameter", "getname") => internal
                        .get("\0parameter_name")
                        .cloned()
                        .ok_or("Invalid ReflectionParameter")?,
                    ("reflectionparameter", "getposition") => internal
                        .get("\0parameter_position")
                        .cloned()
                        .ok_or("Invalid ReflectionParameter")?,
                    ("reflectionparameter", "getdeclaringclass") => {
                        match internal.get("\0target_class") {
                            Some(target) => {
                                Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                    class_name: "ReflectionClass".into(),
                                    properties: HashMap::from([(
                                        "\0target_class".into(),
                                        target.clone(),
                                    )]),
                                    property_order: Vec::new(),
                                    active_property_hooks: Default::default(),
                                    exception: None,
                                })))
                            }
                            None => Value::Null,
                        }
                    }
                    ("reflectionparameter", "getdeclaringfunction") => {
                        if let Some(target) = internal.get("\0target_class") {
                            let method_name = internal
                                .get("\0method_name")
                                .cloned()
                                .ok_or("Invalid ReflectionParameter method")?;
                            Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionMethod".into(),
                                properties: HashMap::from([
                                    ("\0target_class".into(), target.clone()),
                                    ("\0method_name".into(), method_name),
                                ]),
                                property_order: Vec::new(),
                                active_property_hooks: Default::default(),
                                exception: None,
                            })))
                        } else if let Some(function_name) = internal
                            .get("\0function_name")
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                        {
                            let mut properties =
                                HashMap::from([("\0function_name".into(), function_name.clone())]);
                            if let Some(closure) = internal
                                .get("\0closure")
                                .filter(|value| !matches!(value.dereference(), Value::Null))
                            {
                                properties.insert("\0closure".into(), closure.clone());
                            }
                            Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                                class_name: "ReflectionFunction".into(),
                                properties,
                                property_order: Vec::new(),
                                active_property_hooks: Default::default(),
                                exception: None,
                            })))
                        } else {
                            Value::Null
                        }
                    }
                    ("reflectionparameter", "ispassedbyreference") => internal
                        .get("\0parameter_by_reference")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                    ("reflectionparameter", "canbepassedbyvalue") => Value::Bool(
                        !internal
                            .get("\0parameter_by_reference")
                            .is_some_and(Value::truthy),
                    ),
                    ("reflectionparameter", "isvariadic") => internal
                        .get("\0parameter_variadic")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                    ("reflectionparameter", "ispromoted") => internal
                        .get("\0parameter_promoted")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                    ("reflectionparameter", "isoptional") => internal
                        .get("\0parameter_optional")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                    ("reflectionparameter", "isdefaultvalueavailable") => internal
                        .get("\0parameter_has_default")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                    ("reflectionparameter", "isdefaultvalueconstant") => Value::Bool(
                        internal
                            .get("\0parameter_default_constant_name")
                            .is_some_and(|value| !matches!(value.dereference(), Value::Null)),
                    ),
                    ("reflectionparameter", "getdefaultvalueconstantname") => {
                        if !internal
                            .get("\0parameter_has_default")
                            .is_some_and(Value::truthy)
                        {
                            let name = internal
                                .get("\0parameter_name")
                                .map(Value::show)
                                .unwrap_or_default();
                            return Err(format!("ReflectionException:Parameter ${name} does not have a default value"));
                        }
                        internal
                            .get("\0parameter_default_constant_name")
                            .cloned()
                            .unwrap_or(Value::Null)
                    }
                    ("reflectionparameter", "getdefaultvalue") => {
                        if !internal
                            .get("\0parameter_has_default")
                            .is_some_and(Value::truthy)
                        {
                            let name = internal
                                .get("\0parameter_name")
                                .map(Value::show)
                                .unwrap_or_default();
                            return Err(format!("ReflectionException:Parameter ${name} does not have a default value"));
                        }
                        internal
                            .get("\0parameter_default")
                            .cloned()
                            .unwrap_or(Value::Null)
                    }
                    ("reflectionparameter", "hastype") => Value::Bool(
                        internal
                            .get("\0parameter_type")
                            .is_some_and(|value| !matches!(value.dereference(), Value::Null)),
                    ),
                    ("reflectionparameter", "gettype") => internal
                        .get("\0parameter_type")
                        .filter(|value| !matches!(value.dereference(), Value::Null))
                        .map(|value| reflection_type(&value.show()))
                        .unwrap_or(Value::Null),
                    ("reflectionparameter", "allowsnull") => {
                        match internal
                            .get("\0parameter_type")
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                        {
                            None => Value::Bool(true),
                            Some(type_name) => {
                                let reflected_type = reflection_type(&type_name.show());
                                let type_allows_null = match reflected_type.dereference() {
                                    Value::Object(object) => object
                                        .borrow()
                                        .properties
                                        .get("\0allows_null")
                                        .is_some_and(Value::truthy),
                                    _ => false,
                                };
                                let default_is_null = internal
                                    .get("\0parameter_has_default")
                                    .is_some_and(Value::truthy)
                                    && internal.get("\0parameter_default").is_some_and(|value| {
                                        matches!(value.dereference(), Value::Null)
                                    });
                                Value::Bool(type_allows_null || default_is_null)
                            }
                        }
                    }
                    ("reflectionparameter", "getclass" | "isarray" | "iscallable") => {
                        let method_name = match method.as_str() {
                            "getclass" => "getClass",
                            "isarray" => "isArray",
                            _ => "isCallable",
                        };
                        global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Deprecated,
                            message: format!("Method ReflectionParameter::{method_name}() is deprecated since 8.0, use ReflectionParameter::getType() instead"),
                            source_file: None, source_line: None, output_offset: 0,
                        });
                        let type_name = internal
                            .get("\0parameter_type")
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let names: Vec<String> = type_name
                            .as_deref()
                            .map(|name| {
                                let normalized = name.strip_prefix('?').unwrap_or(name);
                                let normalized = strip_type_parentheses(normalized);
                                let union = split_top_level_type(normalized, '|');
                                if union.is_empty() {
                                    vec![normalized.to_string()]
                                } else {
                                    union.into_iter().map(str::to_string).collect()
                                }
                            })
                            .unwrap_or_default();
                        match method.as_str() {
                            "isarray" => Value::Bool(
                                names.iter().any(|name| name.eq_ignore_ascii_case("array")),
                            ),
                            "iscallable" => Value::Bool(
                                names
                                    .iter()
                                    .any(|name| name.eq_ignore_ascii_case("callable")),
                            ),
                            _ => {
                                let class_name = (names.len() == 1
                                    && !matches!(
                                        names[0].to_ascii_lowercase().as_str(),
                                        "int"
                                            | "float"
                                            | "string"
                                            | "bool"
                                            | "array"
                                            | "object"
                                            | "callable"
                                            | "iterable"
                                            | "void"
                                            | "never"
                                            | "mixed"
                                            | "null"
                                            | "false"
                                            | "true"
                                    ))
                                .then(|| names[0].clone());
                                class_name
                                    .map(|name| {
                                        Value::Object(Rc::new(std::cell::RefCell::new(
                                            crate::PhpObject {
                                                class_name: "ReflectionClass".into(),
                                                properties: HashMap::from([(
                                                    "\0target_class".into(),
                                                    Value::String(name),
                                                )]),
                                                property_order: Vec::new(),
                                                active_property_hooks: Default::default(),
                                                exception: None,
                                            },
                                        )))
                                    })
                                    .unwrap_or(Value::Null)
                            }
                        }
                    }
                    ("reflectionparameter", "getattributes") => {
                        if let Some(Value::Array(attributes)) = internal
                            .get("\0parameter_attributes")
                            .map(Value::dereference)
                        {
                            let filter = arguments
                                .first()
                                .filter(|value| !matches!(value.dereference(), Value::Null))
                                .map(Value::show);
                            let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                            if flags & !2 != 0 {
                                return Err(
                                    "ReflectionParameter::getAttributes(): unsupported flags"
                                        .into(),
                                );
                            }
                            let filtered = attributes
                                .iter()
                                .filter(|(_, attribute)| {
                                    let Value::Object(attribute) = attribute.dereference() else {
                                        return false;
                                    };
                                    let attribute = attribute.borrow();
                                    filter.as_ref().is_none_or(|name| {
                                        attribute.properties.get("\0attribute_name").is_some_and(
                                            |value| {
                                                let actual = value.show();
                                                actual.eq_ignore_ascii_case(name)
                                                    || flags & 2 != 0
                                                        && super::is_a_type(
                                                            &actual.to_ascii_lowercase(),
                                                            &name.to_ascii_lowercase(),
                                                            classes,
                                                            &mut HashSet::new(),
                                                        )
                                            },
                                        )
                                    })
                                })
                                .cloned()
                                .collect();
                            return {
                                stack.push(Value::from_array_entries(filtered));
                                Ok(true)
                            };
                        }
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionParameter target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionParameter method")?
                            .show();
                        let position = internal
                            .get("\0parameter_position")
                            .map_or(usize::MAX, |value| value.number() as usize);
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let attributes = class
                            .parameter_attributes
                            .get(&method_name.to_ascii_lowercase())
                            .and_then(|items| items.get(position))
                            .map(Vec::as_slice)
                            .unwrap_or_default();
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionParameter::getAttributes(): unsupported flags".into()
                            );
                        }
                        let entries = attributes
                            .iter()
                            .filter(|attribute| {
                                filter.as_ref().is_none_or(|name| {
                                    attribute.name.eq_ignore_ascii_case(name)
                                        || (flags & 2 != 0
                                            && super::is_a_type(
                                                &attribute.name,
                                                name,
                                                classes,
                                                &mut HashSet::new(),
                                            ))
                                })
                            })
                            .enumerate()
                            .map(|(index, attribute)| {
                                (
                                    ArrayKey::Integer(index as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionAttribute".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0attribute_name".into(),
                                                    Value::String(attribute.name.clone()),
                                                ),
                                                (
                                                    "\0attribute_arguments".into(),
                                                    reflection_attribute_arguments(
                                                        &attribute.arguments,
                                                    ),
                                                ),
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(class.name.clone()),
                                                ),
                                                ("\0target_type".into(), Value::Integer(32)),
                                                (
                                                    "\0target_member".into(),
                                                    Value::String(method_name.to_ascii_lowercase()),
                                                ),
                                                (
                                                    "\0target_position".into(),
                                                    Value::Integer(position as i64),
                                                ),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionmethod", "getattributes") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionMethod target")?
                            .show();
                        let method_name = internal
                            .get("\0method_name")
                            .ok_or("Invalid ReflectionMethod name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let attributes = class
                            .method_attributes
                            .get(&method_name.to_ascii_lowercase())
                            .map(Vec::as_slice)
                            .unwrap_or_default();
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionMethod::getAttributes(): unsupported flags".into()
                            );
                        }
                        let entries = attributes
                            .iter()
                            .filter(|attribute| {
                                filter.as_ref().is_none_or(|name| {
                                    attribute.name.eq_ignore_ascii_case(name)
                                        || (flags & 2 != 0
                                            && super::is_a_type(
                                                &attribute.name,
                                                name,
                                                classes,
                                                &mut HashSet::new(),
                                            ))
                                })
                            })
                            .enumerate()
                            .map(|(index, attribute)| {
                                (
                                    ArrayKey::Integer(index as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionAttribute".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0attribute_name".into(),
                                                    Value::String(attribute.name.clone()),
                                                ),
                                                (
                                                    "\0attribute_arguments".into(),
                                                    reflection_attribute_arguments(
                                                        &attribute.arguments,
                                                    ),
                                                ),
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(class.name.clone()),
                                                ),
                                                ("\0target_type".into(), Value::Integer(4)),
                                                (
                                                    "\0target_member".into(),
                                                    Value::String(method_name.to_ascii_lowercase()),
                                                ),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionproperty", "getname") => internal
                        .get("\0property_name")
                        .cloned()
                        .ok_or("Invalid ReflectionProperty")?,
                    ("reflectionproperty", "getvalue") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionProperty target")?
                            .show();
                        let property = internal
                            .get("\0property_name")
                            .ok_or("Invalid ReflectionProperty name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let metadata = class
                            .properties
                            .get(&property)
                            .or_else(|| class.static_properties.get(&property))
                            .ok_or("Reflected property no longer exists")?;
                        if class.static_properties.contains_key(&property) {
                            static_properties.get(&(target.to_ascii_lowercase(), property.clone())).cloned()
                                .ok_or_else(|| format!("Error:Typed static property {}::${property} must not be accessed before initialization", class.name))?
                        } else {
                            let object = match arguments.first().map(Value::dereference) {
                                Some(Value::Object(object)) => object,
                                None | Some(Value::Null) => return Err(format!("ReflectionException:ReflectionProperty::getValue() for non-static property {}::${property} requires an object", class.name)),
                                Some(value) => return Err(format!("TypeError:ReflectionProperty::getValue(): Argument #1 ($object) must be of type ?object, {} given", value.type_name())),
                            };
                            let object_class = object.borrow().class_name.clone();
                            if !super::is_a_type(
                                &object_class.to_ascii_lowercase(),
                                &class.name,
                                classes,
                                &mut HashSet::new(),
                            ) {
                                return Err(format!("ReflectionException:Given object is not an instance of the class {}", class.name));
                            }
                            super::objects::read_property(
                                &Value::Object(object),
                                &property,
                                Some(&metadata.declaring_class),
                                functions,
                                classes,
                                static_properties,
                                global_variables,
                                output,
                                steps,
                            )?
                            .dereference()
                        }
                    }
                    ("reflectionproperty", "setvalue") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionProperty target")?
                            .show();
                        let property = internal
                            .get("\0property_name")
                            .ok_or("Invalid ReflectionProperty name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let metadata = class
                            .properties
                            .get(&property)
                            .or_else(|| class.static_properties.get(&property))
                            .ok_or("Reflected property no longer exists")?;
                        let object_argument = arguments.first().map(Value::dereference);
                        let value = arguments
                            .get(1)
                            .ok_or("ReflectionProperty::setValue() expects exactly 2 arguments")?
                            .dereference();
                        if class.static_properties.contains_key(&property) {
                            if !matches!(object_argument, None | Some(Value::Null)) {
                                return Err("ReflectionException:Static property ReflectionProperty::setValue() expects null for the object argument".into());
                            }
                            if metadata.is_readonly
                                && static_properties
                                    .contains_key(&(target.to_ascii_lowercase(), property.clone()))
                            {
                                return Err(format!(
                                    "Error:Cannot modify readonly property {}::${property}",
                                    class.name
                                ));
                            }
                            if let Some(type_name) = &metadata.type_name {
                                super::objects::ensure_property_type(
                                    &value, type_name, &property, classes,
                                )?;
                            }
                            static_properties
                                .insert((target.to_ascii_lowercase(), property), value);
                        } else {
                            let object = match object_argument {
                                Some(Value::Object(object)) => object,
                                None | Some(Value::Null) => return Err(format!("ReflectionException:ReflectionProperty::setValue() for non-static property {}::${property} requires an object", class.name)),
                                Some(other) => return Err(format!("TypeError:ReflectionProperty::setValue(): Argument #1 ($object) must be of type ?object, {} given", other.type_name())),
                            };
                            let object_class = object.borrow().class_name.clone();
                            if !super::is_a_type(
                                &object_class.to_ascii_lowercase(),
                                &class.name,
                                classes,
                                &mut HashSet::new(),
                            ) {
                                return Err(format!("ReflectionException:Given object is not an instance of the class {}", class.name));
                            }
                            super::objects::assign_property(
                                &Value::Object(object),
                                &property,
                                value,
                                Some(&metadata.declaring_class),
                                true,
                                functions,
                                classes,
                                static_properties,
                                global_variables,
                                output,
                                steps,
                            )?;
                        }
                        Value::Null
                    }
                    ("reflectionproperty", "setaccessible") => {
                        let _accessible = arguments
                            .first()
                            .ok_or(
                                "ReflectionProperty::setAccessible() expects exactly 1 argument",
                            )?
                            .truthy();
                        global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Deprecated,
                            message: "Method ReflectionProperty::setAccessible() is deprecated since 8.5, as it has no effect since PHP 8.1".into(),
                            source_file: None, source_line: None, output_offset: 0,
                        });
                        Value::Null
                    }
                    ("reflectionproperty", "getdeclaringclass") => {
                        let declaring = internal
                            .get("\0target_class")
                            .cloned()
                            .ok_or("Invalid ReflectionProperty target")?;
                        Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
                            class_name: "ReflectionClass".into(),
                            properties: HashMap::from([("\0target_class".into(), declaring)]),
                            property_order: Vec::new(),
                            active_property_hooks: Default::default(),
                            exception: None,
                        })))
                    }
                    (
                        "reflectionproperty",
                        "hastype" | "gettype" | "getmodifiers" | "ispublic" | "isprotected"
                        | "isprivate" | "isstatic" | "isreadonly" | "isprivateset"
                        | "isprotectedset" | "ispromoted",
                    ) => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionProperty target")?
                            .show();
                        let property = internal
                            .get("\0property_name")
                            .ok_or("Invalid ReflectionProperty name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let metadata = class
                            .properties
                            .get(&property)
                            .or_else(|| class.static_properties.get(&property))
                            .ok_or("Reflected property no longer exists")?;
                        if method == "hastype" {
                            Value::Bool(metadata.type_name.is_some())
                        } else if method == "gettype" {
                            metadata
                                .type_name
                                .as_deref()
                                .map(reflection_type)
                                .unwrap_or(Value::Null)
                        } else {
                            let visibility = match metadata.visibility {
                                crate::MemberVisibility::Public => 1,
                                crate::MemberVisibility::Protected => 2,
                                crate::MemberVisibility::Private => 4,
                            };
                            let set_modifier = match metadata.set_visibility {
                                Some(crate::MemberVisibility::Private) => 32,
                                Some(crate::MemberVisibility::Protected) => 64,
                                _ => 0,
                            };
                            let modifiers = visibility
                                | set_modifier
                                | if metadata.default.is_some()
                                    && class.static_properties.contains_key(&property)
                                {
                                    16
                                } else {
                                    0
                                }
                                | if metadata.is_readonly { 128 } else { 0 };
                            match method.as_str() {
                                "getmodifiers" => Value::Integer(modifiers),
                                "ispublic" => Value::Bool(
                                    metadata.visibility == crate::MemberVisibility::Public,
                                ),
                                "isprotected" => Value::Bool(
                                    metadata.visibility == crate::MemberVisibility::Protected,
                                ),
                                "isprivate" => Value::Bool(
                                    metadata.visibility == crate::MemberVisibility::Private,
                                ),
                                "isstatic" => {
                                    Value::Bool(class.static_properties.contains_key(&property))
                                }
                                "isreadonly" => Value::Bool(metadata.is_readonly),
                                "isprivateset" => Value::Bool(
                                    metadata.set_visibility
                                        == Some(crate::MemberVisibility::Private),
                                ),
                                "isprotectedset" => Value::Bool(
                                    metadata.set_visibility
                                        == Some(crate::MemberVisibility::Protected),
                                ),
                                "ispromoted" => Value::Bool(metadata.is_promoted),
                                _ => unreachable!(),
                            }
                        }
                    }
                    ("reflectionproperty", "isinitialized") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionProperty target")?
                            .show();
                        let property = internal
                            .get("\0property_name")
                            .ok_or("Invalid ReflectionProperty name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let metadata = class
                            .properties
                            .get(&property)
                            .or_else(|| class.static_properties.get(&property))
                            .ok_or("Reflected property no longer exists")?;
                        let object_argument = match arguments.first().map(Value::dereference) {
                            None | Some(Value::Null) => None,
                            Some(Value::Object(object)) => Some(object),
                            Some(value) => return Err(format!("TypeError:ReflectionProperty::isInitialized(): Argument #1 ($object) must be of type ?object, {} given", value.type_name())),
                        };
                        if class.static_properties.contains_key(&property) {
                            Value::Bool(
                                static_properties
                                    .contains_key(&(target.to_ascii_lowercase(), property)),
                            )
                        } else {
                            let object = object_argument
                                .ok_or_else(|| format!("ReflectionException:Trying to check initialization of non-static property {}::${property} without an object", class.name))?;
                            let object_class = object.borrow().class_name.clone();
                            if !super::is_a_type(
                                &object_class,
                                &class.name,
                                classes,
                                &mut HashSet::new(),
                            ) {
                                return Err(format!("ReflectionException:Given object is not an instance of the class {}", class.name));
                            }
                            let storage_key = instance_property_storage_key(metadata, &property);
                            let initialized = object.borrow().properties.contains_key(&storage_key);
                            Value::Bool(initialized)
                        }
                    }
                    ("reflectionproperty", "hasdefaultvalue" | "getdefaultvalue") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionProperty target")?
                            .show();
                        let property = internal
                            .get("\0property_name")
                            .ok_or("Invalid ReflectionProperty name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let metadata = class
                            .properties
                            .get(&property)
                            .or_else(|| class.static_properties.get(&property))
                            .ok_or("Reflected property no longer exists")?;
                        let has_default = metadata.default.is_some()
                            || (metadata.type_name.is_none() && !metadata.is_promoted);
                        if method == "hasdefaultvalue" {
                            Value::Bool(has_default)
                        } else {
                            if !has_default {
                                global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                    level: crate::DiagnosticLevel::Deprecated,
                                    message: "ReflectionProperty::getDefaultValue() for a property without a default value is deprecated, use ReflectionProperty::hasDefaultValue() to check if the default value exists".into(),
                                    source_file: None, source_line: None, output_offset: 0,
                                });
                            }
                            metadata.default.clone().unwrap_or(Value::Null)
                        }
                    }
                    ("reflectionproperty", "getattributes") => {
                        let target = internal
                            .get("\0target_class")
                            .ok_or("Invalid ReflectionProperty target")?
                            .show();
                        let property = internal
                            .get("\0property_name")
                            .ok_or("Invalid ReflectionProperty name")?
                            .show();
                        let class = classes
                            .get(&target.to_ascii_lowercase())
                            .ok_or("Reflected class no longer exists")?;
                        let attributes = class
                            .property_attributes
                            .get(&property)
                            .map(Vec::as_slice)
                            .unwrap_or_default();
                        let filter = arguments
                            .first()
                            .filter(|value| !matches!(value.dereference(), Value::Null))
                            .map(Value::show);
                        let flags = arguments.get(1).map_or(0, |flags| flags.number() as i64);
                        if flags & !2 != 0 {
                            return Err(
                                "ReflectionProperty::getAttributes(): unsupported flags".into()
                            );
                        }
                        let entries = attributes
                            .iter()
                            .filter(|attribute| {
                                filter.as_ref().is_none_or(|name| {
                                    attribute.name.eq_ignore_ascii_case(name)
                                        || (flags & 2 != 0
                                            && super::is_a_type(
                                                &attribute.name,
                                                name,
                                                classes,
                                                &mut HashSet::new(),
                                            ))
                                })
                            })
                            .enumerate()
                            .map(|(index, attribute)| {
                                (
                                    ArrayKey::Integer(index as i64),
                                    Value::Object(Rc::new(std::cell::RefCell::new(
                                        crate::PhpObject {
                                            class_name: "ReflectionAttribute".into(),
                                            properties: HashMap::from([
                                                (
                                                    "\0attribute_name".into(),
                                                    Value::String(attribute.name.clone()),
                                                ),
                                                (
                                                    "\0attribute_arguments".into(),
                                                    reflection_attribute_arguments(
                                                        &attribute.arguments,
                                                    ),
                                                ),
                                                (
                                                    "\0target_class".into(),
                                                    Value::String(class.name.clone()),
                                                ),
                                                ("\0target_type".into(), Value::Integer(8)),
                                                (
                                                    "\0target_member".into(),
                                                    Value::String(property.clone()),
                                                ),
                                            ]),
                                            property_order: Vec::new(),
                                            active_property_hooks: Default::default(),
                                            exception: None,
                                        },
                                    ))),
                                )
                            })
                            .collect();
                        Value::from_array_entries(entries)
                    }
                    ("reflectionattribute", "getname") => internal
                        .get("\0attribute_name")
                        .cloned()
                        .ok_or("Invalid ReflectionAttribute")?,
                    ("reflectionattribute", "getarguments") => internal
                        .get("\0attribute_arguments")
                        .cloned()
                        .ok_or("Invalid ReflectionAttribute")?,
                    ("reflectionattribute", "gettarget") => internal
                        .get("\0target_type")
                        .cloned()
                        .unwrap_or(Value::Integer(1)),
                    ("reflectionattribute", "isrepeated") => {
                        if let Some(repeated) = internal.get("\0is_repeated") {
                            Value::Bool(repeated.truthy())
                        } else {
                            let name = internal
                                .get("\0attribute_name")
                                .ok_or("Invalid ReflectionAttribute")?
                                .show();
                            let target = internal
                                .get("\0target_class")
                                .ok_or("Invalid ReflectionAttribute target")?
                                .show();
                            let target_type = internal
                                .get("\0target_type")
                                .map_or(1, |value| value.number() as i64);
                            let count = if target_type == 2 {
                                functions
                                    .get(&target.to_ascii_lowercase())
                                    .map_or(0, |function| {
                                        function
                                            .attributes
                                            .iter()
                                            .filter(|attribute| {
                                                attribute.name.eq_ignore_ascii_case(&name)
                                            })
                                            .count()
                                    })
                            } else if target_type == 64 {
                                global_variables.constant_attributes.get(&target).map_or(
                                    0,
                                    |attributes| {
                                        attributes
                                            .iter()
                                            .filter(|attribute| {
                                                attribute.name.eq_ignore_ascii_case(&name)
                                            })
                                            .count()
                                    },
                                )
                            } else {
                                classes
                                    .get(&target.to_ascii_lowercase())
                                    .map_or(0, |class| {
                                        if target_type == 4 {
                                            let method = internal
                                                .get("\0target_member")
                                                .map(Value::show)
                                                .unwrap_or_default();
                                            class
                                                .method_attributes
                                                .get(&method.to_ascii_lowercase())
                                                .map_or(0, |attributes| {
                                                    attributes
                                                        .iter()
                                                        .filter(|attribute| {
                                                            attribute
                                                                .name
                                                                .eq_ignore_ascii_case(&name)
                                                        })
                                                        .count()
                                                })
                                        } else if target_type == 8 {
                                            let property = internal
                                                .get("\0target_member")
                                                .map(Value::show)
                                                .unwrap_or_default();
                                            class.property_attributes.get(&property).map_or(
                                                0,
                                                |attributes| {
                                                    attributes
                                                        .iter()
                                                        .filter(|attribute| {
                                                            attribute
                                                                .name
                                                                .eq_ignore_ascii_case(&name)
                                                        })
                                                        .count()
                                                },
                                            )
                                        } else if target_type == 16 {
                                            let constant = internal
                                                .get("\0target_member")
                                                .map(Value::show)
                                                .unwrap_or_default();
                                            class.constants.get(&constant).map_or(0, |constant| {
                                                constant
                                                    .attributes
                                                    .iter()
                                                    .filter(|attribute| {
                                                        attribute.name.eq_ignore_ascii_case(&name)
                                                    })
                                                    .count()
                                            })
                                        } else if target_type == 32 {
                                            let method = internal
                                                .get("\0target_member")
                                                .map(Value::show)
                                                .unwrap_or_default();
                                            let position = internal
                                                .get("\0target_position")
                                                .map_or(usize::MAX, |value| {
                                                    value.number() as usize
                                                });
                                            class
                                                .parameter_attributes
                                                .get(&method.to_ascii_lowercase())
                                                .and_then(|items| items.get(position))
                                                .map_or(0, |attributes| {
                                                    attributes
                                                        .iter()
                                                        .filter(|attribute| {
                                                            attribute
                                                                .name
                                                                .eq_ignore_ascii_case(&name)
                                                        })
                                                        .count()
                                                })
                                        } else {
                                            class
                                                .attributes
                                                .iter()
                                                .filter(|attribute| {
                                                    attribute.name.eq_ignore_ascii_case(&name)
                                                })
                                                .count()
                                        }
                                    })
                            };
                            Value::Bool(count > 1)
                        }
                    }
                    ("reflectionattribute", "newinstance") => instantiate_reflection_attribute(
                        &internal,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?,
                    _ => {
                        return Err(format!(
                            "Call to undefined method {class_name}::{method_display}()"
                        ))
                    }
                };
                stack.push(result);
                return Ok(true);
            }
            if object.borrow().exception.is_some()
                && find_method(&class_name, &method, classes).is_none()
            {
                super::builtin_arguments::bind(
                    &format!("exception::{method}"),
                    arguments.clone(),
                    &reference_names,
                    &argument_names,
                )?;
                stack.push(crate::exception_builtins::call_method(
                    &Value::Object(object),
                    &method,
                    &arguments,
                )?);
                return Ok(true);
            }
            if class_name.eq_ignore_ascii_case("pdo")
                || class_name.eq_ignore_ascii_case("pdostatement")
            {
                let method_signature = format!("{}::{method}", class_name.to_ascii_lowercase());
                let bound = super::builtin_arguments::bind(
                    &method_signature,
                    arguments,
                    &reference_names,
                    &argument_names,
                )?;
                arguments = bound.0;
                reference_names = bound.1;
                if class_name.eq_ignore_ascii_case("pdostatement")
                    && matches!(
                        method.to_ascii_lowercase().as_str(),
                        "bindparam" | "bindcolumn"
                    )
                {
                    let target = reference_names.get(1).and_then(Option::as_ref)
                        .ok_or_else(|| format!("PDOStatement::{method}(): argument #2 ($var) must be passed by reference"))?;
                    let cell = reference_cell_for_argument(
                        target,
                        variables,
                        bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )?;
                    if arguments.len() < 2 {
                        return Err(format!(
                            "PDOStatement::{method}() expects at least 2 arguments"
                        ));
                    }
                    arguments[1] = Value::Reference(cell);
                }
                let mut instantiate_row =
                    |row: &Value, fetched_class: &str, constructor_args: &[Value]| {
                        if fetched_class == "\0PDO_FETCH_FUNC" {
                            let packed = Value::from_array_entries(
                                constructor_args
                                    .iter()
                                    .enumerate()
                                    .map(|(index, value)| {
                                        (ArrayKey::Integer(index as i64), value.clone())
                                    })
                                    .collect(),
                            );
                            let (instruction, mut callback_stack) =
                                callback_call(row.clone(), packed, false)?;
                            execute_instruction(
                                &instruction,
                                &mut callback_stack,
                                variables,
                                bindings,
                                global_variables,
                                global_scope,
                                frame_id,
                                functions,
                                classes,
                                static_properties,
                                output,
                                steps,
                            )?;
                            return callback_stack.pop().ok_or_else(|| {
                                "Invalid PDO::FETCH_FUNC callback result stack".to_string()
                            });
                        }
                        let (fetched_class, props_late) = fetched_class
                            .strip_prefix("\0PDO_FETCH_PROPS_LATE:")
                            .map_or((fetched_class, false), |name| (name, true));
                        instantiate_pdo_row(
                            row,
                            fetched_class,
                            constructor_args,
                            props_late,
                            variables,
                            bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                            functions,
                            classes,
                            static_properties,
                            output,
                            steps,
                        )
                    };
                stack.push(crate::pdo::call_method(
                    &object,
                    &method,
                    &arguments,
                    &mut instantiate_row,
                )?);
                return Ok(true);
            }
            if class_name.to_ascii_lowercase().starts_with("mongodb\\") {
                let method_signature = format!("{}::{method}", class_name.to_ascii_lowercase());
                arguments = super::builtin_arguments::bind(
                    &method_signature,
                    arguments,
                    &reference_names,
                    &argument_names,
                )?
                .0;
                stack.push(crate::mongo_php::call_method(&object, &method, &arguments)?);
                return Ok(true);
            }
            let function = match find_method(&class_name, &method, classes) {
                Some(function)
                    if ensure_access(
                        function.visibility,
                        function.declaring_class.as_deref(),
                        access_scope.as_deref(),
                        classes,
                        &method,
                    )
                    .is_ok() =>
                {
                    function
                }
                _ => {
                    let magic = find_method(&class_name, "__call", classes).ok_or_else(|| {
                        format!("Call to undefined method {class_name}::{method_display}()")
                    })?;
                    let result = invoke_magic_method(
                        &Value::Object(object.clone()),
                        magic,
                        &method,
                        arguments,
                        &argument_names,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                    stack.push(result);
                    return Ok(true);
                }
            };
            if function.is_abstract {
                return Err(format!(
                    "Cannot call abstract method {class_name}::{method}()"
                ));
            }
            if function.is_static {
                return Err(format!(
                    "Non-static method {class_name}::{method}() cannot be called statically"
                ));
            }
            let (mut locals, parameter_bindings) = prepare_reference_arguments(
                arguments,
                &reference_names,
                function,
                &argument_names,
                &format!("{class_name}::{method}()"),
                variables,
                bindings,
                global_variables,
                global_scope,
                frame_id,
                classes,
                *strict_types,
            )?;
            let concrete_class = object.borrow().class_name.clone();
            locals.insert("this".to_string(), Value::Object(object));
            locals.insert(
                "\0static_class".to_string(),
                Value::String(concrete_class.clone()),
            );
            super::initialize_local_order(
                &mut locals,
                std::iter::once("this".to_string()).chain(function.parameters.iter().cloned()),
            );
            if super::is_generator_function(function) {
                stack.push(super::new_generator(
                    function.clone(),
                    locals,
                    parameter_bindings,
                    *steps,
                    classes,
                )?);
                return Ok(true);
            }
            let result = execute_user_frame(
                &function.instructions,
                functions,
                classes,
                static_properties,
                &mut locals,
                global_variables,
                false,
                &function_scope(function),
                parameter_bindings,
                output,
                steps,
            )?
            .unwrap_or(Value::Null);
            let result = clean_frame_after_error(
                super::validate_return_type_in_scope(
                    result,
                    function,
                    classes,
                    Some(&concrete_class),
                ),
                &mut locals,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            finish_user_frame(
                locals,
                &result,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            stack.push(result);
        }
        Op::CallParentMethod(
            parent,
            method,
            _argument_count,
            reference_names,
            argument_names,
            unpack,
            access_scope,
            strict_types,
        ) => {
            let (arguments, reference_names, argument_names) =
                take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let object = variables
                .get("this")
                .cloned()
                .ok_or("parent call outside an instance method")?;
            let Value::Object(object) = object else {
                return Err("Invalid $this object".into());
            };
            let concrete_class = object.borrow().class_name.clone();
            if method.eq_ignore_ascii_case("__construct")
                && find_method(parent, method, classes).is_none()
                && super::is_a_type(
                    parent,
                    "throwable",
                    classes,
                    &mut std::collections::HashSet::new(),
                )
            {
                crate::exception_builtins::initialize_object(
                    &Value::Object(object),
                    parent,
                    &arguments,
                    classes,
                )?;
                stack.push(Value::Null);
                return Ok(true);
            }
            let function = find_method(parent, method, classes)
                .ok_or_else(|| format!("Call to undefined parent method {parent}::{method}()"))?;
            if function.is_abstract {
                return Err(format!("Cannot call abstract method {parent}::{method}()"));
            }
            ensure_access(
                function.visibility,
                function.declaring_class.as_deref(),
                access_scope.as_deref(),
                classes,
                method,
            )?;
            let (mut locals, parameter_bindings) = prepare_reference_arguments(
                arguments,
                &reference_names,
                function,
                &argument_names,
                &format!("{parent}::{method}()"),
                variables,
                bindings,
                global_variables,
                global_scope,
                frame_id,
                classes,
                *strict_types,
            )?;
            locals.insert("this".to_string(), Value::Object(object));
            locals.insert(
                "\0static_class".to_string(),
                Value::String(concrete_class.clone()),
            );
            super::initialize_local_order(
                &mut locals,
                std::iter::once("this".to_string()).chain(function.parameters.iter().cloned()),
            );
            if super::is_generator_function(function) {
                stack.push(super::new_generator(
                    function.clone(),
                    locals,
                    parameter_bindings,
                    *steps,
                    classes,
                )?);
                return Ok(true);
            }
            let result = execute_user_frame(
                &function.instructions,
                functions,
                classes,
                static_properties,
                &mut locals,
                global_variables,
                false,
                &function_scope(function),
                parameter_bindings,
                output,
                steps,
            )?
            .unwrap_or(Value::Null);
            let result = clean_frame_after_error(
                super::validate_return_type_in_scope(
                    result,
                    function,
                    classes,
                    Some(&concrete_class),
                ),
                &mut locals,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            finish_user_frame(
                locals,
                &result,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            stack.push(result);
        }
        Op::CallStaticMethod(
            class_name,
            method,
            _argument_count,
            reference_names,
            argument_names,
            unpack,
            access_scope,
            strict_types,
        ) => {
            let (mut arguments, reference_names, argument_names) =
                take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let method = if method == "\0dynamic" {
                stack
                    .pop()
                    .ok_or("Missing dynamic static method name")?
                    .show()
                    .to_ascii_lowercase()
            } else {
                method.clone()
            };
            let class_name = if class_name == "\0dynamic" {
                match stack
                    .pop()
                    .ok_or_else(|| "Missing dynamic class name".to_string())?
                {
                    Value::String(class) => class.to_ascii_lowercase(),
                    value => value.show().to_ascii_lowercase(),
                }
            } else {
                class_name.clone()
            };
            let resolved_class = if class_name == "static" {
                match variables.get("\0static_class") {
                    Some(Value::String(class)) => class.clone(),
                    _ => return Err("static:: used outside a class method".into()),
                }
            } else {
                class_name.clone()
            };
            let class_name = resolved_class.as_str();
            let class_key = class_name.trim_start_matches('\\').to_ascii_lowercase();
            if !classes.contains_key(&class_key)
                && !is_builtin_class_for_autoload(&class_key)
                && !global_variables.autoload_callbacks.is_empty()
            {
                invoke_registered_autoloaders(
                    class_name,
                    variables,
                    bindings,
                    global_variables,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )?;
            }
            let mut merged_functions = None;
            let mut merged_classes = None;
            if global_variables.dynamic_symbol_generation != 0 {
                let mut all_functions = functions.clone();
                all_functions.extend(global_variables.dynamic_functions.clone());
                merged_functions = Some(all_functions);
                let mut all_classes = classes.clone();
                all_classes.extend(global_variables.dynamic_classes.clone());
                merged_classes = Some(all_classes);
            }
            let functions = merged_functions.as_ref().unwrap_or(functions);
            let classes = merged_classes.as_ref().unwrap_or(classes);
            if class_name.eq_ignore_ascii_case("ReflectionMethod")
                && method.eq_ignore_ascii_case("createFromMethodName")
            {
                let (bound, _) = super::builtin_arguments::bind(
                    "reflectionmethod::createfrommethodname",
                    arguments,
                    &reference_names,
                    &argument_names,
                )?;
                arguments = bound;
                let requested = arguments
                    .first()
                    .ok_or("ReflectionMethod::createFromMethodName() expects exactly 1 argument")?;
                let Value::String(requested) = requested.dereference() else {
                    return Err(format!("TypeError:ReflectionMethod::createFromMethodName(): Argument #1 ($method) must be of type string, {} given", requested.type_name()));
                };
                let Some((target, method_name)) = requested.split_once("::") else {
                    return Err("ReflectionException:Method name must be a class name followed by :: and a method name".into());
                };
                let target = target.trim_start_matches('\\');
                if target.is_empty() || method_name.is_empty() {
                    return Err("ReflectionException:Method name must be a class name followed by :: and a method name".into());
                }
                let class = classes
                    .get(&target.to_ascii_lowercase())
                    .ok_or_else(|| format!("ReflectionException:Class {target} does not exist"))?;
                let function = find_method(target, &method_name.to_ascii_lowercase(), classes)
                    .ok_or_else(|| {
                        format!(
                            "ReflectionException:Method {target}::{method_name}() does not exist"
                        )
                    })?;
                let declaring_class = function
                    .declaring_class
                    .as_deref()
                    .and_then(|name| classes.get(&name.to_ascii_lowercase()))
                    .map_or_else(|| class.name.clone(), |declaring| declaring.name.clone());
                stack.push(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(
                    crate::PhpObject {
                        class_name: "ReflectionMethod".into(),
                        properties: std::collections::HashMap::from([
                            ("\0target_class".into(), Value::String(declaring_class)),
                            (
                                "\0method_name".into(),
                                Value::String(function.display_name.clone()),
                            ),
                        ]),
                        property_order: Vec::new(),
                        active_property_hooks: Default::default(),
                        exception: None,
                    },
                ))));
                return Ok(true);
            }
            if let Some(class) = classes
                .get(&class_name.to_ascii_lowercase())
                .filter(|class| class.is_enum)
            {
                if method.eq_ignore_ascii_case("cases")
                    || method.eq_ignore_ascii_case("from")
                    || method.eq_ignore_ascii_case("tryfrom")
                {
                    arguments = super::builtin_arguments::bind(
                        &format!("enum::{method}"),
                        arguments,
                        &reference_names,
                        &argument_names,
                    )?
                    .0;
                }
                if method.eq_ignore_ascii_case("cases") {
                    if !arguments.is_empty() {
                        return Err(format!(
                            "{}::cases() expects exactly 0 arguments",
                            class.name
                        ));
                    }
                    let cases = class
                        .enum_cases
                        .iter()
                        .enumerate()
                        .map(|(index, case)| {
                            (
                                ArrayKey::Integer(index as i64),
                                class
                                    .constants
                                    .get(case)
                                    .map(|constant| constant.value.clone())
                                    .unwrap_or(Value::Null),
                            )
                        })
                        .collect();
                    stack.push(Value::array(cases));
                    return Ok(true);
                }
                if method.eq_ignore_ascii_case("from") || method.eq_ignore_ascii_case("tryfrom") {
                    let Some(backing_type) = class.enum_backing_type.as_deref() else {
                        return Err(format!(
                            "Call to undefined method {}::{method}()",
                            class.name
                        ));
                    };
                    if arguments.len() != 1 {
                        return Err(format!(
                            "{}::{method}() expects exactly 1 argument",
                            class.name
                        ));
                    }
                    let input = arguments[0].dereference();
                    let scalar = match (backing_type, input) {
                        ("int", Value::Integer(value)) => Value::Integer(value),
                        ("int", Value::Number(value)) if value.is_finite() => {
                            Value::Integer(value as i64)
                        }
                        ("int", Value::String(value)) => {
                            Value::Integer(value.trim().parse::<i64>().map_err(|_| {
                                format!(
                                    "{}::{method}(): Argument #1 must be a valid integer",
                                    class.name
                                )
                            })?)
                        }
                        ("string", Value::String(value)) => Value::String(value),
                        ("string", Value::Integer(value)) => Value::String(value.to_string()),
                        ("string", Value::Number(value)) if value.is_finite() => {
                            Value::String(value.to_string())
                        }
                        (_, value) => {
                            let kind = match value {
                                Value::Null => "null",
                                Value::Bool(_) => "bool",
                                Value::Array(_) => "array",
                                Value::Object(_) => "object",
                                Value::Closure(_) => "Closure",
                                Value::Generator(_) => "Generator",
                                Value::Stream(_) => "resource",
                                Value::Integer(_) => "int",
                                Value::Number(_) => "float",
                                Value::String(_) => "string",
                                Value::Reference(_) => "reference",
                            };
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
                        return Err(format!(
                            "ValueError: {:?} is not a valid backing value for enum {}",
                            scalar.show(),
                            class.name
                        ));
                    }
                    return Ok(true);
                }
            }
            if find_method(class_name, &method, classes).is_none() {
                if class_name.eq_ignore_ascii_case("PDO")
                    && matches!(
                        method.to_ascii_lowercase().as_str(),
                        "connect" | "getavailabledrivers"
                    )
                {
                    arguments = super::builtin_arguments::bind(
                        &format!("pdo::{method}"),
                        arguments,
                        &reference_names,
                        &argument_names,
                    )?
                    .0;
                }
                if let Some(result) =
                    crate::pdo::call_static_method(class_name, &method, &arguments)
                {
                    stack.push(result?);
                    return Ok(true);
                }
            }
            let function = match find_method(class_name, &method, classes) {
                Some(function)
                    if ensure_access(
                        function.visibility,
                        function.declaring_class.as_deref(),
                        access_scope.as_deref(),
                        classes,
                        &method,
                    )
                    .is_ok() =>
                {
                    function
                }
                _ => {
                    let magic =
                        find_method(class_name, "__callstatic", classes).ok_or_else(|| {
                            format!("Call to undefined method {class_name}::{method}()")
                        })?;
                    let result = invoke_magic_method(
                        &Value::Null,
                        magic,
                        &method,
                        arguments,
                        &argument_names,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                    stack.push(result);
                    return Ok(true);
                }
            };
            if function.is_abstract {
                return Err(format!(
                    "Cannot call abstract method {class_name}::{method}()"
                ));
            }
            if !function.is_static {
                return Err(format!(
                    "Non-static method {class_name}::{method}() cannot be called statically"
                ));
            }
            let (mut locals, parameter_bindings) = prepare_reference_arguments(
                arguments,
                &reference_names,
                function,
                &argument_names,
                &format!("{class_name}::{method}()"),
                variables,
                bindings,
                global_variables,
                global_scope,
                frame_id,
                classes,
                *strict_types,
            )?;
            locals.insert(
                "\0static_class".to_string(),
                Value::String(class_name.to_string()),
            );
            if super::is_generator_function(function) {
                stack.push(super::new_generator(
                    function.clone(),
                    locals,
                    parameter_bindings,
                    *steps,
                    classes,
                )?);
                return Ok(true);
            }
            let result = execute_user_frame(
                &function.instructions,
                functions,
                classes,
                static_properties,
                &mut locals,
                global_variables,
                false,
                &function_scope(function),
                parameter_bindings,
                output,
                steps,
            )?
            .unwrap_or(Value::Null);
            let result = clean_frame_after_error(
                super::validate_return_type_in_scope(result, function, classes, Some(class_name)),
                &mut locals,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            finish_user_frame(
                locals,
                &result,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
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
        return Err(format!(
            "Failed to create closure from callable {}",
            display_name
        ));
    }
    let function = BytecodeFunction::new(
        "{first-class-callable}".into(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
        false,
        Vec::new(),
        false,
        false,
        false,
        MemberVisibility::Public,
        None,
    );
    Ok(Value::Closure(Rc::new(std::cell::RefCell::new(
        PhpClosure {
            function,
            captures: HashMap::new(),
            capture_order: Vec::new(),
            reference_captures: HashMap::new(),
            callable_target: Some(target),
            callable_access_scope: None,
        },
    ))))
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
    if let Some(user_data) = user_data {
        arguments.push(user_data);
    }
    let callback_references = callable_parameter_references(callback, functions, classes);
    let reference_names = arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let variable = if index == 0 {
                Some(value_variable.clone())
            } else if callback_references.get(index).copied().unwrap_or(false) {
                let name = format!("\0array_walk_arg:{}:{}", frame_id, index);
                variables.insert(name.clone(), argument.clone());
                Some(name)
            } else {
                None
            };
            variable.map(|variable| ReferenceArgument {
                variable,
                array_keys: Vec::new(),
                object_property: None,
                direct_reference: None,
            })
        })
        .collect::<Vec<_>>();
    let argument_names = vec![None; arguments.len()];
    match callback {
        Value::String(name) if functions.contains_key(&name.to_ascii_lowercase()) => {
            invoke_with_references(
                name,
                arguments,
                &reference_names,
                &argument_names,
                false,
                &mut variables,
                &mut bindings,
                global_variables,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
        }
        Value::String(name) if name.contains("::") => {
            let (class_name, method) = name.split_once("::").expect("contains separator");
            let target = Value::array(vec![
                (
                    ArrayKey::Integer(0),
                    Value::String(class_name.trim_start_matches('\\').to_string()),
                ),
                (ArrayKey::Integer(1), Value::String(method.to_string())),
            ]);
            invoke_array_callable(
                &target,
                arguments,
                &reference_names,
                &argument_names,
                &mut variables,
                &mut bindings,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )?;
        }
        Value::String(name) => {
            invoke_callable(
                name,
                arguments,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
        }
        Value::Closure(closure) => {
            invoke_closure(
                closure,
                arguments,
                &reference_names,
                &argument_names,
                &mut variables,
                &mut bindings,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )?;
        }
        Value::Array(_) => {
            invoke_array_callable(
                callback,
                arguments,
                &reference_names,
                &argument_names,
                &mut variables,
                &mut bindings,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )?;
        }
        Value::Object(_) => {
            invoke_object_callable(
                callback,
                arguments,
                &reference_names,
                &argument_names,
                &mut variables,
                &mut bindings,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )?;
        }
        _ => {
            return Err(format!(
                "{builtin_name}(): callback must be a valid callback"
            ))
        }
    }
    Ok(read_variable(
        &value_variable,
        &variables,
        &bindings,
        global_variables,
        false,
    ))
}

fn visible_argument_count(
    function: &BytecodeFunction,
    argument_count: usize,
    argument_names: &[Option<String>],
) -> usize {
    if !argument_names.iter().any(Option::is_some) {
        return argument_count;
    }
    let fixed_count = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic)
        .unwrap_or(function.parameters.len());
    let mut positional_count = 0usize;
    let mut visible_count = 0usize;
    for name in argument_names {
        if let Some(name) = name {
            if let Some(index) = function.parameters[..fixed_count]
                .iter()
                .position(|parameter| parameter == name)
            {
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
    variables: &HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &GlobalVariables,
    global_scope: bool,
) -> Result<Vec<Value>, String> {
    let count = variables
        .get("\0func_argument_count")
        .map(Value::number)
        .ok_or("func_*() called from the global scope")? as usize;
    let fixed_count = variables
        .get("\0func_fixed_parameter_count")
        .map(Value::number)
        .unwrap_or(0.0) as usize;
    let parameter_names = variables
        .get("\0func_parameter_names")
        .ok_or("func_*() called from the global scope")?;
    let variadic_name = variables
        .get("\0func_variadic_parameter")
        .map(Value::show)
        .filter(|name| !name.is_empty());
    let mut arguments = Vec::with_capacity(count);
    for index in 0..count {
        if index < fixed_count {
            let parameter_name = parameter_names
                .array_get(&ArrayKey::Integer(index as i64))?
                .show();
            arguments.push(read_variable(
                &parameter_name,
                variables,
                bindings,
                globals,
                global_scope,
            ));
        } else if let Some(variadic_name) = &variadic_name {
            let variadic = read_variable(variadic_name, variables, bindings, globals, global_scope);
            arguments.push(variadic.array_get(&ArrayKey::Integer((index - fixed_count) as i64))?);
        } else {
            let extras = variables
                .get("\0func_extra_arguments")
                .cloned()
                .unwrap_or(Value::Null);
            arguments.push(extras.array_get(&ArrayKey::Integer((index - fixed_count) as i64))?);
        }
    }
    Ok(arguments)
}

fn normalize_named_arguments(
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    argument_names: &[Option<String>],
    function: &BytecodeFunction,
    call_name: &str,
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<(Vec<Value>, Vec<Option<ReferenceArgument>>, bool), String> {
    let variadic_index = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic);
    let reference_variadic = variadic_index.is_some_and(|index| {
        function
            .parameter_by_reference
            .get(index)
            .copied()
            .unwrap_or(false)
    });
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
            if let Some(parameter_index) = function.parameters[..fixed_count]
                .iter()
                .position(|parameter| parameter == &name)
            {
                if slots[parameter_index].is_some() {
                    return Err(format!(
                        "Named parameter ${name} overwrites previous argument"
                    ));
                }
                slots[parameter_index] = Some((value, reference));
            } else if variadic_index.is_some() {
                if variadic_entries
                    .iter()
                    .any(|(key, _, _)| key == &ArrayKey::String(name.clone()))
                {
                    return Err(format!(
                        "Named parameter ${name} overwrites previous argument"
                    ));
                }
                variadic_entries.push((ArrayKey::String(name), value, reference));
            } else {
                return Err(format!("{call_name}: unknown named parameter ${name}"));
            }
        } else {
            if named_seen {
                return Err(format!(
                    "{call_name}: positional argument follows a named argument"
                ));
            }
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
            return Err(format!(
                "{call_name}: required parameter ${} was not passed",
                function.parameters[index]
            ));
        }
    }
    if variadic_index.is_some() {
        let mut packed = crate::PhpArray::from_entries(Vec::new());
        for (extra_index, (key, value, reference)) in variadic_entries.into_iter().enumerate() {
            if reference_variadic {
                match reference {
                    Some(reference) if is_traversable_unpack_reference(&reference) => {
                        warn_traversable_by_reference(
                            global_variables,
                            fixed_count + extra_index + 1,
                            call_name,
                        );
                        packed.set(key, value);
                    }
                    Some(reference) => {
                        let cell = reference_cell_for_argument(
                            &reference,
                            caller_variables,
                            caller_bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                        )?;
                        packed.set_reference(key, cell);
                    }
                    None => {
                        return Err(format!(
                            "Argument to {call_name} must be a variable passed by reference"
                        ))
                    }
                }
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
    target: &ReferenceArgument,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<Rc<std::cell::RefCell<Value>>, String> {
    if let Some(reference) = &target.direct_reference {
        return Ok(reference.clone());
    }
    if let Some((receiver, property)) = &target.object_property {
        let Value::Object(object) = receiver else {
            return Err("Only object properties can be passed by reference".into());
        };
        let mut object = object.borrow_mut();
        let current = object
            .properties
            .get(property)
            .cloned()
            .unwrap_or(Value::Null);
        if target.array_keys.is_empty() {
            if let Value::Reference(reference) = &current {
                return Ok(reference.clone());
            }
            let reference = Rc::new(std::cell::RefCell::new(current.dereference()));
            if !object.properties.contains_key(property) {
                object.property_order.push(property.clone());
            }
            object
                .properties
                .insert(property.clone(), Value::Reference(reference.clone()));
            return Ok(reference);
        }
        let (updated, reference) = current.ensure_array_reference(&target.array_keys)?;
        if !object.properties.contains_key(property) {
            object.property_order.push(property.clone());
        }
        object.properties.insert(property.clone(), updated);
        return Ok(reference);
    }
    let (_, reference) = super::reference_cell_for_path(
        &target.variable,
        &target.array_keys,
        variables,
        bindings,
        globals,
        global_scope,
        frame_id,
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
        let Value::Object(object) = receiver else {
            return Err("Only objects can have properties".into());
        };
        let mut object = object.borrow_mut();
        let current = object
            .properties
            .get(property)
            .cloned()
            .unwrap_or(Value::Null);
        let updated = if target.array_keys.is_empty() {
            value
        } else {
            current.array_set_path(&target.array_keys, value)?
        };
        if !object.properties.contains_key(property) {
            object.property_order.push(property.clone());
        }
        object.properties.insert(property.clone(), updated);
        return Ok(());
    }
    if target.array_keys.is_empty() {
        return write_variable(
            target.variable.clone(),
            value,
            variables,
            bindings,
            globals,
            global_scope,
        );
    }
    let current = read_variable(&target.variable, variables, bindings, globals, global_scope);
    let updated = current.array_set_path(&target.array_keys, value)?;
    write_array_root(
        target.variable.clone(),
        updated,
        variables,
        bindings,
        globals,
        global_scope,
    )
}

pub(super) fn pdo_iterator_rows(
    value: &Value,
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
) -> Result<Vec<(ArrayKey, Value)>, String> {
    let mut instantiate_row = |row: &Value, class_name: &str, constructor_args: &[Value]| {
        if class_name == "\0PDO_FETCH_FUNC" {
            let packed = Value::from_array_entries(
                constructor_args
                    .iter()
                    .enumerate()
                    .map(|(index, value)| (ArrayKey::Integer(index as i64), value.clone()))
                    .collect(),
            );
            let (instruction, mut callback_stack) = callback_call(row.clone(), packed, false)?;
            execute_instruction(
                &instruction,
                &mut callback_stack,
                variables,
                bindings,
                globals,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            return callback_stack
                .pop()
                .ok_or_else(|| "Invalid PDO::FETCH_FUNC callback result stack".into());
        }
        let (class_name, props_late) = class_name
            .strip_prefix("\0PDO_FETCH_PROPS_LATE:")
            .map_or((class_name, false), |name| (name, true));
        instantiate_pdo_row(
            row,
            class_name,
            constructor_args,
            props_late,
            variables,
            bindings,
            globals,
            global_scope,
            frame_id,
            functions,
            classes,
            static_properties,
            output,
            steps,
        )
    };
    crate::pdo::statement_iterator_rows(value, &mut instantiate_row)
}

fn iterator_count(
    arguments: &[Value],
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
) -> Result<Value, String> {
    if arguments.len() != 1 {
        return Err(format!(
            "iterator_count() expects exactly 1 argument, {} given",
            arguments.len()
        ));
    }
    let count = collect_traversable(
        arguments[0].clone(),
        variables,
        bindings,
        globals,
        global_scope,
        frame_id,
        functions,
        classes,
        static_properties,
        output,
        steps,
        0,
        true,
        false,
    )
    .map_err(|error| {
        if error == "Value is not Traversable|array" {
            "iterator_count(): Argument #1 ($iterator) must be of type Traversable|array".into()
        } else {
            error
        }
    })?
    .len();
    i64::try_from(count)
        .map(Value::Integer)
        .map_err(|_| "iterator_count(): iterator is too large".into())
}

fn iterator_to_array(
    arguments: &[Value],
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
) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!(
            "iterator_to_array() expects 1 to 2 arguments, {} given",
            arguments.len()
        ));
    }
    let preserve_keys = arguments.get(1).is_none_or(Value::truthy);
    let mut result = Vec::<(ArrayKey, Value)>::new();
    let mut insert_entry = |key: ArrayKey, value: Value| {
        let key = if preserve_keys {
            key
        } else {
            ArrayKey::Integer(result.len() as i64)
        };
        if let Some((_, existing)) = result
            .iter_mut()
            .find(|(existing_key, _)| *existing_key == key)
        {
            *existing = value;
        } else {
            result.push((key, value));
        }
    };
    for (key, value) in collect_traversable(
        arguments[0].clone(),
        variables,
        bindings,
        globals,
        global_scope,
        frame_id,
        functions,
        classes,
        static_properties,
        output,
        steps,
        0,
        !preserve_keys,
        preserve_keys,
    )
    .map_err(|error| {
        if error == "Value is not Traversable|array" {
            "iterator_to_array(): Argument #1 ($iterator) must be of type Traversable|array".into()
        } else {
            error
        }
    })? {
        insert_entry(key, value);
    }
    Ok(Value::from_array_entries(result))
}

fn json_error_code(message: &str) -> i64 {
    if message.contains("Maximum stack depth") {
        1
    } else if message.contains("State mismatch") {
        2
    } else if message.contains("Unescaped control character") {
        3
    } else if message.contains("Unicode surrogate") {
        10
    } else if message.contains("Malformed UTF-8") {
        5
    } else if message.contains("Recursion") {
        6
    } else if message.contains("Inf and NaN") {
        7
    } else if message.contains("Non-backed enums") {
        11
    } else if message.contains("Invalid property name")
        || message.contains("decoded property name is invalid")
    {
        9
    } else if message.contains("Resources") {
        8
    } else {
        4
    }
}

fn json_exception_marker(
    global_variables: &mut GlobalVariables,
    step: u64,
    code: i64,
    message: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<String, String> {
    let exception = crate::exception_builtins::construct(
        "JsonException",
        &[Value::String(message.to_string()), Value::Integer(code)],
        classes,
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
enum FilesystemAccess {
    Read,
    Write,
    Execute,
}

fn file_timestamp_value(
    name: &str,
    filename: &str,
    path: Option<&std::path::Path>,
    globals: &mut GlobalVariables,
) -> Value {
    let timestamp = path
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|metadata| {
            if name.eq_ignore_ascii_case("filemtime") {
                metadata.modified().ok()
            } else {
                metadata.accessed().ok()
            }
        })
        .and_then(|time| match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => i64::try_from(duration.as_secs()).ok(),
            Err(error) => i64::try_from(error.duration().as_secs())
                .ok()
                .and_then(i64::checked_neg),
        });
    if let Some(timestamp) = timestamp {
        return Value::Integer(timestamp);
    }
    globals.report_diagnostic(crate::RuntimeDiagnostic {
        level: crate::DiagnosticLevel::Warning,
        message: format!("{}(): stat failed for {filename}", name),
        source_file: None,
        source_line: None,
        output_offset: 0,
    });
    Value::Bool(false)
}

fn md5_file_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!(
            "ArgumentCountError:md5_file() expects 1 or 2 arguments, {} given",
            arguments.len()
        ));
    }
    let filename = arguments[0].show();
    let binary = arguments.get(1).is_some_and(Value::truthy);
    let contents = filesystem_path(globals, std::path::Path::new(&filename), false)
        .and_then(|path| std::fs::read(path).ok());
    if let Some(contents) = contents {
        let digest = crate::string_builtins::md5_digest(&contents);
        let result = if binary {
            String::from_utf8_lossy(&digest).into_owned()
        } else {
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        };
        Ok(Value::String(result))
    } else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("md5_file(): Failed to open stream: {filename}"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        Ok(Value::Bool(false))
    }
}

fn sha1_file_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!(
            "ArgumentCountError:sha1_file() expects 1 or 2 arguments, {} given",
            arguments.len()
        ));
    }
    let filename = arguments[0].show();
    let binary = arguments.get(1).is_some_and(Value::truthy);
    let contents = filesystem_path(globals, std::path::Path::new(&filename), false)
        .and_then(|path| std::fs::read(path).ok());
    if let Some(contents) = contents {
        let digest = crate::string_builtins::sha1_digest(&contents);
        let result = if binary {
            String::from_utf8_lossy(&digest).into_owned()
        } else {
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        };
        Ok(Value::String(result))
    } else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("sha1_file(): Failed to open stream: {filename}"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        Ok(Value::Bool(false))
    }
}

fn hash_hmac_file_builtin(
    arguments: &[Value],
    globals: &mut GlobalVariables,
) -> Result<Value, String> {
    if arguments.len() < 3 || arguments.len() > 4 {
        return Err(format!(
            "ArgumentCountError:hash_hmac_file() expects 3 or 4 arguments, {} given",
            arguments.len()
        ));
    }
    let algorithm = arguments[0].show().to_ascii_lowercase();
    let digest = match algorithm.as_str() {
        "md5" => crate::string_builtins::md5_digest as fn(&[u8]) -> Vec<u8>,
        "sha1" => crate::string_builtins::sha1_digest as fn(&[u8]) -> Vec<u8>,
        "sha224" => crate::string_builtins::sha224_digest as fn(&[u8]) -> Vec<u8>,
        "sha256" => crate::string_builtins::sha256_digest as fn(&[u8]) -> Vec<u8>,
        "sha384" => crate::string_builtins::sha384_digest as fn(&[u8]) -> Vec<u8>,
        "sha512" => crate::string_builtins::sha512_digest as fn(&[u8]) -> Vec<u8>,
        "sha3-224" => crate::string_builtins::sha3_224_digest as fn(&[u8]) -> Vec<u8>,
        "sha3-256" => crate::string_builtins::sha3_256_digest as fn(&[u8]) -> Vec<u8>,
        "sha3-384" => crate::string_builtins::sha3_384_digest as fn(&[u8]) -> Vec<u8>,
        "sha3-512" => crate::string_builtins::sha3_512_digest as fn(&[u8]) -> Vec<u8>,
        _ => return Err(format!("ValueError:hash_hmac_file(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm, {algorithm} given")),
    };
    let filename = arguments[1].show();
    let key = arguments[2].show();
    let binary = arguments.get(3).is_some_and(Value::truthy);
    let contents = filesystem_path(globals, std::path::Path::new(&filename), false)
        .and_then(|path| std::fs::read(path).ok());
    if let Some(contents) = contents {
        let result = crate::string_builtins::hmac_digest_bytes_with_block(
            &key.as_bytes(),
            &contents,
            digest,
            crate::string_builtins::hmac_block_size(&algorithm),
        );
        let output = if binary {
            String::from_utf8_lossy(&result).into_owned()
        } else {
            result.iter().map(|byte| format!("{byte:02x}")).collect()
        };
        Ok(Value::String(output))
    } else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("hash_hmac_file(): Failed to open stream: {filename}"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        Ok(Value::Bool(false))
    }
}

fn hash_file_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.len() < 2 || arguments.len() > 3 {
        return Err(format!(
            "ArgumentCountError:hash_file() expects 2 or 3 arguments, {} given",
            arguments.len()
        ));
    }
    let algorithm = arguments[0].show().to_ascii_lowercase();
    let filename = arguments[1].show();
    let binary = arguments.get(2).is_some_and(Value::truthy);
    if !matches!(
        algorithm.as_str(),
        "md5"
            | "sha1"
            | "sha224"
            | "sha256"
            | "sha384"
            | "sha512"
            | "sha3-224"
            | "sha3-256"
            | "sha3-384"
            | "sha3-512"
            | "crc32b"
    ) {
        return Err(format!("ValueError:hash_file(): Argument #1 ($algo) must be a valid hashing algorithm, {algorithm} given"));
    }
    let Some(contents) = filesystem_path(globals, std::path::Path::new(&filename), false)
        .and_then(|path| std::fs::read(path).ok())
    else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("hash_file(): Failed to open stream: {filename}"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        return Ok(Value::Bool(false));
    };
    let digest = crate::string_builtins::hash_digest(&algorithm, &contents)
        .expect("algorithm checked above");
    let output = if binary {
        String::from_utf8_lossy(&digest).into_owned()
    } else {
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    };
    Ok(Value::String(output))
}

fn file_stat_value(name: &str, filename: &str, globals: &mut GlobalVariables) -> Value {
    let input_path = std::path::Path::new(filename);
    let resolved = if name.eq_ignore_ascii_case("lstat") {
        filesystem_lstat_path(globals, input_path)
    } else {
        filesystem_path(globals, input_path, false)
    };
    let Some(resolved) = resolved else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("{}(): stat failed for {filename}", name),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        return Value::Bool(false);
    };
    let metadata = if name.eq_ignore_ascii_case("lstat") {
        std::fs::symlink_metadata(&resolved)
    } else {
        std::fs::metadata(&resolved)
    };
    let Ok(metadata) = metadata else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("{}(): stat failed for {filename}", name),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        return Value::Bool(false);
    };
    #[cfg(unix)]
    let (dev, ino, mode, nlink, uid, gid, rdev, size, atime, mtime, ctime, blksize, blocks) = {
        use std::os::unix::fs::MetadataExt;
        (
            metadata.dev() as i64,
            metadata.ino() as i64,
            metadata.mode() as i64,
            metadata.nlink() as i64,
            metadata.uid() as i64,
            metadata.gid() as i64,
            metadata.rdev() as i64,
            metadata.size() as i64,
            metadata.atime(),
            metadata.mtime(),
            metadata.ctime(),
            metadata.blksize() as i64,
            metadata.blocks() as i64,
        )
    };
    #[cfg(not(unix))]
    let (dev, ino, mode, nlink, uid, gid, rdev, size, atime, mtime, ctime, blksize, blocks) = {
        let timestamp = |time: std::io::Result<std::time::SystemTime>| -> i64 {
            time.ok()
                .and_then(|time| match time.duration_since(std::time::UNIX_EPOCH) {
                    Ok(duration) => i64::try_from(duration.as_secs()).ok(),
                    Err(error) => i64::try_from(error.duration().as_secs())
                        .ok()
                        .and_then(i64::checked_neg),
                })
                .unwrap_or(0)
        };
        let file_type = metadata.file_type();
        let type_bits = if file_type.is_dir() {
            0o040000
        } else if file_type.is_symlink() {
            0o120000
        } else {
            0o100000
        };
        let readonly = metadata.permissions().readonly();
        (
            0,
            0,
            type_bits | if readonly { 0o444 } else { 0o666 },
            1,
            0,
            0,
            0,
            metadata.len() as i64,
            timestamp(metadata.accessed()),
            timestamp(metadata.modified()),
            timestamp(metadata.created()),
            0,
            0,
        )
    };
    let values = [
        dev, ino, mode, nlink, uid, gid, rdev, size, atime, mtime, ctime, blksize, blocks,
    ];
    let names = [
        "dev", "ino", "mode", "nlink", "uid", "gid", "rdev", "size", "atime", "mtime", "ctime",
        "blksize", "blocks",
    ];
    let mut entries = Vec::with_capacity(26);
    for (index, value) in values.iter().copied().enumerate() {
        entries.push((ArrayKey::Integer(index as i64), Value::Integer(value)));
    }
    for (field, value) in names.into_iter().zip(values) {
        entries.push((ArrayKey::String(field.into()), Value::Integer(value)));
    }
    Value::from_array_entries(entries)
}

fn file_stat_field_value(
    name: &str,
    filename: &str,
    field: &str,
    globals: &mut GlobalVariables,
) -> Value {
    file_stat_value(name, filename, globals)
        .array_get(&ArrayKey::String(field.into()))
        .unwrap_or(Value::Bool(false))
}

fn filetype_value(name: &str, filename: &str, globals: &mut GlobalVariables) -> Value {
    let mode = file_stat_field_value(name, filename, "mode", globals);
    let mode = match mode {
        Value::Integer(mode) => mode,
        _ => return Value::Bool(false),
    };
    let kind = match mode & 0o170000 {
        0o0100000 => "file",
        0o0040000 => "dir",
        0o0120000 => "link",
        0o0060000 => "block",
        0o0020000 => "char",
        0o0010000 => "fifo",
        0o0140000 => "socket",
        _ => "unknown",
    };
    Value::String(kind.into())
}

fn scandir_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.is_empty() {
        return Err("ArgumentCountError:scandir() expects at least 1 argument, 0 given".into());
    }
    if arguments.len() > 3 {
        return Err(format!(
            "ArgumentCountError:scandir() expects at most 3 arguments, {} given",
            arguments.len()
        ));
    }
    if let Some(context) = arguments
        .get(2)
        .filter(|value| !matches!(value.dereference(), Value::Null | Value::Stream(_)))
    {
        return Err(format!(
            "TypeError:scandir(): Argument #3 ($context) must be of type ?resource, {} given",
            context.type_name()
        ));
    }
    let directory = arguments[0].show();
    let sorting_order = arguments.get(1).map_or(0, |value| value.number() as i64);
    let resolved = filesystem_path(globals, std::path::Path::new(&directory), false);
    let entries = resolved.and_then(|path| std::fs::read_dir(path).ok());
    let Some(entries) = entries else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("scandir(): failed to open directory {directory}"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        return Ok(Value::Bool(false));
    };
    let mut names = vec![".".to_string(), "..".to_string()];
    for entry in entries {
        match entry {
            Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Err(_) => {
                globals.report_diagnostic(crate::RuntimeDiagnostic {
                    level: crate::DiagnosticLevel::Warning,
                    message: format!("scandir(): failed to read directory {directory}"),
                    source_file: None,
                    source_line: None,
                    output_offset: 0,
                });
                return Ok(Value::Bool(false));
            }
        }
    }
    match sorting_order {
        0 => names.sort(),
        1 => names.sort_by(|left, right| right.cmp(left)),
        _ => {}
    }
    Ok(Value::from_array_entries(
        names
            .into_iter()
            .enumerate()
            .map(|(index, name)| (ArrayKey::Integer(index as i64), Value::String(name)))
            .collect(),
    ))
}

fn opendir_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.is_empty() {
        return Err("ArgumentCountError:opendir() expects at least 1 argument, 0 given".into());
    }
    if arguments.len() > 2 {
        return Err(format!(
            "ArgumentCountError:opendir() expects at most 2 arguments, {} given",
            arguments.len()
        ));
    }
    if let Some(context) = arguments
        .get(1)
        .filter(|value| !matches!(value.dereference(), Value::Null | Value::Stream(_)))
    {
        return Err(format!(
            "TypeError:opendir(): Argument #2 ($context) must be of type ?resource, {} given",
            context.type_name()
        ));
    }
    let directory = arguments[0].show();
    let Some(path) = filesystem_path(globals, std::path::Path::new(&directory), false) else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("opendir({directory}): failed to open directory"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        return Ok(Value::Bool(false));
    };
    let Ok(entries) = std::fs::read_dir(&path) else {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("opendir({directory}): failed to open directory"),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
        return Ok(Value::Bool(false));
    };
    let mut names = vec![".".to_string(), "..".to_string()];
    for entry in entries {
        match entry {
            Ok(entry) => names.push(entry.file_name().to_string_lossy().into_owned()),
            Err(_) => {
                globals.report_diagnostic(crate::RuntimeDiagnostic {
                    level: crate::DiagnosticLevel::Warning,
                    message: format!("opendir({directory}): failed to read directory"),
                    source_file: None,
                    source_line: None,
                    output_offset: 0,
                });
                return Ok(Value::Bool(false));
            }
        }
    }
    let handle = Value::Stream(std::rc::Rc::new(std::cell::RefCell::new(
        crate::PhpStream::directory(path, names),
    )));
    globals.last_directory_handle = Some(handle.clone());
    Ok(handle)
}

fn directory_handle(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    function: &str,
) -> Result<Option<std::rc::Rc<std::cell::RefCell<crate::PhpStream>>>, String> {
    if arguments
        .first()
        .is_some_and(|value| matches!(value.dereference(), Value::Null))
    {
        globals.report_diagnostic(crate::RuntimeDiagnostic { level: crate::DiagnosticLevel::Deprecated,
            message: format!("Passing null to {function}() is deprecated, instead the last opened directory stream should be provided"),
            source_file: None, source_line: None, output_offset: 0 });
    }
    let explicit = arguments
        .first()
        .filter(|value| !matches!(value.dereference(), Value::Null));
    let handle = explicit
        .cloned()
        .or_else(|| globals.last_directory_handle.clone());
    let Some(handle) = handle else {
        return Ok(None);
    };
    let Value::Stream(stream) = handle.dereference() else {
        return Err(format!(
            "TypeError:{function}(): Argument #1 ($dir_handle) must be of type ?resource, {} given",
            handle.type_name()
        ));
    };
    if !stream.borrow().is_open() || !stream.borrow().is_directory() {
        return Err(format!(
            "TypeError:{function}(): supplied resource is not a valid directory resource"
        ));
    }
    Ok(Some(stream))
}

fn readdir_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.len() > 1 {
        return Err(format!(
            "ArgumentCountError:readdir() expects at most 1 argument, {} given",
            arguments.len()
        ));
    }
    let Some(handle) = directory_handle(arguments, globals, "readdir")? else {
        return Ok(Value::Bool(false));
    };
    let next = handle.borrow_mut().read_directory();
    match next {
        Ok(Some(name)) => Ok(Value::String(name)),
        Ok(None) => Ok(Value::Bool(false)),
        Err(error) => Err(format!("readdir(): {error}")),
    }
}

fn closedir_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.len() > 1 {
        return Err(format!(
            "ArgumentCountError:closedir() expects at most 1 argument, {} given",
            arguments.len()
        ));
    }
    let Some(handle) = directory_handle(arguments, globals, "closedir")? else {
        return Err(
            "TypeError:closedir(): Argument #1 ($dir_handle) must be of type ?resource, null given"
                .into(),
        );
    };
    handle
        .borrow_mut()
        .close()
        .map_err(|error| format!("closedir(): {error}"))?;
    if let Some(Value::Stream(last)) = globals.last_directory_handle.as_ref() {
        if std::rc::Rc::ptr_eq(last, &handle) {
            globals.last_directory_handle = None;
        }
    }
    Ok(Value::Null)
}

fn rewinddir_builtin(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.len() > 1 {
        return Err(format!(
            "ArgumentCountError:rewinddir() expects at most 1 argument, {} given",
            arguments.len()
        ));
    }
    let Some(handle) = directory_handle(arguments, globals, "rewinddir")? else {
        return Err("TypeError:rewinddir(): Argument #1 ($dir_handle) must be of type ?resource, null given".into());
    };
    let path = handle
        .borrow()
        .directory_path()
        .map(std::path::Path::to_path_buf)
        .ok_or_else(|| {
            "TypeError:rewinddir(): supplied resource is not a valid directory resource".to_string()
        })?;
    let mut entries = vec![".".to_string(), "..".to_string()];
    let read_dir = std::fs::read_dir(path).map_err(|error| format!("rewinddir(): {error}"))?;
    for entry in read_dir {
        entries.push(
            entry
                .map_err(|error| format!("rewinddir(): {error}"))?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    handle
        .borrow_mut()
        .rewind_directory(entries)
        .map_err(|error| format!("rewinddir(): {error}"))?;
    Ok(Value::Null)
}

/// Resolve PHP filesystem paths against the server's application root when
/// the request installed a sandbox root. CLI executions do not set one.
fn filesystem_path(
    global_variables: &GlobalVariables,
    path: &std::path::Path,
    allow_create: bool,
) -> Option<PathBuf> {
    let Some(root) = global_variables.get("\0filesystem_root") else {
        return Some(path.to_path_buf());
    };
    let root = std::path::PathBuf::from(root.show()).canonicalize().ok()?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    if let Ok(canonical) = candidate.canonicalize() {
        return canonical.starts_with(&root).then_some(canonical);
    }
    if !allow_create || std::fs::symlink_metadata(&candidate).is_ok() {
        return None;
    }
    let name = candidate.file_name()?;
    let parent = candidate.parent()?.canonicalize().ok()?;
    parent.starts_with(&root).then(|| parent.join(name))
}

/// Resolve an lstat target without following the final path component, while
/// still rejecting paths whose parent resolves outside the configured root.
fn filesystem_lstat_path(
    global_variables: &GlobalVariables,
    path: &std::path::Path,
) -> Option<PathBuf> {
    let Some(root) = global_variables.get("\0filesystem_root") else {
        return Some(path.to_path_buf());
    };
    let root = PathBuf::from(root.show()).canonicalize().ok()?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let filename = candidate.file_name()?.to_os_string();
    let parent = candidate.parent()?.canonicalize().ok()?;
    if !parent.starts_with(&root) {
        return None;
    }
    Some(parent.join(filename))
}

fn filesystem_lookup_path(
    global_variables: &GlobalVariables,
    path: &std::path::Path,
    allow_create: bool,
    use_include_path: bool,
) -> Option<PathBuf> {
    if let Some(candidate) = filesystem_path(global_variables, path, allow_create) {
        if !use_include_path || candidate.exists() {
            return Some(candidate);
        }
    }
    if !use_include_path || path.is_absolute() {
        return filesystem_path(global_variables, path, allow_create);
    }
    let separator = if cfg!(windows) { ';' } else { ':' };
    let found = global_variables
        .include_path()
        .split(separator)
        .filter(|directory| !directory.is_empty())
        .find_map(|directory| {
            filesystem_path(
                global_variables,
                &PathBuf::from(directory).join(path),
                allow_create,
            )
            .filter(|candidate| candidate.exists() || allow_create)
        });
    found.or_else(|| {
        allow_create
            .then(|| filesystem_path(global_variables, path, true))
            .flatten()
    })
}

#[cfg(unix)]
fn filesystem_access(path: &std::path::Path, access: FilesystemAccess) -> bool {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mode = match access {
        FilesystemAccess::Read => libc::R_OK,
        FilesystemAccess::Write => libc::W_OK,
        FilesystemAccess::Execute => libc::X_OK,
    };
    unsafe { libc::access(path.as_ptr(), mode) == 0 }
}

#[cfg(not(unix))]
fn filesystem_access(path: &std::path::Path, access: FilesystemAccess) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
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
            for (_, item) in &array.entries {
                bytes.extend_from_slice(item.show().as_bytes());
            }
            Some(bytes)
        }
        Value::Object(_) | Value::Closure(_) | Value::Generator(_) => None,
        Value::Null => Some(Vec::new()),
        scalar => Some(scalar.show().into_bytes()),
    }
}

fn unix_time() -> (u64, u32) {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    (elapsed.as_secs(), elapsed.subsec_micros())
}

fn gettimeofday_value(as_float: bool) -> Result<Value, String> {
    let (seconds, microseconds) = unix_time();
    if as_float {
        return Ok(Value::Number(
            seconds as f64 + microseconds as f64 / 1_000_000.0,
        ));
    }
    let tm = date_tm(seconds as i64, false)?;
    let minutes_west = {
        #[cfg(target_os = "linux")]
        {
            -(tm.tm_gmtoff as i64) / 60
        }
        #[cfg(not(target_os = "linux"))]
        {
            0i64
        }
    };
    Ok(Value::from_array_entries(vec![
        (
            ArrayKey::String("sec".into()),
            Value::Integer(seconds as i64),
        ),
        (
            ArrayKey::String("usec".into()),
            Value::Integer(microseconds as i64),
        ),
        (
            ArrayKey::String("minuteswest".into()),
            Value::Integer(minutes_west),
        ),
        (
            ArrayKey::String("dsttime".into()),
            Value::Integer(i64::from(tm.tm_isdst > 0)),
        ),
    ]))
}

fn random_int_bound(function: &str, value: &Value, argument: usize) -> Result<i64, String> {
    let value = value.dereference();
    match value {
        Value::Integer(number) => Ok(number),
        Value::Number(number)
            if number.is_finite() && number >= i64::MIN as f64 && number < -(i64::MIN as f64) =>
        {
            Ok(number.trunc() as i64)
        }
        Value::String(text) => {
            if let Ok(number) = text.trim().parse::<i64>() {
                return Ok(number);
            }
            let number = text.trim().parse::<f64>().ok().filter(|number| {
                number.is_finite() && *number >= i64::MIN as f64 && *number < -(i64::MIN as f64)
            });
            number.map(|number| number.trunc() as i64).ok_or_else(|| {
                format!("TypeError:{function}(): Argument #{argument} must be a valid integer")
            })
        }
        Value::Bool(value) => Ok(i64::from(value)),
        Value::Null => Ok(0),
        value => Err(format!(
            "TypeError:{function}(): Argument #{argument} must be of type int, {} given",
            value.type_name()
        )),
    }
}

fn date_timestamp(value: &Value) -> Result<i64, String> {
    match value.dereference() {
        Value::Integer(value) => Ok(value),
        Value::Number(value)
            if value.is_finite() && value >= i64::MIN as f64 && value < -(i64::MIN as f64) =>
        {
            Ok(value.trunc() as i64)
        }
        Value::String(value) => value
            .trim()
            .parse::<i64>()
            .or_else(|_| {
                value
                    .trim()
                    .parse::<f64>()
                    .map(|number| number.trunc() as i64)
            })
            .map_err(|_| "TypeError:date(): Argument #2 ($timestamp) must be of type ?int".into()),
        Value::Bool(value) => Ok(i64::from(value)),
        Value::Null => Ok(unix_time().0 as i64),
        value => Err(format!(
            "TypeError:date(): Argument #2 ($timestamp) must be of type ?int, {} given",
            value.type_name()
        )),
    }
}

fn secure_random_integer(min: i64, max: i64) -> Result<i64, String> {
    let width = (i128::from(max) - i128::from(min) + 1) as u128;
    let sample_space = u64::MAX as u128 + 1;
    let accepted_limit = sample_space / width * width;
    loop {
        let mut bytes = [0u8; 8];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut source| std::io::Read::read_exact(&mut source, &mut bytes))
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
        Value::String(value) => value.trim().parse::<i64>().ok().or_else(|| {
            value
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map(|n| n.trunc() as i64)
        }),
        Value::Bool(value) => Some(i64::from(value)),
        Value::Null => Some(0),
        _ => None,
    }
}

fn date_tm(timestamp: i64, utc: bool) -> Result<libc::tm, String> {
    let timestamp = timestamp as libc::time_t;
    let mut broken_down = std::mem::MaybeUninit::<libc::tm>::zeroed();
    let result = unsafe {
        if utc {
            libc::gmtime_r(&timestamp, broken_down.as_mut_ptr())
        } else {
            libc::localtime_r(&timestamp, broken_down.as_mut_ptr())
        }
    };
    if result.is_null() {
        return Err("date/time value is out of range".into());
    }
    Ok(unsafe { broken_down.assume_init() })
}

fn format_strftime(format: &str, timestamp: i64, utc: bool) -> Value {
    if format.is_empty() {
        return Value::Bool(false);
    }
    let Ok(format) = std::ffi::CString::new(format) else {
        return Value::Bool(false);
    };
    let Ok(mut broken_down) = date_tm(timestamp, utc) else {
        return Value::Bool(false);
    };
    let mut output = [0u8; 4096];
    let length = unsafe {
        libc::strftime(
            output.as_mut_ptr().cast(),
            output.len(),
            format.as_ptr(),
            &mut broken_down,
        )
    };
    if length == 0 {
        Value::Bool(false)
    } else {
        Value::String(String::from_utf8_lossy(&output[..length]).into_owned())
    }
}

#[cfg(unix)]
fn parse_strptime(input: &str, format: &str) -> Value {
    let (Ok(input_c), Ok(format_c)) = (
        std::ffi::CString::new(input),
        std::ffi::CString::new(format),
    ) else {
        return Value::Bool(false);
    };
    let mut parsed = unsafe { std::mem::MaybeUninit::<libc::tm>::zeroed().assume_init() };
    let remainder = unsafe { libc::strptime(input_c.as_ptr(), format_c.as_ptr(), &mut parsed) };
    if remainder.is_null() {
        return Value::Bool(false);
    }
    let remaining = unsafe { remainder.offset_from(input_c.as_ptr()) }.max(0) as usize;
    let unparsed = input.get(remaining..).unwrap_or("");
    Value::from_array_entries(vec![
        (
            ArrayKey::String("tm_sec".into()),
            Value::Integer(parsed.tm_sec as i64),
        ),
        (
            ArrayKey::String("tm_min".into()),
            Value::Integer(parsed.tm_min as i64),
        ),
        (
            ArrayKey::String("tm_hour".into()),
            Value::Integer(parsed.tm_hour as i64),
        ),
        (
            ArrayKey::String("tm_mday".into()),
            Value::Integer(parsed.tm_mday as i64),
        ),
        (
            ArrayKey::String("tm_mon".into()),
            Value::Integer(parsed.tm_mon as i64),
        ),
        (
            ArrayKey::String("tm_year".into()),
            Value::Integer(parsed.tm_year as i64),
        ),
        (
            ArrayKey::String("tm_wday".into()),
            Value::Integer(parsed.tm_wday as i64),
        ),
        (
            ArrayKey::String("tm_yday".into()),
            Value::Integer(parsed.tm_yday as i64),
        ),
        (
            ArrayKey::String("unparsed".into()),
            Value::String(unparsed.to_string()),
        ),
    ])
}

fn make_date_timestamp(parts: [i64; 6], utc: bool) -> Option<i64> {
    let year = match parts[5] {
        0..=69 => parts[5] + 2000,
        70..=100 => parts[5] + 1900,
        _ => parts[5],
    };
    let mut tm = unsafe { std::mem::MaybeUninit::<libc::tm>::zeroed().assume_init() };
    tm.tm_sec = parts[2] as libc::c_int;
    tm.tm_min = parts[1] as libc::c_int;
    tm.tm_hour = parts[0] as libc::c_int;
    tm.tm_mday = parts[4] as libc::c_int;
    tm.tm_mon = (parts[3] - 1) as libc::c_int;
    tm.tm_year = (year - 1900) as libc::c_int;
    tm.tm_isdst = if utc { 0 } else { -1 };
    let timestamp = unsafe {
        if utc {
            libc::timegm(&mut tm)
        } else {
            libc::mktime(&mut tm)
        }
    };
    Some(timestamp as i64)
}

fn parse_php_strtotime(source: &str, base: i64) -> Option<i64> {
    let input = source.trim();
    let normalized = input.to_ascii_lowercase();
    if normalized == "now" {
        return Some(base);
    }
    let current = date_tm(base, false).ok()?;
    let mut parts = [
        current.tm_hour as i64,
        current.tm_min as i64,
        current.tm_sec as i64,
        (current.tm_mon + 1) as i64,
        current.tm_mday as i64,
        (current.tm_year + 1900) as i64,
    ];
    match normalized.as_str() {
        "today" | "tomorrow" | "yesterday" => {
            parts[0] = 0;
            parts[1] = 0;
            parts[2] = 0;
            if normalized == "tomorrow" {
                parts[4] += 1;
            }
            if normalized == "yesterday" {
                parts[4] -= 1;
            }
            return make_date_timestamp(parts, false);
        }
        _ => {}
    }
    if let Some((amount, unit)) = input.split_once(char::is_whitespace) {
        let amount = amount.parse::<i64>().ok()?;
        if !matches!(unit.trim().split_whitespace().count(), 1) {
            return None;
        }
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

    let (date_part, time_part) =
        if let Some((date, time)) = input.split_once('T').or_else(|| input.split_once('t')) {
            (date, Some(time))
        } else if let Some((date, time)) = input.split_once(' ') {
            (date, Some(time.trim()))
        } else {
            (input, None)
        };
    let mut date_fields = date_part.split('-');
    let year = date_fields.next()?.parse::<i64>().ok()?;
    let month = date_fields.next()?.parse::<i64>().ok()?;
    let day = date_fields.next()?.parse::<i64>().ok()?;
    if date_fields.next().is_some()
        || !(1..=32767).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
    {
        return None;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if day > month_days[(month - 1) as usize] {
        return None;
    }
    parts[3] = month;
    parts[4] = day;
    parts[5] = year;
    let mut utc = false;
    let mut offset_seconds = 0i64;
    if let Some(mut time) = time_part {
        if time.ends_with('Z') || time.ends_with('z') {
            utc = true;
            time = &time[..time.len() - 1];
        }
        if let Some(zone_start) = time
            .char_indices()
            .skip(5)
            .find_map(|(index, ch)| matches!(ch, '+' | '-').then_some(index))
        {
            let zone = &time[zone_start..];
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let mut offset = zone[1..].split(':');
            let hours = offset.next()?.parse::<i64>().ok()?;
            let minutes = offset.next()?.parse::<i64>().ok()?;
            if offset.next().is_some() || hours > 23 || minutes > 59 {
                return None;
            }
            offset_seconds = sign * (hours * 3600 + minutes * 60);
            utc = true;
            time = &time[..zone_start];
        }
        let mut clock = time.split(':');
        parts[0] = clock.next()?.parse().ok()?;
        parts[1] = clock.next()?.parse().ok()?;
        if let Some(second) = clock.next() {
            parts[2] = second.parse().ok()?;
        }
        if clock.next().is_some()
            || !(0..=24).contains(&parts[0])
            || !(0..=59).contains(&parts[1])
            || !(0..=60).contains(&parts[2])
        {
            return None;
        }
    } else {
        parts[0] = 0;
        parts[1] = 0;
        parts[2] = 0;
    }
    let timestamp = make_date_timestamp(parts, utc)?;
    Some(timestamp - offset_seconds)
}

fn getdate_value(timestamp: i64) -> Result<Value, String> {
    let timestamp = timestamp as libc::time_t;
    let mut broken_down = std::mem::MaybeUninit::<libc::tm>::zeroed();
    let result = unsafe { libc::localtime_r(&timestamp, broken_down.as_mut_ptr()) };
    if result.is_null() {
        return Err("getdate(): timestamp is out of range".into());
    }
    let tm = unsafe { broken_down.assume_init() };
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut entries = vec![
        (ArrayKey::Integer(0), Value::Integer(timestamp as i64)),
        (
            ArrayKey::String("seconds".into()),
            Value::Integer(tm.tm_sec as i64),
        ),
        (
            ArrayKey::String("minutes".into()),
            Value::Integer(tm.tm_min as i64),
        ),
        (
            ArrayKey::String("hours".into()),
            Value::Integer(tm.tm_hour as i64),
        ),
        (
            ArrayKey::String("mday".into()),
            Value::Integer(tm.tm_mday as i64),
        ),
        (
            ArrayKey::String("wday".into()),
            Value::Integer(tm.tm_wday as i64),
        ),
        (
            ArrayKey::String("mon".into()),
            Value::Integer((tm.tm_mon + 1) as i64),
        ),
        (
            ArrayKey::String("year".into()),
            Value::Integer((tm.tm_year + 1900) as i64),
        ),
        (
            ArrayKey::String("yday".into()),
            Value::Integer(tm.tm_yday as i64),
        ),
        (
            ArrayKey::String("weekday".into()),
            Value::String(DAYS[tm.tm_wday.clamp(0, 6) as usize].into()),
        ),
        (
            ArrayKey::String("month".into()),
            Value::String(MONTHS[tm.tm_mon.clamp(0, 11) as usize].into()),
        ),
    ];
    entries.shrink_to_fit();
    Ok(Value::from_array_entries(entries))
}

fn localtime_value(timestamp: i64, associative: bool) -> Result<Value, String> {
    let tm = date_tm(timestamp, false)?;
    let fields = [
        ("tm_sec", tm.tm_sec),
        ("tm_min", tm.tm_min),
        ("tm_hour", tm.tm_hour),
        ("tm_mday", tm.tm_mday),
        ("tm_mon", tm.tm_mon),
        ("tm_year", tm.tm_year),
        ("tm_wday", tm.tm_wday),
        ("tm_yday", tm.tm_yday),
        ("tm_isdst", tm.tm_isdst),
    ];
    Ok(Value::from_array_entries(
        fields
            .iter()
            .enumerate()
            .map(|(index, (name, value))| {
                let key = if associative {
                    ArrayKey::String((*name).into())
                } else {
                    ArrayKey::Integer(index as i64)
                };
                (key, Value::Integer(*value as i64))
            })
            .collect(),
    ))
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
        if utc {
            libc::gmtime_r(&timestamp, broken_down.as_mut_ptr())
        } else {
            libc::localtime_r(&timestamp, broken_down.as_mut_ptr())
        }
    };
    if result.is_null() {
        return Err("date(): timestamp is out of range".into());
    }
    let tm = unsafe { broken_down.assume_init() };
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let year = tm.tm_year + 1900;
    let month = tm.tm_mon + 1;
    let day = tm.tm_mday;
    let hour = tm.tm_hour;
    let weekday = tm.tm_wday.clamp(0, 6) as usize;
    let month_index = tm.tm_mon.clamp(0, 11) as usize;
    let offset = {
        #[cfg(target_os = "linux")]
        {
            tm.tm_gmtoff as i64
        }
        #[cfg(not(target_os = "linux"))]
        {
            0i64
        }
    };
    let absolute_offset = offset.unsigned_abs();
    let suffix = match day % 100 {
        11..=13 => "th",
        _ => match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        },
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
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
            'd' => output.push_str(&format!("{day:02}")),
            'D' => output.push_str(&DAYS[weekday][..3]),
            'j' => output.push_str(&day.to_string()),
            'l' => output.push_str(DAYS[weekday]),
            'N' => output.push_str(&(if weekday == 0 { 7 } else { weekday }).to_string()),
            'S' => output.push_str(suffix),
            'w' => output.push_str(&weekday.to_string()),
            'z' => output.push_str(&tm.tm_yday.to_string()),
            'W' => output.push_str(iso_week.trim()),
            'F' => output.push_str(MONTHS[month_index]),
            'm' => output.push_str(&format!("{month:02}")),
            'M' => output.push_str(&MONTHS[month_index][..3]),
            'n' => output.push_str(&month.to_string()),
            't' => output.push_str(&month_days[month_index].to_string()),
            'L' => output.push(if leap { '1' } else { '0' }),
            'o' => output.push_str(iso_year.trim()),
            'Y' => output.push_str(&year.to_string()),
            'y' => output.push_str(&format!("{:02}", year.rem_euclid(100))),
            'a' => output.push_str(if hour < 12 { "am" } else { "pm" }),
            'A' => output.push_str(if hour < 12 { "AM" } else { "PM" }),
            'g' | 'h' => {
                let twelve = match hour % 12 {
                    0 => 12,
                    value => value,
                };
                if token == 'h' {
                    output.push_str(&format!("{twelve:02}"));
                } else {
                    output.push_str(&twelve.to_string());
                }
            }
            'G' => output.push_str(&hour.to_string()),
            'H' => output.push_str(&format!("{hour:02}")),
            'i' => output.push_str(&format!("{:02}", tm.tm_min)),
            's' => output.push_str(&format!("{:02}", tm.tm_sec)),
            'u' => output.push_str("000000"),
            'v' => output.push_str("000"),
            'e' => {
                #[cfg(target_os = "linux")]
                {
                    if !utc {
                        let zone =
                            unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }.to_string_lossy();
                        output.push_str(&zone);
                    } else {
                        output.push_str("UTC");
                    }
                }
                #[cfg(not(target_os = "linux"))]
                {
                    output.push_str("UTC");
                }
            }
            'I' => output.push(if tm.tm_isdst > 0 { '1' } else { '0' }),
            'O' | 'P' => {
                let sign = if offset < 0 { '-' } else { '+' };
                if token == 'P' {
                    output.push_str(&format!(
                        "{sign}{:02}:{:02}",
                        absolute_offset / 3600,
                        absolute_offset % 3600 / 60
                    ));
                } else {
                    output.push_str(&format!(
                        "{sign}{:02}{:02}",
                        absolute_offset / 3600,
                        absolute_offset % 3600 / 60
                    ));
                }
            }
            'p' => {
                if offset == 0 {
                    output.push('Z');
                } else {
                    output.push_str(&format_php_date("P", timestamp as i64, utc)?);
                }
            }
            'T' => {
                #[cfg(target_os = "linux")]
                {
                    output.push_str(
                        &unsafe { std::ffi::CStr::from_ptr(tm.tm_zone) }.to_string_lossy(),
                    );
                }
                #[cfg(not(target_os = "linux"))]
                {
                    output.push_str("UTC");
                }
            }
            'Z' => output.push_str(&offset.to_string()),
            'B' => {
                let beats =
                    (timestamp.rem_euclid(86_400) + 3_600).rem_euclid(86_400) * 1_000 / 86_400;
                output.push_str(&format!("{beats:03}"));
            }
            'c' => output.push_str(&format_php_date("Y-m-d\\TH:i:sP", timestamp as i64, utc)?),
            'r' => output.push_str(&format_php_date("D, d M Y H:i:s O", timestamp as i64, utc)?),
            'U' => output.push_str(&(timestamp as i64).to_string()),
            other => output.push(other),
        }
    }
    Ok(output)
}

fn date_strftime(format: &str, tm: &libc::tm) -> String {
    let Ok(format) = std::ffi::CString::new(format) else {
        return String::new();
    };
    let mut buffer = [0u8; 32];
    let length = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            format.as_ptr(),
            tm,
        )
    };
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
    if lock {
        unsafe {
            libc::flock(std::os::fd::AsRawFd::as_raw_fd(&file), libc::LOCK_UN);
        }
    }
    result
}

fn create_temporary_file(
    directory: &std::path::Path,
    prefix: &str,
) -> std::io::Result<std::path::PathBuf> {
    static NEXT_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let prefix = prefix
        .chars()
        .take(32)
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    loop {
        let id = NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = directory.join(format!("{prefix}{}-{id}.tmp", std::process::id()));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
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
        let Some(value) = arguments.get_mut(position) else {
            continue;
        };
        if matches!(value.dereference(), Value::Object(_)) {
            *value = super::objects::stringify(
                value,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )
            .map(Value::String)
            .map_err(|error| format!("{function_name}(): {error}"))?;
        }
    }
    Ok(())
}

pub(super) fn apply_exit_status(
    status: Value,
    output: &mut String,
    global_variables: &mut GlobalVariables,
    strict_types: bool,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    steps: &mut u64,
) -> Result<(), String> {
    let mut status = status.dereference();
    if !strict_types {
        if let Value::Object(object) = &status {
            let class_name = object.borrow().class_name.clone();
            status = match super::objects::stringify(
                &status,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            ) {
                Ok(string) => Value::String(string),
                Err(error) if error.starts_with("Object of class ") => {
                    return Err(format!(
                        "TypeError:exit(): Argument #1 ($status) must be of type string|int, {class_name} given"
                    ));
                }
                Err(error) if error.starts_with(super::THROWN_VALUE_PREFIX) => return Err(error),
                Err(error) => return Err(format!("exit(): {error}")),
            };
        }
    }
    let status = match status {
        Value::Integer(_) | Value::String(_) => status,
        Value::Bool(value) if !strict_types => Value::Integer(i64::from(value)),
        Value::Number(value) if !strict_types && value.is_finite() => {
            Value::Integer(value.trunc() as i64)
        }
        value => {
            return Err(format!(
                "TypeError:exit(): Argument #1 ($status) must be of type string|int, {} given",
                value.type_name()
            ));
        }
    };
    match status {
        Value::Integer(code) => global_variables.exit_code = code as i32,
        Value::String(message) => {
            output.push_str(&message);
            global_variables.exit_code = 0;
        }
        _ => unreachable!("exit status coercion only returns int|string"),
    }
    global_variables.exiting = true;
    Ok(())
}

fn format_dump_value(
    value: &Value,
    depth: usize,
    seen: &mut HashSet<usize>,
    object_ids: &mut HashMap<usize, usize>,
    next_object_id: &mut usize,
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
        if !seen.insert(identity) {
            return Ok("*RECURSION*\n".into());
        }
        let dump = format_dump_value(
            &reference.borrow(),
            depth,
            seen,
            object_ids,
            next_object_id,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        );
        seen.remove(&identity);
        return dump;
    }
    let value = value.dereference();
    Ok(match value {
        Value::Null => format!("{indent}NULL\n"),
        Value::Bool(value) => format!("{indent}bool({value})\n"),
        Value::Integer(value) => format!("{indent}int({value})\n"),
        Value::Number(value) => format!("{indent}float({})\n", Value::Number(value).show()),
        Value::String(value) => format!(
            "{indent}string({}) \"{}\"\n",
            value.len(),
            escape_dump_string(&value)
        ),
        Value::Array(array) => {
            if depth >= 64 {
                return Err("var_dump(): nesting level too deep".into());
            }
            let mut dump = format!("{indent}array({}) {{\n", array.len());
            for (key, item) in array.iter() {
                let key = match key {
                    ArrayKey::Integer(key) => format!("[{key}]"),
                    ArrayKey::String(key) => format!("[\"{}\"]", escape_dump_string(key)),
                };
                dump.push_str(&format!("{nested_indent}{key}=>\n"));
                dump.push_str(&format_dump_value(
                    item,
                    depth + 1,
                    seen,
                    object_ids,
                    next_object_id,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                )?);
            }
            dump.push_str(&format!("{indent}}}\n"));
            dump
        }
        Value::Object(object) => {
            let identity = Rc::as_ptr(&object) as usize;
            if !seen.insert(identity) {
                return Ok("*RECURSION*\n".into());
            }
            let object_id = *object_ids.entry(identity).or_insert_with(|| {
                let id = *next_object_id;
                *next_object_id += 1;
                id
            });
            if depth >= 64 {
                return Err("var_dump(): nesting level too deep".into());
            }
            let (class_name, raw_properties, property_order) = {
                let object = object.borrow();
                (
                    object.class_name.clone(),
                    object.properties.clone(),
                    object.property_order.clone(),
                )
            };
            let properties = if let Some(method) =
                super::objects::find_method(&class_name, "__debuginfo", classes)
            {
                let mut locals = HashMap::from([("this".into(), Value::Object(object.clone()))]);
                super::initialize_local_order(&mut locals, ["this".to_string()]);
                let returned = execute_user_frame(
                    &method.instructions,
                    functions,
                    classes,
                    static_properties,
                    &mut locals,
                    global_variables,
                    false,
                    &function_scope(method),
                    HashMap::new(),
                    output,
                    steps,
                )?
                .unwrap_or(Value::Null);
                finish_user_frame(
                    locals,
                    &returned,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                )?;
                match returned.dereference() {
                    Value::Array(properties) => Value::Array(properties),
                    other => {
                        return Err(format!(
                        "TypeError:{class_name}::__debugInfo() must return an array, {} returned",
                        other.type_name()
                    ))
                    }
                }
            } else {
                let mut names = property_order;
                let mut remaining = raw_properties
                    .keys()
                    .filter(|name| !names.contains(name))
                    .cloned()
                    .collect::<Vec<_>>();
                remaining.sort();
                names.extend(remaining);
                Value::from_array_entries(
                    names
                        .into_iter()
                        .filter_map(|name| {
                            raw_properties.get(&name).cloned().map(|value| {
                                (
                                    ArrayKey::String(dump_property_name(
                                        &name,
                                        &class_name,
                                        classes,
                                    )),
                                    value,
                                )
                            })
                        })
                        .collect(),
                )
            };
            let Value::Array(properties) = properties.dereference() else {
                unreachable!()
            };
            let property_count = properties.len();
            let mut dump =
                format!("{indent}object({class_name})#{object_id} ({property_count}) {{\n");
            for (key, item) in properties.iter() {
                let key = match key {
                    ArrayKey::Integer(key) => format!("[{key}]"),
                    ArrayKey::String(key) => format!("[\"{}\"]", escape_dump_string(key)),
                };
                dump.push_str(&format!("{nested_indent}{key}=>\n"));
                dump.push_str(&format_dump_value(
                    item,
                    depth + 1,
                    seen,
                    object_ids,
                    next_object_id,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                )?);
            }
            dump.push_str(&format!("{indent}}}\n"));
            seen.remove(&identity);
            dump
        }
        Value::Closure(_) => format!("{indent}object(Closure)\n"),
        Value::Generator(_) => format!("{indent}object(Generator)\n"),
        Value::Stream(_) => format!("{indent}resource(stream) of type (stream)\n"),
        Value::Reference(_) => unreachable!(),
    })
}

fn dump_property_name(
    storage_name: &str,
    class_name: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> String {
    if let Some(private_name) = storage_name.strip_prefix('\0') {
        if let Some((declaring_class, property)) = private_name.split_once("::") {
            let (display_class, display_property) =
                classes
                    .get(declaring_class)
                    .map_or((declaring_class, property), |class| {
                        (
                            class.name.as_str(),
                            class
                                .properties
                                .keys()
                                .find(|name| name.eq_ignore_ascii_case(property))
                                .map_or(property, String::as_str),
                        )
                    });
            return format!("\0{display_class}\0{display_property}");
        }
    }
    let mut current = Some(class_name);
    while let Some(name) = current {
        if let Some(class) = classes.get(&name.to_ascii_lowercase()) {
            if let Some((property_name, property)) = class
                .properties
                .iter()
                .find(|(property_name, _)| property_name.eq_ignore_ascii_case(storage_name))
            {
                if property.visibility == crate::MemberVisibility::Protected {
                    return format!("\0*\0{property_name}");
                }
            }
            current = class.parent.as_deref();
        } else {
            break;
        }
    }
    storage_name.to_string()
}

fn escape_dump_string(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\0' => escaped.push_str("\\0"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            character if character.is_control() => {
                escaped.push_str(&format!("\\x{:02x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn getenv_value(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.len() > 2 {
        return Err(format!(
            "getenv() expects at most 2 arguments, {} given",
            arguments.len()
        ));
    }
    let name = arguments.first().map(Value::dereference);
    if name
        .as_ref()
        .is_none_or(|value| matches!(value, Value::Null))
    {
        let mut entries = std::env::vars_os()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect::<Vec<_>>();
        for (name, value) in &globals.environment {
            if let Some(index) = entries.iter().position(|(key, _)| key == name) {
                if let Some(value) = value {
                    entries[index].1 = value.clone();
                } else {
                    entries.remove(index);
                }
            } else if let Some(value) = value {
                entries.push((name.clone(), value.clone()));
            }
        }
        let entries = entries
            .into_iter()
            .map(|(name, value)| (ArrayKey::String(name), Value::String(value)))
            .collect();
        return Ok(Value::from_array_entries(entries));
    }
    let name = name.unwrap().show();
    if name.is_empty() {
        return Ok(Value::Bool(false));
    }
    if let Some(value) = globals.environment.get(&name) {
        return Ok(value
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Bool(false)));
    }
    Ok(std::env::var_os(name)
        .map(|value| Value::String(value.to_string_lossy().into_owned()))
        .unwrap_or(Value::Bool(false)))
}

fn putenv_value(arguments: &[Value], globals: &mut GlobalVariables) -> Result<Value, String> {
    if arguments.len() != 1 {
        return Err(format!(
            "putenv() expects exactly 1 argument, {} given",
            arguments.len()
        ));
    }
    let assignment = arguments[0].show();
    let (name, value) = match assignment.split_once('=') {
        Some((name, value)) if !name.is_empty() => (name.to_string(), Some(value.to_string())),
        Some(_) => return Ok(Value::Bool(false)),
        None if !assignment.is_empty() => (assignment, None),
        None => return Ok(Value::Bool(false)),
    };
    globals.environment.insert(name, value);
    Ok(Value::Bool(true))
}

fn copy_value(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.len() < 2 {
        return Err(format!(
            "copy() expects at least 2 arguments, {} given",
            arguments.len()
        ));
    }
    if arguments.len() > 3 {
        return Err(format!(
            "copy() expects at most 3 arguments, {} given",
            arguments.len()
        ));
    }
    if let Some(context) = arguments
        .get(2)
        .filter(|value| !matches!(value.dereference(), Value::Null | Value::Stream(_)))
    {
        return Err(format!(
            "TypeError:copy(): Argument #3 ($context) must be of type ?resource, {} given",
            context.dereference().type_name()
        ));
    }
    let from = arguments[0].show();
    let to = arguments[1].show();
    let copied = filesystem_lookup_path(globals, std::path::Path::new(&from), false, false)
        .zip(filesystem_path(globals, std::path::Path::new(&to), true))
        .is_some_and(|(from_path, to_path)| std::fs::copy(from_path, to_path).is_ok());
    if !copied {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("copy(): Failed to copy {from} to {to}"),
            source_file: None,
            source_line: None,
            output_offset: output.len(),
        });
    }
    Ok(Value::Bool(copied))
}

fn rename_value(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.len() < 2 {
        return Err(format!(
            "rename() expects at least 2 arguments, {} given",
            arguments.len()
        ));
    }
    if arguments.len() > 3 {
        return Err(format!(
            "rename() expects at most 3 arguments, {} given",
            arguments.len()
        ));
    }
    if let Some(context) = arguments
        .get(2)
        .filter(|value| !matches!(value.dereference(), Value::Null | Value::Stream(_)))
    {
        return Err(format!(
            "TypeError:rename(): Argument #3 ($context) must be of type ?resource, {} given",
            context.dereference().type_name()
        ));
    }
    let from = arguments[0].show();
    let to = arguments[1].show();
    let renamed = filesystem_lookup_path(globals, std::path::Path::new(&from), false, false)
        .zip(filesystem_path(globals, std::path::Path::new(&to), true))
        .is_some_and(|(from_path, to_path)| std::fs::rename(from_path, to_path).is_ok());
    if !renamed {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("rename(): Failed to rename {from} to {to}"),
            source_file: None,
            source_line: None,
            output_offset: output.len(),
        });
    }
    Ok(Value::Bool(renamed))
}

fn mkdir_value(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.is_empty() {
        return Err("mkdir() expects at least 1 argument, 0 given".into());
    }
    if arguments.len() > 4 {
        return Err(format!(
            "mkdir() expects at most 4 arguments, {} given",
            arguments.len()
        ));
    }
    if let Some(context) = arguments
        .get(3)
        .filter(|value| !matches!(value.dereference(), Value::Null | Value::Stream(_)))
    {
        return Err(format!(
            "TypeError:mkdir(): Argument #4 ($context) must be of type ?resource, {} given",
            context.dereference().type_name()
        ));
    }
    let directory = arguments[0].show();
    let mode = arguments
        .get(1)
        .map(|value| value.number() as i64)
        .unwrap_or(0o777);
    let recursive = arguments.get(2).is_some_and(Value::truthy);
    let path = if recursive {
        filesystem_recursive_create_path(globals, std::path::Path::new(&directory))
    } else {
        filesystem_path(globals, std::path::Path::new(&directory), true)
    };
    let created = path.is_some_and(|path| {
        !path.exists() && create_directory(&path, (mode & 0o7777) as u32, recursive).is_ok()
    });
    if !created {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("mkdir(): Failed to create directory {directory}"),
            source_file: None,
            source_line: None,
            output_offset: output.len(),
        });
    }
    Ok(Value::Bool(created))
}

fn rmdir_value(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.is_empty() {
        return Err("rmdir() expects exactly 1 argument, 0 given".into());
    }
    if arguments.len() > 2 {
        return Err(format!(
            "rmdir() expects at most 2 arguments, {} given",
            arguments.len()
        ));
    }
    if let Some(context) = arguments
        .get(1)
        .filter(|value| !matches!(value.dereference(), Value::Null | Value::Stream(_)))
    {
        return Err(format!(
            "TypeError:rmdir(): Argument #2 ($context) must be of type ?resource, {} given",
            context.dereference().type_name()
        ));
    }
    let directory = arguments[0].show();
    let removed = filesystem_path(globals, std::path::Path::new(&directory), false)
        .is_some_and(|path| std::fs::remove_dir(path).is_ok());
    if !removed {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("rmdir(): Failed to remove directory {directory}"),
            source_file: None,
            source_line: None,
            output_offset: output.len(),
        });
    }
    Ok(Value::Bool(removed))
}

fn touch_value(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.is_empty() {
        return Err("touch() expects at least 1 argument, 0 given".into());
    }
    if arguments.len() > 3 {
        return Err(format!(
            "touch() expects at most 3 arguments, {} given",
            arguments.len()
        ));
    }
    let filename = arguments[0].show();
    let now = std::time::SystemTime::now();
    let to_time = |value: Option<&Value>, default: std::time::SystemTime| {
        let Some(value) = value.filter(|value| !matches!(value.dereference(), Value::Null)) else {
            return Some(default);
        };
        let seconds = value.number() as i64;
        if seconds >= 0 {
            std::time::SystemTime::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(seconds as u64))
        } else {
            std::time::SystemTime::UNIX_EPOCH
                .checked_sub(std::time::Duration::from_secs(seconds.unsigned_abs()))
        }
    };
    let modified = to_time(arguments.get(1), now)
        .ok_or_else(|| "touch(): timestamp is out of range".to_string())?;
    let accessed = to_time(arguments.get(2), modified)
        .ok_or_else(|| "touch(): timestamp is out of range".to_string())?;
    let touched =
        filesystem_path(globals, std::path::Path::new(&filename), true).is_some_and(|path| {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .open(&path)
                .or_else(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(path)
                    } else {
                        Err(error)
                    }
                });
            file.is_ok_and(|file| {
                file.set_times(
                    std::fs::FileTimes::new()
                        .set_modified(modified)
                        .set_accessed(accessed),
                )
                .is_ok()
            })
        });
    if !touched {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("touch(): Unable to create file {filename}"),
            source_file: None,
            source_line: None,
            output_offset: output.len(),
        });
    }
    Ok(Value::Bool(touched))
}

fn chmod_value(
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.len() != 2 {
        return Err(format!(
            "chmod() expects exactly 2 arguments, {} given",
            arguments.len()
        ));
    }
    let filename = arguments[0].show();
    let mode = arguments[1].number() as u32;
    let changed =
        filesystem_path(globals, std::path::Path::new(&filename), false).is_some_and(|path| {
            let Ok(metadata) = std::fs::metadata(&path) else {
                return false;
            };
            let mut permissions = metadata.permissions();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                permissions.set_mode(mode & 0o7777);
            }
            #[cfg(windows)]
            permissions.set_readonly(mode & 0o200 == 0);
            std::fs::set_permissions(path, permissions).is_ok()
        });
    if !changed {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("chmod(): Operation not permitted: {filename}"),
            source_file: None,
            source_line: None,
            output_offset: output.len(),
        });
    }
    Ok(Value::Bool(changed))
}

fn disk_space_value(
    function: &str,
    arguments: &[Value],
    globals: &mut GlobalVariables,
    output: &str,
) -> Result<Value, String> {
    if arguments.len() != 1 {
        return Err(format!(
            "{function}() expects exactly 1 argument, {} given",
            arguments.len()
        ));
    }
    let directory = arguments[0].show();
    let bytes =
        filesystem_path(globals, std::path::Path::new(&directory), false).and_then(|path| {
            disk_space_bytes(&path, function.eq_ignore_ascii_case("disk_total_space"))
        });
    match bytes {
        Some(bytes) => Ok(Value::Number(bytes)),
        None => {
            globals.report_diagnostic(crate::RuntimeDiagnostic {
                level: crate::DiagnosticLevel::Warning,
                message: format!("{function}(): could not find path {directory}"),
                source_file: None,
                source_line: None,
                output_offset: output.len(),
            });
            Ok(Value::Bool(false))
        }
    }
}

#[cfg(unix)]
fn disk_space_bytes(path: &std::path::Path, total: bool) -> Option<f64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = if stats.f_frsize != 0 {
        stats.f_frsize
    } else {
        stats.f_bsize
    } as f64;
    let block_count = if total {
        stats.f_blocks
    } else {
        stats.f_bavail
    } as f64;
    Some(block_size * block_count)
}

#[cfg(not(unix))]
fn disk_space_bytes(_path: &std::path::Path, _total: bool) -> Option<f64> {
    None
}

fn create_directory(path: &std::path::Path, mode: u32, recursive: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(recursive).mode(mode);
        builder.create(path)
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        if recursive {
            std::fs::create_dir_all(path)
        } else {
            std::fs::create_dir(path)
        }
    }
}

fn filesystem_recursive_create_path(
    globals: &GlobalVariables,
    path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let Some(root) = globals.get("\0filesystem_root") else {
        return Some(path.to_path_buf());
    };
    let root = std::path::PathBuf::from(root.show()).canonicalize().ok()?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut normalized = std::path::PathBuf::new();
    for component in candidate.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    if !normalized.starts_with(&root) {
        return None;
    }
    let canonical = normalized.canonicalize();
    if let Ok(canonical) = canonical {
        return canonical.starts_with(&root).then_some(canonical);
    }
    let mut existing = normalized.as_path();
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let canonical_parent = existing.canonicalize().ok()?;
    canonical_parent.starts_with(&root).then_some(normalized)
}

fn runtime_function_exists(name: &str, functions: &HashMap<String, BytecodeFunction>) -> bool {
    let name = name.trim_start_matches('\\').to_ascii_lowercase();
    functions.contains_key(&name)
        || crate::array_builtins::is_supported(&name)
        || crate::math_builtins::is_supported(&name)
        || crate::string_builtins::is_supported(&name)
        || matches!(
            name.as_str(),
            "call_user_func"
                | "call_user_func_array"
                | "exit"
                | "die"
                | "func_get_args"
                | "func_get_arg"
                | "func_num_args"
                | "var_dump"
                | "function_exists"
                | "gc_collect_cycles"
                | "php_sapi_name"
                | "error_reporting"
                | "error_get_last"
                | "error_clear_last"
                | "class_exists"
                | "interface_exists"
                | "trait_exists"
                | "method_exists"
                | "property_exists"
                | "get_class"
                | "get_parent_class"
                | "get_called_class"
                | "is_a"
                | "is_subclass_of"
                | "is_callable"
                | "getenv"
                | "putenv"
                | "mongo_find"
                | "mongo_aggregate"
                | "mongo_count_documents"
                | "mongo_estimated_document_count"
                | "mongo_insert_one"
                | "mongo_insert_many"
                | "mongo_replace_one"
                | "mongo_update_one"
                | "mongo_update_many"
                | "mongo_delete_one"
                | "mongo_delete_many"
                | "mongo_begin_transaction"
                | "mongo_commit"
                | "mongo_rollback"
                | "header"
                | "http_response_code"
                | "header_remove"
                | "headers_list"
                | "file_exists"
                | "is_file"
                | "is_dir"
                | "is_link"
                | "is_readable"
                | "is_writable"
                | "is_executable"
                | "filesize"
                | "filemtime"
                | "fileatime"
                | "filectime"
                | "fileinode"
                | "fileowner"
                | "filegroup"
                | "fileperms"
                | "filetype"
                | "stat"
                | "lstat"
                | "scandir"
                | "opendir"
                | "readdir"
                | "closedir"
                | "rewinddir"
                | "realpath"
                | "getcwd"
                | "get_include_path"
                | "set_include_path"
                | "sys_get_temp_dir"
                | "tempnam"
                | "unlink"
                | "touch"
                | "chmod"
                | "disk_free_space"
                | "diskfreespace"
                | "disk_total_space"
                | "copy"
                | "rename"
                | "mkdir"
                | "rmdir"
                | "md5_file"
                | "file_get_contents"
                | "file_put_contents"
                | "file"
                | "fopen"
                | "fwrite"
                | "fread"
                | "stream_get_contents"
                | "rewind"
                | "fseek"
                | "ftell"
                | "feof"
                | "fclose"
                | "time"
                | "microtime"
                | "hrtime"
                | "random_int"
                | "rand"
                | "mt_rand"
                | "getrandmax"
                | "mt_getrandmax"
                | "date"
                | "gmdate"
                | "strftime"
                | "gmstrftime"
                | "getdate"
                | "gettimeofday"
                | "idate"
                | "checkdate"
                | "mktime"
                | "gmmktime"
                | "localtime"
                | "strtotime"
                | "json_encode"
                | "json_decode"
                | "json_validate"
                | "json_last_error"
                | "json_last_error_msg"
                | "is_uploaded_file"
                | "move_uploaded_file"
                | "session_start"
                | "session_destroy"
                | "session_unset"
                | "session_status"
                | "session_id"
                | "strtolower"
                | "strtoupper"
                | "strlen"
                | "ucfirst"
                | "lcfirst"
                | "strrev"
                | "bin2hex"
                | "ord"
                | "number_format"
                | "str_increment"
                | "str_decrement"
                | "strspn"
                | "strcspn"
                | "strpbrk"
                | "str_word_count"
                | "strcmp"
                | "strcasecmp"
                | "strncmp"
                | "strncasecmp"
                | "strnatcmp"
                | "strnatcasecmp"
                | "strtr"
                | "strstr"
                | "strchr"
                | "stristr"
                | "ucwords"
                | "trim"
                | "ltrim"
                | "rtrim"
                | "str_repeat"
                | "str_pad"
                | "str_rot13"
                | "str_shuffle"
                | "addslashes"
                | "stripslashes"
                | "str_split"
                | "strpos"
                | "stripos"
                | "strrpos"
                | "strripos"
                | "substr_count"
                | "substr_compare"
                | "substr"
                | "explode"
                | "implode"
                | "htmlspecialchars"
                | "htmlspecialchars_decode"
                | "nl2br"
                | "str_contains"
                | "str_starts_with"
                | "str_ends_with"
                | "str_replace"
                | "str_ireplace"
                | "define"
                | "defined"
                | "constant"
                | "compact"
                | "extract"
                | "get_defined_vars"
                | "intval"
                | "floatval"
                | "strval"
                | "boolval"
                | "gettype"
                | "get_debug_type"
                | "is_int"
                | "is_integer"
                | "is_long"
                | "is_float"
                | "is_double"
                | "is_real"
                | "is_string"
                | "is_array"
                | "is_countable"
                | "is_iterable"
                | "is_bool"
                | "is_null"
                | "is_object"
                | "is_resource"
                | "is_scalar"
                | "is_numeric"
        )
        || super::builtin_arguments::is_vm_function(&name)
}

fn type_builtin(
    name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    let max_arguments = if name == "intval" { 2 } else { 1 };
    if arguments.is_empty() || arguments.len() > max_arguments {
        if name == "intval" {
            let (expectation, noun) = if arguments.is_empty() {
                ("at least 1", "argument")
            } else {
                ("at most 2", "arguments")
            };
            return Err(format!(
                "ArgumentCountError:intval() expects {expectation} {noun}, {} given",
                arguments.len()
            ));
        }
        return Err(format!(
            "ArgumentCountError:{name}() expects exactly 1 argument, {} given",
            arguments.len()
        ));
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
                && !matches!(
                    text.to_ascii_lowercase().as_str(),
                    "nan"
                        | "+nan"
                        | "-nan"
                        | "inf"
                        | "+inf"
                        | "-inf"
                        | "infinity"
                        | "+infinity"
                        | "-infinity"
                )
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
                        return Err(
                            "ValueError:intval(): Argument #2 ($base) must be between 2 and 36"
                                .into(),
                        );
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
        "gettype" => Value::String(
            match &value {
                Value::Null => "NULL",
                Value::Bool(_) => "boolean",
                Value::Integer(_) => "integer",
                Value::Number(_) => "double",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Stream(_) => "resource",
                Value::Object(_) | Value::Closure(_) | Value::Generator(_) => "object",
                Value::Reference(_) => unreachable!(),
            }
            .into(),
        ),
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
        "is_object" => Value::Bool(matches!(
            &value,
            Value::Object(_) | Value::Closure(_) | Value::Generator(_)
        )),
        "is_resource" => {
            Value::Bool(matches!(&value, Value::Stream(stream) if stream.borrow().is_open()))
        }
        "is_scalar" => Value::Bool(matches!(
            &value,
            Value::Bool(_) | Value::Integer(_) | Value::Number(_) | Value::String(_)
        )),
        "is_numeric" => Value::Bool(is_numeric),
        _ => unreachable!("type builtin dispatch is exhaustive"),
    };
    Ok(result)
}

pub(super) fn intval_string(input: &str, requested_base: u32) -> i64 {
    let text = input.trim_start_matches(|character| {
        matches!(character, ' ' | '\t' | '\n' | '\r' | '\x0b' | '\x0c' | '\0')
    });
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (base, digits) = if requested_base == 0 {
        if let Some(rest) = unsigned
            .strip_prefix("0x")
            .or_else(|| unsigned.strip_prefix("0X"))
        {
            (16, rest)
        } else if let Some(rest) = unsigned
            .strip_prefix("0b")
            .or_else(|| unsigned.strip_prefix("0B"))
        {
            (2, rest)
        } else if unsigned.starts_with('0') {
            (8, unsigned)
        } else {
            (10, unsigned)
        }
    } else if requested_base == 16 {
        (
            16,
            unsigned
                .strip_prefix("0x")
                .or_else(|| unsigned.strip_prefix("0X"))
                .unwrap_or(unsigned),
        )
    } else if requested_base == 2 {
        (
            2,
            unsigned
                .strip_prefix("0b")
                .or_else(|| unsigned.strip_prefix("0B"))
                .unwrap_or(unsigned),
        )
    } else {
        (requested_base, unsigned)
    };

    // Decimal floats and exponents are converted as numbers before truncation.
    if base == 10 {
        let mut numeric_prefix = None;
        for (index, _) in digits
            .char_indices()
            .skip(1)
            .chain(std::iter::once((digits.len(), '\0')))
        {
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
        if digit >= base {
            break;
        }
        found = true;
        magnitude = magnitude
            .saturating_mul(base as i128)
            .saturating_add(digit as i128);
    }
    if !found {
        return 0;
    }
    let signed = if negative {
        magnitude.saturating_neg()
    } else {
        magnitude
    };
    signed.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn declared_property_exists(
    class_name: &str,
    property: &str,
    classes: &HashMap<String, BytecodeClass>,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    let key = class_name.trim_start_matches('\\').to_ascii_lowercase();
    if !visited.insert(key.clone()) {
        return false;
    }
    let Some(class) = classes.get(&key) else {
        return false;
    };
    class.properties.contains_key(property)
        || class.static_properties.contains_key(property)
        || class
            .parent
            .as_deref()
            .is_some_and(|parent| declared_property_exists(parent, property, classes, visited))
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
                let callable = valid
                    && (syntax_only
                        || classes.get(&class).is_some_and(|_| {
                            find_method(&class, &method, classes).is_some_and(|function| {
                                function.is_static
                                    && function.visibility == MemberVisibility::Public
                            }) || find_method(&class, "__callstatic", classes).is_some_and(
                                |function| {
                                    function.is_static
                                        && function.visibility == MemberVisibility::Public
                                },
                            )
                        }));
                (callable, format!("{class}::{method}"))
            } else {
                let valid = !name.trim().is_empty();
                (
                    valid && (syntax_only || runtime_function_exists(&name, functions)),
                    name.clone(),
                )
            }
        }
        Value::Array(array) => {
            let entries = array.iter().collect::<Vec<_>>();
            if entries.len() != 2
                || entries[0].0 != ArrayKey::Integer(0)
                || entries[1].0 != ArrayKey::Integer(1)
            {
                return (false, String::new());
            }
            let Value::String(method) = entries[1].1.dereference() else {
                return (false, String::new());
            };
            let method = method.to_ascii_lowercase();
            let class_name = match entries[0].1.dereference() {
                Value::Object(object) => object.borrow().class_name.to_ascii_lowercase(),
                Value::String(class_name) => {
                    class_name.trim_start_matches('\\').to_ascii_lowercase()
                }
                _ => return (false, String::new()),
            };
            let is_object = matches!(entries[0].1.dereference(), Value::Object(_));
            let callable = if syntax_only {
                true
            } else {
                let user_method =
                    find_method(&class_name, &method, classes).is_some_and(|function| {
                        function.visibility == MemberVisibility::Public
                            && function.is_static == !is_object
                    });
                let magic_method = find_method(
                    &class_name,
                    if is_object { "__call" } else { "__callstatic" },
                    classes,
                )
                .is_some_and(|function| {
                    function.visibility == MemberVisibility::Public
                        && function.is_static == !is_object
                });
                let native_exception_method = is_object
                    && super::is_a_type(
                        &class_name,
                        "throwable",
                        classes,
                        &mut std::collections::HashSet::new(),
                    )
                    && crate::exception_builtins::method_exists("Exception", &method);
                user_method || magic_method || native_exception_method
            };
            (callable, format!("{class_name}::{method}"))
        }
        Value::Object(object) => {
            let class_name = object.borrow().class_name.clone();
            let callable = syntax_only
                || find_method(&class_name, "__invoke", classes).is_some_and(|function| {
                    !function.is_static && function.visibility == MemberVisibility::Public
                });
            (callable, format!("{class_name}::__invoke"))
        }
        _ => (false, String::new()),
    }
}

fn autoload_callback_identity(value: &Value) -> String {
    match value.dereference() {
        Value::String(name) => format!("function:{}", name.to_ascii_lowercase()),
        Value::Object(object) => format!("object:{:p}", Rc::as_ptr(&object)),
        Value::Closure(closure) => format!("closure:{:p}", Rc::as_ptr(&closure)),
        Value::Array(array) => {
            let parts = array
                .iter()
                .map(|(key, value)| {
                    let item = match value.dereference() {
                        Value::Object(object) => format!("object:{:p}", Rc::as_ptr(&object)),
                        Value::Closure(closure) => format!("closure:{:p}", Rc::as_ptr(&closure)),
                        value => value.show(),
                    };
                    format!("{key:?}={item}")
                })
                .collect::<Vec<_>>()
                .join(";");
            format!("array:{parts}")
        }
        other => format!("other:{}", other.show()),
    }
}

fn is_builtin_class_for_autoload(name: &str) -> bool {
    crate::exception_builtins::is_class(name)
        || matches!(
            name,
            "generator"
                | "pdo"
                | "pdostatement"
                | "reflectionclass"
                | "reflectionclassconstant"
                | "reflectionconstant"
                | "reflectionenum"
                | "reflectionenumunitcase"
                | "reflectionenumbackedcase"
                | "reflectionnamedtype"
                | "reflectiontype"
                | "reflectionuniontype"
                | "reflectionintersectiontype"
                | "reflectionfunction"
                | "reflectionmethod"
                | "reflectionproperty"
                | "reflectionattribute"
                | "reflectionparameter"
                | "attribute"
                | "countable"
                | "iterator"
                | "iteratoraggregate"
                | "traversable"
                | "stringable"
                | "jsonserializable"
                | "mongodb\\client"
                | "mongodb\\database"
                | "mongodb\\collection"
                | "mongodb\\cursor"
                | "mongodb\\insertresult"
                | "mongodb\\insertmanyresult"
                | "mongodb\\updateresult"
                | "mongodb\\deleteresult"
        )
}

pub(super) fn invoke_registered_autoloaders(
    requested: &str,
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
) -> Result<(), String> {
    let key = requested.trim_start_matches('\\').to_ascii_lowercase();
    if !global_variables.autoload_in_progress.insert(key.clone()) {
        return Ok(());
    }
    let callbacks = global_variables.autoload_callbacks.clone();
    let mut result = Ok(());
    for callback in callbacks {
        let arguments = vec![Value::String(requested.into())];
        let invocation = match callback.dereference() {
            Value::String(name) if name.eq_ignore_ascii_case("spl_autoload") => {
                invoke_spl_autoload(
                    requested,
                    None,
                    variables,
                    bindings,
                    global_variables,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )
            }
            Value::String(name) => invoke_callable(
                &name,
                arguments,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )
            .map(|_| ()),
            Value::Array(_) => invoke_array_callable(
                &callback,
                arguments,
                &[],
                &[],
                variables,
                bindings,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )
            .map(|_| ()),
            Value::Object(_) => invoke_object_callable(
                &callback,
                arguments,
                &[],
                &[],
                variables,
                bindings,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )
            .map(|_| ()),
            Value::Closure(closure) => invoke_closure(
                &closure,
                arguments,
                &[],
                &[],
                variables,
                bindings,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )
            .map(|_| ()),
            _ => Err("Autoload callback must be callable".into()),
        };
        if let Err(error) = invocation {
            result = Err(error);
            break;
        }
        let loaded =
            classes.contains_key(&key) || global_variables.dynamic_classes.contains_key(&key);
        if loaded {
            break;
        }
    }
    global_variables.autoload_in_progress.remove(&key);
    result
}

fn invoke_spl_autoload(
    requested: &str,
    file_extensions: Option<&str>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    _frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let Some(loader) = super::INCLUDE_LOADER.with(|loader| loader.borrow().clone()) else {
        return Err("spl_autoload() requires a source loader; use compile_file()".into());
    };
    let relative = requested.trim_start_matches('\\').replace('\\', "/");
    let extensions = file_extensions
        .map(str::to_string)
        .or_else(|| global_variables.autoload_extensions.clone())
        .unwrap_or_else(|| ".inc,.php".into());
    let current_file = global_variables
        .active_trace_frames
        .last()
        .map(|frame| frame.file.as_str())
        .unwrap_or(&global_variables.source_file)
        .to_string();
    let include_path = global_variables.include_path().to_string();
    for extension in extensions
        .split(',')
        .filter(|extension| !extension.is_empty())
    {
        let candidate = format!("{relative}{extension}");
        match loader(&candidate, &current_file, &include_path) {
            Ok(module) => {
                global_variables
                    .included_files
                    .insert(module.source_file.clone());
                super::execute_dynamic_module(
                    module,
                    variables,
                    bindings,
                    global_variables,
                    global_scope,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )?;
                return Ok(());
            }
            Err(error) if error.contains("No such file or directory") => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(super) fn prepare_reference_arguments(
    arguments: Vec<Value>,
    reference_names: &[Option<ReferenceArgument>],
    function: &BytecodeFunction,
    argument_names: &[Option<String>],
    call_name: &str,
    caller_variables: &mut HashMap<String, Value>,
    caller_bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    classes: &HashMap<String, BytecodeClass>,
    strict_types: bool,
) -> Result<(HashMap<String, Value>, HashMap<String, VariableBinding>), String> {
    let visible_argument_count = visible_argument_count(function, arguments.len(), argument_names);
    let (arguments, reference_names, variadic_is_packed) = normalize_named_arguments(
        arguments,
        reference_names,
        argument_names,
        function,
        call_name,
        caller_variables,
        caller_bindings,
        global_variables,
        global_scope,
        frame_id,
    )?;
    let fixed_parameter_count = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic)
        .unwrap_or(function.parameters.len());
    let extra_arguments = if function.parameter_variadic.iter().any(|variadic| *variadic) {
        Vec::new()
    } else {
        arguments
            .iter()
            .skip(fixed_parameter_count)
            .cloned()
            .collect::<Vec<_>>()
    };
    let mut arguments = if variadic_is_packed {
        arguments
    } else {
        complete_arguments(arguments, function, call_name)?
    }
    .into_iter()
    .map(|value| value.dereference())
    .collect::<Vec<_>>();
    if strict_types {
        validate_strict_arguments(&mut arguments, function, classes)?;
    } else {
        coerce_numeric_arguments(&mut arguments, &reference_names, function);
        validate_arguments(&arguments, function, classes)?;
    }
    let mut locals = function
        .parameters
        .iter()
        .cloned()
        .zip(arguments.into_iter().take(function.parameters.len()))
        .collect::<HashMap<_, _>>();
    super::initialize_local_order(&mut locals, function.parameters.iter().cloned());
    locals.insert(
        "\0func_argument_count".into(),
        Value::Number(visible_argument_count as f64),
    );
    locals.insert(
        "\0func_fixed_parameter_count".into(),
        Value::Number(
            function
                .parameter_variadic
                .iter()
                .position(|variadic| *variadic)
                .unwrap_or(function.parameters.len()) as f64,
        ),
    );
    locals.insert(
        "\0func_parameter_names".into(),
        Value::from_array_entries(
            function
                .parameters
                .iter()
                .enumerate()
                .map(|(index, name)| (ArrayKey::Integer(index as i64), Value::String(name.clone())))
                .collect(),
        ),
    );
    locals.insert(
        "\0func_extra_arguments".into(),
        Value::from_array_entries(
            extra_arguments
                .into_iter()
                .enumerate()
                .map(|(index, value)| (ArrayKey::Integer(index as i64), value.dereference()))
                .collect(),
        ),
    );
    locals.insert(
        "\0func_variadic_parameter".into(),
        function
            .parameter_variadic
            .iter()
            .position(|variadic| *variadic)
            .map(|index| Value::String(function.parameters[index].clone()))
            .unwrap_or(Value::Null),
    );
    let mut parameter_bindings = HashMap::new();
    let variadic_index = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic);
    for (index, is_reference) in function.parameter_by_reference.iter().enumerate() {
        if !is_reference {
            continue;
        }
        // By-reference variadics hold one live reference per packed element. The
        // parameter itself remains an ordinary local array containing those cells.
        if Some(index) == variadic_index {
            continue;
        }
        let Some(Some(source)) = reference_names.get(index) else {
            return Err(format!(
                "Argument {} to {call_name} must be a variable passed by reference",
                index + 1
            ));
        };
        if is_traversable_unpack_reference(source) {
            warn_traversable_by_reference(global_variables, index + 1, call_name);
            continue;
        }
        let parameter = function.parameters[index].clone();
        locals.remove(&parameter);
        let binding = if let Some((receiver, property)) = &source.object_property {
            if let Value::Object(object) = receiver {
                if source.array_keys.is_empty() {
                    let mut object = object.borrow_mut();
                    if !object.properties.contains_key(property) {
                        object.property_order.push(property.clone());
                        object.properties.insert(property.clone(), Value::Null);
                    }
                    VariableBinding::ObjectProperty {
                        receiver: receiver.clone(),
                        property: property.clone(),
                    }
                } else {
                    let mut object = object.borrow_mut();
                    let current = object
                        .properties
                        .get(property)
                        .cloned()
                        .unwrap_or(Value::Null);
                    let (updated, reference) =
                        current.ensure_array_reference(&source.array_keys)?;
                    if !object.properties.contains_key(property) {
                        object.property_order.push(property.clone());
                    }
                    object.properties.insert(property.clone(), updated);
                    VariableBinding::Reference(global_variables.store_reference(reference))
                }
            } else {
                return Err("Only object properties can be passed by reference".into());
            }
        } else if let Some(reference) = &source.direct_reference {
            VariableBinding::Reference(global_variables.store_reference(reference.clone()))
        } else {
            if let Some(VariableBinding::ObjectProperty { .. }) =
                caller_bindings.get(&source.variable)
            {
                if source.array_keys.is_empty() {
                    caller_bindings
                        .get(&source.variable)
                        .cloned()
                        .expect("checked property binding")
                } else {
                    return Err("Array-element references through object-property references are not supported".into());
                }
            } else {
                let (key, _) = super::reference_cell_for_path(
                    &source.variable,
                    &source.array_keys,
                    caller_variables,
                    caller_bindings,
                    global_variables,
                    global_scope,
                    frame_id,
                )?;
                VariableBinding::Reference(key)
            }
        };
        parameter_bindings.insert(parameter, binding);
    }
    Ok((locals, parameter_bindings))
}

pub(super) fn instantiate_pdo_row(
    row: &Value,
    class_name: &str,
    constructor_args: &[Value],
    props_late: bool,
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
    let normalized = class_name.trim_start_matches('\\');
    let object = if normalized.eq_ignore_ascii_case("stdClass") {
        let object = Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
            class_name: "stdClass".into(),
            properties: HashMap::new(),
            property_order: Vec::new(),
            active_property_hooks: Default::default(),
            exception: None,
        })));
        if !props_late {
            hydrate_pdo_row(&object, row)?;
        }
        object
    } else {
        let class = classes
            .get(&normalized.to_ascii_lowercase())
            .ok_or_else(|| format!("Class '{normalized}' not found"))?;
        if class.is_interface {
            return Err(format!("Cannot instantiate interface {normalized}"));
        }
        if class.is_abstract {
            return Err(format!("Cannot instantiate abstract class {normalized}"));
        }
        let mut properties = HashMap::new();
        super::objects::collect_properties(&class.name, classes, &mut properties)?;
        let object = Value::Object(Rc::new(std::cell::RefCell::new(crate::PhpObject {
            class_name: class.name.clone(),
            properties,
            property_order: {
                let mut order = Vec::new();
                super::objects::collect_property_order(&class.name, classes, &mut order)?;
                order
            },
            active_property_hooks: Default::default(),
            exception: None,
        })));
        global_variables.register_destructor_object(&object);
        hydrate_pdo_row(&object, row)?;
        if let Some(constructor) = find_method(&class.name, "__construct", classes) {
            if constructor.visibility != MemberVisibility::Public {
                return Err(format!(
                    "Call to {} {}::__construct() from scope PDO",
                    match constructor.visibility {
                        MemberVisibility::Private => "private",
                        MemberVisibility::Protected => "protected",
                        MemberVisibility::Public => "public",
                    },
                    class.name
                ));
            }
            let references = vec![None; constructor_args.len()];
            let names = vec![None; constructor_args.len()];
            let (mut locals, parameter_bindings) = prepare_reference_arguments(
                constructor_args.to_vec(),
                &references,
                constructor,
                &names,
                &format!("{}::__construct()", class.name),
                variables,
                bindings,
                global_variables,
                global_scope,
                frame_id,
                classes,
                false,
            )?;
            locals.insert("this".into(), object.clone());
            locals.insert("\0static_class".into(), Value::String(class.name.clone()));
            super::initialize_local_order(
                &mut locals,
                std::iter::once("this".to_string()).chain(constructor.parameters.iter().cloned()),
            );
            execute_user_frame(
                &constructor.instructions,
                functions,
                classes,
                static_properties,
                &mut locals,
                global_variables,
                false,
                &function_scope(constructor),
                parameter_bindings,
                output,
                steps,
            )?;
            finish_user_frame(
                locals,
                &object,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
        } else if !constructor_args.is_empty() {
            return Err(format!("Class '{}' has no constructor", class.name));
        }
        if props_late {
            hydrate_pdo_row(&object, row)?;
        }
        object
    };
    Ok(object)
}

fn hydrate_pdo_row(object: &Value, row: &Value) -> Result<(), String> {
    let Value::Object(instance) = object else {
        return Ok(());
    };
    if let Value::Array(fields) = row.dereference() {
        let mut instance = instance.borrow_mut();
        for (key, value) in fields.iter() {
            if let ArrayKey::String(name) = key {
                if !instance.properties.contains_key(name) {
                    instance.property_order.push(name.clone());
                }
                instance
                    .properties
                    .insert(name.clone(), value.dereference());
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
    let variadic_index = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic);
    for (index, argument) in arguments.iter_mut().enumerate() {
        if function
            .parameter_by_reference
            .get(index)
            .copied()
            .unwrap_or(false)
            && reference_names
                .get(index)
                .and_then(Option::as_ref)
                .is_some_and(|reference| !is_traversable_unpack_reference(reference))
        {
            continue;
        }
        let Some(type_name) = function
            .parameter_types
            .get(index)
            .and_then(Option::as_deref)
        else {
            continue;
        };
        let nullable = type_name.starts_with('?');
        let type_name = type_name.trim_start_matches('?').to_ascii_lowercase();
        if nullable && matches!(argument, Value::Null) {
            continue;
        }
        if type_name.contains('|') {
            if Some(index) == variadic_index {
                if let Value::Array(values) = argument {
                    let mut converted = values.clone();
                    for (extra_index, (_, value)) in converted.iter_mut().enumerate() {
                        let reference_index = index + extra_index;
                        if function
                            .parameter_by_reference
                            .get(index)
                            .copied()
                            .unwrap_or(false)
                            && reference_names
                                .get(reference_index)
                                .and_then(Option::as_ref)
                                .is_some_and(|reference| {
                                    !is_traversable_unpack_reference(reference)
                                })
                        {
                            continue;
                        }
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
            let Value::Array(values) = argument else {
                continue;
            };
            let mut converted = values.clone();
            for (extra_index, (_, value)) in converted.iter_mut().enumerate() {
                let reference_index = index + extra_index;
                if function
                    .parameter_by_reference
                    .get(index)
                    .copied()
                    .unwrap_or(false)
                    && reference_names
                        .get(reference_index)
                        .and_then(Option::as_ref)
                        .is_some_and(|reference| !is_traversable_unpack_reference(reference))
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
    let variadic_index = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic);
    for (index, (argument, expected)) in arguments
        .iter_mut()
        .zip(&function.parameter_types)
        .enumerate()
    {
        let Some(expected) = expected.as_deref() else {
            continue;
        };
        let expected_normalized = expected.trim_start_matches('?').to_ascii_lowercase();
        if expected_normalized == "float" || expected_normalized == "double" {
            if Some(index) != variadic_index {
                if let Value::Integer(value) = argument.dereference() {
                    *argument = Value::Number(value as f64);
                }
            }
        }
        let parameter = function
            .parameters
            .get(index)
            .map(String::as_str)
            .unwrap_or("?");
        let label = function.declaring_class.as_deref().map_or_else(
            || function.name.clone(),
            |class| format!("{class}::{}", function.name),
        );
        if Some(index) == variadic_index {
            let Value::Array(values) = argument else {
                continue;
            };
            for (extra_index, (_, value)) in values.iter_mut().enumerate() {
                if expected_normalized == "float" || expected_normalized == "double" {
                    if let Value::Integer(number) = value.dereference() {
                        *value = Value::Number(number as f64);
                    }
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

fn strict_value_matches_type(
    value: &Value,
    type_name: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> bool {
    let value = value.dereference();
    if type_name.contains('|') {
        return type_name.split('|').any(|member| {
            strict_value_matches_type(&value, member.trim_matches(['(', ')']), classes)
        });
    }
    if type_name.contains('&') {
        return super::value_matches_type(&value, type_name, classes);
    }
    let nullable = type_name.starts_with('?');
    let expected = type_name
        .trim_start_matches('?')
        .trim()
        .to_ascii_lowercase();
    if nullable && matches!(value, Value::Null) {
        return true;
    }
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
            Value::Number(number)
                if number.is_finite()
                    && number >= i64::MIN as f64
                    && number < -(i64::MIN as f64) =>
            {
                Value::Integer(number.trunc() as i64)
            }
            Value::Bool(value) => Value::Integer(i64::from(value)),
            Value::String(text) => text
                .trim()
                .parse::<i64>()
                .map(Value::Integer)
                .or_else(|_| {
                    text.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|number| {
                            number.is_finite()
                                && *number >= i64::MIN as f64
                                && *number < -(i64::MIN as f64)
                        })
                        .map(|number| Value::Integer(number.trunc() as i64))
                        .ok_or(())
                })
                .unwrap_or(Value::String(text)),
            value => value,
        },
        "float" | "double" => match value {
            Value::Integer(value) => Value::Number(value as f64),
            Value::Number(_) => value,
            Value::Bool(value) => Value::Number(f64::from(value)),
            Value::String(text) => text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|number| number.is_finite())
                .map(Value::Number)
                .unwrap_or(Value::String(text)),
            value => value,
        },
        _ => value,
    }
}

fn coerce_numeric_union(value: Value, type_name: &str) -> Value {
    let has = |name: &str| {
        type_name.split('|').any(|member| {
            member == name
                || (name == "int" && member == "integer")
                || (name == "float" && member == "double")
        })
    };
    match value {
        Value::Integer(number) if has("int") => Value::Integer(number),
        Value::Integer(number) if has("float") => Value::Number(number as f64),
        Value::Number(number) if has("float") => Value::Number(number),
        Value::Number(number) if has("int") && number.is_finite() && number.fract() == 0.0 => {
            Value::Integer(number as i64)
        }
        Value::String(text) if has("int") => text
            .trim()
            .parse::<i64>()
            .map(Value::Integer)
            .unwrap_or_else(|_| {
                if has("float") {
                    text.trim()
                        .parse::<f64>()
                        .map(Value::Number)
                        .unwrap_or(Value::String(text))
                } else {
                    Value::String(text)
                }
            }),
        Value::String(text) if has("float") => text
            .trim()
            .parse::<f64>()
            .map(Value::Number)
            .unwrap_or(Value::String(text)),
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
    let magic_arguments = vec![
        Value::String(missing_method.to_string()),
        Value::Array(packed),
    ];
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
    locals.insert(
        "\0static_class".to_string(),
        Value::String(static_class.clone()),
    );
    for (parameter, argument) in function.parameters.iter().cloned().zip(magic_arguments) {
        locals.insert(parameter, argument);
    }
    let mut local_order = if locals.contains_key("this") {
        vec!["this".to_string()]
    } else {
        Vec::new()
    };
    local_order.extend(function.parameters.iter().cloned());
    super::initialize_local_order(&mut locals, local_order);
    let result = execute_user_frame(
        &function.instructions,
        functions,
        classes,
        static_properties,
        &mut locals,
        global_variables,
        false,
        &function_scope(function),
        HashMap::new(),
        output,
        steps,
    )?
    .unwrap_or(Value::Null);
    let result = clean_frame_after_error(
        super::validate_return_type_in_scope(result, function, classes, Some(&static_class)),
        &mut locals,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
    finish_user_frame(
        locals,
        &result,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
    Ok(result)
}

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
        Value::String(name) => functions
            .get(&name.to_ascii_lowercase())
            .cloned()
            .or_else(|| {
                let (class_name, method) = name.split_once("::")?;
                find_method(
                    class_name.trim_start_matches('\\'),
                    &method.to_ascii_lowercase(),
                    classes,
                )
                .cloned()
            }),
        Value::Array(array) => {
            let entries = array.iter().collect::<Vec<_>>();
            if entries.len() != 2 {
                None
            } else {
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
    function
        .map(|function| function.parameter_by_reference)
        .unwrap_or_default()
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
            Value::String(name) if name.contains("::") => {
                let (class_name, method) = name.split_once("::").expect("contains separator");
                let target = Value::array(vec![
                    (
                        ArrayKey::Integer(0),
                        Value::String(class_name.trim_start_matches('\\').to_string()),
                    ),
                    (ArrayKey::Integer(1), Value::String(method.to_string())),
                ]);
                invoke_array_callable(
                    &target,
                    arguments,
                    &[],
                    &[],
                    caller_variables,
                    caller_bindings,
                    caller_global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    false,
                )
            }
            Value::String(name) => invoke_callable(
                name,
                arguments,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            ),
            Value::Closure(closure) => invoke_closure(
                closure,
                arguments,
                &[],
                &[],
                caller_variables,
                caller_bindings,
                caller_global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            ),
            Value::Array(_) => invoke_array_callable(
                callback,
                arguments,
                &[],
                &[],
                caller_variables,
                caller_bindings,
                caller_global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            ),
            Value::Object(_) => invoke_object_callable(
                callback,
                arguments,
                &[],
                &[],
                caller_variables,
                caller_bindings,
                caller_global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            ),
            _ => Err("Value is not callable".into()),
        };
    }
    // PHP warns for these callback APIs and binds by-reference parameters to
    // temporary callback arguments, leaving the source array unchanged.
    let values = arguments
        .into_iter()
        .map(|value| value.dereference())
        .collect::<Vec<_>>();
    let mut local_variables = HashMap::new();
    let mut local_bindings = HashMap::new();
    let reference_arguments = values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let name = format!("\0callback_argument:{frame_id}:{index}");
            local_variables.insert(name.clone(), value.clone());
            references
                .get(index)
                .copied()
                .unwrap_or(false)
                .then_some(ReferenceArgument {
                    variable: name,
                    array_keys: Vec::new(),
                    object_property: None,
                    direct_reference: None,
                })
        })
        .collect::<Vec<_>>();
    let names = vec![None; values.len()];
    match callback {
        Value::String(name) if functions.contains_key(&name.to_ascii_lowercase()) => {
            invoke_with_references(
                name,
                values,
                &reference_arguments,
                &names,
                false,
                &mut local_variables,
                &mut local_bindings,
                global_variables,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )
        }
        Value::String(name) if name.contains("::") => {
            let (class_name, method) = name.split_once("::").expect("contains separator");
            let target = Value::array(vec![
                (
                    ArrayKey::Integer(0),
                    Value::String(class_name.trim_start_matches('\\').to_string()),
                ),
                (ArrayKey::Integer(1), Value::String(method.to_string())),
            ]);
            invoke_array_callable(
                &target,
                values,
                &reference_arguments,
                &names,
                &mut local_variables,
                &mut local_bindings,
                false,
                frame_id,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )
        }
        Value::Closure(closure) => invoke_closure(
            closure,
            values,
            &reference_arguments,
            &names,
            &mut local_variables,
            &mut local_bindings,
            false,
            frame_id,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
            false,
        ),
        Value::Array(_) => invoke_array_callable(
            callback,
            values,
            &reference_arguments,
            &names,
            &mut local_variables,
            &mut local_bindings,
            false,
            frame_id,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
            false,
        ),
        Value::Object(_) => invoke_object_callable(
            callback,
            values,
            &reference_arguments,
            &names,
            &mut local_variables,
            &mut local_bindings,
            false,
            frame_id,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
            false,
        ),
        Value::String(name) => invoke_callable(
            name,
            values,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        ),
        _ => Err("Value is not callable".into()),
    }
}

fn strtok_with_state(
    arguments: &[Value],
    global_variables: &mut GlobalVariables,
) -> Result<Value, String> {
    if arguments.is_empty() || arguments.len() > 2 {
        return Err(format!(
            "strtok() expects 1 or 2 arguments, {} given",
            arguments.len()
        ));
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
        let offset = global_variables
            .get("\0strtok_offset")
            .map(|value| value.number() as usize)
            .unwrap_or(input.len());
        (input, offset)
    };
    let delimiters = arguments
        .last()
        .expect("strtok requires a delimiter")
        .show()
        .into_bytes();
    let bytes = input.as_bytes();
    let mut cursor = offset.min(bytes.len());
    while cursor < bytes.len() && delimiters.contains(&bytes[cursor]) {
        cursor += 1;
    }
    if cursor == bytes.len() {
        global_variables.set("\0strtok_offset".into(), Value::Integer(bytes.len() as i64));
        return Ok(Value::Bool(false));
    }
    let start = cursor;
    while cursor < bytes.len() && !delimiters.contains(&bytes[cursor]) {
        cursor += 1;
    }
    global_variables.set("\0strtok_offset".into(), Value::Integer(cursor as i64));
    Ok(Value::String(
        String::from_utf8_lossy(&bytes[start..cursor]).into_owned(),
    ))
}

fn hex2bin_with_diagnostic(
    arguments: &[Value],
    globals: &mut GlobalVariables,
) -> Result<Value, String> {
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
            message: message.into(),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
    }
    Ok(result)
}

fn chr_with_diagnostic(
    arguments: &[Value],
    globals: &mut GlobalVariables,
) -> Result<Value, String> {
    let codepoint = arguments.first().map(Value::number).unwrap_or(0.0) as i64;
    if !(0..=255).contains(&codepoint) {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Deprecated,
            message: "chr(): Providing a value not in-between 0 and 255 is deprecated, this is because a byte value must be in the [0, 255] interval. The value used will be constrained using % 256".into(),
            source_file: None, source_line: None,
            output_offset: 0,
        });
    }
    crate::string_builtins::call("chr", arguments)
}

fn json_encode_builtin(
    arguments: &[Value],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let value = arguments
        .first()
        .ok_or("json_encode() expects exactly 1 argument")?;
    let flags = arguments
        .get(1)
        .map(|value| value.number() as i64)
        .unwrap_or(0);
    let depth = arguments
        .get(2)
        .map(|value| value.number() as i64)
        .unwrap_or(512);
    let throw_on_error = flags & 4194304 != 0;
    if depth > i32::MAX as i64 || depth < i32::MIN as i64 {
        return Err(format!(
            "ValueError:json_encode(): Argument #3 ($depth) must be between 1 and {}",
            i32::MAX
        ));
    }
    if depth < 1 {
        return Err("ValueError:json_encode(): Argument #3 ($depth) must be greater than 0".into());
    }
    if flags & !(4095 | 1048576 | 2097152 | 4194304) != 0 {
        let code = 4;
        if throw_on_error && flags & 512 == 0 {
            return Err(json_exception_marker(
                global_variables,
                *steps,
                code,
                json_error_message(code),
                classes,
            )?);
        }
        global_variables.json_last_error = code;
        return Ok(Value::Bool(false));
    }
    let mut serialize = |object: &Value| -> Result<Option<Value>, String> {
        let Value::Object(instance) = object else {
            return Ok(None);
        };
        let class_name = instance.borrow().class_name.clone();
        let Some(method) = super::objects::find_method(&class_name, "jsonserialize", classes)
        else {
            return Ok(None);
        };
        if method.is_static
            || !method.parameters.is_empty()
            || method.visibility != crate::MemberVisibility::Public
        {
            return Err(format!("{class_name}::jsonSerialize() must be a public non-static method without parameters"));
        }
        let mut locals = HashMap::from([
            ("this".to_string(), object.clone()),
            (
                "\0static_class".to_string(),
                Value::String(class_name.clone()),
            ),
        ]);
        super::initialize_local_order(&mut locals, ["this".to_string()]);
        let result = execute_user_frame(
            &method.instructions,
            functions,
            classes,
            static_properties,
            &mut locals,
            global_variables,
            false,
            &super::function_scope(method),
            HashMap::new(),
            output,
            steps,
        )?
        .unwrap_or(Value::Null);
        let result = clean_frame_after_error(
            super::validate_return_type_in_scope(result, method, classes, Some(&class_name)),
            &mut locals,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )?;
        finish_user_frame(
            locals,
            &result,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )?;
        Ok(Some(result))
    };
    match crate::json_builtins::encode_with_serializer(
        value,
        depth as usize,
        crate::json_builtins::EncodeOptions {
            pretty: flags & 128 != 0,
            unescaped_slashes: flags & 64 != 0,
            unescaped_unicode: flags & 256 != 0,
            unescaped_line_terminators: flags & 2048 != 0,
            force_object: flags & 16 != 0,
            hex_flags: flags & 15,
            preserve_zero_fraction: flags & 1024 != 0,
            numeric_check: flags & 32 != 0,
            partial_output: flags & 512 != 0,
        },
        classes,
        &mut serialize,
    ) {
        Ok((json, error_code)) => {
            if !throw_on_error || flags & 512 != 0 || error_code != 0 {
                global_variables.json_last_error = error_code;
            }
            Ok(Value::String(json))
        }
        Err(error) => {
            let code = json_error_code(&error);
            if throw_on_error && flags & 512 == 0 {
                return Err(json_exception_marker(
                    global_variables,
                    *steps,
                    code,
                    json_error_message(code),
                    classes,
                )?);
            }
            global_variables.json_last_error = code;
            Ok(Value::Bool(false))
        }
    }
}

fn json_decode_builtin(
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    global_variables: &mut GlobalVariables,
    steps: &mut u64,
) -> Result<Value, String> {
    let source = arguments
        .first()
        .ok_or("json_decode() expects exactly 1 argument")?
        .show();
    let depth = arguments
        .get(2)
        .map(|value| value.number() as i64)
        .unwrap_or(512);
    let flags = arguments
        .get(3)
        .map(|value| value.number() as i64)
        .unwrap_or(0);
    let associative = arguments.get(1).map_or(flags & 1 != 0, |value| {
        matches!(value.dereference(), Value::Null) && flags & 1 != 0 || value.truthy()
    });
    let throw_on_error = flags & 4194304 != 0;
    if depth < 1 || depth > i32::MAX as i64 {
        return Err(format!(
            "ValueError:json_decode(): Argument #3 ($depth) must be between 1 and {}",
            i32::MAX
        ));
    }
    if flags & !(3 | 1048576 | 2097152 | 4194304) != 0 {
        let code = 4;
        if throw_on_error {
            return Err(json_exception_marker(
                global_variables,
                *steps,
                code,
                json_error_message(code),
                classes,
            )?);
        }
        global_variables.json_last_error = code;
        return Ok(Value::Null);
    }
    match crate::json_builtins::decode(&source, associative, depth as usize, flags & 2 != 0) {
        Ok(value) => {
            if !throw_on_error {
                global_variables.json_last_error = 0;
            }
            Ok(value)
        }
        Err(error) => {
            let code = json_error_code(&error);
            if throw_on_error {
                return Err(json_exception_marker(
                    global_variables,
                    *steps,
                    code,
                    json_error_message(code),
                    classes,
                )?);
            }
            global_variables.json_last_error = code;
            Ok(Value::Null)
        }
    }
}

fn json_validate_builtin(
    arguments: &[Value],
    global_variables: &mut GlobalVariables,
) -> Result<Value, String> {
    let value = arguments
        .first()
        .ok_or("json_validate() expects exactly 1 argument")?;
    let source = match value.dereference() {
        Value::String(source) => source,
        Value::Integer(number) => number.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Bool(value) => {
            if value {
                "1".into()
            } else {
                String::new()
            }
        }
        Value::Null => String::new(),
        other => {
            return Err(format!(
                "TypeError:json_validate(): Argument #1 ($json) must be of type string, {} given",
                other.type_name().to_ascii_lowercase()
            ))
        }
    };
    let depth = arguments
        .get(1)
        .map(|value| value.number() as i64)
        .unwrap_or(512);
    let flags = arguments
        .get(2)
        .map(|value| value.number() as i64)
        .unwrap_or(0);
    if depth < 1 || depth > i32::MAX as i64 {
        return Err(format!(
            "ValueError:json_validate(): Argument #2 ($depth) must be between 1 and {}",
            i32::MAX
        ));
    }
    if flags & !1048576 != 0 {
        return Err("ValueError:json_validate(): Argument #3 ($flags) must be a valid flag (allowed flags: JSON_INVALID_UTF8_IGNORE)".into());
    }
    match crate::json_builtins::decode(&source, true, depth as usize, false) {
        Ok(_) => {
            global_variables.json_last_error = 0;
            Ok(Value::Bool(true))
        }
        Err(error) => {
            global_variables.json_last_error = json_error_code(&error);
            Ok(Value::Bool(false))
        }
    }
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
        if function
            .parameter_by_reference
            .iter()
            .any(|by_reference| *by_reference)
        {
            return Err(format!(
                "{name}() requires a variable for its by-reference parameter"
            ));
        }
        let arguments = complete_arguments(arguments, function, &format!("{name}()"))?;
        validate_arguments(&arguments, function, classes)?;
        let mut local_variables = function
            .parameters
            .iter()
            .cloned()
            .zip(arguments.into_iter().take(function.parameters.len()))
            .collect::<HashMap<_, _>>();
        super::initialize_local_order(&mut local_variables, function.parameters.iter().cloned());
        if super::is_generator_function(function) {
            return super::new_generator(
                function.clone(),
                local_variables,
                HashMap::new(),
                *steps,
                classes,
            );
        }
        let result = execute_user_frame(
            &function.instructions,
            functions,
            classes,
            static_properties,
            &mut local_variables,
            global_variables,
            false,
            &function_scope(function),
            HashMap::new(),
            output,
            steps,
        )?
        .unwrap_or(Value::Null);
        let result = clean_frame_after_error(
            validate_return_type(result, function, classes),
            &mut local_variables,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )?;
        finish_user_frame(
            local_variables,
            &result,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )?;
        return Ok(result);
    }
    if normalized_name == "md5_file" {
        return md5_file_builtin(&arguments, global_variables);
    }
    if normalized_name == "sha1_file" {
        return sha1_file_builtin(&arguments, global_variables);
    }
    if normalized_name == "hash_hmac_file" {
        return hash_hmac_file_builtin(&arguments, global_variables);
    }
    if normalized_name == "hash_file" {
        return hash_file_builtin(&arguments, global_variables);
    }
    if normalized_name == "count" {
        let mut callback_variables = HashMap::new();
        let mut callback_bindings = HashMap::new();
        let mut invoke = |callable: &Value, callback_arguments: Vec<Value>| {
            invoke_array_callable(
                callable,
                callback_arguments,
                &[],
                &[],
                &mut callback_variables,
                &mut callback_bindings,
                true,
                0,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                false,
            )
        };
        let mut unused_walk = |_: &Value, _: Value, _: Value, _: Option<Value>| {
            Err("Unexpected array walk callback".into())
        };
        let mut stringify = |value: &Value| Ok(value.show());
        let mut diagnostics = Vec::new();
        let (result, _) = crate::array_builtins::call(
            "count",
            arguments,
            classes,
            &mut invoke,
            &mut unused_walk,
            &mut stringify,
            &mut diagnostics,
        )?;
        for diagnostic in diagnostics {
            global_variables.report_diagnostic(diagnostic);
        }
        return Ok(result);
    }
    if normalized_name == "copy" {
        return copy_value(&arguments, global_variables, output);
    }
    if normalized_name == "rename" {
        return rename_value(&arguments, global_variables, output);
    }
    if normalized_name == "mkdir" {
        return mkdir_value(&arguments, global_variables, output);
    }
    if normalized_name == "rmdir" {
        return rmdir_value(&arguments, global_variables, output);
    }
    if normalized_name == "touch" {
        return touch_value(&arguments, global_variables, output);
    }
    if normalized_name == "chmod" {
        return chmod_value(&arguments, global_variables, output);
    }
    if matches!(
        normalized_name.as_str(),
        "disk_free_space" | "diskfreespace" | "disk_total_space"
    ) {
        return disk_space_value(name, &arguments, global_variables, output);
    }
    if normalized_name == "scandir" {
        return scandir_builtin(&arguments, global_variables);
    }
    if normalized_name == "opendir" {
        return opendir_builtin(&arguments, global_variables);
    }
    if normalized_name == "readdir" {
        return readdir_builtin(&arguments, global_variables);
    }
    if normalized_name == "closedir" {
        return closedir_builtin(&arguments, global_variables);
    }
    if normalized_name == "rewinddir" {
        return rewinddir_builtin(&arguments, global_variables);
    }
    if normalized_name == "filemtime" || normalized_name == "fileatime" {
        if arguments.len() != 1 {
            return Err(format!(
                "ArgumentCountError:{}() expects exactly 1 argument, {} given",
                name,
                arguments.len()
            ));
        }
        let path = arguments
            .first()
            .ok_or_else(|| format!("{}() expects exactly 1 argument", name))?
            .show();
        let resolved = filesystem_path(global_variables, std::path::Path::new(&path), false);
        return Ok(file_timestamp_value(
            name,
            &path,
            resolved.as_deref(),
            global_variables,
        ));
    }
    if matches!(
        normalized_name.as_str(),
        "filectime" | "fileinode" | "fileowner" | "filegroup" | "fileperms"
    ) {
        if arguments.len() != 1 {
            return Err(format!(
                "ArgumentCountError:{}() expects exactly 1 argument, {} given",
                name,
                arguments.len()
            ));
        }
        let filename = arguments[0].show();
        let field = match normalized_name.as_str() {
            "filectime" => "ctime",
            "fileinode" => "ino",
            "fileowner" => "uid",
            "filegroup" => "gid",
            _ => "mode",
        };
        return Ok(file_stat_field_value(
            name,
            &filename,
            field,
            global_variables,
        ));
    }
    if normalized_name == "filetype" {
        if arguments.len() != 1 {
            return Err(format!(
                "ArgumentCountError:{}() expects exactly 1 argument, {} given",
                name,
                arguments.len()
            ));
        }
        return Ok(filetype_value(name, &arguments[0].show(), global_variables));
    }
    if normalized_name == "stat" || normalized_name == "lstat" {
        if arguments.len() != 1 {
            return Err(format!(
                "ArgumentCountError:{}() expects exactly 1 argument, {} given",
                name,
                arguments.len()
            ));
        }
        let filename = arguments[0].show();
        return Ok(file_stat_value(name, &filename, global_variables));
    }
    if name.eq_ignore_ascii_case("strval") {
        let value = arguments
            .first()
            .ok_or("strval() expects exactly 1 argument")?
            .dereference();
        return super::objects::stringify(
            &value,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )
        .map(Value::String);
    }
    if name.eq_ignore_ascii_case("is_countable") {
        return type_builtin("is_countable", &arguments, classes);
    }
    if name.eq_ignore_ascii_case("is_iterable") {
        return type_builtin("is_iterable", &arguments, classes);
    }
    if name.eq_ignore_ascii_case("gettimeofday") {
        if arguments.len() > 1 {
            return Err(format!(
                "gettimeofday() expects at most 1 argument, {} given",
                arguments.len()
            ));
        }
        return gettimeofday_value(arguments.first().is_some_and(Value::truthy));
    }
    if name.eq_ignore_ascii_case("getenv") {
        return getenv_value(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("putenv") {
        return putenv_value(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("json_encode") {
        return json_encode_builtin(
            &arguments,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        );
    }
    if name.eq_ignore_ascii_case("json_decode") {
        return json_decode_builtin(&arguments, classes, global_variables, steps);
    }
    if name.eq_ignore_ascii_case("json_validate") {
        return json_validate_builtin(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("json_last_error") {
        if !arguments.is_empty() {
            return Err("json_last_error() expects exactly 0 arguments".into());
        }
        return Ok(Value::Integer(global_variables.json_last_error));
    }
    if name.eq_ignore_ascii_case("json_last_error_msg") {
        if !arguments.is_empty() {
            return Err("json_last_error_msg() expects exactly 0 arguments".into());
        }
        return Ok(Value::String(
            json_error_message(global_variables.json_last_error).into(),
        ));
    }
    if name.eq_ignore_ascii_case("strtok") {
        let mut arguments = arguments;
        coerce_stringable_builtin_args(
            "strtok",
            &mut arguments,
            crate::string_builtins::string_argument_positions("strtok"),
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )?;
        return strtok_with_state(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("hex2bin") {
        return hex2bin_with_diagnostic(&arguments, global_variables);
    }
    if name.eq_ignore_ascii_case("chr") {
        return chr_with_diagnostic(&arguments, global_variables);
    }
    let mut diagnostics = Vec::new();
    let mut stringify = |value: &Value| {
        super::objects::stringify(
            value,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
        )
    };
    let result = crate::array_builtins::call_callback_builtin(
        name,
        arguments,
        classes,
        &mut diagnostics,
        &mut stringify,
    )?;
    drop(stringify);
    for diagnostic in diagnostics {
        global_variables.report_diagnostic(diagnostic);
    }
    Ok(result)
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
    invoke_array_callable_scoped(
        callable,
        arguments,
        reference_names,
        argument_names,
        caller_variables,
        caller_bindings,
        caller_global_scope,
        caller_frame_id,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
        strict_types,
        None,
    )
}

/// Invoke a callable with an optional class scope, used by ReflectionMethod's visibility bypass.
fn invoke_array_callable_scoped(
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
    access_scope: Option<&str>,
) -> Result<Value, String> {
    let Value::Array(items) = callable else {
        return Err("Value is not callable".into());
    };
    if items.len() != 2
        || items
            .get(0)
            .map_or(true, |(key, _)| key != &ArrayKey::Integer(0))
        || items
            .get(1)
            .map_or(true, |(key, _)| key != &ArrayKey::Integer(1))
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
            ensure_access(
                function.visibility,
                function.declaring_class.as_deref(),
                access_scope,
                classes,
                &method,
            )?;
            if function.is_static {
                return Err(format!(
                    "Non-static callback target {class_name}::{method}() is static"
                ));
            }
            (class_name, function, Some(Value::Object(object)))
        }
        Value::String(class_name) => {
            let class_name = class_name.to_ascii_lowercase();
            let function = find_method(&class_name, &method, classes)
                .ok_or_else(|| format!("Call to undefined method {class_name}::{method}()"))?;
            ensure_access(
                function.visibility,
                function.declaring_class.as_deref(),
                access_scope,
                classes,
                &method,
            )?;
            if !function.is_static {
                return Err(format!(
                    "Non-static method {class_name}::{method}() cannot be called statically"
                ));
            }
            (class_name, function, None)
        }
        _ => return Err("Array callback must start with an object or class name".into()),
    };
    let (mut locals, parameter_bindings) = prepare_reference_arguments(
        arguments,
        reference_names,
        function,
        &argument_names,
        &format!("{class_name}::{method}()"),
        caller_variables,
        caller_bindings,
        global_variables,
        caller_global_scope,
        caller_frame_id,
        classes,
        strict_types,
    )?;
    if let Some(receiver) = receiver_object {
        locals.insert("this".into(), receiver);
        super::initialize_local_order(
            &mut locals,
            std::iter::once("this".to_string()).chain(function.parameters.iter().cloned()),
        );
    }
    locals.insert("\0static_class".into(), Value::String(class_name.clone()));
    if super::is_generator_function(function) {
        return super::new_generator(
            function.clone(),
            locals,
            parameter_bindings,
            *steps,
            classes,
        );
    }
    let result = execute_user_frame(
        &function.instructions,
        functions,
        classes,
        static_properties,
        &mut locals,
        global_variables,
        false,
        &function_scope(function),
        parameter_bindings,
        output,
        steps,
    )?
    .unwrap_or(Value::Null);
    let result = clean_frame_after_error(
        super::validate_return_type_in_scope(result, function, classes, Some(&class_name)),
        &mut locals,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
    finish_user_frame(
        locals,
        &result,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
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
    if !matches!(callable.dereference(), Value::Object(_)) {
        return Err("Value is not callable".into());
    }
    let target = Value::from_array_entries(vec![
        (ArrayKey::Integer(0), callable.dereference()),
        (ArrayKey::Integer(1), Value::String("__invoke".into())),
    ]);
    invoke_array_callable(
        &target,
        arguments,
        reference_names,
        argument_names,
        caller_variables,
        caller_bindings,
        caller_global_scope,
        caller_frame_id,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
        strict_types,
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
    let function = functions
        .get(&normalized_name)
        .ok_or_else(|| format!("Call to undefined function {name}()"))?;
    let (mut locals, parameter_bindings) = prepare_reference_arguments(
        arguments,
        reference_names,
        function,
        argument_names,
        &format!("{name}()"),
        caller_variables,
        caller_bindings,
        global_variables,
        caller_global_scope,
        caller_frame_id,
        classes,
        strict_types,
    )?;
    if super::is_generator_function(function) {
        return super::new_generator(
            function.clone(),
            locals,
            parameter_bindings,
            *steps,
            classes,
        );
    }
    let result = execute_user_frame(
        &function.instructions,
        functions,
        classes,
        static_properties,
        &mut locals,
        global_variables,
        false,
        &function_scope(function),
        parameter_bindings,
        output,
        steps,
    )?
    .unwrap_or(Value::Null);
    let result = clean_frame_after_error(
        validate_return_type(result, function, classes),
        &mut locals,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
    finish_user_frame(
        locals,
        &result,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
    Ok(result)
}

pub(super) fn finish_user_frame(
    locals: HashMap<String, Value>,
    result: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let frame_values = Value::from_array_entries(
        locals
            .into_iter()
            .map(|(name, value)| (ArrayKey::String(name), value))
            .collect(),
    );
    super::objects::run_destructors_removed_from_value(
        frame_values,
        result,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )
}

/// Execute a user frame and destroy its locals while unwinding an exception.
/// Successful frames remain owned by their caller so returned values and captures survive.
pub(super) fn execute_user_frame(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    locals: &mut HashMap<String, Value>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    access_scope: &str,
    bindings: HashMap<String, VariableBinding>,
    output: &mut String,
    steps: &mut u64,
) -> Result<Option<Value>, String> {
    match execute_code(
        code,
        functions,
        classes,
        static_properties,
        locals,
        global_variables,
        global_scope,
        access_scope,
        bindings,
        output,
        steps,
    ) {
        Ok(result) => Ok(result),
        Err(error) => {
            let frame = std::mem::take(locals);
            finish_user_frame(
                frame,
                &Value::Null,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            Err(error)
        }
    }
}

fn clean_frame_after_error<T>(
    result: Result<T, String>,
    locals: &mut HashMap<String, Value>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            let frame = std::mem::take(locals);
            finish_user_frame(
                frame,
                &Value::Null,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            Err(error)
        }
    }
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
    let (
        function,
        captures,
        capture_order,
        reference_captures,
        callable_target,
        callable_access_scope,
    ) = {
        let closure = closure.borrow();
        (
            closure.function.clone(),
            closure.captures.clone(),
            closure.capture_order.clone(),
            closure.reference_captures.clone(),
            closure.callable_target.clone(),
            closure.callable_access_scope.clone(),
        )
    };
    if let Some(target) = callable_target {
        return match target {
            Value::Array(_) => {
                if let Some(scope) = callable_access_scope.as_deref() {
                    invoke_array_callable_scoped(
                        &target,
                        arguments,
                        reference_names,
                        argument_names,
                        caller_variables,
                        caller_bindings,
                        caller_global_scope,
                        caller_frame_id,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                        strict_types,
                        Some(scope),
                    )
                } else {
                    invoke_array_callable(
                        &target,
                        arguments,
                        reference_names,
                        argument_names,
                        caller_variables,
                        caller_bindings,
                        caller_global_scope,
                        caller_frame_id,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                        strict_types,
                    )
                }
            }
            Value::String(name) => {
                let packed_arguments = pack_callable_arguments(
                    arguments,
                    reference_names,
                    argument_names,
                    caller_variables,
                    caller_bindings,
                    global_variables,
                    caller_global_scope,
                    caller_frame_id,
                )?;
                let (instruction, mut callback_stack) =
                    callback_call(Value::String(name), packed_arguments, strict_types)?;
                execute_instruction(
                    &instruction,
                    &mut callback_stack,
                    caller_variables,
                    caller_bindings,
                    global_variables,
                    caller_global_scope,
                    caller_frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )?;
                callback_stack
                    .pop()
                    .ok_or_else(|| "Invalid first-class callable result stack".into())
            }
            _ => Err("Invalid first-class callable target".into()),
        };
    }
    let function = &function;
    let (mut parameter_locals, parameter_bindings) = prepare_reference_arguments(
        arguments,
        reference_names,
        function,
        argument_names,
        "Closure()",
        caller_variables,
        caller_bindings,
        global_variables,
        caller_global_scope,
        caller_frame_id,
        classes,
        strict_types,
    )?;
    let mut locals = captures.clone();
    locals.extend(parameter_locals.drain());
    super::initialize_local_order(
        &mut locals,
        capture_order
            .into_iter()
            .chain(function.parameters.iter().cloned()),
    );
    let mut frame_bindings = reference_captures
        .into_iter()
        .map(|(name, key)| (name, VariableBinding::Reference(key)))
        .collect::<HashMap<_, _>>();
    frame_bindings.extend(parameter_bindings);
    if super::is_generator_function(function) {
        return super::new_generator(function.clone(), locals, frame_bindings, *steps, classes);
    }
    let closure_scope = format!("<closure:{:p}>", Rc::as_ptr(closure));
    let result = execute_user_frame(
        &function.instructions,
        functions,
        classes,
        static_properties,
        &mut locals,
        global_variables,
        false,
        &closure_scope,
        frame_bindings,
        output,
        steps,
    )?
    .unwrap_or(Value::Null);
    let result = clean_frame_after_error(
        validate_return_type(result, function, classes),
        &mut locals,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
    let mut closure = closure.borrow_mut();
    for name in closure.captures.keys().cloned().collect::<Vec<_>>() {
        if let Some(value) = locals.get(&name) {
            closure.captures.insert(name, value.clone());
        }
    }
    drop(closure);
    finish_user_frame(
        locals,
        &result,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
    )?;
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
                reference,
                variables,
                bindings,
                globals,
                global_scope,
                frame_id,
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
            assert!(
                super::runtime_function_exists(name, &functions),
                "function_exists({name:?}) returned false"
            );
        }
    }
}
