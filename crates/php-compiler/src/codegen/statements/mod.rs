//! Bytecode generation for PHP statements and control flow.

use php_runtime::{bytecode::Op, Value};
use php_syntax::{AssignTarget, Expr, Stmt};

use super::CodeGenerator;

fn reference_path(expression: &Expr) -> Option<(String, Vec<&Expr>)> {
    match expression {
        Expr::Variable(variable) => Some((variable.clone(), Vec::new())),
        Expr::Index(array, key) => {
            let (variable, mut keys) = reference_path(array)?;
            keys.push(key);
            Some((variable, keys))
        }
        _ => None,
    }
}

impl CodeGenerator<'_> {
    /// Generate bytecode for each statement in source order and propagate code-generation errors.
    pub(super) fn statements(&mut self, statements: &[Stmt]) {
        for statement in statements { self.statement(statement); }
    }

    /// Emit stack-machine instructions for one statement node.
    pub(super) fn statement(&mut self, statement: &Stmt) {
        match statement {
            Stmt::SourceLine(line, statement) => {
                self.instructions.push(Op::SetLine(*line));
                self.statement(statement);
            }
            Stmt::SourceFile(file, strict_types, body) => {
                let previous = self.current_source_file.replace(file.clone());
                let previous_strict_types = self.strict_types;
                self.strict_types = *strict_types;
                self.instructions.push(Op::SetFile(file.clone()));
                self.statements(body);
                self.strict_types = previous_strict_types;
                if let Some(previous) = previous {
                    self.instructions.push(Op::SetFile(previous.clone()));
                    self.current_source_file = Some(previous);
                } else {
                    self.current_source_file = None;
                }
            }
            Stmt::IncludeScope(body) => {
                self.include_return_stack.push(Vec::new());
                self.statements(body);
                let target = self.instructions.len();
                for instruction in self.include_return_stack.pop().unwrap_or_default() {
                    self.instructions[instruction] = Op::Jump(target);
                }
            }
            Stmt::IncludeReturn => {
                let instruction = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                if let Some(jumps) = self.include_return_stack.last_mut() { jumps.push(instruction); }
                else { self.errors.push("include return outside an included-file scope".into()); }
            }
            Stmt::IncludeReturnValue(value) => {
                self.expression(value);
                self.instructions.push(Op::Pop);
                let instruction = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                if let Some(jumps) = self.include_return_stack.last_mut() { jumps.push(instruction); }
                else { self.errors.push("include return outside an included-file scope".into()); }
            }
            Stmt::FunctionDeclaration(_) | Stmt::ClassDeclaration(_) | Stmt::InterfaceDeclaration(_) | Stmt::TraitDeclaration(_) => {},
            Stmt::Include { .. } => self.errors.push("include/require needs a source file context; use compile_file()".into()),
            Stmt::Echo(expressions) => {
                for expression in expressions {
                    self.expression(expression);
                    self.instructions.push(Op::Echo);
                }
            }
            Stmt::Expression(expression) => {
                self.expression(expression);
                self.instructions.push(Op::Pop);
            }
            Stmt::Assign { target, value } => match target {
                AssignTarget::Variable(name) => {
                    if name == "GLOBALS" {
                        self.errors.push("Cannot re-assign $GLOBALS".into());
                        return;
                    }
                    self.expression(value);
                    self.instructions.push(Op::Store(name.clone()));
                }
                AssignTarget::DynamicVariable(name) => {
                    self.expression(value);
                    self.instructions.push(Op::StoreDynamic(name.clone()));
                }
                AssignTarget::DynamicVariableExpr(name) => {
                    self.expression(name);
                    self.expression(value);
                    self.instructions.push(Op::StoreDynamicExpr);
                }
                AssignTarget::Index { variable, keys } => {
                    self.instructions.push(Op::Load(variable.clone()));
                    for key in keys { self.expression(key); }
                    self.expression(value);
                    self.instructions.push(Op::ArraySetPath(keys.len()));
                    self.instructions.push(Op::StoreArrayRoot(variable.clone()));
                }
                AssignTarget::Append { variable, keys } => {
                    self.instructions.push(Op::Load(variable.clone()));
                    for key in keys { self.expression(key); }
                    self.expression(value);
                    if keys.is_empty() {
                        self.instructions.push(Op::ArrayAppend);
                    } else {
                        self.instructions.push(Op::ArrayAppendPath(keys.len()));
                    }
                    self.instructions.push(Op::StoreArrayRoot(variable.clone()));
                }
                AssignTarget::ArrayPath { variable, segments } => {
                    self.instructions.push(Op::Load(variable.clone()));
                    for segment in segments {
                        if let php_syntax::ArrayPathSegment::Key(key) = segment { self.expression(key); }
                    }
                    self.expression(value);
                    self.instructions.push(Op::ArraySetMixedPath(segments.iter().map(|segment| matches!(segment, php_syntax::ArrayPathSegment::Append)).collect()));
                    self.instructions.push(Op::StoreArrayRoot(variable.clone()));
                }
                AssignTarget::Property { object, property } => {
                    self.instructions.push(Op::Load(object.clone()));
                    self.expression(value);
                    self.instructions.push(Op::SetProperty(property.clone(), self.current_class.clone(), self.current_method.as_deref() == Some("__construct")));
                }
                AssignTarget::StaticProperty { class, property } => {
                    self.expression(value);
                    let class = match class.as_str() {
                        "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                        "static" => "static".into(),
                        _ => class.to_ascii_lowercase(),
                    };
                    self.instructions.push(Op::SetStaticProperty(class, property.clone(), self.current_class.clone()));
                }
            },
            Stmt::ListAssign { targets, value } => {
                self.expression(value);
                let mut target_key_count = 0;
                let targets = targets.iter().map(|target| compile_destructure_target(self, target, &mut target_key_count)).collect();
                self.instructions.push(Op::ListDestructure(targets, target_key_count, self.current_class.clone(), self.current_method.as_deref() == Some("__construct")));
            }
            Stmt::Global(variables) => {
                for variable in variables { self.instructions.push(Op::DeclareGlobal(variable.clone())); }
            }
            Stmt::ReferenceAssign { target, source } => {
                if target == "GLOBALS" || matches!(source, Expr::Variable(name) if name == "GLOBALS") {
                    self.errors.push("Cannot create a reference to $GLOBALS itself".into());
                    return;
                }
                if let php_syntax::Expr::Property(receiver, property) = source {
                    self.expression(receiver);
                    self.instructions.push(Op::LoadPropertyReference(property.clone(), self.current_class.clone()));
                    self.instructions.push(Op::BindReturnedReference(target.clone()));
                } else if let Some((class, property, keys)) = super::expressions::static_property_path(source) {
                    let class = match class {
                        "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                        "parent" => self.parent_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                        "static" => "static".into(),
                        _ => class.to_ascii_lowercase(),
                    };
                    self.instructions.push(Op::LoadStaticPropertyReference(class, property.to_string(), self.current_class.clone()));
                    for key in &keys { self.expression(key); }
                    if !keys.is_empty() { self.instructions.push(Op::LoadStaticPropertyArrayReference(keys.len())); }
                    self.instructions.push(Op::BindReturnedReference(target.clone()));
                } else if let Some((source, keys)) = reference_path(source) {
                    if keys.is_empty() {
                        self.instructions.push(Op::AssignReference(target.clone(), source));
                    } else {
                        for key in &keys { self.expression(key); }
                        self.instructions.push(Op::AssignArrayReference(target.clone(), source, keys.len()));
                    }
                } else {
                    self.expression(source);
                    self.instructions.push(Op::BindReturnedReference(target.clone()));
                }
            }
            Stmt::ArrayReferenceAssign { target, target_keys, source, source_keys } => {
                for key in target_keys { self.expression(key); }
                for key in source_keys { self.expression(key); }
                self.instructions.push(Op::BindArrayReference(target.clone(), target_keys.len(), source.clone(), source_keys.len()));
            }
            Stmt::ArrayReferenceAssignExpr { target, target_keys, source } => {
                for key in target_keys { self.expression(key); }
                if let Expr::Property(receiver, property) = source {
                    self.expression(receiver);
                    self.instructions.push(Op::LoadPropertyReference(property.clone(), self.current_class.clone()));
                    self.instructions.push(Op::BindArrayReturnedReference(target.clone(), target_keys.len()));
                } else {
                    self.errors.push("Array reference assignment source must be a reference-returning property".into());
                }
            }
            Stmt::StaticPropertyArrayReferenceAssign { target_class, target_property, target_keys, source_class, source_property, source_keys } => {
                for key in target_keys { self.expression(key); }
                for key in source_keys { self.expression(key); }
                let class_name = |class: &str, current: &Option<String>| match class {
                    "self" => current.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                    "static" => "static".to_string(),
                    _ => class.to_ascii_lowercase(),
                };
                self.instructions.push(Op::BindStaticPropertyArrayReference(
                    class_name(target_class, &self.current_class), target_property.clone(), target_keys.len(),
                    class_name(source_class, &self.current_class), source_property.clone(), source_keys.len(),
                    self.current_class.clone(),
                ));
            }
            Stmt::DynamicPropertyReferenceAssign { receiver, property, source } => {
                self.expression(receiver);
                self.expression(property);
                if let Expr::Property(source_receiver, source_property) = source {
                    self.expression(source_receiver);
                    self.instructions.push(Op::LoadPropertyReference(source_property.clone(), self.current_class.clone()));
                    self.instructions.push(Op::BindDynamicPropertyReference);
                } else {
                    self.errors.push("Dynamic property reference assignment requires a reference-returning property".into());
                }
            }
            Stmt::DynamicPropertyArrayReferenceAssign { receiver, property, target_keys, source, source_keys } => {
                self.expression(receiver);
                self.expression(property);
                for key in target_keys { self.expression(key); }
                for key in source_keys { self.expression(key); }
                self.instructions.push(Op::BindDynamicPropertyArrayReference(
                    target_keys.len(), source.clone(), source_keys.len(), self.current_class.clone(),
                    self.current_method.as_deref() == Some("__construct"),
                ));
            }
            Stmt::PropertyAppendReferenceAssign { receiver, property, source } => {
                self.expression(receiver);
                if let Expr::Property(source_receiver, source_property) = source {
                    self.expression(source_receiver);
                    self.instructions.push(Op::LoadPropertyReference(source_property.clone(), self.current_class.clone()));
                    self.instructions.push(Op::BindPropertyAppendReference(property.clone(), self.current_class.clone(), self.current_method.as_deref() == Some("__construct")));
                } else {
                    self.errors.push("Property array append reference requires a reference-returning property".into());
                }
            }
            Stmt::PropertyArrayReferenceAssign { receiver, property, target_keys, source, source_keys } => {
                self.expression(receiver);
                for key in target_keys { self.expression(key); }
                for key in source_keys { self.expression(key); }
                self.instructions.push(Op::BindPropertyArrayReference(
                    property.clone(), target_keys.len(), source.clone(), source_keys.len(),
                    self.current_class.clone(), self.current_method.as_deref() == Some("__construct"),
                ));
            }
            Stmt::Static(variables) => {
                for variable in variables {
                    let initial = match &variable.initial {
                        None => Value::Null,
                        Some(php_syntax::Expr::Null) => Value::Null,
                        Some(php_syntax::Expr::Bool(value)) => Value::Bool(*value),
                        Some(php_syntax::Expr::Integer(value)) => Value::Integer(*value),
                        Some(php_syntax::Expr::Number(value)) => Value::Number(*value),
                        Some(php_syntax::Expr::String(value)) => Value::String(value.clone()),
                        Some(php_syntax::Expr::Negate(value)) => match value.as_ref() {
                            php_syntax::Expr::Integer(number) => Value::Integer(-number),
                            php_syntax::Expr::Number(number) => Value::Number(-number),
                            _ => {
                                let check = self.instructions.len();
                                self.instructions.push(Op::StaticInitializerCheck(variable.name.clone(), 0));
                                self.expression(variable.initial.as_ref().expect("initializer expression exists"));
                                self.instructions.push(Op::InitializeStatic(variable.name.clone()));
                                let end = self.instructions.len();
                                self.instructions[check] = Op::StaticInitializerCheck(variable.name.clone(), end);
                                continue;
                            }
                        },
                        Some(initializer) => {
                            let check = self.instructions.len();
                            self.instructions.push(Op::StaticInitializerCheck(variable.name.clone(), 0));
                            self.expression(initializer);
                            self.instructions.push(Op::InitializeStatic(variable.name.clone()));
                            let end = self.instructions.len();
                            self.instructions[check] = Op::StaticInitializerCheck(variable.name.clone(), end);
                            continue;
                        }
                    };
                    self.instructions.push(Op::DeclareStatic(variable.name.clone(), initial));
                }
            }
            Stmt::CompoundAssign { target, operation, value } => {
                if matches!(target, AssignTarget::Variable(name) if name == "GLOBALS") {
                    self.errors.push("Cannot re-assign $GLOBALS".into());
                    return;
                }
                if *operation == php_syntax::BinaryOp::Coalesce {
                    match target {
                        AssignTarget::Variable(variable) => {
                            self.instructions.push(Op::LoadSilent(variable.clone()));
                            let present_branch = self.instructions.len();
                            self.instructions.push(Op::CoalesceJump(0));
                            self.expression(value);
                            self.instructions.push(Op::Dup);
                            self.instructions.push(Op::Store(variable.clone()));
                            let end = self.instructions.len();
                            self.instructions[present_branch] = Op::CoalesceJump(end);
                            self.instructions.push(Op::Pop);
                        }
                        AssignTarget::Index { variable, keys } => {
                            self.instructions.push(Op::Load(variable.clone()));
                            for key in keys { self.expression(key); }
                            let present_branch = self.instructions.len();
                            self.instructions.push(Op::CoalescePathJump(0, keys.len()));
                            self.expression(value);
                            self.instructions.push(Op::CoalesceAssignPath(variable.clone(), keys.len()));
                            self.instructions.push(Op::Pop);
                            let end = self.instructions.len();
                            self.instructions[present_branch] = Op::CoalescePathJump(end, keys.len());
                        }
                        AssignTarget::Property { object, property } => {
                            self.instructions.push(Op::Load(object.clone()));
                            let present_branch = self.instructions.len();
                            self.instructions.push(Op::CoalescePropertyJump(0, property.clone(), self.current_class.clone()));
                            self.expression(value);
                            self.instructions.push(Op::CoalesceAssignProperty(
                                property.clone(), self.current_class.clone(),
                                self.current_method.as_deref() == Some("__construct"),
                            ));
                            self.instructions.push(Op::Pop);
                            let end = self.instructions.len();
                            self.instructions[present_branch] = Op::CoalescePropertyJump(end, property.clone(), self.current_class.clone());
                        }
                        AssignTarget::StaticProperty { class, property } => {
                            let class = match class.as_str() {
                                "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                                "static" => "static".into(),
                                _ => class.to_ascii_lowercase(),
                            };
                            let present_branch = self.instructions.len();
                            self.instructions.push(Op::CoalesceStaticPropertyJump(0, class.clone(), property.clone(), self.current_class.clone()));
                            self.expression(value);
                            self.instructions.push(Op::CoalesceAssignStaticProperty(class.clone(), property.clone(), self.current_class.clone()));
                            let pop = self.instructions.len();
                            self.instructions.push(Op::Pop);
                            self.instructions[present_branch] = Op::CoalesceStaticPropertyJump(pop, class, property.clone(), self.current_class.clone());
                        }
                        _ => self.errors.push("Coalescing assignment currently requires a variable or array element".into()),
                    }
                    return;
                }
                match target {
                    AssignTarget::Variable(variable) => {
                        self.instructions.push(Op::Load(variable.clone()));
                        self.expression(value);
                        self.instructions.push(match operation {
                            php_syntax::BinaryOp::Add => Op::Add,
                            php_syntax::BinaryOp::Subtract => Op::Sub,
                            php_syntax::BinaryOp::Multiply => Op::Mul,
                            php_syntax::BinaryOp::Power => Op::Pow,
                            php_syntax::BinaryOp::Divide => Op::Div,
                            php_syntax::BinaryOp::Modulo => Op::Mod,
                            php_syntax::BinaryOp::Concatenate => Op::Concat,
                            php_syntax::BinaryOp::BitAnd => Op::BitAnd,
                            php_syntax::BinaryOp::BitXor => Op::BitXor,
                            php_syntax::BinaryOp::BitOr => Op::BitOr,
                            php_syntax::BinaryOp::ShiftLeft => Op::ShiftLeft,
                            php_syntax::BinaryOp::ShiftRight => Op::ShiftRight,
                            _ => unreachable!("unsupported compound assignment operator"),
                        });
                        self.instructions.push(Op::Store(variable.clone()));
                    }
                    AssignTarget::Index { variable, keys } => {
                        self.instructions.push(Op::Load(variable.clone()));
                        for key in keys { self.expression(key); }
                        self.expression(value);
                        let operation = match operation {
                            php_syntax::BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                            php_syntax::BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                            php_syntax::BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                            php_syntax::BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                            php_syntax::BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                            php_syntax::BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                            php_syntax::BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                            php_syntax::BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                            php_syntax::BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                            php_syntax::BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                            php_syntax::BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                            php_syntax::BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                            _ => unreachable!("unsupported compound assignment operator"),
                        };
                        self.instructions.push(Op::CompoundAssignPath(keys.len(), operation));
                        self.instructions.push(Op::StoreArrayRoot(variable.clone()));
                    }
                    AssignTarget::Property { object, property } => {
                        if *operation == php_syntax::BinaryOp::Coalesce {
                            self.errors.push("Coalescing assignment to object properties is not implemented".into());
                            return;
                        }
                        let operation = match operation {
                            php_syntax::BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                            php_syntax::BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                            php_syntax::BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                            php_syntax::BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                            php_syntax::BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                            php_syntax::BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                            php_syntax::BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                            php_syntax::BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                            php_syntax::BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                            php_syntax::BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                            php_syntax::BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                            php_syntax::BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                            _ => unreachable!("unsupported compound assignment operator"),
                        };
                        self.instructions.push(Op::Load(object.clone()));
                        self.expression(value);
                        self.instructions.push(Op::CompoundAssignProperty(
                            property.clone(), operation, self.current_class.clone(),
                            self.current_method.as_deref() == Some("__construct"),
                        ));
                        self.instructions.push(Op::Pop);
                    }
                    AssignTarget::StaticProperty { class, property } => {
                        let class = match class.as_str() {
                            "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                            "static" => "static".into(),
                            _ => class.to_ascii_lowercase(),
                        };
                        let operation = match operation {
                            php_syntax::BinaryOp::Add => php_runtime::bytecode::CompoundOperation::Add,
                            php_syntax::BinaryOp::Subtract => php_runtime::bytecode::CompoundOperation::Subtract,
                            php_syntax::BinaryOp::Multiply => php_runtime::bytecode::CompoundOperation::Multiply,
                            php_syntax::BinaryOp::Power => php_runtime::bytecode::CompoundOperation::Power,
                            php_syntax::BinaryOp::Divide => php_runtime::bytecode::CompoundOperation::Divide,
                            php_syntax::BinaryOp::Modulo => php_runtime::bytecode::CompoundOperation::Modulo,
                            php_syntax::BinaryOp::Concatenate => php_runtime::bytecode::CompoundOperation::Concatenate,
                            php_syntax::BinaryOp::BitAnd => php_runtime::bytecode::CompoundOperation::BitAnd,
                            php_syntax::BinaryOp::BitXor => php_runtime::bytecode::CompoundOperation::BitXor,
                            php_syntax::BinaryOp::BitOr => php_runtime::bytecode::CompoundOperation::BitOr,
                            php_syntax::BinaryOp::ShiftLeft => php_runtime::bytecode::CompoundOperation::ShiftLeft,
                            php_syntax::BinaryOp::ShiftRight => php_runtime::bytecode::CompoundOperation::ShiftRight,
                            _ => unreachable!("unsupported compound assignment operator"),
                        };
                        self.expression(value);
                        self.instructions.push(Op::CompoundAssignStaticProperty(class, property.clone(), operation, self.current_class.clone()));
                        self.instructions.push(Op::Pop);
                    }
                    _ => self.errors.push("Unsupported compound assignment target".into()),
                }
            }
            Stmt::If { condition, then_body, else_body } => {
                self.expression(condition);
                let false_jump = self.instructions.len();
                self.instructions.push(Op::JumpFalse(0));
                self.statements(then_body);

                if else_body.is_empty() {
                    self.instructions[false_jump] = Op::JumpFalse(self.instructions.len());
                } else {
                    let end_jump = self.instructions.len();
                    self.instructions.push(Op::Jump(0));
                    self.instructions[false_jump] = Op::JumpFalse(self.instructions.len());
                    self.statements(else_body);
                    self.instructions[end_jump] = Op::Jump(self.instructions.len());
                }
            }
            Stmt::Switch { .. } => self.switch_statement(statement),
            Stmt::TryCatch { try_body, catches, finally_body: None } => {
                let handler = self.instructions.len();
                self.instructions.push(Op::TryStart(catches.iter().map(|catch| php_runtime::bytecode::CatchTarget {
                    target: 0, variable: catch.variable.clone(), types: catch.types.clone(),
                }).collect()));
                self.statements(try_body);
                self.instructions.push(Op::TryEnd);
                let skip_catch = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                let mut targets = Vec::new();
                let mut exits = Vec::new();
                for catch in catches {
                    let target = self.instructions.len();
                    targets.push(php_runtime::bytecode::CatchTarget { target, variable: catch.variable.clone(), types: catch.types.clone() });
                    self.statements(&catch.body);
                    exits.push(self.instructions.len());
                    self.instructions.push(Op::Jump(0));
                }
                let end = self.instructions.len();
                self.instructions[handler] = Op::TryStart(targets);
                for exit in exits { self.instructions[exit] = Op::Jump(end); }
                self.instructions[skip_catch] = Op::Jump(end);
            }
            Stmt::TryCatch { try_body, catches, finally_body: Some(finally_body) } => {
                let finally_handler = self.instructions.len();
                self.instructions.push(Op::FinallyStart(0, Vec::new()));
                let catch_handler = self.instructions.len();
                self.instructions.push(Op::TryStart(catches.iter().map(|catch| php_runtime::bytecode::CatchTarget {
                    target: 0, variable: catch.variable.clone(), types: catch.types.clone(),
                }).collect()));
                let try_start = self.instructions.len();
                self.statements(try_body);
                let try_end = self.instructions.len();
                self.instructions.push(Op::TryEnd); // Remove catch handler.
                self.instructions.push(Op::TryEnd); // Remove finally handler before running it normally.
                self.statements(finally_body);
                let skip_catch = self.instructions.len();
                self.instructions.push(Op::Jump(0));

                let mut targets = Vec::new();
                let mut protected_ranges = vec![(try_start, try_end)];
                let mut exits = Vec::new();
                for catch in catches {
                    let target = self.instructions.len();
                    targets.push(php_runtime::bytecode::CatchTarget { target, variable: catch.variable.clone(), types: catch.types.clone() });
                    let catch_body_start = self.instructions.len();
                    self.statements(&catch.body);
                    let catch_body_end = self.instructions.len();
                    protected_ranges.push((catch_body_start, catch_body_end));
                    self.instructions.push(Op::TryEnd); // Remove finally handler before running it after catch.
                    self.statements(finally_body);
                    exits.push(self.instructions.len());
                    self.instructions.push(Op::Jump(0));
                }

                let exception_finally_start = self.instructions.len();
                self.instructions[catch_handler] = Op::TryStart(targets);
                self.instructions[finally_handler] = Op::FinallyStart(exception_finally_start, protected_ranges);
                self.statements(finally_body);
                self.instructions.push(Op::FinallyResume);
                let end = self.instructions.len();
                self.instructions[skip_catch] = Op::Jump(end);
                for exit in exits { self.instructions[exit] = Op::Jump(end); }
            }
            Stmt::TryFinally { try_body, finally_body } => {
                let finally_handler = self.instructions.len();
                self.instructions.push(Op::FinallyStart(0, Vec::new()));
                let try_start = self.instructions.len();
                self.statements(try_body);
                let try_end = self.instructions.len();
                self.instructions.push(Op::TryEnd);
                self.statements(finally_body);
                let skip_exception_finally = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                let exception_finally_start = self.instructions.len();
                self.instructions[finally_handler] = Op::FinallyStart(exception_finally_start, vec![(try_start, try_end)]);
                self.statements(finally_body);
                self.instructions.push(Op::FinallyResume);
                let end = self.instructions.len();
                self.instructions[skip_exception_finally] = Op::Jump(end);
            }
            Stmt::Throw { value, line } => {
                self.expression(value);
                self.instructions.push(Op::ThrowAt(*line));
            }
            Stmt::While { .. } | Stmt::Foreach { .. } => self.loop_statement(statement),
            Stmt::DoWhile { .. } | Stmt::For { .. } => self.loop_statement(statement),
            Stmt::Break(level) => self.loop_jump_expr(true, level),
            Stmt::Continue(level) => self.loop_jump_expr(false, level),
            Stmt::Goto(label) => {
                let instruction = self.instructions.len();
                self.instructions.push(Op::Jump(0));
                self.pending_gotos.push((label.clone(), instruction));
            }
            Stmt::Label(label) => {
                if self.labels.insert(label.clone(), self.instructions.len()).is_some() {
                    self.errors.push(format!("Label '{label}' is already defined"));
                }
            }
            // Preserve object/property operands so unset can dispatch __unset at runtime.
            Stmt::UnsetProperty { object, property } => {
                self.instructions.push(Op::Load(object.clone()));
                self.instructions.push(Op::UnsetProperty(property.clone(), self.current_class.clone()));
            }
            Stmt::UnsetPropertyArrayPath { object, property, keys } => {
                self.expression(object);
                for key in keys { self.expression(key); }
                self.instructions.push(Op::UnsetPropertyArray(property.clone(), keys.len(), self.current_class.clone()));
            }
            Stmt::UnsetDynamicProperty { object, property } => {
                self.expression(object);
                self.expression(property);
                self.instructions.push(Op::UnsetDynamicProperty(self.current_class.clone()));
            }
            Stmt::Unset { variable, keys } => {
                if keys.is_empty() {
                    if variable == "GLOBALS" {
                        self.errors.push("Cannot unset $GLOBALS".into());
                        return;
                    }
                    self.instructions.push(Op::UnsetVariable(variable.clone()));
                } else {
                    for key in keys { self.expression(key); }
                    self.instructions.push(Op::UnsetArrayPath(variable.clone(), keys.len()));
                }
            }
            Stmt::UnsetMany(statements) => for statement in statements { self.statement(statement); },
            Stmt::UnsetStaticPropertyPath { class, property, keys } => {
                for key in keys { self.expression(key); }
                self.instructions.push(Op::UnsetStaticPropertyArray(class.to_ascii_lowercase(), property.clone(), keys.len(), self.current_class.clone()));
            }
            Stmt::Return(value) => {
                if let Some(value) = value {
                    if self.returns_by_reference {
                        if let Some((variable, keys)) = reference_path(value) {
                            for key in &keys { self.expression(key); }
                            self.instructions.push(Op::LoadReference(variable, keys.len()));
                        } else if let Expr::Property(receiver, property) = value {
                            self.expression(receiver);
                            self.instructions.push(Op::LoadPropertyReference(property.clone(), self.current_class.clone()));
                        } else if let Expr::StaticProperty(class, property) = value {
                            let class = match class.as_str() {
                                "self" => self.current_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                                "parent" => self.parent_class.clone().unwrap_or_else(|| class.to_ascii_lowercase()),
                                "static" => "static".into(),
                                _ => class.to_ascii_lowercase(),
                            };
                            self.instructions.push(Op::LoadStaticPropertyReference(class, property.clone(), self.current_class.clone()));
                        } else { self.expression(value); }
                    } else { self.expression(value); }
                }
                else { self.instructions.push(Op::Const(Value::Null)); }
                self.instructions.push(Op::Return);
            }
        }
    }

}

pub(super) fn compile_destructure_target(
    generator: &mut super::CodeGenerator,
    target: &php_syntax::DestructureTarget,
    target_key_count: &mut usize,
) -> php_runtime::bytecode::DestructureTarget {
    let key = target.key.as_ref().and_then(|key| match key {
        php_syntax::DestructureKey::Integer(value) => Some(php_runtime::ArrayKey::Integer(*value)),
        php_syntax::DestructureKey::String(value) => Some(php_runtime::ArrayKey::String(value.clone())),
        php_syntax::DestructureKey::Computed(_) => None,
    });
    if let Some(php_syntax::DestructureKey::Computed(expression)) = &target.key {
        generator.expression(expression);
        *target_key_count += 1;
    }
    if target.array_variable.is_some() || target.property_receiver.is_some() {
        for expression in &target.array_keys {
            generator.expression(expression);
            *target_key_count += 1;
        }
    }
    if let Some(receiver) = &target.property_receiver {
        generator.expression(receiver);
        *target_key_count += 1;
    }
    php_runtime::bytecode::DestructureTarget {
        key,
        computed_key: matches!(target.key.as_ref(), Some(php_syntax::DestructureKey::Computed(_))),
        variable: target.variable.clone(),
        array_variable: target.array_variable.clone(),
        array_key_count: target.array_keys.len(),
        array_append: target.array_append,
        property_receiver: target.property_receiver.is_some(),
        property: target.property.clone(),
        property_array_key_count: if target.property_receiver.is_some() { target.array_keys.len() } else { 0 },
        nested: target.nested.as_ref().map(|targets| targets.iter().map(|target| compile_destructure_target(generator, target, target_key_count)).collect()),
    }
}
