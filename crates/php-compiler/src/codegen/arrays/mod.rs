//! Bytecode generation for PHP array literals and indexed reads.

use php_runtime::bytecode::{ArrayEntryKind, Op};
use php_syntax::Expr;
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

fn property_reference_path(expression: &Expr) -> Option<(Expr, String, Vec<&Expr>)> {
    match expression {
        Expr::Property(receiver, property) => Some(((**receiver).clone(), property.clone(), Vec::new())),
        Expr::Index(array, key) => {
            let (receiver, property, mut keys) = property_reference_path(array)?;
            keys.push(key);
            Some((receiver, property, keys))
        }
        _ => None,
    }
}

impl CodeGenerator<'_> {
    /// Emit a literal or indexed read while preserving PHP array key evaluation order.
    pub(super) fn array_expression(&mut self, expression: &Expr) {
        match expression {
            Expr::Array(items) => {
                let mut entry_kinds = Vec::with_capacity(items.len());
                for item in items {
                    if item.unpack {
                        self.expression(&item.value);
                        entry_kinds.push(ArrayEntryKind::Unpack);
                    } else if item.by_reference {
                        if let Some((variable, keys)) = reference_path(&item.value) {
                            if let Some(key) = &item.key { self.expression(key); }
                            for key in &keys { self.expression(key); }
                            self.instructions.push(Op::LoadReference(variable, keys.len()));
                        } else if let Some((receiver, property, keys)) = property_reference_path(&item.value) {
                            if let Some(key) = &item.key { self.expression(key); }
                            self.expression(&receiver);
                            if keys.is_empty() {
                                self.instructions.push(Op::LoadPropertyReference(property, self.current_class.clone()));
                            } else {
                                for key in &keys { self.expression(key); }
                                self.instructions.push(Op::LoadPropertyArrayReference(property, keys.len(), self.current_class.clone()));
                            }
                        } else {
                            self.errors.push("Array reference values must be variables, array elements, or property array elements".into());
                            return;
                        }
                        if item.key.is_some() {
                            entry_kinds.push(ArrayEntryKind::KeyedReference);
                        } else {
                            entry_kinds.push(ArrayEntryKind::PackedReference);
                        }
                    } else if let Some(key) = &item.key {
                        self.expression(key);
                        entry_kinds.push(ArrayEntryKind::Keyed);
                        self.expression(&item.value);
                    } else {
                        entry_kinds.push(ArrayEntryKind::Packed);
                        self.expression(&item.value);
                    }
                }
                self.instructions.push(Op::MakeArray(entry_kinds));
            }
            Expr::Index(array, key) => {
                self.expression(array);
                self.expression(key);
                self.instructions.push(Op::ArrayGet);
            }
            _ => unreachable!("array_expression only handles array AST nodes"),
        }
    }
}
