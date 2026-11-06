//! Object creation, property access, class constants, cloning, and magic hooks.

use std::{cell::RefCell, collections::{HashMap, HashSet}, rc::Rc};

use crate::{bytecode::{CompoundOperation, Op}, ArrayKey, BytecodeClass, BytecodeConstant, BytecodeFunction, BytecodeProperty, MemberVisibility, PhpObject, Value};
use super::{complete_arguments, function_scope, validate_arguments, validate_return_type, GlobalVariables, StaticProperties, VariableBinding};

fn is_builtin_runtime_class_name(name: &str) -> bool {
    crate::exception_builtins::is_class(name) || matches!(name,
        "generator" | "stdclass" | "pdo" | "pdostatement" | "reflectionclass" | "reflectionclassconstant" | "reflectionconstant"
        | "reflectionobject" | "reflectionenum" | "reflectionenumunitcase" | "reflectionenumbackedcase" | "reflectionnamedtype"
        | "reflectiontype" | "reflectionuniontype" | "reflectionintersectiontype" | "reflectionfunction"
        | "reflectionmethod" | "reflectionproperty" | "reflectionattribute" | "reflectionparameter"
        | "attribute" | "countable" | "iterator" | "iteratoraggregate" | "traversable" | "stringable"
        | "jsonserializable" | "mongodb\\client" | "mongodb\\database" | "mongodb\\collection"
        | "mongodb\\cursor" | "mongodb\\insertresult" | "mongodb\\insertmanyresult" | "mongodb\\updateresult"
        | "mongodb\\deleteresult")
}

fn isset_array_key(value: &Value) -> Result<ArrayKey, String> {
    let type_name = match value.dereference() {
        Value::Object(object) => object.borrow().class_name.clone(),
        value => value.type_name().to_string(),
    };
    value.array_key().map_err(|_| format!("TypeError:Cannot access offset of type {type_name} in isset or empty"))
}

fn unset_array_key(value: &Value, container: &Value) -> Result<ArrayKey, String> {
    let type_name = match value.dereference() {
        Value::Object(object) => object.borrow().class_name.clone(),
        value => value.type_name().to_string(),
    };
    value.array_key().map_err(|_| format!("Error:Cannot unset offset of type {type_name} on {}", container.type_name()))
}

fn reflection_parameter_object(function: &BytecodeFunction, position: usize, mut origin: HashMap<String, Value>) -> Value {
    let is_method_parameter = origin.contains_key("\0target_class");
    let attributes = function.parameter_attributes.get(position).map(Vec::as_slice).unwrap_or_default();
    let reflected_attributes = attributes.iter().map(|attribute| {
        let repeated = attributes.iter().filter(|candidate| candidate.name.eq_ignore_ascii_case(&attribute.name)).count() > 1;
        Value::Object(Rc::new(RefCell::new(PhpObject {
            class_name: "ReflectionAttribute".into(),
            properties: HashMap::from([
                ("\0attribute_name".into(), Value::String(attribute.name.clone())),
                ("\0attribute_arguments".into(), super::functions::reflection_attribute_arguments(&attribute.arguments)),
                ("\0target_class".into(), Value::String(function.display_name.clone())),
                ("\0target_function".into(), Value::String(function.display_name.clone())),
                ("\0target_type".into(), Value::Integer(32)),
                ("\0target_position".into(), Value::Integer(position as i64)),
                ("\0is_repeated".into(), Value::Bool(repeated)),
            ]),
            property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
        })))
    }).enumerate().map(|(index, attribute)| (ArrayKey::Integer(index as i64), attribute)).collect();
    origin.extend(HashMap::from([
        ("\0parameter_name".into(), Value::String(function.parameters[position].clone())),
        ("\0parameter_position".into(), Value::Integer(position as i64)),
        ("\0parameter_type".into(), function.parameter_types.get(position).and_then(Clone::clone).map(Value::String).unwrap_or(Value::Null)),
        ("\0parameter_by_reference".into(), Value::Bool(function.parameter_by_reference.get(position).copied().unwrap_or(false))),
        ("\0parameter_variadic".into(), Value::Bool(function.parameter_variadic.get(position).copied().unwrap_or(false))),
        ("\0parameter_promoted".into(), Value::Bool(function.parameter_promoted.get(position).copied().unwrap_or(false))),
        ("\0parameter_has_default".into(), Value::Bool(function.parameter_defaults.get(position).is_some_and(Option::is_some))),
        ("\0parameter_optional".into(), Value::Bool(function.parameter_variadic.get(position).copied().unwrap_or(false) || (function.parameter_defaults.get(position).is_some_and(Option::is_some) && position >= super::functions::required_parameter_count(function)))),
        ("\0parameter_default".into(), function.parameter_defaults.get(position).and_then(Clone::clone).unwrap_or(Value::Null)),
        ("\0parameter_default_constant_name".into(), function.parameter_default_constant_names.get(position).and_then(Clone::clone).map(Value::String).unwrap_or(Value::Null)),
    ]));
    if !is_method_parameter {
        origin.insert("\0parameter_attributes".into(), Value::from_array_entries(reflected_attributes));
    }
    Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: "ReflectionParameter".into(), properties: origin,
        property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
    })))
}

/// Execute object construction and instance-property instructions.
pub(super) fn execute_instruction(
    instruction: &Op,
    stack: &mut Vec<Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    frame_scope: &str,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
    program_counter: &mut usize,
) -> Result<bool, String> {
    match instruction {
        Op::NewObject(name, _argument_count, reference_names, argument_names, unpack, access_scope, source_line, strict_types) => {
            let (arguments, reference_names, argument_names) = super::functions::take_call_arguments(stack, reference_names, argument_names, unpack, classes)?;
            let dynamic_name = if name == "\0dynamic" {
                match stack.pop().ok_or_else(|| "Missing dynamic class name".to_string())? { Value::String(class) => class, value => value.show() }
            } else { name.clone() };
            let resolved_name = if dynamic_name == "static" {
                match variables.get("\0static_class") { Some(Value::String(class)) => class.clone(), _ => return Err("static:: used outside a class method".into()) }
            } else { dynamic_name };
            let name = &resolved_name;
            let dynamic_key = name.trim_start_matches('\\').to_ascii_lowercase();
            if !classes.contains_key(&dynamic_key) && !is_builtin_runtime_class_name(&dynamic_key)
                && !global_variables.autoload_callbacks.is_empty()
            {
                super::functions::invoke_registered_autoloaders(name, variables, bindings, global_variables,
                    global_scope, frame_id, functions, classes, static_properties, output, steps)?;
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
            if let Some(class) = classes.get(&name.to_ascii_lowercase()) {
                if let Some(trait_name) = class.unresolved_traits.first() {
                    return Err(format!("Trait '{}' not found", trait_name));
                }
            }
            if name.eq_ignore_ascii_case("ReflectionEnum") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionenum::__construct", arguments, &reference_names, &argument_names)?;
                let requested_value = arguments.first().ok_or("ReflectionEnum::__construct() expects exactly 1 argument")?;
                let requested = match requested_value.dereference() {
                    Value::Object(object) => object.borrow().class_name.clone(),
                    value => value.show(),
                };
                let class = classes.get(&requested.to_ascii_lowercase()).filter(|class| class.is_enum)
                    .ok_or_else(|| format!("Class \"{requested}\" is not an enum"))?;
                stack.push(Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionEnum".into(),
                    properties: HashMap::from([("\0target_class".into(), Value::String(class.name.clone()))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))));
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionClassConstant") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionclassconstant::__construct", arguments, &reference_names, &argument_names)?;
                let requested = arguments.first().ok_or("ReflectionClassConstant::__construct() expects exactly 2 arguments")?;
                let target = match requested.dereference() {
                    Value::Object(object) => object.borrow().class_name.clone(),
                    value => value.show(),
                };
                let constant_name = arguments.get(1).ok_or("ReflectionClassConstant::__construct() expects exactly 2 arguments")?.show();
                let (_, constant) = find_class_constant(&target, &constant_name, classes)
                    .ok_or_else(|| format!("ReflectionException:Constant {target}::{constant_name} does not exist"))?;
                let declaring = classes.get(&constant.declaring_class).map_or_else(|| constant.declaring_class.clone(), |class| class.name.clone());
                stack.push(Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionClassConstant".into(),
                    properties: HashMap::from([("\0target_class".into(), Value::String(target)), ("\0constant_name".into(), Value::String(constant_name)), ("\0declaring_class".into(), Value::String(declaring))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))));
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionConstant") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionconstant::__construct", arguments, &reference_names, &argument_names)?;
                let requested_value = arguments.first().ok_or("ReflectionConstant::__construct() expects exactly 1 argument")?;
                let requested = requested_value.show();
                if !super::constant_exists(&requested, global_variables) {
                    return Err(format!("ReflectionException:Constant {requested} does not exist"));
                }
                stack.push(Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionConstant".into(),
                    properties: HashMap::from([("\0constant_name".into(), Value::String(requested.trim_start_matches('\\').to_string()))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))));
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionClass") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionclass::__construct", arguments, &reference_names, &argument_names)?;
                let requested_value = arguments.first().ok_or("ReflectionClass::__construct() expects exactly 1 argument")?;
                let requested = match requested_value.dereference() {
                    Value::Object(object) => object.borrow().class_name.clone(),
                    value => value.show(),
                };
                let class = classes.get(&requested.to_ascii_lowercase()).ok_or_else(|| format!("Class \"{requested}\" does not exist"))?;
                let object = Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionClass".into(),
                    properties: HashMap::from([("\0target_class".into(), Value::String(class.name.clone()))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                })));
                stack.push(object);
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionObject") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionobject::__construct", arguments, &reference_names, &argument_names)?;
                let requested_value = arguments.first().ok_or("ReflectionObject::__construct() expects exactly 1 argument")?;
                let Value::Object(reflected) = requested_value.dereference() else {
                    return Err(format!("TypeError:ReflectionObject::__construct(): Argument #1 ($object) must be of type object, {} given", requested_value.type_name()));
                };
                let target = reflected.borrow().class_name.clone();
                stack.push(Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionObject".into(),
                    properties: HashMap::from([("\0target_class".into(), Value::String(target))]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))));
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionFunction") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionfunction::__construct", arguments, &reference_names, &argument_names)?;
                let requested_value = arguments.first().ok_or("ReflectionFunction::__construct() expects exactly 1 argument")?;
                let (requested, closure) = match requested_value.dereference() {
                    Value::Closure(_) => ("Closure".to_string(), Some(requested_value.clone())),
                    value => (value.show(), None),
                };
                if closure.is_none() && !functions.contains_key(&requested.to_ascii_lowercase()) {
                    return Err(format!("Function {requested}() does not exist"));
                }
                let mut properties = HashMap::from([("\0function_name".into(), Value::String(requested))]);
                if let Some(closure) = closure { properties.insert("\0closure".into(), closure); }
                stack.push(Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionFunction".into(),
                    properties,
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))));
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionParameter") {
                let (arguments, _) = super::builtin_arguments::bind("reflectionparameter::__construct", arguments, &reference_names, &argument_names)?;
                let callable = arguments.first().ok_or("ReflectionParameter::__construct() expects exactly 2 arguments")?;
                let selector = arguments.get(1).ok_or("ReflectionParameter::__construct() expects exactly 2 arguments")?;
                let (function, mut origin) = match callable.dereference() {
                    Value::Closure(closure) => {
                        let function = closure.borrow().function.clone();
                        (function, HashMap::from([
                            ("\0function_name".into(), Value::String("Closure".into())),
                            ("\0closure".into(), callable.clone()),
                        ]))
                    }
                    Value::Object(object) => {
                        let class_name = object.borrow().class_name.clone();
                        let function = find_method(&class_name, "__invoke", classes)
                            .ok_or_else(|| format!("ReflectionException:Object of class {class_name} is not a callable"))?.clone();
                        let declaring = function.declaring_class.clone().unwrap_or_else(|| class_name.clone());
                        (function, HashMap::from([
                            ("\0target_class".into(), Value::String(declaring)),
                            ("\0method_name".into(), Value::String("__invoke".into())),
                        ]))
                    }
                    Value::Array(items) if items.len() == 2 && items[0].0 == ArrayKey::Integer(0) && items[1].0 == ArrayKey::Integer(1) => {
                        let receiver = &items[0].1;
                        let method_name = match items[1].1.dereference() {
                            Value::String(method) => method,
                            value => return Err(format!("TypeError:ReflectionParameter::__construct(): Callable method name must be a string, {} given", value.type_name())),
                        };
                        let class_name = match receiver.dereference() {
                            Value::Object(object) => object.borrow().class_name.clone(),
                            Value::String(class) => class,
                            value => return Err(format!("TypeError:ReflectionParameter::__construct(): Callable class must be a string or object, {} given", value.type_name())),
                        };
                        let function = find_method(&class_name, &method_name.to_ascii_lowercase(), classes)
                            .ok_or_else(|| format!("ReflectionException:Method {class_name}::{method_name}() does not exist"))?.clone();
                        let declaring = function.declaring_class.clone().unwrap_or_else(|| class_name.clone());
                        (function.clone(), HashMap::from([
                            ("\0target_class".into(), Value::String(declaring)),
                            ("\0method_name".into(), Value::String(function.display_name.to_ascii_lowercase())),
                        ]))
                    }
                    Value::Array(_) => return Err("ReflectionException:Callable must be a two-element array".into()),
                    Value::String(requested) => {
                        if let Some((class_name, method_name)) = requested.split_once("::") {
                            let function = find_method(class_name, &method_name.to_ascii_lowercase(), classes)
                                .ok_or_else(|| format!("ReflectionException:Method {class_name}::{method_name}() does not exist"))?.clone();
                            let declaring = function.declaring_class.clone().unwrap_or_else(|| class_name.to_string());
                            (function.clone(), HashMap::from([
                                ("\0target_class".into(), Value::String(declaring)),
                                ("\0method_name".into(), Value::String(function.display_name.to_ascii_lowercase())),
                            ]))
                        } else {
                            let function = functions.get(&requested.to_ascii_lowercase())
                                .ok_or_else(|| format!("ReflectionException:Function {requested}() does not exist"))?.clone();
                            let function_name = function.display_name.clone();
                            (function, HashMap::from([("\0function_name".into(), Value::String(function_name))]))
                        }
                    }
                    value => return Err(format!("TypeError:ReflectionParameter::__construct(): Argument #1 ($function) must be of type array|string|object, {} given", value.type_name())),
                };
                let position = match selector.dereference() {
                    Value::Integer(position) if position >= 0 => position as usize,
                    Value::Integer(_) => return Err("ReflectionException:Parameter number must be a non-negative integer".into()),
                    Value::String(name) => function.parameters.iter().position(|parameter| parameter == &name)
                        .ok_or_else(|| format!("ReflectionException:Parameter ${name} does not exist"))?,
                    value => return Err(format!("TypeError:ReflectionParameter::__construct(): Argument #2 ($param) must be of type string|int, {} given", value.type_name())),
                };
                if position >= function.parameters.len() {
                    return Err(format!("ReflectionException:Parameter number {position} does not exist"));
                }
                // The parameter's declaring class is the declaring class of inherited methods.
                if let Some(Value::String(target)) = origin.get("\0target_class") {
                    if let Some(declaring) = classes.get(&target.to_ascii_lowercase()).map(|class| class.name.clone()) {
                        origin.insert("\0target_class".into(), Value::String(declaring));
                    }
                }
                stack.push(reflection_parameter_object(&function, position, origin));
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("ReflectionMethod") {
                let single_argument_form = arguments.len() == 1;
                let (arguments, _) = super::builtin_arguments::bind("reflectionmethod::__construct", arguments, &reference_names, &argument_names)?;
                let requested = arguments.first().ok_or("ReflectionMethod::__construct() expects at least 1 argument")?;
                let (class_name, method_name) = match requested.dereference() {
                    Value::Object(object) => {
                        if single_argument_form { return Err("ReflectionMethod::__construct() expects exactly 2 arguments".into()); }
                        let class_name = object.borrow().class_name.clone();
                        let method_name = match arguments.get(1).map(Value::dereference) {
                            Some(Value::String(method)) => method,
                            Some(value) => return Err(format!("TypeError:ReflectionMethod::__construct(): Argument #2 ($method) must be of type string, {} given", value.type_name())),
                            None => return Err("ReflectionMethod::__construct() expects exactly 2 arguments".into()),
                        };
                        (class_name, method_name)
                    }
                    Value::String(combined) if single_argument_form => {
                        let Some((class_name, method_name)) = combined.split_once("::") else {
                            return Err("ReflectionException:Method name must be a class name followed by :: and a method name".into());
                        };
                        if class_name.is_empty() || method_name.is_empty() {
                            return Err("ReflectionException:Method name must be a class name followed by :: and a method name".into());
                        }
                        global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                            level: crate::DiagnosticLevel::Deprecated,
                            message: "Calling ReflectionMethod::__construct() with 1 argument is deprecated since 8.4".into(),
                            source_file: Some(global_variables.source_file.clone()),
                            source_line: *source_line,
                            output_offset: 0,
                        });
                        (class_name.to_string(), method_name.to_string())
                    }
                    Value::String(class_name) => {
                        let method_name = match arguments.get(1).map(Value::dereference) {
                            Some(Value::String(method)) => method,
                            Some(value) => return Err(format!("TypeError:ReflectionMethod::__construct(): Argument #2 ($method) must be of type string, {} given", value.type_name())),
                            None => return Err("ReflectionMethod::__construct() expects exactly 2 arguments".into()),
                        };
                        (class_name, method_name)
                    }
                    value => return Err(format!("TypeError:ReflectionMethod::__construct(): Argument #1 ($objectOrMethod) must be of type object|string, {} given", value.type_name())),
                };
                let class = classes.get(&class_name.to_ascii_lowercase())
                    .ok_or_else(|| format!("ReflectionException:Class {class_name} does not exist"))?;
                let function = super::objects::find_method(&class_name, &method_name.to_ascii_lowercase(), classes)
                    .ok_or_else(|| format!("ReflectionException:Method {class_name}::{method_name}() does not exist"))?;
                let declaring_class = function.declaring_class.as_deref()
                    .and_then(|name| classes.get(&name.to_ascii_lowercase()))
                    .map_or_else(|| class.name.clone(), |declaring| declaring.name.clone());
                stack.push(Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "ReflectionMethod".into(),
                    properties: HashMap::from([
                        ("\0target_class".into(), Value::String(declaring_class)),
                        ("\0method_name".into(), Value::String(function.display_name.clone())),
                    ]),
                    property_order: Vec::new(), active_property_hooks: Default::default(), exception: None,
                }))));
                return Ok(true);
            }
            if classes.get(&name.to_ascii_lowercase()).is_some_and(|class| class.is_enum) {
                return Err(format!("Cannot instantiate enum {name}"));
            }
            if name.eq_ignore_ascii_case("PDO") {
                let (arguments, _) = super::builtin_arguments::bind("pdo::__construct", arguments, &reference_names, &argument_names)?;
                stack.push(crate::pdo::construct(&arguments)?);
                return Ok(true);
            }
            if !classes.contains_key(&name.to_ascii_lowercase()) {
                if let Some(class_name) = crate::exception_builtins::canonical_name(name)
                    .or_else(|| name.rsplit('\\').next().and_then(crate::exception_builtins::canonical_name)) {
                    let (arguments, _) = super::builtin_arguments::bind(
                        &class_name.to_ascii_lowercase(),
                        arguments,
                        &reference_names,
                        &argument_names,
                    )?;
                    let object = crate::exception_builtins::construct(class_name, &arguments, classes)?;
                    set_exception_source_location(&object, *source_line, &global_variables.source_file, frame_scope, variables.contains_key("this"), global_variables);
                    stack.push(object);
                    return Ok(true);
                }
            }
            if crate::exception_builtins::is_interface(name)
                || name.rsplit('\\').next().is_some_and(crate::exception_builtins::is_interface) {
                return Err("Cannot instantiate interface Throwable".into());
            }
            if name.eq_ignore_ascii_case("MongoDB\\Client") {
                let arguments = super::builtin_arguments::bind("mongodb\\client::__construct", arguments, &reference_names, &argument_names)?.0;
                stack.push(crate::mongo_php::construct(&name, &arguments)?);
                return Ok(true);
            }
            if name.eq_ignore_ascii_case("stdClass") {
                if !arguments.is_empty() {
                    return Err("Error:Class stdClass does not have a constructor, so you cannot pass any constructor arguments".into());
                }
                let object = Value::Object(Rc::new(RefCell::new(PhpObject {
                    class_name: "stdClass".into(),
                    properties: HashMap::new(),
                    property_order: Vec::new(),
                    active_property_hooks: Default::default(),
                    exception: None,
                })));
                global_variables.register_destructor_object(&object);
                stack.push(object);
                return Ok(true);
            }
            let class = classes.get(&name.to_ascii_lowercase()).ok_or_else(|| format!("Class '{name}' not found"))?;
            if class.is_interface { return Err(format!("Cannot instantiate interface {name}")); }
            if class.is_abstract { return Err(format!("Cannot instantiate abstract class {name}")); }
            let mut properties = HashMap::new();
            // Parent defaults are applied first; properties declared on the concrete class take precedence.
            collect_properties(name, classes, &mut properties)?;
            let mut property_order = Vec::new();
            collect_property_order(name, classes, &mut property_order)?;
            let native_throwable = super::is_a_type(name, "throwable", classes, &mut std::collections::HashSet::new());
            let object = Value::Object(Rc::new(RefCell::new(PhpObject {
                class_name: class.name.clone(), properties, property_order,
                active_property_hooks: Default::default(),
                exception: native_throwable.then(crate::exception_builtins::empty_data),
            })));
            global_variables.register_destructor_object(&object);
            if let Some(constructor) = find_method(name, "__construct", classes) {
                ensure_access(constructor.visibility, constructor.declaring_class.as_deref(), access_scope.as_deref(), classes, "__construct")?;
                let (mut locals, parameter_bindings) = super::functions::prepare_reference_arguments(
                    arguments, &reference_names, constructor, &argument_names, &format!("{name}::__construct()"),
                    variables, bindings, global_variables, global_scope, frame_id, classes, *strict_types,
                )?;
                locals.insert("this".to_string(), object.clone());
                locals.insert("\0static_class".to_string(), Value::String(name.clone()));
                super::initialize_local_order(&mut locals, std::iter::once("this".to_string()).chain(constructor.parameters.iter().cloned()));
                super::functions::execute_user_frame(&constructor.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(constructor), parameter_bindings, output, steps)?;
                finish_object_frame(locals, &Value::Null, functions, classes, static_properties, global_variables, output, steps)?;
            } else if native_throwable {
                let (arguments, _) = super::builtin_arguments::bind("exception", arguments, &reference_names, &argument_names)?;
                crate::exception_builtins::initialize_object(&object, name, &arguments, classes)?;
            } else if !arguments.is_empty() {
                return Err(format!("Class '{name}' has no constructor"));
            }
            set_exception_source_location(&object, *source_line, &global_variables.source_file, frame_scope, variables.contains_key("this"), global_variables);
            stack.push(object);
        }
        // A visible, initialized declaration wins; otherwise PHP property overloading may handle the read.
        Op::GetProperty(property, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?);
        }
        Op::GetPropertyCoalesce(property, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(read_property_for_coalesce(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?);
        }
        Op::GetDynamicProperty(access_scope) => {
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?);
        }
        Op::GetDynamicPropertyCoalesce(access_scope) => {
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            stack.push(read_property_for_coalesce(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?);
        }
        Op::SetDynamicProperty(access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            assign_property(&receiver, &property, value.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::BindPropertyAppendReference(property, access_scope, in_constructor) => {
            let Value::Reference(reference) = stack.pop().ok_or("Invalid bytecode stack")? else {
                return Err("Only a reference-returning property can be appended by reference".into());
            };
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let updated = current.array_append_reference(reference)?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
        }
        Op::BindPropertyArrayReference(property, target_key_count, source, source_key_count, access_scope, in_constructor) => {
            let key_count = target_key_count + source_key_count;
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let target_keys = raw_keys[..*target_key_count].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let source_keys = raw_keys[*target_key_count..].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let (_, reference) = super::reference_cell_for_path(source, &source_keys, variables, bindings, global_variables, global_scope, frame_id)?;
            let current = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let updated = current.bind_array_reference(&target_keys, reference)?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
        }
        Op::LoadPropertyReference(property, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(object) = &receiver else { return Err("Only object properties can be returned by reference".into()); };
            let class_name = object.borrow().class_name.clone();
            if classes.get(&class_name.to_ascii_lowercase()).is_some_and(|class| class.is_enum) {
                return Err(format!("Cannot modify enum property {class_name}::${property}"));
            }
            let metadata = find_property_in_scope(&class_name, property, access_scope.as_deref(), classes);
            let directly_accessible = metadata.map_or_else(
                || object.borrow().properties.contains_key(property) || class_name.eq_ignore_ascii_case("stdclass"),
                |metadata| ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).is_ok(),
            );
            if !directly_accessible {
                if let Some(getter) = find_method(&class_name, "__get", classes) {
                    let value = call_magic(&receiver, getter, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?;
                    if !matches!(value, Value::Reference(_)) {
                        return Err(format!("Cannot assign by reference to overloaded property {class_name}::${property}; __get() must return by reference"));
                    }
                    if let Some(metadata) = metadata.filter(|metadata| metadata.type_name.is_some()) {
                        ensure_magic_get_property_type(&value, &class_name, property, metadata, classes)?;
                    }
                    stack.push(value);
                    return Ok(true);
                }
                if let Some(metadata) = metadata {
                    ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
                }
            }
            if let Some(metadata) = metadata.filter(|_| directly_accessible) {
                if metadata.is_readonly {
                    return Err(format!("Cannot indirectly modify readonly property {class_name}::${property}"));
                }
                if metadata.type_name.is_some() {
                    return Err(format!("Typed property {class_name}::${property} must not be referenced"));
                }
            }
            let storage_key = metadata.map_or_else(|| property.clone(), |metadata| instance_property_storage_key(metadata, property));
            let mut object = object.borrow_mut();
            let reference = match object.properties.get(&storage_key) {
                Some(Value::Reference(reference)) => reference.clone(),
                value => Rc::new(RefCell::new(value.cloned().unwrap_or(Value::Null).dereference())),
            };
            object.properties.insert(storage_key, Value::Reference(reference.clone()));
            stack.push(Value::Reference(reference));
        }
        Op::LoadPropertyArrayReference(property, key_count, access_scope) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let root = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let (updated, reference) = root.ensure_array_reference(&keys)?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), false, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(Value::Reference(reference));
        }
        // Writes to inaccessible or undeclared properties are forwarded to __set when available.
        Op::SetProperty(property, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let previous = existing_property_for_assignment(&receiver, property, access_scope.as_deref(), classes);
            assign_property(&receiver, property, value.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            if let Some(previous) = previous {
                run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
            }
        }
        Op::AssignPropertyExpr(property, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let previous = existing_property_for_assignment(&receiver, property, access_scope.as_deref(), classes);
            assign_property(&receiver, property, value.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            if let Some(previous) = previous {
                run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
            }
            stack.push(value);
        }
        Op::CompoundAssignProperty(property, operation, access_scope, in_constructor) => {
            let right = stack.pop().ok_or("Invalid bytecode stack")?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(object) = &receiver else { return Err("Attempt to assign property on non-object".into()); };
            let class_name = object.borrow().class_name.clone();
            let metadata = find_property_in_scope(&class_name, property, access_scope.as_deref(), classes);
            let readable = metadata.is_some_and(|metadata| {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).is_ok()
            });
            let current = if readable {
                let metadata = metadata.expect("readable property has metadata");
                let storage_key = instance_property_storage_key(metadata, property);
                if !object.borrow().properties.contains_key(&storage_key) && metadata.type_name.is_some() {
                    return Err(format!("Typed property {class_name}::${property} must not be accessed before initialization"));
                }
                object.borrow().properties.get(&storage_key).cloned().unwrap_or(Value::Null)
            } else if metadata.is_none() && object.borrow().properties.contains_key(property) {
                object.borrow().properties.get(property).cloned().unwrap_or(Value::Null)
            } else if let Some(magic) = find_method(&class_name, "__get", classes) {
                call_magic(&receiver, magic, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?
            } else if let Some(metadata) = metadata {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
                unreachable!();
            } else { Value::Null };
            let operator = match operation {
                CompoundOperation::Add => Op::Add,
                CompoundOperation::Subtract => Op::Sub,
                CompoundOperation::Multiply => Op::Mul,
                CompoundOperation::Power => Op::Pow,
                CompoundOperation::Divide => Op::Div,
                CompoundOperation::Modulo => Op::Mod,
                CompoundOperation::Concatenate => Op::Concat,
                    CompoundOperation::BitAnd => Op::BitAnd,
                    CompoundOperation::BitXor => Op::BitXor,
                    CompoundOperation::BitOr => Op::BitOr,
                    CompoundOperation::ShiftLeft => Op::ShiftLeft,
                    CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let updated = super::operators::apply_binary(&operator, current, right)?;
            assign_property(&receiver, property, updated.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(updated);
        }
        Op::CompoundAssignDynamicProperty(operation, access_scope, in_constructor) => {
            let right = stack.pop().ok_or("Invalid bytecode stack")?;
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let operator = match operation {
                CompoundOperation::Add => Op::Add,
                CompoundOperation::Subtract => Op::Sub,
                CompoundOperation::Multiply => Op::Mul,
                CompoundOperation::Power => Op::Pow,
                CompoundOperation::Divide => Op::Div,
                CompoundOperation::Modulo => Op::Mod,
                CompoundOperation::Concatenate => Op::Concat,
                CompoundOperation::BitAnd => Op::BitAnd,
                CompoundOperation::BitXor => Op::BitXor,
                CompoundOperation::BitOr => Op::BitOr,
                CompoundOperation::ShiftLeft => Op::ShiftLeft,
                CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let updated = super::operators::apply_binary(&operator, current, right)?;
            assign_property(&receiver, &property, updated.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(updated);
        }
        Op::IncrementProperty(property, delta, post, access_scope, in_constructor) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let old_value = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let updated = super::increment_value(&old_value, *delta, global_variables)?;
            assign_property(&receiver, property, updated.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(if *post { old_value } else { updated });
        }
        Op::IncrementDynamicProperty(delta, post, access_scope, in_constructor) => {
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let old_value = read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let updated = super::increment_value(&old_value, *delta, global_variables)?;
            assign_property(&receiver, &property, updated.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(if *post { old_value } else { updated });
        }
        Op::IncrementDynamicPropertyArrayPath(key_count, delta, post, access_scope, in_constructor) => {
            if stack.len() < key_count + 2 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let old_root = read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            super::warn_undefined_array_key(&old_root, &keys, global_variables);
            let old_value = old_root.array_get_path(&keys)?;
            let updated_value = super::increment_value(&old_value, *delta, global_variables)?;
            let updated_root = old_root.array_set_path(&keys, updated_value.clone())?;
            assign_property(&receiver, &property, updated_root, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(if *post { old_value } else { updated_value });
        }
        Op::IncrementPropertyArrayPath(property, key_count, delta, post, access_scope, in_constructor) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let old_root = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties,
                global_variables, output, steps)?;
            super::warn_undefined_array_key(&old_root, &keys, global_variables);
            let old_value = old_root.array_get_path(&keys)?;
            let updated_value = super::increment_value(&old_value, *delta, global_variables)?;
            let updated_root = old_root.array_set_path(&keys, updated_value.clone())?;
            assign_property(&receiver, property, updated_root, access_scope.as_deref(), *in_constructor,
                functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(if *post { old_value } else { updated_value });
        }
        Op::IncrementDynamicStaticPropertyArrayPath(key_count, delta, post, access_scope) => {
            if stack.len() < key_count + 2 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let class_value = stack.pop().ok_or("Invalid bytecode stack")?;
            let class_name = match class_value.dereference() {
                Value::Object(object) => object.borrow().class_name.clone(),
                Value::String(name) => name,
                other => return Err(format!("Cannot access static property on {}", other.type_name())),
            };
            let resolved_class = resolve_static_class(&class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, &property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, &property)?;
            let storage_key = (owner.to_string(), property.clone());
            let old_root = static_properties.get(&storage_key).cloned().unwrap_or(Value::Null);
            super::warn_undefined_array_key(&old_root, &keys, global_variables);
            let old_value = if keys.is_empty() { old_root.clone() } else { old_root.array_get_path(&keys)? };
            let updated_value = super::increment_value(&old_value, *delta, global_variables)?;
            let updated_root = if keys.is_empty() { updated_value.clone() } else { old_root.array_set_path(&keys, updated_value.clone())? };
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated_root, type_name, &property, classes)?; }
            store_static_property(static_properties, storage_key, updated_root);
            stack.push(if *post { old_value } else { updated_value });
        }
        Op::IncrementStaticProperty(class_name, property, delta, post, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let old_value = match static_properties.get(&key) {
                Some(value) => value.clone(),
                None if metadata.type_name.is_some() => return Err(format!("Typed static property {resolved_class}::${property} must not be accessed before initialization")),
                None => Value::Null,
            };
            let updated = super::increment_value(&old_value, *delta, global_variables)?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated, type_name, property, classes)?; }
            store_static_property(static_properties, key, updated.clone());
            stack.push(if *post { old_value } else { updated });
        }
        Op::IncrementStaticPropertyArrayPath(class_name, property, key_count, delta, post, access_scope) => {
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let old_root = match static_properties.get(&key) {
                Some(value) => value.clone(),
                None if metadata.type_name.is_some() => return Err(format!("Typed static property {resolved_class}::${property} must not be accessed before initialization")),
                None => Value::Null,
            };
            super::warn_undefined_array_key(&old_root, &keys, global_variables);
            let old_value = old_root.array_get_path(&keys)?;
            let updated_value = super::increment_value(&old_value, *delta, global_variables)?;
            let updated_root = old_root.array_set_path(&keys, updated_value.clone())?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated_root, type_name, property, classes)?; }
            store_static_property(static_properties, key, updated_root);
            stack.push(if *post { old_value } else { updated_value });
        }
        Op::CoalescePropertyJump(target, property, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property_for_coalesce(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            if matches!(current, Value::Null) { stack.push(receiver); }
            else { stack.push(current); *program_counter = *target; }
        }
        Op::CoalesceAssignProperty(property, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            assign_property(&receiver, property, value.clone(), access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::CoalescePropertyArrayJump(target, property, key_count, access_scope) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let split = stack.len() - key_count;
            let raw_keys = stack.split_off(split);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property_for_coalesce(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let value = current.array_get_path(&keys)?;
            if matches!(value, Value::Null) {
                stack.push(receiver);
                stack.extend(raw_keys);
            } else {
                stack.push(value);
                *program_counter = *target;
            }
        }
        Op::CoalesceAssignPropertyArray(property, key_count, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property_for_coalesce(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let updated = current.clone().array_set_path(&keys, value.clone())?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::AssignPropertyArray(property, key_count, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let previous = current.array_get_path(&keys)?;
            let updated = current.clone().array_set_path(&keys, value.clone())?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            drop(current);
            run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::CompoundAssignPropertyArray(property, key_count, operation, access_scope, in_constructor) => {
            let right = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let current_value = current.array_get_path(&keys)?;
            let operator = match operation {
                CompoundOperation::Add => Op::Add,
                CompoundOperation::Subtract => Op::Sub,
                CompoundOperation::Multiply => Op::Mul,
                CompoundOperation::Power => Op::Pow,
                CompoundOperation::Divide => Op::Div,
                CompoundOperation::Modulo => Op::Mod,
                CompoundOperation::Concatenate => Op::Concat,
                CompoundOperation::BitAnd => Op::BitAnd,
                CompoundOperation::BitXor => Op::BitXor,
                CompoundOperation::BitOr => Op::BitOr,
                CompoundOperation::ShiftLeft => Op::ShiftLeft,
                CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let value = super::operators::apply_binary(&operator, current_value.clone(), right)?;
            let updated = current.clone().array_set_path(&keys, value.clone())?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            run_destructors_removed_from_value(current_value, &value, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::AppendProperty(property, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?
                .array_append(value.clone())?;
            assign_property(&receiver, property, current, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::AppendPropertyArray(property, key_count, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let updated = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?
                .array_append_path(&keys, value.clone())?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::AppendDynamicPropertyArray(key_count, access_scope, in_constructor) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 2 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let updated = read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?
                .array_append_path(&keys, value.clone())?;
            assign_property(&receiver, &property, updated, access_scope.as_deref(), *in_constructor, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::CloneObject => {
            let original = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(original_object) = &original else { return Err("clone expects an object".into()); };
            // Clone the property map; nested object handles remain shared, matching a shallow PHP clone.
            let class_name = original_object.borrow().class_name.clone();
            if classes.get(&class_name.to_ascii_lowercase()).is_some_and(|class| class.is_enum) {
                return Err(format!("Trying to clone an uncloneable object of class {class_name}"));
            }
            let cloned_object = Value::Object(Rc::new(RefCell::new(original_object.borrow().clone())));
            global_variables.register_destructor_object(&cloned_object);
            if let Some(hook) = find_method(&class_name, "__clone", classes) {
                if hook.is_static || !hook.parameters.is_empty() {
                    return Err(format!("{class_name}::__clone() must be a non-static method without parameters"));
                }
                let mut locals = HashMap::from([
                    ("this".to_string(), cloned_object.clone()),
                    ("\0static_class".to_string(), Value::String(class_name)),
                ]);
                // PHP 8.3 permits readonly properties to be initialized again by __clone().
                // They are unset on the clone before the hook runs, so the normal readonly
                // write check can allow a single write from the declaring scope.
                let mut readonly_slots = Vec::new();
                let mut class_cursor = Some(original_object.borrow().class_name.clone());
                while let Some(name) = class_cursor {
                    let Some(metadata) = classes.get(&name.to_ascii_lowercase()) else { break; };
                    for (property_name, property) in &metadata.properties {
                        if property.is_readonly {
                            readonly_slots.push(instance_property_storage_key(property, property_name));
                        }
                    }
                    class_cursor = metadata.parent.clone();
                }
                if let Value::Object(object) = &cloned_object {
                    let mut object = object.borrow_mut();
                    for slot in readonly_slots { object.properties.remove(&slot); }
                }
                super::initialize_local_order(&mut locals, ["this".to_string()]);
                let clone_id = match &cloned_object { Value::Object(object) => Rc::as_ptr(object) as usize, _ => unreachable!() };
                global_variables.readonly_clone_objects.push(clone_id);
                let hook_result = super::functions::execute_user_frame(&hook.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(hook), HashMap::new(), output, steps);
                if let Some(position) = global_variables.readonly_clone_objects.iter().rposition(|id| *id == clone_id) {
                    global_variables.readonly_clone_objects.remove(position);
                }
                let result = hook_result?.unwrap_or(Value::Null);
                let validated_result = validate_return_type(result, hook, classes)?;
                finish_object_frame(locals, &validated_result, functions, classes, static_properties, global_variables, output, steps)?;
            }
            stack.push(cloned_object);
        }
        // isset treats null/uninitialized declarations as false and delegates overloaded properties.
        Op::IssetProperty(property, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(object) = &receiver else { stack.push(Value::Bool(false)); return Ok(true); };
            let class_name = object.borrow().class_name.clone();
            let metadata = find_property_in_scope(&class_name, property, access_scope.as_deref(), classes);
            let directly_accessible = metadata.is_some_and(|metadata| {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).is_ok()
            });
            if directly_accessible {
                let key = instance_property_storage_key(metadata.expect("accessible property metadata"), property);
                let present = object.borrow().properties.get(&key).is_some_and(|value| !matches!(value, Value::Null));
                stack.push(Value::Bool(present));
            } else if metadata.is_none() && object.borrow().properties.contains_key(property) {
                let present = object.borrow().properties.get(property).is_some_and(|value| !matches!(value, Value::Null));
                stack.push(Value::Bool(present));
            } else if let Some(magic) = find_method(&class_name, "__isset", classes) {
                let result = call_magic(&receiver, magic, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?;
                stack.push(Value::Bool(result.truthy()));
            } else {
                stack.push(Value::Bool(false));
            }
        }
        Op::IssetChainProperty(property, terminal, failure_target, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let set_value = if let Value::Object(object) = receiver.dereference() {
                let receiver = Value::Object(object.clone());
                let class_name = object.borrow().class_name.clone();
                let metadata = find_property_in_scope(&class_name, property, access_scope.as_deref(), classes);
                let direct = metadata.is_some_and(|metadata| ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).is_ok());
                let direct_value = if direct {
                    let key = instance_property_storage_key(metadata.expect("direct property metadata"), property);
                    object.borrow().properties.get(&key).cloned()
                } else if metadata.is_none() { object.borrow().properties.get(property).cloned() } else { None };
                let is_direct = direct || direct_value.is_some();
                if is_direct {
                    direct_value.filter(|value| !matches!(value.dereference(), Value::Null)).map(|value| (true, value))
                } else if let Some(magic) = find_method(&class_name, "__isset", classes) {
                    call_magic(&receiver, magic, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?
                        .truthy().then(|| (false, Value::Null))
                } else { None }
            } else { None };
            if let Some((is_direct, value)) = set_value {
                if *terminal { stack.push(Value::Bool(true)); }
                else {
                    let value = if is_direct { value } else {
                        let Value::Object(_) = receiver.dereference() else { unreachable!() };
                        read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?
                    };
                    stack.push(value);
                }
            } else {
                stack.push(Value::Bool(false));
                *program_counter = *failure_target;
            }
        }
        Op::IssetChainDynamicProperty(terminal, failure_target, access_scope) => {
            let property = stack.pop().ok_or("Missing dynamic property name")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            // Reuse the fixed-property semantics after resolving the name at this point in evaluation.
            let instruction = Op::IssetChainProperty(property, *terminal, *failure_target, access_scope.clone());
            stack.push(receiver);
            return execute_instruction(&instruction, stack, variables, bindings, global_variables, global_scope, frame_id, frame_scope,
                functions, classes, static_properties, output, steps, program_counter);
        }
        Op::IssetChainArrayOffsets(key_count, terminal, failure_target) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(isset_array_key).collect::<Result<Vec<_>, _>>()?;
            let current = stack.pop().ok_or("Invalid bytecode stack")?;
            match current.array_get_path(&keys) {
                Ok(value) if !matches!(value.dereference(), Value::Null) => {
                    if *terminal { stack.push(Value::Bool(true)); } else { stack.push(value); }
                }
                _ => { stack.push(Value::Bool(false)); *program_counter = *failure_target; }
            }
        }
        Op::IssetPropertyArray(property, key_count, access_scope) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(isset_array_key).collect::<Result<Vec<_>, _>>()?;
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(object) = &receiver else { stack.push(Value::Bool(false)); return Ok(true); };
            let class_name = object.borrow().class_name.clone();
            let metadata = find_property_in_scope(&class_name, property, access_scope.as_deref(), classes);
            let directly_accessible = metadata.is_some_and(|metadata| {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).is_ok()
            });
            let base_is_set = if directly_accessible {
                let key = instance_property_storage_key(metadata.expect("accessible property metadata"), property);
                object.borrow().properties.get(&key).is_some_and(|value| !matches!(value, Value::Null))
            } else if metadata.is_none() && object.borrow().properties.contains_key(property) {
                object.borrow().properties.get(property).is_some_and(|value| !matches!(value, Value::Null))
            } else if let Some(magic) = find_method(&class_name, "__isset", classes) {
                call_magic(&receiver, magic, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?.truthy()
            } else {
                false
            };
            let is_set = if base_is_set {
                match read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps) {
                    Ok(value) => value.array_get_path(&keys).is_ok_and(|value| !matches!(value, Value::Null)),
                    Err(_) => false,
                }
            } else { false };
            stack.push(Value::Bool(is_set));
        }
        Op::IssetStaticProperty(class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let value = static_property_owner(&resolved_class, property, classes).and_then(|(owner, metadata)| {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).ok()?;
                static_properties.get(&(owner.to_string(), property.clone())).cloned()
            }).unwrap_or(Value::Null);
            stack.push(Value::Bool(!matches!(value, Value::Null)));
        }
        Op::IssetDynamicProperty(access_scope) => {
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let value = Value::Null;
            let Value::Object(object) = &receiver else { stack.push(Value::Bool(false)); return Ok(true); };
            let class_name = object.borrow().class_name.clone();
            let metadata = find_property_in_scope(&class_name, &property, access_scope.as_deref(), classes);
            let accessible = metadata.is_some_and(|metadata| ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, &property).is_ok());
            if accessible || (metadata.is_none() && object.borrow().properties.contains_key(&property)) {
                let key = metadata.map_or_else(|| property.clone(), |metadata| instance_property_storage_key(metadata, &property));
                let present = object.borrow().properties.get(&key).is_some_and(|value| !matches!(value, Value::Null));
                stack.push(Value::Bool(present));
            } else if let Some(magic) = find_method(&class_name, "__isset", classes) {
                let result = call_magic(&receiver, magic, vec![Value::String(property)], functions, classes, static_properties, global_variables, output, steps)?;
                stack.push(Value::Bool(result.truthy()));
            } else { stack.push(value); }
        }
        Op::IssetDynamicPropertyArray(key_count, access_scope) => {
            if stack.len() < key_count + 2 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(isset_array_key).collect::<Result<Vec<_>, _>>()?;
            let property = stack.pop().ok_or("Missing dynamic property name")?.show();
            let receiver = stack.pop().ok_or("Missing dynamic property receiver")?;
            let Value::Object(object) = &receiver else { stack.push(Value::Bool(false)); return Ok(true); };
            let class_name = object.borrow().class_name.clone();
            let metadata = find_property_in_scope(&class_name, &property, access_scope.as_deref(), classes);
            let directly_accessible = metadata.is_some_and(|metadata| {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, &property).is_ok()
            });
            let base_is_set = if directly_accessible {
                let key = instance_property_storage_key(metadata.expect("accessible property metadata"), &property);
                object.borrow().properties.get(&key).is_some_and(|value| !matches!(value, Value::Null))
            } else if metadata.is_none() && object.borrow().properties.contains_key(&property) {
                object.borrow().properties.get(&property).is_some_and(|value| !matches!(value, Value::Null))
            } else if let Some(magic) = find_method(&class_name, "__isset", classes) {
                call_magic(&receiver, magic, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?.truthy()
            } else { false };
            let is_set = if base_is_set {
                read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)
                    .ok().is_some_and(|value| value.array_get_path(&keys).is_ok_and(|value| !matches!(value, Value::Null)))
            } else { false };
            stack.push(Value::Bool(is_set));
        }
        // Only directly accessible properties are removed here; inaccessible ones use __unset.
        Op::UnsetProperty(property, access_scope) => {
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(object) = &receiver else { return Err("Attempt to unset property on non-object".into()); };
            let class_name = object.borrow().class_name.clone();
            if classes.get(&class_name.to_ascii_lowercase()).is_some_and(|class| class.is_enum) {
                return Err(format!("Cannot modify enum property {class_name}::${property}"));
            }
            let metadata = find_property_in_scope(&class_name, property, access_scope.as_deref(), classes);
            let directly_accessible = metadata.is_some_and(|metadata| {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property).is_ok()
            });
            if directly_accessible {
                let metadata = metadata.expect("accessible property metadata");
                let storage_key = instance_property_storage_key(metadata, property);
                validate_readonly_unset(object, metadata, &storage_key, property, access_scope.as_deref(), classes)?;
                remove_object_property(object, &storage_key, property);
            } else if metadata.is_none() && object.borrow().properties.contains_key(property) {
                remove_object_property(object, property, property);
            } else if let Some(magic) = find_method(&class_name, "__unset", classes) {
                call_magic(&receiver, magic, vec![Value::String(property.clone())], functions, classes, static_properties, global_variables, output, steps)?;
            }
        }
        Op::UnsetDynamicProperty(access_scope) => {
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let Value::Object(object) = &receiver else { return Ok(true); };
            let class_name = object.borrow().class_name.clone();
            if let Some(metadata) = find_property_in_scope(&class_name, &property, access_scope.as_deref(), classes) {
                ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, &property)?;
                let storage_key = instance_property_storage_key(metadata, &property);
                validate_readonly_unset(object, metadata, &storage_key, &property, access_scope.as_deref(), classes)?;
                remove_object_property(object, &storage_key, &property);
            } else if object.borrow().properties.contains_key(&property) {
                remove_object_property(object, &property, &property);
            } else if let Some(magic) = find_method(&class_name, "__unset", classes) {
                call_magic(&receiver, magic, vec![Value::String(property)], functions, classes, static_properties, global_variables, output, steps)?;
            }
        }
        Op::UnsetPropertyArray(property, key_count, access_scope) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
            let current = read_property(&receiver, property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
            let keys = raw_keys.iter().map(|key| unset_array_key(key, &current)).collect::<Result<Vec<_>, _>>()?;
            let (updated, removed) = current.clone().array_unset_path_with_removed(&keys)?;
            assign_property(&receiver, property, updated, access_scope.as_deref(), false, functions, classes, static_properties, global_variables, output, steps)?;
            drop(current);
            if let Some(value) = removed {
                run_destructors_removed_from_value(value, &Value::Null, functions, classes, static_properties, global_variables, output, steps)?;
            }
        }
        Op::GetDynamicStaticProperty(property) => {
            let class_value = stack.pop().ok_or("Missing dynamic static property class")?;
            let class_name = match class_value.dereference() {
                Value::Object(object) => object.borrow().class_name.clone(),
                Value::String(name) => name,
                other => return Err(format!("Cannot access static property on {}", other.type_name())),
            };
            let resolved_class = resolve_static_class(&class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), None, classes, property)?;
            stack.push(static_properties.get(&(owner.to_string(), property.clone())).cloned().unwrap_or(Value::Null));
        }
        Op::GetDynamicStaticPropertyExpr => {
            let property = stack.pop().ok_or("Missing dynamic static property name")?.show();
            let class_value = stack.pop().ok_or("Missing dynamic static property class")?;
            let class_name = match class_value.dereference() {
                Value::Object(object) => object.borrow().class_name.clone(),
                Value::String(name) => name,
                other => return Err(format!("Cannot access static property on {}", other.type_name())),
            };
            let resolved_class = resolve_static_class(&class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, &property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), None, classes, &property)?;
            let key = (owner.to_string(), property.clone());
            match static_properties.get(&key) {
                Some(value) => stack.push(value.clone()),
                None if metadata.type_name.is_some() => return Err(format!("Typed static property {resolved_class}::${property} must not be accessed before initialization")),
                None => stack.push(Value::Null),
            }
        }
        Op::AssignDynamicStaticPropertyArray(key_count, instruction, access_scope) => {
            let right = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < key_count + 2 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
            let class_value = stack.pop().ok_or("Invalid bytecode stack")?;
            let class_name = match class_value.dereference() {
                Value::Object(object) => object.borrow().class_name.clone(),
                Value::String(name) => name,
                other => return Err(format!("Cannot access static property on {}", other.type_name())),
            };
            let resolved_class = resolve_static_class(&class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, &property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, &property)?;
            let key = (owner.to_string(), property.clone());
            let current = static_properties.get(&key).cloned().unwrap_or(Value::Null);
            let value = if let Some(operation) = instruction {
                let current_value = current.array_get_path(&keys)?;
                let operator = match operation {
                    CompoundOperation::Add => Op::Add,
                    CompoundOperation::Subtract => Op::Sub,
                    CompoundOperation::Multiply => Op::Mul,
                    CompoundOperation::Power => Op::Pow,
                    CompoundOperation::Divide => Op::Div,
                    CompoundOperation::Modulo => Op::Mod,
                    CompoundOperation::Concatenate => Op::Concat,
                    CompoundOperation::BitAnd => Op::BitAnd,
                    CompoundOperation::BitXor => Op::BitXor,
                    CompoundOperation::BitOr => Op::BitOr,
                    CompoundOperation::ShiftLeft => Op::ShiftLeft,
                    CompoundOperation::ShiftRight => Op::ShiftRight,
                };
                super::operators::apply_binary(&operator, current_value, right)?
            } else { right };
            let updated = current.array_set_path(&keys, value.clone())?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated, type_name, &property, classes)?; }
            store_static_property(static_properties, key, updated);
            stack.push(value);
        }
        Op::GetStaticProperty(class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let class_name = resolved_class.as_str();
            let (owner, metadata) = static_property_owner(class_name, property, classes)
                .ok_or_else(|| format!("Undefined static property {class_name}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            match static_properties.get(&key) {
                Some(value) => stack.push(value.clone()),
                None if metadata.type_name.is_some() => return Err(format!("Typed static property {class_name}::${property} must not be accessed before initialization")),
                None => stack.push(Value::Null),
            }
        }
        Op::LoadStaticPropertyReference(class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            if metadata.type_name.is_some() {
                return Err(format!("Typed property {resolved_class}::${property} must not be referenced"));
            }
            let key = (owner.to_string(), property.clone());
            let value = match static_properties.get(&key) {
                Some(Value::Reference(reference)) => reference.clone(),
                Some(value) => Rc::new(RefCell::new(value.clone().dereference())),
                None if metadata.type_name.is_some() => return Err(format!("Typed static property {resolved_class}::${property} must not be accessed before initialization")),
                None => Rc::new(RefCell::new(Value::Null)),
            };
            static_properties.insert(key, Value::Reference(value.clone()));
            stack.push(Value::Reference(value));
        }
        Op::LoadStaticPropertyArrayReference(key_count) => {
            if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let Value::Reference(root) = stack.pop().ok_or("Invalid bytecode stack")? else {
                return Err("Static property array reference requires a reference root".into());
            };
            let (updated, element_reference) = root.borrow().clone().ensure_array_reference(&keys)?;
            *root.borrow_mut() = updated;
            stack.push(Value::Reference(element_reference));
        }
        Op::SetStaticProperty(class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let class_name = resolved_class.as_str();
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let (owner, metadata) = static_property_owner(class_name, property, classes)
                .ok_or_else(|| format!("Undefined static property {class_name}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&value, type_name, property, classes)?; }
            let key = (owner.to_string(), property.clone());
            let previous = static_properties.get(&key).map(Value::dereference);
            store_static_property(static_properties, key, value.clone());
            if let Some(previous) = previous {
                run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
            }
        }
        Op::BindStaticPropertyArrayReference(target_class, target_property, target_key_count, source_class, source_property, source_key_count, access_scope) => {
            let key_count = target_key_count + source_key_count;
            if stack.len() < key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let target_keys = raw_keys[..*target_key_count].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let source_keys = raw_keys[*target_key_count..].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;

            let resolved_source_class = resolve_static_class(source_class, variables)?;
            let (source_owner, source_metadata) = static_property_owner(&resolved_source_class, source_property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_source_class}::${source_property}"))?;
            ensure_access(source_metadata.visibility, Some(&source_metadata.declaring_class), access_scope.as_deref(), classes, source_property)?;
            let source_key = (source_owner.to_string(), source_property.clone());
            let source_root = static_properties.get(&source_key).cloned().unwrap_or(Value::Null);
            let (updated_source, reference) = source_root.ensure_array_reference(&source_keys)?;
            if let Some(type_name) = &source_metadata.type_name { ensure_property_type(&updated_source, type_name, source_property, classes)?; }
            store_static_property(static_properties, source_key, updated_source);

            let resolved_target_class = resolve_static_class(target_class, variables)?;
            let (target_owner, target_metadata) = static_property_owner(&resolved_target_class, target_property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_target_class}::${target_property}"))?;
            ensure_access(target_metadata.visibility, Some(&target_metadata.declaring_class), access_scope.as_deref(), classes, target_property)?;
            let target_key = (target_owner.to_string(), target_property.clone());
            let target_root = static_properties.get(&target_key).cloned().unwrap_or(Value::Null);
            let updated_target = target_root.bind_array_reference(&target_keys, reference)?;
            if let Some(type_name) = &target_metadata.type_name { ensure_property_type(&updated_target, type_name, target_property, classes)?; }
            store_static_property(static_properties, target_key, updated_target);
        }
        Op::AssignStaticPropertyExpr(class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&value, type_name, property, classes)?; }
            let key = (owner.to_string(), property.clone());
            let previous = static_properties.get(&key).map(Value::dereference);
            store_static_property(static_properties, key, value.clone());
            if let Some(previous) = previous {
                run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
            }
            stack.push(value);
        }
        Op::AssignStaticPropertyArray(class_name, property, key_count, access_scope) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let current = static_properties.get(&key).cloned().unwrap_or(Value::Null);
            let previous = current.array_get_path(&keys)?;
            let updated = current.clone().array_set_path(&keys, value.clone())?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated, type_name, property, classes)?; }
            store_static_property(static_properties, key, updated);
            drop(current);
            run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
            stack.push(value);
        }
        Op::CompoundAssignStaticPropertyArray(class_name, property, key_count, operation, access_scope) => {
            let right = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let current = static_properties.get(&key).cloned().unwrap_or(Value::Null);
            let current_value = current.array_get_path(&keys)?;
            let operator = match operation {
                CompoundOperation::Add => Op::Add,
                CompoundOperation::Subtract => Op::Sub,
                CompoundOperation::Multiply => Op::Mul,
                CompoundOperation::Power => Op::Pow,
                CompoundOperation::Divide => Op::Div,
                CompoundOperation::Modulo => Op::Mod,
                CompoundOperation::Concatenate => Op::Concat,
                CompoundOperation::BitAnd => Op::BitAnd,
                CompoundOperation::BitXor => Op::BitXor,
                CompoundOperation::BitOr => Op::BitOr,
                CompoundOperation::ShiftLeft => Op::ShiftLeft,
                CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let value = super::operators::apply_binary(&operator, current_value, right)?;
            let updated = current.array_set_path(&keys, value.clone())?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated, type_name, property, classes)?; }
            store_static_property(static_properties, key, updated);
            stack.push(value);
        }
        Op::CoalesceStaticPropertyArrayJump(target, class_name, property, key_count, access_scope) => {
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let split = stack.len() - key_count;
            let raw_keys = stack.split_off(split);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let current = static_properties.get(&(owner.to_string(), property.clone())).cloned().unwrap_or(Value::Null);
            let value = current.array_get_path(&keys)?;
            if matches!(value, Value::Null) {
                stack.extend(raw_keys);
            } else {
                stack.push(value);
                *program_counter = *target;
            }
        }
        Op::UnsetStaticPropertyArray(class_name, property, key_count, access_scope) => {
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let current = static_properties.get(&key).cloned().unwrap_or(Value::Null);
            let keys = raw_keys.iter().map(|key| unset_array_key(key, &current)).collect::<Result<Vec<_>, _>>()?;
            let (updated, removed) = current.clone().array_unset_path_with_removed(&keys)?;
            store_static_property(static_properties, key, updated);
            drop(current);
            if let Some(value) = removed {
                run_destructors_removed_from_value(value, &Value::Null, functions, classes, static_properties, global_variables, output, steps)?;
            }
        }
        Op::AppendStaticPropertyArray(class_name, property, key_count, access_scope) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
            let raw_keys = stack.split_off(stack.len() - key_count);
            let keys = raw_keys.iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let current = static_properties.get(&key).cloned().unwrap_or(Value::Null);
            let updated = current.array_append_path(&keys, value.clone())?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&updated, type_name, property, classes)?; }
            store_static_property(static_properties, key, updated);
            stack.push(value);
        }
        Op::CompoundAssignStaticProperty(class_name, property, operation, access_scope) => {
            let right = stack.pop().ok_or("Invalid bytecode stack")?;
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            let key = (owner.to_string(), property.clone());
            let current = match static_properties.get(&key) {
                Some(value) => value.clone(),
                None if metadata.type_name.is_some() => return Err(format!("Typed static property {resolved_class}::${property} must not be accessed before initialization")),
                None => Value::Null,
            };
            let operator = match operation {
                CompoundOperation::Add => Op::Add,
                CompoundOperation::Subtract => Op::Sub,
                CompoundOperation::Multiply => Op::Mul,
                CompoundOperation::Power => Op::Pow,
                CompoundOperation::Divide => Op::Div,
                CompoundOperation::Modulo => Op::Mod,
                CompoundOperation::Concatenate => Op::Concat,
                    CompoundOperation::BitAnd => Op::BitAnd,
                    CompoundOperation::BitXor => Op::BitXor,
                    CompoundOperation::BitOr => Op::BitOr,
                    CompoundOperation::ShiftLeft => Op::ShiftLeft,
                    CompoundOperation::ShiftRight => Op::ShiftRight,
            };
            let value = super::operators::apply_binary(&operator, current, right)?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&value, type_name, property, classes)?; }
            store_static_property(static_properties, key, value.clone());
            stack.push(value);
        }
        Op::GetStaticPropertyCoalesce(class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            stack.push(static_properties.get(&(owner.to_string(), property.clone())).cloned().unwrap_or(Value::Null));
        }
        Op::CoalesceStaticPropertyJump(target, class_name, property, access_scope) => {
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            if let Some(value) = static_properties.get(&(owner.to_string(), property.clone())).filter(|value| !matches!(value, Value::Null)) {
                stack.push(value.clone());
                *program_counter = *target;
            }
        }
        Op::CoalesceAssignStaticProperty(class_name, property, access_scope) => {
            let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
            let resolved_class = resolve_static_class(class_name, variables)?;
            let (owner, metadata) = static_property_owner(&resolved_class, property, classes)
                .ok_or_else(|| format!("Undefined static property {resolved_class}::${property}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, property)?;
            if let Some(type_name) = &metadata.type_name { ensure_property_type(&value, type_name, property, classes)?; }
            store_static_property(static_properties, (owner.to_string(), property.clone()), value.clone());
            stack.push(value);
        }
        Op::GetClassConstant(class_name, constant, access_scope) => {
            let dynamic_class;
            let class_name = if class_name == "\0dynamic" {
                dynamic_class = stack.pop().ok_or("Missing dynamic class name")?.show().to_ascii_lowercase();
                &dynamic_class
            } else { class_name };
            let resolved_class = resolve_static_class(class_name, variables)?;
            let class_name = resolved_class.as_str();
            if let Some(value) = crate::pdo::class_constant(class_name, constant) {
                stack.push(value);
                return Ok(true);
            }
            if let Some(value) = crate::intl_class_constant(class_name, constant) {
                stack.push(value);
                return Ok(true);
            }
            if let Some(value) = crate::reflection_method_class_constant(class_name, constant) {
                stack.push(Value::Integer(value));
                return Ok(true);
            }
            if let Some(value) = crate::reflection_property_class_constant(class_name, constant) {
                stack.push(Value::Integer(value));
                return Ok(true);
            }
            if class_name.eq_ignore_ascii_case("Attribute") {
                let value = match constant.to_ascii_uppercase().as_str() {
                    "TARGET_CLASS" => 1, "TARGET_FUNCTION" => 2, "TARGET_METHOD" => 4,
                    "TARGET_PROPERTY" => 8, "TARGET_CLASS_CONSTANT" => 16,
                    "TARGET_PARAMETER" => 32, "TARGET_CONSTANT" => 64,
                    "TARGET_ALL" => 127, "IS_REPEATABLE" => 128,
                    _ => return Err(format!("Undefined class constant Attribute::{constant}")),
                };
                stack.push(Value::Integer(value));
                return Ok(true);
            }
            let (_, metadata) = find_class_constant(class_name, constant, classes)
                .ok_or_else(|| format!("Undefined class constant {class_name}::{constant}"))?;
            ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope.as_deref(), classes, constant)?;
            stack.push(metadata.value.clone());
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn set_exception_source_location(value: &Value, line: Option<usize>, file: &str, frame_scope: &str, instance_method: bool, global_variables: &GlobalVariables) {
    let file = global_variables.active_trace_frames.last().map(|frame| frame.file.as_str()).unwrap_or(file);
    if let Value::Object(object) = value {
        if let Some(data) = object.borrow_mut().exception.as_mut() {
            if data.file.is_empty() { data.file = file.to_string(); }
            if data.line.is_none() { data.line = line; }
            if data.trace.is_empty() && frame_scope != "<main>" {
                if let Some(line) = line {
                    if let Some(mut frame) = global_variables.active_trace_frames.last().cloned() {
                        frame.file = file.to_string();
                        frame.line = line;
                        if frame.class.is_some() && frame.call_type.is_none() {
                            frame.call_type = Some(if instance_method { "->".into() } else { "::".into() });
                        }
                        data.trace.push(frame);
                    }
                    data.trace.extend(global_variables.active_trace_frames.iter().rev().skip(1)
                        .filter(|frame| frame.function != "{main}").cloned());
                }
            }
        }
    }
}

fn read_property_for_coalesce(
    receiver: &Value,
    property: &str,
    access_scope: Option<&str>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let Value::Object(object) = receiver else { return Ok(Value::Null); };
    let class_name = object.borrow().class_name.clone();
    let metadata = find_property_in_scope(&class_name, property, access_scope, classes);
    let accessible = metadata.is_some_and(|metadata| ensure_access(
        metadata.visibility, Some(&metadata.declaring_class), access_scope, classes, property,
    ).is_ok());
    if accessible {
        let metadata = metadata.expect("accessible property metadata");
        let storage_key = instance_property_storage_key(metadata, property);
        if metadata.get_hook.is_none() {
            let value = object.borrow().properties.get(&storage_key).cloned();
            if value.is_none() && metadata.type_name.is_some() { return Ok(Value::Null); }
            return Ok(value.unwrap_or(Value::Null).dereference());
        }
        let value = read_property(receiver, property, access_scope, functions, classes, static_properties, global_variables, output, steps)?;
        return Ok(if matches!(value.dereference(), Value::Null) { Value::Null } else { value });
    }
    if metadata.is_none() && object.borrow().properties.contains_key(property) {
        return Ok(object.borrow().properties.get(property).cloned().unwrap_or(Value::Null).dereference());
    }
    if let Some(magic) = find_method(&class_name, "__isset", classes) {
        let present = call_magic(receiver, magic, vec![Value::String(property.to_owned())], functions, classes, static_properties, global_variables, output, steps)?;
        if !present.truthy() { return Ok(Value::Null); }
        let value = read_property(receiver, property, access_scope, functions, classes, static_properties, global_variables, output, steps)?;
        return Ok(if matches!(value.dereference(), Value::Null) { Value::Null } else { value });
    }
    Ok(Value::Null)
}

pub(super) fn read_property(
    receiver: &Value,
    property: &str,
    access_scope: Option<&str>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let Value::Object(object) = receiver else { return Err("Attempt to read property on non-object".into()); };
    let class_name = object.borrow().class_name.clone();
    let metadata = find_property_in_scope(&class_name, property, access_scope, classes);
    let direct_access = metadata.is_some_and(|metadata| {
        ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope, classes, property).is_ok()
    });
    if direct_access {
        let metadata = metadata.expect("direct access requires property metadata");
        let storage_key = instance_property_storage_key(metadata, property);
        if metadata.get_hook.is_none() && !object.borrow().properties.contains_key(&storage_key) && metadata.type_name.is_some() {
            return Err(format!("Typed property {class_name}::${property} must not be accessed before initialization"));
        }
        let hook_key = format!("get:{property}");
        if !object.borrow().active_property_hooks.contains(&hook_key) {
            if let Some(hook) = &metadata.get_hook {
                object.borrow_mut().active_property_hooks.insert(hook_key.clone());
                let result = call_magic(receiver, hook, Vec::new(), functions, classes, static_properties, global_variables, output, steps);
                object.borrow_mut().active_property_hooks.remove(&hook_key);
                return result;
            }
        }
        Ok(object.borrow().properties.get(&storage_key).cloned().unwrap_or(Value::Null))
    } else if metadata.is_none() && object.borrow().properties.contains_key(property) {
        Ok(object.borrow().properties.get(property).cloned().unwrap_or(Value::Null))
    } else if class_name.eq_ignore_ascii_case("stdclass") && metadata.is_none() {
        Ok(object.borrow().properties.get(property).cloned().unwrap_or(Value::Null))
    } else if let Some(magic) = find_method(&class_name, "__get", classes) {
        let value = call_magic(receiver, magic, vec![Value::String(property.into())], functions, classes, static_properties, global_variables, output, steps)?;
        if let Some(metadata) = metadata.filter(|metadata| metadata.type_name.is_some()) {
            ensure_magic_get_property_type(&value, &class_name, property, metadata, classes)?;
        }
        Ok(value)
    } else if let Some(metadata) = metadata {
        ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope, classes, property)?;
        unreachable!();
    } else { Ok(Value::Null) }
}

/// Assign an instance property while honoring visibility, type, readonly, and magic-set rules.
pub(super) fn assign_property(
    receiver: &Value,
    property: &str,
    value: Value,
    access_scope: Option<&str>,
    in_constructor: bool,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let value = value.dereference();
    let Value::Object(object) = receiver else { return Err("Attempt to assign property on non-object".into()); };
    let class_name = object.borrow().class_name.clone();
    if classes.get(&class_name.to_ascii_lowercase()).is_some_and(|class| class.is_enum) {
        return Err(format!("Cannot modify enum property {class_name}::${property}"));
    }
    let metadata = find_property_in_scope(&class_name, property, access_scope, classes);
    if let Some(metadata) = metadata {
        if metadata.set_visibility.is_some() {
            ensure_access(metadata.set_visibility.unwrap(), Some(&metadata.declaring_class), access_scope, classes, property)?;
        }
    }
    let direct_access = metadata.is_some_and(|metadata| {
        ensure_access(metadata.set_visibility.unwrap_or(metadata.visibility), Some(&metadata.declaring_class), access_scope, classes, property).is_ok()
    });
    if direct_access {
        let metadata = metadata.expect("direct access requires property metadata");
        if let Some(type_name) = &metadata.type_name { ensure_property_type(&value, type_name, property, classes)?; }
        let storage_key = instance_property_storage_key(metadata, property);
        let cloning_readonly = global_variables.readonly_clone_objects.contains(&(Rc::as_ptr(object) as usize));
        if metadata.is_readonly && ((!in_constructor && !cloning_readonly) || access_scope != Some(metadata.declaring_class.as_str()) || object.borrow().properties.contains_key(&storage_key)) {
            return Err(format!("Cannot modify readonly property {}::${property}", metadata.declaring_class));
        }
        let hook_key = format!("set:{property}");
        if !object.borrow().active_property_hooks.contains(&hook_key) {
            if let Some(hook) = &metadata.set_hook {
                object.borrow_mut().active_property_hooks.insert(hook_key.clone());
                let result = call_magic(receiver, hook, vec![value], functions, classes, static_properties, global_variables, output, steps);
                object.borrow_mut().active_property_hooks.remove(&hook_key);
                result?;
                return Ok(());
            }
        }
        let mut object = object.borrow_mut();
        if let Some(Value::Reference(reference)) = object.properties.get(&storage_key) {
            *reference.borrow_mut() = value.dereference();
        } else {
            if !object.properties.contains_key(&storage_key) && metadata.visibility != MemberVisibility::Private { object.property_order.push(property.to_string()); }
            object.properties.insert(storage_key, value);
        }
    } else if metadata.is_none() && object.borrow().properties.contains_key(property) {
        object.borrow_mut().properties.insert(property.to_string(), value);
    } else if let Some(magic) = find_method(&class_name, "__set", classes) {
        call_magic(receiver, magic, vec![Value::String(property.to_string()), value], functions, classes, static_properties, global_variables, output, steps)?;
    } else if metadata.is_none() {
        let mut object = object.borrow_mut();
        object.property_order.push(property.to_string());
        object.properties.insert(property.to_string(), value);
    } else if let Some(metadata) = metadata {
        ensure_access(metadata.set_visibility.unwrap_or(metadata.visibility), Some(&metadata.declaring_class), access_scope, classes, property)?;
        unreachable!();
    } else {
        return Err(format!("Undefined property {class_name}::${property}"));
    }
    Ok(())
}

pub(super) fn bind_property_reference(
    receiver: &Value,
    property: &str,
    reference: Rc<RefCell<Value>>,
    access_scope: Option<&str>,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Option<Value>, String> {
    let Value::Object(object) = receiver else { return Err("Attempt to assign property on non-object".into()); };
    let class_name = object.borrow().class_name.clone();
    if classes.get(&class_name.to_ascii_lowercase()).is_some_and(|class| class.is_enum) {
        return Err(format!("Cannot modify enum property {class_name}::${property}"));
    }
    let metadata = find_property_in_scope(&class_name, property, access_scope, classes);
    if let Some(metadata) = metadata {
        ensure_access(metadata.set_visibility.unwrap_or(metadata.visibility), Some(&metadata.declaring_class), access_scope, classes, property)?;
        let declaring_name = classes.get(&metadata.declaring_class.to_ascii_lowercase()).map_or(metadata.declaring_class.as_str(), |class| class.name.as_str());
        if metadata.is_readonly {
            return Err(format!("Cannot indirectly modify readonly property {declaring_name}::${property}"));
        }
        if metadata.type_name.is_some() {
            return Err(format!("Typed property {declaring_name}::${property} must not be referenced"));
        }
    } else if !object.borrow().properties.contains_key(property) {
        if find_method(&class_name, "__get", classes).is_some() {
            return Err(format!("Cannot assign by reference to overloaded property {class_name}::${property}; __get() must return by reference"));
        }
    }
    let storage_key = metadata.map_or_else(|| property.to_string(), |metadata| instance_property_storage_key(metadata, property));
    let mut object = object.borrow_mut();
    let previous = object.properties.insert(storage_key, Value::Reference(reference));
    if previous.is_none() && metadata.is_none_or(|metadata| metadata.visibility != MemberVisibility::Private) {
        object.property_order.push(property.to_string());
    }
    Ok(previous)
}

pub(super) fn existing_property_for_assignment(
    receiver: &Value,
    property: &str,
    access_scope: Option<&str>,
    classes: &HashMap<String, BytecodeClass>,
) -> Option<Value> {
    let Value::Object(object) = receiver else { return None; };
    let class_name = object.borrow().class_name.clone();
    if let Some(metadata) = find_property_in_scope(&class_name, property, access_scope, classes) {
        ensure_access(metadata.visibility, Some(&metadata.declaring_class), access_scope, classes, property).ok()?;
    } else if !object.borrow().properties.contains_key(property) {
        return None;
    }
    let key = find_property_in_scope(&class_name, property, access_scope, classes)
        .map_or_else(|| property.to_string(), |metadata| instance_property_storage_key(metadata, property));
    object.borrow().properties.get(&key).map(Value::dereference)
}

fn remove_object_property(object: &Rc<RefCell<PhpObject>>, storage_key: &str, property: &str) {
    let mut object = object.borrow_mut();
    object.properties.remove(storage_key);
    if storage_key == property { object.property_order.retain(|name| name != property); }
}

fn validate_readonly_unset(
    object: &Rc<RefCell<PhpObject>>,
    metadata: &BytecodeProperty,
    storage_key: &str,
    property: &str,
    access_scope: Option<&str>,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    if !metadata.is_readonly { return Ok(()); }
    let initialized = object.borrow().properties.contains_key(storage_key);
    let declaring_class = classes.get(&metadata.declaring_class.to_ascii_lowercase())
        .map_or(metadata.declaring_class.as_str(), |class| class.name.as_str());
    if initialized {
        return Err(format!("Error:Cannot unset readonly property {declaring_class}::${property}"));
    }
    if !access_scope.is_some_and(|scope| scope.eq_ignore_ascii_case(declaring_class)) {
        let scope = access_scope.unwrap_or("global scope");
        return Err(format!("Error:Cannot unset readonly property {declaring_class}::${property} from {scope}"));
    }
    Ok(())
}

fn finish_object_frame(
    locals: HashMap<String, Value>,
    result: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let frame_values = Value::from_array_entries(locals.into_iter()
        .map(|(name, value)| (ArrayKey::String(name), value)).collect());
    run_destructors_removed_from_value(
        frame_values, result, functions, classes, static_properties, global_variables, output, steps,
    )
}

/// Invoke a public instance magic method with checked arguments and return type.
/// Call a magic property hook with the receiver in `$this` and checked argument types.
fn call_magic(
    receiver: &Value,
    function: &BytecodeFunction,
    arguments: Vec<Value>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    if function.is_static { return Err(format!("{}::__get/__set() must not be static", function.declaring_class.as_deref().unwrap_or("class"))); }
    let label = format!("{}::{}()", function.declaring_class.as_deref().unwrap_or("class"), function.name);
    let arguments = complete_arguments(arguments, function, &label)?;
    validate_arguments(&arguments, function, classes)?;
    let class_name = match receiver { Value::Object(object) => object.borrow().class_name.clone(), _ => unreachable!() };
    let mut locals = HashMap::from([
        ("this".to_string(), receiver.clone()),
        ("\0static_class".to_string(), Value::String(class_name.clone())),
    ]);
    for (parameter, argument) in function.parameters.iter().cloned().zip(arguments) { locals.insert(parameter, argument); }
    super::initialize_local_order(&mut locals, std::iter::once("this".to_string()).chain(function.parameters.iter().cloned()));
    let result = super::functions::execute_user_frame(&function.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(function), HashMap::new(), output, steps)?.unwrap_or(Value::Null);
    let result = match super::validate_return_type_in_scope(result, function, classes, Some(&class_name)) {
        Ok(result) => result,
        Err(error) => {
            finish_object_frame(locals, &Value::Null, functions, classes, static_properties, global_variables, output, steps)?;
            return Err(error);
        }
    };
    finish_object_frame(locals, &result, functions, classes, static_properties, global_variables, output, steps)?;
    Ok(result)
}

/// Resolve `static::` against the called class stored in the current VM frame.
fn resolve_static_class(class_name: &str, variables: &HashMap<String, Value>) -> Result<String, String> {
    if class_name != "static" { return Ok(class_name.to_ascii_lowercase()); }
    match variables.get("\0static_class") {
        Some(Value::String(class)) => Ok(class.clone()),
        _ => Err("static:: used outside a class method".into()),
    }
}

/// Convert objects printed by `echo` through their PHP `__toString()` method.
pub(super) fn stringify(
    value: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<String, String> {
    let Value::Object(object) = value else { return Ok(value.show()); };
    let class_name = object.borrow().class_name.clone();
    if object.borrow().exception.is_some() && find_method(&class_name, "__tostring", classes).is_none() {
        return crate::exception_builtins::stringify(value);
    }
    let method = find_method(&class_name, "__tostring", classes)
        .ok_or_else(|| format!("Object of class {class_name} could not be converted to string"))?;
    if method.is_static || !method.parameters.is_empty() {
        return Err(format!("{}::__toString() must be a non-static method without parameters", class_name));
    }
    let mut locals = HashMap::from([("this".to_string(), value.clone())]);
    super::initialize_local_order(&mut locals, ["this".to_string()]);
    let result = super::functions::execute_user_frame(&method.instructions, functions, classes, static_properties, &mut locals, global_variables, false, &function_scope(method), HashMap::new(), output, steps)?
        .unwrap_or(Value::Null);
    finish_object_frame(locals, &result, functions, classes, static_properties, global_variables, output, steps)?;
    match result {
        Value::String(string) => Ok(string),
        Value::Integer(_) | Value::Number(_) | Value::Bool(_) | Value::Null if method.strict_types => {
            Err(format!("TypeError:{class_name}::__toString() must return string"))
        }
        Value::Integer(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(Value::Number(value).show()),
        Value::Bool(value) => Ok(if value { "1".into() } else { String::new() }),
        Value::Null => Ok(String::new()),
        _ => Err(format!("Error:{}::__toString() must return a string", class_name)),
    }
}

/// Run remaining user destructors when the script execution context shuts down.
pub(super) fn run_registered_destructors(
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    while let Some(value) = globals.destructor_queue.pop() {
        let Value::Object(object) = value else { continue; };
        invoke_destructor(object, functions, classes, static_properties, globals, output, steps)?;
    }
    Ok(())
}

/// Collect unreachable object cycles on an explicit `gc_collect_cycles()` call.
/// The shutdown queue keeps one strong handle to each PHP object so destructors
/// can run at script shutdown; account for those handles separately from PHP
/// references when deciding whether an object is a live root.
pub(super) fn collect_object_cycles(
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<usize, String> {
    fn graph(
        globals: &GlobalVariables,
    ) -> (HashMap<usize, Rc<RefCell<PhpObject>>>, HashMap<usize, Vec<usize>>, HashMap<usize, usize>, HashMap<usize, usize>) {
        #[derive(Default)]
        struct SeenContainers {
            references: HashSet<usize>,
            closures: HashSet<usize>,
            generators: HashSet<usize>,
        }

        fn edges(
            value: &Value,
            known: &HashMap<usize, Rc<RefCell<PhpObject>>>,
            output: &mut Vec<usize>,
            seen: &mut SeenContainers,
        ) {
            match value {
                Value::Object(object) => {
                    let identity = Rc::as_ptr(object) as usize;
                    if known.contains_key(&identity) { output.push(identity); }
                }
                Value::Array(array) => for (_, child) in array.iter() { edges(child, known, output, seen); },
                Value::Reference(reference) => {
                    let identity = Rc::as_ptr(reference) as usize;
                    if seen.references.insert(identity) { edges(&reference.borrow(), known, output, seen); }
                }
                Value::Closure(closure) => {
                    let identity = Rc::as_ptr(closure) as usize;
                    if seen.closures.insert(identity) {
                        let closure = closure.borrow();
                        for capture in closure.captures.values() { edges(capture, known, output, seen); }
                        if let Some(target) = &closure.callable_target { edges(target, known, output, seen); }
                    }
                }
                Value::Generator(generator) => {
                    let identity = Rc::as_ptr(generator) as usize;
                    if seen.generators.insert(identity) {
                        let generator = generator.borrow();
                        for variable in generator.variables.values() { edges(variable, known, output, seen); }
                        if let Some((key, value)) = &generator.current {
                            edges(key, known, output, seen);
                            edges(value, known, output, seen);
                        }
                        if let Some(value) = &generator.return_value { edges(value, known, output, seen); }
                        let continuation = &generator.continuation;
                        for value in &continuation.stack { edges(value, known, output, seen); }
                        for iterator in continuation.iterators.values() {
                            match iterator {
                                super::control::ForeachIterator::Array(values, _) => for (_, value) in values { edges(value, known, output, seen); },
                                super::control::ForeachIterator::Generator(nested, _) => edges(&Value::Generator(nested.clone()), known, output, seen),
                                super::control::ForeachIterator::UserIterator { object, .. } => edges(object, known, output, seen),
                            }
                        }
                        for iterator in continuation.yield_from.values() {
                            match iterator {
                                super::YieldFromIterator::Array(values, _) => for (_, value) in values { edges(value, known, output, seen); },
                                super::YieldFromIterator::Generator(nested) => edges(&Value::Generator(nested.clone()), known, output, seen),
                                super::YieldFromIterator::UserIterator { object, .. } => edges(object, known, output, seen),
                            }
                        }
                        for exit in &continuation.pending_exits {
                            if let super::PendingExit::Return(value) = exit { edges(value, known, output, seen); }
                        }
                        if let Some((key, value)) = &continuation.yielded {
                            if let Some(key) = key { edges(key, known, output, seen); }
                            edges(value, known, output, seen);
                        }
                        for binding in continuation.bindings.values() {
                            if let VariableBinding::ObjectProperty { receiver, .. } = binding { edges(receiver, known, output, seen); }
                        }
                    }
                }
                _ => {}
            }
        }

        let mut objects = HashMap::new();
        let mut queued_counts = HashMap::<usize, usize>::new();
        for value in &globals.destructor_queue {
            if let Value::Object(object) = value.dereference() {
                let identity = Rc::as_ptr(&object) as usize;
                *queued_counts.entry(identity).or_default() += 1;
                objects.entry(identity).or_insert(object);
            }
        }
        let mut outgoing = HashMap::new();
        for (identity, object) in &objects {
            let mut object_edges = Vec::new();
            let mut seen = SeenContainers::default();
            for value in object.borrow().properties.values() {
                edges(value, &objects, &mut object_edges, &mut seen);
            }
            outgoing.insert(*identity, object_edges);
        }
        // Count real object-bearing slots once. A shared closure/generator may
        // be reachable from multiple owner objects, but its captured/frame
        // values are stored only once and must not be overcounted as roots.
        let mut incoming = HashMap::<usize, usize>::new();
        let mut all_edges = Vec::new();
        let mut seen = SeenContainers::default();
        for object in objects.values() {
            for value in object.borrow().properties.values() {
                edges(value, &objects, &mut all_edges, &mut seen);
            }
        }
        for child in all_edges { *incoming.entry(child).or_default() += 1; }
        (objects, outgoing, incoming, queued_counts)
    }

    fn reachable_from_roots(
        objects: &HashMap<usize, Rc<RefCell<PhpObject>>>,
        outgoing: &HashMap<usize, Vec<usize>>,
        incoming: &HashMap<usize, usize>,
        queued_counts: &HashMap<usize, usize>,
    ) -> HashSet<usize> {
        let mut roots = Vec::new();
        for (identity, object) in objects {
            // One handle is held by this temporary graph and queued_counts by
            // the shutdown queue. Remaining handles are PHP-visible roots.
            let expected_internal = 1 + queued_counts.get(identity).copied().unwrap_or(0) + incoming.get(identity).copied().unwrap_or(0);
            if Rc::strong_count(object) > expected_internal { roots.push(*identity); }
        }
        let mut reachable = HashSet::new();
        while let Some(identity) = roots.pop() {
            if !reachable.insert(identity) { continue; }
            if let Some(children) = outgoing.get(&identity) { roots.extend(children.iter().copied()); }
        }
        reachable
    }

    let (objects, outgoing, incoming, queued_counts) = graph(globals);
    if objects.is_empty() { return Ok(0); }
    let reachable = reachable_from_roots(&objects, &outgoing, &incoming, &queued_counts);
    let unreachable: HashSet<usize> = objects.keys().filter(|identity| !reachable.contains(identity)).copied().collect();

    // Ordinary unreferenced objects are released by reference counting. Only
    // start a collection if an unreachable strongly connected object cycle is
    // present; descendants of that cycle are released with the same sweep.
    let has_cycle = unreachable.iter().any(|start| {
        let mut pending = outgoing.get(start).cloned().unwrap_or_default();
        let mut visited = HashSet::new();
        while let Some(identity) = pending.pop() {
            if identity == *start { return true; }
            if !visited.insert(identity) || !unreachable.contains(&identity) { continue; }
            pending.extend(outgoing.get(&identity).into_iter().flatten().copied());
        }
        false
    });
    if !has_cycle { return Ok(0); }
    drop(objects);

    // Run destructors before breaking the cycle, allowing them to inspect the
    // original object graph. Reverse registration order matches shutdown order.
    let mut garbage: Vec<_> = globals.destructor_queue.iter().rev().filter_map(|value| {
        let Value::Object(object) = value.dereference() else { return None; };
        let identity = Rc::as_ptr(&object) as usize;
        unreachable.contains(&identity).then_some(object)
    }).collect();
    let mut invoked = HashSet::new();
    for object in &garbage {
        let identity = Rc::as_ptr(object) as usize;
        if invoked.insert(identity) {
            invoke_destructor(object.clone(), functions, classes, static_properties, globals, output, steps)?;
        }
    }
    garbage.clear();

    // Destructors may resurrect objects. Recompute liveness after they return;
    // preserve every resurrected object and anything reachable from it.
    let (current_objects, current_outgoing, current_incoming, current_queued_counts) = graph(globals);
    let current_reachable = reachable_from_roots(&current_objects, &current_outgoing, &current_incoming, &current_queued_counts);
    let collected: HashSet<usize> = unreachable.into_iter()
        .filter(|identity| current_objects.contains_key(identity) && !current_reachable.contains(identity))
        .collect();
    if collected.is_empty() { return Ok(0); }
    for identity in &collected {
        if let Some(object) = current_objects.get(identity) { object.borrow_mut().properties.clear(); }
    }
    globals.destructor_queue.retain(|value| match value.dereference() {
        Value::Object(object) => !collected.contains(&(Rc::as_ptr(&object) as usize)),
        _ => true,
    });
    globals.forget_collected_objects(&collected);
    Ok(collected.len())
}

pub(super) fn run_destructor_if_unreferenced(
    value: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    let Value::Object(object) = value else { return Ok(()); };
    // One strong handle belongs to the shutdown queue and one to this temporary value.
    if Rc::strong_count(&object) != 2 { return Ok(()); }
    invoke_destructor(object.clone(), functions, classes, static_properties, globals, output, steps)
}

pub(super) fn run_destructors_removed_from_value(
    previous: Value,
    updated: &Value,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    fn collect(value: &Value, objects: &mut HashMap<usize, Value>) {
        match value.dereference() {
            Value::Object(object) => { objects.entry(Rc::as_ptr(&object) as usize).or_insert(Value::Object(object)); }
            Value::Array(array) => for (_, child) in array.iter() { collect(child, objects); },
            _ => {}
        }
    }
    let mut previous_objects = HashMap::new();
    let mut updated_objects = HashMap::new();
    collect(&previous, &mut previous_objects);
    collect(updated, &mut updated_objects);
    drop(previous);
    for (identity, value) in previous_objects {
        if !updated_objects.contains_key(&identity) {
            run_destructor_if_unreferenced(&value, functions, classes, static_properties, globals, output, steps)?;
        }
    }
    Ok(())
}

fn invoke_destructor(
    object: Rc<RefCell<PhpObject>>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    if !globals.mark_destructed(&object) { return Ok(()); }
    let class_name = object.borrow().class_name.clone();
    let Some(destructor) = find_method(&class_name, "__destruct", classes) else { return Ok(()); };
    if destructor.is_static || !destructor.parameters.is_empty() {
        return Err(format!("{}::__destruct() must be a non-static method without parameters", class_name));
    }
    let mut locals = HashMap::from([
        ("this".to_string(), Value::Object(object)),
        ("\0static_class".to_string(), Value::String(class_name)),
    ]);
    super::initialize_local_order(&mut locals, ["this".to_string()]);
    let result = super::functions::execute_user_frame(&destructor.instructions, functions, classes, static_properties, &mut locals, globals, false, &function_scope(destructor), HashMap::new(), output, steps)?;
    let _validated_result = validate_return_type(result.unwrap_or(Value::Null), destructor, classes)?;
    finish_object_frame(locals, &Value::Null, functions, classes, static_properties, globals, output, steps)?;
    Ok(())
}

/// Find a method by walking the class and parent-class metadata.
pub(super) fn find_method<'a>(class_name: &str, method: &str, classes: &'a HashMap<String, BytecodeClass>) -> Option<&'a BytecodeFunction> {
    // Method lookup walks up the parent chain only after checking the concrete class.
    let normalized_name = class_name.to_ascii_lowercase();
    let class = classes.get(&normalized_name)?;
    class.methods.get(method).or_else(|| class.parent.as_deref().and_then(|parent| find_method(parent, method, classes)))
}

/// Collect inherited property defaults when a new object is created.
pub(super) fn collect_properties(class_name: &str, classes: &HashMap<String, BytecodeClass>, target: &mut HashMap<String, Value>) -> Result<(), String> {
    let normalized_name = class_name.to_ascii_lowercase();
    let class = classes.get(&normalized_name).ok_or_else(|| format!("Class '{class_name}' not found"))?;
    if let Some(parent) = &class.parent { collect_properties(parent, classes, target)?; }
    target.extend(class.properties.iter().filter_map(|(name, property)| {
        property.default.clone().or_else(|| property.type_name.is_none().then_some(Value::Null))
            .map(|value| (instance_property_storage_key(property, name), value))
    }));
    Ok(())
}

pub(super) fn collect_property_order(class_name: &str, classes: &HashMap<String, BytecodeClass>, target: &mut Vec<String>) -> Result<(), String> {
    let class = classes.get(&class_name.to_ascii_lowercase()).ok_or_else(|| format!("Class '{class_name}' not found"))?;
    if let Some(parent) = &class.parent { collect_property_order(parent, classes, target)?; }
    for property in &class.property_order {
        if !target.contains(property) { target.push(property.clone()); }
    }
    Ok(())
}

pub(super) fn foreach_entries(
    object: &Rc<RefCell<PhpObject>>,
    classes: &HashMap<String, BytecodeClass>,
    access_scope: Option<&str>,
    by_reference: bool,
) -> Vec<(ArrayKey, Value)> {
    let mut object = object.borrow_mut();
    let class_name = object.class_name.clone();
    let property_order = object.property_order.clone();
    let mut chain = Vec::new();
    let mut current = Some(class_name.as_str());
    while let Some(name) = current {
        let Some(class) = classes.get(&name.to_ascii_lowercase()) else { break; };
        chain.push(class);
        current = class.parent.as_deref();
    }
    chain.reverse();

    let declared_names = chain.iter().flat_map(|class| class.property_order.iter().cloned()).collect::<HashSet<_>>();
    let mut selected = Vec::<(String, BytecodeProperty)>::new();
    let mut visible_names = HashSet::new();
    for class in &chain {
        for name in &class.property_order {
            let Some(metadata) = class.properties.get(name) else { continue; };
            if metadata.visibility == MemberVisibility::Private {
                if !access_scope.is_some_and(|scope| metadata.declaring_class.eq_ignore_ascii_case(scope)) { continue; }
                selected.push((name.clone(), metadata.clone()));
            } else {
                let accessible = match metadata.visibility {
                    MemberVisibility::Public => true,
                    MemberVisibility::Protected => access_scope.is_some_and(|scope| {
                        is_subclass_of(scope, &metadata.declaring_class, classes)
                            || is_subclass_of(&metadata.declaring_class, scope, classes)
                    }),
                    MemberVisibility::Private => unreachable!(),
                };
                if accessible && visible_names.insert(name.clone()) {
                    selected.push((name.clone(), metadata.clone()));
                }
            }
        }
    }

    let mut result = Vec::new();
    for (name, metadata) in selected {
        let storage_key = instance_property_storage_key(&metadata, &name);
        let Some(value) = object.properties.get_mut(&storage_key) else { continue; };
        let value = foreach_value(value, by_reference);
        result.push((ArrayKey::String(name), value));
    }
    for name in property_order {
        if name.starts_with('\0') || declared_names.contains(&name) { continue; }
        let Some(value) = object.properties.get_mut(&name) else { continue; };
        result.push((ArrayKey::String(name), foreach_value(value, by_reference)));
    }
    result
}

fn foreach_value(value: &mut Value, by_reference: bool) -> Value {
    if !by_reference { return value.clone(); }
    match value {
        Value::Reference(reference) => Value::Reference(reference.clone()),
        current => {
            let reference = Rc::new(RefCell::new(current.clone()));
            *current = Value::Reference(reference.clone());
            Value::Reference(reference)
        }
    }
}

/// Locate the declaring storage slot for an inherited static property.
fn store_static_property(static_properties: &mut StaticProperties, key: (String, String), value: Value) {
    if let Some(Value::Reference(reference)) = static_properties.get(&key) {
        *reference.borrow_mut() = value;
    } else {
        static_properties.insert(key, value);
    }
}

fn static_property_owner<'a>(class_name: &'a str, property: &str, classes: &'a HashMap<String, BytecodeClass>) -> Option<(&'a str, &'a BytecodeProperty)> {
    let normalized_name = class_name.to_ascii_lowercase();
    let class = classes.get(&normalized_name)?;
    if let Some(metadata) = class.static_properties.get(property) { return Some((class_name, metadata)); }
    class.parent.as_deref().and_then(|parent| static_property_owner(parent, property, classes))
}

/// Find a constant on a class, parent, or implemented interface.
pub(super) fn find_class_constant<'a>(
    class_name: &str,
    constant: &str,
    classes: &'a HashMap<String, BytecodeClass>,
) -> Option<(&'a str, &'a BytecodeConstant)> {
    // A visited set prevents interface cycles from recursing forever during lookup.
    /// Find the class that owns the requested static property.
fn search<'a>(
        name: &str,
        constant: &str,
        classes: &'a HashMap<String, BytecodeClass>,
        visited: &mut std::collections::HashSet<String>,
    ) -> Option<(&'a str, &'a BytecodeConstant)> {
        let normalized_name = name.to_ascii_lowercase();
        if !visited.insert(normalized_name.clone()) { return None; }
        let (key, class) = classes.get_key_value(&normalized_name)?;
        if let Some(value) = class.constants.get(constant) { return Some((key.as_str(), value)); }
        if let Some(parent) = &class.parent {
            if let Some(value) = search(parent, constant, classes, visited) { return Some(value); }
        }
        for interface in &class.interfaces {
            if let Some(value) = search(interface, constant, classes, visited) { return Some(value); }
        }
        None
    }
    search(class_name, constant, classes, &mut std::collections::HashSet::new())
}

/// Locate an instance property by walking the class inheritance chain.
pub(super) fn find_property<'a>(class_name: &str, property: &str, classes: &'a HashMap<String, BytecodeClass>) -> Option<&'a BytecodeProperty> {
    let normalized_name = class_name.to_ascii_lowercase();
    let class = classes.get(&normalized_name)?;
    class.properties.get(property).or_else(|| class.parent.as_deref().and_then(|parent| find_property(parent, property, classes)))
}

pub(super) fn find_property_in_scope<'a>(class_name: &str, property: &str, access_scope: Option<&str>, classes: &'a HashMap<String, BytecodeClass>) -> Option<&'a BytecodeProperty> {
    if let Some(scope) = access_scope {
        let mut current = Some(class_name);
        while let Some(name) = current {
            if name.eq_ignore_ascii_case(scope) {
                if let Some(metadata) = classes.get(&name.to_ascii_lowercase()).and_then(|class| class.properties.get(property)) {
                    if metadata.visibility == MemberVisibility::Private { return Some(metadata); }
                }
                break;
            }
            current = classes.get(&name.to_ascii_lowercase()).and_then(|class| class.parent.as_deref());
        }
    }
    find_property(class_name, property, classes)
}

pub(super) fn instance_property_storage_key(property: &BytecodeProperty, name: &str) -> String {
    if property.visibility == MemberVisibility::Private {
        format!("\0{}::{name}", property.declaring_class.to_ascii_lowercase())
    } else {
        name.to_string()
    }
}

/// Enforce scalar, nullable, or nominal class types on property writes.
pub(super) fn ensure_property_type(
    value: &Value,
    type_name: &str,
    property: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    if type_name.contains('|') {
        if type_name.split('|').any(|member| ensure_property_type(value, member, property, classes).is_ok()) { return Ok(()); }
        return Err(format!("TypeError:Cannot assign value to {type_name} property ${property}"));
    }
    if type_name.contains('&') {
        for member in type_name.split('&') { ensure_property_type(value, member, property, classes)?; }
        return Ok(());
    }
    let nullable = type_name.starts_with('?');
    let type_name = type_name.trim_start_matches('?');
    let valid = if nullable && matches!(value, Value::Null) {
        true
    } else {
        match type_name.to_ascii_lowercase().as_str() {
            "mixed" => true,
            "null" => matches!(value, Value::Null),
            "true" => matches!(value, Value::Bool(true)),
            "false" => matches!(value, Value::Bool(false)),
            "int" | "integer" => matches!(value, Value::Integer(_)) || matches!(value, Value::Number(number) if number.is_finite() && number.fract() == 0.0),
            "float" | "double" => matches!(value, Value::Number(_) | Value::Integer(_)),
            "string" => matches!(value, Value::String(_)),
            "bool" | "boolean" => matches!(value, Value::Bool(_)),
            "array" => matches!(value, Value::Array(_)),
            class_name => match value {
                Value::Object(object) => super::is_a_type(
                    &object.borrow().class_name.to_ascii_lowercase(),
                    &class_name.to_ascii_lowercase(),
                    classes,
                    &mut std::collections::HashSet::new(),
                ),
                _ => false,
            },
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!("TypeError:Cannot assign value to {type_name} property ${property}"))
    }
}

fn ensure_magic_get_property_type(
    value: &Value,
    class_name: &str,
    property: &str,
    metadata: &BytecodeProperty,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    let Some(type_name) = &metadata.type_name else { return Ok(()); };
    let value = value.dereference();
    if ensure_property_type(&value, type_name, property, classes).is_ok() { return Ok(()); }
    let actual_type = value.type_name();
    let declaring_class = if metadata.declaring_class.eq_ignore_ascii_case(class_name) {
        class_name
    } else {
        metadata.declaring_class.as_str()
    };
    Err(format!("TypeError:Value of type {actual_type} returned from {declaring_class}::__get() must be compatible with unset property {declaring_class}::${property} of type {type_name}"))
}

/// Enforce public, protected, and private access based on the current scope.
pub(super) fn ensure_access(
    visibility: MemberVisibility,
    declaring_class: Option<&str>,
    access_scope: Option<&str>,
    classes: &HashMap<String, BytecodeClass>,
    member: &str,
) -> Result<(), String> {
    let allowed = match visibility {
        MemberVisibility::Public => true,
        MemberVisibility::Private => declaring_class.is_some() && declaring_class == access_scope,
        MemberVisibility::Protected => match (declaring_class, access_scope) {
            (Some(declaring), Some(scope)) => is_subclass_of(scope, declaring, classes) || is_subclass_of(declaring, scope, classes),
            _ => false,
        },
    };
    if allowed { Ok(()) } else { Err(format!("Cannot access {visibility:?} member {member}")) }
}

/// Check class inheritance recursively for visibility and property type checks.
fn is_subclass_of(candidate: &str, ancestor: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    let candidate = candidate.to_ascii_lowercase();
    let ancestor = ancestor.to_ascii_lowercase();
    if candidate == ancestor { return true; }
    classes.get(&candidate).and_then(|class| class.parent.as_deref())
        .is_some_and(|parent| is_subclass_of(parent, &ancestor, classes))
}

