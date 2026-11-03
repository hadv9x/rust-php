//! Bytecode generation and jump resolution for PHP loop statements.

use php_runtime::bytecode::Op;
use php_syntax::Stmt;
use super::{CodeGenerator, ControlContext};

fn variable_array_path(expression: &php_syntax::Expr) -> Option<(String, Vec<&php_syntax::Expr>)> {
    match expression {
        php_syntax::Expr::Variable(variable) => Some((variable.clone(), Vec::new())),
        php_syntax::Expr::Index(array, key) => {
            let (variable, mut keys) = variable_array_path(array)?;
            keys.push(key);
            Some((variable, keys))
        }
        _ => None,
    }
}

impl CodeGenerator<'_> {
    pub(super) fn loop_jump_expr(&mut self, is_break: bool, level: &php_syntax::Expr) {
        match level {
            php_syntax::Expr::Integer(level) if *level > 0 => self.loop_jump(is_break, *level as usize),
            php_syntax::Expr::Integer(_) | php_syntax::Expr::Number(_) => self.errors.push("break/continue level must be a positive integer".into()),
            _ => {
                if self.control_stack.is_empty() {
                    self.errors.push(if is_break { "break used outside a loop or switch" } else { "continue used outside a loop or switch" }.into());
                    return;
                }
                self.expression(level);
                let instruction = self.instructions.len();
                let depth = self.control_stack.len();
                self.instructions.push(Op::DynamicLoopJump { targets: vec![usize::MAX; depth], is_break });
                for (context_index, context) in self.control_stack.iter_mut().enumerate() {
                    context.dynamic_jumps.push((instruction, depth - context_index - 1, is_break));
                }
            }
        }
    }

    /// Emit loop setup, exit jumps, iteration steps, and correctly nested control targets.
    pub(super) fn loop_statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::While { condition, body } => {
                let condition_start = self.instructions.len();
                self.expression(condition);
                let exit_jump = self.instructions.len();
                self.instructions.push(Op::JumpFalse(0));
                self.control_stack.push(ControlContext::default());
                self.statements(body);
                self.instructions.push(Op::Jump(condition_start));
                let end = self.instructions.len();
                self.instructions[exit_jump] = Op::JumpFalse(end);
                self.finish_loop(condition_start, end);
            }
            Stmt::DoWhile { condition, body } => {
                let body_start = self.instructions.len();
                self.control_stack.push(ControlContext::default());
                self.statements(body);
                let condition_start = self.instructions.len();
                self.expression(condition);
                let exit_jump = self.instructions.len();
                self.instructions.push(Op::JumpFalse(0));
                self.instructions.push(Op::Jump(body_start));
                let end = self.instructions.len();
                self.instructions[exit_jump] = Op::JumpFalse(end);
                self.finish_loop(condition_start, end);
            }
            Stmt::For { initializers, condition, updates, body } => {
                for initializer in initializers { self.statement(initializer); }
                let condition_start = self.instructions.len();
                let exit_jump = condition.as_ref().map(|condition| {
                    self.expression(condition);
                    let jump = self.instructions.len();
                    self.instructions.push(Op::JumpFalse(0));
                    jump
                });
                self.control_stack.push(ControlContext::default());
                self.statements(body);
                let update_start = self.instructions.len();
                for update in updates { self.statement(update); }
                self.instructions.push(Op::Jump(condition_start));
                let end = self.instructions.len();
                if let Some(exit_jump) = exit_jump { self.instructions[exit_jump] = Op::JumpFalse(end); }
                self.finish_loop(update_start, end);
            }
            Stmt::Foreach { iterable, key_name, value_name, value_by_reference, body, .. } => {
                let iterator_id = self.instructions.len();
                let reference_path = if *value_by_reference { variable_array_path(iterable) } else { None };
                let uses_references = reference_path.is_some();
                if let Some((variable, keys)) = reference_path {
                    self.instructions.push(Op::Load(variable.clone()));
                    for key in &keys { self.expression(key); }
                    self.instructions.push(Op::IterStartReference(iterator_id, variable, keys.len()));
                } else {
                    self.expression(iterable);
                    self.instructions.push(Op::IterStart(iterator_id));
                }
                let next_item = self.instructions.len();
                self.instructions.push(Op::IterNext(iterator_id, key_name.clone(), value_name.clone(), uses_references, 0));
                self.control_stack.push(ControlContext::default());
                self.statements(body);
                self.instructions.push(Op::Jump(next_item));
                let end = self.instructions.len();
                self.instructions[next_item] = Op::IterNext(iterator_id, key_name.clone(), value_name.clone(), uses_references, end);
                self.finish_loop(next_item, end);
            }
            _ => unreachable!("loop_statement only handles loop AST nodes"),
        }
    }

    /// Emit an unresolved jump for break/continue, or record an invalid out-of-loop use.
    pub(super) fn loop_jump(&mut self, is_break: bool, level: usize) {
        if level == 0 || level > self.control_stack.len() {
            self.errors.push(if is_break { "break level exceeds the active breakable depth" } else { "continue level exceeds the active breakable depth" }.into());
            return;
        }
        let jump = self.instructions.len();
        self.instructions.push(Op::Jump(0));
        let target_index = self.control_stack.len() - level;
        let context = &mut self.control_stack[target_index];
        if is_break { context.breaks.push(jump); } else { context.continues.push(jump); }
    }

    /// Patch pending break jumps to loop exit and continue jumps to the next iteration point.
    fn finish_loop(&mut self, continue_target: usize, break_target: usize) {
        let context = self.control_stack.pop().expect("each emitted loop pushes one control context");
        for jump in context.breaks { self.instructions[jump] = Op::Jump(break_target); }
        for jump in context.continues { self.instructions[jump] = Op::Jump(continue_target); }
        for (instruction, level_slot, is_break) in context.dynamic_jumps {
            if let Op::DynamicLoopJump { targets, .. } = &mut self.instructions[instruction] {
                targets[level_slot] = if is_break { break_target } else { continue_target };
            }
        }
    }
}
