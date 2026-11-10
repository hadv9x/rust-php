//! Mutable object instance storage shared by runtime handles.

use std::collections::{HashMap, HashSet};

use crate::Value;

/// Mutable per-instance property storage shared through `Rc<RefCell<_>>` object handles.
#[derive(Clone, Debug)]
/// An object instance with a class name and property storage.
pub struct PhpObject {
    /// Runtime class name used during inherited method lookup.
    pub class_name: String,
    /// Current property values, including inherited defaults.
    pub properties: HashMap<String, Value>,
    /// Known public/object property order for serialization and iteration.
    pub property_order: Vec<String>,
    /// Properties whose setter hook is currently writing to its own backing slot.
    pub active_property_hooks: HashSet<String>,
    /// Native state for built-in objects that cannot be represented as user properties.
    pub exception: Option<PhpExceptionData>,
}

/// Core fields carried by built-in `Exception` and `Error` objects.
#[derive(Clone, Debug)]
pub struct PhpExceptionData {
    pub message: String,
    pub code: i64,
    pub severity: i64,
    pub is_error_exception: bool,
    pub previous: Value,
    pub file: String,
    pub line: Option<usize>,
    /// Closest-first creation stack frames captured by the current VM frame.
    pub trace: Vec<PhpTraceFrame>,
}

/// Source frame retained by a native Throwable object.
#[derive(Clone, Debug)]
pub struct PhpTraceFrame {
    pub file: String,
    pub line: usize,
    pub function: String,
    pub class: Option<String>,
    pub call_type: Option<String>,
    /// Values of the callable parameters at the time this frame was entered.
    pub arguments: Vec<Value>,
}
