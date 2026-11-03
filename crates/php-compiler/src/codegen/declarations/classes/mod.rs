//! Class metadata resolution, OOP validation, and method bytecode generation.

use std::collections::HashMap;

use php_runtime::{bytecode::Op, BytecodeAttribute, BytecodeClass, BytecodeConstant, BytecodeFunction, BytecodeProperty, MemberVisibility, Value};
use php_syntax::{FunctionDecl, Program, Visibility};

use super::CodeGenerator;

mod compatibility;
mod metadata;

use compatibility::*;
use metadata::*;

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
        current_function: Some(name.clone()), strict_types, returns_by_reference: false,
        control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), include_return_stack: Vec::new(), errors: Vec::new(),
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
    seen.push(key);
    let mut methods = Vec::new();
    for nested_name in &declaration.traits {
        let nested = expanded_trait_methods(nested_name, declarations, seen)?;
        methods.extend(nested.iter().cloned());
    }
    // Apply this trait's adaptations against methods from its direct nested
    // traits, retaining each method's declaring trait for outer conflict rules.
    for adaptation in &declaration.trait_adaptations {
        methods.retain(|(origin, method)| !(adaptation.method_name.eq_ignore_ascii_case(&method.name)
            && adaptation.excluded_traits.iter().any(|excluded| excluded.eq_ignore_ascii_case(origin))));
        if let Some((_, source)) = methods.iter().find(|(origin, method)| origin.eq_ignore_ascii_case(&adaptation.trait_name)
            && method.name.eq_ignore_ascii_case(&adaptation.method_name)) {
            let mut adapted = source.clone();
            if let Some(visibility) = adaptation.visibility { adapted.visibility = visibility; }
            if let Some(alias) = &adaptation.alias {
                adapted.name = alias.clone();
                methods.push((adaptation.trait_name.to_ascii_lowercase(), adapted));
            } else if adaptation.visibility.is_some() {
                if let Some((_, method)) = methods.iter_mut().find(|(origin, method)| origin.eq_ignore_ascii_case(&adaptation.trait_name)
                    && method.name.eq_ignore_ascii_case(&adaptation.method_name)) { method.visibility = adapted.visibility; }
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
    seen.pop();
    Ok(methods)
}

/// Resolve traits, compile methods, assemble class metadata, and check interfaces.
pub(super) fn compile(program: &Program) -> Result<HashMap<String, BytecodeClass>, String> {
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
    let interface_declarations = program.interfaces.iter().map(|declaration| (declaration.name.to_ascii_lowercase(), declaration)).collect::<HashMap<_, _>>();
    let class_declarations = program.classes.iter().map(|declaration| (declaration.name.to_ascii_lowercase(), declaration)).collect::<HashMap<_, _>>();
    let mut classes = HashMap::new();
    for trait_decl in &program.traits {
        classes.insert(trait_decl.name.to_ascii_lowercase(), BytecodeClass {
            name: trait_decl.name.clone(), attributes: Vec::new(), parent: None, interfaces: Vec::new(), is_interface: false, is_trait: true, unresolved_traits: Vec::new(),
            is_abstract: false, is_final: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(),
            static_properties: HashMap::new(), constants: HashMap::new(), methods: HashMap::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
        });
    }
    classes.insert("throwable".into(), BytecodeClass {
        name: "Throwable".into(), attributes: Vec::new(), parent: None, interfaces: Vec::new(), is_interface: true, is_trait: false, unresolved_traits: Vec::new(),
        is_abstract: true, is_final: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(), static_properties: HashMap::new(),
        constants: HashMap::new(), methods: HashMap::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
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
            name: canonical.into(), attributes: Vec::new(), parent: None, interfaces: parents, is_interface: true, is_trait: false, unresolved_traits: Vec::new(), is_abstract: true,
            is_final: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(), static_properties: HashMap::new(), constants: HashMap::new(), methods: HashMap::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
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
            name: canonical_name.into(), attributes: Vec::new(), parent: parent.map(str::to_string), interfaces: Vec::new(),
            is_interface: false, is_trait: false, unresolved_traits: Vec::new(), is_abstract: false, is_final: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(),
            static_properties: HashMap::new(), constants: HashMap::new(), methods: HashMap::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
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
        });
    }
    classes.insert("roundingmode".into(), BytecodeClass {
        name: "RoundingMode".into(), attributes: Vec::new(), parent: None, interfaces: vec!["unitenum".into()], is_interface: false, is_trait: false, unresolved_traits: Vec::new(),
        is_abstract: false, is_final: true, is_enum: true, enum_backing_type: None,
        enum_cases: rounding_cases.iter().map(|case| (*case).to_string()).collect(),
        properties: HashMap::new(), property_order: Vec::new(), static_properties: HashMap::new(), constants: rounding_constants, methods: HashMap::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
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
        sort_constants.insert(case.to_string(), BytecodeConstant { value, visibility: MemberVisibility::Public, declaring_class: "sortdirection".into(), is_final: true });
    }
    classes.insert("sortdirection".into(), BytecodeClass {
        name: "SortDirection".into(), attributes: Vec::new(), parent: None, interfaces: vec!["unitenum".into()], is_interface: false, is_trait: false, unresolved_traits: Vec::new(),
        is_abstract: false, is_final: true, is_enum: true, enum_backing_type: None,
        enum_cases: sort_cases.iter().map(|case| (*case).to_string()).collect(), properties: HashMap::new(), property_order: Vec::new(),
        static_properties: HashMap::new(), constants: sort_constants, methods: HashMap::new(), method_attributes: HashMap::new(),
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
        let mut static_properties = HashMap::new();
        let mut constants = HashMap::new();
        let mut enum_cases = Vec::new();
        let mut methods = HashMap::new();
        let mut trait_methods: HashMap<String, Vec<(String, FunctionDecl)>> = HashMap::new();
        let is_anonymous = class.name.starts_with("class@anonymous#");
        let mut unresolved_traits = Vec::new();
        // Trait members are inserted first so explicit class members can override them.
        for trait_name in &class.traits {
            let Some(trait_decl) = trait_declarations.get(&trait_name.to_ascii_lowercase()) else {
                if is_anonymous {
                    unresolved_traits.push(trait_name.clone());
                    continue;
                }
                return Err(format!("Trait '{trait_name}' not found"));
            };
            for constant in &trait_decl.constants {
                let value = constant_value_in_class(&constant.value, Some(class), Some(program))?;
                constants.entry(constant.name.clone()).or_insert_with(|| BytecodeConstant {
                    value,
                    visibility: compile_visibility(constant.visibility),
                    declaring_class: class.name.to_ascii_lowercase(),
                    is_final: constant.is_final,
                });
            }
        for property in &trait_decl.properties {
            trait_properties.insert(property.name.to_ascii_lowercase());
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
                trait_methods.entry(method_name).or_default().push(method.clone());
            }
        }
        // Resolve each trait method after reading all traits, applying `insteadof` rules.
        for (method_name, candidates) in trait_methods {
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
                methods.insert(method_name.clone(), (*method).clone());
            }
        }
        // Apply `as` visibility changes and aliases. An alias keeps the original method too.
        for adaptation in &class.trait_adaptations {
            if unresolved_traits.iter().any(|name| name.eq_ignore_ascii_case(&adaptation.trait_name)) { continue; }
            // Adaptations may name a method supplied by a nested trait, so
            // resolve against the composed method set rather than only the
            // methods declared directly on the named trait.
            let source = expanded_trait_methods(&adaptation.trait_name, &trait_declarations, &mut Vec::new())?
                .into_iter().find(|(_, method)| method.name.eq_ignore_ascii_case(&adaptation.method_name)).map(|(_, method)| method)
                .ok_or_else(|| format!("Trait method {}::{} not found", adaptation.trait_name, adaptation.method_name))?;
            let mut adapted = source;
            if let Some(visibility) = adaptation.visibility { adapted.visibility = visibility; }
            if let Some(alias) = &adaptation.alias {
                adapted.name = alias.clone();
                methods.insert(alias.to_ascii_lowercase(), adapted);
            } else {
                methods.insert(adaptation.method_name.to_ascii_lowercase(), adapted);
            }
        }
        for property in &class.properties {
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
            let typed_magic_signature: Option<(&str, Option<&str>)> = match magic.as_str() {
                "__serialize" | "__sleep" | "__debuginfo" => Some(("array", Some("array"))),
                "__unserialize" => Some(("array", Some("void"))),
                "__wakeup" => Some(("void", None)),
                "__set_state" => Some(("array", Some("object"))),
                _ => None,
            };
            if let Some((parameter_type, return_type)) = typed_magic_signature {
                if method.parameter_types.first().and_then(Option::as_deref)
                    .is_some_and(|actual| !actual.eq_ignore_ascii_case(parameter_type))
                {
                    return Err(format!("{}::{}() must accept a {parameter_type} parameter", class.name, method.name));
                }
                if let Some(expected_return) = return_type {
                    if method.return_type.as_deref().is_some_and(|actual|
                        !actual.eq_ignore_ascii_case(expected_return) && !actual.eq_ignore_ascii_case("never"))
                    {
                        return Err(format!("{}::{}() must return {expected_return}", class.name, method.name));
                    }
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
                if properties.contains_key(&property.name) || static_properties.contains_key(&property.name) {
                    let existing = properties.get(&property.name);
                    let compatible_trait_property = existing.is_some_and(|existing| {
                        trait_properties.contains(&property.name.to_ascii_lowercase())
                            && existing.visibility == compile_visibility(property.visibility)
                            && existing.set_visibility == property.set_visibility.map(compile_visibility)
                            && existing.is_readonly == property.is_readonly
                            && existing.type_name.as_deref().zip(property_type_in_class(property.type_name.as_deref(), class).as_deref())
                                .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
                    });
                    if !compatible_trait_property {
                        return Err(format!("Promoted property ${} conflicts with a declared property", property.name));
                    }
                    continue;
                }
                properties.insert(property.name.clone(), BytecodeProperty {
                    default: class_property_default(property, class, program)?,
                    visibility: compile_visibility(property.visibility),
                    set_visibility: property.set_visibility.map(compile_visibility),
                    declaring_class: class.name.to_ascii_lowercase(),
                    type_name: property_type_in_class(property.type_name.as_deref(), class),
                    is_readonly: property.is_readonly,
                    get_hook: None,
                    set_hook: None,
                });
                property_order.push(property.name.clone());
            }
        }
        for constant in &class.constants {
            if constants.contains_key(&constant.name) {
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
            });
        }
        for method in &class.methods {
            methods.insert(method.name.to_ascii_lowercase(), method.clone());
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
            let mut generator = CodeGenerator { program: Some(program), instructions: Vec::new(), current_source_file: None, parent_class: class.parent.as_ref().map(|name| name.to_ascii_lowercase()), current_class: Some(class.name.to_ascii_lowercase()), current_class_display: Some(class.name.clone()), current_method: Some(method.name.to_ascii_lowercase()), current_function: Some(method.name.clone()), strict_types, returns_by_reference: method.returns_by_reference, control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), include_return_stack: Vec::new(), errors: Vec::new() };
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
        classes.insert(class.name.to_ascii_lowercase(), BytecodeClass { name: class.name.clone(), attributes, parent: class.parent.as_ref().map(|name| name.to_ascii_lowercase()), interfaces: implemented_interfaces, is_interface: false, is_trait: false, unresolved_traits, is_abstract: class.is_abstract, is_final: class.is_final || class.is_enum, is_enum: class.is_enum, enum_backing_type: class.enum_backing_type.clone(), enum_cases, properties, property_order, static_properties, constants, methods: compiled_methods, method_attributes, method_display_names, property_attributes, parameter_attributes });
    }
    // Interfaces live in the runtime class table so `InterfaceName::CONSTANT` resolves normally.
    for interface in &program.interfaces {
        let mut constants = HashMap::new();
        for constant in &interface.constants {
            constants.insert(constant.name.clone(), BytecodeConstant {
                value: constant_value(&constant.value)?, visibility: MemberVisibility::Public,
                declaring_class: interface.name.to_ascii_lowercase(), is_final: false,
            });
        }
        let attributes = interface.attributes.iter().map(|attribute| {
            let arguments = attribute.arguments.iter().map(|argument| match argument {
                php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), constant_value_in_class(value, None, Some(program))?)),
                value => Ok((None, constant_value_in_class(value, None, Some(program))?)),
            }).collect::<Result<Vec<_>, String>>()?;
            Ok(BytecodeAttribute { name: attribute.name.clone(), arguments })
        }).collect::<Result<Vec<_>, String>>()?;
        classes.insert(interface.name.to_ascii_lowercase(), BytecodeClass {
            name: interface.name.clone(),
            attributes,
            parent: None, interfaces: interface.parents.iter().map(|name| name.to_ascii_lowercase()).collect(),
            is_interface: true, is_trait: false, unresolved_traits: Vec::new(), is_abstract: true, is_final: false, is_enum: false, enum_backing_type: None, enum_cases: Vec::new(), properties: HashMap::new(), property_order: Vec::new(),
            static_properties: HashMap::new(), constants, methods: HashMap::new(), method_attributes: HashMap::new(), method_display_names: HashMap::new(), property_attributes: HashMap::new(), parameter_attributes: HashMap::new(),
        });
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
            let mut inherited = HashMap::new();
            collect_class_methods(&parent_name.to_ascii_lowercase(), &classes, &mut inherited)?;
            let compiled_class = classes.get(&class.name.to_ascii_lowercase()).expect("class metadata was created above");
            for (method_name, method) in &compiled_class.methods {
                if let Some(parent_method) = inherited.get(method_name) {
                    if parent_method.is_final {
                        return Err(format!("Class {} cannot override final method {}()", class.name, method_name));
                    }
                    if visibility_rank(method.visibility) < visibility_rank(parent_method.visibility) {
                        return Err(format!("Method {}::{}() cannot reduce inherited visibility", class.name, method_name));
                    }
                    if method.parameter_variadic != parent_method.parameter_variadic
                        || method.parameter_by_reference != parent_method.parameter_by_reference
                        || method.returns_by_reference != parent_method.returns_by_reference
                        || !signature_compatible(
                        &method.parameter_types, method.return_type.as_deref(),
                        &parent_method.parameter_types, parent_method.return_type.as_deref(),
                        &class_declarations, &interface_declarations,
                    ) {
                        return Err(format!("Method {}::{}() has incompatible parameter or return types with parent", class.name, method_name));
                    }
                }
            }
            for property in &class.properties {
                if let Some(parent_property) = find_property(&parent_name.to_ascii_lowercase(), &property.name, &classes) {
                    if property.is_static != parent_property.1 {
                        return Err(format!("Property {}::${} cannot change staticness", class.name, property.name));
                    }
                    if visibility_rank(compile_visibility(property.visibility)) < visibility_rank(parent_property.0.visibility) {
                        return Err(format!("Property {}::${} cannot reduce inherited visibility", class.name, property.name));
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
                let implementation = available.get(&method_name).ok_or_else(|| format!("Class {} must implement {}() from interface {}", class.name, method_name, interface_name))?;
                if implementation.is_static != signature.is_static
                    || implementation.parameters.len() != signature.parameters.len()
                    || implementation.parameter_variadic != signature.parameter_variadic
                    || implementation.parameter_by_reference != signature.parameter_by_reference
                    || implementation.returns_by_reference != signature.returns_by_reference
                    || !signature_compatible(
                        &implementation.parameter_types, implementation.return_type.as_deref(),
                        &signature.parameter_types, signature.return_type.as_deref(),
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

