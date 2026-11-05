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
    /// Simple straight-line callable assignments used for compile-time diagnostics.
    pub(super) known_callables: HashMap<String, php_syntax::Expr>,
    /// Declared parameter types for a function/method body, used for internal method signatures.
    pub(super) variable_types: HashMap<String, String>,
    /// Whether a return expression must preserve an l-value reference cell.
    pub(super) returns_by_reference: bool,
    /// Active loop/switch patch lists used to resolve nested break and continue levels.
    pub(super) control_stack: Vec<ControlContext>,
    /// Label destinations local to this compiled program, function, method, or closure.
    pub(super) labels: HashMap<String, (usize, Vec<usize>)>,
    /// Goto jumps awaiting label resolution at the end of this code body.
    pub(super) pending_gotos: Vec<(String, usize, Vec<usize>)>,
    /// Unique ids for loop/switch regions, used to validate goto targets.
    pub(super) next_control_id: usize,
    /// Jump sites for top-level `return;` statements inside an included file scope.
    pub(super) include_return_stack: Vec<Vec<usize>>,
    /// Compile-time control-flow errors such as break outside a breakable construct.
    pub(super) errors: Vec<String>,
}

pub(super) fn declared_parameter_types(parameters: &[String], types: &[Option<String>]) -> HashMap<String, String> {
    parameters.iter().zip(types).filter_map(|(parameter, ty)| {
        ty.as_ref().map(|ty| (parameter.clone(), ty.trim_start_matches('\\').to_ascii_lowercase()))
    }).collect()
}

impl CodeGenerator<'_> {
    pub(super) fn resolve_gotos(&mut self) {
        for (label, instruction, source_path) in std::mem::take(&mut self.pending_gotos) {
            if let Some((target, target_path)) = self.labels.get(&label).cloned() {
                if !source_path.starts_with(&target_path) {
                    self.errors.push("Cannot 'goto' into loop or switch statement".into());
                } else {
                    self.instructions[instruction] = Op::Jump(target);
                }
            } else {
                self.errors.push(format!("'goto' to undefined label '{label}'"));
            }
        }
    }
}

impl CodeGenerator<'_> {
    pub(super) fn push_control_context(&mut self) {
        let id = self.next_control_id;
        self.next_control_id += 1;
        self.control_stack.push(ControlContext { id, ..ControlContext::default() });
    }

    pub(super) fn control_path(&self) -> Vec<usize> {
        self.control_stack.iter().map(|context| context.id).collect()
    }
}

/// Unresolved break/continue jumps emitted inside one loop or switch body.
#[derive(Default)]
/// Compiler state used to patch the exits of one breakable construct.
pub(super) struct ControlContext {
    pub(super) id: usize,
    pub(super) breaks: Vec<usize>,
    pub(super) continues: Vec<usize>,
    pub(super) dynamic_jumps: Vec<(usize, usize, bool)>,
}

/// Compile the parsed program using the expression, statement, and declaration emitters.
pub(super) fn compile(program: &Program) -> Result<BytecodeProgram, String> {
    CodeGenerator { program: Some(program), instructions: Vec::new(), current_source_file: None, parent_class: None, current_class: None, current_class_display: None, current_method: None, current_function: None, strict_types: program.strict_types, known_callables: HashMap::new(), variable_types: HashMap::new(), returns_by_reference: false, control_stack: Vec::new(), labels: HashMap::new(), pending_gotos: Vec::new(), next_control_id: 0, include_return_stack: Vec::new(), errors: Vec::new() }.generate(program)
}
