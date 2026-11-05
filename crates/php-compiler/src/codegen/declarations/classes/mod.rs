//! Class metadata resolution, OOP validation, and method bytecode generation.

use std::collections::{HashMap, HashSet};

use php_runtime::{bytecode::Op, BytecodeAttribute, BytecodeClass, BytecodeConstant, BytecodeFunction, BytecodeProperty, MemberVisibility, Value};
use php_syntax::{FunctionDecl, Program, Visibility};

use super::CodeGenerator;

mod compatibility;
mod metadata;

use compatibility::*;
use metadata::*;

fn compile_constant_attributes(attributes: &[php_syntax::AttributeDecl], class: Option<&php_syntax::ClassDecl>, program: &Program) -> Result<Vec<BytecodeAttribute>, String> {
    attributes.iter().map(|attribute| {
        let arguments = attribute.arguments.iter().map(|argument| match argument {
            php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, class, Some(program))?)),
            value => Ok((None, constant_value_in_class(value, class, Some(program))?)),
        }).collect::<Result<Vec<_>, String>>()?;
        Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
    }).collect()
}

fn split_type_expression(type_name: &str, separator: char) -> Vec<&str> {
    let mut depth = 0usize;
    let mut start = 0usize;
    let mut members = Vec::new();
    for (index, character) in type_name.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            value if value == separator && depth == 0 => {
                members.push(type_name[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if !members.is_empty() { members.push(type_name[start..].trim()); }
    members
}

fn constant_value_type_name(value: &Value) -> String {
    match value {
        Value::Integer(_) => "int".into(), Value::Number(_) => "float".into(), Value::String(_) => "string".into(),
        Value::Bool(_) => "bool".into(), Value::Array(_) => "array".into(), Value::Object(object) => object.borrow().class_name.clone(),
        Value::Closure(_) => "Closure".into(), Value::Generator(_) => "Generator".into(), Value::Stream(_) => "resource".into(),
        Value::Reference(value) => constant_value_type_name(&value.borrow()), Value::Null => "null".into(),
    }
}

fn unsupported_constant_type(type_name: &str) -> Option<&'static str> {
    let type_name = type_name.trim();
    if let Some(inner) = type_name.strip_prefix('?') {
        return unsupported_constant_type(inner);
    }
    let type_name = type_name.strip_prefix('(').and_then(|inner| inner.strip_suffix(')')).unwrap_or(type_name).trim();
    for separator in ['|', '&'] {
        let members = split_type_expression(type_name, separator);
        if !members.is_empty() {
            return members.into_iter().find_map(unsupported_constant_type);
        }
    }
    match type_name.to_ascii_lowercase().as_str() {
        "callable" => Some("callable"),
        "void" => Some("void"),
        "never" => Some("never"),
        _ => None,
    }
}

fn constant_class_is_a(actual: &str, expected: &str, classes: &HashMap<String, BytecodeClass>, visited: &mut std::collections::HashSet<String>) -> bool {
    let actual = actual.to_ascii_lowercase();
    if actual.eq_ignore_ascii_case(expected) { return true; }
    if !visited.insert(actual.clone()) { return false; }
    let Some(class) = classes.get(&actual) else { return false; };
    class.parent.as_deref().is_some_and(|parent| constant_class_is_a(parent, expected, classes, visited))
        || class.interfaces.iter().any(|interface| constant_class_is_a(interface, expected, classes, visited))
}

fn constant_value_matches_type(value: &Value, type_name: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    let type_name = type_name.trim();
    if let Some(inner) = type_name.strip_prefix('?') {
        if matches!(value, Value::Null) { return true; }
        return constant_value_matches_type(value, inner, classes);
    }
    let type_name = type_name.strip_prefix('(').and_then(|inner| inner.strip_suffix(')')).unwrap_or(type_name).trim();
    let union = split_type_expression(type_name, '|');
    if !union.is_empty() { return union.iter().any(|member| constant_value_matches_type(value, member, classes)); }
    let intersection = split_type_expression(type_name, '&');
    if !intersection.is_empty() { return intersection.iter().all(|member| constant_value_matches_type(value, member, classes)); }
    match type_name.to_ascii_lowercase().as_str() {
        "mixed" => true,
        "null" => matches!(value, Value::Null),
        "true" => matches!(value, Value::Bool(true)),
        "false" => matches!(value, Value::Bool(false)),
        "int" | "integer" => matches!(value, Value::Integer(_)),
        "float" | "double" => matches!(value, Value::Integer(_) | Value::Number(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "array" => matches!(value, Value::Array(_)),
        "object" => matches!(value, Value::Object(_)),
        "iterable" => matches!(value, Value::Array(_) | Value::Generator(_)) || matches!(value, Value::Object(object) if constant_class_is_a(&object.borrow().class_name, "Traversable", classes, &mut std::collections::HashSet::new())),
        "void" | "never" => false,
        expected => matches!(value, Value::Object(object) if constant_class_is_a(&object.borrow().class_name, expected, classes, &mut std::collections::HashSet::new())),
    }
}

fn enum_backing_values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::String(left), Value::String(right)) => left == right,
        _ => false,
    }
}

fn property_type_in_class(type_name: Option<&str>, class: &php_syntax::ClassDecl) -> Option<String> {
    let type_name = type_name?;
    let declaring_class = class.name.as_str();
    let parent_class = class.parent.as_deref().unwrap_or("parent");
    let mut resolved = String::with_capacity(type_name.len());
    let mut token = String::new();
    let flush_token = |token: &mut String, output: &mut String| {
        if token.eq_ignore_ascii_case("self") {
            output.push_str(declaring_class);
        } else if token.eq_ignore_ascii_case("parent") {
            output.push_str(parent_class);
        } else {
            output.push_str(token);
        }
        token.clear();
    };
    for character in type_name.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '\\' {
            token.push(character);
        } else {
            flush_token(&mut token, &mut resolved);
            resolved.push(character);
        }
    }
    flush_token(&mut token, &mut resolved);
    Some(resolved)
}

fn compatible_trait_property(left: &php_syntax::PropertyDecl, right: &php_syntax::PropertyDecl, class: &php_syntax::ClassDecl, program: &Program) -> Result<bool, String> {
    if left.is_static != right.is_static || left.visibility != right.visibility || left.is_readonly != right.is_readonly
        || left.set_visibility != right.set_visibility
        || !option_type_names_equal(
            property_type_in_class(left.type_name.as_deref(), class).as_deref(),
            property_type_in_class(right.type_name.as_deref(), class).as_deref(),
        ) || left.get_hook.is_some() || right.get_hook.is_some() || left.set_hook.is_some() || right.set_hook.is_some() {
        return Ok(false);
    }
    let left_default = class_property_default(left, class, program)?;
    let right_default = class_property_default(right, class, program)?;
    Ok(constant_options_identical(&left_default, &right_default))
}

fn option_type_names_equal(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => canonical_type_name(left) == canonical_type_name(right),
        _ => false,
    }
}

fn canonical_type_name(type_name: &str) -> String {
    let type_name = type_name.trim();
    if let Some(inner) = type_name.strip_prefix('?') {
        let mut members = vec![canonical_type_name(inner), "null".to_owned()];
        members.sort();
        members.dedup();
        return members.join("|");
    }
    if type_name.starts_with('(') && type_name.ends_with(')') {
        let mut depth = 0usize;
        let mut encloses_expression = true;
        for (index, character) in type_name.char_indices() {
            match character {
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 && index + character.len_utf8() != type_name.len() {
                        encloses_expression = false;
                        break;
                    }
                }
                _ => {}
            }
        }
        if encloses_expression && depth == 0 {
            return canonical_type_name(&type_name[1..type_name.len() - 1]);
        }
    }
    for separator in ['|', '&'] {
        let members = split_type_expression(type_name, separator);
        if !members.is_empty() {
            let mut members: Vec<String> = members.into_iter().map(canonical_type_name).collect();
            members.sort();
            members.dedup();
            return members.join(&separator.to_string());
        }
    }
    type_name.chars().filter(|character| !character.is_whitespace()).flat_map(char::to_lowercase).collect()
}

fn constant_options_identical(left: &Option<Value>, right: &Option<Value>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => constant_values_identical(left, right),
        _ => false,
    }
}

fn constant_values_identical(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Integer(left), Value::Integer(right)) => left == right,
        (Value::Number(left), Value::Number(right)) => left.to_bits() == right.to_bits(),
        (Value::String(left), Value::String(right)) => left == right,
        (Value::Bool(left), Value::Bool(right)) => left == right,
        (Value::Null, Value::Null) => true,
        (Value::Object(left), Value::Object(right)) => {
            let left = left.borrow();
            let right = right.borrow();
            left.class_name.eq_ignore_ascii_case(&right.class_name) && left.properties.len() == right.properties.len()
                && left.properties.iter().all(|(name, value)| right.properties.get(name).is_some_and(|other| constant_values_identical(value, other)))
        }
        (Value::Array(left), Value::Array(right)) => left.len() == right.len() && left.iter().zip(right.iter()).all(|((left_key, left_value), (right_key, right_value))| {
            left_key == right_key && constant_values_identical(left_value, right_value)
        }),
        _ => false,
    }
}

fn expanded_trait_properties<'a>(name: &str, declarations: &HashMap<String, &'a php_syntax::TraitDecl>, class: &php_syntax::ClassDecl, program: &Program, seen: &mut Vec<String>) -> Result<Vec<&'a php_syntax::PropertyDecl>, String> {
    let key = name.to_ascii_lowercase();
    if seen.iter().any(|item| item == &key) { return Err(format!("Trait composition cycle involving {name}")); }
    let declaration = declarations.get(&key).ok_or_else(|| format!("Trait '{name}' not found"))?;
    seen.push(key);
    let mut properties: Vec<&php_syntax::PropertyDecl> = Vec::new();
    for nested_name in &declaration.traits {
        for property in expanded_trait_properties(nested_name, declarations, class, program, seen)? {
            if let Some(previous) = properties.iter().find(|previous| previous.name == property.name) {
                if !compatible_trait_property(previous, property, class, program)? {
                    return Err(format!("Trait property conflict for ${} in composition of trait {}", property.name, declaration.name));
                }
            } else {
                properties.push(property);
            }
        }
    }
    for property in &declaration.properties {
        if let Some(previous) = properties.iter().find(|previous| previous.name == property.name) {
            if !compatible_trait_property(previous, property, class, program)? {
                return Err(format!("Trait property conflict for ${} in composition of trait {}", property.name, declaration.name));
            }
        } else {
            properties.push(property);
        }
    }
    seen.pop();
    Ok(properties)
}

fn compatible_trait_constant(left: &php_syntax::ClassConstantDecl, right: &php_syntax::ClassConstantDecl, class: &php_syntax::ClassDecl, program: &Program) -> Result<bool, String> {
    if left.visibility != right.visibility || left.is_final != right.is_final
        || !option_type_names_equal(
            property_type_in_class(left.type_name.as_deref(), class).as_deref(),
            property_type_in_class(right.type_name.as_deref(), class).as_deref(),
        ) {
        return Ok(false);
    }
    let left_value = constant_value_in_class(&left.value, Some(class), Some(program))?;
    let right_value = constant_value_in_class(&right.value, Some(class), Some(program))?;
    Ok(constant_values_identical(&left_value, &right_value))
}

fn expanded_trait_constants<'a>(name: &str, declarations: &HashMap<String, &'a php_syntax::TraitDecl>, class: &php_syntax::ClassDecl, program: &Program, seen: &mut Vec<String>) -> Result<Vec<&'a php_syntax::ClassConstantDecl>, String> {
    let key = name.to_ascii_lowercase();
    if seen.iter().any(|item| item == &key) { return Err(format!("Trait composition cycle involving {name}")); }
    let declaration = declarations.get(&key).ok_or_else(|| format!("Trait '{name}' not found"))?;
    seen.push(key);
    let mut constants: Vec<&php_syntax::ClassConstantDecl> = Vec::new();
    for nested_name in &declaration.traits {
        for constant in expanded_trait_constants(nested_name, declarations, class, program, seen)? {
            if let Some(previous) = constants.iter().find(|previous| previous.name == constant.name) {
                if !compatible_trait_constant(previous, constant, class, program)? {
                    return Err(format!("Trait constant conflict for {} in composition of trait {}", constant.name, declaration.name));
                }
            } else {
                constants.push(constant);
            }
        }
    }
    for constant in declaration.constants.iter().filter(|constant| !constant.is_enum_case) {
        if let Some(previous) = constants.iter().find(|previous| previous.name == constant.name) {
            if !compatible_trait_constant(previous, constant, class, program)? {
                return Err(format!("Trait constant conflict for {} in composition of trait {}", constant.name, declaration.name));
            }
        } else {
            constants.push(constant);
        }
    }
    seen.pop();
    Ok(constants)
}

fn compile_property_hook(
    program: &Program,
    class: &php_syntax::ClassDecl,
    property: &php_syntax::PropertyDecl,
    name: String,
    parameters: Vec<String>,
    parameter_types: Vec<Option<String>>,
    return_type: Option<String>,
    body: &[php_syntax::Stmt],
) -> Result<BytecodeFunction, String> {
    let strict_types = super::strict_mode_for_body(body, program.strict_types);
    let mut generator = CodeGenerator {
        program: Some(program), instructions: Vec::new(), current_source_file: None,
        parent_class: class.parent.as_ref().map(|name| name.to_ascii_lowercase()),
        current_class: Some(class.name.to_ascii_lowercase()),
        current_class_display: Some(class.name.clone()), current_method: Some(name.clone()),
        current_function: Some(name.clone()), strict_types, known_callables: HashMap::new(), variable_types: super::super::declared_parameter_types(&parameters, &parameter_types), returns_by_reference: false,
        control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), next_control_id: 0, include_return_stack: Vec::new(), errors: Vec::new(),
    };
    generator.statements(body);
    generator.resolve_gotos();
    if let Some(error) = generator.errors.first() { return Err(error.clone()); }
    generator.instructions.push(Op::Const(Value::Null));
    generator.instructions.push(Op::Return);
    let mut function = BytecodeFunction::new(
        name, parameters, parameter_types, Vec::new(), Vec::new(), Vec::new(), return_type,
        false, generator.instructions, false, false, false, compile_visibility(property.visibility),
        Some(class.name.to_ascii_lowercase()),
    );
    function.strict_types = strict_types;
    Ok(function)
}

fn expanded_trait_methods(name: &str, declarations: &HashMap<String, &php_syntax::TraitDecl>, seen: &mut Vec<String>) -> Result<Vec<(String, FunctionDecl)>, String> {
    let key = name.to_ascii_lowercase();
    if seen.iter().any(|item| item == &key) { return Err(format!("Trait composition cycle involving {name}")); }
    let declaration = declarations.get(&key).ok_or_else(|| format!("Trait '{name}' not found"))?;
    seen.push(key.clone());
    let mut methods = Vec::new();
    for nested_name in &declaration.traits {
        let nested = expanded_trait_methods(nested_name, declarations, seen)?;
        methods.extend(nested.into_iter().map(|(_, method)| (nested_name.to_ascii_lowercase(), method)));
    }
    let composed_methods = methods.clone();
    // Apply this trait's adaptations against methods from its direct nested
    // traits, keeping the immediate provider for conflict resolution.
    for adaptation in &declaration.trait_adaptations {
        for excluded in &adaptation.excluded_traits {
            if !declaration.traits.iter().any(|nested| nested.eq_ignore_ascii_case(excluded)) {
                return Err(format!("Trait '{excluded}' is not part of the composition of trait {}", declaration.name));
            }
        }
        methods.retain(|(origin, method)| !(adaptation.method_name.eq_ignore_ascii_case(&method.name)
            && adaptation.excluded_traits.iter().any(|excluded| excluded.eq_ignore_ascii_case(origin))));
        let source = composed_methods.iter().find(|(origin, method)| origin.eq_ignore_ascii_case(&adaptation.trait_name)
            && method.name.eq_ignore_ascii_case(&adaptation.method_name)).map(|(_, method)| method.clone())
            .ok_or_else(|| format!("Trait method {}::{} not found", adaptation.trait_name, adaptation.method_name))?;
        let mut adapted = source;
        if let Some(visibility) = adaptation.visibility { adapted.visibility = visibility; }
        if adaptation.is_final { adapted.is_final = true; }
        if let Some(alias) = &adaptation.alias {
            adapted.name = alias.clone();
            methods.push((adaptation.trait_name.to_ascii_lowercase(), adapted));
        } else if adaptation.visibility.is_some() || adaptation.is_final {
            if let Some((_, method)) = methods.iter_mut().find(|(origin, method)| origin.eq_ignore_ascii_case(&adaptation.trait_name)
                && method.name.eq_ignore_ascii_case(&adaptation.method_name)) {
                method.visibility = adapted.visibility;
                method.is_final = adapted.is_final;
            }
        }
    }
    // Methods declared by the using trait override same-named methods pulled
    // in from its nested traits (for example Laravel's InteractsWithInput
    // overrides dump() from Dumpable/InteractsWithData).
    for method in &declaration.methods {
        methods.retain(|(_, candidate)| !candidate.name.eq_ignore_ascii_case(&method.name));
        methods.push((name.to_ascii_lowercase(), method.clone()));
    }
    let mut concrete_methods = HashSet::new();
    for (_, method) in &methods {
        if !method.is_abstract && !concrete_methods.insert(method.name.to_ascii_lowercase()) {
            return Err(format!("Trait method conflict for {} in trait {}; use insteadof to resolve it", method.name, declaration.name));
        }
    }
    for (origin, _) in &mut methods { *origin = key.clone(); }
    seen.pop();
    Ok(methods)
}

/// Resolve traits, compile methods, assemble class metadata, and check interfaces.
pub(super) fn compile(program: &Program) -> Result<HashMap<String, BytecodeClass>, String> {
    validate_attribute_class_declarations(program)?;
    let builtin_classes = [
        ("exception", None), ("error", None), ("typeerror", Some("error")),
        ("argumentcounterror", Some("typeerror")), ("compileerror", Some("error")),
        ("parseerror", Some("compileerror")), ("arithmeticerror", Some("error")),
        ("divisionbyzeroerror", Some("arithmeticerror")), ("assertionerror", Some("error")),
        ("unhandledmatcherror", Some("error")), ("valueerror", Some("error")), ("jsonexception", Some("exception")),
        ("logicexception", Some("exception")), ("badfunctioncallexception", Some("logicexception")),
        ("badmethodcallexception", Some("badfunctioncallexception")), ("domainexception", Some("logicexception")),
        ("invalidargumentexception", Some("logicexception")), ("lengthexception", Some("logicexception")),
        ("outofrangeexception", Some("logicexception")), ("runtimeexception", Some("exception")),
        ("outofboundsexception", Some("runtimeexception")), ("overflowexception", Some("runtimeexception")),
        ("rangeexception", Some("runtimeexception")), ("underflowexception", Some("runtimeexception")),
        ("unexpectedvalueexception", Some("runtimeexception")),
    ];
    if let Some(class) = program.classes.iter().find(|class| builtin_classes.iter().any(|(name, _)| class.name.eq_ignore_ascii_case(name))) {
        return Err(format!("Cannot redeclare built-in class {}", class.name));
    }
    let builtin_interfaces = ["throwable", "traversable", "iterator", "iteratoraggregate", "countable", "outeriterator", "recursiveiterator", "jsonserializable", "stringable", "unitenum", "backedenum"];
    if let Some(interface) = program.interfaces.iter().find(|interface| builtin_interfaces.iter().any(|name| interface.name.eq_ignore_ascii_case(name))) {
        return Err(format!("Cannot redeclare built-in interface {}", interface.name));
    }
    if let Some(class) = program.classes.iter().find(|class| class.interfaces.iter().any(|name| name.eq_ignore_ascii_case("Throwable"))) {
        return Err(format!("Class {} cannot implement interface Throwable directly; extend Exception or Error instead", class.name));
    }
    let trait_declarations = program.traits.iter().map(|declaration| (declaration.name.to_ascii_lowercase(), declaration)).collect::<HashMap<_, _>>();
    for declaration in &program.traits {
        expanded_trait_methods(&declaration.name, &trait_declarations, &mut Vec::new())?;
    }
    let interface_declarations = program.interfaces.iter().map(|declaration| (declaration.name.to_ascii_lowercase(), declaration)).collect::<HashMap<_, _>>();
    let class_declarations = program.classes.iter().map(|declaration| (declaration.name.to_ascii_lowercase(), declaration)).collect::<HashMap<_, _>>();
    let mut classes = HashMap::new();
    for trait_decl in &program.traits {
        classes.insert(trait_decl.name.to_ascii_lowercase(), BytecodeClass {
            name: trait_decl.name.clone(), reflection_name: trait_decl.name.clone(), attributes: compile_constant_attributes(&trait_decl.attributes, None, program)?, parent: None, interfaces: Vec::new(), traits: trait_decl.traits.clone(), trait_aliases: trait_decl.trait_adaptations.iter().filter_map(|adaptation| adaptation.alias.as_ref().map(|alias| (alias.clone(), format!("{}::{}", adaptation.trait_name, adaptation.method_name)))).collect(), is_interface: false, is_trait: true, unresolved_traits: Vec::new(),
            is_abstract: false, is_final: false, is_readonly: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(),
            static_properties: HashMap::new(), constants: HashMap::new(), constant_order: Vec::new(), methods: HashMap::new(), method_order: Vec::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
        });
    }
    classes.insert("throwable".into(), BytecodeClass {
        name: "Throwable".into(), reflection_name: "Throwable".into(), attributes: Vec::new(), parent: None, interfaces: Vec::new(), traits: Vec::new(), trait_aliases: Vec::new(), is_interface: true, is_trait: false, unresolved_traits: Vec::new(),
        is_abstract: true, is_final: false, is_readonly: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(), static_properties: HashMap::new(),
        constants: HashMap::new(), constant_order: Vec::new(), methods: HashMap::new(), method_order: Vec::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
    });
    for (name, parents) in [
        ("traversable", Vec::<String>::new()),
        ("iterator", vec!["traversable".into()]),
        ("iteratoraggregate", vec!["traversable".into()]),
        ("countable", Vec::new()),
        ("outeriterator", vec!["iterator".into()]),
        ("recursiveiterator", vec!["iterator".into()]),
        ("jsonserializable", Vec::new()),
        ("stringable", Vec::new()),
        ("unitenum", Vec::new()),
        ("backedenum", vec!["unitenum".into()]),
    ] {
        let canonical = match name {
            "traversable" => "Traversable", "iterator" => "Iterator", "iteratoraggregate" => "IteratorAggregate",
            "countable" => "Countable", "outeriterator" => "OuterIterator", "recursiveiterator" => "RecursiveIterator",
            "jsonserializable" => "JsonSerializable", "stringable" => "Stringable", "unitenum" => "UnitEnum",
            "backedenum" => "BackedEnum", _ => unreachable!(),
        };
        classes.insert(name.into(), BytecodeClass {
            name: canonical.into(), reflection_name: canonical.into(), attributes: Vec::new(), parent: None, interfaces: parents, traits: Vec::new(), trait_aliases: Vec::new(), is_interface: true, is_trait: false, unresolved_traits: Vec::new(), is_abstract: true,
            is_final: false, is_readonly: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(), static_properties: HashMap::new(), constants: HashMap::new(), constant_order: Vec::new(), methods: HashMap::new(), method_order: Vec::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
        });
    }
    for (name, parent) in builtin_classes {
        let canonical_name = match name {
            "exception" => "Exception", "error" => "Error", "typeerror" => "TypeError",
            "argumentcounterror" => "ArgumentCountError", "compileerror" => "CompileError",
            "parseerror" => "ParseError", "arithmeticerror" => "ArithmeticError",
            "divisionbyzeroerror" => "DivisionByZeroError", "assertionerror" => "AssertionError",
            "unhandledmatcherror" => "UnhandledMatchError", "valueerror" => "ValueError", "jsonexception" => "JsonException",
            "logicexception" => "LogicException", "badfunctioncallexception" => "BadFunctionCallException",
            "badmethodcallexception" => "BadMethodCallException", "domainexception" => "DomainException",
            "invalidargumentexception" => "InvalidArgumentException", "lengthexception" => "LengthException",
            "outofrangeexception" => "OutOfRangeException", "runtimeexception" => "RuntimeException",
            "outofboundsexception" => "OutOfBoundsException", "overflowexception" => "OverflowException",
            "rangeexception" => "RangeException", "underflowexception" => "UnderflowException",
            "unexpectedvalueexception" => "UnexpectedValueException", _ => unreachable!(),
        };
        classes.insert(name.into(), BytecodeClass {
            name: canonical_name.into(), reflection_name: canonical_name.into(), attributes: Vec::new(), parent: parent.map(str::to_string), interfaces: Vec::new(), traits: Vec::new(), trait_aliases: Vec::new(),
            is_interface: false, is_trait: false, unresolved_traits: Vec::new(), is_abstract: false, is_final: false, is_readonly: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(),
            static_properties: HashMap::new(), constants: HashMap::new(), constant_order: Vec::new(), methods: HashMap::new(), method_order: Vec::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
        });
    }
    if program.classes.iter().any(|class| class.name.eq_ignore_ascii_case("RoundingMode")) {
        return Err("Cannot redeclare built-in enum RoundingMode".into());
    }
    let rounding_cases = ["HalfAwayFromZero", "HalfTowardsZero", "HalfEven", "HalfOdd", "TowardsZero", "AwayFromZero", "NegativeInfinity", "PositiveInfinity"];
    let mut rounding_constants = HashMap::new();
    for case in rounding_cases {
        let value = Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
            class_name: "RoundingMode".into(),
            properties: HashMap::from([("name".into(), Value::String(case.into()))]),
            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
        })));
        rounding_constants.insert(case.to_string(), BytecodeConstant {
            value, visibility: MemberVisibility::Public, declaring_class: "roundingmode".into(), is_final: true,
            type_name: None, attributes: Vec::new(),
        });
    }
    classes.insert("roundingmode".into(), BytecodeClass {
        name: "RoundingMode".into(), reflection_name: "RoundingMode".into(), attributes: Vec::new(), parent: None, interfaces: vec!["unitenum".into()], traits: Vec::new(), trait_aliases: Vec::new(), is_interface: false, is_trait: false, unresolved_traits: Vec::new(),
        is_abstract: false, is_final: true, is_readonly: false, is_enum: true, enum_backing_type: None,
        enum_cases: rounding_cases.iter().map(|case| (*case).to_string()).collect(),
        properties: HashMap::new(), property_order: Vec::new(), static_properties: HashMap::new(), constants: rounding_constants, constant_order: rounding_cases.iter().map(|case| (*case).to_string()).collect(), methods: HashMap::new(), method_order: Vec::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
    });
    if program.classes.iter().any(|class| class.name.eq_ignore_ascii_case("SortDirection")) {
        return Err("Cannot redeclare built-in enum SortDirection".into());
    }
    let sort_cases = ["Ascending", "Descending"];
    let mut sort_constants = HashMap::new();
    for case in sort_cases {
        let value = Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
            class_name: "SortDirection".into(), properties: HashMap::from([("name".into(), Value::String(case.into()))]),
            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
        })));
        sort_constants.insert(case.to_string(), BytecodeConstant { value, visibility: MemberVisibility::Public, declaring_class: "sortdirection".into(), is_final: true, type_name: None, attributes: Vec::new() });
    }
    classes.insert("sortdirection".into(), BytecodeClass {
        name: "SortDirection".into(), reflection_name: "SortDirection".into(), attributes: Vec::new(), parent: None, interfaces: vec!["unitenum".into()], traits: Vec::new(), trait_aliases: Vec::new(), is_interface: false, is_trait: false, unresolved_traits: Vec::new(),
        is_abstract: false, is_final: true, is_readonly: false, is_enum: true, enum_backing_type: None,
        enum_cases: sort_cases.iter().map(|case| (*case).to_string()).collect(), properties: HashMap::new(), property_order: Vec::new(),
        static_properties: HashMap::new(), constants: sort_constants, constant_order: sort_cases.iter().map(|case| (*case).to_string()).collect(), methods: HashMap::new(), method_order: Vec::new(), method_attributes: HashMap::new(),
        method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
    });
    for class in &program.classes {
        if class.is_enum {
            if class.is_abstract { return Err(format!("Enum {} cannot be abstract", class.name)); }
            if !class.properties.is_empty() { return Err(format!("Enum {} cannot include properties", class.name)); }
            if class.methods.iter().any(|method| method.name.eq_ignore_ascii_case("cases")
                || (class.enum_backing_type.is_some() && matches!(method.name.to_ascii_lowercase().as_str(), "from" | "tryfrom"))) {
                return Err(format!("Enum {} cannot redeclare a built-in enum method", class.name));
            }
            let mut backing_values = Vec::new();
            for case in class.constants.iter().filter(|constant| constant.is_enum_case) {
                if let Some(backing_type) = class.enum_backing_type.as_deref() {
                    if !case.enum_case_has_value { return Err(format!("Backed enum case {}::{} must have a value", class.name, case.name)); }
                    let value = constant_value_in_class(&case.value, Some(class), Some(program))?;
                    let valid_type = matches!((backing_type, value), ("int", Value::Integer(_)) | ("string", Value::String(_)));
                    if !valid_type { return Err(format!("Enum case {}::{} has the wrong backing type", class.name, case.name)); }
                    let value = constant_value_in_class(&case.value, Some(class), Some(program))?;
                    if backing_values.iter().any(|existing| enum_backing_values_equal(existing, &value)) {
                        return Err(format!("Duplicate value in backed enum {}", class.name));
                    }
                    backing_values.push(value);
                } else if case.enum_case_has_value {
                    return Err(format!("Pure enum case {}::{} cannot have a value", class.name, case.name));
                }
            }
        }
        let mut properties = HashMap::new();
        let mut property_order = Vec::new();
        let mut trait_properties = std::collections::HashSet::new();
        let mut trait_property_declarations = HashMap::<String, &php_syntax::PropertyDecl>::new();
        let mut trait_constant_declarations = HashMap::<String, &php_syntax::ClassConstantDecl>::new();
        let mut static_properties = HashMap::new();
        let mut constants = HashMap::new();
        let mut constant_order = Vec::new();
        let mut enum_cases = Vec::new();
        let mut methods = HashMap::new();
        let mut method_order = Vec::new();
        let mut trait_methods: HashMap<String, Vec<(String, FunctionDecl)>> = HashMap::new();
        let mut trait_method_order = Vec::new();
        let is_anonymous = class.name.starts_with("class@anonymous#");
        let mut unresolved_traits = Vec::new();
        // Trait members are inserted first so explicit class members can override them.
        for trait_name in &class.traits {
            if !trait_declarations.contains_key(&trait_name.to_ascii_lowercase()) {
                if is_anonymous {
                    unresolved_traits.push(trait_name.clone());
                    continue;
                }
                return Err(format!("Trait '{trait_name}' not found"));
            }
            for constant in expanded_trait_constants(trait_name, &trait_declarations, class, program, &mut Vec::new())? {
                if let Some(previous) = trait_constant_declarations.get(&constant.name) {
                    if !compatible_trait_constant(previous, constant, class, program)? {
                        return Err(format!("Trait constant conflict for {} in class {}", constant.name, class.name));
                    }
                    continue;
                }
                trait_constant_declarations.insert(constant.name.clone(), constant);
                let value = constant_value_in_class(&constant.value, Some(class), Some(program))?;
                let attributes = compile_constant_attributes(&constant.attributes, Some(class), program)?;
                if !constants.contains_key(&constant.name) { constant_order.push(constant.name.clone()); }
                constants.entry(constant.name.clone()).or_insert_with(|| BytecodeConstant {
                    value,
                    visibility: compile_visibility(constant.visibility),
                    declaring_class: class.name.to_ascii_lowercase(),
                    is_final: constant.is_final,
                    type_name: property_type_in_class(constant.type_name.as_deref(), class),
                    attributes,
                });
            }
        let composed_properties = expanded_trait_properties(trait_name, &trait_declarations, class, program, &mut Vec::new())?;
        if class.is_enum && !composed_properties.is_empty() {
            return Err(format!("Enum {} cannot include properties", class.name));
        }
        for property in composed_properties {
            let property_key = property.name.clone();
            if let Some(previous) = trait_property_declarations.get(&property_key) {
                if !compatible_trait_property(previous, property, class, program)? {
                    return Err(format!("Trait property conflict for ${} in class {}", property.name, class.name));
                }
                continue;
            }
            trait_property_declarations.insert(property_key, property);
            trait_properties.insert(property.name.clone());
            let default = class_property_default(property, class, program)?;
            let get_hook = property.get_hook.as_ref().map(|body| compile_property_hook(
                program, class, property, format!("${}::get", property.name), Vec::new(), Vec::new(), property.type_name.clone(), body,
            )).transpose()?;
            let set_hook = property.set_hook.as_ref().map(|hook| compile_property_hook(
                program, class, property, format!("${}::set", property.name), vec![hook.parameter.clone()],
                vec![property.type_name.clone()], None, &hook.body,
            )).transpose()?;
            let metadata = BytecodeProperty {
                default,
                visibility: compile_visibility(property.visibility),
                set_visibility: property.set_visibility.map(compile_visibility),
                declaring_class: class.name.to_ascii_lowercase(),
                type_name: property_type_in_class(property.type_name.as_deref(), class),
                is_readonly: property.is_readonly,
                is_promoted: false,
                get_hook,
                set_hook,
            };
            if property.is_static {
                static_properties.entry(property.name.clone()).or_insert(metadata);
            } else {
                if !properties.contains_key(&property.name) { property_order.push(property.name.clone()); }
                properties.entry(property.name.clone()).or_insert(metadata);
            }
            }
            for method in &expanded_trait_methods(trait_name, &trait_declarations, &mut Vec::new())? {
                let method_name = method.1.name.to_ascii_lowercase();
                if !trait_methods.contains_key(&method_name) { trait_method_order.push(method_name.clone()); }
                trait_methods.entry(method_name).or_default().push(method.clone());
            }
        }
        // Resolve each trait method after reading all traits, applying `insteadof` rules.
        for method_name in trait_method_order {
            let candidates = trait_methods.remove(&method_name).unwrap_or_default();
            let survivors = candidates.iter().filter(|(candidate_trait, _)| {
                !class.trait_adaptations.iter().any(|adaptation| {
                    adaptation.method_name.eq_ignore_ascii_case(&method_name)
                        && adaptation.excluded_traits.iter().any(|excluded| excluded.eq_ignore_ascii_case(candidate_trait))
                })
            }).collect::<Vec<_>>();
            let concrete = survivors.iter().filter(|(_, method)| !method.is_abstract).collect::<Vec<_>>();
            if concrete.len() > 1 {
                return Err(format!("Trait method conflict for {method_name} in class {}; use insteadof to resolve it", class.name));
            }
            let selected = concrete.first().map(|candidate| **candidate).or_else(|| survivors.first().copied());
            if let Some((_, method)) = selected {
                method_order.push(method_name.clone());
                methods.insert(method_name.clone(), (*method).clone());
            }
        }
        // Apply `as` visibility changes and aliases. An alias keeps the original method too.
        for adaptation in &class.trait_adaptations {
            if unresolved_traits.iter().any(|name| name.eq_ignore_ascii_case(&adaptation.trait_name)) { continue; }
            if !class.traits.iter().any(|name| name.eq_ignore_ascii_case(&adaptation.trait_name)) {
                return Err(format!("Trait '{}' is not part of the composition of class {}", adaptation.trait_name, class.name));
            }
            for excluded in &adaptation.excluded_traits {
                if !class.traits.iter().any(|name| name.eq_ignore_ascii_case(excluded)) {
                    return Err(format!("Trait '{excluded}' is not part of the composition of class {}", class.name));
                }
            }
            // Adaptations may name a method supplied by a nested trait, so
            // resolve against the composed method set rather than only the
            // methods declared directly on the named trait.
            let source = expanded_trait_methods(&adaptation.trait_name, &trait_declarations, &mut Vec::new())?
                .into_iter().find(|(_, method)| method.name.eq_ignore_ascii_case(&adaptation.method_name)).map(|(_, method)| method)
                .ok_or_else(|| format!("Trait method {}::{} not found", adaptation.trait_name, adaptation.method_name))?;
            // A visibility-only adaptation changes the method imported under
            // its original name. If that trait lost the name to an
            // `insteadof` rule, it must not replace the selected implementation.
            // An explicit alias still imports the losing implementation under
            // the alias, as PHP allows.
            if adaptation.alias.is_none() && class.trait_adaptations.iter().any(|precedence| {
                precedence.method_name.eq_ignore_ascii_case(&adaptation.method_name)
                    && precedence.excluded_traits.iter().any(|excluded| excluded.eq_ignore_ascii_case(&adaptation.trait_name))
            }) {
                continue;
            }
            let mut adapted = source;
            if let Some(visibility) = adaptation.visibility { adapted.visibility = visibility; }
            if adaptation.is_final { adapted.is_final = true; }
            if let Some(alias) = &adaptation.alias {
                adapted.name = alias.clone();
                let alias_name = alias.to_ascii_lowercase();
                if !methods.contains_key(&alias_name) { method_order.push(alias_name.clone()); }
                methods.insert(alias_name, adapted);
            } else {
                let method_name = adaptation.method_name.to_ascii_lowercase();
                if !methods.contains_key(&method_name) { method_order.push(method_name.clone()); }
                methods.insert(method_name, adapted);
            }
        }
        for property in &class.properties {
            let default = class_property_default(property, class, program)?;
            if let Some(trait_property) = trait_property_declarations.get(&property.name) {
                if !compatible_trait_property(trait_property, property, class, program)? {
                    return Err(format!("Property {}::${} conflicts with a trait property", class.name, property.name));
                }
            }
            let get_hook = property.get_hook.as_ref().map(|body| compile_property_hook(
                program, class, property, format!("${}::get", property.name), Vec::new(), Vec::new(), property.type_name.clone(), body,
            )).transpose()?;
            let set_hook = property.set_hook.as_ref().map(|hook| compile_property_hook(
                program, class, property, format!("${}::set", property.name), vec![hook.parameter.clone()],
                vec![property.type_name.clone()], None, &hook.body,
            )).transpose()?;
            let metadata = BytecodeProperty {
                default,
                visibility: compile_visibility(property.visibility),
                set_visibility: property.set_visibility.map(compile_visibility),
                declaring_class: class.name.to_ascii_lowercase(),
                type_name: property_type_in_class(property.type_name.as_deref(), class),
                is_readonly: property.is_readonly,
                is_promoted: false,
                get_hook,
                set_hook,
            };
            if property.is_static { static_properties.insert(property.name.clone(), metadata); }
            else { if !properties.contains_key(&property.name) { property_order.push(property.name.clone()); } properties.insert(property.name.clone(), metadata); }
        }
        for method in &class.methods {
            let magic = method.name.to_ascii_lowercase();
            // Reject malformed magic hooks during compilation instead of failing at dispatch time.
            let magic_arity = match magic.as_str() {
                "__get" => Some((1, false)),
                "__set" => Some((2, false)),
                "__call" => Some((2, false)),
                "__callstatic" => Some((2, true)),
                "__isset" => Some((1, false)),
                "__unset" => Some((1, false)),
                "__clone" => Some((0, false)),
                "__destruct" => Some((0, false)),
                "__serialize" | "__sleep" | "__wakeup" | "__debuginfo" => Some((0, false)),
                "__unserialize" => Some((1, false)),
                "__set_state" => Some((1, true)),
                _ => None,
            };
            if let Some((expected_parameters, is_static)) = magic_arity {
                let visibility_is_valid = matches!(magic.as_str(), "__clone" | "__destruct") || method.visibility == Visibility::Public;
                if !visibility_is_valid || method.is_static != is_static || method.parameters.len() != expected_parameters {
                    let static_description = if is_static { "static" } else { "non-static" };
                    let visibility_description = if matches!(magic.as_str(), "__clone" | "__destruct") { "" } else { "public, " };
                    return Err(format!("{}::{}() must be {visibility_description}{static_description}, and accept {expected_parameters} parameter(s)", class.name, method.name));
                }
            }
            if matches!(magic.as_str(), "__construct" | "__destruct") && method.return_type.is_some() {
                return Err(format!("{}::{}() cannot declare a return type", class.name, method.name));
            }
            let typed_magic_parameters: &[&str] = match magic.as_str() {
                "__get" | "__isset" | "__unset" => &["string"],
                "__set" => &["string", "mixed"],
                "__call" | "__callstatic" => &["string", "array"],
                "__unserialize" | "__set_state" => &["array"],
                "__serialize" | "__sleep" | "__wakeup" | "__debuginfo" | "__clone" | "__tostring" => &[],
                _ => &[],
            };
            for (index, expected) in typed_magic_parameters.iter().enumerate() {
                if method.parameter_types.get(index).and_then(Option::as_deref)
                    .is_some_and(|actual| !actual.eq_ignore_ascii_case(expected))
                {
                    return Err(format!("{}::{}() parameter #{} must be of type {expected}", class.name, method.name, index + 1));
                }
            }
            let typed_magic_return = match magic.as_str() {
                "__serialize" | "__sleep" | "__debuginfo" => Some("array"),
                "__unserialize" | "__wakeup" | "__set" | "__unset" | "__clone" => Some("void"),
                "__set_state" => Some("object"),
                "__get" | "__call" | "__callstatic" => Some("mixed"),
                "__isset" => Some("bool"),
                "__tostring" => Some("string"),
                _ => None,
            };
            if let Some(expected_return) = typed_magic_return {
                if method.return_type.as_deref().is_some_and(|actual|
                    !actual.eq_ignore_ascii_case(expected_return) && !actual.eq_ignore_ascii_case("never"))
                {
                    return Err(format!("{}::{}() must return {expected_return}", class.name, method.name));
                }
            }
            if matches!(magic.as_str(), "__invoke" | "__tostring")
                && (method.visibility != Visibility::Public || method.is_static || (magic == "__tostring" && !method.parameters.is_empty()))
            {
                let requirement = if magic == "__tostring" { "public, non-static, and accept no parameters" } else { "public and non-static" };
                return Err(format!("{}::{}() must be {requirement}", class.name, method.name));
            }
            if magic == "__tostring" && method.return_type.as_deref().is_some_and(|return_type| !return_type.eq_ignore_ascii_case("string")) {
                return Err(format!("{}::__toString() must declare a string return type", class.name));
            }
            if !method.promoted_properties.is_empty() && method.name.to_ascii_lowercase() != "__construct" {
                return Err(format!("Only __construct() may promote properties in class {}", class.name));
            }
            if method.name.to_ascii_lowercase() == "__construct" && method.is_static && !method.promoted_properties.is_empty() {
                return Err(format!("Static constructor {}::__construct() cannot promote properties", class.name));
            }
            for property in &method.promoted_properties {
                let promoted_default = class_property_default(property, class, program)?;
                if properties.contains_key(&property.name) || static_properties.contains_key(&property.name) {
                    let existing = properties.get(&property.name);
                    let compatible_trait_property = existing.is_some_and(|existing| {
                        trait_properties.contains(&property.name)
                            && existing.visibility == compile_visibility(property.visibility)
                            && existing.set_visibility == property.set_visibility.map(compile_visibility)
                            && existing.is_readonly == property.is_readonly
                            && option_type_names_equal(existing.type_name.as_deref(), property_type_in_class(property.type_name.as_deref(), class).as_deref())
                            && constant_options_identical(&existing.default, &promoted_default)
                    });
                    if !compatible_trait_property {
                        return Err(format!("Promoted property ${} conflicts with a declared property", property.name));
                    }
                    continue;
                }
                properties.insert(property.name.clone(), BytecodeProperty {
                    default: promoted_default,
                    visibility: compile_visibility(property.visibility),
                    set_visibility: property.set_visibility.map(compile_visibility),
                    declaring_class: class.name.to_ascii_lowercase(),
                    type_name: property_type_in_class(property.type_name.as_deref(), class),
                    is_readonly: property.is_readonly,
                    is_promoted: true,
                    get_hook: None,
                    set_hook: None,
                });
                property_order.push(property.name.clone());
            }
        }
        for constant in &class.constants {
            if let Some(trait_constant) = trait_constant_declarations.get(&constant.name) {
                if !compatible_trait_constant(trait_constant, constant, class, program)? {
                    return Err(format!("Class constant {}::{} conflicts with a trait constant", class.name, constant.name));
                }
            } else if constants.contains_key(&constant.name) {
                return Err(format!("Cannot redeclare {}::{}", class.name, constant.name));
            }
            let value = if constant.is_enum_case {
                enum_cases.push(constant.name.clone());
                let mut properties = HashMap::from([("name".into(), Value::String(constant.name.clone()))]);
                if class.enum_backing_type.is_some() {
                    properties.insert("value".into(), constant_value_in_class(&constant.value, Some(class), Some(program))?);
                }
                Value::Object(std::rc::Rc::new(std::cell::RefCell::new(php_runtime::PhpObject {
                    class_name: class.name.clone(), properties, property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                })))
            } else { constant_value_in_class(&constant.value, Some(class), Some(program))? };
            constants.insert(constant.name.clone(), BytecodeConstant {
                value,
                visibility: compile_visibility(constant.visibility),
                declaring_class: class.name.to_ascii_lowercase(),
                is_final: constant.is_final,
                type_name: property_type_in_class(constant.type_name.as_deref(), class),
                attributes: compile_constant_attributes(&constant.attributes, Some(class), program)?,
            });
            constant_order.push(constant.name.clone());
        }
        for method in &class.methods {
            let method_name = method.name.to_ascii_lowercase();
            if !methods.contains_key(&method_name) { method_order.push(method_name.clone()); }
            methods.insert(method_name, method.clone());
        }
        let mut compiled_methods = HashMap::new();
        let mut method_attributes = HashMap::new();
        let mut parameter_attributes = HashMap::new();
        let method_display_names = methods.values().map(|method| (method.name.to_ascii_lowercase(), method.name.clone())).collect::<HashMap<_, _>>();
        let property_attributes = class.properties.iter().filter(|property| !property.attributes.is_empty()).map(|property| {
            let attributes = property.attributes.iter().map(|attribute| {
                let arguments = attribute.arguments.iter().map(|argument| match argument {
                    php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, Some(class), Some(program))?)),
                    value => Ok((None, constant_value_in_class(value, Some(class), Some(program))?)),
                }).collect::<Result<Vec<_>, String>>()?;
                Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
            }).collect::<Result<Vec<_>, String>>()?;
            Ok((property.name.clone(), attributes))
        }).collect::<Result<HashMap<_, _>, String>>()?;
        for method in methods.values() {
            let strict_types = super::strict_mode_for_body(&method.body, program.strict_types);
            let mut generator = CodeGenerator { program: Some(program), instructions: Vec::new(), current_source_file: None, parent_class: class.parent.as_ref().map(|name| name.to_ascii_lowercase()), current_class: Some(class.name.to_ascii_lowercase()), current_class_display: Some(class.name.clone()), current_method: Some(method.name.to_ascii_lowercase()), current_function: Some(method.name.clone()), strict_types, known_callables: HashMap::new(), variable_types: super::super::declared_parameter_types(&method.parameters, &method.parameter_types), returns_by_reference: method.returns_by_reference, control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), next_control_id: 0, include_return_stack: Vec::new(), errors: Vec::new() };
            for property in &method.promoted_properties {
                generator.instructions.push(Op::Load("this".into()));
                generator.instructions.push(Op::Load(property.name.clone()));
                generator.instructions.push(Op::SetProperty(property.name.clone(), Some(class.name.to_ascii_lowercase()), true));
            }
            generator.statements(&method.body);
            generator.resolve_gotos();
            if let Some(error) = generator.errors.first() { return Err(error.clone()); }
            generator.instructions.push(Op::Const(Value::Null));
            generator.instructions.push(Op::Return);
            if method.is_abstract && method.is_final {
                return Err(format!("Method {}::{}() cannot be both abstract and final", class.name, method.name));
            }
            let mut compiled_method = BytecodeFunction::new(
                method.name.to_ascii_lowercase(), method.parameters.clone(), method.parameter_types.clone(), method.parameter_by_reference.clone(), method.parameter_variadic.clone(), super::parameter_defaults(&method.parameter_defaults, Some(program), Some(&class.name), class.parent.as_deref())?, method.return_type.clone(), method.returns_by_reference, generator.instructions, method.is_static, method.is_abstract,
                method.is_final, compile_visibility(method.visibility), Some(class.name.to_ascii_lowercase()),
            );
            compiled_method.parameter_default_constant_names = super::parameter_default_constant_names(&method.parameter_defaults);
            compiled_method.parameter_promoted = method.parameters.iter().map(|parameter| {
                method.promoted_properties.iter().any(|property| property.name == *parameter)
            }).collect();
            compiled_method.strict_types = strict_types;
            compiled_methods.insert(method.name.to_ascii_lowercase(), compiled_method);
            if !method.attributes.is_empty() {
                let attributes = method.attributes.iter().map(|attribute| {
                    let arguments = attribute.arguments.iter().map(|argument| match argument {
                        php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, Some(class), Some(program))?)),
                        value => Ok((None, constant_value_in_class(value, Some(class), Some(program))?)),
                    }).collect::<Result<Vec<_>, String>>()?;
                    Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
                }).collect::<Result<Vec<_>, String>>()?;
                method_attributes.insert(method.name.to_ascii_lowercase(), attributes);
            }
            let attributes = method.parameter_attributes.iter().map(|parameter| parameter.iter().map(|attribute| {
                let arguments = attribute.arguments.iter().map(|argument| match argument {
                    php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, Some(class), Some(program))?)),
                    value => Ok((None, constant_value_in_class(value, Some(class), Some(program))?)),
                }).collect::<Result<Vec<_>, String>>()?;
                Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
            }).collect::<Result<Vec<_>, String>>()).collect::<Result<Vec<_>, String>>()?;
            if attributes.iter().any(|attributes| !attributes.is_empty()) { parameter_attributes.insert(method.name.to_ascii_lowercase(), attributes); }
        }
        let attributes = class.attributes.iter().map(|attribute| {
            let arguments = attribute.arguments.iter().map(|argument| match argument {
                php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, Some(class), Some(program))?)),
                value => Ok((None, constant_value_in_class(value, Some(class), Some(program))?)),
            }).collect::<Result<Vec<_>, String>>()?;
            Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
        }).collect::<Result<Vec<_>, String>>()?;
        let mut implemented_interfaces = class.interfaces.iter().map(|name| name.to_ascii_lowercase()).collect::<Vec<_>>();
        if class.is_enum {
            implemented_interfaces.push("unitenum".into());
            if class.enum_backing_type.is_some() { implemented_interfaces.push("backedenum".into()); }
        }
        let trait_aliases = class.trait_adaptations.iter().filter_map(|adaptation| adaptation.alias.as_ref().map(|alias| (alias.clone(), format!("{}::{}", adaptation.trait_name, adaptation.method_name)))).collect();
        let reflection_name = class.anonymous_source.as_ref().map(|(file, line)| {
            let file = file.as_deref().unwrap_or("Command line code");
            let sequence = class.name.strip_prefix("class@anonymous#").unwrap_or("0");
            let class_prefix = class.parent.as_deref().map(|parent| {
                program.classes.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(parent)).map(|candidate| candidate.name.clone())
                    .or_else(|| program.interfaces.iter().find(|candidate| candidate.name.eq_ignore_ascii_case(parent)).map(|candidate| candidate.name.clone()))
                    .unwrap_or_else(|| match parent.to_ascii_lowercase().as_str() {
                        "exception" => "Exception".into(), "error" => "Error".into(), "stdclass" => "stdClass".into(), _ => parent.to_string(),
                    })
            }).unwrap_or_else(|| "class".into());
            format!("{}@anonymous\0{file}:{line}${sequence}", class_prefix.trim_start_matches('\\'))
        }).unwrap_or_else(|| class.name.clone());
        classes.insert(class.name.to_ascii_lowercase(), BytecodeClass { name: class.name.clone(), reflection_name, attributes, parent: class.parent.as_ref().map(|name| name.to_ascii_lowercase()), interfaces: implemented_interfaces, traits: class.traits.clone(), trait_aliases, is_interface: false, is_trait: false, unresolved_traits, is_abstract: class.is_abstract, is_final: class.is_final || class.is_enum, is_readonly: class.is_readonly, enum_backing_type: class.enum_backing_type.clone(), is_enum: class.is_enum, enum_cases, properties, property_order, static_properties, constants, constant_order: constant_order.clone(), methods: compiled_methods, method_order, method_attributes, method_display_names, property_attributes, parameter_attributes });
    }
    // Interfaces live in the runtime class table so `InterfaceName::CONSTANT` resolves normally.
    for interface in &program.interfaces {
        let mut constants = HashMap::new();
        for constant in &interface.constants {
            constants.insert(constant.name.clone(), BytecodeConstant {
                value: constant_value(&constant.value)?, visibility: MemberVisibility::Public,
                declaring_class: interface.name.to_ascii_lowercase(), is_final: false,
                type_name: constant.type_name.clone(),
                attributes: compile_constant_attributes(&constant.attributes, None, program)?,
            });
        }
        let attributes = interface.attributes.iter().map(|attribute| {
            let arguments = attribute.arguments.iter().map(|argument| match argument {
                php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, None, Some(program))?)),
                value => Ok((None, constant_value_in_class(value, None, Some(program))?)),
            }).collect::<Result<Vec<_>, String>>()?;
            Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
        }).collect::<Result<Vec<_>, String>>()?;
        let mut interface_methods = HashMap::new();
        let mut interface_method_order = Vec::new();
        let mut interface_method_display_names = HashMap::new();
        let mut interface_method_attributes = HashMap::new();
        let mut interface_parameter_attributes = HashMap::new();
        for method in &interface.methods {
            let key = method.name.to_ascii_lowercase();
            interface_method_order.push(key.clone());
            interface_method_display_names.insert(key.clone(), method.name.clone());
            let mut compiled = BytecodeFunction::new(
                key.clone(), method.parameters.clone(), method.parameter_types.clone(), method.parameter_by_reference.clone(),
                method.parameter_variadic.clone(), super::parameter_defaults(&method.parameter_defaults, Some(program), None, None)?,
                method.return_type.clone(), method.returns_by_reference, Vec::new(), method.is_static, true, false,
                compile_visibility(method.visibility), Some(interface.name.to_ascii_lowercase()),
            );
            compiled.parameter_default_constant_names = super::parameter_default_constant_names(&method.parameter_defaults);
            compiled.strict_types = super::strict_mode_for_body(&method.body, program.strict_types);
            interface_methods.insert(key, compiled);
            if !method.attributes.is_empty() {
                let attributes = method.attributes.iter().map(|attribute| {
                    let arguments = attribute.arguments.iter().map(|argument| match argument {
                        php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, None, Some(program))?)),
                        value => Ok((None, constant_value_in_class(value, None, Some(program))?)),
                    }).collect::<Result<Vec<_>, String>>()?;
                    Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
                }).collect::<Result<Vec<_>, String>>()?;
                interface_method_attributes.insert(method.name.to_ascii_lowercase(), attributes);
            }
            let attributes = method.parameter_attributes.iter().map(|parameter| parameter.iter().map(|attribute| {
                let arguments = attribute.arguments.iter().map(|argument| match argument {
                    php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, None, Some(program))?)),
                    value => Ok((None, constant_value_in_class(value, None, Some(program))?)),
                }).collect::<Result<Vec<_>, String>>()?;
                Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
            }).collect::<Result<Vec<_>, String>>()).collect::<Result<Vec<_>, String>>()?;
            if attributes.iter().any(|attributes| !attributes.is_empty()) {
                interface_parameter_attributes.insert(method.name.to_ascii_lowercase(), attributes);
            }
        }
        classes.insert(interface.name.to_ascii_lowercase(), BytecodeClass {
            name: interface.name.clone(),
            reflection_name: interface.name.clone(),
            attributes,
            parent: None, interfaces: interface.parents.iter().map(|name| name.to_ascii_lowercase()).collect(), traits: Vec::new(), trait_aliases: Vec::new(),
            is_interface: true, is_trait: false, unresolved_traits: Vec::new(), is_abstract: true, is_final: false, is_readonly: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(),
            static_properties: HashMap::new(), constants, constant_order: interface.constants.iter().map(|constant| constant.name.clone()).collect(), methods: interface_methods, method_order: interface_method_order, method_attributes: interface_method_attributes, method_display_names: interface_method_display_names, property_attributes: HashMap::new(), parameter_attributes: interface_parameter_attributes,
        });
    }
    for class in classes.values() {
        for (constant_name, constant) in &class.constants {
            if let Some(type_name) = &constant.type_name {
                if let Some(unsupported) = unsupported_constant_type(type_name) {
                    return Err(format!("Class constant {}::{} cannot have type {}", class.name, constant_name, unsupported));
                }
                if !constant_value_matches_type(&constant.value, type_name, &classes) {
                    return Err(format!("Cannot assign {} to class constant {}::{} of type {}", constant_value_type_name(&constant.value).to_ascii_lowercase(), class.name, constant_name, type_name));
                }
            }
        }
    }
    for class in &program.classes {
        let mut available = HashMap::new();
        collect_class_methods(&class.name.to_ascii_lowercase(), &classes, &mut available)?;
        if class.is_abstract && class.is_final { return Err(format!("Class {} cannot be both abstract and final", class.name)); }
        if !class.is_abstract {
            if let Some((method, _)) = available.iter().find(|(_, function)| function.is_abstract) {
                return Err(format!("Class {} must be abstract or implement {}()", class.name, method));
            }
        }
        if let Some(parent_name) = &class.parent {
            let parent = classes.get(&parent_name.to_ascii_lowercase()).ok_or_else(|| format!("Class '{parent_name}' not found"))?;
            if parent.is_interface { return Err(format!("Class {} cannot extend interface {}", class.name, parent_name)); }
            if parent.is_final { return Err(format!("Class {} cannot extend final class {}", class.name, parent_name)); }
            if class.is_readonly != parent.is_readonly {
                return Err(if parent.is_readonly {
                    format!("Non-readonly class {} cannot extend readonly class {}", class.name, parent.name)
                } else {
                    format!("Readonly class {} cannot extend non-readonly class {}", class.name, parent.name)
                });
            }
            let mut inherited = HashMap::new();
            collect_class_methods(&parent_name.to_ascii_lowercase(), &classes, &mut inherited)?;
            let compiled_class = classes.get(&class.name.to_ascii_lowercase()).expect("class metadata was created above");
            for (method_name, method) in &compiled_class.methods {
                if let Some(parent_method) = inherited.get(method_name) {
                    if parent_method.visibility == MemberVisibility::Private
                        && !(method_name.eq_ignore_ascii_case("__construct") && parent_method.is_final)
                    {
                        continue;
                    }
                    if parent_method.is_final {
                        return Err(format!("Class {} cannot override final method {}()", class.name, method_name));
                    }
                    if visibility_rank(method.visibility) < visibility_rank(parent_method.visibility) {
                        return Err(format!("Method {}::{}() cannot reduce inherited visibility", class.name, method_name));
                    }
                    if !method_name.eq_ignore_ascii_case("__construct") && parent_method.visibility != MemberVisibility::Private {
                        let method_has_default = method.parameter_defaults.iter().map(Option::is_some).collect::<Vec<_>>();
                        let parent_has_default = parent_method.parameter_defaults.iter().map(Option::is_some).collect::<Vec<_>>();
                        let parameter_map = parameter_compatibility_map(
                            &method.parameter_by_reference, &method.parameter_variadic, &method_has_default,
                            &parent_method.parameter_by_reference, &parent_method.parameter_variadic, &parent_has_default,
                        );
                        if parameter_map.is_none()
                            || method.returns_by_reference != parent_method.returns_by_reference
                            || !signature_compatible(
                            &method.parameter_types, method.return_type.as_deref(),
                            &parent_method.parameter_types, parameter_map.as_deref().unwrap_or(&[]), parent_method.return_type.as_deref(),
                            &class_declarations, &interface_declarations,
                        ) {
                            return Err(format!("Method {}::{}() has incompatible parameter or return types with parent", class.name, method_name));
                        }
                    }
                }
            }
            for property in &class.properties {
                if let Some(parent_property) = find_property(&parent_name.to_ascii_lowercase(), &property.name, &classes) {
                    if parent_property.0.visibility != MemberVisibility::Private {
                        if property.is_static != parent_property.1 {
                            return Err(format!("Property {}::${} cannot change staticness", class.name, property.name));
                        }
                        if visibility_rank(compile_visibility(property.visibility)) < visibility_rank(parent_property.0.visibility) {
                            return Err(format!("Property {}::${} cannot reduce inherited visibility", class.name, property.name));
                        }
                        if property.is_readonly != parent_property.0.is_readonly {
                            return Err(format!("Property {}::${} cannot change readonly status", class.name, property.name));
                        }
                        if parent_property.0.set_visibility == Some(MemberVisibility::Private) {
                            return Err(format!("Property {}::${} cannot override final private(set) property", class.name, property.name));
                        }
                        let child_set_visibility = property.set_visibility.map(compile_visibility).unwrap_or_else(|| compile_visibility(property.visibility));
                        let parent_set_visibility = parent_property.0.set_visibility.unwrap_or(parent_property.0.visibility);
                        if visibility_rank(child_set_visibility) < visibility_rank(parent_set_visibility) {
                            return Err(format!("Property {}::${} cannot reduce inherited set visibility", class.name, property.name));
                        }
                        let child_type = property_type_in_class(property.type_name.as_deref(), class);
                        if !option_type_names_equal(child_type.as_deref(), parent_property.0.type_name.as_deref()) {
                            return Err(format!("Property {}::${} cannot change inherited type", class.name, property.name));
                        }
                    }
                }
            }
            for constant in &class.constants {
                if find_constant(&parent_name.to_ascii_lowercase(), &constant.name, &classes).is_some_and(|parent_constant| parent_constant.is_final) {
                    return Err(format!("Class {} cannot override final class constant {}", class.name, constant.name));
                }
            }
        }
        // Interface requirements are inherited along the class chain, including abstract parents.
        let mut implemented_interfaces = Vec::new();
        collect_class_interfaces(class, &class_declarations, &mut implemented_interfaces)?;
        for interface_name in &implemented_interfaces {
            let Some(interface) = interface_declarations.get(&interface_name.to_ascii_lowercase()) else {
                if builtin_interfaces.iter().any(|name| interface_name.eq_ignore_ascii_case(name)) { continue; }
                return Err(format!("Interface '{interface_name}' not found"));
            };
            let mut requirements = HashMap::new();
            collect_interface_methods(interface, &interface_declarations, &mut requirements)?;
            for (method_name, signature) in requirements {
                if method_name.eq_ignore_ascii_case("__construct") { continue; }
                let implementation = available.get(&method_name).ok_or_else(|| format!("Class {} must implement {}() from interface {}", class.name, method_name, interface_name))?;
                let implementation_has_default = implementation.parameter_defaults.iter().map(Option::is_some).collect::<Vec<_>>();
                let signature_has_default = signature.parameter_defaults.iter().map(Option::is_some).collect::<Vec<_>>();
                let parameter_map = parameter_compatibility_map(
                    &implementation.parameter_by_reference, &implementation.parameter_variadic, &implementation_has_default,
                    &signature.parameter_by_reference, &signature.parameter_variadic, &signature_has_default,
                );
                if implementation.is_static != signature.is_static
                    || parameter_map.is_none()
                    || implementation.returns_by_reference != signature.returns_by_reference
                    || !signature_compatible(
                        &implementation.parameter_types, implementation.return_type.as_deref(),
                        &signature.parameter_types, parameter_map.as_deref().unwrap_or(&[]), signature.return_type.as_deref(),
                        &class_declarations, &interface_declarations,
                    ) {
                    return Err(format!("Method {}::{}() does not match interface {} signature", class.name, method_name, interface_name));
                }
                if signature.visibility == Visibility::Public && implementation.visibility != MemberVisibility::Public {
                    return Err(format!("Method {}::{}() must be public to implement interface {}", class.name, method_name, interface_name));
                }
            }
        }
        if !class.is_abstract {
            for interface_name in &implemented_interfaces {
                let required: &[&str] = if interface_name.eq_ignore_ascii_case("Iterator") {
                    &["rewind", "valid", "current", "key", "next"]
                } else if interface_name.eq_ignore_ascii_case("IteratorAggregate") {
                    &["getIterator"]
                } else if interface_name.eq_ignore_ascii_case("Countable") {
                    &["count"]
                } else if interface_name.eq_ignore_ascii_case("JsonSerializable") {
                    &["jsonSerialize"]
                } else if interface_name.eq_ignore_ascii_case("Stringable") {
                    &["__toString"]
                } else { &[] };
                for method_name in required {
                    let method = available.get(&method_name.to_ascii_lowercase()).ok_or_else(||
                        format!("Class {} must implement {}() from interface {}", class.name, method_name, interface_name))?;
                    if method.is_static || method.visibility != MemberVisibility::Public {
                        return Err(format!("Class {} must implement {}::{}() as a public instance method", class.name, interface_name, method_name));
                    }
                    if interface_name.eq_ignore_ascii_case("Countable")
                        && (method.parameters.iter().enumerate().any(|(index, _)| {
                                !method.parameter_variadic.get(index).copied().unwrap_or(false)
                                    && method.parameter_defaults.get(index).is_none_or(Option::is_none)
                            })
                            || method.returns_by_reference
                            || method.return_type.as_deref().is_some_and(|ty| !ty.eq_ignore_ascii_case("int"))) {
                        return Err(format!("Class {} must implement Countable::count(): int without required parameters", class.name));
                    }
                if interface_name.eq_ignore_ascii_case("JsonSerializable") && !method.parameters.is_empty() {
                    return Err(format!("Class {} must implement JsonSerializable::jsonSerialize() without parameters", class.name));
                }
                    if interface_name.eq_ignore_ascii_case("Stringable")
                        && (!method.parameters.is_empty() || !method.return_type.as_deref().is_some_and(|ty| ty.eq_ignore_ascii_case("string"))) {
                        return Err(format!("Class {} must implement Stringable::__toString(): string without parameters", class.name));
                    }
                }
            }
        }
    }
    Ok(classes)
}

/// PHP 8.5 rejects using the built-in Attribute marker on abstract or non-class declarations.
fn validate_attribute_class_declarations(program: &Program) -> Result<(), String> {
    let is_attribute_marker = |attributes: &[php_syntax::AttributeDecl]| attributes.iter().any(|attribute| {
        attribute.name.trim_start_matches('\\').eq_ignore_ascii_case("Attribute")
    });
    for class in &program.classes {
        if !is_attribute_marker(&class.attributes) { continue; }
        let message = if class.is_enum {
            Some(format!("Cannot apply #[\\Attribute] to enum {}", class.name))
        } else if class.is_abstract {
            Some(format!("Cannot apply #[\\Attribute] to abstract class {}", class.name))
        } else { None };
        if let Some(message) = message { return Err(message); }
    }
    for interface in &program.interfaces {
        if is_attribute_marker(&interface.attributes) {
            return Err(format!("Cannot apply #[\\Attribute] to interface {}", interface.name));
        }
    }
    for trait_decl in &program.traits {
        if is_attribute_marker(&trait_decl.attributes) {
            return Err(format!("Cannot apply #[\\Attribute] to trait {}", trait_decl.name));
        }
    }
    Ok(())
}

