//! Minimal Composer manifest reader for the project's autoload rules.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
enum JsonValue {
    Object(HashMap<String, JsonValue>),
    Array(Vec<JsonValue>),
    String(String),
    Other,
}

#[derive(Debug, Default)]
pub(super) struct ComposerAutoload {
    pub(super) name: Option<String>,
    pub(super) requires: Vec<String>,
    pub(super) psr4: Vec<(String, Vec<PathBuf>)>,
    pub(super) psr0: Vec<(String, Vec<PathBuf>)>,
    pub(super) classmap: Vec<PathBuf>,
    pub(super) files: Vec<PathBuf>,
}

pub(super) fn read_manifest(path: &Path) -> Result<ComposerAutoload, String> {
    let input = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if input.len() > 4 * 1024 * 1024 {
        return Err(format!(
            "{} exceeds the 4 MiB Composer manifest limit",
            path.display()
        ));
    }
    let mut offset = 0;
    let json = parse_value(input.as_bytes(), &mut offset)?;
    whitespace(input.as_bytes(), &mut offset);
    if offset != input.len() {
        return Err(format!(
            "{} has trailing data after Composer JSON",
            path.display()
        ));
    }
    let root =
        object(&json).ok_or_else(|| format!("{} must contain a JSON object", path.display()))?;
    let autoload = root.get("autoload").and_then(object);
    let mut result = ComposerAutoload::default();
    result.name = root.get("name").and_then(string).map(str::to_string);
    if let Some(requirements) = root.get("require").and_then(object) {
        result.requires.extend(requirements.keys().cloned());
        result.requires.sort();
    }
    let base = path.parent().unwrap_or(Path::new("."));
    if let Some(psr4) = autoload
        .and_then(|value| value.get("psr-4"))
        .and_then(object)
    {
        for (prefix, locations) in psr4 {
            let values = match locations {
                JsonValue::String(value) => vec![value.as_str()],
                JsonValue::Array(values) => values.iter().filter_map(string).collect(),
                _ => continue,
            };
            result.psr4.push((
                prefix.clone(),
                values.into_iter().map(|value| base.join(value)).collect(),
            ));
        }
    }
    if let Some(psr0) = autoload
        .and_then(|value| value.get("psr-0"))
        .and_then(object)
    {
        for (prefix, locations) in psr0 {
            let values = match locations {
                JsonValue::String(value) => vec![value.as_str()],
                JsonValue::Array(values) => values.iter().filter_map(string).collect(),
                _ => continue,
            };
            result.psr0.push((
                prefix.clone(),
                values.into_iter().map(|value| base.join(value)).collect(),
            ));
        }
    }
    if let Some(classmap) = autoload
        .and_then(|value| value.get("classmap"))
        .and_then(array)
    {
        result.classmap.extend(
            classmap
                .iter()
                .filter_map(string)
                .map(|value| base.join(value)),
        );
    }
    if let Some(files) = autoload
        .and_then(|value| value.get("files"))
        .and_then(array)
    {
        result.files.extend(
            files
                .iter()
                .filter_map(string)
                .map(|value| base.join(value)),
        );
    }
    Ok(result)
}

fn object(value: &JsonValue) -> Option<&HashMap<String, JsonValue>> {
    if let JsonValue::Object(value) = value {
        Some(value)
    } else {
        None
    }
}
fn array(value: &JsonValue) -> Option<&[JsonValue]> {
    if let JsonValue::Array(value) = value {
        Some(value)
    } else {
        None
    }
}
fn string(value: &JsonValue) -> Option<&str> {
    if let JsonValue::String(value) = value {
        Some(value)
    } else {
        None
    }
}

fn parse_value(input: &[u8], offset: &mut usize) -> Result<JsonValue, String> {
    whitespace(input, offset);
    match input.get(*offset).copied() {
        Some(b'{') => {
            *offset += 1;
            let mut values = HashMap::new();
            loop {
                whitespace(input, offset);
                if input.get(*offset) == Some(&b'}') {
                    *offset += 1;
                    break;
                }
                let key = parse_string(input, offset)?;
                whitespace(input, offset);
                expect(input, offset, b':')?;
                let value = parse_value(input, offset)?;
                values.insert(key, value);
                whitespace(input, offset);
                if input.get(*offset) == Some(&b',') {
                    *offset += 1;
                } else {
                    expect(input, offset, b'}')?;
                    break;
                }
            }
            Ok(JsonValue::Object(values))
        }
        Some(b'[') => {
            *offset += 1;
            let mut values = Vec::new();
            loop {
                whitespace(input, offset);
                if input.get(*offset) == Some(&b']') {
                    *offset += 1;
                    break;
                }
                values.push(parse_value(input, offset)?);
                whitespace(input, offset);
                if input.get(*offset) == Some(&b',') {
                    *offset += 1;
                } else {
                    expect(input, offset, b']')?;
                    break;
                }
            }
            Ok(JsonValue::Array(values))
        }
        Some(b'"') => parse_string(input, offset).map(JsonValue::String),
        Some(_) => {
            let start = *offset;
            while input.get(*offset).is_some_and(|byte| {
                !byte.is_ascii_whitespace() && !matches!(byte, b',' | b']' | b'}')
            }) {
                *offset += 1;
            }
            if *offset == start {
                return Err(format!("invalid JSON at byte {}", *offset));
            }
            Ok(JsonValue::Other)
        }
        None => Err("unexpected end of Composer JSON".into()),
    }
}

fn parse_string(input: &[u8], offset: &mut usize) -> Result<String, String> {
    expect(input, offset, b'"')?;
    let mut output = String::new();
    while let Some(byte) = input.get(*offset).copied() {
        *offset += 1;
        match byte {
            b'"' => return Ok(output),
            b'\\' => {
                let escaped = *input.get(*offset).ok_or("incomplete JSON escape")?;
                *offset += 1;
                match escaped {
                    b'"' => output.push('"'),
                    b'\\' => output.push('\\'),
                    b'/' => output.push('/'),
                    b'b' => output.push('\u{0008}'),
                    b'f' => output.push('\u{000c}'),
                    b'n' => output.push('\n'),
                    b'r' => output.push('\r'),
                    b't' => output.push('\t'),
                    b'u' => {
                        let end = offset.checked_add(4).ok_or("invalid Unicode escape")?;
                        let digits = input.get(*offset..end).ok_or("incomplete Unicode escape")?;
                        let text =
                            std::str::from_utf8(digits).map_err(|_| "invalid Unicode escape")?;
                        let scalar =
                            u32::from_str_radix(text, 16).map_err(|_| "invalid Unicode escape")?;
                        output.push(
                            char::from_u32(scalar)
                                .ok_or("unsupported surrogate escape in Composer path")?,
                        );
                        *offset = end;
                    }
                    _ => return Err("invalid JSON string escape".into()),
                }
            }
            byte if byte.is_ascii() => output.push(byte as char),
            _ => {
                let start = *offset - 1;
                let text = std::str::from_utf8(&input[start..])
                    .map_err(|_| "invalid UTF-8 in Composer JSON")?;
                let character = text
                    .chars()
                    .next()
                    .ok_or("invalid UTF-8 in Composer JSON")?;
                output.push(character);
                *offset = start + character.len_utf8();
            }
        }
    }
    Err("unterminated JSON string".into())
}

fn whitespace(input: &[u8], offset: &mut usize) {
    while input.get(*offset).is_some_and(u8::is_ascii_whitespace) {
        *offset += 1;
    }
}
fn expect(input: &[u8], offset: &mut usize, expected: u8) -> Result<(), String> {
    if input.get(*offset) != Some(&expected) {
        return Err(format!("invalid Composer JSON at byte {}", *offset));
    }
    *offset += 1;
    Ok(())
}
