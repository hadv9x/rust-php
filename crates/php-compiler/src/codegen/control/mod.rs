//! Bytecode generation for switch dispatch and fall-through case bodies.

use php_runtime::bytecode::Op;
use php_syntax::Stmt;

use super::CodeGenerator;

impl CodeGenerator<'_> {
    /// Compare the switch value against each case, then emit bodies in source order for fall-through.
    pub(super) fn switch_statement(&mut self, statement: &Stmt) {
        let Stmt::Switch { value, cases } = statement else {
            unreachable!("switch_statement only handles switch AST nodes")
        };
        self.expression(value);

        // Each match branch keeps one copy of the switch value on the stack for later comparisons.
        let mut case_jumps = Vec::new();
        let mut default_index = None;
        for (index, case) in cases.iter().enumerate() {
            if let Some(condition) = &case.condition {
                self.instructions.push(Op::Dup);
                self.expression(condition);
                self.instructions.push(Op::Eq);
                let next_case = self.instructions.len();
                self.instructions.push(Op::JumpFalse(0));
                self.instructions.push(Op::Pop);
                let body_jump = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                self.instructions[next_case] = Op::JumpFalse(self.instructions.len());
                case_jumps.push((index, body_jump));
            } else {
                default_index = Some(index);
            }
        }
        self.instructions.push(Op::Pop);
        let default_jump = self.instructions.len();
        self.instructions.push(Op::Jump(0));

        self.push_control_context();
        let mut body_positions = Vec::with_capacity(cases.len());
        for case in cases {
            body_positions.push(self.instructions.len());
            self.statements(&case.body);
        }
        let end = self.instructions.len();
        for (case_index, jump) in case_jumps {
            self.instructions[jump] = Op::Jump(body_positions[case_index]);
        }
        let default_target = default_index.map_or(end, |index| body_positions[index]);
        self.instructions[default_jump] = Op::Jump(default_target);
        self.finish_control(end, end);
    }

    /// Patch switch-owned break and continue jumps to the end of this switch.
    fn finish_control(&mut self, continue_target: usize, break_target: usize) {
        let context = self
            .control_stack
            .pop()
            .expect("each switch pushes one control context");
        for jump in context.breaks {
            self.instructions[jump] = Op::Jump(break_target);
        }
        for jump in context.continues {
            self.instructions[jump] = Op::Jump(continue_target);
        }
        for (instruction, level_slot, is_break) in context.dynamic_jumps {
            if let Op::DynamicLoopJump { targets, .. } = &mut self.instructions[instruction] {
                targets[level_slot] = if is_break {
                    break_target
                } else {
                    continue_target
                };
            }
        }
    }
}
