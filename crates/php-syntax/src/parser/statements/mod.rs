//! Statement parser for assignments, branches, loops, and output.

use crate::{ast::{AssignTarget, CatchClause, DestructureKey, DestructureTarget, Expr, IncludeKind, Stmt}, lexer::token::TokenKind};
use super::Parser;

fn is_static_property_reference_source(expression: &Expr) -> bool {
    match expression {
        Expr::StaticProperty(_, _) => true,
        Expr::Index(array, _) => is_static_property_reference_source(array),
        _ => false,
    }
}

impl Parser<'_> {
    fn nested_class_declaration(&mut self, start: usize) -> Result<Stmt, String> {
        let open = (self.cursor..self.tokens.len()).find(|index| self.tokens[*index].kind == TokenKind::Lb)
            .ok_or_else(|| self.error("Expected { in class declaration"))?;
        let mut depth = 0usize;
        let mut close = None;
        for index in open..self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::Lb => depth += 1,
                TokenKind::Rb => {
                    depth -= 1;
                    if depth == 0 { close = Some(index); break; }
                }
                TokenKind::Eof => break,
                _ => {},
            }
        }
        let close = close.ok_or_else(|| self.error("Unterminated class declaration"))?;
        let source_start = self.tokens[start].offset;
        let source_end = self.tokens[close].offset + 1;
        let class_source = self.source.get(source_start..source_end).ok_or_else(|| self.error("Invalid class declaration source range"))?;
        let source = format!("<?php\n{class_source}");
        let parser = Parser {
            source: &source,
            tokens: crate::lexer::lex(&source)?,
            cursor: 0,
            namespace: self.namespace.clone(),
            class_imports: self.class_imports.clone(),
            function_imports: self.function_imports.clone(),
            constant_imports: self.constant_imports.clone(),
            anonymous_classes: Vec::new(),
            next_anonymous_class_id: 0,
            strict_types: false,
            word_operator_floor: self.word_operator_floor,
        };
        let program = parser.program()?;
        let class = program.classes.into_iter().next().ok_or_else(|| self.error("Expected class declaration"))?;
        self.cursor = close + 1;
        Ok(Stmt::ClassDeclaration(class))
    }

    fn nested_interface_declaration(&mut self, start: usize) -> Result<Stmt, String> {
        let open = (self.cursor..self.tokens.len()).find(|index| self.tokens[*index].kind == TokenKind::Lb)
            .ok_or_else(|| self.error("Expected { in interface declaration"))?;
        let mut depth = 0usize;
        let mut close = None;
        for index in open..self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::Lb => depth += 1,
                TokenKind::Rb => { depth -= 1; if depth == 0 { close = Some(index); break; } }
                TokenKind::Eof => break,
                _ => {},
            }
        }
        let close = close.ok_or_else(|| self.error("Unterminated interface declaration"))?;
        let start_offset = self.tokens[start].offset;
        let end_offset = self.tokens[close].offset + 1;
        let fragment = self.source.get(start_offset..end_offset).ok_or_else(|| self.error("Invalid interface declaration source range"))?;
        let source = format!("<?php\n{fragment}");
        let parser = Parser {
            source: &source,
            tokens: crate::lexer::lex(&source)?,
            cursor: 0,
            namespace: self.namespace.clone(),
            class_imports: self.class_imports.clone(),
            function_imports: self.function_imports.clone(),
            constant_imports: self.constant_imports.clone(),
            anonymous_classes: Vec::new(),
            next_anonymous_class_id: 0,
            strict_types: false,
            word_operator_floor: self.word_operator_floor,
        };
        let program = parser.program()?;
        let interface = program.interfaces.into_iter().next().ok_or_else(|| self.error("Expected interface declaration"))?;
        self.cursor = close + 1;
        Ok(Stmt::InterfaceDeclaration(interface))
    }

    fn nested_trait_declaration(&mut self, start: usize) -> Result<Stmt, String> {
        let open = (self.cursor..self.tokens.len()).find(|index| self.tokens[*index].kind == TokenKind::Lb)
            .ok_or_else(|| self.error("Expected { in trait declaration"))?;
        let mut depth = 0usize;
        let mut close = None;
        for index in open..self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::Lb => depth += 1,
                TokenKind::Rb => { depth -= 1; if depth == 0 { close = Some(index); break; } }
                TokenKind::Eof => break,
                _ => {},
            }
        }
        let close = close.ok_or_else(|| self.error("Unterminated trait declaration"))?;
        let start_offset = self.tokens[start].offset;
        let end_offset = self.tokens[close].offset + 1;
        let fragment = self.source.get(start_offset..end_offset).ok_or_else(|| self.error("Invalid trait declaration source range"))?;
        let source = format!("<?php\n{fragment}");
        let parser = Parser {
            source: &source,
            tokens: crate::lexer::lex(&source)?,
            cursor: 0,
            namespace: self.namespace.clone(),
            class_imports: self.class_imports.clone(),
            function_imports: self.function_imports.clone(),
            constant_imports: self.constant_imports.clone(),
            anonymous_classes: Vec::new(),
            next_anonymous_class_id: 0,
            strict_types: false,
            word_operator_floor: self.word_operator_floor,
        };
        let program = parser.program()?;
        let trait_decl = program.traits.into_iter().next().ok_or_else(|| self.error("Expected trait declaration"))?;
        self.cursor = close + 1;
        Ok(Stmt::TraitDeclaration(trait_decl))
    }

    fn array_path_assignment_after(&self, reference_only: bool) -> bool {
        if self.peek() != &TokenKind::Ls { return false; }
        let mut depth = 0usize;
        for index in self.cursor..self.tokens.len() {
            match self.tokens[index].kind {
                TokenKind::Ls => depth += 1,
                TokenKind::Rs => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if self.tokens.get(index + 1).is_some_and(|token| token.kind == TokenKind::Ls) { continue; }
                        let operator = self.tokens.get(index + 1).map(|token| &token.kind);
                        if reference_only {
                            return operator == Some(&TokenKind::Eq)
                                && self.tokens.get(index + 2).is_some_and(|token| token.kind == TokenKind::Ampersand);
                        }
                        return operator.is_some_and(|operator| matches!(operator,
                            TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq
                            | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq
                            | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq
                            | TokenKind::CoalesceEq));
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
        }
        false
    }

    pub(super) fn short_list_assignment_ahead(&self) -> bool {
        let mut cursor = self.cursor;
        let mut square_depth = 0usize;
        let mut paren_depth = 0usize;
        while let Some(token) = self.tokens.get(cursor) {
            match token.kind {
                TokenKind::Ls => { square_depth += 1; cursor += 1; }
                TokenKind::Rs if square_depth > 0 => {
                    square_depth -= 1;
                    cursor += 1;
                    if square_depth == 0 && paren_depth == 0 {
                        return self.tokens.get(cursor).is_some_and(|next| next.kind == TokenKind::Eq);
                    }
                }
                TokenKind::Rs => return paren_depth == 0 && self.tokens.get(cursor + 1).is_some_and(|next| next.kind == TokenKind::Eq),
                TokenKind::Lp => { paren_depth += 1; cursor += 1; }
                TokenKind::Rp if paren_depth > 0 => { paren_depth -= 1; cursor += 1; }
                TokenKind::Semi | TokenKind::Eof => return false,
                _ => cursor += 1,
            }
        }
        false
    }

    pub(super) fn destructure_targets(&mut self, closing: TokenKind) -> Result<Vec<DestructureTarget>, String> {
        let mut targets = Vec::new();
        if self.eat(&closing) { return Ok(targets); }
        loop {
            if self.eat(&TokenKind::Comma) {
                targets.push(DestructureTarget { key: None, variable: None, array_variable: None, array_keys: Vec::new(), array_append: false, property_receiver: None, property: None, nested: None });
                if self.eat(&closing) { break; }
                continue;
            }
            let parsed = match self.take() {
                TokenKind::Ls => {
                    let nested = self.destructure_targets(TokenKind::Rs)?;
                    targets.push(DestructureTarget { key: None, variable: None, array_variable: None, array_keys: Vec::new(), array_append: false, property_receiver: None, property: None, nested: Some(nested) });
                    if self.eat(&closing) { break; }
                    self.expect(TokenKind::Comma, "Expected , or closing token in destructuring targets")?;
                    if self.eat(&closing) { break; }
                    continue;
                }
                TokenKind::Word(word) if word == "list" && self.eat(&TokenKind::Lp) => {
                    let nested = self.destructure_targets(TokenKind::Rp)?;
                    targets.push(DestructureTarget { key: None, variable: None, array_variable: None, array_keys: Vec::new(), array_append: false, property_receiver: None, property: None, nested: Some(nested) });
                    if self.eat(&closing) { break; }
                    self.expect(TokenKind::Comma, "Expected , or closing token in destructuring targets")?;
                    if self.eat(&closing) { break; }
                    continue;
                }
                _ => { self.cursor -= 1; self.expression(1)? }
            };
            let target = if self.eat(&TokenKind::Arrow) {
                let (variable, array_variable, array_keys, array_append, property_receiver, property, nested) = self.destructure_value(closing.clone())?;
                DestructureTarget {
                    key: Some(DestructureKey::Computed(Box::new(parsed))),
                    variable, array_variable, array_keys, array_append, property_receiver, property, nested,
                }
            } else {
                let (variable, array_variable, array_keys, array_append, property_receiver, property) = Self::destructure_target_expression(parsed)?;
                DestructureTarget { key: None, variable, array_variable, array_keys, array_append, property_receiver, property, nested: None }
            };
            targets.push(target);
            if self.eat(&closing) { break; }
            self.expect(TokenKind::Comma, "Expected , or closing token in destructuring targets")?;
            if self.eat(&closing) { break; }
        }
        Ok(targets)
    }

    fn destructure_value(&mut self, closing: TokenKind) -> Result<(Option<String>, Option<String>, Vec<crate::ast::Expr>, bool, Option<crate::ast::Expr>, Option<String>, Option<Vec<DestructureTarget>>), String> {
        if self.peek() == &TokenKind::Comma || self.peek() == &closing {
            return Ok((None, None, Vec::new(), false, None, None, None));
        }
        match self.take() {
            TokenKind::Ls => Ok((None, None, Vec::new(), false, None, None, Some(self.destructure_targets(TokenKind::Rs)?))),
            TokenKind::Word(word) if word == "list" && self.eat(&TokenKind::Lp) => Ok((None, None, Vec::new(), false, None, None, Some(self.destructure_targets(TokenKind::Rp)?))),
            _ => {
                self.cursor -= 1;
                let expression = self.expression(1)?;
                let (variable, array_variable, array_keys, array_append, property_receiver, property) = Self::destructure_target_expression(expression)?;
                Ok((variable, array_variable, array_keys, array_append, property_receiver, property, None))
            }
        }
    }

    fn destructure_target_expression(expression: crate::ast::Expr) -> Result<(Option<String>, Option<String>, Vec<crate::ast::Expr>, bool, Option<crate::ast::Expr>, Option<String>), String> {
        fn property_path(expression: crate::ast::Expr, keys: &mut Vec<crate::ast::Expr>) -> Option<(crate::ast::Expr, String)> {
            match expression {
                crate::ast::Expr::Property(object, property) => Some((*object, property)),
                crate::ast::Expr::Index(array, key) => { keys.push(*key); property_path(*array, keys) }
                _ => None,
            }
        }
        let mut property_keys = Vec::new();
        if let Some((receiver, property)) = property_path(expression.clone(), &mut property_keys) {
            property_keys.reverse();
            return Ok((None, None, property_keys, false, Some(receiver), Some(property)));
        }
        if let crate::ast::Expr::PropertyAppend(base) = expression.clone() {
            let mut keys = Vec::new();
            fn collect_append(base: crate::ast::Expr, keys: &mut Vec<crate::ast::Expr>) -> Result<String, String> {
                match base {
                    crate::ast::Expr::Variable(name) => Ok(name),
                    crate::ast::Expr::Index(array, key) => { keys.push(*key); collect_append(*array, keys) }
                    _ => Err("Destructuring append target must be a variable array path".into()),
                }
            }
            let name = collect_append(*base, &mut keys)?;
            keys.reverse();
            return Ok((None, Some(name), keys, true, None, None));
        }
        fn collect(expression: crate::ast::Expr, keys: &mut Vec<crate::ast::Expr>) -> Result<String, String> {
            match expression {
                crate::ast::Expr::Variable(name) => Ok(name),
                crate::ast::Expr::Index(base, key) => {
                    keys.push(*key);
                    collect(*base, keys)
                }
                _ => Err("Destructuring targets must be variables or array offsets".into()),
            }
        }
        let mut keys = Vec::new();
        let name = collect(expression, &mut keys)?;
        if keys.is_empty() { return Ok((Some(name), None, keys, false, None, None)); }
        keys.reverse();
        Ok((None, Some(name), keys, false, None, None))
    }

    /// Parse a braced block of statements.
    pub(in crate::parser) fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(TokenKind::Lb, "Expected {")?;
        let mut statements = Vec::new();
        while !self.eat(&TokenKind::Rb) {
            if self.peek() == &TokenKind::Eof { return Err(self.error("Unterminated block")); }
            statements.push(self.statement()?);
        }
        Ok(statements)
    }

    fn else_if_body(&mut self) -> Result<Vec<Stmt>, String> {
        if matches!(self.peek(), TokenKind::Word(word) if word == "elseif") {
            self.take();
            self.expect(TokenKind::Lp, "Expected ( after elseif")?;
            let condition = self.expression(0)?;
            self.expect(TokenKind::Rp, "Expected ) after elseif condition")?;
            let then_body = if self.peek() == &TokenKind::Lb { self.block()? } else { vec![self.statement()?] };
            let else_body = self.else_if_body()?;
            Ok(vec![Stmt::If { condition, then_body, else_body }])
        } else if matches!(self.peek(), TokenKind::Word(word) if word == "else") {
            self.take();
            if self.peek() == &TokenKind::Lb { self.block() } else { Ok(vec![self.statement()?]) }
        } else {
            Ok(Vec::new())
        }
    }

    /// Parse one statement or declaration from the token stream.
    pub(in crate::parser) fn statement(&mut self) -> Result<Stmt, String> {
        if self.peek() == &TokenKind::AttributeStart {
            // Conditional declarations are parsed through statement(); retain their declaration
            // body while ignoring attribute metadata that this statement AST cannot store.
            self.attributes()?;
            return self.statement();
        }
        if self.eat(&TokenKind::Semi) {
            if matches!(self.peek(), TokenKind::Eof | TokenKind::Rb) { return Ok(Stmt::Expression(Expr::Null)); }
            return self.statement();
        }

        let statement_start = self.cursor;
        let statement_line = self.source.get(..self.tokens[statement_start].offset).unwrap_or_default().bytes().filter(|byte| *byte == b'\n').count() + 1;
        let (statement, needs_semicolon) = match self.take() {
            TokenKind::InlineHtml(html) => (Stmt::Echo(vec![Expr::String(html)]), false),
            TokenKind::Word(label) if self.eat(&TokenKind::Colon) => (Stmt::Label(label), false),
            TokenKind::Word(word) if word == "class" || word == "enum" => (self.nested_class_declaration(statement_start)?, false),
            TokenKind::Word(word) if matches!(word.as_str(), "abstract" | "final" | "readonly")
                && matches!(self.peek(), TokenKind::Word(next) if matches!(next.as_str(), "class" | "readonly" | "final" | "abstract")) =>
                (self.nested_class_declaration(statement_start)?, false),
            TokenKind::Word(word) if word == "interface" => (self.nested_interface_declaration(statement_start)?, false),
            TokenKind::Word(word) if word == "trait" => (self.nested_trait_declaration(statement_start)?, false),
            TokenKind::Word(word) if word == "goto" => {
                let label = match self.take() {
                    TokenKind::Word(label) => label,
                    token => return Err(self.error(&format!("Expected label name after goto, got {token:?}"))),
                };
                (Stmt::Goto(label), true)
            }
            TokenKind::Word(word) if word == "function" => {
                self.cursor -= 1;
                let mut function = self.function_declaration(false, false, false, crate::ast::Visibility::Public)?;
                function.name = self.qualify_declaration_name(function.name);
                (Stmt::FunctionDeclaration(function), false)
            }
            TokenKind::Word(word) if word == "declare" => {
                self.expect(TokenKind::Lp, "Expected ( after declare")?;
                match self.take() {
                    TokenKind::Word(name) if name == "strict_types" => {},
                    token => return Err(self.error(&format!("Unsupported declare directive {token:?}"))),
                }
                self.expect(TokenKind::Eq, "Expected = after declare directive")?;
                let strict_types = match self.expression(0)? {
                    Expr::Integer(0) => false,
                    Expr::Integer(1) => true,
                    _ => return Err(self.error("declare(strict_types) must be 0 or 1")),
                };
                self.strict_types = strict_types;
                self.expect(TokenKind::Rp, "Expected ) after declare directive")?;
                (Stmt::Expression(Expr::Null), true)
            }
            TokenKind::Word(word) if matches!(word.as_str(), "include" | "include_once" | "require" | "require_once") => {
                let kind = match word.as_str() {
                    "include" => IncludeKind::Include,
                    "include_once" => IncludeKind::IncludeOnce,
                    "require" => IncludeKind::Require,
                    _ => IncludeKind::RequireOnce,
                };
                (Stmt::Include { kind, path: self.expression(1)? }, true)
            }
            TokenKind::Word(word) if word == "list" && self.eat(&TokenKind::Lp) => {
                let targets = self.destructure_targets(TokenKind::Rp)?;
                self.expect(TokenKind::Eq, "Expected = after list() targets")?;
                (Stmt::ListAssign { targets, value: self.expression(1)? }, true)
            }
            TokenKind::Ls => {
                if !self.short_list_assignment_ahead() {
                    self.cursor -= 1;
                    (Stmt::Expression(self.expression(0)?), true)
                } else {
                    let targets = self.destructure_targets(TokenKind::Rs)?;
                    self.expect(TokenKind::Eq, "Expected = after destructuring targets")?;
                    (Stmt::ListAssign { targets, value: self.expression(1)? }, true)
                }
            }
            TokenKind::Word(word) if word == "global" => {
                let mut variables = Vec::new();
                loop {
                    match self.take() {
                        TokenKind::Var(name) => variables.push(name),
                        token => return Err(self.error(&format!("Expected variable after global, got {token:?}"))),
                    }
                    if !self.eat(&TokenKind::Comma) { break; }
                }
                (Stmt::Global(variables), true)
            }
            TokenKind::Word(word) if word == "static" && self.peek() != &TokenKind::DoubleColon => {
                let mut variables = Vec::new();
                loop {
                    let name = match self.take() {
                        TokenKind::Var(name) => name,
                        token => return Err(self.error(&format!("Expected variable after static, got {token:?}"))),
                    };
                    let initial = if self.eat(&TokenKind::Eq) { Some(self.expression(1)?) } else { None };
                    variables.push(crate::ast::StaticVariable { name, initial });
                    if !self.eat(&TokenKind::Comma) { break; }
                }
                (Stmt::Static(variables), true)
            }
            TokenKind::Word(word) if word == "echo" || word == "print" => {
                let mut expressions = vec![self.expression(1)?];
                while self.eat(&TokenKind::Comma) { expressions.push(self.expression(1)?); }
                (Stmt::Echo(expressions), true)
            }
            TokenKind::Word(word) if word == "if" => {
                self.expect(TokenKind::Lp, "Expected ( after if")?;
                let condition = self.expression(0)?;
                self.expect(TokenKind::Rp, "Expected ) after if condition")?;
                let (then_body, else_body) = if self.eat(&TokenKind::Colon) {
                    let mut then_body = Vec::new();
                    while !matches!(self.peek(), TokenKind::Word(end) if end == "else" || end == "elseif" || end == "endif") {
                        if self.peek() == &TokenKind::Eof { return Err(self.error("Unterminated alternative if body")); }
                        then_body.push(self.statement()?);
                    }
                    let else_body = if matches!(self.peek(), TokenKind::Word(end) if end == "else") {
                        self.take();
                        self.expect(TokenKind::Colon, "Expected : after else")?;
                        let mut body = Vec::new();
                        while !matches!(self.peek(), TokenKind::Word(end) if end == "endif") {
                            if self.peek() == &TokenKind::Eof { return Err(self.error("Unterminated alternative else body")); }
                            body.push(self.statement()?);
                        }
                        body
                    } else { Vec::new() };
                    if matches!(self.peek(), TokenKind::Word(end) if end == "elseif") { return Err(self.error("Alternative elseif is not supported yet")); }
                    self.expect(TokenKind::Word("endif".into()), "Expected endif")?;
                    self.expect(TokenKind::Semi, "Expected ; after endif")?;
                    (then_body, else_body)
                } else {
                    (if self.peek() == &TokenKind::Lb { self.block()? } else { vec![self.statement()?] }, self.else_if_body()?)
                };
                (Stmt::If { condition, then_body, else_body }, false)
            }
            TokenKind::Word(word) if word == "switch" => (self.switch_statement()?, false),
            TokenKind::Word(word) if word == "try" => {
                let try_body = self.block()?;
                if !matches!(self.peek(), TokenKind::Word(word) if word == "catch") {
                    if matches!(self.peek(), TokenKind::Word(word) if word == "finally") {
                        self.take();
                        (Stmt::TryFinally { try_body, finally_body: self.block()? }, false)
                    } else {
                        return Err(self.error("Expected catch or finally after try body"));
                    }
                } else {
                    let mut catches = Vec::new();
                    while matches!(self.peek(), TokenKind::Word(word) if word == "catch") {
                        self.take();
                        self.expect(TokenKind::Lp, "Expected ( after catch")?;
                        let mut types = Vec::new();
                        loop {
                            match self.take() {
                                TokenKind::Word(name) => types.push(self.qualify_type(name)),
                                token => return Err(self.error(&format!("Expected catch type, got {token:?}"))),
                            }
                            if self.eat(&TokenKind::Pipe) { continue; }
                            break;
                        }
                        let variable = if let TokenKind::Var(name) = self.peek().clone() {
                            self.take();
                            Some(name)
                        } else { None };
                        self.expect(TokenKind::Rp, "Expected ) after catch variable")?;
                        catches.push(CatchClause { types, variable, body: self.block()? });
                    }
                    if catches.is_empty() { return Err(self.error("Expected catch or finally after try body")); }
                    let finally_body = if matches!(self.peek(), TokenKind::Word(word) if word == "finally") {
                        self.take();
                        Some(self.block()?)
                    } else { None };
                    (Stmt::TryCatch { try_body, catches, finally_body }, false)
                }
            }
            TokenKind::Word(word) if word == "throw" => (Stmt::Throw { value: self.expression(0)?, line: statement_line }, true),
            TokenKind::Word(word) if word == "while" || word == "foreach" => {
                (self.loop_statement(&word)?, false)
            }
            TokenKind::Word(word) if word == "do" || word == "for" => {
                (self.loop_statement(&word)?, false)
            }
            TokenKind::Word(word) if word == "break" || word == "continue" => {
                let level = if matches!(self.peek(), TokenKind::Semi | TokenKind::Rb | TokenKind::Eof) { Expr::Integer(1) } else { self.expression(1)? };
                (if word == "break" { Stmt::Break(level) } else { Stmt::Continue(level) }, true)
            }
            TokenKind::Word(word) if word == "return" => {
                let value = if matches!(self.peek(), TokenKind::Semi | TokenKind::Rb | TokenKind::Eof) {
                    None
                } else {
                    Some(self.expression(1)?)
                };
                (Stmt::Return(value), true)
            }
            TokenKind::Word(word) if word == "unset" => {
                self.expect(TokenKind::Lp, "Expected ( after unset")?;
                if matches!(self.peek(), TokenKind::Word(_)) && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::DoubleColon) {
                    let class = match self.take() { TokenKind::Word(class) => class, _ => unreachable!() };
                    self.take();
                    let property = match self.take() { TokenKind::Var(property) => property, token => return Err(self.error(&format!("Expected static property name in unset, got {token:?}"))) };
                    let mut keys = Vec::new();
                    while self.eat(&TokenKind::Ls) { keys.push(self.expression(0)?); self.expect(TokenKind::Rs, "Expected ] after static property array key")?; }
                    if keys.is_empty() { return Err(self.error("unset() static property target must include an array offset")); }
                    self.expect(TokenKind::Rp, "Expected ) after unset")?;
                    (Stmt::UnsetStaticPropertyPath { class, property, keys }, true)
                } else {
                let variable = match self.take() {
                    TokenKind::Var(name) => name,
                    token => return Err(self.error(&format!("Expected variable in unset, got {token:?}"))),
                };
                // Object-property unset needs its own AST node so runtime magic __unset can run.
                if self.eat(&TokenKind::ObjectArrow) {
                    if self.eat(&TokenKind::Lb) {
                        let property = self.expression(0)?;
                        self.expect(TokenKind::Rb, "Expected } after dynamic property in unset")?;
                        self.expect(TokenKind::Rp, "Expected ) after unset dynamic property")?;
                        self.eat(&TokenKind::Semi);
                        return Ok(Stmt::SourceLine(statement_line, Box::new(Stmt::UnsetDynamicProperty { object: Expr::Variable(variable), property })));
                    }
                    if let TokenKind::Var(property) = self.peek().clone() {
                        self.take();
                        self.expect(TokenKind::Rp, "Expected ) after dynamic property unset")?;
                        self.eat(&TokenKind::Semi);
                        return Ok(Stmt::SourceLine(statement_line, Box::new(Stmt::UnsetDynamicProperty { object: Expr::Variable(variable), property: Expr::Variable(property) })));
                    }
                    let property_offset = self.peek_offset();
                    match self.take() {
                        TokenKind::Word(_) => {},
                        token => return Err(self.error(&format!("Expected property in unset, got {token:?}"))),
                    };
                    let mut receiver = Expr::Variable(variable.clone());
                    let mut property = self.source_word(property_offset);
                    while self.eat(&TokenKind::ObjectArrow) {
                        receiver = Expr::Property(Box::new(receiver), property);
                        if self.eat(&TokenKind::Lb) {
                            let dynamic_property = self.expression(0)?;
                            self.expect(TokenKind::Rb, "Expected } after dynamic property in unset")?;
                            self.expect(TokenKind::Rp, "Expected ) after dynamic property unset")?;
                            self.eat(&TokenKind::Semi);
                            return Ok(Stmt::SourceLine(statement_line, Box::new(Stmt::UnsetDynamicProperty { object: receiver, property: dynamic_property })));
                        }
                        let next_offset = self.peek_offset();
                        match self.take() { TokenKind::Word(_) => {}, token => return Err(self.error(&format!("Expected property in unset chain, got {token:?}"))) }
                        property = self.source_word(next_offset);
                    }
                    let item = if self.eat(&TokenKind::Ls) {
                        let mut keys = vec![self.expression(0)?];
                        self.expect(TokenKind::Rs, "Expected ] after property array offset")?;
                        while self.eat(&TokenKind::Ls) { keys.push(self.expression(0)?); self.expect(TokenKind::Rs, "Expected ] after property array offset")?; }
                        Stmt::UnsetPropertyArrayPath { object: receiver, property, keys }
                    } else {
                        Stmt::UnsetProperty { object: variable, property }
                    };
                    let mut items = vec![item];
                    while self.eat(&TokenKind::Comma) {
                        if self.peek() == &TokenKind::Rp { break; }
                        let object = match self.take() { TokenKind::Var(object) => object, token => return Err(self.error(&format!("Expected object variable in unset list, got {token:?}"))) };
                        self.expect(TokenKind::ObjectArrow, "Expected -> in unset property target")?;
                        let offset = self.peek_offset();
                        match self.take() { TokenKind::Word(_) => {}, token => return Err(self.error(&format!("Expected property name in unset list, got {token:?}"))) }
                        let property = self.source_word(offset);
                        if self.eat(&TokenKind::Ls) {
                            let mut keys = vec![self.expression(0)?];
                            self.expect(TokenKind::Rs, "Expected ] after property array offset")?;
                            while self.eat(&TokenKind::Ls) { keys.push(self.expression(0)?); self.expect(TokenKind::Rs, "Expected ] after property array offset")?; }
                            items.push(Stmt::UnsetPropertyArrayPath { object: Expr::Variable(object), property, keys });
                        } else { items.push(Stmt::UnsetProperty { object, property }); }
                    }
                    self.expect(TokenKind::Rp, "Expected ) after unset property")?;
                    (if items.len() == 1 { items.pop().unwrap() } else { Stmt::UnsetMany(items) }, true)
                } else {
                let mut keys = Vec::new();
                while self.eat(&TokenKind::Ls) {
                    keys.push(self.expression(0)?);
                    self.expect(TokenKind::Rs, "Expected ] after array key")?;
                }
                let mut items = vec![Stmt::Unset { variable, keys }];
                while self.eat(&TokenKind::Comma) {
                    if self.peek() == &TokenKind::Rp { break; }
                    let variable = match self.take() { TokenKind::Var(variable) => variable, token => return Err(self.error(&format!("Expected variable in unset list, got {token:?}"))) };
                    if self.eat(&TokenKind::ObjectArrow) {
                        let offset = self.peek_offset();
                        match self.take() { TokenKind::Word(_) => {}, token => return Err(self.error(&format!("Expected property name in unset list, got {token:?}"))) }
                        let property = self.source_word(offset);
                        if self.eat(&TokenKind::Ls) {
                            let mut keys = vec![self.expression(0)?];
                            self.expect(TokenKind::Rs, "Expected ] after property array offset")?;
                            while self.eat(&TokenKind::Ls) { keys.push(self.expression(0)?); self.expect(TokenKind::Rs, "Expected ] after property array offset")?; }
                            items.push(Stmt::UnsetPropertyArrayPath { object: Expr::Variable(variable), property, keys });
                        } else {
                            items.push(Stmt::UnsetProperty { object: variable, property });
                        }
                    } else {
                        let mut keys = Vec::new();
                        while self.eat(&TokenKind::Ls) { keys.push(self.expression(0)?); self.expect(TokenKind::Rs, "Expected ] after array key")?; }
                        items.push(Stmt::Unset { variable, keys });
                    }
                }
                self.expect(TokenKind::Rp, "Expected ) after unset")?;
                (if items.len() == 1 { items.pop().unwrap() } else { Stmt::UnsetMany(items) }, true)
                }
                }
            }
            TokenKind::Word(class) if self.peek() == &TokenKind::DoubleColon && matches!(self.tokens.get(self.cursor + 1).map(|token| &token.kind), Some(TokenKind::Var(_))) => {
                self.take();
                let property = match self.take() {
                    TokenKind::Var(name) => name,
                    _ => unreachable!("lookahead verified static property token"),
                };
                let reference_start = self.cursor;
                let mut target_keys = Vec::new();
                while self.peek() == &TokenKind::Ls {
                    if self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Rs) {
                        self.cursor = statement_start;
                        let statement = Stmt::Expression(self.expression(0)?);
                        if !self.eat(&TokenKind::Semi) && !matches!(self.peek(), TokenKind::Rb | TokenKind::Eof) {
                            return Err(self.error("Expected ;"));
                        }
                        return Ok(Stmt::SourceLine(statement_line, Box::new(statement)));
                    }
                    self.take();
                    target_keys.push(self.expression(0)?);
                    self.expect(TokenKind::Rs, "Expected ] after static property array key")?;
                }
                if !target_keys.is_empty() && self.eat(&TokenKind::Eq) && self.eat(&TokenKind::Ampersand) {
                    let source_class = match self.take() {
                        TokenKind::Word(name) => name,
                        token => return Err(self.error(&format!("Expected class name in static property reference, got {token:?}"))),
                    };
                    self.expect(TokenKind::DoubleColon, "Expected :: in static property reference")?;
                    let source_property = match self.take() {
                        TokenKind::Var(name) => name,
                        token => return Err(self.error(&format!("Expected static property in reference source, got {token:?}"))),
                    };
                    let mut source_keys = Vec::new();
                    while self.eat(&TokenKind::Ls) {
                        source_keys.push(self.expression(0)?);
                        self.expect(TokenKind::Rs, "Expected ] after static property array key")?;
                    }
                    if source_keys.is_empty() { return Err(self.error("Static property reference source must select an array element")); }
                    return Ok(Stmt::SourceLine(statement_line, Box::new(Stmt::StaticPropertyArrayReferenceAssign {
                        target_class: class, target_property: property, target_keys,
                        source_class, source_property, source_keys,
                    })));
                }
                self.cursor = reference_start;
                if matches!(self.peek(), TokenKind::Ls | TokenKind::ObjectArrow | TokenKind::NullsafeArrow | TokenKind::DoubleColon | TokenKind::Lp)
                    || matches!(self.peek(), TokenKind::PlusPlus | TokenKind::MinusMinus) {
                    self.cursor = statement_start;
                    (Stmt::Expression(self.expression(0)?), true)
                } else {
                let assignment_operator = self.take();
                let operation = match assignment_operator {
                    TokenKind::Eq => None,
                    TokenKind::PlusEq => Some(crate::ast::BinaryOp::Add),
                    TokenKind::MinusEq => Some(crate::ast::BinaryOp::Subtract),
                    TokenKind::StarEq => Some(crate::ast::BinaryOp::Multiply),
                    TokenKind::PowerEq => Some(crate::ast::BinaryOp::Power),
                    TokenKind::SlashEq => Some(crate::ast::BinaryOp::Divide),
                    TokenKind::PercentEq => Some(crate::ast::BinaryOp::Modulo),
                    TokenKind::DotEq => Some(crate::ast::BinaryOp::Concatenate),
                    TokenKind::AmpersandEq => Some(crate::ast::BinaryOp::BitAnd),
                    TokenKind::CaretEq => Some(crate::ast::BinaryOp::BitXor),
                    TokenKind::PipeEq => Some(crate::ast::BinaryOp::BitOr),
                    TokenKind::ShiftLeftEq => Some(crate::ast::BinaryOp::ShiftLeft),
                    TokenKind::ShiftRightEq => Some(crate::ast::BinaryOp::ShiftRight),
                    TokenKind::CoalesceEq => Some(crate::ast::BinaryOp::Coalesce),
                    _ => return Err(self.error("Expected = or a compound assignment operator for static property")),
                };
                let value = self.expression(1)?;
                let target = AssignTarget::StaticProperty { class, property };
                match operation {
                    Some(operation) => (Stmt::CompoundAssign { target, operation, value }, true),
                    None => (Stmt::Assign { target, value }, true),
                }
                }
            }
            TokenKind::Var(name) => {
                let method_call = self.peek() == &TokenKind::ObjectArrow
                    && matches!(self.tokens.get(self.cursor + 1).map(|token| &token.kind), Some(TokenKind::Word(_)))
                    && matches!(self.tokens.get(self.cursor + 2).map(|token| &token.kind), Some(TokenKind::Lp));
                let dynamic_property = self.peek() == &TokenKind::ObjectArrow
                    && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Lb);
                let dynamic_method_call = self.peek() == &TokenKind::ObjectArrow
                    && matches!(self.tokens.get(self.cursor + 1).map(|token| &token.kind), Some(TokenKind::Var(_)))
                    && self.tokens.get(self.cursor + 2).is_some_and(|token| token.kind == TokenKind::Lp);
                let dynamic_variable_property = self.peek() == &TokenKind::ObjectArrow
                    && matches!(self.tokens.get(self.cursor + 1).map(|token| &token.kind), Some(TokenKind::Var(_)))
                    && !self.tokens.get(self.cursor + 2).is_some_and(|token| token.kind == TokenKind::Lp);
                let dynamic_property_reference = dynamic_property
                    && (self.cursor + 4..self.tokens.len()).any(|index| self.tokens[index].kind == TokenKind::Eq
                        && self.tokens.get(index + 1).is_some_and(|token| token.kind == TokenKind::Ampersand)
                        && self.tokens.get(index - 1).is_some_and(|token| matches!(token.kind, TokenKind::Rb | TokenKind::Rs)));
                if dynamic_property_reference {
                    self.expect(TokenKind::ObjectArrow, "Expected -> before dynamic property")?;
                    self.expect(TokenKind::Lb, "Expected { before dynamic property name")?;
                    let property = self.expression(0)?;
                    self.expect(TokenKind::Rb, "Expected } after dynamic property name")?;
                    let mut target_keys = Vec::new();
                    while self.eat(&TokenKind::Ls) {
                        target_keys.push(self.expression(0)?);
                        self.expect(TokenKind::Rs, "Expected ] after dynamic property array key")?;
                    }
                    self.expect(TokenKind::Eq, "Expected = in dynamic property reference assignment")?;
                    self.expect(TokenKind::Ampersand, "Expected & in dynamic property reference assignment")?;
                    let source_expr = self.expression(0)?;
                    fn reference_path(source: Expr) -> Option<(String, Vec<Expr>)> {
                        match source {
                            Expr::Variable(variable) => Some((variable, Vec::new())),
                            Expr::Index(array, key) => { let mut path = reference_path(*array)?; path.1.push(*key); Some(path) }
                            _ => None,
                        }
                    }
                    if target_keys.is_empty() {
                        (Stmt::DynamicPropertyReferenceAssign { receiver: Expr::Variable(name), property, source: source_expr }, true)
                    } else {
                        let (source, source_keys) = reference_path(source_expr)
                            .ok_or_else(|| self.error("Dynamic property array reference source must be a variable or array element"))?;
                        (Stmt::DynamicPropertyArrayReferenceAssign { receiver: Expr::Variable(name), property, target_keys, source, source_keys }, true)
                    }
                } else if method_call || dynamic_property || dynamic_method_call || dynamic_variable_property {
                    self.cursor -= 1;
                    (Stmt::Expression(self.expression(0)?), true)
                } else if self.peek() == &TokenKind::Lp {
                    // A variable followed by `(` is an invocation expression, such as `$callback(...)`.
                    self.cursor -= 1;
                    (Stmt::Expression(self.expression(0)?), true)
                } else if self.eat(&TokenKind::ObjectArrow) {
                    let property_offset = self.peek_offset();
                    match self.take() {
                        TokenKind::Word(_) => {},
                        token => return Err(self.error(&format!("Expected property name, got {token:?}"))),
                    };
                    if self.peek() == &TokenKind::Ls
                        && !self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Rs)
                        && self.array_path_assignment_after(true) {
                        let mut target_keys = Vec::new();
                        while self.eat(&TokenKind::Ls) {
                            target_keys.push(self.expression(0)?);
                            self.expect(TokenKind::Rs, "Expected ] after property array key")?;
                        }
                        self.expect(TokenKind::Eq, "Expected = in property array reference assignment")?;
                        self.expect(TokenKind::Ampersand, "Expected & in property array reference assignment")?;
                        let source_expr = self.expression(0)?;
                        fn reference_path(source: Expr) -> Option<(String, Vec<Expr>)> {
                            match source {
                                Expr::Variable(variable) => Some((variable, Vec::new())),
                                Expr::Index(array, key) => { let mut path = reference_path(*array)?; path.1.push(*key); Some(path) }
                                _ => None,
                            }
                        }
                        let (source, source_keys) = reference_path(source_expr)
                            .ok_or_else(|| self.error("Property array reference source must be a variable or array element"))?;
                        (Stmt::PropertyArrayReferenceAssign { receiver: Expr::Variable(name), property: self.source_word(property_offset), target_keys, source, source_keys }, true)
                    } else if (self.peek() == &TokenKind::ObjectArrow || self.peek() == &TokenKind::NullsafeArrow || self.peek() == &TokenKind::DoubleColon)
                        || (self.peek() == &TokenKind::Ls && !self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Rs)) {
                        self.cursor = statement_start;
                        (Stmt::Expression(self.expression(0)?), true)
                    } else if self.eat(&TokenKind::Ls) && self.eat(&TokenKind::Rs) {
                        self.expect(TokenKind::Eq, "Expected = after property array append target")?;
                        if self.eat(&TokenKind::Ampersand) {
                            let source = self.expression(0)?;
                            (Stmt::PropertyAppendReferenceAssign { receiver: Expr::Variable(name), property: self.source_word(property_offset), source }, true)
                        } else {
                        let value = self.expression(1)?;
                        (Stmt::Expression(crate::ast::Expr::PropertyAppendAssignment {
                            receiver: Box::new(crate::ast::Expr::Variable(name)),
                            property: self.source_word(property_offset),
                            keys: Vec::new(),
                            value: Box::new(value),
                        }), true)
                        }
                    } else if matches!(self.peek(), TokenKind::PlusPlus | TokenKind::MinusMinus) {
                        self.cursor = statement_start;
                        (Stmt::Expression(self.expression(0)?), true)
                    } else if !matches!(self.peek(), TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq) {
                        self.cursor = statement_start;
                        (Stmt::Expression(self.expression(0)?), true)
                    } else {
                    let assignment_operator = self.take();
                    let operation = match assignment_operator {
                        TokenKind::Eq => None,
                        TokenKind::PlusEq => Some(crate::ast::BinaryOp::Add),
                        TokenKind::MinusEq => Some(crate::ast::BinaryOp::Subtract),
                        TokenKind::StarEq => Some(crate::ast::BinaryOp::Multiply),
                        TokenKind::PowerEq => Some(crate::ast::BinaryOp::Power),
                        TokenKind::SlashEq => Some(crate::ast::BinaryOp::Divide),
                        TokenKind::PercentEq => Some(crate::ast::BinaryOp::Modulo),
                        TokenKind::DotEq => Some(crate::ast::BinaryOp::Concatenate),
                    TokenKind::AmpersandEq => Some(crate::ast::BinaryOp::BitAnd),
                    TokenKind::CaretEq => Some(crate::ast::BinaryOp::BitXor),
                    TokenKind::PipeEq => Some(crate::ast::BinaryOp::BitOr),
                    TokenKind::ShiftLeftEq => Some(crate::ast::BinaryOp::ShiftLeft),
                    TokenKind::ShiftRightEq => Some(crate::ast::BinaryOp::ShiftRight),
                        TokenKind::CoalesceEq => Some(crate::ast::BinaryOp::Coalesce),
                        _ => return Err(self.error("Expected = or a compound assignment operator for property")),
                    };
                    if assignment_operator == TokenKind::Eq { self.eat(&TokenKind::Ampersand); }
                    let value = self.expression(1)?;
                    let target = AssignTarget::Property { object: name, property: self.source_word(property_offset) };
                    match operation {
                        Some(operation) => (Stmt::CompoundAssign { target, operation, value }, true),
                        None => (Stmt::Assign { target, value }, true),
                    }
                    }
                } else if self.eat(&TokenKind::PlusPlus) {
                    (Stmt::Expression(crate::ast::Expr::PostIncrement(crate::ast::IncrementTarget::Variable(name), 1)), true)
                } else if self.eat(&TokenKind::MinusMinus) {
                    (Stmt::Expression(crate::ast::Expr::PostIncrement(crate::ast::IncrementTarget::Variable(name), -1)), true)
                } else if !matches!(self.peek(), TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq)
                    && !self.array_path_assignment_after(false) {
                    self.cursor = statement_start;
                    (Stmt::Expression(self.expression(0)?), true)
                } else {
                let mut keys = Vec::new();
                let mut segments = Vec::new();
                while self.eat(&TokenKind::Ls) {
                    if self.eat(&TokenKind::Rs) {
                        segments.push(crate::ast::ArrayPathSegment::Append);
                    } else {
                        let key = self.expression(0)?;
                        self.expect(TokenKind::Rs, "Expected ] after array key")?;
                        keys.push(key.clone());
                        segments.push(crate::ast::ArrayPathSegment::Key(key));
                    }
                }
                let target = if segments.iter().any(|segment| matches!(segment, crate::ast::ArrayPathSegment::Append))
                    && !(matches!(segments.last(), Some(crate::ast::ArrayPathSegment::Append))
                        && segments[..segments.len().saturating_sub(1)].iter().all(|segment| matches!(segment, crate::ast::ArrayPathSegment::Key(_))))
                {
                    AssignTarget::ArrayPath { variable: name, segments }
                } else if matches!(segments.last(), Some(crate::ast::ArrayPathSegment::Append)) {
                    AssignTarget::Append { variable: name, keys }
                } else if keys.is_empty() {
                    AssignTarget::Variable(name)
                } else {
                    AssignTarget::Index { variable: name, keys }
                };
                if self.peek() == &TokenKind::Eq && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Ampersand) {
                    self.take();
                    self.take();
                    let source_expr = self.expression(0)?;
                    fn reference_path(source: Expr) -> Option<(String, Vec<Expr>)> {
                        match source {
                            Expr::Variable(variable) => Some((variable, Vec::new())),
                            Expr::Index(array, key) => { let mut path = reference_path(*array)?; path.1.push(*key); Some(path) }
                            _ => None,
                        }
                    }
                    if let Some((source, source_keys)) = reference_path(source_expr.clone()) {
                        return match target {
                            AssignTarget::Index { variable: target, keys: target_keys } => Ok(Stmt::ArrayReferenceAssign { target, target_keys, source, source_keys }),
                            AssignTarget::Variable(target) if source_keys.is_empty() => Ok(Stmt::ReferenceAssign { target, source: Expr::Variable(source) }),
                            AssignTarget::Variable(target) => Ok(Stmt::ReferenceAssign { target, source: source_expr }),
                            _ => Err(self.error("Reference assignment target must be a variable or array element")),
                        };
                    }
                    return match target {
                        AssignTarget::Variable(target) => Ok(Stmt::ReferenceAssign { target, source: source_expr }),
                        AssignTarget::Index { variable: target, keys: target_keys } if matches!(source_expr, Expr::Property(_, _)) => {
                            Ok(Stmt::ArrayReferenceAssignExpr { target, target_keys, source: source_expr })
                        }
                        _ => Err(self.error("Reference assignment target must be a variable or array element")),
                    };
                }
                let assignment_follows = matches!(self.peek(), TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq);
                if !assignment_follows && !matches!(self.peek(), TokenKind::Semi | TokenKind::Rb | TokenKind::Eof | TokenKind::PlusPlus | TokenKind::MinusMinus) {
                    self.cursor = statement_start;
                    (Stmt::Expression(self.expression(0)?), true)
                } else if matches!(self.peek(), TokenKind::PlusPlus | TokenKind::MinusMinus) {
                    let delta = if self.eat(&TokenKind::PlusPlus) { 1 } else { self.expect(TokenKind::MinusMinus, "Expected increment or decrement")?; -1 };
                    let target = match target {
                        AssignTarget::Index { variable, keys } => crate::ast::IncrementTarget::ArrayPath { variable, keys },
                        AssignTarget::Variable(variable) => crate::ast::IncrementTarget::Variable(variable),
                        _ => return Err(self.error("Increment requires a variable or array element")),
                    };
                    (Stmt::Expression(crate::ast::Expr::PostIncrement(target, delta)), true)
                } else {
                // Keep simple assignments in the expression parser so low-precedence
                // word operators (`$value = false or true`) bind after assignment.
                if matches!(target, AssignTarget::Variable(_))
                    && matches!(self.peek(), TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq)
                    && !(self.peek() == &TokenKind::Eq && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Ampersand))
                {
                    self.cursor = statement_start;
                    (Stmt::Expression(self.expression(0)?), true)
                } else {
                let assignment_operator = self.take();
                if assignment_operator == TokenKind::Eq && self.eat(&TokenKind::Ampersand) {
                    let source = self.expression(0)?;
                    fn reference_path(source: crate::ast::Expr) -> Option<(String, Vec<crate::ast::Expr>)> {
                        match source {
                            crate::ast::Expr::Variable(variable) => Some((variable, Vec::new())),
                            crate::ast::Expr::Index(array, key) => {
                                let mut path = reference_path(*array)?;
                                path.1.push(*key);
                                Some(path)
                            }
                            _ => None,
                        }
                    }
                    if let Some((source, source_keys)) = reference_path(source.clone()) {
                        match target {
                            AssignTarget::Variable(target) if source_keys.is_empty() => {
                                (Stmt::ReferenceAssign { target, source: crate::ast::Expr::Variable(source) }, true)
                            }
                            AssignTarget::Variable(target) => {
                                (Stmt::ReferenceAssign { target, source: {
                                    let mut expression = crate::ast::Expr::Variable(source);
                                    for key in source_keys { expression = crate::ast::Expr::Index(Box::new(expression), Box::new(key)); }
                                    expression
                                } }, true)
                            }
                            AssignTarget::Index { variable: target, keys: target_keys } => {
                                (Stmt::ArrayReferenceAssign { target, target_keys, source, source_keys }, true)
                            }
                            _ => return Err(self.error("Reference assignment target must be a variable or array element")),
                        }
                    } else if matches!(&source, crate::ast::Expr::Call(_, _) | crate::ast::Expr::Invoke(_, _) | crate::ast::Expr::MethodCall(_, _, _) | crate::ast::Expr::StaticCall(_, _, _) | crate::ast::Expr::DynamicStaticCall(_, _, _) | crate::ast::Expr::Property(_, _)) || is_static_property_reference_source(&source) {
                        match target {
                            AssignTarget::Variable(target) => (Stmt::ReferenceAssign { target, source }, true),
                            _ => return Err(self.error("A callable reference result can only be assigned to a variable")),
                        }
                    } else {
                        return Err(self.error("Reference assignment source must be a variable, array element, or reference-returning call"));
                    }
                } else {
                let value = self.expression(1)?;
                let statement = match assignment_operator {
                    TokenKind::Eq => Stmt::Assign { target, value },
                    TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq => {
                        if !matches!(&target, AssignTarget::Variable(_) | AssignTarget::Index { .. }) {
                            return Err(self.error("Compound assignment currently requires a variable or array element"));
                        }
                        let operation = match assignment_operator {
                            TokenKind::PlusEq => crate::ast::BinaryOp::Add,
                            TokenKind::MinusEq => crate::ast::BinaryOp::Subtract,
                            TokenKind::StarEq => crate::ast::BinaryOp::Multiply,
                            TokenKind::PowerEq => crate::ast::BinaryOp::Power,
                            TokenKind::SlashEq => crate::ast::BinaryOp::Divide,
                            TokenKind::PercentEq => crate::ast::BinaryOp::Modulo,
                            TokenKind::DotEq => crate::ast::BinaryOp::Concatenate,
                            TokenKind::AmpersandEq => crate::ast::BinaryOp::BitAnd,
                            TokenKind::CaretEq => crate::ast::BinaryOp::BitXor,
                            TokenKind::PipeEq => crate::ast::BinaryOp::BitOr,
                            TokenKind::ShiftLeftEq => crate::ast::BinaryOp::ShiftLeft,
                            TokenKind::ShiftRightEq => crate::ast::BinaryOp::ShiftRight,
                            TokenKind::CoalesceEq => crate::ast::BinaryOp::Coalesce,
                            _ => unreachable!(),
                        };
                        Stmt::CompoundAssign { target, operation, value }
                    }
                    _ => return Err(self.error("Expected = or a supported compound assignment operator")),
                };
                (statement, true)
                }
                }
                }
                }
            }
            TokenKind::DynamicVar(_) => {
                if !matches!(self.peek(), TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq) {
                    return Err(self.error("Expected assignment operator in variable-variable assignment"));
                }
                self.cursor = statement_start;
                (Stmt::Expression(self.expression(0)?), true)
            }
            TokenKind::DynamicExprStart => {
                let _name = self.expression(0)?;
                self.expect(TokenKind::Rb, "Expected } after dynamic variable target")?;
                if !matches!(self.peek(), TokenKind::Eq | TokenKind::PlusEq | TokenKind::MinusEq | TokenKind::StarEq | TokenKind::PowerEq | TokenKind::SlashEq | TokenKind::PercentEq | TokenKind::DotEq | TokenKind::AmpersandEq | TokenKind::CaretEq | TokenKind::PipeEq | TokenKind::ShiftLeftEq | TokenKind::ShiftRightEq | TokenKind::CoalesceEq) {
                    return Err(self.error("Expected assignment operator after dynamic variable target"));
                }
                self.cursor = statement_start;
                (Stmt::Expression(self.expression(0)?), true)
            }
            _ => {
                self.cursor -= 1;
                (Stmt::Expression(self.expression(0)?), true)
            }
        };

        if needs_semicolon && !self.eat(&TokenKind::Semi) && !matches!(self.peek(), TokenKind::Rb | TokenKind::Eof) {
            return Err(self.error("Expected ;"));
        }
        if !needs_semicolon { self.eat(&TokenKind::Semi); }
        Ok(Stmt::SourceLine(statement_line, Box::new(statement)))
    }

}
