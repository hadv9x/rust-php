//! Runtime bytecode structures and public execution interface.

mod array_builtins;
mod json_builtins;
mod math_builtins;
mod exception_builtins;
mod pdo;
mod mongo_php;
mod string_builtins;
mod values;
mod vm;
pub mod bytecode;

pub use vm::PhpGenerator;

use std::sync::{OnceLock, RwLock};
use std::rc::Rc;

/// Compiler-provided runtime loader used by dynamic include and require expressions.
pub type IncludeLoader = Rc<dyn Fn(&str, &str, &str) -> Result<BytecodeProgram, String>>;

static DATABASE_REGISTRY: OnceLock<RwLock<php_db::DatabaseRegistry>> = OnceLock::new();

/// Install process-wide named database handles used by PHP's PDO compatibility layer.
pub fn set_database_registry(registry: php_db::DatabaseRegistry) {
    *DATABASE_REGISTRY.get_or_init(|| RwLock::new(php_db::DatabaseRegistry::default()))
        .write().unwrap_or_else(std::sync::PoisonError::into_inner) = registry;
}

pub(crate) fn database_registry() -> php_db::DatabaseRegistry {
    DATABASE_REGISTRY.get_or_init(|| RwLock::new(php_db::DatabaseRegistry::default()))
        .read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

pub(crate) use values::{PhpExceptionData, PhpTraceFrame};
pub use values::{ArrayKey, PhpArray, PhpClosure, PhpObject, PhpStream, Value};

/// Return a runtime-provided predefined constant when the name is supported.
pub fn predefined_constant(name: &str) -> Option<Value> {
    vm::builtin_constant(name)
}

/// Resolve constants exposed by the built-in PDO and PDOStatement classes.
pub fn pdo_class_constant(class: &str, constant: &str) -> Option<Value> {
    pdo::class_constant(class, constant)
}

/// Resolve the integer constants on PHP's Intl `NumberFormatter` class.
pub fn intl_class_constant(class: &str, constant: &str) -> Option<Value> {
    if !class.rsplit('\\').next()?.eq_ignore_ascii_case("NumberFormatter") { return None; }
    let value = match constant.to_ascii_uppercase().as_str() {
        "PATTERN_DECIMAL" => 0, "DECIMAL" => 1, "CURRENCY" => 2, "PERCENT" => 3,
        "SCIENTIFIC" => 4, "SPELLOUT" => 5, "ORDINAL" => 6, "DURATION" => 7,
        "PATTERN_RULEBASED" => 9, "IGNORE" | "DEFAULT_STYLE" => 0,
        "TYPE_DEFAULT" => 0, "TYPE_INT32" => 1, "TYPE_INT64" => 2, "TYPE_DOUBLE" => 3, "TYPE_CURRENCY" => 4,
        "PARSE_INT_ONLY" | "POSITIVE_PREFIX" | "DECIMAL_SEPARATOR_SYMBOL" | "ROUND_CEILING" | "PAD_BEFORE_PREFIX" => 0,
        "GROUPING_USED" | "POSITIVE_SUFFIX" | "GROUPING_SEPARATOR_SYMBOL" | "ROUND_FLOOR" | "PAD_AFTER_PREFIX" => 1,
        "DECIMAL_ALWAYS_SHOWN" | "NEGATIVE_PREFIX" | "PATTERN_SEPARATOR_SYMBOL" | "ROUND_DOWN" | "PAD_BEFORE_SUFFIX" => 2,
        "MAX_INTEGER_DIGITS" | "NEGATIVE_SUFFIX" | "PERCENT_SYMBOL" | "ROUND_UP" | "PAD_AFTER_SUFFIX" => 3,
        "MIN_INTEGER_DIGITS" | "PADDING_CHARACTER" | "ZERO_DIGIT_SYMBOL" | "ROUND_HALFEVEN" => 4,
        "INTEGER_DIGITS" | "CURRENCY_CODE" | "DIGIT_SYMBOL" | "ROUND_HALFDOWN" => 5,
        "MAX_FRACTION_DIGITS" | "DEFAULT_RULESET" | "MINUS_SIGN_SYMBOL" | "ROUND_HALFUP" => 6,
        "MIN_FRACTION_DIGITS" | "PUBLIC_RULESETS" | "PLUS_SIGN_SYMBOL" => 7,
        "FRACTION_DIGITS" | "CURRENCY_SYMBOL" => 8,
        "MULTIPLIER" | "INTL_CURRENCY_SYMBOL" => 9,
        "GROUPING_SIZE" | "MONETARY_SEPARATOR_SYMBOL" => 10,
        "ROUNDING_MODE" | "EXPONENTIAL_SYMBOL" => 11,
        "ROUNDING_INCREMENT" | "PERMILL_SYMBOL" => 12,
        "FORMAT_WIDTH" | "PAD_ESCAPE_SYMBOL" => 13,
        "PADDING_POSITION" | "INFINITY_SYMBOL" => 14,
        "SECONDARY_GROUPING_SIZE" | "NAN_SYMBOL" => 15,
        "SIGNIFICANT_DIGITS_USED" | "SIGNIFICANT_DIGIT_SYMBOL" => 16,
        "MIN_SIGNIFICANT_DIGITS" | "MONETARY_GROUPING_SEPARATOR_SYMBOL" => 17,
        "MAX_SIGNIFICANT_DIGITS" => 18, "LENIENT_PARSE" => 19,
        _ => return None,
    };
    Some(Value::Integer(value))
}

/// Resolve the modifier constants exposed by PHP's built-in ReflectionMethod class.
pub fn reflection_method_class_constant(class: &str, constant: &str) -> Option<i64> {
    if !class.rsplit('\\').next()?.eq_ignore_ascii_case("ReflectionMethod") { return None; }
    match constant.to_ascii_uppercase().as_str() {
        "IS_PUBLIC" => Some(1),
        "IS_PROTECTED" => Some(2),
        "IS_PRIVATE" => Some(4),
        "IS_STATIC" => Some(16),
        "IS_FINAL" => Some(32),
        "IS_ABSTRACT" => Some(64),
        _ => None,
    }
}

/// Resolve the modifier constants exposed by PHP's built-in ReflectionProperty class.
pub fn reflection_property_class_constant(class: &str, constant: &str) -> Option<i64> {
    if !class.rsplit('\\').next()?.eq_ignore_ascii_case("ReflectionProperty") { return None; }
    match constant.to_ascii_uppercase().as_str() {
        "IS_PUBLIC" => Some(1),
        "IS_PROTECTED" => Some(2),
        "IS_PRIVATE" => Some(4),
        "IS_STATIC" => Some(16),
        "IS_PRIVATE_SET" => Some(32),
        "IS_PROTECTED_SET" => Some(64),
        "IS_READONLY" => Some(128),
        _ => None,
    }
}

/// Per-program safeguards for VM work and captured output.
#[derive(Clone, Copy, Debug)]
pub struct ExecutionLimits {
    pub max_instructions: u64,
    pub max_output_bytes: usize,
    /// Optional wall-clock budget checked between VM instructions.
    pub max_execution_time: Option<std::time::Duration>,
}

impl Default for ExecutionLimits {
    fn default() -> Self {
        Self { max_instructions: 10_000_000, max_output_bytes: 8 * 1024 * 1024, max_execution_time: Some(std::time::Duration::from_secs(30)) }
    }
}

/// Send-safe subset of PHP values that can be retained between HTTP requests.
#[derive(Clone, Debug)]
pub enum SessionValue {
    Null,
    Integer(i64),
    Number(f64),
    String(String),
    Bool(bool),
    Array(Vec<(ArrayKey, SessionValue)>),
}

/// Compiled program for the php-rust bytecode VM.
#[derive(Clone, Debug)]
pub struct BytecodeFunction {
    pub name: String,
    /// Source spelling retained for reflection; `name` remains the normalized lookup key.
    pub display_name: String,
    pub parameters: Vec<String>,
    pub parameter_types: Vec<Option<String>>,
    /// Whether each positional parameter uses the supported by-reference mode.
    pub parameter_by_reference: Vec<bool>,
    /// Whether each declared parameter collects extra positional arguments.
    pub parameter_variadic: Vec<bool>,
    /// Whether each constructor parameter is promoted to a property.
    pub parameter_promoted: Vec<bool>,
    /// Literal defaults for optional positional parameters.
    pub parameter_defaults: Vec<Option<Value>>,
    /// Source-level names for named-constant parameter defaults, aligned with parameters.
    pub parameter_default_constant_names: Vec<Option<String>>,
    pub return_type: Option<String>,
    /// Whether scalar return declarations use strict typing from the defining file.
    pub strict_types: bool,
    /// Whether this callable exposes a returned reference cell to its caller.
    pub returns_by_reference: bool,
    pub instructions: Vec<bytecode::Op>,
    pub is_static: bool,
    pub is_abstract: bool,
    pub is_final: bool,
    pub visibility: MemberVisibility,
    pub declaring_class: Option<String>,
    /// Attributes attached to top-level functions.
    pub attributes: Vec<BytecodeAttribute>,
    /// Attributes attached to parameters, aligned with `parameters`.
    pub parameter_attributes: Vec<Vec<BytecodeAttribute>>,
}

impl BytecodeFunction {
    /// Build a VM function or method from parameter names and compiled instructions.
    pub fn new(name: String, parameters: Vec<String>, parameter_types: Vec<Option<String>>, parameter_by_reference: Vec<bool>, parameter_variadic: Vec<bool>, parameter_defaults: Vec<Option<Value>>, return_type: Option<String>, returns_by_reference: bool, instructions: Vec<bytecode::Op>, is_static: bool, is_abstract: bool, is_final: bool, visibility: MemberVisibility, declaring_class: Option<String>) -> Self {
        let parameter_attributes = vec![Vec::new(); parameters.len()];
        let parameter_promoted = vec![false; parameters.len()];
        let parameter_default_constant_names = vec![None; parameters.len()];
        Self { display_name: name.clone(), name, parameters, parameter_types, parameter_by_reference, parameter_variadic, parameter_promoted, parameter_defaults, parameter_default_constant_names, return_type, strict_types: false, returns_by_reference, instructions, is_static, is_abstract, is_final, visibility, declaring_class, attributes: Vec::new(), parameter_attributes }
    }
}

/// Executable bytecode plus resolved function and class metadata.
pub struct BytecodeProgram {
    instructions: Vec<bytecode::Op>,
    functions: std::collections::HashMap<String, BytecodeFunction>,
    classes: std::collections::HashMap<String, BytecodeClass>,
    constant_attributes: std::collections::HashMap<String, Vec<BytecodeAttribute>>,
    source_file: String,
    include_loader: Option<IncludeLoader>,
}

/// Runtime metadata for a class or interface.
#[derive(Clone)]
pub struct BytecodeClass {
    /// Class or interface spelling as declared in PHP source; map keys remain lowercase.
    pub name: String,
    /// Name exposed by PHP reflection, including source location for anonymous classes.
    pub reflection_name: String,
    /// Attribute declarations retained for PHP reflection.
    pub attributes: Vec<BytecodeAttribute>,
    pub parent: Option<String>,
    pub interfaces: Vec<String>,
    /// Traits declared directly in this class or trait's `use` list.
    pub traits: Vec<String>,
    /// Aliases declared in this class or trait's own trait-use adaptations.
    pub trait_aliases: Vec<(String, String)>,
    pub is_interface: bool,
    pub is_trait: bool,
    /// Trait names that can only be validated if this lazily created anonymous class executes.
    pub unresolved_traits: Vec<String>,
    pub is_abstract: bool,
    pub is_final: bool,
    pub is_readonly: bool,
    pub is_enum: bool,
    pub enum_backing_type: Option<String>,
    pub enum_cases: Vec<String>,
    pub properties: std::collections::HashMap<String, BytecodeProperty>,
    /// Non-static instance properties in their PHP declaration order.
    pub property_order: Vec<String>,
    pub static_properties: std::collections::HashMap<String, BytecodeProperty>,
    pub constants: std::collections::HashMap<String, BytecodeConstant>,
    /// Class constants in declaration/composition order for reflection.
    pub constant_order: Vec<String>,
    pub methods: std::collections::HashMap<String, BytecodeFunction>,
    /// Method names in class composition order; maps remain for case-insensitive lookup.
    pub method_order: Vec<String>,
    /// Attributes attached to declared methods, keyed by lowercase method name.
    pub method_attributes: std::collections::HashMap<String, Vec<BytecodeAttribute>>,
    /// Source spelling of declared method names, keyed by lowercase method name.
    pub method_display_names: std::collections::HashMap<String, String>,
    /// Parameter attributes by lowercase method name, then parameter position.
    pub parameter_attributes: std::collections::HashMap<String, Vec<Vec<BytecodeAttribute>>>,
    /// Attributes attached to declared properties, keyed by property name.
    pub property_attributes: std::collections::HashMap<String, Vec<BytecodeAttribute>>,
}

/// A class-level PHP attribute and its evaluated constant arguments.
#[derive(Clone, Debug)]
pub struct BytecodeAttribute {
    pub name: String,
    pub arguments: Vec<(Option<String>, Value)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Runtime visibility values used by member access checks.
pub enum MemberVisibility { Public, Protected, Private }

/// Class property metadata kept separately from its per-object or static value.
#[derive(Clone, Debug)]
/// Runtime property metadata and its optional initial value.
pub struct BytecodeProperty {
    pub default: Option<Value>,
    pub visibility: MemberVisibility,
    /// Visibility used by PHP 8.4 asymmetric property writes, defaulting to `visibility`.
    pub set_visibility: Option<MemberVisibility>,
    pub declaring_class: String,
    pub type_name: Option<String>,
    pub is_readonly: bool,
    pub is_promoted: bool,
    /// Compiled PHP 8.4 getter hook, if declared.
    pub get_hook: Option<BytecodeFunction>,
    /// Compiled PHP 8.4 setter hook, if declared.
    pub set_hook: Option<BytecodeFunction>,
}

/// Class constant value and access metadata.
#[derive(Clone, Debug)]
/// Runtime class constant value and access metadata.
pub struct BytecodeConstant {
    pub value: Value,
    pub visibility: MemberVisibility,
    pub declaring_class: String,
    pub is_final: bool,
    pub type_name: Option<String>,
    pub attributes: Vec<BytecodeAttribute>,
}

impl BytecodeProgram {
    /// Create a program from compiler-generated instructions and resolved declarations.
    pub fn new(
        instructions: Vec<bytecode::Op>,
        functions: std::collections::HashMap<String, BytecodeFunction>,
        classes: std::collections::HashMap<String, BytecodeClass>,
    ) -> Self {
        Self { instructions, functions, classes, constant_attributes: std::collections::HashMap::new(), source_file: String::new(), include_loader: None }
    }

    /// Set attribute metadata attached to user-defined global constants.
    pub fn set_constant_attributes(&mut self, attributes: std::collections::HashMap<String, Vec<BytecodeAttribute>>) {
        self.constant_attributes = attributes;
    }

    /// Set the source path used for exception location metadata.
    pub fn set_source_file(&mut self, source_file: String) { self.source_file = source_file; }

    /// Attach the compiler's source loader for runtime-selected include paths.
    pub fn set_include_loader(&mut self, loader: IncludeLoader) { self.include_loader = Some(loader); }

    /// Execute in a fresh variable/output context and return captured stdout.
    pub fn execute(&self) -> Result<String, String> {
        vm::with_include_loader(self.include_loader.clone(), || vm::execute(&self.instructions, &self.functions, &self.classes, &self.constant_attributes, &self.source_file))
    }

    /// Execute with predefined PHP global variables and capture web response metadata.
    pub fn execute_with_globals(&self, globals: std::collections::HashMap<String, Value>) -> Result<ExecutionResult, String> {
        vm::with_include_loader(self.include_loader.clone(), || vm::execute_with_globals(&self.instructions, &self.functions, &self.classes, &self.constant_attributes, globals, &self.source_file))
    }

    /// Execute with request-specific VM instruction and output limits.
    pub fn execute_with_globals_and_limits(
        &self,
        globals: std::collections::HashMap<String, Value>,
        limits: ExecutionLimits,
    ) -> Result<ExecutionResult, String> {
        vm::with_include_loader(self.include_loader.clone(), || vm::execute_with_globals_and_limits(&self.instructions, &self.functions, &self.classes, &self.constant_attributes, globals, limits, &self.source_file))
    }

    /// Execute with request globals, limits, and diagnostics raised before the script begins.
    pub fn execute_with_globals_limits_and_diagnostics(
        &self,
        globals: std::collections::HashMap<String, Value>,
        limits: ExecutionLimits,
        initial_diagnostics: Vec<RuntimeDiagnostic>,
    ) -> Result<ExecutionResult, String> {
        vm::with_include_loader(self.include_loader.clone(), || vm::execute_with_globals_and_limits_and_diagnostics(&self.instructions, &self.functions, &self.classes, &self.constant_attributes, globals, limits, &self.source_file, initial_diagnostics))
    }
}

/// Output and response metadata produced by one isolated PHP execution.
#[derive(Clone, Debug)]
pub struct ExecutionResult {
    pub output: String,
    pub diagnostics: Vec<RuntimeDiagnostic>,
    pub status_code: Option<u16>,
    pub headers: Vec<(String, String)>,
    pub session_started: bool,
    pub session_destroyed: bool,
    pub session_id: String,
    pub session_data: Option<SessionValue>,
}

/// Severity of a non-fatal PHP runtime diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticLevel { Warning, Deprecated }

/// A warning or deprecation emitted during one program execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    /// Canonical PHP source path for the active statement, when available.
    pub source_file: Option<String>,
    /// One-based source line for the active statement, when available.
    pub source_line: Option<usize>,
    /// Byte offset in captured PHP output where this diagnostic occurred.
    pub output_offset: usize,
}
