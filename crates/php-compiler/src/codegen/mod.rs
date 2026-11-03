//! Compiler code generation shared state and declaration passes.

mod declarations;
mod arrays;
mod expressions;
mod loops;
mod control;
mod statements;

use std::collections::HashMap;

use php_runtime::{bytecode::Op, BytecodeProgram};
use php_syntax::Program;

/// Shared bytecode output and class context used by expression/statement emitters.
pub(super) struct CodeGenerator<'a> {
    /// Parsed declaration table used when compile-time expressions reference class constants.
    pub(super) program: Option<&'a Program>,
    pub(super) instructions: Vec<Op>,
    pub(super) current_source_file: Option<String>,
    pub(super) parent_class: Option<String>,
    pub(super) current_class: Option<String>,
    /// Original class spelling for magic constants.
    pub(super) current_class_display: Option<String>,
    pub(super) current_method: Option<String>,
    /// Current function or method spelling for magic constants.
    pub(super) current_function: Option<String>,
    /// Caller-side scalar argument checking mode for this compiled source unit.
    pub(super) strict_types: bool,
    /// Whether a return expression must preserve an l-value reference cell.
    pub(super) returns_by_reference: bool,
    /// Active loop/switch patch lists used to resolve nested break and continue levels.
    pub(super) control_stack: Vec<ControlContext>,
    /// Label destinations local to this compiled program, function, method, or closure.
    pub(super) labels: HashMap<String, usize>,
    /// Goto jumps awaiting label resolution at the end of this code body.
    pub(super) pending_gotos: Vec<(String, usize)>,
    /// Jump sites for top-level `return;` statements inside an included file scope.
    pub(super) include_return_stack: Vec<Vec<usize>>,
    /// Compile-time control-flow errors such as break outside a breakable construct.
    pub(super) errors: Vec<String>,
}

impl CodeGenerator<'_> {
    pub(super) fn resolve_gotos(&mut self) {
        for (label, instruction) in std::mem::take(&mut self.pending_gotos) {
            if let Some(target) = self.labels.get(&label).copied() {
                self.instructions[instruction] = Op::Jump(target);
            } else {
                self.errors.push(format!("'goto' to undefined label '{label}'"));
            }
        }
    }
}

/// Unresolved break/continue jumps emitted inside one loop or switch body.
#[derive(Default)]
/// Compiler state used to patch the exits of one breakable construct.
pub(super) struct ControlContext {
    pub(super) breaks: Vec<usize>,
    pub(super) continues: Vec<usize>,
    pub(super) dynamic_jumps: Vec<(usize, usize, bool)>,
}

/// Compile the parsed program using the expression, statement, and declaration emitters.
pub(super) fn compile(program: &Program) -> Result<BytecodeProgram, String> {
    CodeGenerator { program: Some(program), instructions: Vec::new(), current_source_file: None, parent_class: None, current_class: None, current_class_display: None, current_method: None, current_function: None, strict_types: program.strict_types, returns_by_reference: false, control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), include_return_stack: Vec::new(), errors: Vec::new() }.generate(program)
}
