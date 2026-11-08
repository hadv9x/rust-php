//! Bytecode generation for PHP expressions.

use php_runtime::{bytecode::{Op, ReferenceDescriptor}, Value};
use php_syntax::{BinaryOp, Expr};

use super::CodeGenerator;

fn reference_path(expression: &Expr) -> Option<(String, Vec<&Expr>)> {
    match expression {
        Expr::Variable(variable) => Some((variable.clone(), Vec::new())),
        Expr::Index(array, key) => {
            let (variable, mut keys) = reference_path(array)?;
            keys.push(key);
            Some((variable, keys))
        }
        Expr::NamedArgument(_, value) => reference_path(value),
        _ => None,
    }
}

pub(super) fn static_property_path(expression: &Expr) -> Option<(&str, &str, Vec<&Expr>)> {
    fn split<'a>(expression: &'a Expr, keys: &mut Vec<&'a Expr>) -> Option<(&'a str, &'a str)> {
        match expression {
            Expr::StaticProperty(class, property) => Some((class, property)),
            Expr::Index(array, key) => {
                let root = split(array, keys)?;
                keys.push(key);
                Some(root)
            }
            Expr::NamedArgument(_, value) => split(value, keys),
            _ => None,
        }
    }
    let mut keys = Vec::new();
    let (class, property) = split(expression, &mut keys)?;
    Some((class, property, keys))
}

fn object_property_path(expression: &Expr) -> Option<(&Expr, String, Vec<&Expr>)> {
    fn split<'a>(expression: &'a Expr, keys: &mut Vec<&'a Expr>) -> Option<(&'a Expr, String)> {
        match expression {
            Expr::Property(receiver, property) => Some((receiver, property.clone())),
            Expr::Index(array, key) => {
                let root = split(array, keys)?;
                keys.push(key);
                Some(root)
            }
            Expr::NamedArgument(_, value) => split(value, keys),
            _ => None,
        }
    }
    let mut keys = Vec::new();
    let (receiver, property) = split(expression, &mut keys)?;
    Some((receiver, property, keys))
}

fn dynamic_object_property_path(expression: &Expr) -> Option<(&Expr, &Expr, Vec<&Expr>)> {
    fn split<'a>(expression: &'a Expr, keys: &mut Vec<&'a Expr>) -> Option<(&'a Expr, &'a Expr)> {
        match expression {
            Expr::DynamicProperty(object, property) => Some((object, property)),
            Expr::Index(array, key) => {
                let root = split(array, keys)?;
                keys.push(key);
                Some(root)
            }
            _ => None,
        }
    }
    let mut keys = Vec::new();
    let (object, property) = split(expression, &mut keys)?;
    Some((object, property, keys))
}

fn isset_operand(generator: &mut CodeGenerator<'_>, expression: &Expr) {
    match expression {
        Expr::Variable(variable) => generator.instructions.push(Op::LoadSilent(variable.clone())),
        Expr::DynamicVariable(variable) => generator.instructions.push(Op::LoadDynamicSilent(variable.clone())),
        Expr::DynamicVariableExpr(name) => {
            generator.expression(name);
            generator.instructions.push(Op::LoadDynamicExprSilent);
        }
        Expr::Index(array, key) => {
            isset_operand(generator, array);
            generator.expression(key);
            generator.instructions.push(Op::ArrayGetIsset);
        }
        _ => generator.expression(expression),
    }
}

fn coalesce_operand(generator: &mut CodeGenerator<'_>, expression: &Expr) {
    match expression {
        Expr::Variable(variable) => generator.instructions.push(Op::LoadSilent(variable.clone())),
        Expr::DynamicVariable(variable) => generator.instructions.push(Op::LoadDynamicSilent(variable.clone())),
        Expr::DynamicVariableExpr(name) => {
            generator.expression(name);
            generator.instructions.push(Op::LoadDynamicExprSilent);
        }
        Expr::Index(array, key) => {
            coalesce_operand(generator, array);
            generator.expression(key);
            generator.instructions.push(Op::ArrayGetSilent);
        }
        Expr::Property(receiver, property) => {
            generator.expression(receiver);
            generator.instructions.push(Op::GetPropertyCoalesce(property.clone(), generator.current_class.clone()));
        }
        Expr::DynamicProperty(receiver, property) => {
            generator.expression(receiver);
            generator.expression(property);
            generator.instructions.push(Op::GetDynamicPropertyCoalesce(generator.current_class.clone()));
        }
        Expr::StaticProperty(class, property) => generator.instructions.push(Op::GetStaticPropertyCoalesce(
            class.clone(), property.clone(), generator.current_class.clone(),
        )),
        _ => generator.expression(expression),
    }
}

fn argument_metadata(arguments: &[Expr], access_scope: Option<String>) -> (Vec<Option<ReferenceDescriptor>>, Vec<Option<String>>, Vec<bool>) {
    let references = arguments.iter().map(|argument| {
        if matches!(argument, Expr::UnpackArgument(_)) { return None; }
        let argument = match argument { Expr::NamedArgument(_, value) => value.as_ref(), value => value };
        if let Some((variable, keys)) = reference_path(argument) {
            Some(ReferenceDescriptor { variable, array_key_count: keys.len(), object_property: None, access_scope: access_scope.clone(), direct_reference: false })
        } else if let Some((_, property, keys)) = object_property_path(argument) {
            Some(ReferenceDescriptor { variable: String::new(), array_key_count: keys.len(), object_property: Some(property), access_scope: access_scope.clone(), direct_reference: false })
        } else if let Some((_, _, keys)) = static_property_path(argument) {
            Some(ReferenceDescriptor { variable: String::new(), array_key_count: keys.len(), object_property: None, access_scope: access_scope.clone(), direct_reference: true })
        } else {
            None
        }
    }).collect();
    let names = arguments.iter().map(|argument| match argument {
        Expr::NamedArgument(name, _) => Some(name.clone()),
        _ => None,
    }).collect();
    let unpack = arguments.iter().map(|argument| matches!(argument, Expr::UnpackArgument(_))).collect();
    (references, names, unpack)
}

fn passed_whole_globals_by_reference(name: &str, arguments: &[Expr], program: Option<&php_syntax::Program>) -> bool {
    let global_arguments = arguments.iter().enumerate().filter_map(|(index, argument)| {
        let (value, name) = match argument {
            Expr::NamedArgument(name, value) => (value.as_ref(), Some(name.as_str())),
            Expr::UnpackArgument(_) => return None,
            value => (value, None),
        };
        matches!(value, Expr::Variable(variable) if variable == "GLOBALS").then_some((index, name))
    }).collect::<Vec<_>>();
    if global_arguments.is_empty() { return false; }
    let normalized = name.trim_start_matches('\\').to_ascii_lowercase();
    let builtin_reference_indices: &[usize] = match normalized.as_str() {
        "array_pop" | "array_shift" | "array_push" | "array_unshift" | "array_splice" | "array_walk" | "array_walk_recursive"
        | "sort" | "rsort" | "asort" | "arsort" | "ksort" | "krsort" | "natsort" | "natcasesort" | "shuffle"
        | "usort" | "uasort" | "uksort" | "next" | "prev" | "reset" | "end" | "each" => &[0],
        "extract" => &[0],
        "array_multisort" => return true,
        _ => &[],
    };
    if global_arguments.iter().any(|(index, name)| builtin_reference_indices.contains(index) || name.is_some_and(|name| name.eq_ignore_ascii_case("array"))) { return true; }
    let function = program.and_then(|program| program.functions.iter().find(|function| {
        function.name.trim_start_matches('\\').eq_ignore_ascii_case(&normalized)
    }));
    function.is_some_and(|function| global_arguments.iter().any(|(index, name)| {
        let parameter_index = name.and_then(|name| function.parameters.iter().position(|parameter| parameter == name)).unwrap_or(*index);
        function.parameter_by_reference.get(parameter_index).copied().unwrap_or(false)
    }))
}

fn declared_method<'a>(
    program: &'a php_syntax::Program,
    class_name: &str,
    method_name: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Option<&'a php_syntax::FunctionDecl> {
    let normalized_class = class_name.trim_start_matches('\\').to_ascii_lowercase();
    if !seen.insert(normalized_class.clone()) { return None; }
    if let Some(class) = program.classes.iter().find(|class| class.name.eq_ignore_ascii_case(&normalized_class)) {
        if let Some(method) = class.methods.iter().find(|method| method.name.eq_ignore_ascii_case(method_name)) { return Some(method); }
        for trait_name in &class.traits {
            if let Some(method) = declared_method(program, trait_name, method_name, seen) { return Some(method); }
        }
        if let Some(parent) = &class.parent { return declared_method(program, parent, method_name, seen); }
    }
    if let Some(interface) = program.interfaces.iter().find(|interface| interface.name.eq_ignore_ascii_case(&normalized_class)) {
        if let Some(method) = interface.methods.iter().find(|method| method.name.eq_ignore_ascii_case(method_name)) { return Some(method); }
        for parent in &interface.parents {
            if let Some(method) = declared_method(program, parent, method_name, seen) { return Some(method); }
        }
    }
    if let Some(trait_decl) = program.traits.iter().find(|trait_decl| trait_decl.name.eq_ignore_ascii_case(&normalized_class)) {
        if let Some(method) = trait_decl.methods.iter().find(|method| method.name.eq_ignore_ascii_case(method_name)) { return Some(method); }
        for trait_name in &trait_decl.traits {
            if let Some(method) = declared_method(program, trait_name, method_name, seen) { return Some(method); }
        }
    }
    None
}

fn method_passes_whole_globals_by_reference(
    class_name: &str,
    method_name: &str,
    arguments: &[Expr],
    program: Option<&php_syntax::Program>,
) -> bool {
    let Some(program) = program else { return false; };
    let Some(method) = declared_method(program, class_name, method_name, &mut std::collections::HashSet::new()) else { return false; };
    arguments.iter().enumerate().any(|(index, argument)| {
        let (value, name) = match argument {
            Expr::NamedArgument(name, value) => (value.as_ref(), Some(name.as_str())),
            Expr::UnpackArgument(_) => return false,
            value => (value, None),
        };
        if !matches!(value, Expr::Variable(variable) if variable == "GLOBALS") { return false; }
        let parameter_index = name.and_then(|name| method.parameters.iter().position(|parameter| parameter == name)).unwrap_or(index);
        method.parameter_by_reference.get(parameter_index).copied().unwrap_or(false)
    })
}

fn statically_known_method_class(receiver: &Expr, current_class: Option<&str>) -> Option<String> {
    match receiver {
        Expr::NewObject(class, _, _) => Some(class.clone()),
        Expr::Variable(variable) if variable == "this" => current_class.map(str::to_string),
        _ => None,
    }
}

fn internal_method_passes_whole_globals_by_reference(class_name: &str, method: &str, arguments: &[Expr]) -> bool {
    if !class_name.split(['|', '&', '(', ')']).any(|part| {
        part.trim().trim_start_matches('?').trim_start_matches('\\').eq_ignore_ascii_case("PDOStatement")
    }) || !matches!(method.to_ascii_lowercase().as_str(), "bindparam" | "bindcolumn") {
        return false;
    }
    arguments.iter().enumerate().any(|(index, argument)| {
        let (value, name) = match argument {
            Expr::NamedArgument(name, value) => (value.as_ref(), Some(name.as_str())),
            Expr::UnpackArgument(_) => return false,
            value => (value, None),
        };
        matches!(value, Expr::Variable(variable) if variable == "GLOBALS")
            && (name.is_some_and(|name| name.eq_ignore_ascii_case("var")) || name.is_none() && index == 1)
    })
}

pub(super) fn assigned_type(expression: &Expr, variable_types: &std::collections::HashMap<String, String>) -> Option<String> {
    match expression {
        Expr::NewObject(class, _, _) => Some(class.trim_start_matches('\\').to_ascii_lowercase()),
        Expr::Variable(variable) => variable_types.get(variable).cloned(),
        _ => None,
    }
}

fn statically_known_receiver_class(receiver: &Expr, current_class: Option<&str>, variable_types: &std::collections::HashMap<String, String>) -> Option<String> {
    statically_known_method_class(receiver, current_class).or_else(|| match receiver {
        Expr::Variable(variable) => variable_types.get(variable).cloned(),
        _ => None,
    })
}

fn callable_passes_whole_globals_by_reference(
    callable: &Expr,
    arguments: &[Expr],
    program: Option<&php_syntax::Program>,
    current_class: Option<&str>,
    parent_class: Option<&str>,
) -> bool {
    match callable {
        Expr::String(name) => passed_whole_globals_by_reference(name, arguments, program),
        Expr::Array(entries) if entries.len() == 2 => {
            let method = match &entries[1].value { Expr::String(method) => method.as_str(), _ => return false };
            match &entries[0].value {
                Expr::String(class) => method_passes_whole_globals_by_reference(class, method, arguments, program),
                receiver => statically_known_method_class(receiver, current_class).is_some_and(|class| {
                    method_passes_whole_globals_by_reference(&class, method, arguments, program)
                }),
            }
        }
        Expr::FirstClassCallable(target) => match target {
            php_syntax::FirstClassCallableTarget::Function(name) => passed_whole_globals_by_reference(name, arguments, program),
            php_syntax::FirstClassCallableTarget::StaticMethod(class, method) => {
                let class = match class.as_str() {
                    "self" | "static" => current_class,
                    "parent" => parent_class,
                    _ => Some(class.as_str()),
                };
                class.is_some_and(|class| method_passes_whole_globals_by_reference(class, method, arguments, program))
            }
            php_syntax::FirstClassCallableTarget::InstanceMethod(receiver, method) => {
                statically_known_method_class(receiver, current_class).is_some_and(|class| {
                    method_passes_whole_globals_by_reference(&class, method, arguments, program)
                })
            }
            php_syntax::FirstClassCallableTarget::Value(value) => callable_passes_whole_globals_by_reference(value, arguments, program, current_class, parent_class),
        },
        _ => false,
    }
}

impl CodeGenerator<'_> {
    fn call_arguments(&mut self, arguments: &[Expr]) {
        for argument in arguments {
            if let Expr::UnpackArgument(value) = argument {
                self.expression(value);
                self.instructions.push(Op::ExpandUnpack);
                continue;
            }
            let argument = match argument {
                Expr::NamedArgument(_, value) => value.as_ref(),
                value => value,
            };
            if let Some((class, property, keys)) = static_property_path(argument) {
                let class = match class {
                    "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "parent" => self.parent_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "static" => "static".into(),
                    _ => class.to_ascii_lowercase(),
                };
                self.instructions.push(Op::LoadStaticPropertyReference(class, property.to_string(), self.current_class.clone()));
                for key in keys { self.expression(key); }
            } else if let Some((variable, keys)) = reference_path(argument) {
                self.instructions.push(Op::Load(variable));
                for key in keys { self.expression(key); }
            } else if let Some((receiver, _, keys)) = object_property_path(argument) {
                self.expression(receiver);
                for key in keys { self.expression(key); }
            } else {
                self.expression(argument);
            }
        }
    }
}

impl CodeGenerator<'_> {
    /// Emit stack-machine instructions for one expression, leaving one value on the stack.
    pub(super) fn expression(&mut self, expression: &Expr) {
        match expression {
            Expr::Integer(value) => self.instructions.push(Op::Const(Value::Integer(*value))),
            Expr::Number(value) => self.instructions.push(Op::Const(Value::Number(*value))),
            Expr::String(value) => self.instructions.push(Op::Const(Value::String(value.clone()))),
            Expr::DeprecatedInterpolation(value, message) => {
                self.instructions.push(Op::ReportDiagnostic(php_runtime::DiagnosticLevel::Deprecated, message.clone()));
                self.expression(value);
            }
            Expr::Bool(value) => self.instructions.push(Op::Const(Value::Bool(*value))),
            Expr::Null => self.instructions.push(Op::Const(Value::Null)),
            Expr::Constant(name) => self.instructions.push(Op::LoadConstant(name.clone())),
            Expr::MagicConstant(name) => {
                let value = match name.as_str() {
                    "__class__" => self.current_class_display.clone().unwrap_or_default(),
                    "__function__" => self.current_function.clone().unwrap_or_default(),
                    "__method__" => match (&self.current_class_display, &self.current_function) {
                        (Some(class), Some(function)) => format!("{class}::{function}"),
                        (None, Some(function)) if function == "{closure}" => function.clone(),
                        _ => String::new(),
                    },
                    _ => { self.errors.push(format!("Unsupported magic constant {name}")); return; }
                };
                self.instructions.push(Op::Const(Value::String(value)));
            }
            Expr::Variable(name) => self.instructions.push(Op::Load(name.clone())),
            Expr::DynamicVariable(name) => self.instructions.push(Op::LoadDynamic(name.clone())),
            Expr::DynamicVariableExpr(name) => {
                self.expression(name);
                self.instructions.push(Op::LoadDynamicExpr);
            }
            Expr::ListAssignment { targets, value } => {
                self.expression(value);
                let mut target_key_count = 0;
                let targets = targets.iter().map(|target| super::statements::compile_destructure_target(self, target, &mut target_key_count)).collect();
                let source_variable = match value.as_ref() { Expr::Variable(name) => Some(name.clone()), _ => None };
                self.instructions.push(Op::ListDestructureKeep(targets, target_key_count, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct")), source_variable));
            }
            Expr::Array(_) | Expr::Index(_, _) => self.array_expression(expression),
            Expr::Call(name, arguments) if name == "\0php_throw_expression" => {
                if arguments.len() != 1 {
                    self.errors.push("throw expression requires exactly one value".into());
                    return;
                }
                self.expression(&arguments[0]);
                self.instructions.push(Op::Throw);
            }
            Expr::Call(name, arguments) if name.starts_with("\0php_include:") => {
                if arguments.len() != 1 {
                    self.errors.push("include expression requires exactly one path".into());
                    return;
                }
                self.expression(&arguments[0]);
                let mode = match &name["\0php_include:".len()..] {
                    "include" => php_runtime::bytecode::IncludeMode::Include,
                    "include_once" => php_runtime::bytecode::IncludeMode::IncludeOnce,
                    "require" => php_runtime::bytecode::IncludeMode::Require,
                    "require_once" => php_runtime::bytecode::IncludeMode::RequireOnce,
                    _ => { self.errors.push("unknown include expression kind".into()); return; }
                };
                self.instructions.push(Op::Include(mode));
            }
            Expr::Call(name, arguments) => {
                if passed_whole_globals_by_reference(name, arguments, self.program) {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                let (by_reference, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::Call(name.clone(), by_reference, argument_names, unpack, self.strict_types));
            }
            Expr::NamedArgument(_, value) | Expr::UnpackArgument(value) => self.expression(value),
            Expr::Closure { declaration, captures } => {
                let mut captures = captures.clone();
                if !declaration.is_static
                    && self.current_class.is_some()
                    && !captures.iter().any(|(name, _)| name == "this")
                {
                    captures.push(("this".into(), false));
                }
                let mut instructions = Vec::new();
                if let Some(file) = &self.current_source_file { instructions.push(Op::SetFile(file.clone())); }
                let mut generator = CodeGenerator {
                    program: self.program, instructions, current_source_file: self.current_source_file.clone(), parent_class: self.parent_class.clone(), current_class: self.current_class.clone(),
                    current_method: None, current_class_display: self.current_class_display.clone(), current_function: Some("{closure}".into()),
                    strict_types: self.strict_types, known_callables: std::collections::HashMap::new(), variable_types: super::declared_parameter_types(&declaration.parameters, &declaration.parameter_types),
                    returns_by_reference: declaration.returns_by_reference, control_stack: Vec::new(), labels: std::collections::HashMap::new(), pending_gotos: Vec::new(), next_control_id: 0, include_return_stack: Vec::new(), errors: Vec::new(),
                };
                generator.statements(&declaration.body);
                generator.resolve_gotos();
                if let Some(error) = generator.errors.first() {
                    self.errors.push(error.clone());
                    return;
                }
                generator.instructions.push(Op::Const(Value::Null));
                generator.instructions.push(Op::Return);
                let mut function = php_runtime::BytecodeFunction::new(
                    "{closure}".into(), declaration.parameters.clone(), declaration.parameter_types.clone(),
                    declaration.parameter_by_reference.clone(), declaration.parameter_variadic.clone(), super::declarations::parameter_defaults(&declaration.parameter_defaults, self.program, self.current_class.as_deref(), self.parent_class.as_deref())
                        .unwrap_or_else(|error| { self.errors.push(error); Vec::new() }),
                    declaration.return_type.clone(), declaration.returns_by_reference, generator.instructions, declaration.is_static, false, false,
                    php_runtime::MemberVisibility::Public, self.current_class.clone(),
                );
                function.parameter_default_constant_names = super::declarations::parameter_default_constant_names(&declaration.parameter_defaults);
                function.strict_types = self.strict_types;
                function.parameter_attributes = declaration.parameter_attributes.iter().map(|attributes| attributes.iter().map(|attribute| {
                    let arguments = attribute.arguments.iter().map(|argument| match argument {
                        php_syntax::Expr::NamedArgument(name, value) => Ok((Some(name.clone()), super::declarations::constant_parameter_value(value, self.program, self.current_class.as_deref(), self.parent_class.as_deref())?)),
                        value => Ok((None, super::declarations::constant_parameter_value(value, self.program, self.current_class.as_deref(), self.parent_class.as_deref())?)),
                    }).collect::<Result<Vec<_>, String>>()?;
                    Ok(php_runtime::BytecodeAttribute { name: attribute.name.clone(), arguments })
                }).collect::<Result<Vec<_>, String>>()).collect::<Result<Vec<_>, String>>().unwrap_or_else(|error| { self.errors.push(error); Vec::new() });
                if !self.errors.is_empty() { return; }
                self.instructions.push(Op::MakeClosure(Box::new(function), captures));
            }
            Expr::Invoke(callee, arguments) => {
                let resolved = match callee.as_ref() {
                    Expr::Variable(name) => self.known_callables.get(name).unwrap_or(callee.as_ref()),
                    _ => callee.as_ref(),
                };
                if callable_passes_whole_globals_by_reference(resolved, arguments, self.program, self.current_class.as_deref(), self.parent_class.as_deref()) {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                self.expression(callee);
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::CallValue(arguments.len(), references, argument_names, unpack, self.strict_types));
            }
            Expr::FirstClassCallable(target) => match target {
                php_syntax::FirstClassCallableTarget::Value(value) => self.expression(value),
                php_syntax::FirstClassCallableTarget::Function(name) => {
                    self.instructions.push(Op::MakeFunctionCallable(name.clone()));
                }
                php_syntax::FirstClassCallableTarget::StaticMethod(class, method) => {
                    if class == "static" {
                        self.instructions.push(Op::MakeLateStaticCallable(method.clone()));
                        return;
                    }
                    let resolved_class = match class.as_str() {
                        "self" => self.current_class.clone(),
                        "parent" => self.parent_class.clone(),
                        _ => Some(class.clone()),
                    };
                    if let Some(class) = resolved_class {
                        self.instructions.push(Op::MakeStaticCallable(class, method.clone()));
                    } else {
                        self.errors.push(format!("Cannot create callable for {class}::{method}() outside a class context"));
                    }
                }
                php_syntax::FirstClassCallableTarget::InstanceMethod(receiver, method) => {
                    self.expression(receiver);
                    self.instructions.push(Op::MakeMethodCallable(method.clone()));
                }
            },
            Expr::Yield { key, value } => {
                if let Some(key) = key { self.expression(key); }
                if self.returns_by_reference {
                    if let Some((variable, keys)) = reference_path(value) {
                        for key in &keys { self.expression(key); }
                        self.instructions.push(Op::LoadReference(variable, keys.len()));
                    } else {
                        self.expression(value);
                    }
                } else {
                    self.expression(value);
                }
                self.instructions.push(if key.is_some() { Op::YieldKeyed } else { Op::Yield });
            }
            Expr::YieldFrom(iterable) => {
                self.expression(iterable);
                self.instructions.push(Op::YieldFrom);
            }
            Expr::NewObject(name, arguments, source_line) => {
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::NewObject(name.to_ascii_lowercase(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), Some(*source_line), self.strict_types));
            }
            Expr::Property(object, property) => {
                self.expression(object);
                self.instructions.push(Op::GetProperty(property.clone(), self.current_class.clone()));
            }
            // Emit a dedicated opcode to avoid reading the property before __isset runs.
            Expr::IssetProperty(object, property) => {
                self.expression(object);
                self.instructions.push(Op::IssetProperty(property.clone(), self.current_class.clone()));
            }
            Expr::IssetPropertyChain(object, properties) => {
                self.expression(object);
                let mut failure_jumps = Vec::new();
                for (index, property) in properties.iter().enumerate() {
                    let terminal = index + 1 == properties.len();
                    match property {
                        php_syntax::IssetPropertySegment::Fixed(name) => {
                            failure_jumps.push(self.instructions.len());
                            self.instructions.push(Op::IssetChainProperty(name.clone(), terminal, 0, self.current_class.clone()));
                        }
                        php_syntax::IssetPropertySegment::Dynamic(name) => {
                            self.expression(name);
                            failure_jumps.push(self.instructions.len());
                            self.instructions.push(Op::IssetChainDynamicProperty(terminal, 0, self.current_class.clone()));
                        }
                        php_syntax::IssetPropertySegment::ArrayOffset(keys) => {
                            for key in keys { self.expression(key); }
                            failure_jumps.push(self.instructions.len());
                            self.instructions.push(Op::IssetChainArrayOffsets(keys.len(), terminal, 0));
                        }
                    }
                }
                let target = self.instructions.len();
                for instruction in failure_jumps {
                    match &mut self.instructions[instruction] {
                        Op::IssetChainProperty(_, _, target_slot, _) | Op::IssetChainDynamicProperty(_, target_slot, _) => *target_slot = target,
                        Op::IssetChainArrayOffsets(_, _, target_slot) => *target_slot = target,
                        _ => unreachable!("isset chain jump opcode"),
                    }
                }
            }
            Expr::IssetPropertyPath(object, property, keys) => {
                self.expression(object);
                for key in keys { self.expression(key); }
                self.instructions.push(Op::IssetPropertyArray(property.clone(), keys.len(), self.current_class.clone()));
            }
            Expr::IssetStaticProperty(class, property) => {
                let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                self.instructions.push(Op::IssetStaticProperty(class, property.clone(), self.current_class.clone()));
            }
            Expr::IssetDynamicProperty(object, property) => { self.expression(object); self.expression(property); self.instructions.push(Op::IssetDynamicProperty(self.current_class.clone())); }
            Expr::IssetDynamicPropertyPath(object, property, keys) => {
                self.expression(object);
                self.expression(property);
                for key in keys { self.expression(key); }
                self.instructions.push(Op::IssetDynamicPropertyArray(keys.len(), self.current_class.clone()));
            }
            Expr::Isset(values) => {
                let mut false_branches = Vec::with_capacity(values.len());
                for value in values {
                    isset_operand(self, value);
                    let property_result = matches!(value, Expr::IssetProperty(_, _) | Expr::IssetPropertyChain(_, _) | Expr::IssetPropertyPath(_, _, _) | Expr::IssetStaticProperty(_, _) | Expr::IssetDynamicProperty(_, _) | Expr::IssetDynamicPropertyPath(_, _, _));
                    false_branches.push((self.instructions.len(), property_result));
                    self.instructions.push(Op::IssetJump(0, property_result));
                }
                self.instructions.push(Op::Const(Value::Bool(true)));
                let end = self.instructions.len();
                for (branch, property_result) in false_branches {
                    self.instructions[branch] = Op::IssetJump(end, property_result);
                }
            }
            Expr::Empty(value) => {
                if let Some((receiver, property, keys)) = object_property_path(value) {
                    self.expression(receiver);
                    self.instructions.push(Op::Dup);
                    self.instructions.push(Op::IssetProperty(property.clone(), self.current_class.clone()));
                    let missing = self.instructions.len();
                    self.instructions.push(Op::JumpFalse(0));
                    self.instructions.push(Op::GetProperty(property, self.current_class.clone()));
                    for key in keys {
                        self.expression(key);
                        self.instructions.push(Op::ArrayGetIsset);
                    }
                    self.instructions.push(Op::Not);
                    let done = self.instructions.len();
                    self.instructions.push(Op::Jump(0));
                    let empty = self.instructions.len();
                    self.instructions[missing] = Op::JumpFalse(empty);
                    self.instructions.push(Op::Pop);
                    self.instructions.push(Op::Const(Value::Bool(true)));
                    let end = self.instructions.len();
                    self.instructions[done] = Op::Jump(end);
                } else if let Some((receiver, property, keys)) = dynamic_object_property_path(value) {
                    self.expression(receiver);
                    self.expression(property);
                    self.instructions.push(Op::Dup2);
                    self.instructions.push(Op::IssetDynamicProperty(self.current_class.clone()));
                    let missing = self.instructions.len();
                    self.instructions.push(Op::JumpFalse(0));
                    self.instructions.push(Op::GetDynamicProperty(self.current_class.clone()));
                    for key in keys {
                        self.expression(key);
                        self.instructions.push(Op::ArrayGetIsset);
                    }
                    self.instructions.push(Op::Not);
                    let done = self.instructions.len();
                    self.instructions.push(Op::Jump(0));
                    let empty = self.instructions.len();
                    self.instructions[missing] = Op::JumpFalse(empty);
                    self.instructions.push(Op::Pop);
                    self.instructions.push(Op::Pop);
                    self.instructions.push(Op::Const(Value::Bool(true)));
                    let end = self.instructions.len();
                    self.instructions[done] = Op::Jump(end);
                } else {
                    isset_operand(self, value);
                    self.instructions.push(Op::Not);
                }
            }
            Expr::MethodCall(object, method, arguments) => {
                let class = statically_known_receiver_class(object, self.current_class.as_deref(), &self.variable_types);
                if class.as_deref().is_some_and(|class| method_passes_whole_globals_by_reference(class, method, arguments, self.program)
                    || internal_method_passes_whole_globals_by_reference(class, method, arguments)) {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                self.expression(object);
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::CallMethod(method.clone(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), self.strict_types));
            }
            Expr::DynamicMethodCall(object, method, arguments) => {
                let class = statically_known_receiver_class(object, self.current_class.as_deref(), &self.variable_types);
                let known_method = match method.as_ref() { Expr::String(method) => Some(method.as_str()), _ => None };
                if class.as_deref().zip(known_method).is_some_and(|(class, method)| method_passes_whole_globals_by_reference(class, method, arguments, self.program)
                    || internal_method_passes_whole_globals_by_reference(class, method, arguments)) {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                self.expression(object);
                self.expression(method);
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::CallMethod("\0dynamic".into(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), self.strict_types));
            }
            Expr::ParentMethodCall(method, arguments) => {
                if self.parent_class.as_deref().is_some_and(|class| method_passes_whole_globals_by_reference(class, method, arguments, self.program)) {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::CallParentMethod(self.parent_class.clone().unwrap_or_default(), method.to_ascii_lowercase(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), self.strict_types));
            }
            Expr::StaticCall(class, method, arguments) => {
                let class_name = match class.as_str() {
                    "self" | "static" => self.current_class.as_deref(),
                    "parent" => self.parent_class.as_deref(),
                    class => Some(class),
                };
                if class_name.is_some_and(|class| method_passes_whole_globals_by_reference(class, method, arguments, self.program)) {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                let class = match class.as_str() {
                    "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "static" => "static".into(),
                    _ => class.to_ascii_lowercase(),
                };
                self.instructions.push(Op::CallStaticMethod(class, method.to_ascii_lowercase(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), self.strict_types));
            }
            Expr::DynamicStaticCall(class, method, arguments) => {
                self.expression(class);
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::CallStaticMethod("\0dynamic".into(), method.to_ascii_lowercase(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), self.strict_types));
            }
            Expr::DynamicStaticCallExpr(class, method, arguments) => {
                self.expression(class);
                self.expression(method);
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::CallStaticMethod("\0dynamic".into(), "\0dynamic".into(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), self.strict_types));
            }
            Expr::DynamicNewObject(class, arguments, source_line) => {
                self.expression(class);
                let (references, argument_names, unpack) = argument_metadata(arguments, self.current_class.clone());
                self.call_arguments(arguments);
                self.instructions.push(Op::NewObject("\0dynamic".into(), arguments.len(), references, argument_names, unpack, self.current_class.clone(), Some(*source_line), self.strict_types));
            }
            Expr::DynamicClassName(value) => { self.expression(value); self.instructions.push(Op::DynamicClassName); }
            Expr::DynamicClassConstant(class, constant) => {
                self.expression(class);
                self.instructions.push(Op::GetClassConstant("\0dynamic".into(), constant.clone(), self.current_class.clone()));
            }
            Expr::DynamicProperty(receiver, property) => {
                self.expression(receiver);
                self.expression(property);
                self.instructions.push(Op::GetDynamicProperty(self.current_class.clone()));
            }
            Expr::ErrorSuppress(value) => self.expression(value),
            Expr::DynamicPropertyAssignment { receiver, property, value } => {
                self.expression(receiver);
                self.expression(property);
                self.expression(value);
                self.instructions.push(Op::SetDynamicProperty(self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
            }
            Expr::DynamicPropertyCompoundAssignment { receiver, property, operation, value } => {
                let operation = match operation {
                    php_syntax::BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                    php_syntax::BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                    php_syntax::BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                    php_syntax::BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                    php_syntax::BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                    php_syntax::BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                    php_syntax::BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                    php_syntax::BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                    php_syntax::BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                    php_syntax::BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                    php_syntax::BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                    php_syntax::BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                    _ => { self.errors.push("Unsupported compound operation on a dynamic property".into()); return; }
                };
                self.expression(receiver);
                self.expression(property);
                self.expression(value);
                self.instructions.push(Op::CompoundAssignDynamicProperty(operation, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
            }
            Expr::StaticProperty(class, property) => {
                let class = match class.as_str() {
                    "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "static" => "static".into(),
                    _ => class.to_ascii_lowercase(),
                };
                self.instructions.push(Op::GetStaticProperty(class, property.clone(), self.current_class.clone()));
            }
            Expr::DynamicStaticProperty(class, property) => {
                self.expression(class);
                self.instructions.push(Op::GetDynamicStaticProperty(property.clone()));
            }
            Expr::DynamicStaticPropertyExpr(class, property) => {
                self.expression(class);
                self.expression(property);
                self.instructions.push(Op::GetDynamicStaticPropertyExpr);
            }
            Expr::DynamicStaticPropertyArrayAssignment { class, property, keys, operation, value } => {
                self.expression(class);
                self.expression(property);
                for key in keys { self.expression(key); }
                self.expression(value);
                if let Some(operation) = operation {
                    let operation = match operation {
                        BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                        BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                        BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                        BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                        BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                        BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                        BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                        BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                        BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                        BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                        BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                        BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                        _ => { self.errors.push("Unsupported compound operator for dynamic static property array path".into()); return; }
                    };
                    self.instructions.push(Op::AssignDynamicStaticPropertyArray(keys.len(), Some(operation), self.current_class.clone()));
                } else {
                    self.instructions.push(Op::AssignDynamicStaticPropertyArray(keys.len(), None, self.current_class.clone()));
                }
            }
            Expr::ClassConstant(class, constant) => {
                let class = match class.as_str() {
                    "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "parent" => self.parent_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "static" => "static".into(),
                    _ => class.to_ascii_lowercase(),
                };
                self.instructions.push(Op::GetClassConstant(class, constant.clone(), self.current_class.clone()));
            }
            // The runtime owns shallow-copy semantics and the __clone callback.
            Expr::Clone(value) => {
                self.expression(value);
                self.instructions.push(Op::CloneObject);
            }
            Expr::PostIncrement(target, delta) => match target {
                php_syntax::IncrementTarget::Variable(name) => self.instructions.push(Op::PostIncrement(name.clone(), *delta)),
                php_syntax::IncrementTarget::DynamicVariable { name, keys } => {
                    self.expression(name);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementDynamicVariablePath(keys.len(), *delta, true));
                }
                php_syntax::IncrementTarget::ArrayPath { variable, keys } => {
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementArrayPath(variable.clone(), keys.len(), *delta, true));
                }
                php_syntax::IncrementTarget::PropertyArrayPath { receiver, property, keys } => {
                    self.expression(receiver);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementPropertyArrayPath(property.clone(), keys.len(), *delta, true,
                        self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
                php_syntax::IncrementTarget::StaticPropertyArrayPath { class, property, keys } => {
                    for key in keys { self.expression(key); }
                    let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                    self.instructions.push(Op::IncrementStaticPropertyArrayPath(class, property.clone(), keys.len(), *delta, true, self.current_class.clone()));
                }
                php_syntax::IncrementTarget::DynamicPropertyArrayPath { receiver, property, keys } => {
                    self.expression(receiver); self.expression(property);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementDynamicPropertyArrayPath(keys.len(), *delta, true, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
                php_syntax::IncrementTarget::DynamicStaticPropertyArrayPath { class, property, keys } => {
                    self.expression(class); self.expression(property);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementDynamicStaticPropertyArrayPath(keys.len(), *delta, true, self.current_class.clone()));
                }
                php_syntax::IncrementTarget::Property { receiver, property } => {
                    self.expression(receiver);
                    self.instructions.push(Op::IncrementProperty(property.clone(), *delta, true, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
                php_syntax::IncrementTarget::StaticProperty { class, property } => {
                    let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                    self.instructions.push(Op::IncrementStaticProperty(class, property.clone(), *delta, true, self.current_class.clone()));
                }
                php_syntax::IncrementTarget::DynamicProperty { receiver, property } => {
                    self.expression(receiver); self.expression(property);
                    self.instructions.push(Op::IncrementDynamicProperty(*delta, true, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
            },
            Expr::PreIncrement(target, delta) => match target {
                php_syntax::IncrementTarget::Variable(name) => self.instructions.push(Op::PreIncrement(name.clone(), *delta)),
                php_syntax::IncrementTarget::DynamicVariable { name, keys } => {
                    self.expression(name);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementDynamicVariablePath(keys.len(), *delta, false));
                }
                php_syntax::IncrementTarget::ArrayPath { variable, keys } => {
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementArrayPath(variable.clone(), keys.len(), *delta, false));
                }
                php_syntax::IncrementTarget::PropertyArrayPath { receiver, property, keys } => {
                    self.expression(receiver);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementPropertyArrayPath(property.clone(), keys.len(), *delta, false,
                        self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
                php_syntax::IncrementTarget::StaticPropertyArrayPath { class, property, keys } => {
                    for key in keys { self.expression(key); }
                    let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                    self.instructions.push(Op::IncrementStaticPropertyArrayPath(class, property.clone(), keys.len(), *delta, false, self.current_class.clone()));
                }
                php_syntax::IncrementTarget::DynamicPropertyArrayPath { receiver, property, keys } => {
                    self.expression(receiver); self.expression(property);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementDynamicPropertyArrayPath(keys.len(), *delta, false, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
                php_syntax::IncrementTarget::DynamicStaticPropertyArrayPath { class, property, keys } => {
                    self.expression(class); self.expression(property);
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::IncrementDynamicStaticPropertyArrayPath(keys.len(), *delta, false, self.current_class.clone()));
                }
                php_syntax::IncrementTarget::Property { receiver, property } => {
                    self.expression(receiver);
                    self.instructions.push(Op::IncrementProperty(property.clone(), *delta, false, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
                php_syntax::IncrementTarget::StaticProperty { class, property } => {
                    let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                    self.instructions.push(Op::IncrementStaticProperty(class, property.clone(), *delta, false, self.current_class.clone()));
                }
                php_syntax::IncrementTarget::DynamicProperty { receiver, property } => {
                    self.expression(receiver); self.expression(property);
                    self.instructions.push(Op::IncrementDynamicProperty(*delta, false, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                }
            },
            Expr::Negate(value) => {
                self.expression(value);
                self.instructions.push(Op::Neg);
            }
            Expr::Positive(value) => self.expression(value),
            Expr::Not(value) => {
                self.expression(value);
                self.instructions.push(Op::Not);
            }
            Expr::BitwiseNot(value) => {
                self.expression(value);
                self.instructions.push(Op::BitNot);
            }
            Expr::Cast(cast_type, value) => {
                self.expression(value);
                let cast_type = match cast_type {
                    php_syntax::CastType::Integer => php_runtime::bytecode::CastType::Integer,
                    php_syntax::CastType::Float => php_runtime::bytecode::CastType::Float,
                    php_syntax::CastType::String => php_runtime::bytecode::CastType::String,
                    php_syntax::CastType::Bool => php_runtime::bytecode::CastType::Bool,
                    php_syntax::CastType::Array => php_runtime::bytecode::CastType::Array,
                    php_syntax::CastType::Object => php_runtime::bytecode::CastType::Object,
                };
                self.instructions.push(Op::Cast(cast_type));
            }
            Expr::Ternary { condition, then_value, else_value } => {
                self.expression(condition);
                let false_branch = self.instructions.len();
                self.instructions.push(Op::JumpFalse(0));
                self.expression(then_value);
                let end_branch = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                let else_start = self.instructions.len();
                self.instructions[false_branch] = Op::JumpFalse(else_start);
                self.expression(else_value);
                let end = self.instructions.len();
                self.instructions[end_branch] = Op::Jump(end);
            }
            Expr::Match { subject, arms } => {
                self.expression(subject);
                let mut matched_ends = Vec::new();
                let mut default_value = None;
                for arm in arms {
                    if arm.conditions.is_empty() {
                        default_value = Some(&arm.value);
                        continue;
                    }
                    for condition in &arm.conditions {
                        self.instructions.push(Op::Dup);
                        self.expression(condition);
                        self.instructions.push(Op::StrictEq);
                        let next_condition = self.instructions.len();
                        self.instructions.push(Op::JumpFalse(0));
                        self.instructions.push(Op::Pop);
                        self.expression(&arm.value);
                        matched_ends.push(self.instructions.len());
                        self.instructions.push(Op::Jump(0));
                        self.instructions[next_condition] = Op::JumpFalse(self.instructions.len());
                    }
                }
                self.instructions.push(Op::Pop);
                if let Some(value) = default_value {
                    self.expression(value);
                } else {
                    self.instructions.push(Op::Const(Value::String("Unhandled match case".into())));
                    self.instructions.push(Op::NewObject("unhandledmatcherror".into(), 1, vec![None], vec![None], vec![false], self.current_class.clone(), None, self.strict_types));
                    self.instructions.push(Op::Throw);
                    self.instructions.push(Op::Const(Value::Null));
                }
                let end = self.instructions.len();
                for branch in matched_ends { self.instructions[branch] = Op::Jump(end); }
            }
            Expr::ShorthandTernary { condition, else_value } => {
                self.expression(condition);
                self.instructions.push(Op::Dup);
                let false_branch = self.instructions.len();
                self.instructions.push(Op::JumpFalse(0));
                let end_branch = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                let else_start = self.instructions.len();
                self.instructions[false_branch] = Op::JumpFalse(else_start);
                self.instructions.push(Op::Pop);
                self.expression(else_value);
                let end = self.instructions.len();
                self.instructions[end_branch] = Op::Jump(end);
            }
            Expr::Coalesce(value, fallback) => {
                coalesce_operand(self, value);
                let end_branch = self.instructions.len();
                self.instructions.push(Op::CoalesceJump(0));
                self.expression(fallback);
                let end = self.instructions.len();
                self.instructions[end_branch] = Op::CoalesceJump(end);
            }
            Expr::Binary(operation @ (BinaryOp::LogicalAnd | BinaryOp::LogicalOr), left, right) => {
                self.logical_expression(*operation, left, right);
            }
            Expr::Binary(operation, left, right) => {
                self.expression(left);
                self.expression(right);
                self.instructions.push(match operation {
                    BinaryOp::Add => Op::Add,
                    BinaryOp::Subtract => Op::Sub,
                    BinaryOp::Multiply => Op::Mul,
                    BinaryOp::Power => Op::Pow,
                    BinaryOp::Divide => Op::Div,
                    BinaryOp::Modulo => Op::Mod,
                    BinaryOp::Concatenate => Op::Concat,
                    BinaryOp::BitAnd => Op::BitAnd,
                    BinaryOp::BitXor => Op::BitXor,
                    BinaryOp::BitOr => Op::BitOr,
                    BinaryOp::ShiftLeft => Op::ShiftLeft,
                    BinaryOp::ShiftRight => Op::ShiftRight,
                    BinaryOp::Equal => Op::Eq,
                    BinaryOp::NotEqual => Op::NotEq,
                    BinaryOp::StrictEqual => Op::StrictEq,
                    BinaryOp::StrictNotEqual => Op::StrictNotEq,
                    BinaryOp::Less => Op::Lt,
                    BinaryOp::LessEqual => Op::Le,
                    BinaryOp::Greater => Op::Gt,
                    BinaryOp::GreaterEqual => Op::Ge,
                    BinaryOp::Spaceship => Op::Spaceship,
                    BinaryOp::LogicalAnd | BinaryOp::LogicalOr => unreachable!("logical operators use short-circuit code generation"),
                    BinaryOp::LogicalXor => Op::LogicalXor,
                    BinaryOp::Coalesce => unreachable!("coalesce expressions use short-circuit code generation"),
                });
            }
            Expr::InstanceOf(value, class_name) => {
                self.expression(value);
                match class_name.as_ref() {
                    Expr::String(name) if name.eq_ignore_ascii_case("self") => match &self.current_class {
                        Some(class) => self.instructions.push(Op::Const(Value::String(class.clone()))),
                        None => self.errors.push("self used outside a class method in instanceof".into()),
                    },
                    Expr::String(name) if name.eq_ignore_ascii_case("parent") => match &self.parent_class {
                        Some(class) => self.instructions.push(Op::Const(Value::String(class.clone()))),
                        None => self.errors.push("parent used without a parent class in instanceof".into()),
                    },
                    Expr::String(name) if name.eq_ignore_ascii_case("static") => {
                        self.instructions.push(Op::Load("\0static_class".into()));
                    }
                    _ => self.expression(class_name),
                }
                self.instructions.push(Op::InstanceOf);
            }
            Expr::Assignment { variable, operation, value } => {
                if variable == "GLOBALS" {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                let known = if operation.is_none() {
                    match value.as_ref() {
                        Expr::String(_) | Expr::Array(_) | Expr::FirstClassCallable(_) => Some(value.as_ref().clone()),
                        Expr::Variable(source) => self.known_callables.get(source).cloned(),
                        _ => None,
                    }
                } else { None };
                if let Some(known) = known { self.known_callables.insert(variable.clone(), known); }
                else { self.known_callables.remove(variable); }
                let assigned_type = operation.is_none().then(|| assigned_type(value, &self.variable_types)).flatten();
                if let Some(assigned_type) = assigned_type { self.variable_types.insert(variable.clone(), assigned_type); }
                else { self.variable_types.remove(variable); }
                if let Some(operation) = operation {
                    if *operation == BinaryOp::Coalesce {
                        self.instructions.push(Op::LoadSilent(variable.clone()));
                        let present_branch = self.instructions.len();
                        self.instructions.push(Op::CoalesceJump(0));
                        self.expression(value);
                        self.instructions.push(Op::Dup);
                        self.instructions.push(Op::Store(variable.clone()));
                        let end = self.instructions.len();
                        self.instructions[present_branch] = Op::CoalesceJump(end);
                        return;
                    }
                    self.instructions.push(Op::Load(variable.clone()));
                    self.expression(value);
                    self.instructions.push(match operation {
                        BinaryOp::Add => Op::Add,
                        BinaryOp::Subtract => Op::Sub,
                        BinaryOp::Multiply => Op::Mul,
                    BinaryOp::Power => Op::Pow,
                        BinaryOp::Divide => Op::Div,
                        BinaryOp::Modulo => Op::Mod,
                        BinaryOp::Concatenate => Op::Concat,
                    BinaryOp::BitAnd => Op::BitAnd,
                    BinaryOp::BitXor => Op::BitXor,
                    BinaryOp::BitOr => Op::BitOr,
                    BinaryOp::ShiftLeft => Op::ShiftLeft,
                    BinaryOp::ShiftRight => Op::ShiftRight,
                        BinaryOp::Coalesce => unreachable!("coalescing assignment uses short-circuit code generation"),
                        BinaryOp::LogicalXor => unreachable!("logical XOR is not a compound assignment operator"),
                        BinaryOp::LogicalAnd | BinaryOp::LogicalOr | BinaryOp::Equal | BinaryOp::NotEqual | BinaryOp::StrictEqual | BinaryOp::StrictNotEqual | BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual | BinaryOp::Spaceship => unreachable!("only arithmetic and concatenation assignment operators are parsed"),
                    });
                } else {
                    self.expression(value);
                }
                self.instructions.push(Op::Dup);
                self.instructions.push(Op::Store(variable.clone()));
            }
            Expr::ReferenceAssignment { variable, value } => {
                let assigned_type = assigned_type(value, &self.variable_types);
                if let Some(assigned_type) = assigned_type { self.variable_types.insert(variable.clone(), assigned_type); }
                else { self.variable_types.remove(variable); }
                if variable == "GLOBALS" || matches!(value.as_ref(), Expr::Variable(name) if name == "GLOBALS") {
                    self.errors.push("Cannot create a reference to $GLOBALS itself".into());
                    return;
                }
                if let Some((class, property, keys)) = static_property_path(value) {
                    let class = match class {
                        "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                        "parent" => self.parent_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                        "static" => "static".into(),
                        _ => class.to_ascii_lowercase(),
                    };
                    self.instructions.push(Op::LoadStaticPropertyReference(class, property.to_string(), self.current_class.clone()));
                    for key in &keys { self.expression(key); }
                    if !keys.is_empty() { self.instructions.push(Op::LoadStaticPropertyArrayReference(keys.len())); }
                } else {
                    self.expression(value);
                }
                self.instructions.push(Op::BindReturnedReferenceExpr(variable.clone()));
            }
            Expr::DynamicVariableAssignment { name, operation, value } => {
                self.known_callables.clear();
                self.expression(name);
                if *operation == Some(BinaryOp::Coalesce) {
                    let branch = self.instructions.len();
                    self.instructions.push(Op::CoalesceDynamicExprJump(0));
                    self.expression(value);
                    self.instructions.push(Op::AssignDynamicExpr);
                    let end = self.instructions.len();
                    self.instructions[branch] = Op::CoalesceDynamicExprJump(end);
                } else if let Some(operation) = operation {
                    self.instructions.push(Op::Dup);
                    self.instructions.push(Op::LoadDynamicExpr);
                    self.expression(value);
                    let operation = match operation {
                        BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                        BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                        BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                        BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                        BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                        BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                        BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                        BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                        BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                        BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                        BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                        BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                        _ => { self.errors.push("Unsupported dynamic-variable assignment operator".into()); return; }
                    };
                    self.instructions.push(Op::CompoundAssignDynamicExpr(operation));
                } else {
                    self.expression(value);
                    self.instructions.push(Op::AssignDynamicExpr);
                }
            }
            Expr::ArrayAssignment { variable, keys, operation, value } => {
                self.instructions.push(Op::Load(variable.clone()));
                for key in keys { self.expression(key); }
                if *operation == Some(BinaryOp::Coalesce) {
                    let present_branch = self.instructions.len();
                    self.instructions.push(Op::CoalescePathJump(0, keys.len()));
                    self.expression(value);
                    self.instructions.push(Op::CoalesceAssignPath(variable.clone(), keys.len()));
                    let end = self.instructions.len();
                    self.instructions[present_branch] = Op::CoalescePathJump(end, keys.len());
                } else {
                    self.expression(value);
                    match operation {
                        None => self.instructions.push(Op::AssignArrayPath(variable.clone(), keys.len())),
                        Some(operation) => {
                            let operation = match operation {
                                BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                                BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                                BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                                BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                                BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                                BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                                BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                        BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                        BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                        BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                        BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                        BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                                _ => { self.errors.push("Unsupported array assignment operator".into()); return; }
                            };
                            self.instructions.push(Op::AssignCompoundPath(variable.clone(), keys.len(), operation));
                        }
                    }
                }
            }
            Expr::PropertyAssignment { receiver, property, operation, value } => {
                self.expression(receiver);
                if *operation == Some(BinaryOp::Coalesce) {
                    let present_branch = self.instructions.len();
                    self.instructions.push(Op::CoalescePropertyJump(0, property.clone(), self.current_class.clone()));
                    self.expression(value);
                    self.instructions.push(Op::CoalesceAssignProperty(
                        property.clone(), self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct")),
                    ));
                    let end = self.instructions.len();
                    self.instructions[present_branch] = Op::CoalescePropertyJump(end, property.clone(), self.current_class.clone());
                } else {
                    self.expression(value);
                    match operation {
                        None => self.instructions.push(Op::AssignPropertyExpr(
                            property.clone(), self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct")),
                        )),
                        Some(operation) => {
                            let operation = match operation {
                                BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                                BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                                BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                                BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                                BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                                BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                                BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                        BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                        BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                        BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                        BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                        BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                                _ => { self.errors.push("Unsupported property assignment operator".into()); return; }
                            };
                            self.instructions.push(Op::CompoundAssignProperty(
                                property.clone(), operation, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct")),
                            ));
                        }
                    }
                }
            }
            Expr::PropertyArrayAssignment { receiver, property, keys, operation, value } => {
                self.expression(receiver);
                for key in keys { self.expression(key); }
                if *operation == Some(BinaryOp::Coalesce) {
                    let present_branch = self.instructions.len();
                    self.instructions.push(Op::CoalescePropertyArrayJump(0, property.clone(), keys.len(), self.current_class.clone()));
                    self.expression(value);
                    self.instructions.push(Op::CoalesceAssignPropertyArray(property.clone(), keys.len(), self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                    let end = self.instructions.len();
                    self.instructions[present_branch] = Op::CoalescePropertyArrayJump(end, property.clone(), keys.len(), self.current_class.clone());
                } else {
                    self.expression(value);
                    if let Some(operation) = operation {
                        let operation = match operation {
                            BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                            BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                            BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                            BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                            BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                            BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                            BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                            BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                            BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                            BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                            BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                            BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                            _ => { self.errors.push("Unsupported object property array assignment operator".into()); return; }
                        };
                        self.instructions.push(Op::CompoundAssignPropertyArray(property.clone(), keys.len(), operation, self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                    } else {
                        self.instructions.push(Op::AssignPropertyArray(property.clone(), keys.len(), self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
                    }
                }
            }
            Expr::PropertyAppendAssignment { receiver, property, keys, value } => {
                self.expression(receiver);
                for key in keys { self.expression(key); }
                self.expression(value);
                self.instructions.push(Op::AppendPropertyArray(property.clone(), keys.len(), self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
            }
            Expr::DynamicPropertyAppendAssignment { receiver, property, keys, value } => {
                self.expression(receiver);
                self.expression(property);
                for key in keys { self.expression(key); }
                self.expression(value);
                self.instructions.push(Op::AppendDynamicPropertyArray(keys.len(), self.current_class.clone(), self.current_method.as_deref().is_some_and(|method| method.eq_ignore_ascii_case("__construct"))));
            }
            Expr::StaticPropertyArrayAssignment { class, property, keys, operation, value } => {
                let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                for key in keys { self.expression(key); }
                if *operation == Some(BinaryOp::Coalesce) {
                    let branch = self.instructions.len();
                    self.instructions.push(Op::CoalesceStaticPropertyArrayJump(0, class.clone(), property.clone(), keys.len(), self.current_class.clone()));
                    self.expression(value);
                    self.instructions.push(Op::AssignStaticPropertyArray(class.clone(), property.clone(), keys.len(), self.current_class.clone()));
                    let end = self.instructions.len();
                    self.instructions[branch] = Op::CoalesceStaticPropertyArrayJump(end, class, property.clone(), keys.len(), self.current_class.clone());
                } else {
                    self.expression(value);
                    if let Some(operation) = operation {
                        let operation = match operation {
                            BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                            BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                            BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                            BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                            BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                            BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                            BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                            BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                            BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                            BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                            BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                            BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                            _ => { self.errors.push("Unsupported compound operator for static property array path".into()); return; }
                        };
                        self.instructions.push(Op::CompoundAssignStaticPropertyArray(class, property.clone(), keys.len(), operation, self.current_class.clone()));
                    } else {
                        self.instructions.push(Op::AssignStaticPropertyArray(class, property.clone(), keys.len(), self.current_class.clone()));
                    }
                }
            }
            Expr::ArrayAppendAssignment { variable, keys, value } => {
                self.instructions.push(Op::Load(variable.clone()));
                for key in keys { self.expression(key); }
                self.expression(value);
                self.instructions.push(Op::AssignArrayAppendPath(variable.clone(), keys.len()));
            }
            Expr::ArrayExpressionAppendAssignment { array, keys, value } => {
                self.expression(array);
                for key in keys { self.expression(key); }
                self.expression(value);
                self.instructions.push(Op::ArrayAppendPath(keys.len()));
            }
            Expr::StaticPropertyAppendAssignment { class, property, keys, value } => {
                let class = match class.as_str() { "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()), "static" => "static".into(), _ => class.to_ascii_lowercase() };
                for key in keys { self.expression(key); }
                self.expression(value);
                self.instructions.push(Op::AppendStaticPropertyArray(class, property.clone(), keys.len(), self.current_class.clone()));
            }
            Expr::StaticPropertyAssignment { class, property, operation, value } => {
                let class = match class.as_str() {
                    "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "static" => "static".into(),
                    _ => class.to_ascii_lowercase(),
                };
                if *operation == Some(BinaryOp::Coalesce) {
                    let present_branch = self.instructions.len();
                    self.instructions.push(Op::CoalesceStaticPropertyJump(0, class.clone(), property.clone(), self.current_class.clone()));
                    self.expression(value);
                    self.instructions.push(Op::CoalesceAssignStaticProperty(class.clone(), property.clone(), self.current_class.clone()));
                    let end = self.instructions.len();
                    self.instructions[present_branch] = Op::CoalesceStaticPropertyJump(end, class, property.clone(), self.current_class.clone());
                } else {
                    self.expression(value);
                    match operation {
                        None => self.instructions.push(Op::AssignStaticPropertyExpr(class, property.clone(), self.current_class.clone())),
                        Some(operation) => {
                            let operation = match operation {
                                BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                                BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                                BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                                BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                                BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                                BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                                BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                        BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                        BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                        BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                        BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                        BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                                _ => { self.errors.push("Unsupported static-property assignment operator".into()); return; }
                            };
                            self.instructions.push(Op::CompoundAssignStaticProperty(class, property.clone(), operation, self.current_class.clone()));
                        }
                    }
                }
            }
            Expr::PropertyAppend(_) => self.errors.push("Appending to an object property array is only supported as an assignment target".into()),
        }
    }

    /// Emit short-circuit branches and normalize logical results to booleans.
    fn logical_expression(&mut self, operation: BinaryOp, left: &Expr, right: &Expr) {
        self.expression(left);
        let left_branch = self.instructions.len();
        self.instructions.push(Op::JumpFalse(0));

        if matches!(operation, BinaryOp::LogicalOr) {
            self.instructions.push(Op::Const(Value::Bool(true)));
            let end_branch = self.instructions.len();
            self.instructions.push(Op::Jump(0));
            let right_start = self.instructions.len();
            self.instructions[left_branch] = Op::JumpFalse(right_start);
            self.expression(right);
            let right_branch = self.instructions.len();
            self.instructions.push(Op::JumpFalse(0));
            self.instructions.push(Op::Const(Value::Bool(true)));
            let end = self.instructions.len();
            self.instructions.push(Op::Jump(0));
            let false_target = self.instructions.len();
            self.instructions.push(Op::Const(Value::Bool(false)));
            let after_false = self.instructions.len();
            self.instructions[end_branch] = Op::Jump(after_false);
            self.instructions[right_branch] = Op::JumpFalse(false_target);
            self.instructions[end] = Op::Jump(after_false);
        } else {
            self.expression(right);
            let right_branch = self.instructions.len();
            self.instructions.push(Op::JumpFalse(0));
            self.instructions.push(Op::Const(Value::Bool(true)));
            let end_branch = self.instructions.len();
            self.instructions.push(Op::Jump(0));
            let false_target = self.instructions.len();
            self.instructions.push(Op::Const(Value::Bool(false)));
            let end = self.instructions.len();
            self.instructions[left_branch] = Op::JumpFalse(false_target);
            self.instructions[right_branch] = Op::JumpFalse(false_target);
            self.instructions[end_branch] = Op::Jump(end);
        }
    }

}
