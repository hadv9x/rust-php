//! Native objects and nominal type relationships for PHP's built-in Throwable classes.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};

use crate::{ArrayKey, BytecodeClass, PhpExceptionData, PhpObject, Value};

const EXCEPTION_CLASSES: &[&str] = &[
    "exception",
    "errorexception",
    "error",
    "typeerror",
    "argumentcounterror",
    "compileerror",
    "parseerror",
    "arithmeticerror",
    "divisionbyzeroerror",
    "assertionerror",
    "unhandledmatcherror",
    "valueerror",
    "jsonexception",
    "logicexception",
    "badfunctioncallexception",
    "badmethodcallexception",
    "domainexception",
    "invalidargumentexception",
    "lengthexception",
    "outofrangeexception",
    "runtimeexception",
    "outofboundsexception",
    "overflowexception",
    "rangeexception",
    "underflowexception",
    "unexpectedvalueexception",
    "reflectionexception",
];

pub(crate) fn is_class(name: &str) -> bool {
    EXCEPTION_CLASSES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name.trim_start_matches('\\')))
}

pub(crate) fn is_interface(name: &str) -> bool {
    matches!(
        name.trim_start_matches('\\').to_ascii_lowercase().as_str(),
        "throwable" | "jsonserializable" | "stringable" | "countable"
    )
}

pub(crate) fn canonical_name(name: &str) -> Option<&'static str> {
    EXCEPTION_CLASSES
        .iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(name.trim_start_matches('\\')))
        .map(|name| match name {
            "exception" => "Exception",
            "errorexception" => "ErrorException",
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
            "logicexception" => "LogicException",
            "badfunctioncallexception" => "BadFunctionCallException",
            "badmethodcallexception" => "BadMethodCallException",
            "domainexception" => "DomainException",
            "invalidargumentexception" => "InvalidArgumentException",
            "lengthexception" => "LengthException",
            "outofrangeexception" => "OutOfRangeException",
            "runtimeexception" => "RuntimeException",
            "outofboundsexception" => "OutOfBoundsException",
            "overflowexception" => "OverflowException",
            "rangeexception" => "RangeException",
            "underflowexception" => "UnderflowException",
            "unexpectedvalueexception" => "UnexpectedValueException",
            "reflectionexception" => "ReflectionException",
            _ => unreachable!(),
        })
}

pub(crate) fn parent_class(name: &str) -> Option<&'static str> {
    match name.trim_start_matches('\\').to_ascii_lowercase().as_str() {
        "exception" | "error" => Some("Throwable"),
        "errorexception" => Some("Exception"),
        "jsonexception" => Some("Exception"),
        "typeerror"
        | "compileerror"
        | "arithmeticerror"
        | "assertionerror"
        | "unhandledmatcherror"
        | "valueerror" => Some("Error"),
        "argumentcounterror" => Some("TypeError"),
        "parseerror" => Some("CompileError"),
        "divisionbyzeroerror" => Some("ArithmeticError"),
        "logicexception" => Some("Exception"),
        "badfunctioncallexception"
        | "domainexception"
        | "invalidargumentexception"
        | "lengthexception"
        | "outofrangeexception" => Some("LogicException"),
        "badmethodcallexception" => Some("BadFunctionCallException"),
        "runtimeexception" => Some("Exception"),
        "reflectionexception" => Some("Exception"),
        "outofboundsexception"
        | "overflowexception"
        | "rangeexception"
        | "underflowexception"
        | "unexpectedvalueexception" => Some("RuntimeException"),
        _ => None,
    }
}

pub(crate) fn is_a(actual: &str, expected: &str) -> bool {
    let expected = expected.trim_start_matches('\\');
    if expected.eq_ignore_ascii_case("Stringable")
        && (actual.eq_ignore_ascii_case("Throwable") || is_class(actual))
    {
        return true;
    }
    if actual.eq_ignore_ascii_case(expected) {
        return true;
    }
    let mut current = actual;
    while let Some(parent) = parent_class(current) {
        if parent.eq_ignore_ascii_case(expected) {
            return true;
        }
        current = parent;
    }
    false
}

pub(crate) fn method_exists(class_name: &str, method: &str) -> bool {
    let method = method.to_ascii_lowercase();
    if class_name
        .trim_start_matches('\\')
        .eq_ignore_ascii_case("JsonSerializable")
    {
        return method == "jsonserialize";
    }
    if class_name
        .trim_start_matches('\\')
        .eq_ignore_ascii_case("Stringable")
    {
        return method == "__tostring";
    }
    if is_interface(class_name) {
        return matches!(
            method.as_str(),
            "getmessage"
                | "getcode"
                | "getfile"
                | "getline"
                | "gettrace"
                | "gettraceasstring"
                | "getprevious"
                | "__tostring"
        );
    }
    (is_class(class_name)
        && matches!(
            method.as_str(),
            "__construct"
                | "getmessage"
                | "getcode"
                | "getprevious"
                | "getfile"
                | "getline"
                | "gettrace"
                | "gettraceasstring"
                | "__tostring"
        ))
        || class_name.trim_start_matches('\\').eq_ignore_ascii_case("ErrorException")
            && method == "getseverity"
}

pub(crate) fn construct(
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    construct_internal(class_name, arguments, classes, None)
}

pub(crate) fn construct_with_stringifier(
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    stringifier: &mut dyn FnMut(&Value) -> Result<String, String>,
) -> Result<Value, String> {
    construct_internal(class_name, arguments, classes, Some(stringifier))
}

fn construct_internal(
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    stringifier: Option<&mut dyn FnMut(&Value) -> Result<String, String>>,
) -> Result<Value, String> {
    if class_name.eq_ignore_ascii_case("Throwable") {
        return Err("Cannot instantiate interface Throwable".into());
    }
    let data = construct_data_internal(class_name, arguments, classes, stringifier)?;
    let properties = data
        .is_error_exception
        .then(|| HashMap::from([("severity".into(), Value::Integer(data.severity))]))
        .unwrap_or_default();
    let property_order = data
        .is_error_exception
        .then(|| vec!["severity".into()])
        .unwrap_or_default();
    Ok(Value::Object(Rc::new(RefCell::new(PhpObject {
        class_name: canonical_name(class_name).unwrap_or("Exception").into(),
        properties,
        property_order,
        active_property_hooks: HashSet::new(),
        exception: Some(data),
    }))))
}

fn construct_data_internal(
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    mut stringifier: Option<&mut dyn FnMut(&Value) -> Result<String, String>>,
) -> Result<PhpExceptionData, String> {
    let display_name = canonical_name(class_name).unwrap_or(class_name);
    let is_error_exception = crate::vm::is_a_type(
        &class_name.to_ascii_lowercase(),
        "errorexception",
        classes,
        &mut HashSet::new(),
    );
    if arguments.len() > if is_error_exception { 6 } else { 3 } {
        return Err(format!(
            "{display_name}::__construct() expects at most {} arguments",
            if is_error_exception { 6 } else { 3 }
        ));
    }
    let message = match arguments.first() {
        Some(value) => coerce_exception_message(value, display_name, &mut stringifier)?,
        None => String::new(),
    };
    let code = arguments
        .get(1)
        .map(|value| coerce_exception_integer(value, display_name, 2, "code"))
        .transpose()?
        .unwrap_or(0);
    let severity = if is_error_exception {
        arguments
            .get(2)
            .map(|value| coerce_exception_integer(value, display_name, 3, "severity"))
            .transpose()?
            .unwrap_or(1)
    } else {
        1
    };
    let filename = if is_error_exception {
        arguments.get(3).and_then(|value| match value.dereference() {
            Value::Null => None,
            Value::String(filename) => Some(filename),
            Value::Integer(_) | Value::Number(_) | Value::Bool(_) => Some(value.show()),
            _ => None,
        })
    } else {
        None
    };
    if is_error_exception
        && arguments.get(3).is_some_and(|value| {
            !matches!(
                value.dereference(),
                Value::Null
                    | Value::String(_)
                    | Value::Integer(_)
                    | Value::Number(_)
                    | Value::Bool(_)
            )
        })
    {
        let value = arguments[3].dereference();
        return Err(format!(
            "TypeError:{display_name}::__construct(): Argument #4 ($filename) must be of type ?string, {} given",
            value.type_name()
        ));
    }
    let line = if is_error_exception {
        match arguments.get(4).map(Value::dereference) {
            None | Some(Value::Null) => filename.as_ref().map(|_| 0),
            Some(value) => Some(coerce_exception_integer(
                &value,
                display_name,
                5,
                "line",
            )?),
        }
    } else {
        None
    };
    let previous = arguments
        .get(if is_error_exception { 5 } else { 2 })
        .map(Value::dereference)
        .unwrap_or(Value::Null);
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
        return Err(format!(
            "{display_name}::__construct(): Argument #{} ($previous) must be of type ?Throwable",
            if is_error_exception { 6 } else { 3 }
        ));
    }
    Ok(PhpExceptionData {
        message,
        code,
        severity,
        is_error_exception,
        previous,
        file: filename.unwrap_or_default(),
        line: line.map(|line| line as usize),
        trace: Vec::new(),
    })
}

fn coerce_exception_integer(
    value: &Value,
    class_name: &str,
    position: usize,
    parameter: &str,
) -> Result<i64, String> {
    match value.dereference() {
        Value::Integer(value) => Ok(value),
        Value::Number(value) => Ok(value as i64),
        Value::Bool(value) => Ok(i64::from(value)),
        Value::Null => Ok(0),
        Value::String(value) => {
            let numeric = value.trim();
            numeric
                .parse::<i64>()
                .or_else(|_| {
                    numeric
                        .parse::<f64>()
                        .ok()
                        .filter(|number| number.is_finite())
                        .map(|number| number as i64)
                        .ok_or(())
                })
                .map_err(|_| {
                    format!(
                        "TypeError:{class_name}::__construct(): Argument #{position} (${parameter}) must be of type int, string given"
                    )
                })
        }
        value => Err(format!(
            "TypeError:{class_name}::__construct(): Argument #{position} (${parameter}) must be of type int, {} given",
            value.type_name()
        )),
    }
}

fn coerce_exception_message(
    value: &Value,
    class_name: &str,
    stringifier: &mut Option<&mut dyn FnMut(&Value) -> Result<String, String>>,
) -> Result<String, String> {
    match value.dereference() {
        Value::String(value) => Ok(value),
        Value::Integer(_) | Value::Number(_) | Value::Bool(_) | Value::Null => Ok(value.show()),
        Value::Object(_) if stringifier.is_some() => {
            let stringified = stringifier.as_mut().expect("checked stringifier")(value);
            match stringified {
                Ok(value) => Ok(value),
                Err(error) if error.starts_with("Object of class ") => Err(format!(
                    "TypeError:{class_name}::__construct(): Argument #1 ($message) must be of type string, {} given",
                    value.type_name()
                )),
                Err(error) => Err(error),
            }
        }
        value => Err(format!(
            "TypeError:{class_name}::__construct(): Argument #1 ($message) must be of type string, {} given",
            value.type_name()
        )),
    }
}

pub(crate) fn empty_data() -> PhpExceptionData {
    PhpExceptionData {
        message: String::new(),
        code: 0,
        severity: 1,
        is_error_exception: false,
        previous: Value::Null,
        file: String::new(),
        line: None,
        trace: Vec::new(),
    }
}

pub(crate) fn initialize_object_with_stringifier(
    receiver: &Value,
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    stringifier: &mut dyn FnMut(&Value) -> Result<String, String>,
) -> Result<(), String> {
    initialize_object_internal(receiver, class_name, arguments, classes, Some(stringifier))
}

fn initialize_object_internal(
    receiver: &Value,
    class_name: &str,
    arguments: &[Value],
    classes: &HashMap<String, BytecodeClass>,
    stringifier: Option<&mut dyn FnMut(&Value) -> Result<String, String>>,
) -> Result<(), String> {
    let Value::Object(object) = receiver else {
        return Err("Exception constructor receiver must be an object".into());
    };
    let data = construct_data_internal(class_name, arguments, classes, stringifier)?;
    let mut borrowed = object.borrow_mut();
    if data.is_error_exception {
        borrowed
            .properties
            .insert("severity".into(), Value::Integer(data.severity));
        if !borrowed
            .property_order
            .iter()
            .any(|property| property == "severity")
        {
            borrowed.property_order.push("severity".into());
        }
    }
    borrowed.exception = Some(data);
    Ok(())
}

pub(crate) fn call_method(
    receiver: &Value,
    method: &str,
    arguments: &[Value],
    string_param_max_len: usize,
) -> Result<Value, String> {
    let Value::Object(object) = receiver else {
        return Err("Exception method receiver must be an object".into());
    };
    let (class_name, data, severity) = {
        let borrowed = object.borrow();
        (
            borrowed.class_name.clone(),
            borrowed.exception.clone(),
            borrowed
                .properties
                .get("severity")
                .map(|value| value.number() as i64),
        )
    };
    let Some(data) = data else {
        return Err(format!(
            "Call to undefined method {}::{method}()",
            class_name
        ));
    };
    if !arguments.is_empty() {
        return Err(format!(
            "{}::{method}() expects exactly 0 arguments",
            class_name
        ));
    }
    match method.to_ascii_lowercase().as_str() {
        "getseverity" if data.is_error_exception => {
            Ok(Value::Integer(severity.unwrap_or(data.severity)))
        }
        "getmessage" => Ok(Value::String(data.message)),
        "getcode" => Ok(Value::Integer(data.code)),
        "getprevious" => Ok(data.previous),
        "getfile" => Ok(Value::String(if data.file.is_empty() {
            "Command line code".into()
        } else {
            data.file
        })),
        "getline" => Ok(Value::Integer(data.line.unwrap_or(0) as i64)),
        "gettrace" => Ok(Value::from_array_entries(
            data.trace
                .iter()
                .enumerate()
                .map(|(index, frame)| {
                    let mut fields = vec![
                        (
                            ArrayKey::String("file".into()),
                            Value::String(frame.file.clone()),
                        ),
                        (
                            ArrayKey::String("line".into()),
                            Value::Integer(frame.line as i64),
                        ),
                        (
                            ArrayKey::String("function".into()),
                            Value::String(frame.function.clone()),
                        ),
                        (
                            ArrayKey::String("args".into()),
                            Value::from_array_entries(
                                frame
                                    .arguments
                                    .iter()
                                    .cloned()
                                    .enumerate()
                                    .map(|(index, value)| (ArrayKey::Integer(index as i64), value))
                                    .collect(),
                            ),
                        ),
                    ];
                    if let Some(class) = &frame.class {
                        fields.push((
                            ArrayKey::String("class".into()),
                            Value::String(class.clone()),
                        ));
                        fields.push((
                            ArrayKey::String("type".into()),
                            Value::String(frame.call_type.clone().unwrap_or_default()),
                        ));
                    }
                    (
                        ArrayKey::Integer(index as i64),
                        Value::from_array_entries(fields),
                    )
                })
                .collect(),
        )),
        "gettraceasstring" => {
            let mut lines = data
                .trace
                .iter()
                .enumerate()
                .map(|(index, frame)| {
                    let location = if frame.file.is_empty() {
                        format!("Command line code({}):", frame.line)
                    } else {
                        format!("{}({}):", frame.file, frame.line)
                    };
                    let rendered_arguments = frame
                        .arguments
                        .iter()
                        .map(|argument| render_trace_argument(argument, string_param_max_len))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let callable = match (&frame.class, &frame.call_type) {
                        (Some(class), Some(call_type)) => {
                            format!("{class}{call_type}{}({rendered_arguments})", frame.function)
                        }
                        _ => format!("{}({rendered_arguments})", frame.function),
                    };
                    format!("#{index} {location} {callable}\n")
                })
                .collect::<String>();
            lines.push_str(&format!("#{} {{main}}", data.trace.len()));
            Ok(Value::String(lines))
        }
        "__tostring" => Ok(Value::String(stringify_exception(
            receiver,
            &mut HashSet::new(),
        )?)),
        _ => Err(format!(
            "Call to undefined method {}::{method}()",
            class_name
        )),
    }
}

fn render_trace_argument(value: &Value, max_len: usize) -> String {
    match value.dereference() {
        Value::String(value) => {
            let truncated = max_len > 0 && value.len() > max_len;
            let mut boundary = if max_len == 0 {
                value.len()
            } else {
                value.len().min(max_len)
            };
            while !value.is_char_boundary(boundary) {
                boundary -= 1;
            }
            let visible = &value[..boundary];
            let suffix = if truncated { "..." } else { "" };
            format!("'{}{}'", escape_trace_string(visible), suffix)
        }
        Value::Array(_) => "Array".into(),
        Value::Object(object) => format!("Object({})", object.borrow().class_name),
        Value::Closure(_) => "Object(Closure)".into(),
        Value::Generator(_) => "Object(Generator)".into(),
        Value::Stream(stream) => format!("Resource id #{}", stream.borrow().resource_id()),
        Value::Null => "NULL".into(),
        Value::Bool(true) => "true".into(),
        Value::Bool(false) => "false".into(),
        value => value.show(),
    }
}

fn escape_trace_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        escaped.push_str(match character {
            '\\' => "\\\\",
            '\'' => "\\'",
            '\0' => "\\0",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '\x1b' => "\\e",
            _ => {
                escaped.push(character);
                continue;
            }
        });
    }
    escaped
}

fn stringify_exception(receiver: &Value, seen: &mut HashSet<usize>) -> Result<String, String> {
    let Value::Object(object) = receiver else {
        return Err("Exception::__toString() must return a string".into());
    };
    let identity = Rc::as_ptr(object) as usize;
    if !seen.insert(identity) {
        return Ok(String::new());
    }
    let borrowed = object.borrow();
    let class_name = borrowed.class_name.clone();
    let Some(data) = borrowed.exception.clone() else {
        return Err(format!(
            "Call to undefined method {class_name}::__toString()"
        ));
    };
    drop(borrowed);

    let mut trace = data
        .trace
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let location = if frame.file.is_empty() {
                format!("Command line code({}):", frame.line)
            } else {
                format!("{}({}):", frame.file, frame.line)
            };
            let callable = match (&frame.class, &frame.call_type) {
                (Some(class), Some(call_type)) => format!("{class}{call_type}{}()", frame.function),
                _ => format!("{}()", frame.function),
            };
            format!("#{index} {location} {callable}\n")
        })
        .collect::<String>();
    trace.push_str(&format!("#{} {{main}}\n", data.trace.len()));
    let file = if data.file.is_empty() {
        "Command line code"
    } else {
        &data.file
    };
    let head = if data.message.is_empty() {
        format!(
            "{class_name} in {}:{}\nStack trace:\n{trace}",
            file,
            data.line.unwrap_or(0)
        )
    } else {
        format!(
            "{class_name}: {} in {}:{}\nStack trace:\n{trace}",
            data.message,
            file,
            data.line.unwrap_or(0)
        )
    };
    let previous = match data.previous {
        Value::Object(previous) if previous.borrow().exception.is_some() => {
            let previous_value = Value::Object(previous);
            let previous_text = stringify_exception(&previous_value, seen)?;
            if previous_text.is_empty() {
                String::new()
            } else {
                format!("{previous_text}\n\nNext {head}")
            }
        }
        _ => head,
    };
    seen.remove(&identity);
    Ok(previous)
}

pub(crate) fn stringify(receiver: &Value) -> Result<String, String> {
    match call_method(receiver, "__toString", &[], 15)? {
        Value::String(value) => Ok(value),
        _ => Err("Exception::__toString() must return a string".into()),
    }
}
