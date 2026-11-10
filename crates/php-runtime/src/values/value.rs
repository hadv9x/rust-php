//! PHP runtime values and common conversions.

use std::{
    cell::RefCell,
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{ArrayKey, BytecodeFunction, PhpArray, PhpGenerator, PhpObject};

static NEXT_RESOURCE_ID: AtomicU64 = AtomicU64::new(1);

/// Runtime values shared by the compiler, VM, and builtin libraries.
#[derive(Clone, Debug)]
/// A PHP runtime value supported by this interpreter.
pub enum Value {
    Integer(i64),
    Number(f64),
    String(String),
    Bool(bool),
    Array(PhpArray),
    Object(Rc<RefCell<PhpObject>>),
    Closure(Rc<RefCell<PhpClosure>>),
    Generator(Rc<RefCell<PhpGenerator>>),
    /// File-backed temporary stream resource.
    Stream(Rc<RefCell<PhpStream>>),
    /// Shared storage cell retained by PHP references, including references inside arrays.
    Reference(Rc<RefCell<Value>>),
    Null,
}

#[derive(Debug)]
pub struct PhpStream {
    id: u64,
    file: Option<File>,
    directory: Option<VecDeque<String>>,
    directory_snapshot: Option<Vec<String>>,
    path: PathBuf,
    readable: bool,
    writable: bool,
    delete_on_close: bool,
}

impl PhpStream {
    pub fn temporary(readable: bool, writable: bool) -> io::Result<Self> {
        static NEXT_STREAM: AtomicU64 = AtomicU64::new(1);
        let directory = std::env::temp_dir();
        loop {
            let id = NEXT_STREAM.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("php-rust-stream-{}-{id}.tmp", std::process::id()));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        id: NEXT_RESOURCE_ID.fetch_add(1, Ordering::Relaxed),
                        file: Some(file),
                        directory: None,
                        directory_snapshot: None,
                        path,
                        readable,
                        writable,
                        delete_on_close: true,
                    })
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
    pub fn open_file(path: PathBuf, mode: &str) -> io::Result<Self> {
        let mut characters = mode.chars();
        let first = characters
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid stream mode"))?;
        if !matches!(first, 'r' | 'w' | 'a' | 'x' | 'c')
            || characters
                .clone()
                .any(|character| !matches!(character, '+' | 'b' | 't'))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid stream mode",
            ));
        }
        let plus = mode.contains('+');
        let mut options = OpenOptions::new();
        options
            .read(first == 'r' || plus)
            .write(first != 'r' || plus);
        match first {
            'w' => {
                options.create(true).truncate(true);
            }
            'a' => {
                options.create(true).append(true);
            }
            'x' => {
                options.create_new(true);
            }
            'c' => {
                options.create(true);
            }
            _ => {}
        }
        let file = options.open(&path)?;
        Ok(Self {
            id: NEXT_RESOURCE_ID.fetch_add(1, Ordering::Relaxed),
            file: Some(file),
            directory: None,
            directory_snapshot: None,
            path,
            readable: first == 'r' || plus,
            writable: first != 'r' || plus,
            delete_on_close: false,
        })
    }
    pub fn directory(path: PathBuf, entries: Vec<String>) -> Self {
        Self {
            id: NEXT_RESOURCE_ID.fetch_add(1, Ordering::Relaxed),
            file: None,
            directory: Some(entries.clone().into()),
            directory_snapshot: Some(entries),
            path,
            readable: true,
            writable: false,
            delete_on_close: false,
        }
    }
    pub fn read_directory(&mut self) -> io::Result<Option<String>> {
        self.directory
            .as_mut()
            .map(|entries| entries.pop_front())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "resource is not an open directory",
                )
            })
    }
    pub fn rewind_directory(&mut self, entries: Vec<String>) -> io::Result<()> {
        if self.directory_snapshot.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "resource is not an open directory",
            ));
        }
        self.directory_snapshot = Some(entries.clone());
        self.directory = Some(entries.into());
        Ok(())
    }
    pub fn is_directory(&self) -> bool {
        self.directory.is_some()
    }
    pub fn resource_id(&self) -> u64 {
        self.id
    }
    pub fn directory_path(&self) -> Option<&std::path::Path> {
        self.is_directory().then_some(self.path.as_path())
    }
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.writable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "stream is not writable",
            ));
        }
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stream is closed"))?
            .write(bytes)
    }
    pub fn read(&mut self, length: Option<usize>) -> io::Result<Vec<u8>> {
        if !self.readable {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "stream is not readable",
            ));
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stream is closed"))?;
        match length {
            Some(length) => {
                let mut bytes = vec![0; length];
                let read = file.read(&mut bytes)?;
                bytes.truncate(read);
                Ok(bytes)
            }
            None => {
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)?;
                Ok(bytes)
            }
        }
    }
    pub fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "stream is closed"))?
            .seek(position)
    }
    pub fn is_open(&self) -> bool {
        self.file.is_some() || self.directory.is_some()
    }
    pub fn close(&mut self) -> io::Result<()> {
        self.file.take();
        self.directory.take();
        self.directory_snapshot.take();
        if !self.delete_on_close {
            return Ok(());
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for PhpStream {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Anonymous function bytecode and its lexical environment.
#[derive(Clone, Debug)]
pub struct PhpClosure {
    /// Compiled function body and parameter contract.
    pub function: BytecodeFunction,
    /// Variables copied when the closure expression is evaluated.
    pub captures: std::collections::HashMap<String, Value>,
    /// Capture names in source order, including captures stored by reference.
    pub capture_order: Vec<String>,
    /// Storage slots shared with variables captured by reference.
    pub reference_captures: std::collections::HashMap<String, String>,
    /// Callable value targeted by PHP first-class callable syntax, if this is a wrapper closure.
    pub callable_target: Option<Value>,
    /// Access scope carried by ReflectionMethod-bound closures.
    pub callable_access_scope: Option<String>,
}

impl Value {
    /// Materialize a PHP 8.1 `new Class()` parameter default as a fresh object
    /// for each call. Other object-valued defaults (such as enum cases) remain
    /// shared values.
    pub(crate) fn materialize_parameter_default(&self) -> Self {
        const MARKER: &str = "\0php-rust-parameter-default-object";
        let Self::Object(object) = self else {
            return self.clone();
        };
        let mut value = object.borrow().clone();
        if value.properties.remove(MARKER).is_some() {
            Self::Object(Rc::new(RefCell::new(value)))
        } else {
            self.clone()
        }
    }

    /// Return the PHP-facing type name for this value.
    pub(crate) fn type_name(&self) -> &'static str {
        match self.dereference() {
            Self::Integer(_) => "int",
            Self::Number(_) => "float",
            Self::String(_) => "string",
            Self::Bool(_) => "bool",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
            Self::Closure(_) | Self::Generator(_) => "object",
            Self::Stream(_) => "resource",
            Self::Null => "null",
            Self::Reference(_) => unreachable!("dereference resolves reference cells"),
        }
    }

    /// Read through reference cells until a non-reference value is reached.
    pub(crate) fn dereference(&self) -> Self {
        match self {
            Self::Reference(value) => value.borrow().dereference(),
            value => value.clone(),
        }
    }

    /// Construct a PHP array from ordered key/value entries for embedding APIs.
    pub fn from_array_entries(entries: Vec<(ArrayKey, Value)>) -> Self {
        Self::Array(PhpArray::from_entries(entries))
    }

    /// Construct an ordered array and initialize its next-free integer index.
    pub(crate) fn array(entries: Vec<(ArrayKey, Value)>) -> Self {
        Self::Array(PhpArray::from_entries(entries))
    }

    /// Convert a runtime value using PHP's echo-oriented display behavior for this subset.
    pub(crate) fn show(&self) -> String {
        match self.dereference() {
            Self::Integer(n) => n.to_string(),
            Self::Number(n) if n.fract() == 0.0 => format!("{n:.0}"),
            Self::Number(n) => n.to_string(),
            Self::String(s) => s.clone(),
            Self::Array(_) => "Array".into(),
            Self::Object(object) => format!("Object({})", object.borrow().class_name),
            Self::Closure(_) => "Closure Object".into(),
            Self::Generator(_) => "Generator Object".into(),
            Self::Stream(_) => "Resource id #stream".into(),
            Self::Bool(true) => "1".into(),
            Self::Bool(false) | Self::Null => String::new(),
            Self::Reference(_) => unreachable!("dereference removes reference wrappers"),
        }
    }

    /// Convert supported scalar values to the numeric form used by arithmetic.
    pub(crate) fn number(&self) -> f64 {
        match self.dereference() {
            Self::Integer(n) => n as f64,
            Self::Number(n) => n,
            Self::Bool(true) => 1.0,
            Self::Bool(false) | Self::Null => 0.0,
            Self::String(s) => s.trim().parse().unwrap_or(0.0),
            Self::Array(_) => 0.0,
            Self::Object(_) | Self::Closure(_) | Self::Generator(_) | Self::Stream(_) => 1.0,
            Self::Reference(_) => unreachable!("dereference removes reference wrappers"),
        }
    }

    /// Apply an integer increment while retaining the PHP integer type when it fits.
    pub(crate) fn incremented(&self, delta: i8) -> Self {
        match self.dereference() {
            Self::Integer(value) => value
                .checked_add(i64::from(delta))
                .map(Self::Integer)
                .unwrap_or_else(|| Self::Number(value as f64 + f64::from(delta))),
            Self::String(value) if delta > 0 => {
                let numeric = value.trim();
                if let Ok(number) = numeric.parse::<i64>() {
                    return number
                        .checked_add(i64::from(delta))
                        .map(Self::Integer)
                        .unwrap_or_else(|| Self::Number(number as f64 + f64::from(delta)));
                }
                if let Ok(number) = numeric.parse::<f64>() {
                    return Self::Number(number + f64::from(delta));
                }
                if value.is_empty() {
                    return Self::String("1".into());
                }
                if is_ascii_alphanumeric(&value) {
                    return Self::String(increment_alphanumeric(&value));
                }
                Self::String(value)
            }
            Self::String(value) => {
                if value.is_empty() {
                    return Self::Integer(-1);
                }
                let numeric = value.trim();
                if let Ok(number) = numeric.parse::<i64>() {
                    return number
                        .checked_add(i64::from(delta))
                        .map(Self::Integer)
                        .unwrap_or_else(|| Self::Number(number as f64 + f64::from(delta)));
                }
                if let Ok(number) = numeric.parse::<f64>() {
                    return Self::Number(number + f64::from(delta));
                }
                Self::String(value)
            }
            Self::Bool(_) | Self::Null => self.dereference(),
            value => Self::Number(value.number() + f64::from(delta)),
        }
    }

    /// Negate a scalar while preserving integer type except at the signed overflow boundary.
    pub(crate) fn negated(&self) -> Self {
        match self.dereference() {
            Self::Integer(value) => value
                .checked_neg()
                .map(Self::Integer)
                .unwrap_or_else(|| Self::Number(-(value as f64))),
            value => Self::Number(-value.number()),
        }
    }

    /// Apply PHP-style truthiness for the supported runtime values.
    pub(crate) fn truthy(&self) -> bool {
        match self.dereference() {
            Self::Null | Self::Bool(false) => false,
            Self::Integer(n) => n != 0,
            Self::Number(n) => n != 0.0,
            Self::String(s) => !s.is_empty() && s != "0",
            Self::Array(values) => !values.is_empty(),
            Self::Object(_) | Self::Closure(_) | Self::Generator(_) | Self::Stream(_) => true,
            _ => true,
        }
    }

    /// Coerce a value to an integer/string array key using the supported PHP rules.
    pub(crate) fn array_key(&self) -> Result<ArrayKey, String> {
        match self.dereference() {
            Self::Integer(number) => Ok(ArrayKey::Integer(number)),
            Self::Number(number) => Ok(ArrayKey::Integer(number as i64)),
            Self::Bool(value) => Ok(ArrayKey::Integer(i64::from(value))),
            Self::Null => Ok(ArrayKey::String(String::new())),
            Self::String(value) => {
                if let Ok(integer) = value.parse::<i64>() {
                    if integer.to_string() == value {
                        return Ok(ArrayKey::Integer(integer));
                    }
                }
                Ok(ArrayKey::String(value.clone()))
            }
            Self::Array(_)
            | Self::Object(_)
            | Self::Closure(_)
            | Self::Generator(_)
            | Self::Stream(_) => Err("Illegal offset type".into()),
            Self::Reference(_) => unreachable!("dereference removes reference wrappers"),
        }
    }

    /// Read an array offset; absent keys and null-array reads evaluate to null in this VM.
    pub(crate) fn array_get(&self, key: &ArrayKey) -> Result<Value, String> {
        match self.dereference() {
            Self::Array(array) => Ok(array
                .entries
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value.dereference())
                .unwrap_or(Self::Null)),
            Self::Null => Ok(Self::Null),
            Self::String(value) => {
                let ArrayKey::Integer(index) = key else {
                    return Err("Illegal string offset type".into());
                };
                let index = string_offset_index(*index, value.len())?;
                let byte = *value
                    .as_bytes()
                    .get(index)
                    .ok_or("Uninitialized string offset")?;
                Ok(Self::String(char::from(byte).to_string()))
            }
            _ => Err("Cannot access an array offset on a non-array value".into()),
        }
    }

    /// Read a nested array element through a prepared key path.
    pub(crate) fn array_get_path(&self, keys: &[ArrayKey]) -> Result<Value, String> {
        let Some((key, remaining)) = keys.split_first() else {
            return Ok(self.dereference());
        };
        self.array_get(key)?.array_get_path(remaining)
    }

    pub(crate) fn first_missing_array_path_key(&self, keys: &[ArrayKey]) -> Option<ArrayKey> {
        let mut current = self.dereference();
        for key in keys {
            let Self::Array(array) = current else {
                return None;
            };
            let Some((_, value)) = array.entries.iter().find(|(candidate, _)| candidate == key)
            else {
                return Some(key.clone());
            };
            current = value.dereference();
        }
        None
    }

    /// Replace the selected array slot with a shared reference cell and return the updated root.
    pub(crate) fn bind_array_reference(
        self,
        keys: &[ArrayKey],
        cell: Rc<RefCell<Value>>,
    ) -> Result<Self, String> {
        let Some((key, remaining)) = keys.split_first() else {
            return Err("Cannot bind a reference to an empty array path".into());
        };
        match self.dereference() {
            Self::Array(mut array) if remaining.is_empty() => {
                array.set_reference(key.clone(), cell);
                Ok(Self::Array(array))
            }
            Self::Array(mut array) => {
                let child = array
                    .iter()
                    .find(|(candidate, _)| candidate == key)
                    .map(|(_, value)| value.dereference())
                    .unwrap_or(Self::Null);
                let updated_child = child.bind_array_reference(remaining, cell)?;
                array.set(key.clone(), updated_child);
                Ok(Self::Array(array))
            }
            Self::Null => {
                let mut array = PhpArray::new();
                if remaining.is_empty() {
                    array.set_reference(key.clone(), cell);
                } else {
                    let child = Self::Null.bind_array_reference(remaining, cell)?;
                    array.set(key.clone(), child);
                }
                Ok(Self::Array(array))
            }
            _ => Err("Cannot bind an array reference through a non-array value".into()),
        }
    }

    /// Reuse an existing reference at a path or turn its current value into a reference cell.
    pub(crate) fn ensure_array_reference(
        self,
        keys: &[ArrayKey],
    ) -> Result<(Self, Rc<RefCell<Value>>), String> {
        let Some((key, remaining)) = keys.split_first() else {
            return Err("Cannot reference an empty array path".into());
        };
        match self.dereference() {
            Self::Array(mut array) if remaining.is_empty() => {
                let existing_reference = array.iter().find_map(|(candidate, value)| {
                    if candidate != key {
                        return None;
                    }
                    if let Self::Reference(reference) = value {
                        Some(reference.clone())
                    } else {
                        None
                    }
                });
                if let Some(reference) = existing_reference {
                    return Ok((Self::Array(array), reference));
                }
                let value = array
                    .iter()
                    .find(|(candidate, _)| candidate == key)
                    .map(|(_, value)| value.dereference())
                    .unwrap_or(Self::Null);
                let reference = Rc::new(RefCell::new(value));
                array.set_reference(key.clone(), reference.clone());
                Ok((Self::Array(array), reference))
            }
            Self::Array(mut array) => {
                let child = array
                    .iter()
                    .find(|(candidate, _)| candidate == key)
                    .map(|(_, value)| value.dereference())
                    .unwrap_or(Self::Null);
                let (updated_child, reference) = child.ensure_array_reference(remaining)?;
                array.set(key.clone(), updated_child);
                Ok((Self::Array(array), reference))
            }
            Self::Null => {
                let mut array = PhpArray::new();
                if remaining.is_empty() {
                    let reference = Rc::new(RefCell::new(Self::Null));
                    array.set_reference(key.clone(), reference.clone());
                    Ok((Self::Array(array), reference))
                } else {
                    let (child, reference) = Self::Null.ensure_array_reference(remaining)?;
                    array.set(key.clone(), child);
                    Ok((Self::Array(array), reference))
                }
            }
            _ => Err("Cannot take an array element reference through a non-array value".into()),
        }
    }

    /// Return a value with one array entry replaced or added.
    pub(crate) fn array_set(self, key: ArrayKey, value: Value) -> Result<Value, String> {
        match self.dereference() {
            Self::Array(mut array) => {
                array.set(key, value);
                Ok(Self::Array(array))
            }
            Self::Null => {
                let mut array = PhpArray::new();
                array.set(key, value);
                Ok(Self::Array(array))
            }
            Self::String(mut string) => {
                let ArrayKey::Integer(index) = key else {
                    return Err("Illegal string offset type".into());
                };
                let index = string_offset_index(index, string.len())?;
                if !string.is_char_boundary(index) || !string.is_char_boundary(index + 1) {
                    return Err("Cannot assign through a non-ASCII string offset".into());
                }
                let replacement = value.show();
                let byte = replacement.as_bytes().first().copied().unwrap_or(0);
                // The runtime currently stores strings as UTF-8; byte offsets below 0x80
                // can be updated exactly, while larger bytes cannot be represented alone.
                if byte >= 0x80 {
                    return Err("Cannot assign a non-UTF-8 byte string offset".into());
                }
                string.replace_range(index..index + 1, &(byte as char).to_string());
                Ok(Self::String(string))
            }
            _ => Err("Cannot assign an array offset on a non-array value".into()),
        }
    }

    /// Update a nested array path by rebuilding the modified value chain.
    pub(crate) fn array_set_path(self, keys: &[ArrayKey], value: Value) -> Result<Value, String> {
        let Some((key, remaining)) = keys.split_first() else {
            return Err("Cannot assign to an empty array path".into());
        };
        if remaining.is_empty() {
            return self.array_set(key.clone(), value);
        }
        let child = self.array_get(key)?;
        let updated_child = child.array_set_path(remaining, value)?;
        self.array_set(key.clone(), updated_child)
    }

    /// Assign through an array path where `None` means append at that level.
    pub(crate) fn array_set_mixed_path(
        self,
        path: &[Option<ArrayKey>],
        value: Value,
    ) -> Result<Value, String> {
        let Some((segment, remaining)) = path.split_first() else {
            return Err("Cannot assign to an empty array path".into());
        };
        match segment {
            Some(key) if remaining.is_empty() => self.array_set(key.clone(), value),
            Some(key) => {
                let child = self.array_get(key)?;
                let updated = child.array_set_mixed_path(remaining, value)?;
                self.array_set(key.clone(), updated)
            }
            None if remaining.is_empty() => self.array_append(value),
            None => {
                let appended = self.array_append(Value::Null)?;
                let Value::Array(array) = appended.dereference() else {
                    return Err("Cannot append to a non-array value".into());
                };
                let key = array
                    .entries
                    .last()
                    .map(|(key, _)| key.clone())
                    .ok_or("Cannot determine appended array key")?;
                let updated = Value::Null.array_set_mixed_path(remaining, value)?;
                appended.array_set(key, updated)
            }
        }
    }

    /// Remove a nested array entry and return its former value for lifetime handling.
    pub(crate) fn array_unset_path_with_removed(
        self,
        keys: &[ArrayKey],
    ) -> Result<(Value, Option<Value>), String> {
        let Some((key, remaining)) = keys.split_first() else {
            return Ok((self, None));
        };
        match self.dereference() {
            Self::Array(mut array) if remaining.is_empty() => {
                let removed = array
                    .entries
                    .iter()
                    .find(|(candidate, _)| candidate == key)
                    .map(|(_, value)| value.dereference());
                array.unset(key);
                Ok((Self::Array(array), removed))
            }
            Self::Array(mut array) => {
                if let Some((_, child)) =
                    array.entries.iter().find(|(candidate, _)| candidate == key)
                {
                    let (updated_child, removed) =
                        child.clone().array_unset_path_with_removed(remaining)?;
                    array.set(key.clone(), updated_child);
                    Ok((Self::Array(array), removed))
                } else {
                    Ok((Self::Array(array), None))
                }
            }
            other => Ok((other, None)),
        }
    }

    /// Append to an array nested under the supplied key path.
    pub(crate) fn array_append_path(
        self,
        keys: &[ArrayKey],
        value: Value,
    ) -> Result<Value, String> {
        let Some((key, remaining)) = keys.split_first() else {
            return self.array_append(value);
        };
        let child = self.array_get(key)?;
        let updated_child = child.array_append_path(remaining, value)?;
        self.array_set(key.clone(), updated_child)
    }

    /// Append one item to an array value, promoting null to an empty array.
    pub(crate) fn array_append(self, value: Value) -> Result<Value, String> {
        match self.dereference() {
            Self::Array(mut array) => {
                array.append(value)?;
                Ok(Self::Array(array))
            }
            Self::Null => {
                let mut array = PhpArray::new();
                array.append(value)?;
                Ok(Self::Array(array))
            }
            _ => Err("Cannot append to a non-array value".into()),
        }
    }

    pub(crate) fn array_append_reference(
        self,
        reference: Rc<RefCell<Value>>,
    ) -> Result<Value, String> {
        match self.dereference() {
            Self::Array(mut array) => {
                array.append_reference(reference)?;
                Ok(Self::Array(array))
            }
            Self::Null => {
                let mut array = PhpArray::new();
                array.append_reference(reference)?;
                Ok(Self::Array(array))
            }
            _ => Err("Cannot append to a non-array value".into()),
        }
    }
}

fn string_offset_index(index: i64, length: usize) -> Result<usize, String> {
    let resolved = if index < 0 {
        length as i64 + index
    } else {
        index
    };
    if resolved < 0 || resolved >= length as i64 {
        return Err("Uninitialized string offset".into());
    }
    Ok(resolved as usize)
}

fn is_ascii_alphanumeric(value: &str) -> bool {
    value.is_ascii() && !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub(crate) fn increment_alphanumeric(value: &str) -> String {
    debug_assert!(is_ascii_alphanumeric(value));
    let mut bytes = value.as_bytes().to_vec();
    let mut carry = true;
    for byte in bytes.iter_mut().rev() {
        if !carry {
            break;
        }
        match *byte {
            b'0'..=b'8' | b'a'..=b'y' | b'A'..=b'Y' => {
                *byte += 1;
                carry = false;
            }
            b'9' => *byte = b'0',
            b'z' => *byte = b'a',
            b'Z' => *byte = b'A',
            _ => unreachable!("validated ASCII alphanumeric string"),
        }
    }
    if carry {
        let first = match bytes.first().copied() {
            Some(b'a'..=b'z') => b'a',
            Some(b'A'..=b'Z') => b'A',
            _ => b'1',
        };
        bytes.insert(0, first);
    }
    String::from_utf8(bytes).expect("ASCII string remains valid UTF-8")
}
