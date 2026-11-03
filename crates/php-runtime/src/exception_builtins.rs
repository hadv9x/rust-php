//! Native objects and nominal type relationships for PHP's built-in Throwable classes.

use std::{cell::RefCell, collections::{HashMap, HashSet}, rc::Rc};

use crate::{ArrayKey, BytecodeClass, PhpExceptionData, PhpObject, Value};

const EXCEPTION_CLASSES: &[&str] = &[
    "exception", "error", "typeerror", "argumentcounterror", "compileerror", "parseerror",
    "arithmeticerror", "divisionbyzeroerror", "assertionerror", "unhandledmatcherror", "valueerror", "jsonexception",
    "logicexception", "badfunctioncallexception", "badmethodcallexception", "domainexception",
    "invalidargumentexception", "lengthexception", "outofrangeexception", "runtimeexception",
    "outofboundsexception", "overflowexception", "rangeexception", "underflowexception", "unexpectedvalueexception",
];

pub(crate) fn is_class(name: &str) -> bool {
    EXCEPTION_CLASSES.iter().any(|candidate| candidate.eq_ignore_ascii_case(name.trim_start_matches('\\')))
}

pub(crate) fn is_interface(name: &str) -> bool {
    matches!(name.trim_start_matches('\\').to_ascii_lowercase().as_str(), "throwable" | "jsonserializable" | "stringable" | "countable")
}

pub(crate) fn canonical_name(name: &str) -> Option<&'static str> {
    EXCEPTION_CLASSES.iter().copied().find(|candidate| candidate.eq_ignore_ascii_case(name.trim_start_matches('\\')))
        .map(|name| match name {
            "exception" => "Exception",
            "error" => "Error",
            "typeerror" => "TypeError",
            "argumentcounterror" => "ArgumentCountError",
            "compileerror" => "CompileError",
            "parseerror" => "ParseError",
            "arithmeticerror" => "ArithmeticError",
            "divisionbyzeroerror" => "DivisionByZeroError",
            "assertionerror" => "AssertionError",
            "unhandledmatcherror" => "UnhandledMatchError",
            "valueerror" => "ValueError",
            "jsonexception" => "JsonException",
            "logicexception" => "LogicException", "badfunctioncallexception" => "BadFunctionCallException",
            "badmethodcallexception" => "BadMethodCallException", "domainexception" => "DomainException",
            "invalidargumentexception" => "InvalidArgumentException", "lengthexception" => "LengthException",
            "outofrangeexception" => "OutOfRangeException", "runtimeexception" => "RuntimeException",
            "outofboundsexception" => "OutOfBoundsException", "overflowexception" => "OverflowException",
            "rangeexception" => "RangeException", "underflowexception" => "UnderflowException",
            "unexpectedvalueexception" => "UnexpectedValueException",
            _ => unreachable!(),
        })
}

pub(crate) fn parent_class(name: &str) -> Option<&'static str> {
    match name.trim_start_matches('\\').to_ascii_lowercase().as_str() {
        "exception" | "error" => Some("Throwable"),
        "jsonexception" => Some("Exception"),
        "typeerror" | "compileerror" | "arithmeticerror" | "assertionerror" | "unhandledmatcherror" | "valueerror" => Some("Error"),
        "argumentcounterror" => Some("TypeError"),
        "parseerror" => Some("CompileError"),
        "divisionbyzeroerror" => Some("ArithmeticError"),
        "logicexception" => Some("Exception"),
        "badfunctioncallexception" | "domainexception" | "invalidargumentexception" | "lengthexception" | "outofrangeexception" => Some("LogicException"),
        "badmethodcallexception" => Some("BadFunctionCallException"),
        "runtimeexception" => Some("Exception"),
        "outofboundsexception" | "overflowexception" | "rangeexception" | "underflowexception" | "unexpectedvalueexception" => Some("RuntimeException"),
        _ => None,
    }
}

pub(crate) fn is_a(actual: &str, expected: &str) -> bool {
    let expected = expected.trim_start_matches('\\');
    if actual.eq_ignore_ascii_case(expected) { return true; }
    let mut current = actual;
    while let Some(parent) = parent_class(current) {
        if parent.eq_ignore_ascii_case(expected) { return true; }
        current = parent;
    }
    false
}

pub(crate) fn method_exists(class_name: &str, method: &str) -> bool {
    let method = method.to_ascii_lowercase();
    if class_name.trim_start_matches('\\').eq_ignore_ascii_case("JsonSerializable") {
        return method == "jsonserialize";
    }
    if class_name.trim_start_matches('\\').eq_ignore_ascii_case("Stringable") {
        return method == "__tostring";
    }
    if is_interface(class_name) {
        return matches!(method.as_str(), "getmessage" | "getcode" | "getfile" | "getline" | "gettrace" | "gettraceasstring" | "getprevious");
    }
    is_class(class_name) && matches!(method.as_str(),
        "__construct" | "getmessage" | "getcode" | "getprevious" | "getfile" | "getline" | "gettrace" | "gettraceasstring" | "__tostring")
}

pub(crate) fn construct(
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    if class_name.eq_ignore_ascii_case("Throwable") {
        return Err("Cannot instantiate interface Throwable".into());
    }
    Ok(Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: canonical_name(class_name).unwrap_or("Exception").into(),
        properties: HashMap::new(),
        property_order: Vec::new(),
        active_property_hooks: HashSet::new(),
        exception: Some(construct_data(class_name, arguments, classes)?),
    }))))
}

pub(crate) fn construct_data(
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<PhpExceptionData, String> {
    if arguments.len() > 3 { return Err(format!("{}::__construct() expects at most 3 arguments", class_name)); }
    let message = arguments.first().map(Value::show).unwrap_or_default();
    let code = arguments.get(1).map(|value| value.number() as i64).unwrap_or(0);
    let previous = arguments.get(2).map(Value::dereference).unwrap_or(Value::Null);
    let valid_previous = match &previous {
        Value::Null => true,
        Value::Object(object) => crate::vm::is_a_type(
            &object.borrow().class_name.to_ascii_lowercase(),
            "throwable",
            classes,
            &mut std::collections::HashSet::new(),
        ),
        _ => false,
    };
    if !valid_previous {
        return Err(format!("{}::__construct(): Argument #3 ($previous) must be of type ?Throwable", class_name));
    }
    Ok(PhpExceptionData { message, code, previous, file: String::new(), line: None, trace: Vec::new() })
}

pub(crate) fn empty_data() -> PhpExceptionData {
    PhpExceptionData { message: String::new(), code: 0, previous: Value::Null, file: String::new(), line: None, trace: Vec::new() }
}

pub(crate) fn initialize_object(
    receiver: &Value,
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    let Value::Object(object) = receiver else { return Err("Exception constructor receiver must be an object".into()); };
    object.borrow_mut().exception = Some(construct_data(class_name, arguments, classes)?);
    Ok(())
}

pub(crate) fn call_method(receiver: &Value, method: &str, arguments: &[Value]) -> Result<Value, String> {
    let Value::Object(object) = receiver else { return Err("Exception method receiver must be an object".into()); };
    let (class_name, data) = {
        let borrowed = object.borrow();
        (borrowed.class_name.clone(), borrowed.exception.clone())
    };
    let Some(data) = data else { return Err(format!("Call to undefined method {}::{method}()", class_name)); };
    if !arguments.is_empty() { return Err(format!("{}::{method}() expects exactly 0 arguments", class_name)); }
    match method.to_ascii_lowercase().as_str() {
        "getmessage" => Ok(Value::String(data.message)),
        "getcode" => Ok(Value::Integer(data.code)),
        "getprevious" => Ok(data.previous),
        "getfile" => Ok(Value::String(data.file)),
        "getline" => Ok(Value::Integer(data.line.unwrap_or(0) as i64)),
        "gettrace" => Ok(Value::from_array_entries(data.trace.iter().enumerate().map(|(index, frame)| {
            let mut fields = vec![
                (ArrayKey::String("file".into()), Value::String(frame.file.clone())),
                (ArrayKey::String("line".into()), Value::Integer(frame.line as i64)),
                (ArrayKey::String("function".into()), Value::String(frame.function.clone())),
            ];
            if let Some(class) = &frame.class {
                fields.push((ArrayKey::String("class".into()), Value::String(class.clone())));
                fields.push((ArrayKey::String("type".into()), Value::String(frame.call_type.clone().unwrap_or_default())));
            }
            (ArrayKey::Integer(index as i64), Value::from_array_entries(fields))
        }).collect())),
        "gettraceasstring" => {
            let mut lines = data.trace.iter().enumerate().map(|(index, frame)| {
                let location = if frame.file.is_empty() { format!("Command line code({}):", frame.line) }
                    else { format!("{}({}):", frame.file, frame.line) };
                let callable = match (&frame.class, &frame.call_type) {
                    (Some(class), Some(call_type)) => format!("{class}{call_type}{}()", frame.function),
                    _ => format!("{}()", frame.function),
                };
                format!("#{index} {location} {callable}\n")
            }).collect::<String>();
            lines.push_str(&format!("#{} {{main}}", data.trace.len()));
            Ok(Value::String(lines))
        }
        "__tostring" => Ok(Value::String(format!("{}: {}", class_name, data.message))),
        _ => Err(format!("Call to undefined method {}::{method}()", class_name)),
    }
}

pub(crate) fn stringify(receiver: &Value) -> Result<String, String> {
    match call_method(receiver, "__toString", &[])? {
        Value::String(value) => Ok(value),
        _ => Err("Exception::__toString() must return a string".into()),
    }
}
