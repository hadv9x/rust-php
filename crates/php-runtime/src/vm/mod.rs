//! Shared virtual-machine execution loop and runtime type checks.

use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, rc::Rc, time::Instant};

mod functions;
mod builtin_arguments;
mod objects;
mod operators;
mod arrays;
mod control;
mod symbol_table;

pub(crate) fn intval_string(input: &str, base: u32) -> i64 {
    functions::intval_string(input, base)
}

use operators::{apply_binary, apply_bitwise_not, apply_cast};
use crate::{bytecode::{CompoundOperation, Op}, ArrayKey, BytecodeClass, BytecodeFunction, ExecutionLimits, SessionValue, Value};

thread_local! {
    static EXECUTION_DEPTH: Cell<usize> = Cell::new(0);
    static EXECUTION_LIMITS: Cell<ExecutionLimits> = Cell::new(ExecutionLimits {
        max_instructions: 10_000_000,
        max_output_bytes: 8 * 1024 * 1024,
        max_execution_time: Some(std::time::Duration::from_secs(30)),
    });
    static EXECUTION_DEADLINE: Cell<Option<Instant>> = Cell::new(None);
}

const MAX_EXECUTION_DEPTH: usize = 32;

struct ExecutionDepthGuard;

impl ExecutionDepthGuard {
    fn enter() -> Result<Self, String> {
        EXECUTION_DEPTH.with(|depth| {
            let current = depth.get();
            if current >= MAX_EXECUTION_DEPTH { return Err("Maximum call depth exceeded".into()); }
            depth.set(current + 1);
            Ok(Self)
        })
    }
}

impl Drop for ExecutionDepthGuard {
    fn drop(&mut self) { EXECUTION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1))); }
}

struct ExecutionLimitGuard { limits: ExecutionLimits, deadline: Option<Instant> }

impl ExecutionLimitGuard {
    fn enter(limits: ExecutionLimits) -> Self {
        let previous = EXECUTION_LIMITS.with(|current| current.replace(ExecutionLimits {
            max_instructions: limits.max_instructions.max(1),
            max_output_bytes: limits.max_output_bytes.max(1),
            max_execution_time: limits.max_execution_time,
        }));
        let deadline = limits.max_execution_time.and_then(|duration| Instant::now().checked_add(duration));
        let previous_deadline = EXECUTION_DEADLINE.with(|current| current.replace(deadline));
        Self { limits: previous, deadline: previous_deadline }
    }
}

impl Drop for ExecutionLimitGuard {
    fn drop(&mut self) {
        EXECUTION_LIMITS.with(|current| current.set(self.limits));
        EXECUTION_DEADLINE.with(|current| current.set(self.deadline));
    }
}

/// Static property storage shared by function calls during one program execution.
pub(super) type StaticProperties = HashMap<(String, String), Value>;
#[derive(Clone, Debug)]
pub(super) enum VariableBinding {
    Global(String),
    Static(String),
    Reference(String),
    ArrayElement { storage_key: String, keys: Vec<crate::ArrayKey> },
    ObjectProperty { receiver: Value, property: String },
}

#[derive(Default)]
pub(super) struct GlobalVariables {
    pub(super) source_file: String,
    values: HashMap<String, Value>,
    aliases: HashMap<String, String>,
    /// Visible symbol order for PHP's `$GLOBALS` and global `get_defined_vars()`.
    order: Vec<String>,
    constants: HashMap<String, Value>,
    thrown_values: HashMap<u64, Value>,
    http_status: Option<u16>,
    http_headers: Vec<(String, String)>,
    session_started: bool,
    session_destroyed: bool,
    next_reference_id: u64,
    json_last_error: i64,
    diagnostics: Vec<crate::RuntimeDiagnostic>,
    last_error: Option<crate::RuntimeDiagnostic>,
    error_reporting: Option<i64>,
    destructor_queue: Vec<Value>,
    destructed_objects: HashSet<usize>,
    /// Active PHP source frames, updated by compiled statement line markers.
    pub(super) active_trace_frames: Vec<crate::PhpTraceFrame>,
}

impl GlobalVariables {
    fn push_trace_frame(&mut self, frame_scope: &str, has_this: bool, classes: &HashMap<String, BytecodeClass>) {
        let (class, function) = frame_scope.split_once("::")
            .map(|(class, method)| {
                let metadata = classes.get(&class.to_ascii_lowercase());
                let display_class = metadata.map_or_else(|| class.to_string(), |class| class.name.clone());
                let display_method = metadata
                    .and_then(|class| class.method_display_names.get(&method.to_ascii_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| method.to_string());
                (Some(display_class), display_method)
            })
            .unwrap_or((None, if frame_scope == "<main>" { "{main}".into() } else { frame_scope.to_string() }));
        self.active_trace_frames.push(crate::PhpTraceFrame {
            file: self.source_file.clone(), line: 1, function,
            call_type: class.as_ref().map(|_| if has_this { "->".into() } else { "::".into() }),
            class,
        });
    }

    fn set_trace_line(&mut self, line: usize) {
        if let Some(frame) = self.active_trace_frames.last_mut() { frame.line = line; }
    }

    fn set_trace_file(&mut self, file: &str) {
        if let Some(frame) = self.active_trace_frames.last_mut() { frame.file = file.to_string(); }
    }

    pub(super) fn report_diagnostic(&mut self, mut diagnostic: crate::RuntimeDiagnostic) {
        let mask = self.error_reporting.unwrap_or(32767);
        let bit = match diagnostic.level {
            crate::DiagnosticLevel::Warning => 2,
            crate::DiagnosticLevel::Deprecated => 8192,
        };
        if let Some(frame) = self.active_trace_frames.last() {
            if diagnostic.source_file.is_none() && !frame.file.is_empty() { diagnostic.source_file = Some(frame.file.clone()); }
            if diagnostic.source_line.is_none() { diagnostic.source_line = Some(frame.line); }
        }
        self.last_error = Some(diagnostic.clone());
        if mask & bit != 0 {
            self.diagnostics.push(diagnostic);
        }
    }

    pub(super) fn register_destructor_object(&mut self, object: &Value) {
        if matches!(object.dereference(), Value::Object(_)) {
            self.destructor_queue.push(object.clone());
        }
    }

    fn mark_destructed(&mut self, object: &Rc<RefCell<crate::PhpObject>>) -> bool {
        self.destructed_objects.insert(Rc::as_ptr(object) as usize)
    }
}

pub(super) fn increment_value(value: &Value, delta: i8, globals: &mut GlobalVariables) -> Result<Value, String> {
    let original = value.dereference();
    let diagnostic = match (&original, delta) {
        (Value::Bool(_), 1) => Some((crate::DiagnosticLevel::Warning, "Increment on type bool has no effect, this will change in the next major version of PHP")),
        (Value::Bool(_), -1) => Some((crate::DiagnosticLevel::Warning, "Decrement on type bool has no effect, this will change in the next major version of PHP")),
        (Value::Null, -1) => Some((crate::DiagnosticLevel::Warning, "Decrement on type null has no effect, this will change in the next major version of PHP")),
        (Value::String(text), 1) if text.trim().parse::<f64>().is_err() => Some((crate::DiagnosticLevel::Deprecated, "Increment on non-numeric string is deprecated, use str_increment() instead")),
        (Value::String(text), -1) if text.is_empty() => Some((crate::DiagnosticLevel::Deprecated, "Decrement on empty string is deprecated as non-numeric")),
        (Value::String(text), -1) if text.trim().parse::<f64>().is_err() => Some((crate::DiagnosticLevel::Deprecated, "Decrement on non-numeric string has no effect and is deprecated")),
        _ => None,
    };
    if let Some((level, message)) = diagnostic {
        globals.report_diagnostic(crate::RuntimeDiagnostic { level, message: message.into(), source_file: None, source_line: None });
    }
    match &original {
        Value::Array(_) | Value::Stream(_) => return Err(format!("TypeError:Cannot {} {}", if delta > 0 { "increment" } else { "decrement" }, original.type_name())),
        Value::Object(object) => return Err(format!("TypeError:Cannot {} object of class {}", if delta > 0 { "increment" } else { "decrement" }, object.borrow().class_name)),
        Value::Closure(_) => return Err(format!("TypeError:Cannot {} Closure", if delta > 0 { "increment" } else { "decrement" })),
        Value::Generator(_) => return Err(format!("TypeError:Cannot {} Generator", if delta > 0 { "increment" } else { "decrement" })),
        _ => {}
    }
    Ok(if matches!(original, Value::Null) && delta > 0 { Value::Integer(1) }
    else { original.incremented(delta) })
}

const LOCAL_ORDER_KEY: &str = "\0local-symbol-order";

/// Seed a frame's visible-name order before its argument values enter a HashMap.
pub(super) fn initialize_local_order(variables: &mut HashMap<String, Value>, names: impl IntoIterator<Item = String>) {
    let entries = names.into_iter().enumerate().map(|(index, name)| (crate::ArrayKey::Integer(index as i64), Value::String(name))).collect::<Vec<_>>();
    variables.insert(LOCAL_ORDER_KEY.into(), Value::array(entries));
}

fn update_local_order(variables: &mut HashMap<String, Value>, name: &str, present: bool) {
    if name.starts_with('\0') { return; }
    let mut names = variables.get(LOCAL_ORDER_KEY).map(|value| match value.dereference() {
        Value::Array(values) => values.iter().filter_map(|(_, value)| match value.dereference() {
            Value::String(name) => Some(name), _ => None,
        }).collect::<Vec<_>>(),
        _ => Vec::new(),
    }).unwrap_or_default();
    names.retain(|candidate| candidate != name);
    if present { names.push(name.to_string()); }
    variables.insert(LOCAL_ORDER_KEY.into(), Value::array(names.into_iter()
        .enumerate().map(|(index, name)| (crate::ArrayKey::Integer(index as i64), Value::String(name))).collect()));
}

fn is_superglobal(name: &str) -> bool {
    matches!(name, "_GET" | "_POST" | "_COOKIE" | "_REQUEST" | "_FILES" | "_SERVER" | "_ENV" | "_SESSION")
}

pub(super) fn builtin_constant(name: &str) -> Option<Value> {
    match name {
        "PHP_EOL" => return Some(Value::String(if cfg!(windows) { "\r\n" } else { "\n" }.into())),
        "PHP_OS" => return Some(Value::String(std::env::consts::OS.to_string())),
        "PHP_OS_FAMILY" => return Some(Value::String(std::env::consts::FAMILY.to_string())),
        "PHP_INT_MAX" => return Some(Value::Integer(i64::MAX)),
        "PHP_INT_MIN" => return Some(Value::Integer(i64::MIN)),
        "PHP_INT_SIZE" => return Some(Value::Integer(std::mem::size_of::<i64>() as i64)),
        "PHP_QUERY_RFC1738" => return Some(Value::Integer(1)),
        "PHP_QUERY_RFC3986" => return Some(Value::Integer(2)),
        "PHP_FLOAT_MAX" => return Some(Value::Number(f64::MAX)),
        "PHP_FLOAT_MIN" => return Some(Value::Number(f64::MIN_POSITIVE)),
        "PHP_FLOAT_EPSILON" => return Some(Value::Number(f64::EPSILON)),
        "INF" | "INFINITY" => return Some(Value::Number(f64::INFINITY)),
        "NAN" => return Some(Value::Number(f64::NAN)),
        "PHP_FLOAT_DIG" => return Some(Value::Integer(15)),
        "E_WARNING" => return Some(Value::Integer(2)),
        "E_DEPRECATED" => return Some(Value::Integer(8192)),
        "E_USER_WARNING" => return Some(Value::Integer(512)),
        "E_USER_DEPRECATED" => return Some(Value::Integer(16384)),
        "E_ALL" => return Some(Value::Integer(32767)),
        "PHP_ROUND_HALF_UP" => return Some(Value::Integer(1)),
        "PHP_ROUND_HALF_DOWN" => return Some(Value::Integer(2)),
        "PHP_ROUND_HALF_EVEN" => return Some(Value::Integer(3)),
        "PHP_ROUND_HALF_ODD" => return Some(Value::Integer(4)),
        "PHP_ROUND_CEILING" => return Some(Value::Integer(5)),
        "PHP_ROUND_FLOOR" => return Some(Value::Integer(6)),
        "PHP_ROUND_TOWARD_ZERO" => return Some(Value::Integer(7)),
        "PHP_ROUND_AWAY_FROM_ZERO" => return Some(Value::Integer(8)),
        "GRAPHEME_EXTR_COUNT" => return Some(Value::Integer(0)),
        "GRAPHEME_EXTR_MAXBYTES" => return Some(Value::Integer(1)),
        "GRAPHEME_EXTR_MAXCHARS" => return Some(Value::Integer(2)),
        "MB_CASE_UPPER" => return Some(Value::Integer(0)),
        "MB_CASE_LOWER" => return Some(Value::Integer(1)),
        "MB_CASE_TITLE" => return Some(Value::Integer(2)),
        "MB_CASE_FOLD" => return Some(Value::Integer(3)),
        "MB_CASE_UPPER_SIMPLE" => return Some(Value::Integer(4)),
        "MB_CASE_LOWER_SIMPLE" => return Some(Value::Integer(5)),
        "MB_CASE_TITLE_SIMPLE" => return Some(Value::Integer(6)),
        "MB_CASE_FOLD_SIMPLE" => return Some(Value::Integer(7)),
        "IDNA_DEFAULT" => return Some(Value::Integer(0)),
        "IDNA_ALLOW_UNASSIGNED" => return Some(Value::Integer(1)),
        "IDNA_USE_STD3_RULES" => return Some(Value::Integer(2)),
        "IDNA_CHECK_BIDI" => return Some(Value::Integer(4)),
        "IDNA_CHECK_CONTEXTJ" => return Some(Value::Integer(8)),
        "IDNA_NONTRANSITIONAL_TO_ASCII" => return Some(Value::Integer(16)),
        "IDNA_NONTRANSITIONAL_TO_UNICODE" => return Some(Value::Integer(32)),
        "INTL_IDNA_VARIANT_2003" => return Some(Value::Integer(0)),
        "INTL_IDNA_VARIANT_UTS46" => return Some(Value::Integer(1)),
        "UUID_VARIANT_NCS" | "UUID_TYPE_DEFAULT" => return Some(Value::Integer(0)),
        "UUID_VARIANT_DCE" | "UUID_TYPE_TIME" => return Some(Value::Integer(1)),
        "UUID_VARIANT_MICROSOFT" | "UUID_TYPE_SECURITY" => return Some(Value::Integer(2)),
        "UUID_VARIANT_OTHER" => return Some(Value::Integer(3)),
        "UUID_TYPE_DCE" => return Some(Value::Integer(4)),
        "UUID_TYPE_NAME" => return Some(Value::Integer(1)),
        "UUID_TYPE_TIME_V6" => return Some(Value::Integer(6)),
        "UUID_TYPE_TIME_V7" => return Some(Value::Integer(7)),
        "UUID_TYPE_VENDOR" => return Some(Value::Integer(8)),
        "FILE_USE_INCLUDE_PATH" => return Some(Value::Integer(1)),
        "FILE_IGNORE_NEW_LINES" => return Some(Value::Integer(2)),
        "FILE_SKIP_EMPTY_LINES" => return Some(Value::Integer(4)),
        "FILE_APPEND" => return Some(Value::Integer(8)),
        "FILE_NO_DEFAULT_CONTEXT" => return Some(Value::Integer(16)),
        "PATHINFO_DIRNAME" => return Some(Value::Integer(1)),
        "PATHINFO_BASENAME" => return Some(Value::Integer(2)),
        "PATHINFO_EXTENSION" => return Some(Value::Integer(4)),
        "PATHINFO_FILENAME" => return Some(Value::Integer(8)),
        "PATHINFO_ALL" => return Some(Value::Integer(15)),
        "UPLOAD_ERR_OK" => return Some(Value::Integer(0)),
        "UPLOAD_ERR_INI_SIZE" => return Some(Value::Integer(1)),
        "UPLOAD_ERR_FORM_SIZE" => return Some(Value::Integer(2)),
        "UPLOAD_ERR_PARTIAL" => return Some(Value::Integer(3)),
        "UPLOAD_ERR_NO_FILE" => return Some(Value::Integer(4)),
        "UPLOAD_ERR_NO_TMP_DIR" => return Some(Value::Integer(6)),
        "UPLOAD_ERR_CANT_WRITE" => return Some(Value::Integer(7)),
        "UPLOAD_ERR_EXTENSION" => return Some(Value::Integer(8)),
        "LOCK_EX" => return Some(Value::Integer(2)),
        "LOCK_SH" => return Some(Value::Integer(1)),
        "LOCK_UN" => return Some(Value::Integer(3)),
        "SEEK_SET" => return Some(Value::Integer(0)),
        "SEEK_CUR" => return Some(Value::Integer(1)),
        "SEEK_END" => return Some(Value::Integer(2)),
        _ => {}
    }
    let value = match name {
        "SORT_REGULAR" | "COUNT_NORMAL" | "CASE_LOWER" | "EXTR_OVERWRITE" => 0.0,
        "SORT_NUMERIC" | "COUNT_RECURSIVE" | "CASE_UPPER" | "EXTR_SKIP" | "ARRAY_FILTER_USE_BOTH" => 1.0,
        "SORT_STRING" | "EXTR_PREFIX_SAME" | "ARRAY_FILTER_USE_KEY" => 2.0,
        "SORT_DESC" | "EXTR_PREFIX_ALL" => 3.0,
        "SORT_ASC" | "EXTR_PREFIX_INVALID" => 4.0,
        "SORT_LOCALE_STRING" | "EXTR_PREFIX_IF_EXISTS" => 5.0,
        "SORT_NATURAL" | "EXTR_IF_EXISTS" => 6.0,
        "SORT_FLAG_CASE" => 8.0,
        "STR_PAD_LEFT" => 0.0,
        "STR_PAD_RIGHT" => 1.0,
        "STR_PAD_BOTH" => 2.0,
        "JSON_HEX_TAG" => 1.0,
        "JSON_HEX_AMP" => 2.0,
        "JSON_HEX_APOS" => 4.0,
        "JSON_HEX_QUOT" => 8.0,
        "JSON_NUMERIC_CHECK" => 32.0,
        "JSON_UNESCAPED_SLASHES" => 64.0,
        "JSON_PRETTY_PRINT" => 128.0,
        "JSON_OBJECT_AS_ARRAY" => 1.0,
        "JSON_BIGINT_AS_STRING" => 2.0,
        "JSON_UNESCAPED_UNICODE" => 256.0,
        "JSON_UNESCAPED_LINE_TERMINATORS" => 2048.0,
        "JSON_PRESERVE_ZERO_FRACTION" => 1024.0,
        "JSON_FORCE_OBJECT" => 16.0,
        "JSON_PARTIAL_OUTPUT_ON_ERROR" => 512.0,
        "JSON_THROW_ON_ERROR" => 4194304.0,
        "JSON_INVALID_UTF8_IGNORE" => 1048576.0,
        "JSON_INVALID_UTF8_SUBSTITUTE" => 2097152.0,
        "JSON_ERROR_NONE" => 0.0,
        "JSON_ERROR_DEPTH" => 1.0,
        "JSON_ERROR_STATE_MISMATCH" => 2.0,
        "JSON_ERROR_CTRL_CHAR" => 3.0,
        "JSON_ERROR_SYNTAX" => 4.0,
        "JSON_ERROR_UTF8" => 5.0,
        "JSON_ERROR_RECURSION" => 6.0,
        "JSON_ERROR_INF_OR_NAN" => 7.0,
        "JSON_ERROR_UNSUPPORTED_TYPE" => 8.0,
        "JSON_ERROR_INVALID_PROPERTY_NAME" => 9.0,
        "JSON_ERROR_UTF16" => 10.0,
        "JSON_ERROR_NON_BACKED_ENUM" => 11.0,
        "ENT_COMPAT" => 2.0,
        "ENT_QUOTES" => 3.0,
        "ENT_NOQUOTES" | "ENT_HTML401" => 0.0,
        "ENT_SUBSTITUTE" => 8.0,
        "ENT_DISALLOWED" => 128.0,
        "ENT_XML1" => 16.0,
        "ENT_XHTML" => 32.0,
        "ENT_HTML5" => 48.0,
        "EXTR_REFS" => 256.0,
        _ => return None,
    };
    Some(Value::Integer(value as i64))
}

pub(super) fn constant_exists(name: &str, globals: &GlobalVariables) -> bool {
    let name = name.trim_start_matches('\\');
    globals.constants.contains_key(name) || builtin_constant(name).is_some()
}

pub(super) fn read_constant(name: &str, globals: &GlobalVariables) -> Result<Value, String> {
    let name = name.trim_start_matches('\\');
    if let Some(value) = globals.constants.get(name) { return Ok(value.clone()); }
    if let Some(value) = builtin_constant(name) { return Ok(value); }
    // Unqualified constants in a namespace fall back to the global namespace.
    if let Some((_, global_name)) = name.rsplit_once('\\') {
        if let Some(value) = globals.constants.get(global_name) { return Ok(value.clone()); }
        if let Some(value) = builtin_constant(global_name) { return Ok(value); }
    }
    Err(format!("Undefined constant \"{name}\""))
}

/// Check whether a name exists in the active symbol table without confusing a stored null with absence.
pub(super) fn variable_exists(
    name: &str,
    variables: &HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &GlobalVariables,
    global_scope: bool,
) -> bool {
    if name == "GLOBALS" { return true; }
    if let Some(binding) = bindings.get(name) {
        let storage_name = match binding {
            VariableBinding::Global(name) | VariableBinding::Static(name) | VariableBinding::Reference(name) => name,
            VariableBinding::ArrayElement { storage_key, keys } => {
                return globals.get(storage_key).and_then(|value| value.array_get_path(keys).ok()).is_some();
            }
            VariableBinding::ObjectProperty { .. } => return true,
        };
        globals.get(storage_name).is_some()
    } else if global_scope || is_superglobal(name) {
        globals.get_named(name).is_some()
    } else {
        variables.contains_key(name)
    }
}

fn warn_undefined_variable(
    name: &str,
    variables: &HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
) {
    if !variable_exists(name, variables, bindings, globals, global_scope) {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("Undefined variable ${name}"),
            source_file: None,
            source_line: None,
        });
    }
}

pub(super) fn warn_undefined_array_key(root: &Value, keys: &[crate::ArrayKey], globals: &mut GlobalVariables) {
    if let Some(key) = root.first_missing_array_path_key(keys) {
        let rendered = match key {
            crate::ArrayKey::Integer(index) => index.to_string(),
            crate::ArrayKey::String(key) => format!("\"{key}\""),
        };
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Warning,
            message: format!("Undefined array key {rendered}"),
            source_file: None,
            source_line: None,
        });
    }
}

impl GlobalVariables {
    fn new() -> Self { Self::default() }

    fn resolve_key(&self, name: &str) -> String {
        let mut key = name.to_string();
        let mut visited = HashSet::new();
        while visited.insert(key.clone()) {
            let Some(next) = self.aliases.get(&key) else { break; };
            key = next.clone();
        }
        key
    }

    fn get(&self, key: &str) -> Option<&Value> {
        let storage_key = self.resolve_key(key);
        self.values.get(&storage_key)
    }

    fn get_named(&self, name: &str) -> Option<&Value> { self.get(name) }

    /// Materialize the named global symbol table for PHP's `$GLOBALS` array.
    fn globals_array(&self) -> Value {
        let mut names = self.order.iter()
            .filter(|name| !name.starts_with('\0') && name.as_str() != "GLOBALS")
            .cloned().collect::<Vec<_>>();
        // Keep internal or legacy entries visible if they were created outside `set`.
        for name in self.values.keys().chain(self.aliases.keys()) {
            if !name.starts_with('\0') && name != "GLOBALS" && !names.contains(name) {
                names.push(name.clone());
            }
        }
        Value::array(names.into_iter().map(|name| (
            crate::ArrayKey::String(name.clone()),
            self.get(&name).map(Value::dereference).unwrap_or(Value::Null),
        )).collect())
    }

    /// Apply a modified `$GLOBALS` array back to named global slots.
    fn replace_from_globals_array(&mut self, value: Value) -> Result<(), String> {
        let Value::Array(entries) = value.dereference() else {
            return Err("$GLOBALS must remain an array".into());
        };
        let mut replacements = Vec::new();
        for (key, value) in entries.iter() {
            let name = match key {
                crate::ArrayKey::String(name) => name.clone(),
                crate::ArrayKey::Integer(index) => index.to_string(),
            };
            if !name.starts_with('\0') && name != "GLOBALS" {
                replacements.push((name, value.dereference()));
            }
        }
        let old_names = self.values.keys().chain(self.aliases.keys())
            .filter(|name| !name.starts_with('\0') && name.as_str() != "GLOBALS")
            .cloned().collect::<std::collections::BTreeSet<_>>();
        for name in old_names {
            if !replacements.iter().any(|(replacement, _)| replacement == &name) { self.remove_named(&name); }
        }
        for (name, value) in replacements { self.set(name, value); }
        Ok(())
    }

    fn set(&mut self, key: String, value: Value) {
        let storage_key = self.resolve_key(&key);
        if !self.values.contains_key(&storage_key) && !self.order.contains(&key) {
            self.order.push(key.clone());
        }
        if let Some(Value::Reference(reference)) = self.values.get(&storage_key) {
            *reference.borrow_mut() = value.dereference();
        } else {
            self.values.insert(storage_key, value);
        }
    }

    fn ensure(&mut self, key: String, value: Value) {
        let storage_key = self.resolve_key(&key);
        if !self.values.contains_key(&storage_key) && !self.order.contains(&key) {
            self.order.push(key.clone());
        }
        self.values.entry(storage_key).or_insert(value);
    }

    fn remove_named(&mut self, name: &str) {
        self.order.retain(|candidate| candidate != name);
        if self.aliases.remove(name).is_none() {
            self.values.remove(name);
        }
    }

    fn bind_alias(&mut self, name: String, target_key: String) {
        let target_key = self.resolve_key(&target_key);
        if name == target_key { self.aliases.remove(&name); }
        else {
            if !self.order.contains(&name) { self.order.push(name.clone()); }
            self.values.remove(&name);
            self.aliases.insert(name, target_key);
        }
    }

    /// Turn a global storage slot into a shared cell, preserving an existing cell when present.
    fn reference_cell(&mut self, key: &str) -> Rc<RefCell<Value>> {
        let storage_key = self.resolve_key(key);
        let slot = self.values.entry(storage_key).or_insert(Value::Null);
        if let Value::Reference(reference) = slot { return reference.clone(); }
        let reference = Rc::new(RefCell::new(slot.clone()));
        *slot = Value::Reference(reference.clone());
        reference
    }

    /// Store a standalone reference cell under a VM-private storage key.
    pub(super) fn store_reference(&mut self, reference: Rc<RefCell<Value>>) -> String {
        let key = format!("\0array-reference:{}", self.next_reference_id);
        self.next_reference_id = self.next_reference_id.wrapping_add(1);
        self.values.insert(key.clone(), Value::Reference(reference));
        key
    }

    fn store_thrown(&mut self, id: u64, value: Value) { self.thrown_values.insert(id, value); }
    fn take_thrown(&mut self, id: u64) -> Option<Value> { self.thrown_values.remove(&id) }
}

fn export_session_value(value: &Value, depth: usize, nodes: &mut usize) -> Option<SessionValue> {
    *nodes += 1;
    if depth > 32 || *nodes > 10_000 { return None; }
    let value = value.dereference();
    Some(match &value {
        Value::Null => SessionValue::Null,
        Value::Integer(value) => SessionValue::Integer(*value),
        Value::Number(value) => SessionValue::Number(*value),
        Value::String(value) => SessionValue::String(value.clone()),
        Value::Bool(value) => SessionValue::Bool(*value),
        Value::Array(values) => SessionValue::Array(values.iter().map(|(key, value)| {
            Some((key.clone(), export_session_value(value, depth + 1, nodes)?))
        }).collect::<Option<Vec<(ArrayKey, SessionValue)>>>()?),
        Value::Object(_) | Value::Closure(_) | Value::Generator(_) | Value::Stream(_) | Value::Reference(_) => return None,
    })
}
const THROWN_VALUE_PREFIX: &str = "\0php-thrown:";

#[derive(Debug)]
enum ExceptionHandler {
    Catch { clauses: Vec<crate::bytecode::CatchTarget> },
    Finally { target: usize, protected_ranges: Vec<(usize, usize)> },
}

#[derive(Debug)]
enum PendingExit {
    Exception(u64),
    Return(Value),
    Jump(usize),
}

#[derive(Debug)]
struct FrameContinuation {
    pc: usize,
    stack: Vec<Value>,
    iterators: control::Iterators,
    yield_from: HashMap<usize, YieldFromIterator>,
    handlers: Vec<ExceptionHandler>,
    pending_exits: Vec<PendingExit>,
    bindings: HashMap<String, VariableBinding>,
    frame_id: u64,
    awaiting_resume: bool,
    yielded: Option<(Option<Value>, Value)>,
}

impl FrameContinuation {
    fn new(bindings: HashMap<String, VariableBinding>, frame_id: u64) -> Self {
        Self {
            pc: 0,
            stack: Vec::new(),
            iterators: HashMap::new(),
            yield_from: HashMap::new(),
            handlers: Vec::new(),
            pending_exits: Vec::new(),
            bindings,
            frame_id,
            awaiting_resume: false,
            yielded: None,
        }
    }
}

fn advance_yield_from(
    iterator: &mut YieldFromIterator,
    sent: Value,
    thrown: Option<Value>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    globals: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
    stack: &mut Vec<Value>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_scope: bool,
    frame_id: u64,
) -> Result<Option<(Value, Value)>, String> {
    match iterator {
        YieldFromIterator::Array(entries, index) => {
            if *index >= entries.len() { return Ok(None); }
            let entry = entries[*index].clone();
            *index += 1;
            Ok(Some((match entry.0 { ArrayKey::Integer(key) => Value::Integer(key), ArrayKey::String(key) => Value::String(key) }, entry.1)))
        }
        YieldFromIterator::Generator(generator) => {
            if let Some(exception) = thrown {
                advance_generator_with_exception(generator, functions, classes, static_properties, globals, output, steps, Value::Null, Some(exception))
            } else {
                advance_generator(generator, functions, classes, static_properties, globals, output, steps, sent)
            }
        }
        YieldFromIterator::UserIterator { object, advance_pending } => {
            if *advance_pending {
                let _ = control::call_iterator_method(object.clone(), "next", stack, variables, bindings, globals,
                    global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                *advance_pending = false;
            }
            let valid = control::call_iterator_method(object.clone(), "valid", stack, variables, bindings, globals,
                global_scope, frame_id, functions, classes, static_properties, output, steps)?;
            if !valid.truthy() { return Ok(None); }
            let value = control::call_iterator_method(object.clone(), "current", stack, variables, bindings, globals,
                global_scope, frame_id, functions, classes, static_properties, output, steps)?;
            let key = control::call_iterator_method(object.clone(), "key", stack, variables, bindings, globals,
                global_scope, frame_id, functions, classes, static_properties, output, steps)?;
            *advance_pending = true;
            Ok(Some((key, value)))
        }
    }
}

fn yield_from_return_value(iterator: &YieldFromIterator) -> Result<Value, String> {
    match iterator {
        YieldFromIterator::Array(_, _) => Ok(Value::Null),
        YieldFromIterator::Generator(generator) => Ok(generator.try_borrow().map_err(|_| "Generator is already running")?.return_value.clone().unwrap_or(Value::Null)),
        YieldFromIterator::UserIterator { .. } => Ok(Value::Null),
    }
}

#[derive(Debug)]
enum YieldFromIterator {
    Array(Vec<(ArrayKey, Value)>, usize),
    Generator(Rc<RefCell<PhpGenerator>>),
    UserIterator { object: Value, advance_pending: bool },
}

/// A suspended PHP generator. Its continuation is resumed by `foreach` or generator methods.
#[derive(Debug)]
pub struct PhpGenerator {
    function: BytecodeFunction,
    variables: HashMap<String, Value>,
    continuation: FrameContinuation,
    current: Option<(Value, Value)>,
    next_integer_key: i64,
    started: bool,
    advanced_past_first: bool,
    completed: bool,
    return_value: Option<Value>,
}

fn route_exit_through_finally(source: usize, target: Option<usize>, handlers: &mut Vec<ExceptionHandler>) -> Option<usize> {
    let index = handlers.iter().rposition(|handler| match handler {
        ExceptionHandler::Finally { protected_ranges, .. } => {
            let source_is_protected = protected_ranges.iter().any(|(start, end)| *start <= source && source <= *end);
            let target_is_protected = target.is_some_and(|target| protected_ranges.iter().any(|(start, end)| *start <= target && target <= *end));
            source_is_protected && !target_is_protected
        }
        ExceptionHandler::Catch { .. } => false,
    })?;
    let ExceptionHandler::Finally { target, .. } = &handlers[index] else { unreachable!() };
    let finalizer_target = *target;
    // Discard this scope's handler and any nested handlers it encloses, leaving outer scopes active.
    handlers.truncate(index);
    Some(finalizer_target)
}

/// Build the persistent storage namespace used by function-local static variables.
pub(super) fn function_scope(function: &BytecodeFunction) -> String {
    match &function.declaring_class {
        Some(class) => format!("{class}::{}", function.name),
        None => function.display_name.clone(),
    }
}

/// Read a variable from its local frame or the shared global table when it is bound.
pub(super) fn read_variable(
    name: &str,
    variables: &HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &GlobalVariables,
    global_scope: bool,
) -> Value {
    if name == "GLOBALS" { return globals.globals_array(); }
    if let Some(binding) = bindings.get(name) {
        let storage_name = match binding {
            VariableBinding::Global(name) => name,
            VariableBinding::Static(name) | VariableBinding::Reference(name) => name,
            VariableBinding::ArrayElement { storage_key, keys } => {
                return globals.get(storage_key).and_then(|value| value.array_get_path(keys).ok()).unwrap_or(Value::Null);
            }
            VariableBinding::ObjectProperty { receiver, property } => {
                let Value::Object(object) = receiver else { return Value::Null; };
                return object.borrow().properties.get(property).map(Value::dereference).unwrap_or(Value::Null);
            }
        };
        globals.get(storage_name).map(Value::dereference).unwrap_or(Value::Null)
    } else if global_scope || is_superglobal(name) {
        globals.get_named(name).map(Value::dereference).unwrap_or(Value::Null)
    } else {
        variables.get(name).map(Value::dereference).unwrap_or(Value::Null)
    }
}

/// Write through a global binding or update the current local frame.
pub(super) fn write_variable(
    name: String,
    value: Value,
    variables: &mut HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
) -> Result<(), String> {
    let value = value.dereference();
    if name == "GLOBALS" { return Err("Cannot re-assign $GLOBALS".into()); }
    if let Some(binding) = bindings.get(&name) {
        if let VariableBinding::ArrayElement { storage_key, keys } = binding {
            let current = globals.get(storage_key).cloned().unwrap_or(Value::Null);
            let updated = current.array_set_path(keys, value)?;
            globals.set(storage_key.clone(), updated);
            return Ok(());
        }
        if let VariableBinding::ObjectProperty { receiver, property } = binding {
            let Value::Object(object) = receiver else { return Err("Property reference receiver is not an object".into()); };
            let mut object = object.borrow_mut();
            if let Some(Value::Reference(reference)) = object.properties.get(property) {
                *reference.borrow_mut() = value.dereference();
            } else {
                object.properties.insert(property.clone(), value);
            }
            return Ok(());
        }
        let storage_name = match binding {
            VariableBinding::Global(name) => name,
            VariableBinding::Static(name) | VariableBinding::Reference(name) => name,
            VariableBinding::ArrayElement { .. } => unreachable!(),
            VariableBinding::ObjectProperty { .. } => unreachable!(),
        };
        globals.set(storage_name.clone(), value);
    } else if global_scope || is_superglobal(&name) {
        globals.set(name, value);
    } else {
        if !variables.contains_key(&name) { update_local_order(variables, &name, true); }
        variables.insert(name, value);
    }
    Ok(())
}

/// Write an array rebuilt from a path operation, committing `$GLOBALS` entries when needed.
pub(super) fn write_array_root(
    name: String,
    value: Value,
    variables: &mut HashMap<String, Value>,
    bindings: &HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
) -> Result<(), String> {
    if name == "GLOBALS" { globals.replace_from_globals_array(value) }
    else { write_variable(name, value, variables, bindings, globals, global_scope) }
}

/// Remove a variable from its active local or global storage.
pub(super) fn remove_variable(
    name: &str,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
) -> Result<(), String> {
    if name == "GLOBALS" { return Err("Cannot unset $GLOBALS".into()); }
    match bindings.get(name).cloned() {
        Some(VariableBinding::Global(storage_name)) => globals.remove_named(&storage_name),
        Some(VariableBinding::Static(_)) | Some(VariableBinding::Reference(_)) | Some(VariableBinding::ArrayElement { .. }) | Some(VariableBinding::ObjectProperty { .. }) => { bindings.remove(name); }
        None if global_scope || is_superglobal(name) => globals.remove_named(name),
        None => { variables.remove(name); update_local_order(variables, name, false); }
    }
    Ok(())
}

/// Obtain a storage key for a variable, promoting a local to a shared reference cell if needed.
pub(super) fn reference_storage_key(
    name: &str,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> String {
    if let Some(binding) = bindings.get(name) {
        let key = match binding {
            VariableBinding::Global(name) => name,
            VariableBinding::Static(name) | VariableBinding::Reference(name) => name,
            VariableBinding::ArrayElement { storage_key, .. } => storage_key,
            VariableBinding::ObjectProperty { .. } => return format!("\0object-property-reference:{frame_id}:{name}"),
        };
        return globals.resolve_key(key);
    }
    if global_scope {
        globals.ensure(name.to_string(), Value::Null);
        return globals.resolve_key(name);
    }
    let value = variables.remove(name).unwrap_or(Value::Null);
    let key = format!("\0reference:{frame_id}:{name}");
    globals.set(key.clone(), value);
    bindings.insert(name.to_string(), VariableBinding::Reference(key.clone()));
    key
}

pub(super) fn reference_cell_for_path(
    name: &str,
    keys: &[ArrayKey],
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<(String, Rc<RefCell<Value>>), String> {
    if name == "GLOBALS" {
        let Some((key, nested_keys)) = keys.split_first() else {
            return Err("Cannot create a reference to $GLOBALS itself".into());
        };
        let global_name = match key {
            crate::ArrayKey::String(name) => name.clone(),
            crate::ArrayKey::Integer(index) => index.to_string(),
        };
        if global_name == "GLOBALS" { return Err("Cannot create a reference to $GLOBALS itself".into()); }
        if nested_keys.is_empty() {
            let storage_key = globals.resolve_key(&global_name);
            return Ok((storage_key.clone(), globals.reference_cell(&storage_key)));
        }
        let root = globals.get(&global_name).map(Value::dereference).unwrap_or(Value::Null);
        let (updated, reference) = root.ensure_array_reference(nested_keys)?;
        globals.set(global_name, updated);
        let storage_key = globals.store_reference(reference.clone());
        return Ok((storage_key, reference));
    }
    let (storage_key, keys) = match bindings.get(name) {
        Some(VariableBinding::ArrayElement { storage_key, keys: prefix }) => {
            let mut combined = prefix.clone();
            combined.extend_from_slice(keys);
            (globals.resolve_key(storage_key), combined)
        }
        Some(VariableBinding::ObjectProperty { .. }) => return Err("Array-element references through object-property references are not supported".into()),
        _ => (reference_storage_key(name, variables, bindings, globals, global_scope, frame_id), keys.to_vec()),
    };
    if keys.is_empty() {
        return Ok((storage_key.clone(), globals.reference_cell(&storage_key)));
    }
    let root = globals.get(&storage_key).map(Value::dereference).unwrap_or(Value::Null);
    let (updated, reference) = root.ensure_array_reference(&keys)?;
    globals.set(storage_key, updated);
    let key = globals.store_reference(reference.clone());
    Ok((key, reference))
}

fn bind_array_reference(
    target: &str,
    target_keys: Vec<ArrayKey>,
    source: &str,
    source_keys: Vec<ArrayKey>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<(), String> {
    let (source_storage_key, reference) = reference_cell_for_path(source, &source_keys, variables, bindings, globals, global_scope, frame_id)?;
    if target == "GLOBALS" {
        let Some((key, nested_keys)) = target_keys.split_first() else {
            return Err("Cannot bind a reference to $GLOBALS itself".into());
        };
        let global_name = match key {
            crate::ArrayKey::String(name) => name.clone(),
            crate::ArrayKey::Integer(index) => index.to_string(),
        };
        if global_name == "GLOBALS" { return Err("Cannot bind a reference to $GLOBALS itself".into()); }
        if nested_keys.is_empty() {
            globals.bind_alias(global_name, source_storage_key);
        } else {
            let root = globals.get(&global_name).map(Value::dereference).unwrap_or(Value::Null);
            let updated = root.bind_array_reference(nested_keys, reference)?;
            globals.set(global_name, updated);
        }
        return Ok(());
    }
    let target_value = read_variable(target, variables, bindings, globals, global_scope);
    let updated = target_value.bind_array_reference(&target_keys, reference)
        .map_err(|error| format!("Reference target ${target}{target_keys:?}: {error}"))?;
    write_variable(target.to_string(), updated, variables, bindings, globals, global_scope)
}

/// Bind a local name to a live array path within its source variable.
fn assign_array_reference(
    target: &str,
    source: &str,
    keys: Vec<ArrayKey>,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<(), String> {
    let (source_key, _) = reference_cell_for_path(source, &keys, variables, bindings, globals, global_scope, frame_id)?;
    match bindings.get(target).cloned() {
        Some(VariableBinding::Global(name)) | Some(VariableBinding::Static(name)) => globals.bind_alias(name, source_key),
        None if global_scope => globals.bind_alias(target.to_string(), source_key),
        _ => {
            bindings.insert(target.to_string(), VariableBinding::Reference(source_key));
            variables.remove(target);
        }
    }
    Ok(())
}

/// Bind a local or global target name to the same reference cell as its source.
fn assign_reference(
    target: &str,
    source: &str,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<(), String> {
    if target == "GLOBALS" || source == "GLOBALS" {
        return Err("Cannot create a reference to $GLOBALS itself".into());
    }
    let source_element = match bindings.get(source) {
        Some(binding @ (VariableBinding::ArrayElement { .. } | VariableBinding::ObjectProperty { .. })) => Some(binding.clone()),
        _ => None,
    };
    let source_key = source_element.as_ref().map(|binding| match binding {
        VariableBinding::ArrayElement { storage_key, .. } => globals.resolve_key(storage_key),
        VariableBinding::ObjectProperty { .. } => format!("\0object-property-reference:{frame_id}:{source}"),
        _ => unreachable!(),
    }).unwrap_or_else(|| reference_storage_key(source, variables, bindings, globals, global_scope, frame_id));
    match bindings.get(target).cloned() {
        Some(VariableBinding::Global(name)) => globals.bind_alias(name, source_key),
        Some(VariableBinding::Static(name)) => globals.bind_alias(name, source_key),
        Some(VariableBinding::Reference(_)) | Some(VariableBinding::ArrayElement { .. }) | Some(VariableBinding::ObjectProperty { .. }) => {
            bindings.insert(target.to_string(), source_element.unwrap_or(VariableBinding::Reference(source_key)));
        }
        None if global_scope => globals.bind_alias(target.to_string(), source_key),
        None => {
            variables.remove(target);
            bindings.insert(target.to_string(), source_element.unwrap_or(VariableBinding::Reference(source_key)));
        }
    }
    Ok(())
}

/// Fill omitted optional positional parameters and reject missing required ones.
pub(super) fn complete_arguments(
    mut arguments: Vec<Value>,
    function: &BytecodeFunction,
    label: &str,
) -> Result<Vec<Value>, String> {
    if let Some(variadic_index) = function.parameter_variadic.iter().position(|variadic| *variadic) {
        for index in arguments.len()..variadic_index {
            let Some(Some(default)) = function.parameter_defaults.get(index) else {
                return Err(format!("{label} expects at least {} arguments, {} given", index + 1, arguments.len()));
            };
            arguments.push(default.materialize_parameter_default());
        }
        let extra_arguments = if arguments.len() > variadic_index { arguments.split_off(variadic_index) } else { Vec::new() };
        let packed = extra_arguments.into_iter().enumerate()
            .map(|(index, value)| (crate::ArrayKey::Integer(index as i64), value))
            .collect();
        arguments.push(Value::array(packed));
        return Ok(arguments);
    }
    for index in arguments.len()..function.parameters.len() {
        let Some(Some(default)) = function.parameter_defaults.get(index) else {
            return Err(format!("{label} expects at least {} arguments, {} given", index + 1, arguments.len()));
        };
        arguments.push(default.materialize_parameter_default());
    }
    Ok(arguments)
}


/// Check that a function result matches its declared return type.
pub(super) fn validate_return_type(
    value: Value,
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    validate_return_type_in_scope(value, function, classes, function.declaring_class.as_deref())
}

/// Validate a return type using the runtime called class for `static` types.
pub(super) fn validate_return_type_in_scope(
    value: Value,
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
    called_class: Option<&str>,
) -> Result<Value, String> {
    if function.returns_by_reference && !matches!(value, Value::Reference(_)) {
        return Err(format!("{}() must return a reference", function.name));
    }
    let Some(type_name) = function.return_type.as_deref() else { return Ok(value); };
    let value = if function.returns_by_reference {
        value
    } else if function.strict_types {
        coerce_int_to_float_return(value, type_name)
    } else {
        coerce_numeric_return(value, type_name)
    };
    let dereferenced = value.dereference();
    let contextual_type = resolve_class_relative_type(type_name, function, classes, called_class);
    if strict_return_value_matches_type(&dereferenced, &contextual_type, classes) { return Ok(value); }
    let label = function.declaring_class.as_deref().unwrap_or("function");
    Err(format!("TypeError:{label}::{}() must return {type_name}", function.name))
}

fn coerce_int_to_float_return(value: Value, type_name: &str) -> Value {
    let normalized = type_name.trim().trim_start_matches('?').trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "float" | "double") {
        if let Value::Integer(integer) = value { return Value::Number(integer as f64); }
    }
    value
}

fn strict_return_value_matches_type(value: &Value, type_name: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    if value_matches_type(value, type_name, classes) { return true; }
    if type_name.trim().eq_ignore_ascii_case("float") || type_name.trim().eq_ignore_ascii_case("double") {
        return matches!(value, Value::Integer(_));
    }
    if let Some(inner) = type_name.strip_prefix('?') {
        return matches!(value, Value::Null) || strict_return_value_matches_type(value, inner, classes);
    }
    if type_name.contains('|') {
        return type_name.split('|').any(|part| strict_return_value_matches_type(value, part.trim().trim_matches(['(', ')']), classes));
    }
    false
}

fn resolve_class_relative_type(
    type_name: &str,
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
    called_class: Option<&str>,
) -> String {
    let Some(declaring_class) = function.declaring_class.as_deref() else { return type_name.to_string(); };
    let mut resolved = String::with_capacity(type_name.len());
    let mut token = String::new();
    let flush_token = |token: &mut String, output: &mut String| {
        if token.eq_ignore_ascii_case("self") {
            output.push_str(declaring_class);
        } else if token.eq_ignore_ascii_case("parent") {
            output.push_str(classes.get(declaring_class).and_then(|class| class.parent.as_deref()).unwrap_or("parent"));
        } else if token.eq_ignore_ascii_case("static") {
            output.push_str(called_class.unwrap_or(declaring_class));
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
    resolved
}

fn coerce_numeric_return(value: Value, type_name: &str) -> Value {
    let type_name = type_name.trim_start_matches('?').to_ascii_lowercase();
    if type_name.contains('|') {
        let has = |name: &str| type_name.split('|').any(|member| member == name || (name == "int" && member == "integer") || (name == "float" && member == "double"));
        return match value {
            Value::String(text) if has("int") => text.trim().parse::<i64>().map(Value::Integer).unwrap_or_else(|_| {
                if has("float") { text.trim().parse::<f64>().map(Value::Number).unwrap_or(Value::String(text)) } else { Value::String(text) }
            }),
            Value::String(text) if has("float") => text.trim().parse::<f64>().map(Value::Number).unwrap_or(Value::String(text)),
            Value::String(text) if has("string") => Value::String(text),
            Value::Bool(value) if has("bool") || type_name.contains("boolean") => Value::Bool(value),
            Value::Bool(value) if has("int") => Value::Integer(i64::from(value)),
            Value::Bool(value) if has("float") => Value::Number(if value { 1.0 } else { 0.0 }),
            value => value,
        };
    }
    match (type_name.as_str(), value) {
        ("int" | "integer", Value::Number(number)) if number.is_finite() && number >= i64::MIN as f64 && number < -(i64::MIN as f64) => Value::Integer(number.trunc() as i64),
        ("int" | "integer", Value::Bool(value)) => Value::Integer(i64::from(value)),
        ("int" | "integer", Value::String(text)) => text.trim().parse::<i64>().map(Value::Integer).or_else(|_| {
            text.trim().parse::<f64>().ok().filter(|number| number.is_finite() && *number >= i64::MIN as f64 && *number < -(i64::MIN as f64))
                .map(|number| Value::Integer(number.trunc() as i64)).ok_or(())
        }).unwrap_or(Value::String(text)),
        ("float" | "double", Value::Integer(value)) => Value::Number(value as f64),
        ("float" | "double", Value::Bool(value)) => Value::Number(if value { 1.0 } else { 0.0 }),
        ("float" | "double", Value::String(text)) => text.trim().parse::<f64>().map(Value::Number).unwrap_or(Value::String(text)),
        (_, value) => value,
    }
}

/// Check supplied values against declared parameter type hints.
pub(super) fn validate_arguments(
    arguments: &[Value],
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    let variadic_index = function.parameter_variadic.iter().position(|variadic| *variadic);
    for (index, (argument, expected)) in arguments.iter().zip(&function.parameter_types).enumerate() {
        let Some(expected) = expected else { continue; };
        if Some(index) == variadic_index {
            let Value::Array(values) = argument else { continue; };
            for (extra_index, (_, value)) in values.iter().enumerate() {
                let contextual_type = resolve_class_relative_type(expected, function, classes, None);
                if !value_matches_type(value, &contextual_type, classes) {
                    let parameter = function.parameters.get(index).map(String::as_str).unwrap_or("?");
                    let label = function.declaring_class.as_deref().unwrap_or("function");
                    return Err(format!("Variadic argument {} (${}) passed to {}::{}() must be of type {}", index + extra_index + 1, parameter, label, function.name, expected));
                }
            }
            continue;
        }
        let contextual_type = resolve_class_relative_type(expected, function, classes, None);
        if !value_matches_type(argument, &contextual_type, classes) {
            let parameter = function.parameters.get(index).map(String::as_str).unwrap_or("?");
            let label = function.declaring_class.as_deref().unwrap_or("function");
            return Err(format!("Argument {} (${}) passed to {}::{}() must be of type {}", index + 1, parameter, label, function.name, expected));
        }
    }
    Ok(())
}

/// Test whether a runtime value satisfies a supported PHP type hint.
fn value_matches_type(value: &Value, type_name: &str, classes: &HashMap<String, BytecodeClass>) -> bool {
    let dereferenced = value.dereference();
    let value = &dereferenced;
    if type_name.contains('|') || type_name.contains('&') {
        return type_name.split('|').any(|union_member| union_member.split('&').all(|member| value_matches_type(value, member, classes)));
    }
    let nullable = type_name.starts_with('?');
    let type_name = type_name.trim_start_matches('?');
    if nullable && matches!(value, Value::Null) { return true; }
    match type_name.to_ascii_lowercase().as_str() {
        "mixed" => true,
        "null" => matches!(value, Value::Null),
        "true" => matches!(value, Value::Bool(true)),
        "false" => matches!(value, Value::Bool(false)),
        "void" => matches!(value, Value::Null),
        "int" | "integer" => matches!(value, Value::Integer(_)) || matches!(value, Value::Number(number) if number.is_finite() && number.fract() == 0.0),
        "float" | "double" => matches!(value, Value::Number(_) | Value::Integer(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "array" => matches!(value, Value::Array(_)),
        "iterable" => value_is_iterable(value, classes),
        "object" => matches!(value, Value::Object(_) | Value::Closure(_) | Value::Generator(_)),
        expected => match value {
            Value::Object(object) => {
                let actual = object.borrow().class_name.to_ascii_lowercase();
                is_a_type(&actual, expected, classes, &mut std::collections::HashSet::new())
            }
            Value::Generator(_) => expected.eq_ignore_ascii_case("generator"),
            _ => false,
        },
    }
}

pub(super) fn value_is_iterable(value: &Value, classes: &HashMap<String, BytecodeClass>) -> bool {
    match value.dereference() {
        Value::Array(_) | Value::Generator(_) => true,
        Value::Object(object) => {
            let class_name = object.borrow().class_name.to_ascii_lowercase();
            class_name.eq_ignore_ascii_case("PDOStatement")
                || is_a_type(&class_name, "Traversable", classes, &mut std::collections::HashSet::new())
        }
        _ => false,
    }
}

/// Walk class and interface metadata to test nominal runtime assignability.
pub(super) fn is_a_type(actual: &str, expected: &str, classes: &HashMap<String, BytecodeClass>, visited: &mut std::collections::HashSet<String>) -> bool {
    if actual.eq_ignore_ascii_case(expected) { return true; }
    if crate::exception_builtins::is_class(actual) {
        return crate::exception_builtins::is_a(actual, expected);
    }
    if !visited.insert(actual.to_string()) { return false; }
    let Some(class) = classes.get(actual) else { return false; };
    if expected.eq_ignore_ascii_case("stringable") && class_implements_to_string(actual, classes, &mut std::collections::HashSet::new()) {
        return true;
    }
    class.parent.as_deref().is_some_and(|parent| is_a_type(parent, expected, classes, visited))
        || class.interfaces.iter().any(|interface| is_a_type(interface, expected, classes, visited))
}

fn class_implements_to_string(actual: &str, classes: &HashMap<String, BytecodeClass>, visited: &mut std::collections::HashSet<String>) -> bool {
    if !visited.insert(actual.to_string()) { return false; }
    let Some(class) = classes.get(actual) else { return false; };
    class.methods.get("__tostring").is_some_and(|method| !method.is_static && method.visibility == crate::MemberVisibility::Public)
        || class.parent.as_deref().is_some_and(|parent| class_implements_to_string(parent, classes, visited))
}

fn exception_matches(value: &Value, catch_types: &[String], classes: &HashMap<String, BytecodeClass>) -> bool {
    match value {
        Value::Object(object) => {
            let actual = object.borrow().class_name.to_ascii_lowercase();
            catch_types.iter().any(|catch_type| {
                let expected = catch_type.to_ascii_lowercase();
                if is_a_type(&actual, "throwable", classes, &mut HashSet::new()) {
                    return is_a_type(&actual, &expected, classes, &mut HashSet::new());
                }
                matches!(expected.as_str(), "throwable" | "exception")
                    || is_a_type(&actual, &expected, classes, &mut HashSet::new())
            })
        }
        _ => false,
    }
}

fn runtime_error_class(message: &str) -> Option<(&'static str, &str)> {
    if let Some(message) = message.strip_prefix("Error:") {
        return Some(("Error", message.trim()));
    }
    if let Some(message) = message.strip_prefix("ValueError:") {
        return Some(("ValueError", message.trim()));
    }
    if let Some(message) = message.strip_prefix("TypeError:") {
        return Some(("TypeError", message.trim()));
    }
    if let Some(message) = message.strip_prefix("ArithmeticError:") {
        return Some(("ArithmeticError", message.trim()));
    }
    if message.to_ascii_lowercase().starts_with("cannot access private member ")
        || message.to_ascii_lowercase().starts_with("cannot access protected member ")
        || message.to_ascii_lowercase().starts_with("cannot modify readonly property ")
        || message.to_ascii_lowercase().starts_with("cannot indirectly modify readonly property ")
        || message.to_ascii_lowercase().starts_with("typed property ") && message.to_ascii_lowercase().ends_with(" must not be referenced")
        || message.to_ascii_lowercase().contains("cannot be passed by reference")
    {
        return Some(("Error", message));
    }
    if message.contains("Division by zero") || message.contains("Modulo by zero") {
        return Some(("DivisionByZeroError", message));
    }
    if message == "Object cannot be used as an array key" || message == "Array cannot be used as an array key" || message == "Illegal offset type" {
        return Some(("TypeError", message));
    }
    if message.contains("Bit shift by negative number") {
        return Some(("ArithmeticError", message));
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("expects exactly") || lower.contains("expects at least") || lower.contains("expects at most") {
        return Some(("ArgumentCountError", message));
    }
    if lower.contains("must be of type") || lower.contains("must be a ") || lower.contains("must be an ") {
        return Some(("TypeError", message));
    }
    None
}

/// Execute a compiled program with fresh variables, static storage, and captured output.
pub(crate) fn execute(code: &[Op], functions: &HashMap<String, BytecodeFunction>, classes: &HashMap<String, BytecodeClass>, source_file: &str) -> Result<String, String> {
    execute_with_globals(code, functions, classes, HashMap::new(), source_file).map(|result| result.output)
}

pub(crate) fn execute_with_globals(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    predefined_globals: HashMap<String, Value>,
    source_file: &str,
) -> Result<crate::ExecutionResult, String> {
    execute_with_globals_and_limits(code, functions, classes, predefined_globals, ExecutionLimits::default(), source_file)
}

pub(crate) fn execute_with_globals_and_limits(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    predefined_globals: HashMap<String, Value>,
    limits: ExecutionLimits,
    source_file: &str,
) -> Result<crate::ExecutionResult, String> {
    let _database_session_guard = crate::pdo::ExecutionGuard::enter();
    let _limits_guard = ExecutionLimitGuard::enter(limits);
    let mut variables = HashMap::<String, Value>::new();
    let mut global_variables = GlobalVariables::new();
    global_variables.source_file = source_file.to_string();
    for name in ["_GET", "_POST", "_COOKIE", "_REQUEST", "_FILES", "_SERVER", "_ENV", "_SESSION"] {
        global_variables.ensure(name.to_string(), Value::array(Vec::new()));
    }
    for (name, value) in predefined_globals {
        global_variables.set(name, value);
    }
    // Static properties persist across calls inside this execution, but not across program runs.
    let mut static_properties = classes.iter().flat_map(|(class_name, class)| {
        class.static_properties.iter().filter_map(move |(name, property)| {
            property.default.clone().map(|value| ((class_name.clone(), name.clone()), value))
        })
    }).collect::<StaticProperties>();
    let mut output = String::new();
    let mut steps = 0_u64;
    let execution = execute_code(code, functions, classes, &mut static_properties, &mut variables, &mut global_variables, true, "<main>", HashMap::new(), &mut output, &mut steps);
    let destructors = objects::run_registered_destructors(functions, classes, &mut static_properties, &mut global_variables, &mut output, &mut steps);
    match execution {
        Ok(_) => {
            destructors?;
            let mut session_nodes = 0;
            let session_data = global_variables.get_named("_SESSION")
                .and_then(|value| export_session_value(value, 0, &mut session_nodes));
            let session_started = global_variables.session_started;
            let session_destroyed = global_variables.session_destroyed;
            let session_id = global_variables.get("\0session_id").map(Value::show).unwrap_or_default();
            Ok(crate::ExecutionResult {
                output,
                diagnostics: global_variables.diagnostics,
                status_code: global_variables.http_status,
                headers: global_variables.http_headers,
                session_started,
                session_destroyed,
                session_id,
                session_data,
            })
        }
        Err(error) if error.starts_with(THROWN_VALUE_PREFIX) => {
            let _ = destructors;
            let id = error[THROWN_VALUE_PREFIX.len()..].parse::<u64>().ok();
            let message = id.and_then(|id| global_variables.take_thrown(id)).map(|value| match value {
                Value::Object(object) => {
                    let object = object.borrow();
                    object.exception.as_ref().map(|exception| format!("Uncaught {}: {}", object.class_name, exception.message))
                        .unwrap_or_else(|| format!("Uncaught exception: Object({})", object.class_name))
                }
                value => format!("Uncaught exception: {}", value.show()),
            }).unwrap_or_else(|| "Uncaught exception: unknown value".into());
            Err(message)
        }
        Err(error) => { let _ = destructors; Err(error) },
    }
}

/// Run one bytecode frame with local variables, operand stack, and shared call state.
fn execute_code(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    variables: &mut HashMap<String, Value>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_scope: &str,
    initial_bindings: HashMap<String, VariableBinding>,
    output: &mut String,
    steps: &mut u64,
) -> Result<Option<Value>, String> {
    let _depth_guard = ExecutionDepthGuard::enter()?;
    global_variables.push_trace_frame(frame_scope, variables.contains_key("this"), classes);
    let mut continuation = FrameContinuation::new(initial_bindings, *steps);
    let result = execute_frame(
        code, functions, classes, static_properties, variables, global_variables, global_scope,
        frame_scope, &mut continuation, Value::Null, None, false, output, steps,
    );
    global_variables.active_trace_frames.pop();
    result
}

fn execute_frame(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    variables: &mut HashMap<String, Value>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_scope: &str,
    continuation: &mut FrameContinuation,
    resume_value: Value,
    resume_exception: Option<Value>,
    allow_yield: bool,
    output: &mut String,
    steps: &mut u64,
) -> Result<Option<Value>, String> {
    let mut stack = std::mem::take(&mut continuation.stack);
    let mut iterators = std::mem::take(&mut continuation.iterators);
    let mut yield_from = std::mem::take(&mut continuation.yield_from);
    let mut handlers = std::mem::take(&mut continuation.handlers);
    let mut pending_exits = std::mem::take(&mut continuation.pending_exits);
    let mut bindings = std::mem::take(&mut continuation.bindings);
    let frame_id = continuation.frame_id;
    let limits = EXECUTION_LIMITS.with(Cell::get);
    let mut pc = continuation.pc;
    let mut injected_throw = false;
    let mut delegated_exception = None;
    if continuation.awaiting_resume {
        if let Some(exception) = resume_exception {
            if matches!(code.get(pc), Some(Op::YieldFrom)) && matches!(yield_from.get(&pc), Some(YieldFromIterator::Generator(_))) {
                delegated_exception = Some(exception);
            } else {
                stack.push(exception);
                injected_throw = true;
                yield_from.remove(&pc);
            }
        } else {
            stack.push(resume_value);
        }
        continuation.awaiting_resume = false;
    }
    continuation.yielded = None;

    while pc < code.len() {
        // A shared budget also bounds recursive calls and callback execution.
        *steps += 1;
        if *steps > limits.max_instructions { return Err("Execution instruction limit exceeded".into()); }
        if EXECUTION_DEADLINE.with(|deadline| deadline.get().is_some_and(|deadline| Instant::now() >= deadline)) {
            return Err("Maximum execution time exceeded".into());
        }

        let injected_instruction = Op::Throw;
        let instruction = if injected_throw {
            injected_throw = false;
            &injected_instruction
        } else {
            let instruction = &code[pc];
            pc += 1;
            instruction
        };
        let execution = (|| -> Result<Option<Value>, String> {
        if let Op::Jump(target) = instruction {
            if let Some(finalizer) = route_exit_through_finally(pc - 1, Some(*target), &mut handlers) {
                pending_exits.push(PendingExit::Jump(*target));
                pc = finalizer;
                return Ok(None);
            }
        }
        if arrays::execute_instruction(instruction, &mut stack, variables, &bindings, global_variables, global_scope, functions, classes, static_properties, output, steps)? { return Ok(None); }
        if control::execute_instruction(
            instruction, &mut stack, &mut iterators, variables, &mut bindings, global_variables, global_scope,
            frame_id, &mut pc, functions, classes, static_properties, output, steps,
        )? { return Ok(None); }
        if objects::execute_instruction(instruction, &mut stack, variables, &mut bindings, global_variables, global_scope, frame_id, frame_scope, functions, classes, static_properties, output, steps, &mut pc)? { return Ok(None); }
        if functions::execute_instruction(instruction, &mut stack, variables, &mut bindings, global_variables, global_scope, frame_id, functions, classes, static_properties, output, steps)? { return Ok(None); }
        match instruction {
            Op::SetLine(line) => global_variables.set_trace_line(*line),
            Op::SetFile(file) => global_variables.set_trace_file(file),
            Op::Const(value) => stack.push(value.clone()),
            Op::LoadConstant(name) => stack.push(read_constant(name, global_variables)?),
            Op::DynamicClassName => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let name = match value { Value::Object(object) => object.borrow().class_name.clone(), Value::String(name) => name, other => other.show() };
                stack.push(Value::String(name));
            }
            Op::DeclareConstant(name) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                if constant_exists(name, global_variables) {
                    return Err(format!("Cannot redefine constant {name}"));
                }
                global_variables.constants.insert(name.clone(), value);
            }
            Op::DeclareGlobal(name) => {
                global_variables.ensure(name.clone(), Value::Null);
                bindings.insert(name.clone(), VariableBinding::Global(name.clone()));
            }
            Op::DeclareStatic(name, initial) => {
                let storage_name = format!("\0static:{frame_scope}:{name}");
                global_variables.ensure(storage_name.clone(), initial.clone());
                bindings.insert(name.clone(), VariableBinding::Static(storage_name));
            }
            Op::StaticInitializerCheck(name, target) => {
                let storage_name = format!("\0static:{frame_scope}:{name}");
                if global_variables.get(&storage_name).is_some() {
                    bindings.insert(name.clone(), VariableBinding::Static(storage_name));
                    pc = *target;
                } else {
                    // Leave the slot absent until evaluation succeeds; recursive calls may initialize it first.
                    bindings.insert(name.clone(), VariableBinding::Static(storage_name));
                }
            }
            Op::InitializeStatic(name) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let storage_name = format!("\0static:{frame_scope}:{name}");
                // A nested call may have completed this same initializer while the outer expression ran.
                global_variables.ensure(storage_name.clone(), value);
                bindings.insert(name.clone(), VariableBinding::Static(storage_name));
            }
            Op::AssignReference(target, source) => {
                assign_reference(target, source, variables, &mut bindings, global_variables, global_scope, frame_id)?;
            }
            Op::BindReturnedReference(target) => {
                let Value::Reference(reference) = stack.pop().ok_or("Invalid bytecode stack")? else {
                    return Err("Only a reference-returning function can be assigned by reference".into());
                };
                let source_key = global_variables.store_reference(reference);
                match bindings.get(target).cloned() {
                    Some(VariableBinding::Global(name)) | Some(VariableBinding::Static(name)) => global_variables.bind_alias(name, source_key),
                    _ if global_scope => global_variables.bind_alias(target.clone(), source_key),
                    _ => {
                        variables.remove(target);
                        bindings.insert(target.clone(), VariableBinding::Reference(source_key));
                    }
                }
            }
            Op::BindReturnedReferenceExpr(target) => {
                let Value::Reference(reference) = stack.pop().ok_or("Invalid bytecode stack")? else {
                    return Err("Only a reference-returning function can be assigned by reference".into());
                };
                let source_key = global_variables.store_reference(reference.clone());
                match bindings.get(target).cloned() {
                    Some(VariableBinding::Global(name)) | Some(VariableBinding::Static(name)) => global_variables.bind_alias(name, source_key),
                    _ if global_scope => global_variables.bind_alias(target.clone(), source_key),
                    _ => {
                        variables.remove(target);
                        bindings.insert(target.clone(), VariableBinding::Reference(source_key));
                    }
                }
                stack.push(reference.borrow().clone().dereference());
            }
            Op::LoadReference(source, key_count) => {
                if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
                let raw_keys = stack.split_off(stack.len() - key_count);
                let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                let (_, reference) = reference_cell_for_path(source, &keys, variables, &mut bindings, global_variables, global_scope, frame_id)?;
                stack.push(Value::Reference(reference));
            }
            Op::AssignArrayReference(target, source, key_count) => {
                let values = stack.split_off(stack.len().saturating_sub(*key_count));
                let keys = values.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                assign_array_reference(target, source, keys, variables, &mut bindings, global_variables, global_scope, frame_id)?;
            }
            Op::BindArrayReference(target, target_key_count, source, source_key_count) => {
                let source_values = stack.split_off(stack.len().saturating_sub(*source_key_count));
                let source_keys = source_values.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                let target_values = stack.split_off(stack.len().saturating_sub(*target_key_count));
                let target_keys = target_values.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                bind_array_reference(target, target_keys.clone(), source, source_keys, variables, &mut bindings, global_variables, global_scope, frame_id)
                    .map_err(|error| format!("Cannot bind reference to ${target}{target_keys:?}: {error}"))?;
            }
            Op::BindArrayReturnedReference(target, target_key_count) => {
                let Value::Reference(reference) = stack.pop().ok_or("Invalid bytecode stack")? else {
                    return Err("Only a reference-returning property can be bound to an array element".into());
                };
                let raw_keys = stack.split_off(stack.len().saturating_sub(*target_key_count));
                let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                let value = read_variable(target, variables, &bindings, global_variables, global_scope);
                let updated = value.bind_array_reference(&keys, reference)
                    .map_err(|error| format!("Cannot bind reference to ${target}{keys:?}: {error}"))?;
                write_variable(target.clone(), updated, variables, &bindings, global_variables, global_scope)?;
            }
            Op::BindDynamicPropertyReference => {
                let Value::Reference(reference) = stack.pop().ok_or("Invalid bytecode stack")? else {
                    return Err("Only a reference-returning property can be assigned by reference".into());
                };
                let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
                let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
                let Value::Object(object) = receiver else { return Err("Cannot assign a reference to a property on a non-object".into()); };
                object.borrow_mut().properties.insert(property, Value::Reference(reference));
            }
            Op::BindDynamicPropertyArrayReference(target_key_count, source, source_key_count, access_scope, in_constructor) => {
                let key_count = target_key_count + source_key_count;
                if stack.len() < key_count + 2 { return Err("Invalid bytecode stack".into()); }
                let raw_keys = stack.split_off(stack.len() - key_count);
                let target_keys = raw_keys[..*target_key_count].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
                let source_keys = raw_keys[*target_key_count..].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
                let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
                let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
                let (_, reference) = reference_cell_for_path(source, &source_keys, variables, &mut bindings, global_variables, global_scope, frame_id)?;
                let current = objects::read_property(&receiver, &property, access_scope.as_deref(), functions, classes, static_properties, global_variables, output, steps)?;
                let updated = current.bind_array_reference(&target_keys, reference)?;
                objects::assign_property(&receiver, &property, updated, access_scope.as_deref(), *in_constructor,
                    functions, classes, static_properties, global_variables, output, steps)?;
            }
            Op::CoalescePathJump(target, key_count) => {
                if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
                let start = stack.len() - key_count - 1;
                let root = stack[start].clone();
                let keys = stack[start + 1..].iter().map(Value::array_key).collect::<Result<Vec<_>, _>>()?;
                let current = root.array_get_path(&keys)?;
                if !matches!(current, Value::Null) {
                    stack.truncate(start);
                    stack.push(current);
                    pc = *target;
                }
            }
            Op::CoalesceAssignPath(variable, key_count) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
                let raw_keys = stack.split_off(stack.len() - key_count);
                let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                let root = stack.pop().ok_or("Invalid bytecode stack")?;
                let updated = root.array_set_path(&keys, value.clone())?;
                write_array_root(variable.clone(), updated, variables, &bindings, global_variables, global_scope)?;
                stack.push(value);
            }
            Op::Load(name) => {
                warn_undefined_variable(name, variables, &bindings, global_variables, global_scope);
                stack.push(read_variable(name, variables, &bindings, global_variables, global_scope));
            }
            Op::LoadSilent(name) => stack.push(read_variable(name, variables, &bindings, global_variables, global_scope)),
            Op::LoadDynamic(name) => {
                warn_undefined_variable(name, variables, &bindings, global_variables, global_scope);
                let dynamic_name = read_variable(name, variables, &bindings, global_variables, global_scope).show();
                warn_undefined_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                stack.push(read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope));
            }
            Op::LoadDynamicSilent(name) => {
                let dynamic_name = read_variable(name, variables, &bindings, global_variables, global_scope).show();
                stack.push(read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope));
            }
            Op::LoadDynamicExpr => {
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                warn_undefined_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                stack.push(read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope));
            }
            Op::LoadDynamicExprSilent => {
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                stack.push(read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope));
            }
            Op::CoalesceDynamicExprJump(target) => {
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                let current = read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                if matches!(current, Value::Null) {
                    stack.push(Value::String(dynamic_name));
                } else {
                    stack.push(current);
                    pc = *target;
                }
            }
            Op::Store(name) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                if name == "GLOBALS" { return Err("Cannot re-assign $GLOBALS".into()); }
                let previous = read_variable(name, variables, &bindings, global_variables, global_scope);
                let current = value.clone();
                write_variable(name.clone(), value, variables, &bindings, global_variables, global_scope)?;
                objects::run_destructors_removed_from_value(previous, &current, functions, classes, static_properties, global_variables, output, steps)?;
            }
            Op::StoreArrayRoot(name) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let previous = read_variable(name, variables, &bindings, global_variables, global_scope);
                let current = value.clone();
                write_array_root(name.clone(), value, variables, &bindings, global_variables, global_scope)?;
                objects::run_destructors_removed_from_value(previous, &current, functions, classes, static_properties, global_variables, output, steps)?;
            }
            Op::StoreDynamic(name) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let dynamic_name = read_variable(name, variables, &bindings, global_variables, global_scope).show();
                let previous = read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                let current = value.clone();
                write_variable(dynamic_name, value, variables, &bindings, global_variables, global_scope)?;
                objects::run_destructors_removed_from_value(previous, &current, functions, classes, static_properties, global_variables, output, steps)?;
            }
            Op::StoreDynamicExpr => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                let previous = read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                let current = value.clone();
                write_variable(dynamic_name, value, variables, &bindings, global_variables, global_scope)?;
                objects::run_destructors_removed_from_value(previous, &current, functions, classes, static_properties, global_variables, output, steps)?;
            }
            Op::AssignDynamicExpr => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                let previous = read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                write_variable(dynamic_name, value.clone(), variables, &bindings, global_variables, global_scope)?;
                objects::run_destructors_removed_from_value(previous, &value, functions, classes, static_properties, global_variables, output, steps)?;
                stack.push(value);
            }
            Op::CompoundAssignDynamicExpr(operation) => {
                let right = stack.pop().ok_or("Invalid bytecode stack")?;
                let current = stack.pop().ok_or("Invalid bytecode stack")?;
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                let operation = match operation {
                    crate::bytecode::CompoundOperation::Add => Op::Add,
                    crate::bytecode::CompoundOperation::Subtract => Op::Sub,
                    crate::bytecode::CompoundOperation::Multiply => Op::Mul,
                    crate::bytecode::CompoundOperation::Power => Op::Pow,
                    crate::bytecode::CompoundOperation::Divide => Op::Div,
                    crate::bytecode::CompoundOperation::Modulo => Op::Mod,
                    crate::bytecode::CompoundOperation::Concatenate => Op::Concat,
                    CompoundOperation::BitAnd => Op::BitAnd,
                    CompoundOperation::BitXor => Op::BitXor,
                    CompoundOperation::BitOr => Op::BitOr,
                    CompoundOperation::ShiftLeft => Op::ShiftLeft,
                    CompoundOperation::ShiftRight => Op::ShiftRight,
                };
                let updated = apply_binary(&operation, current, right)?;
                write_variable(dynamic_name, updated.clone(), variables, &bindings, global_variables, global_scope)?;
                stack.push(updated);
            }
            Op::PostIncrement(name, delta) => {
                // Return the old value while storing the incremented numeric value.
                warn_undefined_variable(name, variables, &bindings, global_variables, global_scope);
                let old_value = read_variable(name, variables, &bindings, global_variables, global_scope);
                write_variable(name.clone(), increment_value(&old_value, *delta, global_variables)?, variables, &bindings, global_variables, global_scope)?;
                stack.push(old_value);
            }
            Op::PreIncrement(name, delta) => {
                warn_undefined_variable(name, variables, &bindings, global_variables, global_scope);
                let current_value = read_variable(name, variables, &bindings, global_variables, global_scope);
                let next_value = increment_value(&current_value, *delta, global_variables)?;
                write_variable(name.clone(), next_value.clone(), variables, &bindings, global_variables, global_scope)?;
                stack.push(next_value);
            }
            Op::IncrementArrayPath(name, key_count, delta, post) => {
                if stack.len() < *key_count { return Err("Invalid bytecode stack".into()); }
                let raw_keys = stack.split_off(stack.len() - key_count);
                let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                warn_undefined_variable(name, variables, &bindings, global_variables, global_scope);
                let root = read_variable(name, variables, &bindings, global_variables, global_scope);
                warn_undefined_array_key(&root, &keys, global_variables);
                let old_value = root.array_get_path(&keys)?;
                let next_value = increment_value(&old_value, *delta, global_variables)?;
                let updated = root.array_set_path(&keys, next_value.clone())?;
                write_array_root(name.clone(), updated, variables, &bindings, global_variables, global_scope)?;
                stack.push(if *post { old_value } else { next_value });
            }
            Op::IncrementDynamicVariablePath(key_count, delta, post) => {
                if stack.len() < key_count + 1 { return Err("Invalid bytecode stack".into()); }
                let raw_keys = stack.split_off(stack.len() - key_count);
                let keys = raw_keys.into_iter().map(|key| key.array_key()).collect::<Result<Vec<_>, _>>()?;
                let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                warn_undefined_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                let root = read_variable(&dynamic_name, variables, &bindings, global_variables, global_scope);
                if keys.is_empty() {
                    let next_value = increment_value(&root, *delta, global_variables)?;
                    write_variable(dynamic_name, next_value.clone(), variables, &bindings, global_variables, global_scope)?;
                    stack.push(if *post { root } else { next_value });
                } else {
                    warn_undefined_array_key(&root, &keys, global_variables);
                    let old_value = root.array_get_path(&keys)?;
                    let next_value = increment_value(&old_value, *delta, global_variables)?;
                    let updated = root.array_set_path(&keys, next_value.clone())?;
                    write_array_root(dynamic_name, updated, variables, &bindings, global_variables, global_scope)?;
                    stack.push(if *post { old_value } else { next_value });
                }
            }
            Op::Echo => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let rendered = match &value {
                    Value::Object(_) => objects::stringify(&value, functions, classes, static_properties, global_variables, output, steps)?,
                    _ => value.show(),
                };
                if output.len().saturating_add(rendered.len()) > limits.max_output_bytes {
                    return Err("Execution output limit exceeded".into());
                }
                output.push_str(&rendered);
            }
            Op::Pop => { stack.pop().ok_or("Invalid bytecode stack")?; }
            Op::Dup => {
                let value = stack.last().cloned().ok_or("Invalid bytecode stack")?;
                stack.push(value);
            }
            Op::Dup2 => {
                if stack.len() < 2 { return Err("Invalid bytecode stack".into()); }
                let len = stack.len();
                let first = stack[len - 2].clone();
                let second = stack[len - 1].clone();
                stack.push(first);
                stack.push(second);
            }
            Op::IssetJump(target, property_result) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                if matches!(value, Value::Null) || (*property_result && !value.truthy()) {
                    stack.push(Value::Bool(false));
                    pc = *target;
                }
            }
            Op::CoalesceJump(target) => {
                if matches!(stack.last(), Some(Value::Null)) {
                    stack.pop().ok_or("Invalid bytecode stack")?;
                } else {
                    pc = *target;
                }
            }
            Op::Yield | Op::YieldKeyed => {
                if !allow_yield { return Err("Cannot use yield outside a generator function".into()); }
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let key = if matches!(instruction, Op::YieldKeyed) {
                    Some(stack.pop().ok_or("Invalid bytecode stack")?)
                } else {
                    None
                };
                continuation.pc = pc;
                continuation.stack = std::mem::take(&mut stack);
                continuation.iterators = std::mem::take(&mut iterators);
                continuation.yield_from = std::mem::take(&mut yield_from);
                continuation.handlers = std::mem::take(&mut handlers);
                continuation.pending_exits = std::mem::take(&mut pending_exits);
                continuation.bindings = std::mem::take(&mut bindings);
                continuation.awaiting_resume = true;
                continuation.yielded = Some((key, value));
                return Ok(Some(Value::Null));
            }
            Op::YieldFrom => {
                if !allow_yield { return Err("Cannot use yield outside a generator function".into()); }
                let iterator_key = pc - 1;
                let mut first_entry = None;
                if !yield_from.contains_key(&iterator_key) {
                    let iterable = stack.pop().ok_or("Invalid bytecode stack")?;
                    match iterable {
                        Value::Array(entries) => {
                            let mut iterator = YieldFromIterator::Array(entries.into_iter().collect(), 0);
                            first_entry = advance_yield_from(&mut iterator, Value::Null, None, functions, classes, static_properties,
                                global_variables, output, steps, &mut stack, variables, &mut bindings, global_scope, frame_id)?;
                            if first_entry.is_some() { yield_from.insert(iterator_key, iterator); }
                            else { stack.push(Value::Null); }
                        }
                        Value::Generator(generator) => {
                            if rewind_generator(&generator, functions, classes, static_properties, global_variables, output, steps)? {
                                first_entry = current_generator_entry(&generator)?;
                                yield_from.insert(iterator_key, YieldFromIterator::Generator(generator));
                            } else {
                                let iterator = YieldFromIterator::Generator(generator);
                                stack.push(yield_from_return_value(&iterator)?);
                            }
                        }
                        value @ Value::Object(_) => {
                            let mut iterable = value;
                            let mut aggregate_depth = 0usize;
                            while let Value::Object(object) = iterable.dereference() {
                                let class_name = object.borrow().class_name.clone();
                                if !is_a_type(&class_name.to_ascii_lowercase(), "iteratoraggregate", classes, &mut HashSet::new()) { break; }
                                if aggregate_depth >= 32 { return Err("IteratorAggregate nesting is too deep".into()); }
                                iterable = control::call_iterator_method(Value::Object(object), "getiterator", &mut stack,
                                    variables, &mut bindings, global_variables, global_scope, frame_id, functions, classes,
                                    static_properties, output, steps)?;
                                aggregate_depth += 1;
                            }
                            let iterator = match iterable {
                                Value::Generator(generator) => {
                                    if rewind_generator(&generator, functions, classes, static_properties, global_variables, output, steps)? {
                                        let entry = current_generator_entry(&generator)?;
                                        yield_from.insert(iterator_key, YieldFromIterator::Generator(generator));
                                        first_entry = entry;
                                        None
                                    } else {
                                        stack.push(yield_from_return_value(&YieldFromIterator::Generator(generator))?);
                                        None
                                    }
                                }
                                Value::Object(object) => {
                                    let class_name = object.borrow().class_name.clone();
                                    if class_name.eq_ignore_ascii_case("PDOStatement") {
                                        let rows = functions::pdo_iterator_rows(&Value::Object(object), variables, &mut bindings,
                                            global_variables, global_scope, frame_id, functions, classes, static_properties, output, steps)?;
                                        Some(YieldFromIterator::Array(rows, 0))
                                    } else {
                                        if !is_a_type(&class_name.to_ascii_lowercase(), "iterator", classes, &mut HashSet::new()) {
                                            return Err("yield from expects an array or Traversable object".into());
                                        }
                                        let required = ["rewind", "valid", "current", "key", "next"];
                                        if required.iter().any(|method| objects::find_method(&class_name, method, classes).is_none()) {
                                            return Err(format!("Object of type {class_name} does not provide the Iterator methods"));
                                        }
                                        let value = Value::Object(object);
                                        let _ = control::call_iterator_method(value.clone(), "rewind", &mut stack, variables,
                                            &mut bindings, global_variables, global_scope, frame_id, functions, classes,
                                            static_properties, output, steps)?;
                                        Some(YieldFromIterator::UserIterator { object: value, advance_pending: false })
                                    }
                                }
                                _ if aggregate_depth > 0 => return Err("IteratorAggregate::getIterator() must return a Traversable object".into()),
                                _ => return Err("yield from expects an array or Traversable object".into()),
                            };
                            if let Some(mut iterator) = iterator {
                                first_entry = advance_yield_from(&mut iterator, Value::Null, None, functions, classes,
                                    static_properties, global_variables, output, steps, &mut stack, variables, &mut bindings,
                                    global_scope, frame_id)?;
                                if first_entry.is_some() { yield_from.insert(iterator_key, iterator); }
                                else { stack.push(yield_from_return_value(&iterator)?); }
                            }
                        }
                        _ => return Err("yield from expects an array or Traversable object".into()),
                    }
                } else {
                    let sent = if delegated_exception.is_none() { stack.pop().unwrap_or(Value::Null) } else { Value::Null };
                    let iterator = yield_from.get_mut(&iterator_key).ok_or("Invalid yield-from iterator")?;
                    first_entry = advance_yield_from(
                        iterator, sent, delegated_exception.take(), functions, classes, static_properties,
                        global_variables, output, steps, &mut stack, variables, &mut bindings, global_scope, frame_id,
                    )?;
                }
                if let Some((key, value)) = first_entry {
                    continuation.pc = iterator_key;
                    continuation.stack = std::mem::take(&mut stack);
                    continuation.iterators = std::mem::take(&mut iterators);
                    continuation.yield_from = std::mem::take(&mut yield_from);
                    continuation.handlers = std::mem::take(&mut handlers);
                    continuation.pending_exits = std::mem::take(&mut pending_exits);
                    continuation.bindings = std::mem::take(&mut bindings);
                    continuation.awaiting_resume = true;
                    continuation.yielded = Some((Some(key), value));
                    return Ok(Some(Value::Null));
                }
                if let Some(iterator) = yield_from.remove(&iterator_key) {
                    stack.push(yield_from_return_value(&iterator)?);
                }
            }
            Op::Return => {
                if let Some(PendingExit::Exception(id)) = pending_exits.pop() { global_variables.take_thrown(id); }
                let value = stack.pop().unwrap_or(Value::Null);
                if let Some(target) = route_exit_through_finally(pc - 1, None, &mut handlers) {
                    pending_exits.push(PendingExit::Return(value));
                    pc = target;
                } else {
                    return Ok(Some(value));
                }
            }
            Op::TryStart(clauses) => handlers.push(ExceptionHandler::Catch {
                clauses: clauses.clone(),
            }),
            Op::FinallyStart(target, protected_ranges) => handlers.push(ExceptionHandler::Finally { target: *target, protected_ranges: protected_ranges.clone() }),
            Op::TryEnd => { handlers.pop().ok_or("Invalid exception-handler stack")?; }
            Op::Throw | Op::ThrowAt(_) => {
                let throw_line = match instruction { Op::ThrowAt(line) => Some(*line), _ => None };
                let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
                let value = match value {
                    Value::Object(object) if is_a_type(
                        &object.borrow().class_name.to_ascii_lowercase(),
                        "throwable",
                        classes,
                        &mut HashSet::new(),
                    ) => Value::Object(object),
                    _ => crate::exception_builtins::construct(
                        "Error",
                        &[Value::String("Can only throw objects that implement Throwable".into())],
                        classes,
                    )?,
                };
                if let (Some(line), Value::Object(object)) = (throw_line, &value) {
                    if let Some(data) = object.borrow_mut().exception.as_mut() {
                        if data.file.is_empty() { data.file = global_variables.source_file.clone(); }
                        if data.line.is_none() { data.line = Some(line); }
                    }
                }
                if let Some(PendingExit::Exception(superseded_id)) = pending_exits.pop() { global_variables.take_thrown(superseded_id); }
                global_variables.store_thrown(*steps, value);
                return Err(format!("{THROWN_VALUE_PREFIX}{steps}"));
            }
            Op::FinallyResume => {
                match pending_exits.pop().ok_or("No exit is pending in finally")? {
                    PendingExit::Exception(id) => return Err(format!("{THROWN_VALUE_PREFIX}{id}")),
                    PendingExit::Return(value) => {
                        if let Some(target) = route_exit_through_finally(pc - 1, None, &mut handlers) {
                            pending_exits.push(PendingExit::Return(value));
                            pc = target;
                        } else {
                            return Ok(Some(value));
                        }
                    }
                    PendingExit::Jump(target) => {
                        if let Some(finalizer) = route_exit_through_finally(pc - 1, Some(target), &mut handlers) {
                            pending_exits.push(PendingExit::Jump(target));
                            pc = finalizer;
                        } else {
                            pc = target;
                        }
                    }
                }
            }
            Op::Neg => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                stack.push(value.negated());
            }
            Op::Not => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                stack.push(Value::Bool(!value.truthy()));
            }
            Op::BitNot => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                stack.push(apply_bitwise_not(value));
            }
            Op::Cast(cast_type) => {
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let converted = if matches!(cast_type, crate::bytecode::CastType::String) && matches!(value.dereference(), Value::Object(_)) {
                    Value::String(objects::stringify(&value, functions, classes, static_properties, global_variables, output, steps)?)
                } else {
                    apply_cast(*cast_type, value)?
                };
                stack.push(converted);
            }
            Op::InstanceOf => {
                let expected = stack.pop().ok_or("Invalid bytecode stack")?.show().to_ascii_lowercase();
                let value = stack.pop().ok_or("Invalid bytecode stack")?;
                let matches = match value {
                    Value::Object(object) => {
                        let actual = object.borrow().class_name.to_ascii_lowercase();
                        is_a_type(&actual, &expected, classes, &mut HashSet::new())
                    }
                    Value::Generator(_) => expected == "generator",
                    _ => false,
                };
                stack.push(Value::Bool(matches));
            }
            operation => {
                let right = stack.pop().ok_or("Invalid bytecode stack")?;
                let left = stack.pop().ok_or("Invalid bytecode stack")?;
                if matches!(operation, Op::Concat) {
                    let left = objects::stringify(&left, functions, classes, static_properties, global_variables, output, steps)?;
                    let right = objects::stringify(&right, functions, classes, static_properties, global_variables, output, steps)?;
                    stack.push(Value::String(left + &right));
                } else {
                    stack.push(apply_binary(operation, left, right)?);
                }
            }
        }
        Ok(None)
        })();
        match execution {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {}
            Err(error) => {
                let error = if error.starts_with(THROWN_VALUE_PREFIX) {
                    error
                } else if let Some((class_name, message)) = runtime_error_class(&error) {
                    let exception = crate::exception_builtins::construct(
                        class_name, &[Value::String(message.to_string())], classes,
                    )?;
                    if let Value::Object(object) = &exception {
                        if let Some(data) = object.borrow_mut().exception.as_mut() {
                            if let Some(frame) = global_variables.active_trace_frames.last() {
                                if data.file.is_empty() { data.file = frame.file.clone(); }
                                if data.line.is_none() { data.line = Some(frame.line); }
                                if frame.function != "{main}" && data.trace.is_empty() {
                                    data.trace = global_variables.active_trace_frames.iter().rev()
                                        .filter(|frame| frame.function != "{main}").cloned().collect();
                                }
                            }
                        }
                    }
                    global_variables.store_thrown(*steps, exception);
                    format!("{THROWN_VALUE_PREFIX}{steps}")
                } else {
                    return Err(error);
                };
                yield_from.remove(&pc.saturating_sub(1));
                let id = error[THROWN_VALUE_PREFIX.len()..].parse::<u64>().map_err(|_| "Invalid thrown-value token")?;
                let thrown = global_variables.thrown_values.get(&id).cloned().ok_or("Missing thrown value")?;
                let mut handled = false;
                while let Some(handler) = handlers.pop() {
                    match handler {
                        ExceptionHandler::Catch { clauses } => {
                            if let Some(clause) = clauses.iter().find(|clause| exception_matches(&thrown, &clause.types, classes)) {
                                global_variables.take_thrown(id);
                                if let Some(variable) = &clause.variable {
                                    write_variable(variable.clone(), thrown.clone(), variables, &bindings, global_variables, global_scope)?;
                                }
                                pc = clause.target;
                                handled = true;
                                break;
                            }
                        }
                        ExceptionHandler::Finally { target, .. } => {
                            pending_exits.push(PendingExit::Exception(id));
                            pc = target;
                            handled = true;
                            break;
                        }
                    }
                }
                if !handled { return Err(error); }
            }
        }
    }
    Ok(None)
}

pub(super) fn is_generator_function(function: &BytecodeFunction) -> bool {
    function.instructions.iter().any(|instruction| matches!(instruction, Op::Yield | Op::YieldKeyed | Op::YieldFrom))
}

pub(super) fn new_generator(
    function: BytecodeFunction,
    variables: HashMap<String, Value>,
    bindings: HashMap<String, VariableBinding>,
    frame_id: u64,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    let function_for_validation = function.clone();
    let generator = Value::Generator(Rc::new(RefCell::new(PhpGenerator {
        function,
        variables,
        continuation: FrameContinuation::new(bindings, frame_id),
        current: None,
        next_integer_key: 0,
        started: false,
        advanced_past_first: false,
        completed: false,
        return_value: None,
    })));
    validate_return_type(generator.clone(), &function_for_validation, classes)?;
    Ok(generator)
}

pub(super) fn advance_generator(
    generator: &Rc<RefCell<PhpGenerator>>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
    resume_value: Value,
) -> Result<Option<(Value, Value)>, String> {
    advance_generator_with_exception(generator, functions, classes, static_properties, global_variables, output, steps, resume_value, None)
}

pub(super) fn advance_generator_with_exception(
    generator: &Rc<RefCell<PhpGenerator>>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
    resume_value: Value,
    resume_exception: Option<Value>,
) -> Result<Option<(Value, Value)>, String> {
    let Ok(mut generator) = generator.try_borrow_mut() else {
        return Err("Cannot resume an already running generator".into());
    };
    if generator.completed { return Ok(None); }
    if generator.current.is_some() { generator.advanced_past_first = true; }
    generator.started = true;
    generator.current = None;
    let function = generator.function.clone();
    let PhpGenerator { variables, continuation, .. } = &mut *generator;
    let result = execute_frame(
        &function.instructions, functions, classes, static_properties, variables,
        global_variables, false, &function_scope(&function), continuation,
        resume_value, resume_exception, true, output, steps,
    );
    match result {
        Ok(result) => {
            if let Some((yielded_key, value)) = generator.continuation.yielded.take() {
                let key = match yielded_key {
                    Some(key) => key.dereference(),
                    None => {
                        let key = Value::Integer(generator.next_integer_key);
                        generator.next_integer_key = generator.next_integer_key.saturating_add(1);
                        key
                    }
                };
                if let Value::Integer(key_number) = key.dereference() {
                    if key_number >= generator.next_integer_key {
                        generator.next_integer_key = key_number.saturating_add(1);
                    }
                }
                let entry = (key, value);
                generator.current = Some(entry.clone());
                Ok(Some(entry))
            } else {
                generator.completed = true;
                generator.current = None;
                generator.return_value = result;
                Ok(None)
            }
        }
        Err(error) => {
            generator.completed = true;
            generator.current = None;
            Err(error)
        }
    }
}

pub(super) fn current_generator_entry(
    generator: &Rc<RefCell<PhpGenerator>>,
) -> Result<Option<(Value, Value)>, String> {
    let generator = generator.try_borrow().map_err(|_| "Generator is already running")?;
    Ok(generator.current.clone())
}

pub(super) fn rewind_generator(
    generator: &Rc<RefCell<PhpGenerator>>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<bool, String> {
    let (started, advanced) = {
        let generator = generator.try_borrow().map_err(|_| "Generator is already running")?;
        (generator.started, generator.advanced_past_first)
    };
    if advanced { return Err("Cannot rewind a generator that was already run".into()); }
    if !started {
        advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, Value::Null)?;
    }
    Ok(current_generator_entry(generator)?.is_some())
}

pub(super) fn call_generator_method(
    generator: &Rc<RefCell<PhpGenerator>>,
    method: &str,
    arguments: Vec<Value>,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    global_variables: &mut GlobalVariables,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let method = method.to_ascii_lowercase();
    match method.as_str() {
        "current" | "key" | "valid" => {
            if !arguments.is_empty() { return Err(format!("Generator::{method}() expects no arguments")); }
            if !generator.try_borrow().map_err(|_| "Generator is already running")?.started {
                advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, Value::Null)?;
            }
            let generator = generator.try_borrow().map_err(|_| "Generator is already running")?;
            match method.as_str() {
                "current" => Ok(generator.current.as_ref().map(|(_, value)| value.clone()).unwrap_or(Value::Null)),
                "key" => Ok(generator.current.as_ref().map(|(key, _)| key.clone()).unwrap_or(Value::Null)),
                "valid" => Ok(Value::Bool(generator.current.is_some())),
                _ => unreachable!(),
            }
        }
        "rewind" => {
            if !arguments.is_empty() { return Err("Generator::rewind() expects no arguments".into()); }
            rewind_generator(generator, functions, classes, static_properties, global_variables, output, steps)?;
            Ok(Value::Null)
        }
        "next" => {
            if !arguments.is_empty() { return Err("Generator::next() expects no arguments".into()); }
            if !generator.try_borrow().map_err(|_| "Generator is already running")?.started {
                advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, Value::Null)?;
            }
            advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, Value::Null)?;
            Ok(Value::Null)
        }
        "send" => {
            if arguments.len() != 1 { return Err("Generator::send() expects exactly 1 argument".into()); }
            if !generator.try_borrow().map_err(|_| "Generator is already running")?.started {
                advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, Value::Null)?;
            }
            advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, arguments[0].clone())?;
            let generator = generator.try_borrow().map_err(|_| "Generator is already running")?;
            Ok(generator.current.as_ref().map(|(_, value)| value.clone()).unwrap_or(Value::Null))
        }
        "getreturn" => {
            if !arguments.is_empty() { return Err("Generator::getReturn() expects no arguments".into()); }
            let generator = generator.try_borrow().map_err(|_| "Generator is already running")?;
            if !generator.completed { return Err("Generator::getReturn() can only be called once the generator has finished executing".into()); }
            Ok(generator.return_value.clone().unwrap_or(Value::Null))
        }
        "throw" => {
            if arguments.len() != 1 { return Err("Generator::throw() expects exactly 1 argument".into()); }
            let exception = arguments[0].dereference();
            let Value::Object(object) = &exception else {
                return Err("Generator::throw(): Argument #1 ($exception) must be an object implementing Throwable".into());
            };
            let class_name = object.borrow().class_name.to_ascii_lowercase();
            if !is_a_type(&class_name, "throwable", classes, &mut HashSet::new()) {
                return Err(format!("Generator::throw(): {} does not implement Throwable", object.borrow().class_name));
            }
            if !generator.try_borrow().map_err(|_| "Generator is already running")?.started {
                advance_generator(generator, functions, classes, static_properties, global_variables, output, steps, Value::Null)?;
            }
            if generator.try_borrow().map_err(|_| "Generator is already running")?.completed {
                global_variables.store_thrown(*steps, exception);
                return Err(format!("{THROWN_VALUE_PREFIX}{steps}"));
            }
            advance_generator_with_exception(
                generator, functions, classes, static_properties, global_variables, output, steps, Value::Null, Some(exception),
            )?;
            let generator = generator.try_borrow().map_err(|_| "Generator is already running")?;
            Ok(generator.current.as_ref().map(|(_, value)| value.clone()).unwrap_or(Value::Null))
        }
        _ => Err(format!("Call to undefined method Generator::{method}()")),
    }
}

