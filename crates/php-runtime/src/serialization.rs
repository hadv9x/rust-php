//! PHP's scalar and ordered-array serialization format.

use crate::{ArrayKey, BytecodeClass, Value};
use std::collections::HashMap;

const MAX_DEPTH: usize = 256;
pub(crate) const DEFAULT_UNSERIALIZE_DEPTH: usize = 4096;

pub(crate) enum SerializationHook {
    Serialize(Value),
    Sleep {
        names: Vec<String>,
        scope: Option<String>,
    },
}

pub(crate) fn serialize(
    value: &Value,
    classes: &HashMap<String, BytecodeClass>,
    hook: &mut dyn FnMut(&Value) -> Result<Option<SerializationHook>, String>,
) -> Result<String, String> {
    let mut output = String::new();
    encode(value, &mut output, 0, classes, hook)?;
    Ok(output)
}

fn encode(
    value: &Value,
    output: &mut String,
    depth: usize,
    classes: &HashMap<String, BytecodeClass>,
    hook: &mut dyn FnMut(&Value) -> Result<Option<SerializationHook>, String>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("serialize(): nesting level too deep".into());
    }
    match value.dereference() {
        Value::Null => output.push_str("N;"),
        Value::Bool(value) => output.push_str(if value { "b:1;" } else { "b:0;" }),
        Value::Integer(value) => output.push_str(&format!("i:{value};")),
        Value::Number(value) => output.push_str(&format!("d:{};", Value::Number(value).show())),
        Value::String(value) => encode_string(&value, output),
        Value::Array(array) => encode_entries(
            'a',
            array.len(),
            array.iter().map(|(key, value)| (key, value)),
            output,
            depth,
            classes,
            hook,
        )?,
        Value::Object(object) => {
            let receiver = Value::Object(object.clone());
            let custom = hook(&receiver)?;
            let object = object.borrow();
            output.push_str(&format!("O:{}:\"{}\":", object.class_name.len(), object.class_name));
            match custom {
                Some(SerializationHook::Serialize(Value::Array(custom))) => {
                    output.push_str(&format!("{}:{{", custom.len()));
                    for (key, value) in custom.iter() {
                        encode_key(key, output);
                        encode(value, output, depth + 1, classes, hook)?;
                    }
                }
                Some(SerializationHook::Serialize(_)) => {
                    return Err(format!(
                        "{}::__serialize() must return an array",
                        object.class_name
                    ));
                }
                Some(SerializationHook::Sleep { names, scope }) => {
                    let mut properties = Vec::new();
                    for selected_name in names {
                        if let Some(storage_name) =
                            sleep_property_storage_name(&object.properties, &selected_name, scope.as_deref())
                        {
                            if let Some(value) = object.properties.get(&storage_name) {
                                properties.push((storage_name, value.clone()));
                            }
                        }
                    }
                    output.push_str(&format!("{}:{{", properties.len()));
                    for (name, value) in properties {
                        encode_string(&serialized_property_name(&name, classes), output);
                        encode(&value, output, depth + 1, classes, hook)?;
                    }
                }
                None => {
                    let mut names = object
                        .property_order
                        .iter()
                        .filter(|name| object.properties.contains_key(*name))
                        .cloned()
                        .collect::<Vec<_>>();
                    let mut remaining = object
                        .properties
                        .keys()
                        .filter(|name| !names.contains(name))
                        .cloned()
                        .collect::<Vec<_>>();
                    remaining.sort();
                    names.extend(remaining);
                    output.push_str(&format!("{}:{{", names.len()));
                    for name in names {
                        let serialized_name = serialized_property_name(&name, classes);
                        encode_string(&serialized_name, output);
                        encode(&object.properties[&name], output, depth + 1, classes, hook)?;
                    }
                }
            }
            output.push('}');
        }
        Value::Stream(_) => output.push_str("i:0;"),
        Value::Closure(_) | Value::Generator(_) => {
            return Err("Serialization of this value type is not supported yet".into());
        }
        Value::Reference(_) => unreachable!("dereference removes reference wrappers"),
    }
    Ok(())
}

fn sleep_property_storage_name(
    properties: &std::collections::HashMap<String, Value>,
    name: &str,
    scope: Option<&str>,
) -> Option<String> {
    if let Some(mangled) = name.strip_prefix('\0') {
        if let Some((class_name, property)) = mangled.split_once('\0') {
            if class_name == "*" {
                return properties
                    .contains_key(property)
                    .then(|| property.to_string());
            }
            let storage_name = format!("\0{}::{property}", class_name.to_ascii_lowercase());
            return properties
                .contains_key(&storage_name)
                .then_some(storage_name);
        }
    }
    if let Some(scope) = scope {
        let private_name = format!("\0{}::{name}", scope.to_ascii_lowercase());
        if properties.contains_key(&private_name) {
            return Some(private_name);
        }
    }
    properties.contains_key(name).then(|| name.to_string())
}

fn encode_entries<'a>(
    tag: char,
    count: usize,
    entries: impl Iterator<Item = (&'a ArrayKey, &'a Value)>,
    output: &mut String,
    depth: usize,
    classes: &HashMap<String, BytecodeClass>,
    hook: &mut dyn FnMut(&Value) -> Result<Option<SerializationHook>, String>,
) -> Result<(), String> {
    output.push_str(&format!("{tag}:{count}:{{"));
    for (key, value) in entries {
        encode_key(key, output);
        encode(value, output, depth + 1, classes, hook)?;
    }
    output.push('}');
    Ok(())
}

fn encode_key(key: &ArrayKey, output: &mut String) {
    match key {
        ArrayKey::Integer(value) => output.push_str(&format!("i:{value};")),
        ArrayKey::String(value) => encode_string(value, output),
    }
}

fn serialized_property_name(name: &str, classes: &HashMap<String, BytecodeClass>) -> String {
    let Some(mangled) = name.strip_prefix('\0') else {
        return name.to_string();
    };
    let Some((scope, property)) = mangled.split_once("::") else {
        return name.to_string();
    };
    if scope == "*" {
        format!("\0*\0{property}")
    } else {
        let scope = classes
            .get(&scope.to_ascii_lowercase())
            .map(|class| class.name.as_str())
            .unwrap_or(scope);
        format!("\0{scope}\0{property}")
    }
}

fn encode_string(value: &str, output: &mut String) {
    output.push_str(&format!("s:{}:\"", value.len()));
    output.push_str(value);
    output.push_str("\";");
}

pub(crate) fn unserialize(
    serialized: &str,
    max_depth: usize,
    object_hook: &mut dyn FnMut(&str, Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let mut parser = Parser {
        bytes: serialized.as_bytes(),
        offset: 0,
        max_depth,
        object_hook,
    };
    match parser.value(0) {
        Ok(value) if parser.offset == parser.bytes.len() => Ok(value),
        Err(ParseError::Runtime(error)) => Err(error),
        _ => Ok(Value::Bool(false)),
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
    max_depth: usize,
    object_hook: &'a mut dyn FnMut(&str, Value) -> Result<Value, String>,
}

enum ParseError {
    Invalid,
    Runtime(String),
}

impl Parser<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if self.max_depth != 0 && depth > self.max_depth {
            return Err(ParseError::Invalid);
        }
        match self.byte()? {
            b'N' => {
                self.expect(b';')?;
                Ok(Value::Null)
            }
            b'b' => {
                self.expect(b':')?;
                let value = self.byte()?;
                self.expect(b';')?;
                match value {
                    b'0' => Ok(Value::Bool(false)),
                    b'1' => Ok(Value::Bool(true)),
                    _ => Err(ParseError::Invalid),
                }
            }
            b'i' => {
                self.expect(b':')?;
                let value = self.until(b';')?;
                std::str::from_utf8(value)
                    .map_err(|_| ParseError::Invalid)?
                    .parse::<i64>()
                    .map(Value::Integer)
                    .map_err(|_| ParseError::Invalid)
            }
            b'd' => {
                self.expect(b':')?;
                let value = std::str::from_utf8(self.until(b';')?).map_err(|_| ParseError::Invalid)?;
                match value {
                    "NAN" => Ok(Value::Number(f64::NAN)),
                    "INF" => Ok(Value::Number(f64::INFINITY)),
                    "-INF" => Ok(Value::Number(f64::NEG_INFINITY)),
                    _ => value.parse::<f64>().map(Value::Number).map_err(|_| ParseError::Invalid),
                }
            }
            b's' => {
                self.expect(b':')?;
                let length = std::str::from_utf8(self.until(b':')?)
                    .map_err(|_| ParseError::Invalid)?
                    .parse::<usize>()
                    .map_err(|_| ParseError::Invalid)?;
                self.expect(b'"')?;
                let end = self.offset.checked_add(length).ok_or(ParseError::Invalid)?;
                let bytes = self.bytes.get(self.offset..end).ok_or(ParseError::Invalid)?;
                self.offset = end;
                self.expect(b'"')?;
                self.expect(b';')?;
                Ok(Value::String(String::from_utf8_lossy(bytes).into_owned()))
            }
            b'a' => {
                self.expect(b':')?;
                let count = std::str::from_utf8(self.until(b':')?)
                    .map_err(|_| ParseError::Invalid)?
                    .parse::<usize>()
                    .map_err(|_| ParseError::Invalid)?;
                self.expect(b'{')?;
                let mut entries = Vec::with_capacity(count.min(1_000_000));
                for _ in 0..count {
                    let key = match self.value(depth + 1)? {
                        Value::Integer(key) => ArrayKey::Integer(key),
                        Value::String(key) => ArrayKey::String(key),
                        _ => return Err(ParseError::Invalid),
                    };
                    entries.push((key, self.value(depth + 1)?));
                }
                self.expect(b'}')?;
                Ok(Value::from_array_entries(entries))
            }
            b'O' => {
                self.expect(b':')?;
                let class_length = std::str::from_utf8(self.until(b':')?)
                    .map_err(|_| ParseError::Invalid)?
                    .parse::<usize>()
                    .map_err(|_| ParseError::Invalid)?;
                self.expect(b'"')?;
                let class_end = self.offset.checked_add(class_length).ok_or(ParseError::Invalid)?;
                let class_bytes = self.bytes.get(self.offset..class_end).ok_or(ParseError::Invalid)?;
                let class_name = String::from_utf8_lossy(class_bytes).into_owned();
                self.offset = class_end;
                self.expect(b'"')?;
                self.expect(b':')?;
                let count = std::str::from_utf8(self.until(b':')?)
                    .map_err(|_| ParseError::Invalid)?
                    .parse::<usize>()
                    .map_err(|_| ParseError::Invalid)?;
                self.expect(b'{')?;
                let mut entries = Vec::with_capacity(count.min(1_000_000));
                for _ in 0..count {
                    let key = match self.value(depth + 1)? {
                        Value::Integer(key) => ArrayKey::Integer(key),
                        Value::String(key) => ArrayKey::String(key),
                        _ => return Err(ParseError::Invalid),
                    };
                    entries.push((key, self.value(depth + 1)?));
                }
                self.expect(b'}')?;
                (self.object_hook)(&class_name, Value::from_array_entries(entries))
                    .map_err(ParseError::Runtime)
            }
            _ => Err(ParseError::Invalid),
        }
    }

    fn byte(&mut self) -> Result<u8, ParseError> {
        let byte = *self.bytes.get(self.offset).ok_or(ParseError::Invalid)?;
        self.offset += 1;
        Ok(byte)
    }

    fn expect(&mut self, expected: u8) -> Result<(), ParseError> {
        if self.byte()? == expected {
            Ok(())
        } else {
            Err(ParseError::Invalid)
        }
    }

    fn until(&mut self, delimiter: u8) -> Result<&[u8], ParseError> {
        let relative = self.bytes[self.offset..]
            .iter()
            .position(|byte| *byte == delimiter)
            .ok_or(ParseError::Invalid)?;
        let end = self.offset + relative;
        let value = &self.bytes[self.offset..end];
        self.offset = end + 1;
        Ok(value)
    }
}
