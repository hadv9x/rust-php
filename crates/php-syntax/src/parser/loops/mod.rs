//! Parsing for PHP loop statements.

use super::Parser;
use crate::{ast::Stmt, lexer::token::TokenKind};

impl Parser<'_> {
    fn alternative_loop_body(&mut self, terminator: &str) -> Result<Vec<Stmt>, String> {
        let mut body = Vec::new();
        while !matches!(self.peek(), TokenKind::Word(word) if word == terminator) {
            if self.peek() == &TokenKind::Eof {
                return Err(self.error(&format!("Unterminated alternative {terminator} body")));
            }
            body.push(self.statement()?);
        }
        self.take();
        self.expect(TokenKind::Semi, &format!("Expected ; after {terminator}"))?;
        Ok(body)
    }

    /// Parse a while or foreach statement after its keyword has been consumed.
    pub(in crate::parser) fn loop_statement(&mut self, keyword: &str) -> Result<Stmt, String> {
        match keyword {
            "while" => {
                self.expect(TokenKind::Lp, "Expected ( after while")?;
                let condition = self.expression(0)?;
                self.expect(TokenKind::Rp, "Expected ) after while condition")?;
                let body = if self.eat(&TokenKind::Colon) {
                    self.alternative_loop_body("endwhile")?
                } else {
                    self.loop_body()?
                };
                Ok(Stmt::While { condition, body })
            }
            "foreach" => {
                self.expect(TokenKind::Lp, "Expected ( after foreach")?;
                let iterable = self.expression(0)?;
                if !matches!(self.take(), TokenKind::Word(ref word) if word == "as") {
                    return Err(self.error("Expected as in foreach"));
                }
                let first_by_reference = self.eat(&TokenKind::Ampersand);
                let parse_destructure = |parser: &mut Self| -> Result<
                    Option<Vec<crate::ast::DestructureTarget>>,
                    String,
                > {
                    if parser.eat(&TokenKind::Ls) {
                        return parser.destructure_targets(TokenKind::Rs).map(Some);
                    }
                    if matches!(parser.peek(), TokenKind::Word(word) if word == "list")
                        && parser
                            .tokens
                            .get(parser.cursor + 1)
                            .is_some_and(|token| token.kind == TokenKind::Lp)
                    {
                        parser.take();
                        parser.take();
                        return parser.destructure_targets(TokenKind::Rp).map(Some);
                    }
                    Ok(None)
                };
                let key_destructured = parse_destructure(self)?;
                let (first_name, first_property) = if key_destructured.is_some() {
                    (format!("\0foreach_key_destructure_{}", self.cursor), None)
                } else {
                    let name = match self.take() {
                        TokenKind::Var(name) => name,
                        token => {
                            return Err(
                                self.error(&format!("Expected foreach variable, got {token:?}"))
                            )
                        }
                    };
                    if self.eat(&TokenKind::ObjectArrow) {
                        let offset = self.peek_offset();
                        match self.take() {
                            TokenKind::Word(_) => {}
                            token => {
                                return Err(self.error(&format!(
                                    "Expected foreach property target, got {token:?}"
                                )))
                            }
                        }
                        (
                            format!("\0foreach_key_property_{}", self.cursor),
                            Some((name, self.source_word(offset))),
                        )
                    } else {
                        (name, None)
                    }
                };
                let (
                    key_name,
                    key_property,
                    value_name,
                    value_property,
                    value_by_reference,
                    value_destructured,
                ) = if self.eat(&TokenKind::Arrow) {
                    if first_by_reference {
                        return Err(self.error("The foreach key cannot be a reference"));
                    }
                    let value_by_reference = self.eat(&TokenKind::Ampersand);
                    let value_destructured = parse_destructure(self)?;
                    let (value_name, value_property) = if value_destructured.is_some() {
                        (format!("\0foreach_value_destructure_{}", self.cursor), None)
                    } else {
                        let name = match self.take() {
                            TokenKind::Var(name) => name,
                            token => {
                                return Err(self.error(&format!(
                                    "Expected foreach value variable, got {token:?}"
                                )))
                            }
                        };
                        if self.eat(&TokenKind::ObjectArrow) {
                            let offset = self.peek_offset();
                            match self.take() {
                                TokenKind::Word(_) => {}
                                token => {
                                    return Err(self.error(&format!(
                                        "Expected foreach property target, got {token:?}"
                                    )))
                                }
                            }
                            (
                                format!("\0foreach_value_property_{}", self.cursor),
                                Some((name, self.source_word(offset))),
                            )
                        } else {
                            (name, None)
                        }
                    };
                    (
                        Some(first_name),
                        first_property,
                        value_name,
                        value_property,
                        value_by_reference,
                        value_destructured,
                    )
                } else {
                    (
                        None,
                        None,
                        first_name,
                        first_property,
                        first_by_reference,
                        None,
                    )
                };
                if value_by_reference && value_property.is_some() {
                    return Err(
                        self.error("A foreach property target cannot be passed by reference")
                    );
                }
                self.expect(TokenKind::Rp, "Expected ) after foreach")?;
                let mut body = if self.eat(&TokenKind::Colon) {
                    let mut body = Vec::new();
                    while !matches!(self.peek(), TokenKind::Word(word) if word == "endforeach") {
                        if self.peek() == &TokenKind::Eof {
                            return Err(self.error("Unterminated alternative foreach body"));
                        }
                        body.push(self.statement()?);
                    }
                    self.take();
                    self.expect(TokenKind::Semi, "Expected ; after endforeach")?;
                    body
                } else {
                    self.loop_body()?
                };
                if let Some(targets) = key_destructured {
                    body.insert(
                        0,
                        Stmt::ListAssign {
                            targets,
                            value: crate::ast::Expr::Variable(
                                key_name.clone().unwrap_or_else(|| value_name.clone()),
                            ),
                        },
                    );
                }
                if let Some(targets) = value_destructured {
                    body.insert(
                        0,
                        Stmt::ListAssign {
                            targets,
                            value: crate::ast::Expr::Variable(value_name.clone()),
                        },
                    );
                }
                if let Some((object, property)) = &value_property {
                    body.insert(
                        0,
                        Stmt::Assign {
                            target: crate::ast::AssignTarget::Property {
                                object: object.clone(),
                                property: property.clone(),
                            },
                            value: crate::ast::Expr::Variable(value_name.clone()),
                        },
                    );
                }
                if let Some((object, property)) = &key_property {
                    body.insert(
                        0,
                        Stmt::Assign {
                            target: crate::ast::AssignTarget::Property {
                                object: object.clone(),
                                property: property.clone(),
                            },
                            value: crate::ast::Expr::Variable(
                                key_name.clone().unwrap_or_else(|| value_name.clone()),
                            ),
                        },
                    );
                }
                Ok(Stmt::Foreach {
                    iterable,
                    key_name,
                    key_property,
                    value_name,
                    value_property,
                    value_by_reference,
                    body,
                })
            }
            "do" => {
                let body = self.loop_body()?;
                if !matches!(self.take(), TokenKind::Word(ref word) if word == "while") {
                    return Err(self.error("Expected while after do body"));
                }
                self.expect(TokenKind::Lp, "Expected ( after do-while")?;
                let condition = self.expression(0)?;
                self.expect(TokenKind::Rp, "Expected ) after do-while condition")?;
                Ok(Stmt::DoWhile { condition, body })
            }
            "for" => {
                self.expect(TokenKind::Lp, "Expected ( after for")?;
                let initializers = self.for_clauses(TokenKind::Semi)?;
                self.expect(TokenKind::Semi, "Expected ; after for initializer")?;
                let condition = if self.peek() == &TokenKind::Semi {
                    None
                } else {
                    Some(self.expression(0)?)
                };
                self.expect(TokenKind::Semi, "Expected ; after for condition")?;
                let updates = self.for_clauses(TokenKind::Rp)?;
                self.expect(TokenKind::Rp, "Expected ) after for clauses")?;
                let body = if self.eat(&TokenKind::Colon) {
                    self.alternative_loop_body("endfor")?
                } else {
                    self.loop_body()?
                };
                Ok(Stmt::For {
                    initializers,
                    condition,
                    updates,
                    body,
                })
            }
            _ => unreachable!("loop_statement only receives loop keywords"),
        }
    }

    /// Parse comma-separated initializer/update expressions in a for header.
    fn for_clauses(&mut self, closing: TokenKind) -> Result<Vec<Stmt>, String> {
        let mut clauses = Vec::new();
        while self.peek() != &closing {
            let clause = if let TokenKind::Var(name) = self.peek().clone() {
                if self
                    .tokens
                    .get(self.cursor + 1)
                    .is_some_and(|token| token.kind == TokenKind::Eq)
                {
                    self.take();
                    self.take();
                    Stmt::Assign {
                        target: crate::ast::AssignTarget::Variable(name),
                        value: self.expression(0)?,
                    }
                } else {
                    Stmt::Expression(self.expression(0)?)
                }
            } else {
                Stmt::Expression(self.expression(0)?)
            };
            clauses.push(clause);
            if self.peek() == &closing {
                break;
            }
            self.expect(TokenKind::Comma, "Expected , between for clauses")?;
        }
        Ok(clauses)
    }

    /// Accept both PHP's braced loop body and its single-statement form.
    fn loop_body(&mut self) -> Result<Vec<Stmt>, String> {
        if self.peek() == &TokenKind::Lb {
            self.block()
        } else {
            Ok(vec![self.statement()?])
        }
    }
}
