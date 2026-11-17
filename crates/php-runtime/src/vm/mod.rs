//! Shared virtual-machine execution loop and runtime type checks.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    time::Instant,
};

mod arrays;
mod builtin_arguments;
mod control;
mod functions;
pub(crate) mod objects;
mod operators;
mod symbol_table;

/// Apply the VM's PHP 8 loose comparison semantics for runtime builtins.
pub(crate) fn php_loose_equal(left: &Value, right: &Value) -> bool {
    operators::loose_equal(left, right)
}

/// Apply PHP's strict identity rules for values compared by runtime builtins.
pub(crate) fn php_strict_identical(left: &Value, right: &Value) -> bool {
    operators::strict_identical(left, right)
}

pub(crate) fn intval_string(input: &str, base: u32) -> i64 {
    functions::intval_string(input, base)
}

use crate::{
    bytecode::{CompoundOperation, Op},
    ArrayKey, BytecodeClass, BytecodeFunction, ExecutionLimits, SessionValue, Value,
};
use operators::{apply_binary, apply_bitwise_not, apply_cast};

thread_local! {
    static INCLUDE_LOADER: RefCell<Option<crate::IncludeLoader>> = const { RefCell::new(None) };
    static EXECUTION_DEPTH: Cell<usize> = Cell::new(0);
    static EXECUTION_LIMITS: Cell<ExecutionLimits> = Cell::new(ExecutionLimits {
        max_instructions: 10_000_000,
        max_output_bytes: 8 * 1024 * 1024,
        max_execution_time: Some(std::time::Duration::from_secs(30)),
    });
    static EXECUTION_DEADLINE: Cell<Option<Instant>> = Cell::new(None);
}

pub(crate) fn with_include_loader<T>(
    loader: Option<crate::IncludeLoader>,
    execute: impl FnOnce() -> T,
) -> T {
    let previous = INCLUDE_LOADER.with(|current| current.replace(loader));
    struct Restore(Option<crate::IncludeLoader>);
    impl Drop for Restore {
        fn drop(&mut self) {
            INCLUDE_LOADER.with(|current| {
                current.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(previous);
    execute()
}

const MAX_EXECUTION_DEPTH: usize = 32;

struct ExecutionDepthGuard;

impl ExecutionDepthGuard {
    fn enter() -> Result<Self, String> {
        EXECUTION_DEPTH.with(|depth| {
            let current = depth.get();
            if current >= MAX_EXECUTION_DEPTH {
                return Err("Maximum call depth exceeded".into());
            }
            depth.set(current + 1);
            Ok(Self)
        })
    }
}

impl Drop for ExecutionDepthGuard {
    fn drop(&mut self) {
        EXECUTION_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

struct ExecutionLimitGuard {
    limits: ExecutionLimits,
    deadline: Option<Instant>,
}

impl ExecutionLimitGuard {
    fn enter(limits: ExecutionLimits) -> Self {
        let previous = EXECUTION_LIMITS.with(|current| {
            current.replace(ExecutionLimits {
                max_instructions: limits.max_instructions.max(1),
                max_output_bytes: limits.max_output_bytes.max(1),
                max_execution_time: limits.max_execution_time,
            })
        });
        let deadline = limits
            .max_execution_time
            .and_then(|duration| Instant::now().checked_add(duration));
        let previous_deadline = EXECUTION_DEADLINE.with(|current| current.replace(deadline));
        Self {
            limits: previous,
            deadline: previous_deadline,
        }
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
pub(super) const PHP_RUNTIME_VERSION: &str = "8.4.0";
#[derive(Clone, Debug)]
pub(super) enum VariableBinding {
    Global(String),
    Static(String),
    Reference(String),
    ArrayElement {
        storage_key: String,
        keys: Vec<crate::ArrayKey>,
    },
    ObjectProperty {
        receiver: Value,
        property: String,
    },
}

#[derive(Default)]
pub(super) struct GlobalVariables {
    pub(super) source_file: String,
    /// Request-local override for the exception trace string argument limit; `None` means PHP's 15-byte default.
    pub(super) exception_string_param_max_len: Option<usize>,
    exit_code: i32,
    exiting: bool,
    values: HashMap<String, Value>,
    aliases: HashMap<String, String>,
    /// Visible symbol order for PHP's `$GLOBALS` and global `get_defined_vars()`.
    order: Vec<String>,
    constants: HashMap<String, Value>,
    constant_attributes: HashMap<String, Vec<crate::BytecodeAttribute>>,
    thrown_values: HashMap<u64, Value>,
    http_status: Option<u16>,
    http_headers: Vec<(String, String)>,
    session_started: bool,
    session_destroyed: bool,
    next_reference_id: u64,
    json_last_error: i64,
    /// Request-local values set by putenv(); None entries mask inherited values.
    environment: HashMap<String, Option<String>>,
    /// Request-local PHP include_path override; None selects the platform default.
    include_path: Option<String>,
    /// Explicitly seeded, request-local PHP Mersenne Twister state.
    pub(super) mt19937: Option<crate::math_builtins::Mt19937>,
    diagnostics: Vec<crate::RuntimeDiagnostic>,
    output_offset: usize,
    last_error: Option<crate::RuntimeDiagnostic>,
    error_reporting: Option<i64>,
    error_suppress_scopes: Vec<ErrorSuppressScope>,
    error_handler: Option<Value>,
    error_handler_stack: Vec<(Option<Value>, i64)>,
    error_handler_mask: i64,
    pending_user_handler_diagnostics: Vec<crate::RuntimeDiagnostic>,
    handling_user_error: bool,
    /// Implicit directory handle used by readdir()/closedir() when omitted.
    pub(super) last_directory_handle: Option<Value>,
    destructor_queue: Vec<Value>,
    destructed_objects: HashSet<usize>,
    /// Active PHP source frames, updated by compiled statement line markers.
    pub(super) active_trace_frames: Vec<crate::PhpTraceFrame>,
    /// Canonical files already executed through include_once/require_once in this request.
    included_files: HashSet<String>,
    /// Declarations introduced by files loaded through runtime-selected include paths.
    dynamic_functions: HashMap<String, BytecodeFunction>,
    dynamic_classes: HashMap<String, BytecodeClass>,
    dynamic_symbol_generation: u64,
    autoload_callbacks: Vec<Value>,
    autoload_in_progress: HashSet<String>,
    autoload_extensions: Option<String>,
    /// Object handles whose readonly properties may be reinitialized during their __clone hook.
    readonly_clone_objects: Vec<usize>,
}

#[derive(Clone, Debug)]
struct ErrorSuppressScope {
    frame_id: u64,
    end_pc: usize,
    previous: Option<i64>,
}

impl GlobalVariables {
    fn push_trace_frame(
        &mut self,
        frame_scope: &str,
        has_this: bool,
        classes: &HashMap<String, BytecodeClass>,
    ) {
        let (class, function) = frame_scope
            .split_once("::")
            .map(|(class, method)| {
                let metadata = classes.get(&class.to_ascii_lowercase());
                let display_class =
                    metadata.map_or_else(|| class.to_string(), |class| class.name.clone());
                let display_method = metadata
                    .and_then(|class| class.method_display_names.get(&method.to_ascii_lowercase()))
                    .cloned()
                    .unwrap_or_else(|| method.to_string());
                (Some(display_class), display_method)
            })
            .unwrap_or((
                None,
                if frame_scope == "<main>" {
                    "{main}".into()
                } else {
                    frame_scope.to_string()
                },
            ));
        self.active_trace_frames.push(crate::PhpTraceFrame {
            file: self.source_file.clone(),
            line: 1,
            function,
            call_type: class
                .as_ref()
                .map(|_| if has_this { "->".into() } else { "::".into() }),
            class,
            arguments: Vec::new(),
        });
    }

    fn set_trace_line(&mut self, line: usize) {
        if let Some(frame) = self.active_trace_frames.last_mut() {
            frame.line = line;
        }
    }

    fn set_trace_file(&mut self, file: &str) {
        if let Some(frame) = self.active_trace_frames.last_mut() {
            frame.file = file.to_string();
        }
    }

    pub(super) fn report_diagnostic(&mut self, mut diagnostic: crate::RuntimeDiagnostic) {
        let mask = self.error_reporting.unwrap_or(32767);
        let bit = match diagnostic.level {
            crate::DiagnosticLevel::Warning => 2,
            crate::DiagnosticLevel::Deprecated => 8192,
            crate::DiagnosticLevel::Notice => 8,
            crate::DiagnosticLevel::UserWarning => 512,
            crate::DiagnosticLevel::UserDeprecated => 16384,
            crate::DiagnosticLevel::UserNotice => 1024,
            crate::DiagnosticLevel::Fatal => 256,
        };
        if let Some(frame) = self.active_trace_frames.last() {
            if diagnostic.source_file.is_none() && !frame.file.is_empty() {
                diagnostic.source_file = Some(frame.file.clone());
            }
            if diagnostic.source_line.is_none() {
                diagnostic.source_line = Some(frame.line);
            }
        }
        diagnostic.output_offset = self.output_offset;
        self.last_error = Some(diagnostic.clone());
        if diagnostic.level == crate::DiagnosticLevel::Fatal {
            self.diagnostics.push(diagnostic);
        } else if !self.handling_user_error
            && self
                .error_handler
                .as_ref()
                .is_some_and(|_| self.error_handler_mask & bit != 0)
        {
            self.pending_user_handler_diagnostics.push(diagnostic);
        } else if mask & bit != 0 {
            self.diagnostics.push(diagnostic);
        }
    }

    pub(super) fn include_path(&self) -> &str {
        self.include_path.as_deref().unwrap_or(if cfg!(windows) {
            ".;"
        } else {
            ".:/usr/share/php"
        })
    }

    pub(super) fn set_include_path(&mut self, value: String) -> String {
        let previous = self.include_path().to_string();
        self.include_path = Some(value);
        previous
    }

    pub(super) fn register_destructor_object(&mut self, object: &Value) {
        if matches!(object.dereference(), Value::Object(_)) {
            self.destructor_queue.push(object.clone());
        }
    }

    fn mark_destructed(&mut self, object: &Rc<RefCell<crate::PhpObject>>) -> bool {
        self.destructed_objects.insert(Rc::as_ptr(object) as usize)
    }

    pub(super) fn forget_collected_objects(&mut self, objects: &HashSet<usize>) {
        self.destructed_objects
            .retain(|identity| !objects.contains(identity));
    }
}

pub(super) fn increment_value(
    value: &Value,
    delta: i8,
    globals: &mut GlobalVariables,
) -> Result<Value, String> {
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
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level,
            message: message.into(),
            source_file: None,
            source_line: None,
            output_offset: 0,
        });
    }
    match &original {
        Value::Array(_) | Value::Stream(_) => {
            return Err(format!(
                "TypeError:Cannot {} {}",
                if delta > 0 { "increment" } else { "decrement" },
                original.type_name()
            ))
        }
        Value::Object(object) => {
            return Err(format!(
                "TypeError:Cannot {} object of class {}",
                if delta > 0 { "increment" } else { "decrement" },
                object.borrow().class_name
            ))
        }
        Value::Closure(_) => {
            return Err(format!(
                "TypeError:Cannot {} Closure",
                if delta > 0 { "increment" } else { "decrement" }
            ))
        }
        Value::Generator(_) => {
            return Err(format!(
                "TypeError:Cannot {} Generator",
                if delta > 0 { "increment" } else { "decrement" }
            ))
        }
        _ => {}
    }
    Ok(if matches!(original, Value::Null) && delta > 0 {
        Value::Integer(1)
    } else {
        original.incremented(delta)
    })
}

const LOCAL_ORDER_KEY: &str = "\0local-symbol-order";

/// Seed a frame's visible-name order before its argument values enter a HashMap.
pub(super) fn initialize_local_order(
    variables: &mut HashMap<String, Value>,
    names: impl IntoIterator<Item = String>,
) {
    let entries = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (crate::ArrayKey::Integer(index as i64), Value::String(name)))
        .collect::<Vec<_>>();
    variables.insert(LOCAL_ORDER_KEY.into(), Value::array(entries));
}

fn update_local_order(variables: &mut HashMap<String, Value>, name: &str, present: bool) {
    if name.starts_with('\0') {
        return;
    }
    let mut names = variables
        .get(LOCAL_ORDER_KEY)
        .map(|value| match value.dereference() {
            Value::Array(values) => values
                .iter()
                .filter_map(|(_, value)| match value.dereference() {
                    Value::String(name) => Some(name),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    names.retain(|candidate| candidate != name);
    if present {
        names.push(name.to_string());
    }
    variables.insert(
        LOCAL_ORDER_KEY.into(),
        Value::array(
            names
                .into_iter()
                .enumerate()
                .map(|(index, name)| (crate::ArrayKey::Integer(index as i64), Value::String(name)))
                .collect(),
        ),
    );
}

fn is_superglobal(name: &str) -> bool {
    matches!(
        name,
        "_GET" | "_POST" | "_COOKIE" | "_REQUEST" | "_FILES" | "_SERVER" | "_ENV" | "_SESSION"
    )
}

pub(super) fn builtin_constant(name: &str) -> Option<Value> {
    if matches!(name, "STDIN" | "STDOUT" | "STDERR") {
        let (readable, writable) = match name {
            "STDIN" => (true, false),
            "STDOUT" | "STDERR" => (false, true),
            _ => unreachable!(),
        };
        return crate::PhpStream::temporary(readable, writable)
            .ok()
            .map(|stream| Value::Stream(std::rc::Rc::new(std::cell::RefCell::new(stream))));
    }
    match name {
        "PHP_VERSION_ID" => return Some(Value::Integer(80400)),
        "PHP_VERSION" => return Some(Value::String(PHP_RUNTIME_VERSION.into())),
        "PHP_SAPI" => return Some(Value::String("cli".into())),
        "DIRECTORY_SEPARATOR" => {
            return Some(Value::String(std::path::MAIN_SEPARATOR.to_string()))
        }
        "PATH_SEPARATOR" => {
            return Some(Value::String(if cfg!(windows) { ";" } else { ":" }.into()))
        }
        "PHP_EOL" => {
            return Some(Value::String(
                if cfg!(windows) { "\r\n" } else { "\n" }.into(),
            ))
        }
        "PHP_OS" => return Some(Value::String(std::env::consts::OS.to_string())),
        "PHP_OS_FAMILY" => return Some(Value::String(std::env::consts::FAMILY.to_string())),
        "PHP_INT_MAX" => return Some(Value::Integer(i64::MAX)),
        "PHP_INT_MIN" => return Some(Value::Integer(i64::MIN)),
        "PHP_INT_SIZE" => return Some(Value::Integer(std::mem::size_of::<i64>() as i64)),
        "MT_RAND_MT19937" => return Some(Value::Integer(0)),
        "MT_RAND_PHP" => return Some(Value::Integer(1)),
        "PHP_QUERY_RFC1738" => return Some(Value::Integer(1)),
        "PHP_QUERY_RFC3986" => return Some(Value::Integer(2)),
        "PHP_OUTPUT_HANDLER_START" => return Some(Value::Integer(1)),
        "PHP_OUTPUT_HANDLER_WRITE" | "PHP_OUTPUT_HANDLER_CONT" => {
            return Some(Value::Integer(0))
        }
        "PHP_OUTPUT_HANDLER_CLEAN" => return Some(Value::Integer(2)),
        "PHP_OUTPUT_HANDLER_FLUSH" => return Some(Value::Integer(4)),
        "PHP_OUTPUT_HANDLER_FINAL" | "PHP_OUTPUT_HANDLER_END" => {
            return Some(Value::Integer(8))
        }
        "PHP_OUTPUT_HANDLER_CLEANABLE" => return Some(Value::Integer(16)),
        "PHP_OUTPUT_HANDLER_FLUSHABLE" => return Some(Value::Integer(32)),
        "PHP_OUTPUT_HANDLER_REMOVABLE" => return Some(Value::Integer(64)),
        "PHP_OUTPUT_HANDLER_STDFLAGS" => return Some(Value::Integer(112)),
        "PHP_OUTPUT_HANDLER_STARTED" => return Some(Value::Integer(4096)),
        "PHP_OUTPUT_HANDLER_DISABLED" => return Some(Value::Integer(8192)),
        "PHP_OUTPUT_HANDLER_PROCESSED" => return Some(Value::Integer(16384)),
        "FILTER_VALIDATE_INT" => return Some(Value::Integer(257)),
        "FILTER_VALIDATE_BOOL" | "FILTER_VALIDATE_BOOLEAN" => {
            return Some(Value::Integer(258))
        }
        "FILTER_VALIDATE_FLOAT" => return Some(Value::Integer(259)),
        "FILTER_VALIDATE_REGEXP" => return Some(Value::Integer(272)),
        "FILTER_VALIDATE_URL" => return Some(Value::Integer(273)),
        "FILTER_VALIDATE_EMAIL" => return Some(Value::Integer(274)),
        "FILTER_VALIDATE_IP" => return Some(Value::Integer(275)),
        "FILTER_VALIDATE_MAC" => return Some(Value::Integer(276)),
        "FILTER_VALIDATE_DOMAIN" => return Some(Value::Integer(277)),
        "FILTER_DEFAULT" | "FILTER_UNSAFE_RAW" => return Some(Value::Integer(516)),
        "FILTER_SANITIZE_STRING" => return Some(Value::Integer(513)),
        "FILTER_SANITIZE_ENCODED" => return Some(Value::Integer(514)),
        "FILTER_SANITIZE_SPECIAL_CHARS" => return Some(Value::Integer(515)),
        "FILTER_SANITIZE_EMAIL" => return Some(Value::Integer(517)),
        "FILTER_SANITIZE_URL" => return Some(Value::Integer(518)),
        "FILTER_SANITIZE_NUMBER_INT" => return Some(Value::Integer(519)),
        "FILTER_SANITIZE_NUMBER_FLOAT" => return Some(Value::Integer(520)),
        "FILTER_SANITIZE_FULL_SPECIAL_CHARS" => return Some(Value::Integer(522)),
        "FILTER_SANITIZE_ADD_SLASHES" => return Some(Value::Integer(523)),
        "FILTER_CALLBACK" => return Some(Value::Integer(1024)),
        "FILTER_FLAG_NONE" => return Some(Value::Integer(0)),
        "FILTER_REQUIRE_ARRAY" => return Some(Value::Integer(16777216)),
        "FILTER_REQUIRE_SCALAR" => return Some(Value::Integer(33554432)),
        "FILTER_FORCE_ARRAY" => return Some(Value::Integer(67108864)),
        "FILTER_NULL_ON_FAILURE" => return Some(Value::Integer(134217728)),
        "FILTER_FLAG_ALLOW_OCTAL" => return Some(Value::Integer(1)),
        "FILTER_FLAG_ALLOW_HEX" => return Some(Value::Integer(2)),
        "FILTER_FLAG_STRIP_LOW" => return Some(Value::Integer(4)),
        "FILTER_FLAG_STRIP_HIGH" => return Some(Value::Integer(8)),
        "FILTER_FLAG_ENCODE_LOW" => return Some(Value::Integer(16)),
        "FILTER_FLAG_ENCODE_HIGH" => return Some(Value::Integer(32)),
        "FILTER_FLAG_ENCODE_AMP" => return Some(Value::Integer(64)),
        "FILTER_FLAG_NO_ENCODE_QUOTES" => return Some(Value::Integer(128)),
        "FILTER_FLAG_EMPTY_STRING_NULL" => return Some(Value::Integer(256)),
        "FILTER_FLAG_STRIP_BACKTICK" => return Some(Value::Integer(512)),
        "FILTER_FLAG_ALLOW_FRACTION" => return Some(Value::Integer(4096)),
        "FILTER_FLAG_ALLOW_THOUSAND" => return Some(Value::Integer(8192)),
        "FILTER_FLAG_ALLOW_SCIENTIFIC" => return Some(Value::Integer(16384)),
        "FILTER_FLAG_SCHEME_REQUIRED" => return Some(Value::Integer(65536)),
        "FILTER_FLAG_HOST_REQUIRED" => return Some(Value::Integer(131072)),
        "FILTER_FLAG_PATH_REQUIRED" => return Some(Value::Integer(262144)),
        "FILTER_FLAG_QUERY_REQUIRED" => return Some(Value::Integer(524288)),
        "FILTER_FLAG_HOSTNAME" | "FILTER_FLAG_EMAIL_UNICODE" => {
            return Some(Value::Integer(1048576))
        }
        "FILTER_FLAG_IPV4" => return Some(Value::Integer(1048576)),
        "FILTER_FLAG_IPV6" => return Some(Value::Integer(2097152)),
        "FILTER_FLAG_NO_RES_RANGE" => return Some(Value::Integer(4194304)),
        "FILTER_FLAG_NO_PRIV_RANGE" => return Some(Value::Integer(8388608)),
        "FILTER_FLAG_GLOBAL_RANGE" => return Some(Value::Integer(268435456)),
        "PREG_PATTERN_ORDER" => return Some(Value::Integer(1)),
        "PREG_SET_ORDER" => return Some(Value::Integer(2)),
        "PREG_OFFSET_CAPTURE" => return Some(Value::Integer(256)),
        "PREG_UNMATCHED_AS_NULL" => return Some(Value::Integer(512)),
        "PREG_SPLIT_NO_EMPTY" => return Some(Value::Integer(1)),
        "PREG_SPLIT_DELIM_CAPTURE" => return Some(Value::Integer(2)),
        "PREG_SPLIT_OFFSET_CAPTURE" => return Some(Value::Integer(4)),
        "PREG_GREP_INVERT" => return Some(Value::Integer(1)),
        "PREG_NO_ERROR" => return Some(Value::Integer(0)),
        "PREG_INTERNAL_ERROR" => return Some(Value::Integer(1)),
        "PREG_BACKTRACK_LIMIT_ERROR" => return Some(Value::Integer(2)),
        "PREG_RECURSION_LIMIT_ERROR" => return Some(Value::Integer(3)),
        "PREG_BAD_UTF8_ERROR" => return Some(Value::Integer(4)),
        "PREG_BAD_UTF8_OFFSET_ERROR" => return Some(Value::Integer(5)),
        "PREG_JIT_STACKLIMIT_ERROR" => return Some(Value::Integer(6)),
        "PCRE_VERSION_MAJOR" => return Some(Value::Integer(10)),
        "PCRE_VERSION_MINOR" => return Some(Value::Integer(45)),
        "AMQP_NOPARAM" => return Some(Value::Integer(0)),
        "AMQP_DURABLE" => return Some(Value::Integer(2)),
        "AMQP_PASSIVE" => return Some(Value::Integer(4)),
        "AMQP_EXCLUSIVE" => return Some(Value::Integer(8)),
        "AMQP_AUTODELETE" => return Some(Value::Integer(16)),
        "AMQP_INTERNAL" => return Some(Value::Integer(32)),
        "AMQP_NOLOCAL" => return Some(Value::Integer(64)),
        "AMQP_AUTOACK" => return Some(Value::Integer(128)),
        "AMQP_IFEMPTY" => return Some(Value::Integer(256)),
        "AMQP_IFUNUSED" => return Some(Value::Integer(512)),
        "AMQP_MANDATORY" => return Some(Value::Integer(1024)),
        "AMQP_IMMEDIATE" => return Some(Value::Integer(2048)),
        "AMQP_MULTIPLE" => return Some(Value::Integer(4096)),
        "AMQP_NOWAIT" => return Some(Value::Integer(8192)),
        "AMQP_REQUEUE" => return Some(Value::Integer(16384)),
        "AMQP_EX_TYPE_DIRECT" => return Some(Value::Integer(1)),
        "AMQP_EX_TYPE_FANOUT" => return Some(Value::Integer(2)),
        "AMQP_EX_TYPE_TOPIC" => return Some(Value::Integer(3)),
        "AMQP_EX_TYPE_HEADERS" => return Some(Value::Integer(4)),
        "DOM_INDEX_SIZE_ERR" => return Some(Value::Integer(1)),
        "DOM_DOMSTRING_SIZE_ERR" | "DOMSTRING_SIZE_ERR" => return Some(Value::Integer(2)),
        "DOM_HIERARCHY_REQUEST_ERR" => return Some(Value::Integer(3)),
        "DOM_WRONG_DOCUMENT_ERR" => return Some(Value::Integer(4)),
        "DOM_INVALID_CHARACTER_ERR" => return Some(Value::Integer(5)),
        "DOM_NO_DATA_ALLOWED_ERR" => return Some(Value::Integer(6)),
        "DOM_NO_MODIFICATION_ALLOWED_ERR" => return Some(Value::Integer(7)),
        "DOM_NOT_FOUND_ERR" => return Some(Value::Integer(8)),
        "DOM_NOT_SUPPORTED_ERR" => return Some(Value::Integer(9)),
        "DOM_INUSE_ATTRIBUTE_ERR" => return Some(Value::Integer(10)),
        "DOM_INVALID_STATE_ERR" => return Some(Value::Integer(11)),
        "DOM_SYNTAX_ERR" => return Some(Value::Integer(12)),
        "DOM_INVALID_MODIFICATION_ERR" => return Some(Value::Integer(13)),
        "DOM_NAMESPACE_ERR" => return Some(Value::Integer(14)),
        "DOM_INVALID_ACCESS_ERR" => return Some(Value::Integer(15)),
        "DOM_VALIDATION_ERR" => return Some(Value::Integer(16)),
        "DOM_PHP_ERR" => return Some(Value::Integer(0)),
        "XML_ELEMENT_NODE" => return Some(Value::Integer(1)),
        "XML_ATTRIBUTE_NODE" => return Some(Value::Integer(2)),
        "XML_TEXT_NODE" => return Some(Value::Integer(3)),
        "XML_CDATA_SECTION_NODE" => return Some(Value::Integer(4)),
        "XML_ENTITY_REF_NODE" => return Some(Value::Integer(5)),
        "XML_ENTITY_NODE" => return Some(Value::Integer(6)),
        "XML_PI_NODE" => return Some(Value::Integer(7)),
        "XML_COMMENT_NODE" => return Some(Value::Integer(8)),
        "XML_DOCUMENT_NODE" => return Some(Value::Integer(9)),
        "XML_DOCUMENT_TYPE_NODE" => return Some(Value::Integer(10)),
        "XML_DOCUMENT_FRAG_NODE" => return Some(Value::Integer(11)),
        "XML_NOTATION_NODE" => return Some(Value::Integer(12)),
        "XML_HTML_DOCUMENT_NODE" => return Some(Value::Integer(13)),
        "XML_DTD_NODE" => return Some(Value::Integer(14)),
        "XML_ELEMENT_DECL_NODE" => return Some(Value::Integer(15)),
        "XML_ATTRIBUTE_DECL_NODE" => return Some(Value::Integer(16)),
        "XML_ENTITY_DECL_NODE" => return Some(Value::Integer(17)),
        "XML_NAMESPACE_DECL_NODE" => return Some(Value::Integer(18)),
        "XML_ERROR_NONE" => return Some(Value::Integer(0)),
        "XML_ERROR_NO_MEMORY" => return Some(Value::Integer(1)),
        "XML_ERROR_SYNTAX" => return Some(Value::Integer(2)),
        "XML_ERROR_NO_ELEMENTS" => return Some(Value::Integer(3)),
        "XML_ERROR_INVALID_TOKEN" => return Some(Value::Integer(4)),
        "XML_ERROR_UNCLOSED_TOKEN" => return Some(Value::Integer(5)),
        "XML_ERROR_PARTIAL_CHAR" => return Some(Value::Integer(6)),
        "XML_ERROR_TAG_MISMATCH" => return Some(Value::Integer(7)),
        "XML_ERROR_DUPLICATE_ATTRIBUTE" => return Some(Value::Integer(8)),
        "XML_ERROR_JUNK_AFTER_DOC_ELEMENT" => return Some(Value::Integer(9)),
        "XML_ERROR_PARAM_ENTITY_REF" => return Some(Value::Integer(10)),
        "XML_ERROR_UNDEFINED_ENTITY" => return Some(Value::Integer(11)),
        "XML_ERROR_RECURSIVE_ENTITY_REF" => return Some(Value::Integer(12)),
        "XML_ERROR_ASYNC_ENTITY" => return Some(Value::Integer(13)),
        "XML_ERROR_BAD_CHAR_REF" => return Some(Value::Integer(14)),
        "XML_ERROR_BINARY_ENTITY_REF" => return Some(Value::Integer(15)),
        "XML_ERROR_ATTRIBUTE_EXTERNAL_ENTITY_REF" => return Some(Value::Integer(16)),
        "XML_ERROR_MISPLACED_XML_PI" => return Some(Value::Integer(17)),
        "XML_ERROR_UNKNOWN_ENCODING" => return Some(Value::Integer(18)),
        "XML_ERROR_INCORRECT_ENCODING" => return Some(Value::Integer(19)),
        "XML_ERROR_UNCLOSED_CDATA_SECTION" => return Some(Value::Integer(20)),
        "XML_ERROR_EXTERNAL_ENTITY_HANDLING" => return Some(Value::Integer(21)),
        "PGSQL_CONNECTION_OK" => return Some(Value::Integer(0)),
        "PGSQL_CONNECTION_BAD" => return Some(Value::Integer(1)),
        "PGSQL_TRANSACTION_IDLE" => return Some(Value::Integer(0)),
        "PGSQL_TRANSACTION_ACTIVE" => return Some(Value::Integer(1)),
        "PGSQL_TRANSACTION_INTRANS" => return Some(Value::Integer(2)),
        "PGSQL_TRANSACTION_INERROR" => return Some(Value::Integer(3)),
        "PGSQL_TRANSACTION_UNKNOWN" => return Some(Value::Integer(4)),
        "PGSQL_EMPTY_QUERY" => return Some(Value::Integer(0)),
        "PGSQL_COMMAND_OK" => return Some(Value::Integer(1)),
        "PGSQL_TUPLES_OK" => return Some(Value::Integer(2)),
        "PGSQL_COPY_OUT" => return Some(Value::Integer(3)),
        "PGSQL_COPY_IN" => return Some(Value::Integer(4)),
        "PGSQL_BAD_RESPONSE" => return Some(Value::Integer(5)),
        "PGSQL_NONFATAL_ERROR" => return Some(Value::Integer(6)),
        "PGSQL_FATAL_ERROR" => return Some(Value::Integer(7)),
        "PGSQL_DIAG_SEVERITY" => return Some(Value::Integer(i64::from(b'S'))),
        "PGSQL_DIAG_SQLSTATE" => return Some(Value::Integer(i64::from(b'C'))),
        "PGSQL_DIAG_MESSAGE_PRIMARY" => return Some(Value::Integer(i64::from(b'M'))),
        "PGSQL_DIAG_MESSAGE_DETAIL" => return Some(Value::Integer(i64::from(b'D'))),
        "PGSQL_DIAG_MESSAGE_HINT" => return Some(Value::Integer(i64::from(b'H'))),
        "PGSQL_DIAG_STATEMENT_POSITION" => return Some(Value::Integer(i64::from(b'P'))),
        "PGSQL_DIAG_INTERNAL_POSITION" => return Some(Value::Integer(i64::from(b'p'))),
        "PGSQL_DIAG_INTERNAL_QUERY" => return Some(Value::Integer(i64::from(b'q'))),
        "PGSQL_DIAG_CONTEXT" => return Some(Value::Integer(i64::from(b'W'))),
        "PGSQL_DIAG_SOURCE_FILE" => return Some(Value::Integer(i64::from(b'F'))),
        "PGSQL_DIAG_SOURCE_LINE" => return Some(Value::Integer(i64::from(b'L'))),
        "PGSQL_DIAG_SOURCE_FUNCTION" => return Some(Value::Integer(i64::from(b'R'))),
        "PGSQL_ATTR_DISABLE_PREPARES" => return Some(Value::Integer(1000)),
        "PGSQL_STATUS_STRING" => return Some(Value::Integer(1)),
        "LOCK_NB" => return Some(Value::Integer(4)),
        "GLOB_ERR" => return Some(Value::Integer(1)),
        "GLOB_MARK" => return Some(Value::Integer(2)),
        "GLOB_NOSORT" => return Some(Value::Integer(4)),
        "GLOB_NOCHECK" => return Some(Value::Integer(16)),
        "GLOB_NOESCAPE" => return Some(Value::Integer(64)),
        "GLOB_ONLYDIR" => return Some(Value::Integer(256)),
        "GLOB_BRACE" => return Some(Value::Integer(1024)),
        "GLOB_AVAILABLE_FLAGS" => return Some(Value::Integer(1367)),
        // Stable reserved IDs for the partial tokenizer-constant surface. PHP
        // explicitly warns that parser token numbers vary across versions.
        "T_COMMENT" => return Some(Value::Integer(10000)),
        "T_DOC_COMMENT" => return Some(Value::Integer(10001)),
        "T_OPEN_TAG" => return Some(Value::Integer(10002)),
        "T_NEW" => return Some(Value::Integer(10003)),
        "T_WHITESPACE" => return Some(Value::Integer(10004)),
        "T_STRING" => return Some(Value::Integer(10005)),
        "T_STATIC" => return Some(Value::Integer(10006)),
        "T_ABSTRACT" => return Some(Value::Integer(10007)),
        "T_FINAL" => return Some(Value::Integer(10008)),
        "T_PRIVATE" => return Some(Value::Integer(10009)),
        "T_PROTECTED" => return Some(Value::Integer(10010)),
        "T_PUBLIC" => return Some(Value::Integer(10011)),
        "T_READONLY" => return Some(Value::Integer(10012)),
        "T_INCLUDE" => return Some(Value::Integer(10013)),
        "T_INCLUDE_ONCE" => return Some(Value::Integer(10014)),
        "T_EVAL" => return Some(Value::Integer(10015)),
        "T_REQUIRE" => return Some(Value::Integer(10016)),
        "T_REQUIRE_ONCE" => return Some(Value::Integer(10017)),
        "T_LOGICAL_OR" => return Some(Value::Integer(10018)),
        "T_LOGICAL_XOR" => return Some(Value::Integer(10019)),
        "T_LOGICAL_AND" => return Some(Value::Integer(10020)),
        "T_INSTANCEOF" => return Some(Value::Integer(10021)),
        "T_CLONE" => return Some(Value::Integer(10022)),
        "T_EXIT" => return Some(Value::Integer(10023)),
        "T_IF" => return Some(Value::Integer(10024)),
        "T_ELSEIF" => return Some(Value::Integer(10025)),
        "T_ELSE" => return Some(Value::Integer(10026)),
        "T_ENDIF" => return Some(Value::Integer(10027)),
        "T_ECHO" => return Some(Value::Integer(10028)),
        "T_DO" => return Some(Value::Integer(10029)),
        "T_WHILE" => return Some(Value::Integer(10030)),
        "T_ENDWHILE" => return Some(Value::Integer(10031)),
        "T_FOR" => return Some(Value::Integer(10032)),
        "T_ENDFOR" => return Some(Value::Integer(10033)),
        "T_FOREACH" => return Some(Value::Integer(10034)),
        "T_ENDFOREACH" => return Some(Value::Integer(10035)),
        "T_DECLARE" => return Some(Value::Integer(10036)),
        "T_ENDDECLARE" => return Some(Value::Integer(10037)),
        "T_AS" => return Some(Value::Integer(10038)),
        "T_TRY" => return Some(Value::Integer(10039)),
        "T_CATCH" => return Some(Value::Integer(10040)),
        "T_FINALLY" => return Some(Value::Integer(10041)),
        "T_THROW" => return Some(Value::Integer(10042)),
        "T_USE" => return Some(Value::Integer(10043)),
        "T_INSTEADOF" => return Some(Value::Integer(10044)),
        "T_GLOBAL" => return Some(Value::Integer(10045)),
        "T_VAR" => return Some(Value::Integer(10046)),
        "T_UNSET" => return Some(Value::Integer(10047)),
        "T_ISSET" => return Some(Value::Integer(10048)),
        "T_EMPTY" => return Some(Value::Integer(10049)),
        "T_CONTINUE" => return Some(Value::Integer(10050)),
        "T_GOTO" => return Some(Value::Integer(10051)),
        "T_FUNCTION" => return Some(Value::Integer(10052)),
        "T_CONST" => return Some(Value::Integer(10053)),
        "T_RETURN" => return Some(Value::Integer(10054)),
        "T_PRINT" => return Some(Value::Integer(10055)),
        "T_YIELD" => return Some(Value::Integer(10056)),
        "T_LIST" => return Some(Value::Integer(10057)),
        "T_SWITCH" => return Some(Value::Integer(10058)),
        "T_ENDSWITCH" => return Some(Value::Integer(10059)),
        "T_CASE" => return Some(Value::Integer(10060)),
        "T_DEFAULT" => return Some(Value::Integer(10061)),
        "T_BREAK" => return Some(Value::Integer(10062)),
        "T_ARRAY" => return Some(Value::Integer(10063)),
        "T_CALLABLE" => return Some(Value::Integer(10064)),
        "T_EXTENDS" => return Some(Value::Integer(10065)),
        "T_IMPLEMENTS" => return Some(Value::Integer(10066)),
        "T_NAMESPACE" => return Some(Value::Integer(10067)),
        "T_TRAIT" => return Some(Value::Integer(10068)),
        "T_INTERFACE" => return Some(Value::Integer(10069)),
        "T_CLASS" => return Some(Value::Integer(10070)),
        "T_CLASS_C" => return Some(Value::Integer(10071)),
        "T_TRAIT_C" => return Some(Value::Integer(10072)),
        "T_FUNC_C" => return Some(Value::Integer(10073)),
        "T_METHOD_C" => return Some(Value::Integer(10074)),
        "T_LINE" => return Some(Value::Integer(10075)),
        "T_FILE" => return Some(Value::Integer(10076)),
        "T_DIR" => return Some(Value::Integer(10077)),
        "T_NS_C" => return Some(Value::Integer(10078)),
        "T_HALT_COMPILER" => return Some(Value::Integer(10079)),
        "T_FN" => return Some(Value::Integer(10080)),
        "T_MATCH" => return Some(Value::Integer(10081)),
        "T_OPEN_TAG_WITH_ECHO" => return Some(Value::Integer(10082)),
        "T_NAME_FULLY_QUALIFIED" => return Some(Value::Integer(10083)),
        "T_NAME_QUALIFIED" => return Some(Value::Integer(10084)),
        "T_NAME_RELATIVE" => return Some(Value::Integer(10085)),
        "T_NS_SEPARATOR" => return Some(Value::Integer(10086)),
        "T_BAD_CHARACTER" => return Some(Value::Integer(10087)),
        "T_CLOSE_TAG" => return Some(Value::Integer(10088)),
        "T_VARIABLE" => return Some(Value::Integer(10089)),
        "T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG" => return Some(Value::Integer(10100)),
        "T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG" => return Some(Value::Integer(10101)),
        "T_AND_EQUAL" => return Some(Value::Integer(10102)),
        "T_ARRAY_CAST" => return Some(Value::Integer(10103)),
        "T_ATTRIBUTE" => return Some(Value::Integer(10104)),
        "T_BACKTICK" => return Some(Value::Integer(10105)),
        "T_BOOLEAN_AND" => return Some(Value::Integer(10106)),
        "T_BOOLEAN_OR" => return Some(Value::Integer(10107)),
        "T_BOOL_CAST" => return Some(Value::Integer(10108)),
        "T_COALESCE" => return Some(Value::Integer(10109)),
        "T_COALESCE_EQUAL" => return Some(Value::Integer(10110)),
        "T_CONCAT_EQUAL" => return Some(Value::Integer(10111)),
        "T_CONSTANT_ENCAPSED_STRING" => return Some(Value::Integer(10112)),
        "T_CURLY_OPEN" => return Some(Value::Integer(10113)),
        "T_DEC" => return Some(Value::Integer(10114)),
        "T_DIV_EQUAL" => return Some(Value::Integer(10115)),
        "T_DNUMBER" => return Some(Value::Integer(10116)),
        "T_DOLLAR_OPEN_CURLY_BRACES" => return Some(Value::Integer(10117)),
        "T_DOUBLE_ARROW" => return Some(Value::Integer(10118)),
        "T_DOUBLE_CAST" => return Some(Value::Integer(10119)),
        "T_DOUBLE_COLON" => return Some(Value::Integer(10120)),
        "T_ELLIPSIS" => return Some(Value::Integer(10121)),
        "T_END_HEREDOC" => return Some(Value::Integer(10122)),
        "T_ENUM" => return Some(Value::Integer(10123)),
        "T_ENCAPSED_AND_WHITESPACE" => return Some(Value::Integer(10124)),
        "T_INC" => return Some(Value::Integer(10125)),
        "T_INLINE_HTML" => return Some(Value::Integer(10126)),
        "T_INT_CAST" => return Some(Value::Integer(10127)),
        "T_IS_EQUAL" => return Some(Value::Integer(10128)),
        "T_IS_GREATER_OR_EQUAL" => return Some(Value::Integer(10129)),
        "T_IS_IDENTICAL" => return Some(Value::Integer(10130)),
        "T_IS_NOT_EQUAL" => return Some(Value::Integer(10131)),
        "T_IS_NOT_IDENTICAL" => return Some(Value::Integer(10132)),
        "T_IS_SMALLER_OR_EQUAL" => return Some(Value::Integer(10133)),
        "T_LNUMBER" => return Some(Value::Integer(10134)),
        "T_MINUS_EQUAL" => return Some(Value::Integer(10135)),
        "T_MOD_EQUAL" => return Some(Value::Integer(10136)),
        "T_MUL_EQUAL" => return Some(Value::Integer(10137)),
        "T_NUM_STRING" => return Some(Value::Integer(10138)),
        "T_OBJECT_CAST" => return Some(Value::Integer(10139)),
        "T_OBJECT_OPERATOR" => return Some(Value::Integer(10140)),
        "T_NULLSAFE_OBJECT_OPERATOR" => return Some(Value::Integer(10141)),
        "T_OR_EQUAL" => return Some(Value::Integer(10142)),
        "T_PAAMAYIM_NEKUDOTAYIM" => return Some(Value::Integer(10143)),
        "T_PLUS_EQUAL" => return Some(Value::Integer(10144)),
        "T_POW" => return Some(Value::Integer(10145)),
        "T_POW_EQUAL" => return Some(Value::Integer(10146)),
        "T_PRIVATE_SET" => return Some(Value::Integer(10147)),
        "T_PROPERTY_C" => return Some(Value::Integer(10148)),
        "T_PROTECTED_SET" => return Some(Value::Integer(10149)),
        "T_PUBLIC_SET" => return Some(Value::Integer(10150)),
        "T_SL" => return Some(Value::Integer(10151)),
        "T_SL_EQUAL" => return Some(Value::Integer(10152)),
        "T_SPACESHIP" => return Some(Value::Integer(10153)),
        "T_SR" => return Some(Value::Integer(10154)),
        "T_SR_EQUAL" => return Some(Value::Integer(10155)),
        "T_START_HEREDOC" => return Some(Value::Integer(10156)),
        "T_STRING_CAST" => return Some(Value::Integer(10157)),
        "T_STRING_VARNAME" => return Some(Value::Integer(10158)),
        "T_UNSET_CAST" => return Some(Value::Integer(10159)),
        "T_VOID_CAST" => return Some(Value::Integer(10160)),
        "T_XOR_EQUAL" => return Some(Value::Integer(10161)),
        "T_YIELD_FROM" => return Some(Value::Integer(10162)),
        "T_PIPE" => return Some(Value::Integer(10164)),
        "IMAGETYPE_GIF" => return Some(Value::Integer(1)),
        "IMAGETYPE_JPEG" => return Some(Value::Integer(2)),
        "IMAGETYPE_PNG" => return Some(Value::Integer(3)),
        "IMAGETYPE_SWF" => return Some(Value::Integer(4)),
        "IMAGETYPE_PSD" => return Some(Value::Integer(5)),
        "IMAGETYPE_BMP" => return Some(Value::Integer(6)),
        "IMAGETYPE_TIFF_II" => return Some(Value::Integer(7)),
        "IMAGETYPE_TIFF_MM" => return Some(Value::Integer(8)),
        "IMAGETYPE_JPC" => return Some(Value::Integer(9)),
        "IMAGETYPE_JP2" => return Some(Value::Integer(10)),
        "IMAGETYPE_JPX" => return Some(Value::Integer(11)),
        "IMAGETYPE_JB2" => return Some(Value::Integer(12)),
        "IMAGETYPE_SWC" => return Some(Value::Integer(13)),
        "IMAGETYPE_IFF" => return Some(Value::Integer(14)),
        "IMAGETYPE_WBMP" => return Some(Value::Integer(15)),
        "IMAGETYPE_XBM" => return Some(Value::Integer(16)),
        "IMAGETYPE_ICO" => return Some(Value::Integer(17)),
        "IMAGETYPE_WEBP" => return Some(Value::Integer(18)),
        "IMAGETYPE_AVIF" => return Some(Value::Integer(19)),
        "PHP_URL_SCHEME" => return Some(Value::Integer(0)),
        "PHP_URL_HOST" => return Some(Value::Integer(1)),
        "PHP_URL_PORT" => return Some(Value::Integer(2)),
        "PHP_URL_USER" => return Some(Value::Integer(3)),
        "PHP_URL_PASS" => return Some(Value::Integer(4)),
        "PHP_URL_PATH" => return Some(Value::Integer(5)),
        "PHP_URL_QUERY" => return Some(Value::Integer(6)),
        "PHP_URL_FRAGMENT" => return Some(Value::Integer(7)),
        "PHP_FLOAT_MAX" => return Some(Value::Number(f64::MAX)),
        "PHP_FLOAT_MIN" => return Some(Value::Number(f64::MIN_POSITIVE)),
        "PHP_FLOAT_EPSILON" => return Some(Value::Number(f64::EPSILON)),
        "INF" | "INFINITY" => return Some(Value::Number(f64::INFINITY)),
        "NAN" => return Some(Value::Number(f64::NAN)),
        "PHP_FLOAT_DIG" => return Some(Value::Integer(15)),
        "E_ERROR" => return Some(Value::Integer(1)),
        "E_WARNING" => return Some(Value::Integer(2)),
        "E_PARSE" => return Some(Value::Integer(4)),
        "E_NOTICE" => return Some(Value::Integer(8)),
        "E_CORE_ERROR" => return Some(Value::Integer(16)),
        "E_CORE_WARNING" => return Some(Value::Integer(32)),
        "E_COMPILE_ERROR" => return Some(Value::Integer(64)),
        "E_COMPILE_WARNING" => return Some(Value::Integer(128)),
        "E_USER_ERROR" => return Some(Value::Integer(256)),
        "E_USER_WARNING" => return Some(Value::Integer(512)),
        "E_USER_NOTICE" => return Some(Value::Integer(1024)),
        "E_STRICT" => return Some(Value::Integer(2048)),
        "E_RECOVERABLE_ERROR" => return Some(Value::Integer(4096)),
        "E_DEPRECATED" => return Some(Value::Integer(8192)),
        "E_USER_DEPRECATED" => return Some(Value::Integer(16384)),
        "E_ALL" => return Some(Value::Integer(32767)),
        "LOG_EMERG" => return Some(Value::Integer(0)),
        "LOG_ALERT" => return Some(Value::Integer(1)),
        "LOG_CRIT" => return Some(Value::Integer(2)),
        "LOG_ERR" => return Some(Value::Integer(3)),
        "LOG_WARNING" => return Some(Value::Integer(4)),
        "LOG_NOTICE" => return Some(Value::Integer(5)),
        "LOG_INFO" => return Some(Value::Integer(6)),
        "LOG_DEBUG" => return Some(Value::Integer(7)),
        "LOG_KERN" => return Some(Value::Integer(0)),
        "LOG_USER" => return Some(Value::Integer(8)),
        "LOG_MAIL" => return Some(Value::Integer(16)),
        "LOG_DAEMON" => return Some(Value::Integer(24)),
        "LOG_AUTH" => return Some(Value::Integer(32)),
        "LOG_SYSLOG" => return Some(Value::Integer(40)),
        "LOG_LPR" => return Some(Value::Integer(48)),
        "LOG_NEWS" => return Some(Value::Integer(56)),
        "LOG_UUCP" => return Some(Value::Integer(64)),
        "LOG_CRON" => return Some(Value::Integer(72)),
        "LOG_AUTHPRIV" => return Some(Value::Integer(80)),
        "LOG_FTP" => return Some(Value::Integer(88)),
        "LOG_LOCAL0" => return Some(Value::Integer(128)),
        "LOG_LOCAL1" => return Some(Value::Integer(136)),
        "LOG_LOCAL2" => return Some(Value::Integer(144)),
        "LOG_LOCAL3" => return Some(Value::Integer(152)),
        "LOG_LOCAL4" => return Some(Value::Integer(160)),
        "LOG_LOCAL5" => return Some(Value::Integer(168)),
        "LOG_LOCAL6" => return Some(Value::Integer(176)),
        "LOG_LOCAL7" => return Some(Value::Integer(184)),
        "LOG_CONS" => return Some(Value::Integer(2)),
        "LOG_NDELAY" => return Some(Value::Integer(8)),
        "LOG_ODELAY" => return Some(Value::Integer(4)),
        "LOG_NOWAIT" => return Some(Value::Integer(16)),
        "LOG_PID" => return Some(Value::Integer(1)),
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
        "SIG_DFL" => return Some(Value::Integer(0)),
        "SIG_IGN" => return Some(Value::Integer(1)),
        "SIG_ERR" => return Some(Value::Integer(-1)),
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
        "SCANDIR_SORT_ASCENDING" => return Some(Value::Integer(0)),
        "SCANDIR_SORT_DESCENDING" => return Some(Value::Integer(1)),
        "SCANDIR_SORT_NONE" => return Some(Value::Integer(2)),
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
        "SORT_NUMERIC"
        | "COUNT_RECURSIVE"
        | "CASE_UPPER"
        | "EXTR_SKIP"
        | "ARRAY_FILTER_USE_BOTH" => 1.0,
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
        // Common libcurl errors referenced by Monolog during Laravel autoload.
        "CURLE_COULDNT_RESOLVE_HOST" => 6.0,
        "CURLE_COULDNT_CONNECT" => 7.0,
        "CURLE_HTTP_NOT_FOUND" => 22.0,
        "CURLE_READ_ERROR" => 26.0,
        "CURLE_OPERATION_TIMEOUTED" => 28.0,
        "CURLE_HTTP_POST_ERROR" => 34.0,
        "CURLE_SSL_CONNECT_ERROR" => 35.0,
        "ENT_COMPAT" => 2.0,
        "ENT_QUOTES" => 3.0,
        "ENT_NOQUOTES" | "ENT_HTML401" => 0.0,
        "HTML_SPECIALCHARS" => 0.0,
        "HTML_ENTITIES" => 1.0,
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
    if name.eq_ignore_ascii_case("PHP_SAPI") {
        return Ok(globals
            .get("\0sapi_name")
            .cloned()
            .unwrap_or_else(|| Value::String("cli".into())));
    }
    if name.eq_ignore_ascii_case("__FILE__") {
        return Ok(Value::String(globals.source_file.clone()));
    }
    if name.eq_ignore_ascii_case("__DIR__") {
        return Ok(Value::String(
            std::path::Path::new(&globals.source_file)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .to_string_lossy()
                .into_owned(),
        ));
    }
    if let Some(value) = globals.constants.get(name) {
        return Ok(value.clone());
    }
    if let Some(value) = builtin_constant(name) {
        return Ok(value);
    }
    // Unqualified constants in a namespace fall back to the global namespace.
    if let Some((_, global_name)) = name.rsplit_once('\\') {
        if let Some(value) = globals.constants.get(global_name) {
            return Ok(value.clone());
        }
        if let Some(value) = builtin_constant(global_name) {
            return Ok(value);
        }
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
    if name == "GLOBALS" {
        return true;
    }
    if let Some(binding) = bindings.get(name) {
        let storage_name = match binding {
            VariableBinding::Global(name)
            | VariableBinding::Static(name)
            | VariableBinding::Reference(name) => name,
            VariableBinding::ArrayElement { storage_key, keys } => {
                return globals
                    .get(storage_key)
                    .and_then(|value| value.array_get_path(keys).ok())
                    .is_some();
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
            output_offset: 0,
        });
    }
}

pub(super) fn warn_undefined_array_key(
    root: &Value,
    keys: &[crate::ArrayKey],
    globals: &mut GlobalVariables,
) {
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
            output_offset: 0,
        });
    }
}

impl GlobalVariables {
    fn new() -> Self {
        Self::default()
    }

    fn resolve_key(&self, name: &str) -> String {
        let mut key = name.to_string();
        let mut visited = HashSet::new();
        while visited.insert(key.clone()) {
            let Some(next) = self.aliases.get(&key) else {
                break;
            };
            key = next.clone();
        }
        key
    }

    fn get(&self, key: &str) -> Option<&Value> {
        let storage_key = self.resolve_key(key);
        self.values.get(&storage_key)
    }

    fn get_named(&self, name: &str) -> Option<&Value> {
        self.get(name)
    }

    /// Materialize the named global symbol table for PHP's `$GLOBALS` array.
    fn globals_array(&self) -> Value {
        let mut names = self
            .order
            .iter()
            .filter(|name| !name.starts_with('\0') && name.as_str() != "GLOBALS")
            .cloned()
            .collect::<Vec<_>>();
        // Keep internal or legacy entries visible if they were created outside `set`.
        for name in self.values.keys().chain(self.aliases.keys()) {
            if !name.starts_with('\0') && name != "GLOBALS" && !names.contains(name) {
                names.push(name.clone());
            }
        }
        Value::array(
            names
                .into_iter()
                .map(|name| {
                    (
                        crate::ArrayKey::String(name.clone()),
                        self.get(&name)
                            .map(Value::dereference)
                            .unwrap_or(Value::Null),
                    )
                })
                .collect(),
        )
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
        let old_names = self
            .values
            .keys()
            .chain(self.aliases.keys())
            .filter(|name| !name.starts_with('\0') && name.as_str() != "GLOBALS")
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        for name in old_names {
            if !replacements
                .iter()
                .any(|(replacement, _)| replacement == &name)
            {
                self.remove_named(&name);
            }
        }
        for (name, value) in replacements {
            self.set(name, value);
        }
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
        if name == target_key {
            self.aliases.remove(&name);
        } else {
            if !self.order.contains(&name) {
                self.order.push(name.clone());
            }
            self.values.remove(&name);
            self.aliases.insert(name, target_key);
        }
    }

    /// Turn a global storage slot into a shared cell, preserving an existing cell when present.
    fn reference_cell(&mut self, key: &str) -> Rc<RefCell<Value>> {
        let storage_key = self.resolve_key(key);
        let slot = self.values.entry(storage_key).or_insert(Value::Null);
        if let Value::Reference(reference) = slot {
            return reference.clone();
        }
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

    fn store_thrown(&mut self, id: u64, value: Value) {
        self.thrown_values.insert(id, value);
    }
    fn take_thrown(&mut self, id: u64) -> Option<Value> {
        self.thrown_values.remove(&id)
    }
}

fn export_session_value(value: &Value, depth: usize, nodes: &mut usize) -> Option<SessionValue> {
    *nodes += 1;
    if depth > 32 || *nodes > 10_000 {
        return None;
    }
    let value = value.dereference();
    Some(match &value {
        Value::Null => SessionValue::Null,
        Value::Integer(value) => SessionValue::Integer(*value),
        Value::Number(value) => SessionValue::Number(*value),
        Value::String(value) => SessionValue::String(value.clone()),
        Value::Bool(value) => SessionValue::Bool(*value),
        Value::Array(values) => SessionValue::Array(
            values
                .iter()
                .map(|(key, value)| {
                    Some((key.clone(), export_session_value(value, depth + 1, nodes)?))
                })
                .collect::<Option<Vec<(ArrayKey, SessionValue)>>>()?,
        ),
        Value::Object(_)
        | Value::Closure(_)
        | Value::Generator(_)
        | Value::Stream(_)
        | Value::Reference(_) => return None,
    })
}
const THROWN_VALUE_PREFIX: &str = "\0php-thrown:";

#[derive(Debug)]
enum ExceptionHandler {
    Catch {
        clauses: Vec<crate::bytecode::CatchTarget>,
    },
    Finally {
        target: usize,
        protected_ranges: Vec<(usize, usize)>,
    },
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
    error_suppress_scopes: Vec<ErrorSuppressScope>,
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
            error_suppress_scopes: Vec::new(),
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
            if *index >= entries.len() {
                return Ok(None);
            }
            let entry = entries[*index].clone();
            *index += 1;
            Ok(Some((
                match entry.0 {
                    ArrayKey::Integer(key) => Value::Integer(key),
                    ArrayKey::String(key) => Value::String(key),
                },
                entry.1,
            )))
        }
        YieldFromIterator::Generator(generator) => {
            if let Some(exception) = thrown {
                advance_generator_with_exception(
                    generator,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                    Value::Null,
                    Some(exception),
                )
            } else {
                advance_generator(
                    generator,
                    functions,
                    classes,
                    static_properties,
                    globals,
                    output,
                    steps,
                    sent,
                )
            }
        }
        YieldFromIterator::UserIterator {
            object,
            advance_pending,
        } => {
            if *advance_pending {
                let _ = control::call_iterator_method(
                    object.clone(),
                    "next",
                    stack,
                    variables,
                    bindings,
                    globals,
                    global_scope,
                    frame_id,
                    functions,
                    classes,
                    static_properties,
                    output,
                    steps,
                )?;
                *advance_pending = false;
            }
            let valid = control::call_iterator_method(
                object.clone(),
                "valid",
                stack,
                variables,
                bindings,
                globals,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            if !valid.truthy() {
                return Ok(None);
            }
            let value = control::call_iterator_method(
                object.clone(),
                "current",
                stack,
                variables,
                bindings,
                globals,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            let key = control::call_iterator_method(
                object.clone(),
                "key",
                stack,
                variables,
                bindings,
                globals,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            *advance_pending = true;
            Ok(Some((key, value)))
        }
    }
}

fn yield_from_return_value(iterator: &YieldFromIterator) -> Result<Value, String> {
    match iterator {
        YieldFromIterator::Array(_, _) => Ok(Value::Null),
        YieldFromIterator::Generator(generator) => Ok(generator
            .try_borrow()
            .map_err(|_| "Generator is already running")?
            .return_value
            .clone()
            .unwrap_or(Value::Null)),
        YieldFromIterator::UserIterator { .. } => Ok(Value::Null),
    }
}

#[derive(Debug)]
enum YieldFromIterator {
    Array(Vec<(ArrayKey, Value)>, usize),
    Generator(Rc<RefCell<PhpGenerator>>),
    UserIterator {
        object: Value,
        advance_pending: bool,
    },
}

/// A suspended PHP generator. Its continuation is resumed by `foreach` or generator methods.
#[derive(Debug)]
pub struct PhpGenerator {
    function: BytecodeFunction,
    pub(super) by_reference: bool,
    variables: HashMap<String, Value>,
    continuation: FrameContinuation,
    current: Option<(Value, Value)>,
    next_integer_key: i64,
    started: bool,
    advanced_past_first: bool,
    completed: bool,
    return_value: Option<Value>,
}

fn route_exit_through_finally(
    source: usize,
    target: Option<usize>,
    handlers: &mut Vec<ExceptionHandler>,
) -> Option<usize> {
    let index = handlers.iter().rposition(|handler| match handler {
        ExceptionHandler::Finally {
            protected_ranges, ..
        } => {
            let source_is_protected = protected_ranges
                .iter()
                .any(|(start, end)| *start <= source && source <= *end);
            let target_is_protected = target.is_some_and(|target| {
                protected_ranges
                    .iter()
                    .any(|(start, end)| *start <= target && target <= *end)
            });
            source_is_protected && !target_is_protected
        }
        ExceptionHandler::Catch { .. } => false,
    })?;
    let ExceptionHandler::Finally { target, .. } = &handlers[index] else {
        unreachable!()
    };
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
    if name == "GLOBALS" {
        return globals.globals_array();
    }
    if let Some(binding) = bindings.get(name) {
        let storage_name = match binding {
            VariableBinding::Global(name) => name,
            VariableBinding::Static(name) | VariableBinding::Reference(name) => name,
            VariableBinding::ArrayElement { storage_key, keys } => {
                return globals
                    .get(storage_key)
                    .and_then(|value| value.array_get_path(keys).ok())
                    .unwrap_or(Value::Null);
            }
            VariableBinding::ObjectProperty { receiver, property } => {
                let Value::Object(object) = receiver else {
                    return Value::Null;
                };
                return object
                    .borrow()
                    .properties
                    .get(property)
                    .map(Value::dereference)
                    .unwrap_or(Value::Null);
            }
        };
        globals
            .get(storage_name)
            .map(Value::dereference)
            .unwrap_or(Value::Null)
    } else if global_scope || is_superglobal(name) {
        globals
            .get_named(name)
            .map(Value::dereference)
            .unwrap_or(Value::Null)
    } else {
        variables
            .get(name)
            .map(Value::dereference)
            .unwrap_or(Value::Null)
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
    if name == "GLOBALS" {
        return Err("Cannot re-assign $GLOBALS".into());
    }
    if !bindings.contains_key(&name) {
        if let Some(Value::Reference(reference)) = variables.get(&name) {
            *reference.borrow_mut() = value;
            return Ok(());
        }
    }
    if let Some(binding) = bindings.get(&name) {
        if let VariableBinding::ArrayElement { storage_key, keys } = binding {
            let current = globals.get(storage_key).cloned().unwrap_or(Value::Null);
            let updated = current.array_set_path(keys, value)?;
            globals.set(storage_key.clone(), updated);
            return Ok(());
        }
        if let VariableBinding::ObjectProperty { receiver, property } = binding {
            let Value::Object(object) = receiver else {
                return Err("Property reference receiver is not an object".into());
            };
            let mut object = object.borrow_mut();
            if let Some(Value::Reference(reference)) = object.properties.get(property) {
                *reference.borrow_mut() = value.dereference();
            } else {
                if !object.properties.contains_key(property) {
                    object.property_order.push(property.clone());
                }
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
        if !variables.contains_key(&name) {
            update_local_order(variables, &name, true);
        }
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
    if name == "GLOBALS" {
        globals.replace_from_globals_array(value)
    } else {
        write_variable(name, value, variables, bindings, globals, global_scope)
    }
}

/// Remove a variable from its active local or global storage.
pub(super) fn remove_variable(
    name: &str,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
) -> Result<(), String> {
    if name == "GLOBALS" {
        return Err("Cannot unset $GLOBALS".into());
    }
    match bindings.get(name).cloned() {
        Some(VariableBinding::Global(storage_name)) => globals.remove_named(&storage_name),
        Some(VariableBinding::Static(_))
        | Some(VariableBinding::Reference(_))
        | Some(VariableBinding::ArrayElement { .. })
        | Some(VariableBinding::ObjectProperty { .. }) => {
            bindings.remove(name);
        }
        None if global_scope || is_superglobal(name) => globals.remove_named(name),
        None => {
            variables.remove(name);
            update_local_order(variables, name, false);
        }
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
            VariableBinding::ObjectProperty { .. } => {
                return format!("\0object-property-reference:{frame_id}:{name}")
            }
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
        if global_name == "GLOBALS" {
            return Err("Cannot create a reference to $GLOBALS itself".into());
        }
        if nested_keys.is_empty() {
            let storage_key = globals.resolve_key(&global_name);
            return Ok((storage_key.clone(), globals.reference_cell(&storage_key)));
        }
        let root = globals
            .get(&global_name)
            .map(Value::dereference)
            .unwrap_or(Value::Null);
        let (updated, reference) = root.ensure_array_reference(nested_keys)?;
        globals.set(global_name, updated);
        let storage_key = globals.store_reference(reference.clone());
        return Ok((storage_key, reference));
    }
    if let Some(VariableBinding::ObjectProperty { receiver, property }) = bindings.get(name) {
        if !keys.is_empty() {
            let Value::Object(object) = receiver else {
                return Err("Only object properties can be passed by reference".into());
            };
            let mut object = object.borrow_mut();
            let current = object.properties.get(property).cloned().unwrap_or(Value::Null);
            let (updated, reference) = current.ensure_array_reference(keys)?;
            object.properties.insert(property.clone(), updated);
            let storage_key = globals.store_reference(reference.clone());
            return Ok((storage_key, reference));
        }
    }
    let (storage_key, keys) = match bindings.get(name) {
        Some(VariableBinding::ArrayElement {
            storage_key,
            keys: prefix,
        }) => {
            let mut combined = prefix.clone();
            combined.extend_from_slice(keys);
            (globals.resolve_key(storage_key), combined)
        }
        Some(VariableBinding::ObjectProperty { .. }) => unreachable!("empty object-property paths are handled by their caller"),
        _ => (
            reference_storage_key(name, variables, bindings, globals, global_scope, frame_id),
            keys.to_vec(),
        ),
    };
    if keys.is_empty() {
        return Ok((storage_key.clone(), globals.reference_cell(&storage_key)));
    }
    let root = globals
        .get(&storage_key)
        .map(Value::dereference)
        .unwrap_or(Value::Null);
    let (updated, reference) = root.ensure_array_reference(&keys)?;
    globals.set(storage_key, updated);
    let key = globals.store_reference(reference.clone());
    Ok((key, reference))
}

pub(super) fn reference_cell_for_append_path(
    name: &str,
    keys: &[ArrayKey],
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    globals: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
) -> Result<Rc<RefCell<Value>>, String> {
    if name == "GLOBALS" {
        return Err("Cannot append a reference to $GLOBALS".into());
    }
    let storage_key = reference_storage_key(name, variables, bindings, globals, global_scope, frame_id);
    let root = globals
        .get(&storage_key)
        .map(Value::dereference)
        .unwrap_or(Value::Null);
    let reference = Rc::new(RefCell::new(Value::Null));
    let updated = root.array_append_reference_path(keys, reference.clone())?;
    globals.set(storage_key, updated);
    globals.store_reference(reference.clone());
    Ok(reference)
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
    let (source_storage_key, reference) = reference_cell_for_path(
        source,
        &source_keys,
        variables,
        bindings,
        globals,
        global_scope,
        frame_id,
    )?;
    if target == "GLOBALS" {
        let Some((key, nested_keys)) = target_keys.split_first() else {
            return Err("Cannot bind a reference to $GLOBALS itself".into());
        };
        let global_name = match key {
            crate::ArrayKey::String(name) => name.clone(),
            crate::ArrayKey::Integer(index) => index.to_string(),
        };
        if global_name == "GLOBALS" {
            return Err("Cannot bind a reference to $GLOBALS itself".into());
        }
        if nested_keys.is_empty() {
            globals.bind_alias(global_name, source_storage_key);
        } else {
            let root = globals
                .get(&global_name)
                .map(Value::dereference)
                .unwrap_or(Value::Null);
            let updated = root.bind_array_reference(nested_keys, reference)?;
            globals.set(global_name, updated);
        }
        return Ok(());
    }
    let target_value = read_variable(target, variables, bindings, globals, global_scope);
    let updated = target_value
        .bind_array_reference(&target_keys, reference)
        .map_err(|error| format!("Reference target ${target}{target_keys:?}: {error}"))?;
    write_variable(
        target.to_string(),
        updated,
        variables,
        bindings,
        globals,
        global_scope,
    )
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
    let (source_key, _) = reference_cell_for_path(
        source,
        &keys,
        variables,
        bindings,
        globals,
        global_scope,
        frame_id,
    )?;
    match bindings.get(target).cloned() {
        Some(VariableBinding::Global(name)) | Some(VariableBinding::Static(name)) => {
            globals.bind_alias(name, source_key)
        }
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
        Some(
            binding @ (VariableBinding::ArrayElement { .. }
            | VariableBinding::ObjectProperty { .. }),
        ) => Some(binding.clone()),
        _ => None,
    };
    let source_key = source_element
        .as_ref()
        .map(|binding| match binding {
            VariableBinding::ArrayElement { storage_key, .. } => globals.resolve_key(storage_key),
            VariableBinding::ObjectProperty { .. } => {
                format!("\0object-property-reference:{frame_id}:{source}")
            }
            _ => unreachable!(),
        })
        .unwrap_or_else(|| {
            reference_storage_key(source, variables, bindings, globals, global_scope, frame_id)
        });
    match bindings.get(target).cloned() {
        Some(VariableBinding::Global(name)) => globals.bind_alias(name, source_key),
        Some(VariableBinding::Static(name)) => globals.bind_alias(name, source_key),
        Some(VariableBinding::Reference(_))
        | Some(VariableBinding::ArrayElement { .. })
        | Some(VariableBinding::ObjectProperty { .. }) => {
            bindings.insert(
                target.to_string(),
                source_element.unwrap_or(VariableBinding::Reference(source_key)),
            );
        }
        None if global_scope => globals.bind_alias(target.to_string(), source_key),
        None => {
            variables.remove(target);
            bindings.insert(
                target.to_string(),
                source_element.unwrap_or(VariableBinding::Reference(source_key)),
            );
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
    if let Some(variadic_index) = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic)
    {
        for index in arguments.len()..variadic_index {
            let Some(Some(default)) = function.parameter_defaults.get(index) else {
                return Err(format!(
                    "{label} expects at least {} arguments, {} given",
                    index + 1,
                    arguments.len()
                ));
            };
            arguments.push(default.materialize_parameter_default());
        }
        let extra_arguments = if arguments.len() > variadic_index {
            arguments.split_off(variadic_index)
        } else {
            Vec::new()
        };
        let packed = extra_arguments
            .into_iter()
            .enumerate()
            .map(|(index, value)| (crate::ArrayKey::Integer(index as i64), value))
            .collect();
        arguments.push(Value::array(packed));
        return Ok(arguments);
    }
    for index in arguments.len()..function.parameters.len() {
        let Some(Some(default)) = function.parameter_defaults.get(index) else {
            return Err(format!(
                "{label} expects at least {} arguments, {} given",
                index + 1,
                arguments.len()
            ));
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
    validate_return_type_in_scope(
        value,
        function,
        classes,
        function.declaring_class.as_deref(),
    )
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
    let Some(type_name) = function.return_type.as_deref() else {
        return Ok(value);
    };
    if type_name.trim().eq_ignore_ascii_case("never") {
        let function_name = if function.display_name.is_empty() {
            &function.name
        } else {
            &function.display_name
        };
        let qualified_name = function
            .declaring_class
            .as_deref()
            .map(|class| format!("{class}::{function_name}"))
            .unwrap_or_else(|| function_name.to_string());
        return Err(format!(
            "TypeError:{qualified_name}(): never-returning function must not implicitly return"
        ));
    }
    let value = if function.returns_by_reference {
        value
    } else if function.strict_types {
        coerce_int_to_float_return(value, type_name)
    } else {
        coerce_numeric_return(value, type_name)
    };
    let dereferenced = value.dereference();
    let contextual_type = resolve_class_relative_type(type_name, function, classes, called_class);
    if strict_return_value_matches_type(&dereferenced, &contextual_type, classes) {
        return Ok(value);
    }
    let label = function.declaring_class.as_deref().unwrap_or("function");
    Err(format!(
        "TypeError:{label}::{}() must return {type_name}",
        function.name
    ))
}

fn coerce_int_to_float_return(value: Value, type_name: &str) -> Value {
    let normalized = type_name
        .trim()
        .trim_start_matches('?')
        .trim()
        .to_ascii_lowercase();
    if matches!(normalized.as_str(), "float" | "double") {
        if let Value::Integer(integer) = value {
            return Value::Number(integer as f64);
        }
    }
    value
}

fn strict_return_value_matches_type(
    value: &Value,
    type_name: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> bool {
    if value_matches_type(value, type_name, classes) {
        return true;
    }
    if type_name.trim().eq_ignore_ascii_case("float")
        || type_name.trim().eq_ignore_ascii_case("double")
    {
        return matches!(value, Value::Integer(_));
    }
    if let Some(inner) = type_name.strip_prefix('?') {
        return matches!(value, Value::Null)
            || strict_return_value_matches_type(value, inner, classes);
    }
    if type_name.contains('|') {
        return type_name.split('|').any(|part| {
            strict_return_value_matches_type(value, part.trim().trim_matches(['(', ')']), classes)
        });
    }
    false
}

fn resolve_class_relative_type(
    type_name: &str,
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
    called_class: Option<&str>,
) -> String {
    let Some(declaring_class) = function.declaring_class.as_deref() else {
        return type_name.to_string();
    };
    let mut resolved = String::with_capacity(type_name.len());
    let mut token = String::new();
    let flush_token = |token: &mut String, output: &mut String| {
        if token.eq_ignore_ascii_case("self") {
            output.push_str(declaring_class);
        } else if token.eq_ignore_ascii_case("parent") {
            output.push_str(
                classes
                    .get(declaring_class)
                    .and_then(|class| class.parent.as_deref())
                    .unwrap_or("parent"),
            );
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
        let has = |name: &str| {
            type_name.split('|').any(|member| {
                member == name
                    || (name == "int" && member == "integer")
                    || (name == "float" && member == "double")
            })
        };
        return match value {
            Value::String(text) if has("int") => text
                .trim()
                .parse::<i64>()
                .map(Value::Integer)
                .unwrap_or_else(|_| {
                    if has("float") {
                        text.trim()
                            .parse::<f64>()
                            .map(Value::Number)
                            .unwrap_or(Value::String(text))
                    } else {
                        Value::String(text)
                    }
                }),
            Value::String(text) if has("float") => text
                .trim()
                .parse::<f64>()
                .map(Value::Number)
                .unwrap_or(Value::String(text)),
            Value::String(text) if has("string") => Value::String(text),
            Value::Bool(value) if has("bool") || type_name.contains("boolean") => {
                Value::Bool(value)
            }
            Value::Bool(value) if has("int") => Value::Integer(i64::from(value)),
            Value::Bool(value) if has("float") => Value::Number(if value { 1.0 } else { 0.0 }),
            value => value,
        };
    }
    match (type_name.as_str(), value) {
        ("int" | "integer", Value::Number(number))
            if number.is_finite() && number >= i64::MIN as f64 && number < -(i64::MIN as f64) =>
        {
            Value::Integer(number.trunc() as i64)
        }
        ("int" | "integer", Value::Bool(value)) => Value::Integer(i64::from(value)),
        ("int" | "integer", Value::String(text)) => text
            .trim()
            .parse::<i64>()
            .map(Value::Integer)
            .or_else(|_| {
                text.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|number| {
                        number.is_finite()
                            && *number >= i64::MIN as f64
                            && *number < -(i64::MIN as f64)
                    })
                    .map(|number| Value::Integer(number.trunc() as i64))
                    .ok_or(())
            })
            .unwrap_or(Value::String(text)),
        ("float" | "double", Value::Integer(value)) => Value::Number(value as f64),
        ("float" | "double", Value::Bool(value)) => Value::Number(if value { 1.0 } else { 0.0 }),
        ("float" | "double", Value::String(text)) => text
            .trim()
            .parse::<f64>()
            .map(Value::Number)
            .unwrap_or(Value::String(text)),
        (_, value) => value,
    }
}

/// Check supplied values against declared parameter type hints.
pub(super) fn validate_arguments(
    arguments: &[Value],
    function: &BytecodeFunction,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<(), String> {
    let variadic_index = function
        .parameter_variadic
        .iter()
        .position(|variadic| *variadic);
    for (index, (argument, expected)) in arguments.iter().zip(&function.parameter_types).enumerate()
    {
        let Some(expected) = expected else {
            continue;
        };
        if Some(index) == variadic_index {
            let Value::Array(values) = argument else {
                continue;
            };
            for (extra_index, (_, value)) in values.iter().enumerate() {
                let contextual_type =
                    resolve_class_relative_type(expected, function, classes, None);
                if !value_matches_type(value, &contextual_type, classes) {
                    let parameter = function
                        .parameters
                        .get(index)
                        .map(String::as_str)
                        .unwrap_or("?");
                    let label = function.declaring_class.as_deref().unwrap_or("function");
                    return Err(format!(
                        "Variadic argument {} (${}) passed to {}::{}() must be of type {}",
                        index + extra_index + 1,
                        parameter,
                        label,
                        function.name,
                        expected
                    ));
                }
            }
            continue;
        }
        let contextual_type = resolve_class_relative_type(expected, function, classes, None);
        if !value_matches_type(argument, &contextual_type, classes) {
            let parameter = function
                .parameters
                .get(index)
                .map(String::as_str)
                .unwrap_or("?");
            let label = function.declaring_class.as_deref().unwrap_or("function");
            return Err(format!(
                "Argument {} (${}) passed to {}::{}() must be of type {}",
                index + 1,
                parameter,
                label,
                function.name,
                expected
            ));
        }
    }
    Ok(())
}

/// Test whether a runtime value satisfies a supported PHP type hint.
fn value_matches_type(
    value: &Value,
    type_name: &str,
    classes: &HashMap<String, BytecodeClass>,
) -> bool {
    let dereferenced = value.dereference();
    let value = &dereferenced;
    if type_name.contains('|') || type_name.contains('&') {
        return type_name.split('|').any(|union_member| {
            union_member
                .split('&')
                .all(|member| value_matches_type(value, member, classes))
        });
    }
    let nullable = type_name.starts_with('?');
    let type_name = type_name.trim_start_matches('?');
    if nullable && matches!(value, Value::Null) {
        return true;
    }
    match type_name.to_ascii_lowercase().as_str() {
        "mixed" => true,
        "null" => matches!(value, Value::Null),
        "true" => matches!(value, Value::Bool(true)),
        "false" => matches!(value, Value::Bool(false)),
        "void" => matches!(value, Value::Null),
        "int" | "integer" => {
            matches!(value, Value::Integer(_))
                || matches!(value, Value::Number(number) if number.is_finite() && number.fract() == 0.0)
        }
        "float" | "double" => matches!(value, Value::Number(_) | Value::Integer(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "array" => matches!(value, Value::Array(_)),
        "closure" => matches!(value, Value::Closure(_)),
        "callable" => matches!(value, Value::Closure(_) | Value::String(_)),
        "iterable" => value_is_iterable(value, classes),
        "object" => matches!(
            value,
            Value::Object(_) | Value::Closure(_) | Value::Generator(_)
        ),
        expected => match value {
            Value::Object(object) => {
                let actual = object.borrow().class_name.to_ascii_lowercase();
                is_a_type(
                    &actual,
                    expected,
                    classes,
                    &mut std::collections::HashSet::new(),
                )
            }
            Value::Generator(_) => expected.eq_ignore_ascii_case("generator"),
            Value::Closure(_) => expected.eq_ignore_ascii_case("closure"),
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
                || is_a_type(
                    &class_name,
                    "Traversable",
                    classes,
                    &mut std::collections::HashSet::new(),
                )
        }
        _ => false,
    }
}

/// Walk class and interface metadata to test nominal runtime assignability.
pub(super) fn is_a_type(
    actual: &str,
    expected: &str,
    classes: &HashMap<String, BytecodeClass>,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    if actual.eq_ignore_ascii_case(expected) {
        return true;
    }
    if crate::exception_builtins::is_interface(actual)
        && crate::exception_builtins::is_a(actual, expected)
    {
        return true;
    }
    if expected.eq_ignore_ascii_case("Traversable")
        && matches!(
            actual.to_ascii_lowercase().as_str(),
            "iterator"
                | "iteratoraggregate"
                | "generator"
                | "pdostatement"
                | "arrayobject"
                | "arrayiterator"
                | "iteratoriterator"
                | "appenditerator"
                | "directoryiterator"
                | "filesystemiterator"
                | "recursivearrayiterator"
                | "recursiveiteratoriterator"
                | "splfileobject"
                | "filteriterator"
                | "recursivefilteriterator"
                | "callbackfilteriterator"
                | "recursivecallbackfilteriterator"
                | "parentiterator"
                | "limititerator"
                | "norewinditerator"
                | "infiniteiterator"
                | "emptyiterator"
                | "recursivecachingiterator"
                | "recursivetreeiterator"
                | "multipleiterator"
        )
    {
        return true;
    }
    if expected.eq_ignore_ascii_case("DateTimeInterface")
        && matches!(
            actual.to_ascii_lowercase().as_str(),
            "datetime" | "datetimeimmutable"
        )
    {
        return true;
    }
    if crate::exception_builtins::is_class(actual) {
        return crate::exception_builtins::is_a(actual, expected);
    }
    if let Some(parent) = builtin_parent_class(actual) {
        return parent.eq_ignore_ascii_case(expected)
            || is_a_type(parent, expected, classes, visited);
    }
    if !visited.insert(actual.to_string()) {
        return false;
    }
    let Some(class) = classes.get(actual) else {
        return false;
    };
    if expected.eq_ignore_ascii_case("stringable")
        && class_implements_to_string(actual, classes, &mut std::collections::HashSet::new())
    {
        return true;
    }
    class
        .parent
        .as_deref()
        .is_some_and(|parent| is_a_type(parent, expected, classes, visited))
        || class
            .interfaces
            .iter()
            .any(|interface| is_a_type(interface, expected, classes, visited))
}

pub(super) fn builtin_parent_class(name: &str) -> Option<&'static str> {
    match name.trim_start_matches('\\').to_ascii_lowercase().as_str() {
        "reflectionobject" | "reflectionenum" => Some("ReflectionClass"),
        "reflectionenumunitcase" => Some("ReflectionClassConstant"),
        "reflectionenumbackedcase" => Some("ReflectionEnumUnitCase"),
        _ => None,
    }
}

fn class_implements_to_string(
    actual: &str,
    classes: &HashMap<String, BytecodeClass>,
    visited: &mut std::collections::HashSet<String>,
) -> bool {
    if !visited.insert(actual.to_string()) {
        return false;
    }
    let Some(class) = classes.get(actual) else {
        return false;
    };
    class.methods.get("__tostring").is_some_and(|method| {
        !method.is_static && method.visibility == crate::MemberVisibility::Public
    }) || class
        .parent
        .as_deref()
        .is_some_and(|parent| class_implements_to_string(parent, classes, visited))
}

fn exception_matches(
    value: &Value,
    catch_types: &[String],
    classes: &HashMap<String, BytecodeClass>,
) -> bool {
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
    if message == "Cannot rewind a generator that was already run" {
        return Some(("Exception", message));
    }
    if let Some(message) = message.strip_prefix("ReflectionException:") {
        return Some(("ReflectionException", message.trim()));
    }
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
    if message.starts_with("Call to undefined function ")
        || message.starts_with("Call to undefined method ")
        || message.starts_with("Undefined constant ")
        || message.starts_with("Undefined class constant ")
        || message.starts_with("Class '") && message.ends_with("' not found")
        || message.starts_with("Class '") && message.ends_with("' has no constructor")
        || message.starts_with("Cannot instantiate interface ")
        || message.starts_with("Cannot instantiate abstract class ")
        || message.starts_with("Cannot instantiate enum ")
        || message.starts_with("Attempt to assign property on non-object")
        || message == "static:: used outside a class method"
        || message == "Value is not callable"
        || message == "Array callback must contain exactly two numeric entries"
        || message.starts_with("Cannot modify enum property ")
        || (message.starts_with("Typed property ") || message.starts_with("Typed static property "))
            && message.ends_with(" must not be accessed before initialization")
        || message.starts_with("Object of class ")
            && message.ends_with(" could not be converted to string")
    {
        return Some(("Error", message));
    }
    if message
        .to_ascii_lowercase()
        .starts_with("cannot access private member ")
        || message
            .to_ascii_lowercase()
            .starts_with("cannot access protected member ")
        || message
            .to_ascii_lowercase()
            .starts_with("cannot modify readonly property ")
        || message
            .to_ascii_lowercase()
            .starts_with("cannot indirectly modify readonly property ")
        || message.to_ascii_lowercase().starts_with("typed property ")
            && message
                .to_ascii_lowercase()
                .ends_with(" must not be referenced")
        || message
            .to_ascii_lowercase()
            .contains("cannot be passed by reference")
        || message
            .to_ascii_lowercase()
            .contains("must be a variable passed by reference")
    {
        return Some(("Error", message));
    }
    if message.contains("Division by zero") || message.contains("Modulo by zero") {
        return Some(("DivisionByZeroError", message));
    }
    if message == "Object cannot be used as an array key"
        || message == "Array cannot be used as an array key"
        || message == "Illegal offset type"
    {
        return Some(("TypeError", message));
    }
    if message.contains("Bit shift by negative number") {
        return Some(("ArithmeticError", message));
    }
    if message == "clone expects an object" {
        return Some(("TypeError", message));
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("expects exactly")
        || lower.contains("expects at least")
        || lower.contains("expects at most")
        || lower.contains("expects between")
    {
        return Some(("ArgumentCountError", message));
    }
    if lower.contains("must be of type")
        || lower.contains("must be a ")
        || lower.contains("must be an ")
    {
        return Some(("TypeError", message));
    }
    None
}

/// Execute a compiled program with fresh variables, static storage, and captured output.
pub(crate) fn execute(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    constant_attributes: &HashMap<String, Vec<crate::BytecodeAttribute>>,
    source_file: &str,
) -> Result<String, String> {
    execute_with_globals(
        code,
        functions,
        classes,
        constant_attributes,
        HashMap::new(),
        source_file,
    )
    .map(|result| result.output)
}

pub(crate) fn execute_with_globals(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    constant_attributes: &HashMap<String, Vec<crate::BytecodeAttribute>>,
    predefined_globals: HashMap<String, Value>,
    source_file: &str,
) -> Result<crate::ExecutionResult, String> {
    execute_with_globals_and_limits(
        code,
        functions,
        classes,
        constant_attributes,
        predefined_globals,
        ExecutionLimits::default(),
        source_file,
    )
}

pub(crate) fn execute_with_globals_and_limits(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    constant_attributes: &HashMap<String, Vec<crate::BytecodeAttribute>>,
    predefined_globals: HashMap<String, Value>,
    limits: ExecutionLimits,
    source_file: &str,
) -> Result<crate::ExecutionResult, String> {
    execute_with_globals_and_limits_and_diagnostics(
        code,
        functions,
        classes,
        constant_attributes,
        predefined_globals,
        limits,
        source_file,
        Vec::new(),
    )
}

pub(crate) fn execute_with_globals_and_limits_and_diagnostics(
    code: &[Op],
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    constant_attributes: &HashMap<String, Vec<crate::BytecodeAttribute>>,
    predefined_globals: HashMap<String, Value>,
    limits: ExecutionLimits,
    source_file: &str,
    initial_diagnostics: Vec<crate::RuntimeDiagnostic>,
) -> Result<crate::ExecutionResult, String> {
    let _database_session_guard = crate::pdo::ExecutionGuard::enter();
    let _limits_guard = ExecutionLimitGuard::enter(limits);
    let mut variables = HashMap::<String, Value>::new();
    let mut global_variables = GlobalVariables::new();
    global_variables.constant_attributes = constant_attributes.clone();
    global_variables.source_file = source_file.to_string();
    if let Some(Value::String(include_path)) = predefined_globals.get("\0php_include_path") {
        global_variables.include_path = Some(include_path.clone());
    }
    if let Some(Value::Integer(value)) = predefined_globals.get("\0zend.exception_string_param_max_len") {
        if (0..=1_000_000).contains(value) {
            global_variables.exception_string_param_max_len = Some(*value as usize);
        }
    }
    for diagnostic in initial_diagnostics {
        global_variables.report_diagnostic(diagnostic);
    }
    for name in [
        "_GET", "_POST", "_COOKIE", "_REQUEST", "_FILES", "_SERVER", "_ENV", "_SESSION",
    ] {
        global_variables.ensure(name.to_string(), Value::array(Vec::new()));
    }
    for (name, value) in predefined_globals {
        global_variables.set(name, value);
    }
    // Static properties persist across calls inside this execution, but not across program runs.
    let mut static_properties = classes
        .iter()
        .flat_map(|(class_name, class)| {
            class
                .static_properties
                .iter()
                .filter_map(move |(name, property)| {
                    property
                        .default
                        .clone()
                        .or_else(|| property.type_name.is_none().then_some(Value::Null))
                        .map(|value| ((class_name.clone(), name.clone()), value))
                })
        })
        .collect::<StaticProperties>();
    let mut output = String::new();
    let mut steps = 0_u64;
    let execution = execute_code(
        code,
        functions,
        classes,
        &mut static_properties,
        &mut variables,
        &mut global_variables,
        true,
        "<main>",
        HashMap::new(),
        &mut output,
        &mut steps,
    );
    let destructors = objects::run_registered_destructors(
        functions,
        classes,
        &mut static_properties,
        &mut global_variables,
        &mut output,
        &mut steps,
    );
    match execution {
        Ok(_) => {
            destructors?;
            let mut session_nodes = 0;
            let session_data = global_variables
                .get_named("_SESSION")
                .and_then(|value| export_session_value(value, 0, &mut session_nodes));
            let session_started = global_variables.session_started;
            let session_destroyed = global_variables.session_destroyed;
            let session_id = global_variables
                .get("\0session_id")
                .map(Value::show)
                .unwrap_or_default();
            Ok(crate::ExecutionResult {
                output,
                exit_code: global_variables.exit_code,
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
            let message = id
                .and_then(|id| global_variables.take_thrown(id))
                .map(|value| match value {
                    Value::Object(object) => {
                        let object = object.borrow();
                        object
                            .exception
                            .as_ref()
                            .map(|exception| {
                                format!("Uncaught {}: {}", object.class_name, exception.message)
                            })
                            .unwrap_or_else(|| {
                                format!("Uncaught exception: Object({})", object.class_name)
                            })
                    }
                    value => format!("Uncaught exception: {}", value.show()),
                })
                .unwrap_or_else(|| "Uncaught exception: unknown value".into());
            Err(message)
        }
        Err(error) => {
            let _ = destructors;
            Err(error)
        }
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
    if let Some(frame) = global_variables.active_trace_frames.last_mut() {
        let callable = if let Some((class_name, method_name)) = frame_scope.split_once("::") {
            classes
                .get(&class_name.to_ascii_lowercase())
                .and_then(|class| class.methods.get(&method_name.to_ascii_lowercase()))
        } else {
            functions.get(&frame_scope.to_ascii_lowercase())
        };
        if let Some(callable) = callable {
            let visible_argument_count = variables
                .get("\0func_argument_count")
                .map(|value| value.number() as usize)
                .unwrap_or(callable.parameters.len());
            let variadic_index = callable
                .parameter_variadic
                .iter()
                .position(|variadic| *variadic);
            for (index, name) in callable.parameters.iter().enumerate() {
                let Some(value) = variables.get(name.trim_start_matches('$')) else {
                    continue;
                };
                if Some(index) == variadic_index {
                    if let Value::Array(values) = value.dereference() {
                        frame
                            .arguments
                            .extend(
                                values
                                    .iter()
                                    .take(visible_argument_count.saturating_sub(index))
                                    .map(|(_, value)| value.dereference()),
                            );
                        continue;
                    }
                }
                if index < visible_argument_count {
                    frame.arguments.push(value.dereference());
                }
            }
            if variadic_index.is_none() {
                if let Some(Value::Array(extra_arguments)) =
                    variables.get("\0func_extra_arguments").map(Value::dereference)
                {
                    frame.arguments.extend(
                        extra_arguments
                            .iter()
                            .take(visible_argument_count.saturating_sub(callable.parameters.len()))
                            .map(|(_, value)| value.dereference()),
                    );
                }
            }
        }
    }
    let mut continuation = FrameContinuation::new(initial_bindings, *steps);
    let result = execute_frame(
        code,
        functions,
        classes,
        static_properties,
        variables,
        global_variables,
        global_scope,
        frame_scope,
        &mut continuation,
        Value::Null,
        None,
        false,
        false,
        output,
        steps,
    );
    global_variables.active_trace_frames.pop();
    result
}

pub(super) fn execute_dynamic_module(
    module: crate::BytecodeProgram,
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<Value, String> {
    let mut declarations_added = false;
    for (name, function) in module.functions {
        if !functions.contains_key(&name) && !global_variables.dynamic_functions.contains_key(&name)
        {
            global_variables.dynamic_functions.insert(name, function);
            declarations_added = true;
        }
    }
    for (name, class) in module.classes {
        if !classes.contains_key(&name) && !global_variables.dynamic_classes.contains_key(&name) {
            global_variables.dynamic_classes.insert(name, class);
            declarations_added = true;
        }
    }
    if declarations_added {
        global_variables.dynamic_symbol_generation += 1;
    }
    let mut nested = FrameContinuation::new(std::mem::take(bindings), *steps);
    global_variables.push_trace_frame("<include>", variables.contains_key("this"), classes);
    global_variables.set_trace_file(&module.source_file);
    let result = execute_frame(
        &module.instructions,
        functions,
        classes,
        static_properties,
        variables,
        global_variables,
        global_scope,
        &module.source_file,
        &mut nested,
        Value::Null,
        None,
        false,
        false,
        output,
        steps,
    );
    global_variables.active_trace_frames.pop();
    *bindings = nested.bindings;
    result.map(|value| value.unwrap_or(Value::Integer(1)))
}

fn dispatch_pending_error_handlers(
    variables: &mut HashMap<String, Value>,
    bindings: &mut HashMap<String, VariableBinding>,
    global_variables: &mut GlobalVariables,
    global_scope: bool,
    frame_id: u64,
    functions: &HashMap<String, BytecodeFunction>,
    classes: &HashMap<String, BytecodeClass>,
    static_properties: &mut StaticProperties,
    output: &mut String,
    steps: &mut u64,
) -> Result<(), String> {
    while !global_variables.pending_user_handler_diagnostics.is_empty() {
        let diagnostic = global_variables.pending_user_handler_diagnostics.remove(0);
        let Some(handler) = global_variables.error_handler.clone() else {
            continue;
        };
        let error_type = match diagnostic.level {
            crate::DiagnosticLevel::Warning => 2,
            crate::DiagnosticLevel::Deprecated => 8192,
            crate::DiagnosticLevel::Notice => 8,
            crate::DiagnosticLevel::UserWarning => 512,
            crate::DiagnosticLevel::UserDeprecated => 16384,
            crate::DiagnosticLevel::UserNotice => 1024,
            crate::DiagnosticLevel::Fatal => 256,
        };
        let arguments = vec![
            Value::Integer(error_type),
            Value::String(diagnostic.message.clone()),
            Value::String(
                diagnostic
                    .source_file
                    .clone()
                    .unwrap_or_else(|| "Command line code".into()),
            ),
            Value::Integer(diagnostic.source_line.unwrap_or(0) as i64),
        ];
        global_variables.handling_user_error = true;
        let outcome = (|| {
            let (instruction, mut stack) =
                functions::callback_call_args(handler, arguments, false)?;
            functions::execute_instruction(
                &instruction,
                &mut stack,
                variables,
                bindings,
                global_variables,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )?;
            Ok::<bool, String>(stack.pop().is_some_and(|value| value.truthy()))
        })();
        global_variables.handling_user_error = false;
        let handled = outcome?;
        let mask = global_variables.error_reporting.unwrap_or(32767);
        let bit = match diagnostic.level {
            crate::DiagnosticLevel::Warning => 2,
            crate::DiagnosticLevel::Deprecated => 8192,
            crate::DiagnosticLevel::Notice => 8,
            crate::DiagnosticLevel::UserWarning => 512,
            crate::DiagnosticLevel::UserDeprecated => 16384,
            crate::DiagnosticLevel::UserNotice => 1024,
            crate::DiagnosticLevel::Fatal => 256,
        };
        if !handled && mask & bit != 0 {
            global_variables.diagnostics.push(diagnostic);
        }
    }
    Ok(())
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
    yield_by_reference: bool,
    output: &mut String,
    steps: &mut u64,
) -> Result<Option<Value>, String> {
    let frame_id = continuation.frame_id;
    let result = execute_frame_inner(
        code,
        functions,
        classes,
        static_properties,
        variables,
        global_variables,
        global_scope,
        frame_scope,
        continuation,
        resume_value,
        resume_exception,
        allow_yield,
        yield_by_reference,
        output,
        steps,
    );
    if continuation.yielded.is_none() {
        restore_error_suppress_scopes(global_variables, frame_id);
        continuation.error_suppress_scopes.clear();
    }
    result
}

fn restore_error_suppress_scopes(global_variables: &mut GlobalVariables, frame_id: u64) {
    while global_variables
        .error_suppress_scopes
        .last()
        .is_some_and(|scope| scope.frame_id == frame_id)
    {
        if let Some(scope) = global_variables.error_suppress_scopes.pop() {
            global_variables.error_reporting = scope.previous;
        }
    }
}

fn restore_one_error_suppress_scope(global_variables: &mut GlobalVariables) {
    if let Some(scope) = global_variables.error_suppress_scopes.pop() {
        global_variables.error_reporting = scope.previous;
    }
}

fn suspend_error_suppress_scopes(
    global_variables: &mut GlobalVariables,
    frame_id: u64,
    continuation: &mut FrameContinuation,
) {
    let mut scopes = Vec::new();
    while global_variables
        .error_suppress_scopes
        .last()
        .is_some_and(|scope| scope.frame_id == frame_id)
    {
        if let Some(scope) = global_variables.error_suppress_scopes.pop() {
            global_variables.error_reporting = scope.previous;
            scopes.push(scope);
        }
    }
    scopes.reverse();
    continuation.error_suppress_scopes = scopes;
}

fn execute_frame_inner(
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
    yield_by_reference: bool,
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
    let mut active_functions: Option<HashMap<String, BytecodeFunction>> = None;
    let mut active_classes: Option<HashMap<String, BytecodeClass>> = None;
    let mut seen_symbol_generation = u64::MAX;
    if continuation.awaiting_resume {
        if let Some(exception) = resume_exception {
            if matches!(code.get(pc), Some(Op::YieldFrom))
                && matches!(yield_from.get(&pc), Some(YieldFromIterator::Generator(_)))
            {
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
    for scope in std::mem::take(&mut continuation.error_suppress_scopes) {
        global_variables.error_reporting = Some(scope.previous.unwrap_or(32767) & 4437);
        global_variables.error_suppress_scopes.push(scope);
    }
    continuation.yielded = None;

    while pc < code.len() && !global_variables.exiting {
        while global_variables
            .error_suppress_scopes
            .last()
            .is_some_and(|scope| scope.frame_id == frame_id && pc > scope.end_pc)
        {
            restore_one_error_suppress_scope(global_variables);
        }
        if seen_symbol_generation != global_variables.dynamic_symbol_generation {
            if global_variables.dynamic_symbol_generation != 0 {
                let mut combined_functions = functions.clone();
                combined_functions.extend(global_variables.dynamic_functions.clone());
                let mut combined_classes = classes.clone();
                combined_classes.extend(global_variables.dynamic_classes.clone());
                active_functions = Some(combined_functions);
                active_classes = Some(combined_classes);
            }
            seen_symbol_generation = global_variables.dynamic_symbol_generation;
        }
        let functions = active_functions.as_ref().unwrap_or(functions);
        let classes = active_classes.as_ref().unwrap_or(classes);
        global_variables.output_offset = output.len();
        // A shared budget also bounds recursive calls and callback execution.
        *steps += 1;
        if *steps > limits.max_instructions {
            return Err("Execution instruction limit exceeded".into());
        }
        if EXECUTION_DEADLINE.with(|deadline| {
            deadline
                .get()
                .is_some_and(|deadline| Instant::now() >= deadline)
        }) {
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
        let mut execution = (|| -> Result<Option<Value>, String> {
            if let Op::Jump(target) = instruction {
                if let Some(finalizer) =
                    route_exit_through_finally(pc - 1, Some(*target), &mut handlers)
                {
                    pending_exits.push(PendingExit::Jump(*target));
                    pc = finalizer;
                    return Ok(None);
                }
            }
            if arrays::execute_instruction(
                instruction,
                &mut stack,
                variables,
                &mut bindings,
                global_variables,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )? {
                return Ok(None);
            }
            if control::execute_instruction(
                instruction,
                &mut stack,
                &mut iterators,
                variables,
                &mut bindings,
                global_variables,
                global_scope,
                frame_id,
                &mut pc,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )? {
                return Ok(None);
            }
            if objects::execute_instruction(
                instruction,
                &mut stack,
                variables,
                &mut bindings,
                global_variables,
                global_scope,
                frame_id,
                frame_scope,
                functions,
                classes,
                static_properties,
                output,
                steps,
                &mut pc,
            )? {
                return Ok(None);
            }
            if functions::execute_instruction(
                instruction,
                &mut stack,
                variables,
                &mut bindings,
                global_variables,
                global_scope,
                frame_id,
                functions,
                classes,
                static_properties,
                output,
                steps,
            )? {
                return Ok(None);
            }
            match instruction {
                Op::ErrorSuppressStart(end_pc) => {
                    let previous = global_variables.error_reporting;
                    global_variables.error_reporting =
                        Some(global_variables.error_reporting.unwrap_or(32767) & 4437);
                    global_variables.error_suppress_scopes.push(ErrorSuppressScope {
                        frame_id,
                        end_pc: *end_pc,
                        previous,
                    });
                }
                Op::ErrorSuppressEnd => {
                    let scope = global_variables
                        .error_suppress_scopes
                        .pop()
                        .filter(|scope| scope.frame_id == frame_id)
                        .ok_or("Invalid error-suppression scope")?;
                    global_variables.error_reporting = scope.previous;
                }
                Op::ReportDiagnostic(level, message) => {
                    global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                        level: *level,
                        message: message.clone(),
                        source_file: None,
                        source_line: None,
                        output_offset: output.len(),
                    });
                }
                Op::Include(mode) => {
                    let requested = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let current_file = global_variables
                        .active_trace_frames
                        .last()
                        .map(|frame| frame.file.as_str())
                        .unwrap_or(&global_variables.source_file)
                        .to_string();
                    let include_path = global_variables.include_path().to_string();
                    let loader = INCLUDE_LOADER.with(|loader| loader.borrow().clone());
                    let Some(loader) = loader else {
                        return Err(
                            "dynamic include requires a source loader; use compile_file()".into(),
                        );
                    };
                    let module = match loader(&requested, &current_file, &include_path) {
                        Ok(module) => module,
                        Err(error) => {
                            let (source_file, source_line) = global_variables
                                .active_trace_frames
                                .last()
                                .map(|frame| (Some(frame.file.clone()), Some(frame.line)))
                                .unwrap_or((None, None));
                            let operation = match mode {
                                crate::bytecode::IncludeMode::Include => "include",
                                crate::bytecode::IncludeMode::IncludeOnce => "include_once",
                                crate::bytecode::IncludeMode::Require => "require",
                                crate::bytecode::IncludeMode::RequireOnce => "require_once",
                            };
                            global_variables.report_diagnostic(crate::RuntimeDiagnostic {
                                level: crate::DiagnosticLevel::Warning,
                                message: format!(
                                    "{operation}({requested}): Failed to open stream: {error}"
                                ),
                                source_file,
                                source_line,
                                output_offset: output.len(),
                            });
                            if matches!(
                                mode,
                                crate::bytecode::IncludeMode::Include
                                    | crate::bytecode::IncludeMode::IncludeOnce
                            ) {
                                stack.push(Value::Bool(false));
                                return Ok(None);
                            }
                            let message = format!("Failed opening required '{requested}' (include_path='{include_path}')");
                            let mut throwable = crate::exception_builtins::construct(
                                "Error",
                                &[Value::String(message)],
                                classes,
                            )?;
                            if let (Some(frame), Value::Object(object)) =
                                (global_variables.active_trace_frames.last(), &mut throwable)
                            {
                                if let Some(data) = object.borrow_mut().exception.as_mut() {
                                    data.file = frame.file.clone();
                                    data.line = Some(frame.line);
                                }
                            }
                            global_variables.store_thrown(*steps, throwable);
                            return Err(format!("{THROWN_VALUE_PREFIX}{steps}"));
                        }
                    };
                    let canonical = module.source_file.clone();
                    let already_included = global_variables.included_files.contains(&canonical);
                    if matches!(
                        mode,
                        crate::bytecode::IncludeMode::IncludeOnce
                            | crate::bytecode::IncludeMode::RequireOnce
                    ) && already_included
                    {
                        stack.push(Value::Bool(true));
                        return Ok(None);
                    }
                    global_variables.included_files.insert(canonical);
                    stack.push(execute_dynamic_module(
                        module,
                        variables,
                        &mut bindings,
                        global_variables,
                        global_scope,
                        functions,
                        classes,
                        static_properties,
                        output,
                        steps,
                    )?);
                }
                Op::SetLine(line) => global_variables.set_trace_line(*line),
                Op::SetFile(file) => {
                    global_variables.set_trace_file(file);
                }
                Op::MarkIncluded(file) => {
                    global_variables.included_files.insert(file.clone());
                }
                Op::Const(value) => stack.push(value.clone()),
                Op::Exit(argument_name, strict_types) => {
                    if argument_name
                        .as_deref()
                        .is_some_and(|name| name != "status")
                    {
                        return Err(format!(
                            "Error:Unknown named parameter ${}",
                            argument_name.as_deref().unwrap_or_default()
                        ));
                    }
                    let status = stack.pop().unwrap_or(Value::Integer(0)).dereference();
                    functions::apply_exit_status(
                        status,
                        output,
                        global_variables,
                        *strict_types,
                        functions,
                        classes,
                        static_properties,
                        steps,
                    )?;
                }
                Op::LoadConstant(name) => stack.push(read_constant(name, global_variables)?),
                Op::DynamicClassName => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let name = match value {
                        Value::Object(object) => object.borrow().class_name.clone(),
                        Value::String(name) => name,
                        other => other.show(),
                    };
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
                    assign_reference(
                        target,
                        source,
                        variables,
                        &mut bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )?;
                }
                Op::BindReturnedReference(target) => {
                    let Value::Reference(reference) =
                        stack.pop().ok_or("Invalid bytecode stack")?
                    else {
                        return Err(
                            "Only a reference-returning function can be assigned by reference"
                                .into(),
                        );
                    };
                    let source_key = global_variables.store_reference(reference);
                    match bindings.get(target).cloned() {
                        Some(VariableBinding::Global(name))
                        | Some(VariableBinding::Static(name)) => {
                            global_variables.bind_alias(name, source_key)
                        }
                        _ if global_scope => {
                            global_variables.bind_alias(target.clone(), source_key)
                        }
                        _ => {
                            variables.remove(target);
                            bindings.insert(target.clone(), VariableBinding::Reference(source_key));
                        }
                    }
                }
                Op::BindReturnedReferenceExpr(target) => {
                    let Value::Reference(reference) =
                        stack.pop().ok_or("Invalid bytecode stack")?
                    else {
                        return Err(
                            "Only a reference-returning function can be assigned by reference"
                                .into(),
                        );
                    };
                    let source_key = global_variables.store_reference(reference.clone());
                    match bindings.get(target).cloned() {
                        Some(VariableBinding::Global(name))
                        | Some(VariableBinding::Static(name)) => {
                            global_variables.bind_alias(name, source_key)
                        }
                        _ if global_scope => {
                            global_variables.bind_alias(target.clone(), source_key)
                        }
                        _ => {
                            variables.remove(target);
                            bindings.insert(target.clone(), VariableBinding::Reference(source_key));
                        }
                    }
                    stack.push(reference.borrow().clone().dereference());
                }
                Op::LoadReference(source, key_count) => {
                    if stack.len() < *key_count {
                        return Err("Invalid bytecode stack".into());
                    }
                    let raw_keys = stack.split_off(stack.len() - key_count);
                    let keys = raw_keys
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    let (_, reference) = reference_cell_for_path(
                        source,
                        &keys,
                        variables,
                        &mut bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )?;
                    stack.push(Value::Reference(reference));
                }
                Op::AssignArrayReference(target, source, key_count) => {
                    let values = stack.split_off(stack.len().saturating_sub(*key_count));
                    let keys = values
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    assign_array_reference(
                        target,
                        source,
                        keys,
                        variables,
                        &mut bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )?;
                }
                Op::BindArrayReference(target, target_key_count, source, source_key_count) => {
                    let source_values =
                        stack.split_off(stack.len().saturating_sub(*source_key_count));
                    let source_keys = source_values
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    let target_values =
                        stack.split_off(stack.len().saturating_sub(*target_key_count));
                    let target_keys = target_values
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    bind_array_reference(
                        target,
                        target_keys.clone(),
                        source,
                        source_keys,
                        variables,
                        &mut bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )
                    .map_err(|error| {
                        format!("Cannot bind reference to ${target}{target_keys:?}: {error}")
                    })?;
                }
                Op::BindArrayReturnedReference(target, target_key_count) => {
                    let Value::Reference(reference) =
                        stack.pop().ok_or("Invalid bytecode stack")?
                    else {
                        return Err(
                            "Only a reference-returning property can be bound to an array element"
                                .into(),
                        );
                    };
                    let raw_keys = stack.split_off(stack.len().saturating_sub(*target_key_count));
                    let keys = raw_keys
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    let value =
                        read_variable(target, variables, &bindings, global_variables, global_scope);
                    let updated =
                        value
                            .bind_array_reference(&keys, reference)
                            .map_err(|error| {
                                format!("Cannot bind reference to ${target}{keys:?}: {error}")
                            })?;
                    write_variable(
                        target.clone(),
                        updated,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                }
                Op::BindDynamicPropertyReference => {
                    let Value::Reference(reference) =
                        stack.pop().ok_or("Invalid bytecode stack")?
                    else {
                        return Err(
                            "Only a reference-returning property can be assigned by reference"
                                .into(),
                        );
                    };
                    let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
                    let Value::Object(object) = receiver else {
                        return Err(
                            "Cannot assign a reference to a property on a non-object".into()
                        );
                    };
                    let mut object = object.borrow_mut();
                    if !object.properties.contains_key(&property) {
                        object.property_order.push(property.clone());
                    }
                    object
                        .properties
                        .insert(property, Value::Reference(reference));
                }
                Op::BindDynamicPropertyArrayReference(
                    target_key_count,
                    source,
                    source_key_count,
                    access_scope,
                    in_constructor,
                ) => {
                    let key_count = target_key_count + source_key_count;
                    if stack.len() < key_count + 2 {
                        return Err("Invalid bytecode stack".into());
                    }
                    let raw_keys = stack.split_off(stack.len() - key_count);
                    let target_keys = raw_keys[..*target_key_count]
                        .iter()
                        .map(Value::array_key)
                        .collect::<Result<Vec<_>, _>>()?;
                    let source_keys = raw_keys[*target_key_count..]
                        .iter()
                        .map(Value::array_key)
                        .collect::<Result<Vec<_>, _>>()?;
                    let property = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let receiver = stack.pop().ok_or("Invalid bytecode stack")?;
                    let (_, reference) = reference_cell_for_path(
                        source,
                        &source_keys,
                        variables,
                        &mut bindings,
                        global_variables,
                        global_scope,
                        frame_id,
                    )?;
                    let current = objects::read_property(
                        &receiver,
                        &property,
                        access_scope.as_deref(),
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                    let updated = current.bind_array_reference(&target_keys, reference)?;
                    objects::assign_property(
                        &receiver,
                        &property,
                        updated,
                        access_scope.as_deref(),
                        *in_constructor,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                }
                Op::CoalescePathJump(target, key_count) => {
                    if stack.len() < key_count + 1 {
                        return Err("Invalid bytecode stack".into());
                    }
                    let start = stack.len() - key_count - 1;
                    let root = stack[start].clone();
                    let keys = stack[start + 1..]
                        .iter()
                        .map(Value::array_key)
                        .collect::<Result<Vec<_>, _>>()?;
                    let current = root.array_get_path(&keys)?;
                    if !matches!(current, Value::Null) {
                        stack.truncate(start);
                        stack.push(current);
                        pc = *target;
                    }
                }
                Op::CoalesceAssignPath(variable, key_count) => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    if stack.len() < key_count + 1 {
                        return Err("Invalid bytecode stack".into());
                    }
                    let raw_keys = stack.split_off(stack.len() - key_count);
                    let keys = raw_keys
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    let root = stack.pop().ok_or("Invalid bytecode stack")?;
                    let updated = root.array_set_path(&keys, value.clone())?;
                    write_array_root(
                        variable.clone(),
                        updated,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    stack.push(value);
                }
                Op::Load(name) => {
                    warn_undefined_variable(
                        name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    stack.push(read_variable(
                        name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    ));
                }
                Op::LoadSilent(name) => stack.push(read_variable(
                    name,
                    variables,
                    &bindings,
                    global_variables,
                    global_scope,
                )),
                Op::LoadDynamic(name) => {
                    warn_undefined_variable(
                        name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let dynamic_name =
                        read_variable(name, variables, &bindings, global_variables, global_scope)
                            .show();
                    warn_undefined_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    stack.push(read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    ));
                }
                Op::LoadDynamicSilent(name) => {
                    let dynamic_name =
                        read_variable(name, variables, &bindings, global_variables, global_scope)
                            .show();
                    stack.push(read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    ));
                }
                Op::LoadDynamicExpr => {
                    let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    warn_undefined_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    stack.push(read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    ));
                }
                Op::LoadDynamicExprSilent => {
                    let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    stack.push(read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    ));
                }
                Op::CoalesceDynamicExprJump(target) => {
                    let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let current = read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    if matches!(current, Value::Null) {
                        stack.push(Value::String(dynamic_name));
                    } else {
                        stack.push(current);
                        pc = *target;
                    }
                }
                Op::Store(name) => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    if name == "GLOBALS" {
                        return Err("Cannot re-assign $GLOBALS".into());
                    }
                    let previous =
                        read_variable(name, variables, &bindings, global_variables, global_scope);
                    let current = value.clone();
                    write_variable(
                        name.clone(),
                        value,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    objects::run_destructors_removed_from_value(
                        previous,
                        &current,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                }
                Op::StoreArrayRoot(name) => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let previous =
                        read_variable(name, variables, &bindings, global_variables, global_scope);
                    let current = value.clone();
                    write_array_root(
                        name.clone(),
                        value,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    objects::run_destructors_removed_from_value(
                        previous,
                        &current,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                }
                Op::StoreDynamic(name) => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let dynamic_name =
                        read_variable(name, variables, &bindings, global_variables, global_scope)
                            .show();
                    let previous = read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let current = value.clone();
                    write_variable(
                        dynamic_name,
                        value,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    objects::run_destructors_removed_from_value(
                        previous,
                        &current,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                }
                Op::StoreDynamicExpr => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let previous = read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let current = value.clone();
                    write_variable(
                        dynamic_name,
                        value,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    objects::run_destructors_removed_from_value(
                        previous,
                        &current,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
                }
                Op::AssignDynamicExpr => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let previous = read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    write_variable(
                        dynamic_name,
                        value.clone(),
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    objects::run_destructors_removed_from_value(
                        previous,
                        &value,
                        functions,
                        classes,
                        static_properties,
                        global_variables,
                        output,
                        steps,
                    )?;
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
                    write_variable(
                        dynamic_name,
                        updated.clone(),
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    stack.push(updated);
                }
                Op::PostIncrement(name, delta) => {
                    // Return the old value while storing the incremented numeric value.
                    warn_undefined_variable(
                        name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let old_value =
                        read_variable(name, variables, &bindings, global_variables, global_scope);
                    write_variable(
                        name.clone(),
                        increment_value(&old_value, *delta, global_variables)?,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    stack.push(old_value);
                }
                Op::PreIncrement(name, delta) => {
                    warn_undefined_variable(
                        name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let current_value =
                        read_variable(name, variables, &bindings, global_variables, global_scope);
                    let next_value = increment_value(&current_value, *delta, global_variables)?;
                    write_variable(
                        name.clone(),
                        next_value.clone(),
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    stack.push(next_value);
                }
                Op::IncrementArrayPath(name, key_count, delta, post) => {
                    if stack.len() < *key_count {
                        return Err("Invalid bytecode stack".into());
                    }
                    let raw_keys = stack.split_off(stack.len() - key_count);
                    let keys = raw_keys
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    warn_undefined_variable(
                        name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let root =
                        read_variable(name, variables, &bindings, global_variables, global_scope);
                    warn_undefined_array_key(&root, &keys, global_variables);
                    let old_value = root.array_get_path(&keys)?;
                    let next_value = increment_value(&old_value, *delta, global_variables)?;
                    let updated = root.array_set_path(&keys, next_value.clone())?;
                    write_array_root(
                        name.clone(),
                        updated,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    )?;
                    stack.push(if *post { old_value } else { next_value });
                }
                Op::IncrementDynamicVariablePath(key_count, delta, post) => {
                    if stack.len() < key_count + 1 {
                        return Err("Invalid bytecode stack".into());
                    }
                    let raw_keys = stack.split_off(stack.len() - key_count);
                    let keys = raw_keys
                        .into_iter()
                        .map(|key| key.array_key())
                        .collect::<Result<Vec<_>, _>>()?;
                    let dynamic_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    warn_undefined_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    let root = read_variable(
                        &dynamic_name,
                        variables,
                        &bindings,
                        global_variables,
                        global_scope,
                    );
                    if keys.is_empty() {
                        let next_value = increment_value(&root, *delta, global_variables)?;
                        write_variable(
                            dynamic_name,
                            next_value.clone(),
                            variables,
                            &bindings,
                            global_variables,
                            global_scope,
                        )?;
                        stack.push(if *post { root } else { next_value });
                    } else {
                        warn_undefined_array_key(&root, &keys, global_variables);
                        let old_value = root.array_get_path(&keys)?;
                        let next_value = increment_value(&old_value, *delta, global_variables)?;
                        let updated = root.array_set_path(&keys, next_value.clone())?;
                        write_array_root(
                            dynamic_name,
                            updated,
                            variables,
                            &bindings,
                            global_variables,
                            global_scope,
                        )?;
                        stack.push(if *post { old_value } else { next_value });
                    }
                }
                Op::Echo => {
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let rendered = match &value {
                        Value::Object(_) => objects::stringify(
                            &value,
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?,
                        _ => value.show(),
                    };
                    if output.len().saturating_add(rendered.len()) > limits.max_output_bytes {
                        return Err("Execution output limit exceeded".into());
                    }
                    output.push_str(&rendered);
                    global_variables.output_offset = output.len();
                }
                Op::Pop => {
                    stack.pop().ok_or("Invalid bytecode stack")?;
                }
                Op::Dup => {
                    let value = stack.last().cloned().ok_or("Invalid bytecode stack")?;
                    stack.push(value);
                }
                Op::Dup2 => {
                    if stack.len() < 2 {
                        return Err("Invalid bytecode stack".into());
                    }
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
                    if !allow_yield {
                        return Err("Cannot use yield outside a generator function".into());
                    }
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    report_non_reference_yield(
                        global_variables,
                        yield_by_reference,
                        &value,
                        output.len(),
                    );
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
                    suspend_error_suppress_scopes(global_variables, frame_id, continuation);
                    return Ok(Some(Value::Null));
                }
                Op::YieldFrom => {
                    if !allow_yield {
                        return Err("Cannot use yield outside a generator function".into());
                    }
                    let iterator_key = pc - 1;
                    let mut first_entry = None;
                    if !yield_from.contains_key(&iterator_key) {
                        let iterable = stack.pop().ok_or("Invalid bytecode stack")?;
                        match iterable {
                            Value::Array(entries) => {
                                let mut iterator =
                                    YieldFromIterator::Array(entries.into_iter().collect(), 0);
                                first_entry = advance_yield_from(
                                    &mut iterator,
                                    Value::Null,
                                    None,
                                    functions,
                                    classes,
                                    static_properties,
                                    global_variables,
                                    output,
                                    steps,
                                    &mut stack,
                                    variables,
                                    &mut bindings,
                                    global_scope,
                                    frame_id,
                                )?;
                                if first_entry.is_some() {
                                    yield_from.insert(iterator_key, iterator);
                                } else {
                                    stack.push(Value::Null);
                                }
                            }
                            Value::Generator(generator) => {
                                if rewind_generator(
                                    &generator,
                                    functions,
                                    classes,
                                    static_properties,
                                    global_variables,
                                    output,
                                    steps,
                                )? {
                                    first_entry = current_generator_entry(&generator)?;
                                    yield_from.insert(
                                        iterator_key,
                                        YieldFromIterator::Generator(generator),
                                    );
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
                                    if !is_a_type(
                                        &class_name.to_ascii_lowercase(),
                                        "iteratoraggregate",
                                        classes,
                                        &mut HashSet::new(),
                                    ) {
                                        break;
                                    }
                                    if aggregate_depth >= 32 {
                                        return Err("IteratorAggregate nesting is too deep".into());
                                    }
                                    iterable = control::call_iterator_method(
                                        Value::Object(object),
                                        "getiterator",
                                        &mut stack,
                                        variables,
                                        &mut bindings,
                                        global_variables,
                                        global_scope,
                                        frame_id,
                                        functions,
                                        classes,
                                        static_properties,
                                        output,
                                        steps,
                                    )?;
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
                                            return Err(format!("TypeError:Object of type {class_name} does not provide the Iterator methods"));
                                        }
                                        let value = Value::Object(object);
                                        let _ = control::call_iterator_method(value.clone(), "rewind", &mut stack, variables,
                                            &mut bindings, global_variables, global_scope, frame_id, functions, classes,
                                            static_properties, output, steps)?;
                                        Some(YieldFromIterator::UserIterator { object: value, advance_pending: false })
                                    }
                                }
                                _ if aggregate_depth > 0 => return Err("TypeError:IteratorAggregate::getIterator() must return a Traversable object".into()),
                                _ => return Err("yield from expects an array or Traversable object".into()),
                            };
                                if let Some(mut iterator) = iterator {
                                    first_entry = advance_yield_from(
                                        &mut iterator,
                                        Value::Null,
                                        None,
                                        functions,
                                        classes,
                                        static_properties,
                                        global_variables,
                                        output,
                                        steps,
                                        &mut stack,
                                        variables,
                                        &mut bindings,
                                        global_scope,
                                        frame_id,
                                    )?;
                                    if first_entry.is_some() {
                                        yield_from.insert(iterator_key, iterator);
                                    } else {
                                        stack.push(yield_from_return_value(&iterator)?);
                                    }
                                }
                            }
                            _ => {
                                return Err(
                                    "yield from expects an array or Traversable object".into()
                                )
                            }
                        }
                    } else {
                        let sent = if delegated_exception.is_none() {
                            stack.pop().unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        };
                        let iterator = yield_from
                            .get_mut(&iterator_key)
                            .ok_or("Invalid yield-from iterator")?;
                        first_entry = advance_yield_from(
                            iterator,
                            sent,
                            delegated_exception.take(),
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                            &mut stack,
                            variables,
                            &mut bindings,
                            global_scope,
                            frame_id,
                        )?;
                    }
                    if let Some((key, value)) = first_entry {
                        report_non_reference_yield(
                            global_variables,
                            yield_by_reference,
                            &value,
                            output.len(),
                        );
                        continuation.pc = iterator_key;
                        continuation.stack = std::mem::take(&mut stack);
                        continuation.iterators = std::mem::take(&mut iterators);
                        continuation.yield_from = std::mem::take(&mut yield_from);
                        continuation.handlers = std::mem::take(&mut handlers);
                        continuation.pending_exits = std::mem::take(&mut pending_exits);
                        continuation.bindings = std::mem::take(&mut bindings);
                        continuation.awaiting_resume = true;
                        continuation.yielded = Some((Some(key), value));
                        suspend_error_suppress_scopes(global_variables, frame_id, continuation);
                        return Ok(Some(Value::Null));
                    }
                    if let Some(iterator) = yield_from.remove(&iterator_key) {
                        stack.push(yield_from_return_value(&iterator)?);
                    }
                }
                Op::Return => {
                    if let Some(PendingExit::Exception(id)) = pending_exits.pop() {
                        global_variables.take_thrown(id);
                    }
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
                Op::FinallyStart(target, protected_ranges) => {
                    handlers.push(ExceptionHandler::Finally {
                        target: *target,
                        protected_ranges: protected_ranges.clone(),
                    })
                }
                Op::TryEnd => {
                    handlers.pop().ok_or("Invalid exception-handler stack")?;
                }
                Op::Throw | Op::ThrowAt(_) => {
                    let throw_line = match instruction {
                        Op::ThrowAt(line) => Some(*line),
                        _ => None,
                    };
                    let value = stack.pop().ok_or("Invalid bytecode stack")?.dereference();
                    let value = match value {
                        Value::Object(object)
                            if is_a_type(
                                &object.borrow().class_name.to_ascii_lowercase(),
                                "throwable",
                                classes,
                                &mut HashSet::new(),
                            ) =>
                        {
                            Value::Object(object)
                        }
                        invalid => {
                            let message = if matches!(
                                invalid,
                                Value::Object(_) | Value::Generator(_)
                            ) {
                                "Cannot throw objects that do not implement Throwable"
                            } else {
                                "Can only throw objects"
                            };
                            crate::exception_builtins::construct(
                                "Error",
                                &[Value::String(message.into())],
                                classes,
                            )?
                        }
                    };
                    if let (Some(line), Value::Object(object)) = (throw_line, &value) {
                        if let Some(data) = object.borrow_mut().exception.as_mut() {
                            if data.file.is_empty() {
                                data.file = global_variables.source_file.clone();
                            }
                            if data.line.is_none() {
                                data.line = Some(line);
                            }
                        }
                    }
                    if let Some(PendingExit::Exception(superseded_id)) = pending_exits.pop() {
                        global_variables.take_thrown(superseded_id);
                    }
                    global_variables.store_thrown(*steps, value);
                    return Err(format!("{THROWN_VALUE_PREFIX}{steps}"));
                }
                Op::FinallyResume => {
                    match pending_exits.pop().ok_or("No exit is pending in finally")? {
                        PendingExit::Exception(id) => {
                            return Err(format!("{THROWN_VALUE_PREFIX}{id}"))
                        }
                        PendingExit::Return(value) => {
                            if let Some(target) =
                                route_exit_through_finally(pc - 1, None, &mut handlers)
                            {
                                pending_exits.push(PendingExit::Return(value));
                                pc = target;
                            } else {
                                return Ok(Some(value));
                            }
                        }
                        PendingExit::Jump(target) => {
                            if let Some(finalizer) =
                                route_exit_through_finally(pc - 1, Some(target), &mut handlers)
                            {
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
                    let converted = if matches!(cast_type, crate::bytecode::CastType::String)
                        && matches!(value.dereference(), Value::Object(_))
                    {
                        Value::String(objects::stringify(
                            &value,
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?)
                    } else {
                        apply_cast(*cast_type, value)?
                    };
                    stack.push(converted);
                }
                Op::InstanceOf => {
                    let expected_name = stack.pop().ok_or("Invalid bytecode stack")?.show();
                    let expected_key = expected_name.trim_start_matches('\\').to_ascii_lowercase();
                    if !classes.contains_key(&expected_key)
                        && !crate::exception_builtins::is_class(&expected_key)
                        && !global_variables.autoload_callbacks.is_empty()
                    {
                        functions::invoke_registered_autoloaders(
                            &expected_name,
                            variables,
                            &mut bindings,
                            global_variables,
                            global_scope,
                            frame_id,
                            functions,
                            classes,
                            static_properties,
                            output,
                            steps,
                        )?;
                    }
                    let expected = expected_key;
                    let mut active_classes = classes.clone();
                    active_classes.extend(global_variables.dynamic_classes.clone());
                    let value = stack.pop().ok_or("Invalid bytecode stack")?;
                    let matches = match value {
                        Value::Object(object) => {
                            let actual = object.borrow().class_name.to_ascii_lowercase();
                            is_a_type(&actual, &expected, &active_classes, &mut HashSet::new())
                        }
                        Value::Generator(_) => expected == "generator",
                        Value::Closure(_) => expected == "closure",
                        _ => false,
                    };
                    stack.push(Value::Bool(matches));
                }
                operation => {
                    let right = stack.pop().ok_or("Invalid bytecode stack")?;
                    let left = stack.pop().ok_or("Invalid bytecode stack")?;
                    if matches!(operation, Op::Concat) {
                        let left = objects::stringify(
                            &left,
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        let right = objects::stringify(
                            &right,
                            functions,
                            classes,
                            static_properties,
                            global_variables,
                            output,
                            steps,
                        )?;
                        stack.push(Value::String(left + &right));
                    } else {
                        stack.push(apply_binary(operation, left, right)?);
                    }
                }
            }
            Ok(None)
        })();
        if let Err(error) = dispatch_pending_error_handlers(
            variables,
            &mut bindings,
            global_variables,
            global_scope,
            frame_id,
            functions,
            classes,
            static_properties,
            output,
            steps,
        ) {
            execution = Err(error);
        }
        match execution {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {}
            Err(error) => {
                let error = if error.starts_with(THROWN_VALUE_PREFIX) {
                    error
                } else if let Some((class_name, message)) = runtime_error_class(&error) {
                    let exception = crate::exception_builtins::construct(
                        class_name,
                        &[Value::String(message.to_string())],
                        classes,
                    )?;
                    if let Value::Object(object) = &exception {
                        if let Some(data) = object.borrow_mut().exception.as_mut() {
                            if let Some(frame) = global_variables.active_trace_frames.last() {
                                if data.file.is_empty() {
                                    data.file = frame.file.clone();
                                }
                                if data.line.is_none() {
                                    data.line = Some(frame.line);
                                }
                                if frame.function != "{main}" && data.trace.is_empty() {
                                    data.trace = global_variables
                                        .active_trace_frames
                                        .iter()
                                        .rev()
                                        .filter(|frame| frame.function != "{main}")
                                        .cloned()
                                        .collect();
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
                let id = error[THROWN_VALUE_PREFIX.len()..]
                    .parse::<u64>()
                    .map_err(|_| "Invalid thrown-value token")?;
                let thrown = global_variables
                    .thrown_values
                    .get(&id)
                    .cloned()
                    .ok_or("Missing thrown value")?;
                let mut handled = false;
                while let Some(handler) = handlers.pop() {
                    match handler {
                        ExceptionHandler::Catch { clauses } => {
                            if let Some(clause) = clauses
                                .iter()
                                .find(|clause| exception_matches(&thrown, &clause.types, classes))
                            {
                                global_variables.take_thrown(id);
                                if let Some(variable) = &clause.variable {
                                    write_variable(
                                        variable.clone(),
                                        thrown.clone(),
                                        variables,
                                        &bindings,
                                        global_variables,
                                        global_scope,
                                    )?;
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
                if !handled {
                    return Err(error);
                }
            }
        }
    }
    Ok(None)
}

fn report_non_reference_yield(
    globals: &mut GlobalVariables,
    by_reference: bool,
    value: &Value,
    output_offset: usize,
) {
    if by_reference && !matches!(value, Value::Reference(_)) {
        globals.report_diagnostic(crate::RuntimeDiagnostic {
            level: crate::DiagnosticLevel::Notice,
            message: "Only variable references should be yielded by reference".into(),
            source_file: None,
            source_line: None,
            output_offset,
        });
    }
}

pub(super) fn is_generator_function(function: &BytecodeFunction) -> bool {
    function
        .instructions
        .iter()
        .any(|instruction| matches!(instruction, Op::Yield | Op::YieldKeyed | Op::YieldFrom))
}

pub(super) fn new_generator(
    function: BytecodeFunction,
    variables: HashMap<String, Value>,
    bindings: HashMap<String, VariableBinding>,
    frame_id: u64,
    classes: &HashMap<String, BytecodeClass>,
) -> Result<Value, String> {
    // Calling a generator function creates a Generator object; the function's
    // by-reference marker applies to yielded values, not this factory result.
    let mut function_for_validation = function.clone();
    function_for_validation.returns_by_reference = false;
    let generator = Value::Generator(Rc::new(RefCell::new(PhpGenerator {
        by_reference: function.returns_by_reference,
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
    advance_generator_with_exception(
        generator,
        functions,
        classes,
        static_properties,
        global_variables,
        output,
        steps,
        resume_value,
        None,
    )
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
    if generator.completed {
        return Ok(None);
    }
    if generator.current.is_some() {
        generator.advanced_past_first = true;
    }
    generator.started = true;
    generator.current = None;
    let function = generator.function.clone();
    let PhpGenerator {
        variables,
        continuation,
        ..
    } = &mut *generator;
    let result = execute_frame(
        &function.instructions,
        functions,
        classes,
        static_properties,
        variables,
        global_variables,
        false,
        &function_scope(&function),
        continuation,
        resume_value,
        resume_exception,
        true,
        function.returns_by_reference,
        output,
        steps,
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
                let frame = std::mem::take(&mut generator.variables);
                let return_value = generator.return_value.clone().unwrap_or(Value::Null);
                drop(generator);
                functions::finish_user_frame(
                    frame,
                    &return_value,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                )?;
                Ok(None)
            }
        }
        Err(error) => {
            generator.completed = true;
            generator.current = None;
            let frame = std::mem::take(&mut generator.variables);
            drop(generator);
            functions::finish_user_frame(
                frame,
                &Value::Null,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            Err(error)
        }
    }
}

pub(super) fn current_generator_entry(
    generator: &Rc<RefCell<PhpGenerator>>,
) -> Result<Option<(Value, Value)>, String> {
    let generator = generator
        .try_borrow()
        .map_err(|_| "Generator is already running")?;
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
        let generator = generator
            .try_borrow()
            .map_err(|_| "Generator is already running")?;
        (generator.started, generator.advanced_past_first)
    };
    if advanced {
        return Err("Cannot rewind a generator that was already run".into());
    }
    if !started {
        advance_generator(
            generator,
            functions,
            classes,
            static_properties,
            global_variables,
            output,
            steps,
            Value::Null,
        )?;
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
            if !arguments.is_empty() {
                return Err(format!("Generator::{method}() expects no arguments"));
            }
            if !generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?
                .started
            {
                advance_generator(
                    generator,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    Value::Null,
                )?;
            }
            let generator = generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?;
            match method.as_str() {
                "current" => Ok(generator
                    .current
                    .as_ref()
                    .map(|(_, value)| value.clone())
                    .unwrap_or(Value::Null)),
                "key" => Ok(generator
                    .current
                    .as_ref()
                    .map(|(key, _)| key.clone())
                    .unwrap_or(Value::Null)),
                "valid" => Ok(Value::Bool(generator.current.is_some())),
                _ => unreachable!(),
            }
        }
        "rewind" => {
            if !arguments.is_empty() {
                return Err("Generator::rewind() expects no arguments".into());
            }
            rewind_generator(
                generator,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
            )?;
            Ok(Value::Null)
        }
        "next" => {
            if !arguments.is_empty() {
                return Err("Generator::next() expects no arguments".into());
            }
            if !generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?
                .started
            {
                advance_generator(
                    generator,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    Value::Null,
                )?;
            }
            advance_generator(
                generator,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                Value::Null,
            )?;
            Ok(Value::Null)
        }
        "send" => {
            if arguments.len() != 1 {
                return Err("Generator::send() expects exactly 1 argument".into());
            }
            if !generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?
                .started
            {
                advance_generator(
                    generator,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    Value::Null,
                )?;
            }
            advance_generator(
                generator,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                arguments[0].clone(),
            )?;
            let generator = generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?;
            Ok(generator
                .current
                .as_ref()
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null))
        }
        "getreturn" => {
            if !arguments.is_empty() {
                return Err("Generator::getReturn() expects no arguments".into());
            }
            let generator = generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?;
            if !generator.completed {
                return Err("Generator::getReturn() can only be called once the generator has finished executing".into());
            }
            Ok(generator.return_value.clone().unwrap_or(Value::Null))
        }
        "throw" => {
            if arguments.len() != 1 {
                return Err("Generator::throw() expects exactly 1 argument".into());
            }
            let exception = arguments[0].dereference();
            let Value::Object(object) = &exception else {
                return Err("Generator::throw(): Argument #1 ($exception) must be an object implementing Throwable".into());
            };
            let class_name = object.borrow().class_name.to_ascii_lowercase();
            if !is_a_type(&class_name, "throwable", classes, &mut HashSet::new()) {
                return Err(format!(
                    "Generator::throw(): {} does not implement Throwable",
                    object.borrow().class_name
                ));
            }
            if !generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?
                .started
            {
                advance_generator(
                    generator,
                    functions,
                    classes,
                    static_properties,
                    global_variables,
                    output,
                    steps,
                    Value::Null,
                )?;
            }
            if generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?
                .completed
            {
                global_variables.store_thrown(*steps, exception);
                return Err(format!("{THROWN_VALUE_PREFIX}{steps}"));
            }
            advance_generator_with_exception(
                generator,
                functions,
                classes,
                static_properties,
                global_variables,
                output,
                steps,
                Value::Null,
                Some(exception),
            )?;
            let generator = generator
                .try_borrow()
                .map_err(|_| "Generator is already running")?;
            Ok(generator
                .current
                .as_ref()
                .map(|(_, value)| value.clone())
                .unwrap_or(Value::Null))
        }
        _ => Err(format!("Call to undefined method Generator::{method}()")),
    }
}
