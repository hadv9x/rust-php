//! JSON encoding and decoding for values supported by the PHP runtime.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::{ArrayKey, BytecodeClass, PhpObject, Value};

#[derive(Clone, Copy)]
pub(super) struct EncodeOptions {
    pub pretty: bool,
    pub unescaped_slashes: bool,
    pub unescaped_unicode: bool,
    pub unescaped_line_terminators: bool,
    pub force_object: bool,
    pub hex_flags: i64,
    pub preserve_zero_fraction: bool,
    pub numeric_check: bool,
    pub partial_output: bool,
}

#[cfg(test)]
pub(super) fn encode(value: &Value, max_depth: usize, options: EncodeOptions) -> Result<(String, i64), String> {
    encode_with_classes(value, max_depth, options, &HashMap::new())
}

#[cfg(test)]
pub(super) fn encode_with_classes(value: &Value, max_depth: usize, options: EncodeOptions, classes: &HashMap<String, BytecodeClass>) -> Result<(String, i64), String> {
    encode_with_serializer(value, max_depth, options, classes, &mut |_| Ok(None))
}

pub(super) fn encode_with_serializer(
    value: &Value,
    max_depth: usize,
    options: EncodeOptions,
    classes: &HashMap<String, BytecodeClass>,
    serialize: &mut dyn FnMut(&Value) -> Result<Option<Value>, String>,
) -> Result<(String, i64), String> {
    let mut output = String::new();
    let mut error_code = 0;
    let mut recursion_stack = HashSet::new();
    encode_value(value, max_depth, 0, options, classes, serialize, &mut output, &mut error_code, &mut recursion_stack)?;
    Ok((output, error_code))
}

fn indent(output: &mut String, depth: usize) {
    output.push_str(&"  ".repeat(depth));
}

fn encode_value(value: &Value, max_depth: usize, depth: usize, options: EncodeOptions, classes: &HashMap<String, BytecodeClass>, serialize: &mut dyn FnMut(&Value) -> Result<Option<Value>, String>, output: &mut String, error_code: &mut i64, recursion_stack: &mut HashSet<(u8, usize)>) -> Result<(), String> {
    if depth > max_depth { return Err("Maximum stack depth exceeded".into()); }
    if let Value::Reference(reference) = value {
        let identity = (0, std::rc::Rc::as_ptr(reference) as usize);
        if !recursion_stack.insert(identity) {
            if options.partial_output { *error_code = 6; output.push_str("null"); return Ok(()); }
            return Err("Recursion detected".into());
        }
        let result = encode_value(&reference.borrow(), max_depth, depth, options, classes, serialize, output, error_code, recursion_stack);
        recursion_stack.remove(&identity);
        return result;
    }
    let value = value.dereference();
    if let Value::Object(object) = &value {
        let identity = (1, std::rc::Rc::as_ptr(object) as usize);
        if !recursion_stack.insert(identity) {
            if options.partial_output { *error_code = 6; output.push_str("null"); return Ok(()); }
            return Err("Recursion detected".into());
        }
        let result = encode_object(object, max_depth, depth, options, classes, serialize, output, error_code, recursion_stack);
        recursion_stack.remove(&identity);
        return result;
    }
    match &value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Integer(value) => output.push_str(&value.to_string()),
        Value::Number(value) if value.is_finite() => output.push_str(&format_json_float(*value, options.preserve_zero_fraction)),
        Value::Number(_) => {
            if options.partial_output { *error_code = 7; output.push('0'); }
            else { return Err("Inf and NaN cannot be JSON encoded".into()); }
        }
        Value::String(value) if options.numeric_check => {
            let trimmed = value.trim_matches(|character: char| matches!(character, ' ' | '\t' | '\n' | '\r' | '\u{000b}' | '\u{000c}'));
            if let Some((integer, number)) = parse_numeric_string(trimmed) {
                if let Some(integer) = integer {
                    output.push_str(&integer.to_string());
                } else if let Some(number) = number {
                    if number.is_finite() {
                        output.push_str(&format_json_float(number, options.preserve_zero_fraction));
                    } else if options.partial_output {
                        *error_code = 7;
                        output.push('0');
                    } else {
                        return Err("Inf and NaN cannot be JSON encoded".into());
                    }
                }
            } else {
                encode_string(value, options, output);
            }
        }
        Value::String(value) => encode_string(value, options, output),
        Value::Array(values) => {
            let is_list = !options.force_object && values.iter().enumerate().all(|(index, (key, _))| *key == ArrayKey::Integer(index as i64));
            if is_list {
                output.push('[');
                for (index, (_, value)) in values.iter().enumerate() {
                    if index > 0 { output.push(','); }
                    if options.pretty { output.push('\n'); indent(output, depth + 1); }
                    encode_value(value, max_depth, depth + 1, options, classes, serialize, output, error_code, recursion_stack)?;
                }
                if options.pretty && !values.is_empty() { output.push('\n'); indent(output, depth); }
                output.push(']');
            } else {
                output.push('{');
                for (index, (key, value)) in values.iter().enumerate() {
                    if index > 0 { output.push(','); }
                    let key = match key { ArrayKey::Integer(index) => index.to_string(), ArrayKey::String(key) => key.clone() };
                    if options.pretty { output.push('\n'); indent(output, depth + 1); }
                    encode_string(&key, options, output);
                    output.push(':');
                    if options.pretty { output.push(' '); }
                    encode_value(value, max_depth, depth + 1, options, classes, serialize, output, error_code, recursion_stack)?;
                }
                if options.pretty && !values.is_empty() { output.push('\n'); indent(output, depth); }
                output.push('}');
            }
        }
        Value::Object(_) => unreachable!("objects are encoded with identity tracking"),
        // PHP closures and generators are objects. Like other internal objects
        // without public properties, they encode as empty JSON objects.
        Value::Closure(_) | Value::Generator(_) => output.push_str("{}"),
        Value::Stream(_) => {
            if options.partial_output { *error_code = 8; output.push_str("null"); }
            else { return Err("Resources cannot be JSON encoded".into()); }
        }
        Value::Reference(_) => unreachable!("references are encoded with identity tracking"),
    }
    Ok(())
}

fn parse_numeric_string(value: &str) -> Option<(Option<i64>, Option<f64>)> {
    if value.is_empty() { return None; }
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (mantissa, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => {
            if unsigned[index + 1..].contains(['e', 'E']) { return None; }
            let exponent = &unsigned[index + 1..];
            let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
            if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
            (&unsigned[..index], true)
        }
        None => (unsigned, false),
    };
    let mut decimal_parts = mantissa.split('.');
    let whole = decimal_parts.next().unwrap_or_default();
    let fraction = decimal_parts.next();
    if decimal_parts.next().is_some() { return None; }
    let whole_is_digits = !whole.is_empty() && whole.bytes().all(|byte| byte.is_ascii_digit());
    let fraction_is_digits = fraction.is_some_and(|part| part.bytes().all(|byte| byte.is_ascii_digit()));
    if !whole_is_digits && !fraction_is_digits { return None; }
    if !exponent && fraction.is_none() {
        if let Ok(integer) = value.parse::<i64>() { return Some((Some(integer), None)); }
    }
    value.parse::<f64>().ok().map(|number| (None, Some(number)))
}

// Match zend_gcvt's shortest-roundtrip output layout: fixed notation for
// decimal exponents -4 through 16, scientific notation outside that range.
fn format_json_float(value: f64, preserve_zero_fraction: bool) -> String {
    let raw = value.to_string();
    let negative = raw.starts_with('-');
    let unsigned = raw.strip_prefix('-').unwrap_or(&raw);
    let (mantissa, explicit_exp) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let explicit_exp = explicit_exp.parse::<i32>().unwrap_or(0);
    let point = mantissa.find('.').unwrap_or(mantissa.len()) as i32;
    let mut digits: String = mantissa.chars().filter(|ch| *ch != '.').collect();
    let leading = digits.chars().take_while(|ch| *ch == '0').count();
    digits.drain(..leading);
    if digits.is_empty() {
        return match (negative, preserve_zero_fraction) {
            (true, true) => "-0.0".into(),
            (false, true) => "0.0".into(),
            (true, false) => "-0".into(),
            (false, false) => "0".into(),
        };
    }
    let mut decimal_pos = point + explicit_exp - leading as i32;
    while digits.ends_with('0') { digits.pop(); }
    let exponent = decimal_pos - 1;
    let sign = if negative { "-" } else { "" };
    if !(-4..17).contains(&exponent) {
        let mantissa = if digits.len() == 1 { format!("{}.0", &digits[..1]) } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        return format!("{sign}{mantissa}e{exponent:+}");
    }
    decimal_pos = exponent + 1;
    let mut formatted = if decimal_pos <= 0 {
        format!("0.{}{}", "0".repeat((-decimal_pos) as usize), digits)
    } else if decimal_pos as usize >= digits.len() {
        format!("{}{}", digits, "0".repeat(decimal_pos as usize - digits.len()))
    } else {
        format!("{}.{}", &digits[..decimal_pos as usize], &digits[decimal_pos as usize..])
    };
    if preserve_zero_fraction && !formatted.contains('.') { formatted.push_str(".0"); }
    format!("{sign}{formatted}")
}

fn encode_object(object: &std::rc::Rc<std::cell::RefCell<PhpObject>>, max_depth: usize, depth: usize, options: EncodeOptions, classes: &HashMap<String, BytecodeClass>, serialize: &mut dyn FnMut(&Value) -> Result<Option<Value>, String>, output: &mut String, error_code: &mut i64, recursion_stack: &mut HashSet<(u8, usize)>) -> Result<(), String> {
    let borrowed = object.borrow();
    if let Some(class) = classes.get(&borrowed.class_name.to_ascii_lowercase()).filter(|class| class.is_enum) {
        if class.enum_backing_type.is_some() {
            let backing = borrowed.properties.get("value").ok_or("Backed enum case has no backing value")?;
            return encode_value(backing, max_depth, depth, options, classes, serialize, output, error_code, recursion_stack);
        }
        if options.partial_output { *error_code = 11; output.push_str("null"); return Ok(()); }
        return Err("Non-backed enums have no default serialization".into());
    }
    drop(borrowed);
    let receiver = Value::Object(object.clone());
    if implements_json_serializable(&object.borrow().class_name, classes, &mut HashSet::new()) {
        let serialized = serialize(&receiver)?.ok_or("JsonSerializable::jsonSerialize() must be implemented")?;
        return encode_value(&serialized, max_depth, depth, options, classes, serialize, output, error_code, recursion_stack);
    }
    output.push('{');
    let object = object.borrow();
    let class_name = object.class_name.clone();
    let is_visible = |name: &str| json_property_is_public(&class_name, name, classes);
    let mut names = object.property_order.iter().filter(|name| object.properties.contains_key(*name) && is_visible(name)).cloned().collect::<Vec<_>>();
    let mut remaining = object.properties.keys().filter(|name| !name.starts_with('\0') && !names.contains(name) && is_visible(name)).cloned().collect::<Vec<_>>();
    remaining.sort();
    names.extend(remaining);
    for (index, name) in names.iter().enumerate() {
        if index > 0 { output.push(','); }
        if options.pretty { output.push('\n'); indent(output, depth + 1); }
        encode_string(name, options, output);
        output.push(':');
        if options.pretty { output.push(' '); }
        encode_value(&object.properties[name], max_depth, depth + 1, options, classes, serialize, output, error_code, recursion_stack)?;
    }
    if options.pretty && !names.is_empty() { output.push('\n'); indent(output, depth); }
    output.push('}');
    Ok(())
}

fn json_property_is_public(class_name: &str, property: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    let mut current = Some(class_name);
    let mut declared = false;
    let mut public = false;
    while let Some(name) = current {
        let Some(class) = classes.get(&name.to_ascii_lowercase()) else { break; };
        if let Some(metadata) = class.properties.get(property) {
            declared = true;
            public |= metadata.visibility == crate::MemberVisibility::Public;
        }
        current = class.parent.as_deref();
    }
    !declared || public
}

fn implements_json_serializable(class_name: &str, classes: &HashMap<String, BytecodeClass>, visited: &mut HashSet<String>) -> bool {
    let key = class_name.to_ascii_lowercase();
    if !visited.insert(key.clone()) { return false; }
    let Some(class) = classes.get(&key) else { return false; };
    class.interfaces.iter().any(|interface| {
        interface.eq_ignore_ascii_case("jsonserializable") || implements_json_serializable(interface, classes, visited)
    }) || class.parent.as_deref().is_some_and(|parent| implements_json_serializable(parent, classes, visited))
}

fn encode_string(value: &str, options: EncodeOptions, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '\u{2028}' | '\u{2029}' if !options.unescaped_line_terminators => output.push_str(&format!("\\u{:04x}", character as u32)),
            '"' if options.hex_flags & 8 != 0 => output.push_str("\\u0022"),
            '"' => output.push_str("\\\""),
            '&' if options.hex_flags & 2 != 0 => output.push_str("\\u0026"),
            '\'' if options.hex_flags & 4 != 0 => output.push_str("\\u0027"),
            '<' if options.hex_flags & 1 != 0 => output.push_str("\\u003c"),
            '>' if options.hex_flags & 1 != 0 => output.push_str("\\u003e"),
            '\\' => output.push_str("\\\\"),
            '/' if !options.unescaped_slashes => output.push_str("\\/"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character <= '\u{1f}' => output.push_str(&format!("\\u{:04x}", character as u32)),
            character if !options.unescaped_unicode && !character.is_ascii() => {
                let scalar = character as u32;
                if scalar <= 0xffff {
                    output.push_str(&format!("\\u{scalar:04x}"));
                } else {
                    let scalar = scalar - 0x10000;
                    let high = 0xd800 + (scalar >> 10);
                    let low = 0xdc00 + (scalar & 0x3ff);
                    output.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

pub(super) fn decode(source: &str, associative: bool, max_depth: usize, bigints_as_string: bool) -> Result<Value, String> {
    let mut parser = JsonParser { source, cursor: 0, associative, max_depth, bigints_as_string };
    let value = parser.value(0)?;
    parser.whitespace();
    if parser.cursor != source.len() { return Err("Trailing data".into()); }
    Ok(value)
}

struct JsonParser<'a> {
    source: &'a str,
    cursor: usize,
    associative: bool,
    max_depth: usize,
    bigints_as_string: bool,
}

impl JsonParser<'_> {
    fn whitespace(&mut self) {
        while let Some(character) = self.source[self.cursor..].chars().next() {
            if !matches!(character, ' ' | '\t' | '\n' | '\r') { break; }
            self.cursor += character.len_utf8();
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > self.max_depth { return Err("Maximum stack depth exceeded".into()); }
        self.whitespace();
        match self.source.as_bytes().get(self.cursor).copied() {
            Some(b'n') => { self.literal("null")?; Ok(Value::Null) }
            Some(b't') => { self.literal("true")?; Ok(Value::Bool(true)) }
            Some(b'f') => { self.literal("false")?; Ok(Value::Bool(false)) }
            Some(b'"') => self.string().map(Value::String),
            Some(b'[') => self.array(depth + 1),
            Some(b'{') => self.object(depth + 1),
            Some(b']' | b'}') => Err("State mismatch".into()),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err("Syntax error".into()),
        }
    }

    fn literal(&mut self, literal: &str) -> Result<(), String> {
        if self.source[self.cursor..].starts_with(literal) { self.cursor += literal.len(); Ok(()) }
        else { Err("Syntax error".into()) }
    }

    fn string(&mut self) -> Result<String, String> {
        self.cursor += 1;
        let mut result = String::new();
        loop {
            let character = self.source[self.cursor..].chars().next().ok_or("Unterminated string")?;
            self.cursor += character.len_utf8();
            match character {
                '"' => return Ok(result),
                '\\' => {
                    let escaped = self.source.as_bytes().get(self.cursor).copied().ok_or("Unterminated escape")?;
                    self.cursor += 1;
                    match escaped {
                        b'"' => result.push('"'), b'\\' => result.push('\\'), b'/' => result.push('/'),
                        b'b' => result.push('\u{08}'), b'f' => result.push('\u{0c}'), b'n' => result.push('\n'),
                        b'r' => result.push('\r'), b't' => result.push('\t'), b'u' => result.push(self.unicode_escape()?),
                        _ => return Err("Invalid escape sequence".into()),
                    }
                }
                character if character <= '\u{1f}' => return Err("Unescaped control character".into()),
                character => result.push(character),
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let first = self.hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            if !self.source[self.cursor..].starts_with("\\u") { return Err("Invalid Unicode surrogate pair".into()); }
            self.cursor += 2;
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) { return Err("Invalid Unicode surrogate pair".into()); }
            0x10000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) { return Err("Invalid Unicode surrogate".into()); }
        else { u32::from(first) };
        char::from_u32(scalar).ok_or_else(|| "Invalid Unicode escape".into())
    }

    fn hex_quad(&mut self) -> Result<u16, String> {
        let bytes = self.source.as_bytes().get(self.cursor..self.cursor + 4).ok_or("Invalid Unicode escape")?;
        let mut value = 0u16;
        for byte in bytes {
            value = value * 16 + match byte {
                b'0'..=b'9' => u16::from(byte - b'0'), b'a'..=b'f' => u16::from(byte - b'a' + 10),
                b'A'..=b'F' => u16::from(byte - b'A' + 10), _ => return Err("Invalid Unicode escape".into()),
            };
        }
        self.cursor += 4;
        Ok(value)
    }

    fn array(&mut self, depth: usize) -> Result<Value, String> {
        if depth > self.max_depth { return Err("Maximum stack depth exceeded".into()); }
        self.cursor += 1;
        self.whitespace();
        let mut values = Vec::new();
        if self.consume(b']') { return Ok(Value::from_array_entries(values)); }
        loop {
            self.whitespace();
            if self.source.as_bytes().get(self.cursor) == Some(&b']') { return Err("Syntax error".into()); }
            values.push((ArrayKey::Integer(values.len() as i64), self.value(depth)?));
            self.whitespace();
            if self.consume(b']') { break; }
            if !self.consume(b',') {
                if self.source.as_bytes().get(self.cursor) == Some(&b'}') { return Err("State mismatch".into()); }
                return Err("Syntax error".into());
            }
        }
        Ok(Value::from_array_entries(values))
    }

    fn object(&mut self, depth: usize) -> Result<Value, String> {
        if depth > self.max_depth { return Err("Maximum stack depth exceeded".into()); }
        self.cursor += 1;
        self.whitespace();
        let mut properties = HashMap::new();
        let mut entries = Vec::new();
        if !self.consume(b'}') {
            loop {
                self.whitespace();
                if self.source.as_bytes().get(self.cursor) == Some(&b'}') { return Err("Syntax error".into()); }
                if self.source.as_bytes().get(self.cursor) == Some(&b']') { return Err("State mismatch".into()); }
                if self.source.as_bytes().get(self.cursor) != Some(&b'"') { return Err("Object keys must be strings".into()); }
                let key = self.string()?;
                if !self.associative && key.starts_with('\0') { return Err("Invalid property name".into()); }
                self.whitespace();
                if !self.consume(b':') {
                    if self.source.as_bytes().get(self.cursor) == Some(&b']') { return Err("State mismatch".into()); }
                    return Err("Expected colon".into());
                }
                let value = self.value(depth)?;
                properties.insert(key.clone(), value.clone());
                entries.push((ArrayKey::String(key), value));
                self.whitespace();
                if self.consume(b'}') { break; }
                if !self.consume(b',') {
                    if self.source.as_bytes().get(self.cursor) == Some(&b']') { return Err("State mismatch".into()); }
                    return Err("Syntax error".into());
                }
            }
        }
        if self.associative {
            let mut array = Value::array(Vec::new());
            for (key, value) in entries {
                let key = Value::String(match key { ArrayKey::Integer(index) => index.to_string(), ArrayKey::String(name) => name }).array_key()?;
                array = array.array_set(key, value)?;
            }
            Ok(array)
        } else {
            let mut property_order = Vec::new();
            for (key, _) in &entries {
                let name = match key { ArrayKey::Integer(index) => index.to_string(), ArrayKey::String(name) => name.clone() };
                if !property_order.contains(&name) { property_order.push(name); }
            }
            Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(PhpObject { class_name: "stdClass".into(), properties, property_order, active_property_hooks: Default::default(), exception: None }))))
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.cursor;
        let bytes = self.source.as_bytes();
        if bytes.get(self.cursor) == Some(&b'-') { self.cursor += 1; }
        match bytes.get(self.cursor) {
            Some(b'0') => self.cursor += 1,
            Some(b'1'..=b'9') => { self.cursor += 1; while bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) { self.cursor += 1; } }
            _ => return Err("Invalid number".into()),
        }
        if bytes.get(self.cursor) == Some(&b'.') {
            self.cursor += 1;
            let digits = self.cursor;
            while bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) { self.cursor += 1; }
            if self.cursor == digits { return Err("Invalid number".into()); }
        }
        if bytes.get(self.cursor).is_some_and(|byte| *byte == b'e' || *byte == b'E') {
            self.cursor += 1;
            if bytes.get(self.cursor).is_some_and(|byte| *byte == b'+' || *byte == b'-') { self.cursor += 1; }
            let digits = self.cursor;
            while bytes.get(self.cursor).is_some_and(u8::is_ascii_digit) { self.cursor += 1; }
            if self.cursor == digits { return Err("Invalid number".into()); }
        }
        let spelling = &self.source[start..self.cursor];
        if !spelling.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) {
            if let Ok(value) = spelling.parse::<i64>() { return Ok(Value::Integer(value)); }
            if self.bigints_as_string { return Ok(Value::String(spelling.into())); }
        }
        let value = spelling.parse::<f64>().map_err(|_| "Invalid number")?;
        if value.is_finite() { Ok(Value::Number(value)) } else { Err("Inf and NaN are not supported JSON numbers".into()) }
    }

    fn consume(&mut self, byte: u8) -> bool {
        if self.source.as_bytes().get(self.cursor) == Some(&byte) { self.cursor += 1; true } else { false }
    }
}

#[cfg(test)]
#[path = "../../../tests/crates/php-runtime/runtime/json_builtins.rs"]
mod tests;
