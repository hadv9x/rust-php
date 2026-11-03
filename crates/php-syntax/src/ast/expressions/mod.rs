//! Expression nodes for the supported PHP language subset.

#[derive(Clone, Debug)]
pub enum IssetPropertySegment {
    Fixed(String),
    Dynamic(Box<Expr>),
    ArrayOffset(Vec<Expr>),
}

/// PHP expressions represented by the current frontend subset.
#[derive(Clone, Debug)]
/// Expression forms represented in the parsed PHP subset.
pub enum Expr {
    Integer(i64),
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    /// A global or namespaced constant reference.
    Constant(String),
    /// A context-dependent PHP magic constant resolved during code generation.
    MagicConstant(String),
    Variable(String),
    /// A variable whose name comes from another local variable, as in `$$name`.
    DynamicVariable(String),
    /// A variable whose name is computed inside `${...}`.
    DynamicVariableExpr(Box<Expr>),
    Array(Vec<ArrayEntry>),
    ListAssignment { targets: Vec<super::super::DestructureTarget>, value: Box<Expr> },
    Index(Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    /// Argument label and value for PHP named-argument calls.
    NamedArgument(String, Box<Expr>),
    /// Expand an array or Traversable value into a call's argument list.
    UnpackArgument(Box<Expr>),
    NewObject(String, Vec<Expr>, usize),
    DynamicNewObject(Box<Expr>, Vec<Expr>, usize),
    Property(Box<Expr>, String),
    DynamicProperty(Box<Expr>, Box<Expr>),
    DynamicPropertyAssignment { receiver: Box<Expr>, property: Box<Expr>, value: Box<Expr> },
    DynamicPropertyCompoundAssignment { receiver: Box<Expr>, property: Box<Expr>, operation: crate::ast::BinaryOp, value: Box<Expr> },
    /// Property-specific isset form; unlike Property, this must not trigger __get.
    IssetProperty(Box<Expr>, String),
    IssetPropertyChain(Box<Expr>, Vec<IssetPropertySegment>),
    IssetPropertyPath(Box<Expr>, String, Vec<Expr>),
    IssetStaticProperty(String, String),
    IssetDynamicProperty(Box<Expr>, Box<Expr>),
    IssetDynamicPropertyPath(Box<Expr>, Box<Expr>, Vec<Expr>),
    /// Test each variable, array offset, or property and return false if any is null or absent.
    Isset(Vec<Expr>),
    /// Evaluate an expression with PHP's `empty()` semantics.
    Empty(Box<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),
    DynamicMethodCall(Box<Expr>, Box<Expr>, Vec<Expr>),
    ParentMethodCall(String, Vec<Expr>),
    StaticCall(String, String, Vec<Expr>),
    /// Invoke a static method using a class name held in a variable.
    DynamicStaticCall(Box<Expr>, String, Vec<Expr>),
    DynamicStaticCallExpr(Box<Expr>, Box<Expr>, Vec<Expr>),
    DynamicClassName(Box<Expr>),
    DynamicClassConstant(Box<Expr>, String),
    DynamicStaticProperty(Box<Expr>, String),
    DynamicStaticPropertyExpr(Box<Expr>, Box<Expr>),
    StaticProperty(String, String),
    ClassConstant(String, String),
    /// Shallow-copy an object and invoke its optional __clone hook at runtime.
    Clone(Box<Expr>),
    /// Increment an l-value and evaluate to its previous value.
    PostIncrement(IncrementTarget, i8),
    /// Increment an l-value before evaluating to its updated value.
    PreIncrement(IncrementTarget, i8),
    Negate(Box<Expr>),
    Positive(Box<Expr>),
    /// Boolean negation using PHP truthiness.
    Not(Box<Expr>),
    ErrorSuppress(Box<Expr>),
    Cast(CastType, Box<Expr>),
    /// Bitwise complement of an integer or byte string.
    BitwiseNot(Box<Expr>),
    Binary(super::BinaryOp, Box<Expr>, Box<Expr>),
    /// Test an object against a class/interface name or a runtime class-name value.
    InstanceOf(Box<Expr>, Box<Expr>),
    /// Assign a variable within an expression, optionally applying a compound operator.
    Assignment { variable: String, operation: Option<super::BinaryOp>, value: Box<Expr> },
    /// Bind a variable to a reference-returning expression and preserve assignment value semantics.
    ReferenceAssignment { variable: String, value: Box<Expr> },
    /// Assign to a dynamically named variable within an expression.
    DynamicVariableAssignment { name: Box<Expr>, operation: Option<super::BinaryOp>, value: Box<Expr> },
    /// Assign through a nested array path within an expression.
    ArrayAssignment { variable: String, keys: Vec<Expr>, operation: Option<super::BinaryOp>, value: Box<Expr> },
    /// Assign to an instance property within an expression.
    PropertyAssignment { receiver: Box<Expr>, property: String, operation: Option<super::BinaryOp>, value: Box<Expr> },
    /// Assign through nested offsets of an object property.
    PropertyArrayAssignment { receiver: Box<Expr>, property: String, keys: Vec<Expr>, operation: Option<super::BinaryOp>, value: Box<Expr> },
    PropertyAppendAssignment { receiver: Box<Expr>, property: String, keys: Vec<Expr>, value: Box<Expr> },
    DynamicPropertyAppendAssignment { receiver: Box<Expr>, property: Box<Expr>, keys: Vec<Expr>, value: Box<Expr> },
    PropertyAppend(Box<Expr>),
    StaticPropertyArrayAssignment { class: String, property: String, keys: Vec<Expr>, operation: Option<super::BinaryOp>, value: Box<Expr> },
    DynamicStaticPropertyArrayAssignment { class: Box<Expr>, property: Box<Expr>, keys: Vec<Expr>, operation: Option<super::BinaryOp>, value: Box<Expr> },
    ArrayAppendAssignment { variable: String, keys: Vec<Expr>, value: Box<Expr> },
    ArrayExpressionAppendAssignment { array: Box<Expr>, keys: Vec<Expr>, value: Box<Expr> },
    StaticPropertyAppendAssignment { class: String, property: String, keys: Vec<Expr>, value: Box<Expr> },
    /// Assign to a static property within an expression.
    StaticPropertyAssignment { class: String, property: String, operation: Option<super::BinaryOp>, value: Box<Expr> },
    /// Select one branch based on the condition's truthiness.
    Ternary { condition: Box<Expr>, then_value: Box<Expr>, else_value: Box<Expr> },
    /// Select the first strict-value arm, or raise UnhandledMatchError if no arm matches.
    Match { subject: Box<Expr>, arms: Vec<MatchArm> },
    /// Return the already-evaluated condition when truthy, otherwise use the fallback.
    ShorthandTernary { condition: Box<Expr>, else_value: Box<Expr> },
    /// Return the left value unless it is null; otherwise evaluate the right value.
    Coalesce(Box<Expr>, Box<Expr>),
    /// Anonymous function with explicit by-value or by-reference captures.
    Closure { declaration: Box<super::FunctionDecl>, captures: Vec<(String, bool)> },
    /// Invoke an expression that evaluates to a closure value.
    Invoke(Box<Expr>, Vec<Expr>),
    /// Suspend the active generator and yield a value, optionally under an explicit key.
    Yield { key: Option<Box<Expr>>, value: Box<Expr> },
    /// Delegate iteration to an array or generator and evaluate to its return value.
    YieldFrom(Box<Expr>),
    /// Create a first-class callable using PHP's `target(...)` syntax.
    FirstClassCallable(FirstClassCallableTarget),
}

/// One arm of a PHP match expression. The default arm has no conditions.
#[derive(Clone, Debug)]
pub struct MatchArm {
    pub conditions: Vec<Expr>,
    pub value: Expr,
}

/// Callable target forms that can be written with first-class callable syntax.
#[derive(Clone, Debug)]
pub enum FirstClassCallableTarget {
    Value(Box<Expr>),
    Function(String),
    InstanceMethod(Box<Expr>, String),
    StaticMethod(String, String),
}

/// Scalar casts represented by the parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastType { Integer, Float, String, Bool, Array, Object }

/// Variable or nested array path modified by prefix/postfix increment.
#[derive(Clone, Debug)]
pub enum IncrementTarget {
    Variable(String),
    /// A variable-variable target and optional array path rooted at it.
    DynamicVariable { name: Box<Expr>, keys: Vec<Expr> },
    ArrayPath { variable: String, keys: Vec<Expr> },
    PropertyArrayPath { receiver: Box<Expr>, property: String, keys: Vec<Expr> },
    StaticPropertyArrayPath { class: String, property: String, keys: Vec<Expr> },
    DynamicPropertyArrayPath { receiver: Box<Expr>, property: Box<Expr>, keys: Vec<Expr> },
    DynamicStaticPropertyArrayPath { class: Box<Expr>, property: Box<Expr>, keys: Vec<Expr> },
    Property { receiver: Box<Expr>, property: String },
    DynamicProperty { receiver: Box<Expr>, property: Box<Expr> },
    StaticProperty { class: String, property: String },
}

/// One PHP array item; a missing key means the VM assigns the next integer key.
#[derive(Clone, Debug)]
/// An array literal entry with an optional explicit key.
pub struct ArrayEntry {
    pub key: Option<Expr>,
    pub value: Expr,
    pub unpack: bool,
    /// Bind the array slot to the storage named by `value` instead of copying it.
    pub by_reference: bool,
}
