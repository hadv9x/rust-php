//! Instruction set consumed by the virtual machine.

use crate::{ArrayKey, DiagnosticLevel, Value};

/// A caller-side l-value that can be passed to a by-reference parameter.
#[derive(Clone, Debug)]
pub struct ReferenceArgument {
    pub variable: String,
    pub array_keys: Vec<ArrayKey>,
    pub object_property: Option<(Value, String)>,
    /// A reference cell supplied by unpacking an array that already contains a reference.
    pub direct_reference: Option<std::rc::Rc<std::cell::RefCell<Value>>>,
}

/// Compiler metadata describing the extra key operands emitted for an array l-value.
#[derive(Clone, Debug)]
pub struct ReferenceDescriptor {
    pub variable: String,
    pub array_key_count: usize,
    pub object_property: Option<String>,
    pub access_scope: Option<String>,
    /// The argument expression pushes an already resolved reference cell.
    pub direct_reference: bool,
}

/// One destination and type set handled by a compiled catch clause.
#[derive(Clone, Debug)]
pub struct CatchTarget {
    pub target: usize,
    pub variable: Option<String>,
    pub types: Vec<String>,
}

/// Arithmetic or concatenation used while updating a nested array path.
#[derive(Clone, Copy, Debug)]
pub enum CompoundOperation {
    Add,
    Subtract,
    Multiply,
    Power,
    Divide,
    Modulo,
    Concatenate,
    BitAnd,
    BitXor,
    BitOr,
    ShiftLeft,
    ShiftRight,
}

/// Scalar conversions emitted by explicit PHP casts.
#[derive(Clone, Copy, Debug)]
pub enum CastType {
    Integer,
    Float,
    String,
    Bool,
    Array,
    Object,
}

/// Stack-machine operations emitted by `php-compiler` and interpreted by the VM.
#[derive(Clone, Debug)]
/// An instruction in the VM stack-machine bytecode.
pub enum Op {
    /// Emit a PHP warning/deprecation at the current source location.
    ReportDiagnostic(DiagnosticLevel, String),
    /// Set the active source line for the current execution frame.
    SetLine(usize),
    /// Set the active source file for the current execution frame.
    SetFile(String),
    /// Record that this source file has begun executing for include_once semantics.
    MarkIncluded(String),
    /// Include a PHP source file selected at runtime. The path is on the operand stack.
    Include(IncludeMode),
    Const(Value),
    /// Resolve a global or namespaced constant at runtime.
    LoadConstant(String),
    /// Initialize a global or namespaced constant from its pre-evaluated declaration value.
    DeclareConstant(String),
    Load(String),
    /// Read a variable without an undefined-variable warning for null-coalescing contexts.
    LoadSilent(String),
    /// Push the shared storage cell for a variable or nested array element.
    LoadReference(String, usize),
    /// Push a shared reference to a public untyped instance property.
    LoadPropertyReference(String, Option<String>),
    /// Push a shared reference to an array slot held by an object property.
    LoadPropertyArrayReference(String, usize, Option<String>),
    /// Push the shared storage cell for a static property.
    LoadStaticPropertyReference(String, String, Option<String>),
    /// Push/create a shared reference to an array element in a static-property cell.
    LoadStaticPropertyArrayReference(usize),
    /// Bind a local name to the shared program-global variable table in this frame.
    DeclareGlobal(String),
    /// Bind a local name to a persistent storage slot, initializing it only once.
    DeclareStatic(String, Value),
    /// Bind a static local and skip its dynamic initializer once its slot is initialized.
    StaticInitializerCheck(String, usize),
    /// Store the evaluated value of a first-use static local initializer.
    InitializeStatic(String),
    /// Bind a target variable to the storage slot referenced by a source name.
    AssignReference(String, String),
    /// Bind a variable to the reference cell returned by a by-reference callable.
    BindReturnedReference(String),
    /// Bind a variable to a returned reference and push the assignment's resulting value.
    BindReturnedReferenceExpr(String),
    /// Bind a local variable to an array element path on the operand stack.
    AssignArrayReference(String, String, usize),
    /// Bind an array slot to a variable or array-path reference cell.
    BindArrayReference(String, usize, String, usize),
    /// Bind a static-property array slot to another static-property array slot.
    BindStaticPropertyArrayReference(String, String, usize, String, String, usize, Option<String>),
    /// Bind an array element to a reference value already on the operand stack.
    BindArrayReturnedReference(String, usize),
    /// Store a shared reference into a dynamically named object property.
    BindDynamicPropertyReference,
    BindDynamicPropertyArrayReference(usize, String, usize, Option<String>, bool),
    /// Bind an appended array slot inside an object property to a shared reference.
    BindPropertyAppendReference(String, Option<String>, bool),
    /// Bind a selected array element inside an instance property to a variable reference.
    BindPropertyArrayReference(String, usize, String, usize, Option<String>, bool),
    /// Read a local variable named by another local variable's string value.
    LoadDynamic(String),
    /// Read a computed variable name without an undefined-variable warning.
    LoadDynamicSilent(String),
    /// Read a variable whose name is computed on the operand stack.
    LoadDynamicExpr,
    /// Read a stack-computed variable name without an undefined-variable warning.
    LoadDynamicExprSilent,
    Store(String),
    /// Commit a rebuilt array root, including updates to the special `$GLOBALS` table.
    StoreArrayRoot(String),
    /// Store into a local variable named by another local variable's string value.
    StoreDynamic(String),
    /// Store a value into a variable whose name is computed on the operand stack.
    StoreDynamicExpr,
    /// Store a dynamically named variable and leave the assigned value on the stack.
    AssignDynamicExpr,
    /// Apply a compound operation to a dynamically named variable and return the result.
    CompoundAssignDynamicExpr(CompoundOperation),
    /// Keep a non-null dynamic variable value, otherwise retain its name for `??=` storage.
    CoalesceDynamicExprJump(usize),
    /// Increment/decrement a local variable and leave its prior value on the stack.
    PostIncrement(String, i8),
    /// Increment/decrement a local variable and leave its updated value on the stack.
    PreIncrement(String, i8),
    /// Increment or decrement an array path and return either its previous or new value.
    IncrementArrayPath(String, usize, i8, bool),
    /// Increment a variable-variable or nested array path rooted at one.
    IncrementDynamicVariablePath(usize, i8, bool),
    /// Increment an array offset stored in an object property.
    IncrementPropertyArrayPath(String, usize, i8, bool, Option<String>, bool),
    IncrementDynamicPropertyArrayPath(usize, i8, bool, Option<String>, bool),
    IncrementDynamicStaticPropertyArrayPath(usize, i8, bool, Option<String>),
    /// Increment an array offset stored in a static property.
    IncrementStaticPropertyArrayPath(String, String, usize, i8, bool, Option<String>),
    /// Increment a property while preserving prefix/postfix result semantics.
    IncrementProperty(String, i8, bool, Option<String>, bool),
    IncrementDynamicProperty(i8, bool, Option<String>, bool),
    /// Increment a static property while preserving prefix/postfix result semantics.
    IncrementStaticProperty(String, String, i8, bool, Option<String>),
    Add,
    Sub,
    Mul,
    Pow,
    Div,
    Mod,
    Concat,
    BitAnd,
    BitXor,
    BitOr,
    ShiftLeft,
    ShiftRight,
    Neg,
    /// Convert a runtime value to the negation of its PHP truthiness.
    Not,
    BitNot,
    Cast(CastType),
    Eq,
    NotEq,
    StrictEq,
    StrictNotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Spaceship,
    /// Test an object against a runtime class name, including its parents and interfaces.
    InstanceOf,
    LogicalXor,
    Echo,
    Pop,
    /// Copy the top stack value for switch-case comparisons.
    Dup,
    /// Duplicate the top two operands while preserving their order.
    Dup2,
    MakeArray(Vec<ArrayEntryKind>),
    ArrayGet,
    ArrayGetSilent,
    /// Silent for missing offsets, but validates key types for isset()/empty().
    ArrayGetIsset,
    ArraySetPath(usize),
    /// Set a value through a path that may append at intermediate levels.
    ArraySetMixedPath(Vec<bool>),
    /// Store the assigned value through a nested path and return that value.
    AssignArrayPath(String, usize),
    /// Apply a compound operation through a nested path, store the array, and return the result.
    AssignCompoundPath(String, usize, CompoundOperation),
    CompoundAssignPath(usize, CompoundOperation),
    /// Short-circuit a nested array-path coalescing assignment when the slot exists.
    CoalescePathJump(usize, usize),
    /// Store the right-hand value at a nested path and write the updated root variable.
    CoalesceAssignPath(String, usize),
    ArrayAppend,
    ArrayAppendPath(usize),
    /// Append to a nested path and write the updated root variable back.
    AssignArrayAppendPath(String, usize),
    /// Assign explicitly keyed or successive integer offsets to destructuring targets.
    ListDestructure(
        Vec<DestructureTarget>,
        usize,
        Option<String>,
        bool,
        Option<String>,
    ),
    /// Destructure an assigned value and preserve it as the expression result.
    ListDestructureKeep(
        Vec<DestructureTarget>,
        usize,
        Option<String>,
        bool,
        Option<String>,
    ),
    Call(
        String,
        Vec<Option<ReferenceDescriptor>>,
        Vec<Option<String>>,
        Vec<bool>,
        bool,
    ),
    /// Create a closure by copying the named variables from the current frame.
    MakeClosure(Box<crate::BytecodeFunction>, Vec<(String, bool)>),
    /// Create a closure around a named function or builtin.
    MakeFunctionCallable(String),
    /// Create a closure around a public static method.
    MakeStaticCallable(String, String),
    /// Create a closure around a late-statically-bound method using the active class.
    MakeLateStaticCallable(String),
    /// Create a closure around a public instance method; receiver is on the stack.
    MakeMethodCallable(String),
    /// Invoke a closure value already on the operand stack.
    CallValue(
        usize,
        Vec<Option<ReferenceDescriptor>>,
        Vec<Option<String>>,
        Vec<bool>,
        bool,
    ),
    /// Suspend a generator with the value on the operand stack.
    Yield,
    /// Suspend a generator with key and value on the operand stack.
    YieldKeyed,
    /// Delegate yielding to an iterable stored on the operand stack.
    YieldFrom,
    /// Register an exception handler target and catch-variable name.
    TryStart(Vec<CatchTarget>),
    /// Route an exception through a finally block before rethrowing it.
    FinallyStart(usize, Vec<(usize, usize)>),
    /// Remove the most recently registered exception handler.
    TryEnd,
    /// Resume the return or exception that entered a finally handler.
    FinallyResume,
    /// Raise the value currently on the operand stack.
    Throw,
    /// Raise a value with the source line of its throw statement.
    ThrowAt(usize),
    NewObject(
        String,
        usize,
        Vec<Option<ReferenceDescriptor>>,
        Vec<Option<String>>,
        Vec<bool>,
        Option<String>,
        Option<usize>,
        bool,
    ),
    GetProperty(String, Option<String>),
    /// Read a property for `??`, treating an uninitialized typed slot as null.
    GetPropertyCoalesce(String, Option<String>),
    GetDynamicProperty(Option<String>),
    /// Read a computed property for `??` using isset/property-overloading semantics.
    GetDynamicPropertyCoalesce(Option<String>),
    SetDynamicProperty(Option<String>, bool),
    SetProperty(String, Option<String>, bool),
    /// Assign to an instance property and preserve the assigned value as an expression result.
    AssignPropertyExpr(String, Option<String>, bool),
    /// Update an instance property with a compound arithmetic/concatenation operation.
    CompoundAssignProperty(String, CompoundOperation, Option<String>, bool),
    /// Update a property selected by a runtime name with a compound operation.
    CompoundAssignDynamicProperty(CompoundOperation, Option<String>, bool),
    /// Branch past a property coalescing assignment when its current value is non-null.
    CoalescePropertyJump(usize, String, Option<String>),
    /// Assign the right-hand value through property visibility and magic hooks.
    CoalesceAssignProperty(String, Option<String>, bool),
    CoalescePropertyArrayJump(usize, String, usize, Option<String>),
    CoalesceAssignPropertyArray(String, usize, Option<String>, bool),
    AssignPropertyArray(String, usize, Option<String>, bool),
    CompoundAssignPropertyArray(String, usize, CompoundOperation, Option<String>, bool),
    AppendProperty(String, Option<String>, bool),
    AppendPropertyArray(String, usize, Option<String>, bool),
    AppendDynamicPropertyArray(usize, Option<String>, bool),
    AssignStaticPropertyArray(String, String, usize, Option<String>),
    CompoundAssignStaticPropertyArray(String, String, usize, CompoundOperation, Option<String>),
    CoalesceStaticPropertyArrayJump(usize, String, String, usize, Option<String>),
    AppendStaticPropertyArray(String, String, usize, Option<String>),
    DynamicClassName,
    /// Update a static property using a compound arithmetic/concatenation operation.
    CompoundAssignStaticProperty(String, String, CompoundOperation, Option<String>),
    /// Branch past static-property `??=` when its existing slot is non-null.
    CoalesceStaticPropertyJump(usize, String, String, Option<String>),
    /// Store the right-hand value into a static property for `??=`.
    CoalesceAssignStaticProperty(String, String, Option<String>),
    /// Read a static property for `??`, treating an uninitialized typed slot as null.
    GetStaticPropertyCoalesce(String, String, Option<String>),
    /// Test a property without reading it, allowing the VM to dispatch __isset.
    IssetProperty(String, Option<String>),
    IssetChainProperty(String, bool, usize, Option<String>),
    IssetChainDynamicProperty(bool, usize, Option<String>),
    IssetChainArrayOffsets(usize, bool, usize),
    /// Test an overloaded property and then its array offsets without skipping __isset.
    IssetPropertyArray(String, usize, Option<String>),
    IssetStaticProperty(String, String, Option<String>),
    IssetDynamicProperty(Option<String>),
    IssetDynamicPropertyArray(usize, Option<String>),
    /// Continue `isset()` on a non-null value (or true property hook), else jump to false.
    IssetJump(usize, bool),
    /// Copy an object property map and invoke the optional __clone hook.
    CloneObject,
    /// Remove an object property or dispatch the __unset hook.
    UnsetProperty(String, Option<String>),
    GetStaticProperty(String, String, Option<String>),
    GetDynamicStaticProperty(String),
    GetDynamicStaticPropertyExpr,
    AssignDynamicStaticPropertyArray(usize, Option<CompoundOperation>, Option<String>),
    SetStaticProperty(String, String, Option<String>),
    /// Assign to a static property and preserve the value as an expression result.
    AssignStaticPropertyExpr(String, String, Option<String>),
    GetClassConstant(String, String, Option<String>),
    /// Terminate the current script with an optional integer status or string message.
    Exit(Option<String>, bool),
    CallMethod(
        String,
        usize,
        Vec<Option<ReferenceDescriptor>>,
        Vec<Option<String>>,
        Vec<bool>,
        Option<String>,
        bool,
    ),
    CallParentMethod(
        String,
        String,
        usize,
        Vec<Option<ReferenceDescriptor>>,
        Vec<Option<String>>,
        Vec<bool>,
        Option<String>,
        bool,
    ),
    CallStaticMethod(
        String,
        String,
        usize,
        Vec<Option<ReferenceDescriptor>>,
        Vec<Option<String>>,
        Vec<bool>,
        Option<String>,
        bool,
    ),
    Return,
    IterStart(usize, Option<String>),
    /// Snapshot an array variable and convert each iterated slot to a reference cell.
    IterStartReference(usize, String, usize, Option<String>),
    IterNext(usize, Option<String>, String, bool, usize),
    /// Materialize an array or Traversable before argument unpacking.
    ExpandUnpack,
    UnsetVariable(String),
    UnsetArrayPath(String, usize),
    UnsetStaticPropertyArray(String, String, usize, Option<String>),
    UnsetPropertyArray(String, usize, Option<String>),
    UnsetDynamicProperty(Option<String>),
    Jump(usize),
    JumpFalse(usize),
    /// Select a break/continue destination by a runtime nesting level.
    DynamicLoopJump {
        targets: Vec<usize>,
        is_break: bool,
    },
    /// Keep the top stack value and jump if non-null; otherwise discard it and continue.
    CoalesceJump(usize),
}

/// Runtime include/require behavior attached to an include opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncludeMode {
    Include,
    IncludeOnce,
    Require,
    RequireOnce,
}

/// Describes the operands produced by one item in an array literal.
#[derive(Clone, Copy, Debug)]
pub enum ArrayEntryKind {
    Packed,
    Keyed,
    PackedReference,
    KeyedReference,
    Unpack,
}

/// One compiled variable or nested target in a destructuring assignment.
#[derive(Clone, Debug)]
pub struct DestructureTarget {
    pub key: Option<ArrayKey>,
    pub computed_key: bool,
    pub variable: Option<String>,
    pub array_variable: Option<String>,
    pub array_key_count: usize,
    pub array_append: bool,
    pub property_receiver: bool,
    pub property: Option<String>,
    pub property_array_key_count: usize,
    pub nested: Option<Vec<DestructureTarget>>,
    pub by_reference: bool,
}
