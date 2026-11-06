//! Statement, assignment-target, and operator syntax nodes.

use super::super::Expr;

/// L-values accepted by the current assignment grammar.
#[derive(Clone, Debug)]
/// The supported variable, property, and array assignment destinations.
pub enum AssignTarget {
    Variable(String),
    DynamicVariable(String),
    DynamicVariableExpr(Expr),
    Index { variable: String, keys: Vec<Expr> },
    Append { variable: String, keys: Vec<Expr> },
    ArrayPath { variable: String, segments: Vec<ArrayPathSegment> },
    Property { object: String, property: String },
    StaticProperty { class: String, property: String },
}

/// One key or append operation in an array assignment destination.
#[derive(Clone, Debug)]
pub enum ArrayPathSegment { Key(Expr), Append }

/// Operators retain their meaning after parser precedence has been resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Binary operators represented in the parsed expression tree.
pub enum BinaryOp {
    Add, Subtract, Multiply, Divide, Power, Modulo, Concatenate, BitAnd, BitXor, BitOr, ShiftLeft, ShiftRight, Coalesce,
    Equal, NotEqual, StrictEqual, StrictNotEqual, Less, LessEqual, Greater, GreaterEqual, Spaceship, LogicalAnd, LogicalOr, LogicalXor,
}

/// Executable statements represented by the currently supported PHP subset.
#[derive(Clone, Debug)]
/// Statement forms represented in the parsed PHP subset.
pub enum Stmt {
    /// Attach the source line of a statement for runtime diagnostics and traces.
    SourceLine(usize, Box<Stmt>),
    /// Execute statements while attributing their source markers to a PHP file.
    SourceFile(String, bool, Vec<Stmt>),
    /// Execute an included file's top-level statements with local `return;` exits.
    IncludeScope(Vec<Stmt>),
    /// Exit the current included file without returning from its caller.
    IncludeReturn,
    /// Evaluate an included file's return expression, then exit that file scope.
    IncludeReturnValue(Expr),
    /// Include a statically resolvable PHP source file at this statement position.
    Include { kind: IncludeKind, path: Expr },
    Echo(Vec<Expr>),
    FunctionDeclaration(super::super::FunctionDecl),
    ClassDeclaration(super::super::ClassDecl),
    InterfaceDeclaration(super::super::InterfaceDecl),
    TraitDeclaration(super::super::TraitDecl),
    Expression(Expr),
    Assign { target: AssignTarget, value: Expr },
    /// Assign selected array keys into positional or keyed destructuring targets.
    ListAssign { targets: Vec<DestructureTarget>, value: Expr },
    /// Bind a simple variable name to a variable or array element storage cell.
    ReferenceAssign { target: String, source: Expr },
    /// Bind an array element to a variable or another array element storage cell.
    ArrayReferenceAssign { target: String, target_keys: Vec<Expr>, source: String, source_keys: Vec<Expr> },
    /// Bind an array element to an arbitrary reference-returning expression.
    ArrayReferenceAssignExpr { target: String, target_keys: Vec<Expr>, source: Expr },
    /// Bind one static-property array element to another static-property array element.
    StaticPropertyArrayReferenceAssign { target_class: String, target_property: String, target_keys: Vec<Expr>, source_class: String, source_property: String, source_keys: Vec<Expr> },
    /// Bind a dynamic object property to a reference-returning property expression.
    DynamicPropertyReferenceAssign { receiver: Expr, property: Expr, source: Expr },
    /// Bind an array offset rooted at a computed object property to a variable/reference path.
    DynamicPropertyArrayReferenceAssign { receiver: Expr, property: Expr, target_keys: Vec<Expr>, source: String, source_keys: Vec<Expr> },
    /// Bind an appended property-array slot to a reference-returning property.
    PropertyAppendReferenceAssign { receiver: Expr, property: String, source: Expr },
    /// Bind an array element in an instance property to a variable or array-element reference.
    PropertyArrayReferenceAssign { receiver: Expr, property: String, target_keys: Vec<Expr>, source: String, source_keys: Vec<Expr> },
    /// Bind the listed names to the shared program-global variable table.
    Global(Vec<String>),
    /// Declare function-local variables whose values persist across calls.
    Static(Vec<StaticVariable>),
    /// Apply a supported arithmetic or concatenation assignment to a local variable.
    CompoundAssign { target: AssignTarget, operation: BinaryOp, value: Expr },
    If { condition: Expr, then_body: Vec<Stmt>, else_body: Vec<Stmt> },
    /// Select a case by PHP-style loose comparison and execute with fall-through.
    Switch { value: Expr, cases: Vec<SwitchCase> },
    /// Run a body and route an explicit thrown value into a catch variable.
    TryCatch { try_body: Vec<Stmt>, catches: Vec<CatchClause>, finally_body: Option<Vec<Stmt>> },
    /// Run cleanup after a try body even when no catch clause is declared.
    TryFinally { try_body: Vec<Stmt>, finally_body: Vec<Stmt> },
    /// Raise a PHP value that can be caught by a surrounding try statement.
    Throw { value: Expr, line: usize },
    /// Terminate the script, optionally with a PHP exit status or message.
    Exit(Option<Expr>),
    While { condition: Expr, body: Vec<Stmt> },
    DoWhile { condition: Expr, body: Vec<Stmt> },
    For { initializers: Vec<Stmt>, condition: Option<Expr>, updates: Vec<Stmt>, body: Vec<Stmt> },
    Foreach { iterable: Expr, key_name: Option<String>, key_property: Option<(String, String)>, value_name: String, value_property: Option<(String, String)>, value_by_reference: bool, body: Vec<Stmt> },
    /// Exit the requested nesting level, where one means the innermost loop.
    Break(Expr),
    /// Continue at the requested nesting level, where one means the innermost loop.
    Continue(Expr),
    /// Jump to a label in the current PHP function or top-level scope.
    Goto(String),
    /// Mark a destination for a `goto` in the current PHP function or top-level scope.
    Label(String),
    Unset { variable: String, keys: Vec<Expr> },
    UnsetMany(Vec<Stmt>),
    UnsetStaticPropertyPath { class: String, property: String, keys: Vec<Expr> },
    UnsetPropertyArrayPath { object: Expr, property: String, keys: Vec<Expr> },
    UnsetDynamicProperty { object: Expr, property: Expr },
    /// Object property unset, which may dispatch to the class's __unset hook.
    UnsetProperty { object: String, property: String },
    Return(Option<Expr>),
}

/// One ordered PHP catch clause; each clause may handle a union of class types.
#[derive(Clone, Debug)]
pub struct CatchClause {
    pub types: Vec<String>,
    pub variable: Option<String>,
    pub body: Vec<Stmt>,
}

/// PHP file inclusion mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IncludeKind { Include, IncludeOnce, Require, RequireOnce }

/// One variable, ignored slot, or nested target in destructuring syntax.
#[derive(Clone, Debug)]
pub struct DestructureTarget {
    /// `None` advances from the next implicit integer key.
    pub key: Option<DestructureKey>,
    /// A missing variable represents an ignored slot.
    pub variable: Option<String>,
    /// A writable array offset target such as `$values[$index]`.
    pub array_variable: Option<String>,
    /// Offset expressions evaluated before the destructuring operation runs.
    pub array_keys: Vec<Expr>,
    /// Whether the destination appends instead of assigning an explicit offset.
    pub array_append: bool,
    /// Receiver expression for a target such as `$objects[$i]->property`.
    pub property_receiver: Option<Expr>,
    /// Property name paired with `property_receiver`.
    pub property: Option<String>,
    /// Nested `list()` or short-array targets selected from this slot.
    pub nested: Option<Vec<DestructureTarget>>,
    /// Whether the selected source slot is bound to a variable by reference.
    pub by_reference: bool,
}

/// Literal array key accepted by the destructuring grammar.
#[derive(Clone, Debug)]
pub enum DestructureKey {
    Integer(i64),
    String(String),
    /// A key expression evaluated while destructuring an array.
    Computed(Box<Expr>),
}

/// One `case` or `default` arm in a switch statement.
#[derive(Clone, Debug)]
pub struct SwitchCase {
    /// `None` marks the default arm; other values are compared to the switch value.
    pub condition: Option<Expr>,
    /// Statements execute from this arm until a break or the switch ends.
    pub body: Vec<Stmt>,
}

/// One function-local static variable and its optional initializer expression.
#[derive(Clone, Debug)]
pub struct StaticVariable {
    pub name: String,
    pub initial: Option<Expr>,
}
