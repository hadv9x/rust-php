//! Expression parser and operator-precedence handling.

use crate::{ast::{BinaryOp, Expr}, lexer::token::TokenKind};
use super::Parser;
use crate::ast::IncrementTarget;

impl Parser<'_> {
    fn anonymous_class_declaration(&mut self, attributes: Vec<crate::ast::AttributeDecl>, is_readonly: bool, source_line: usize) -> Result<String, String> {
        let id = self.next_anonymous_class_id;
        self.next_anonymous_class_id += 1;
        let name = format!("class@anonymous#{id}");
        let parent = if matches!(self.peek(), TokenKind::Word(word) if word == "extends") {
            self.take();
            match self.take() {
                TokenKind::Word(parent) => Some(self.qualify_name(parent)),
                token => return Err(self.error(&format!("Expected anonymous class parent, got {token:?}"))),
            }
        } else { None };
        let mut interfaces = Vec::new();
        if matches!(self.peek(), TokenKind::Word(word) if word == "implements") {
            self.take();
            loop {
                match self.take() {
                    TokenKind::Word(interface) => interfaces.push(self.qualify_name(interface)),
                    token => return Err(self.error(&format!("Expected anonymous class interface, got {token:?}"))),
                }
                if !self.eat(&TokenKind::Comma) { break; }
            }
        }
        self.expect(TokenKind::Lb, "Expected { in anonymous class")?;
        let mut properties = Vec::new();
        let mut methods = Vec::new();
        let mut traits = Vec::new();
        let mut trait_adaptations = Vec::new();
        while !self.eat(&TokenKind::Rb) {
            if self.peek() == &TokenKind::Eof { return Err(self.error("Unterminated anonymous class")); }
            if matches!(self.peek(), TokenKind::Word(word) if word == "use") {
                self.take();
                loop {
                    match self.take() { TokenKind::Word(trait_name) => traits.push(self.qualify_name(self.previous_word_spelling(&trait_name))), token => return Err(self.error(&format!("Expected trait name in anonymous class, got {token:?}"))) }
                    if !self.eat(&TokenKind::Comma) { break; }
                }
                if self.eat(&TokenKind::Semi) { continue; }
                self.expect(TokenKind::Lb, "Expected ; or { after anonymous class trait use")?;
                while !self.eat(&TokenKind::Rb) {
                    let first = match self.take() { TokenKind::Word(name) => self.previous_word_spelling(&name), token => return Err(self.error(&format!("Expected trait method adaptation, got {token:?}"))) };
                    let (trait_name, method_name) = if self.eat(&TokenKind::DoubleColon) {
                        let method = match self.take() { TokenKind::Word(name) => self.previous_word_spelling(&name), token => return Err(self.error(&format!("Expected trait method name, got {token:?}"))) };
                        (self.qualify_name(first), method)
                    } else { (traits.last().cloned().ok_or_else(|| self.error("Unqualified trait adaptation requires a trait use"))?, first) };
                    let mut alias = None;
                    let mut visibility = None;
                    let mut excluded_traits = Vec::new();
                    let mut is_final = false;
                    match self.take() {
                        TokenKind::Word(word) if word == "as" => {
                            while let TokenKind::Word(word) = self.peek() {
                                match word.as_str() {
                                    "public" => { visibility = Some(crate::ast::Visibility::Public); self.take(); }
                                    "protected" => { visibility = Some(crate::ast::Visibility::Protected); self.take(); }
                                    "private" => { visibility = Some(crate::ast::Visibility::Private); self.take(); }
                                    "final" => { is_final = true; self.take(); }
                                    _ => break,
                                }
                            }
                            if let TokenKind::Word(name) = self.peek() { let fallback = name.clone(); self.take(); alias = Some(self.previous_word_spelling(&fallback)); }
                            if alias.is_none() && visibility.is_none() && !is_final { return Err(self.error("Trait alias must specify a name, visibility, or final modifier")); }
                        }
                        TokenKind::Word(word) if word == "insteadof" => loop {
                            match self.take() { TokenKind::Word(name) => excluded_traits.push(self.qualify_name(self.previous_word_spelling(&name))), token => return Err(self.error(&format!("Expected excluded trait, got {token:?}"))) }
                            if !self.eat(&TokenKind::Comma) { break; }
                        },
                        token => return Err(self.error(&format!("Expected insteadof or as, got {token:?}"))),
                    }
                    self.expect(TokenKind::Semi, "Expected ; after trait adaptation")?;
                    trait_adaptations.push(crate::ast::TraitAdaptation { trait_name, method_name, excluded_traits, alias, visibility, is_final });
                }
                continue;
            }
            let member_attributes = self.attributes()?;
            let mut visibility = crate::ast::Visibility::Public;
            let mut is_static = false;
            let mut is_abstract = false;
            let mut is_final = false;
            loop {
                match self.peek() {
                    TokenKind::Word(word) if word == "public" => { self.take(); visibility = crate::ast::Visibility::Public; }
                    TokenKind::Word(word) if word == "protected" => { self.take(); visibility = crate::ast::Visibility::Protected; }
                    TokenKind::Word(word) if word == "private" => { self.take(); visibility = crate::ast::Visibility::Private; }
                    TokenKind::Word(word) if word == "static" => { self.take(); is_static = true; }
                    TokenKind::Word(word) if word == "abstract" => { self.take(); is_abstract = true; }
                    TokenKind::Word(word) if word == "final" => { self.take(); is_final = true; }
                    _ => break,
                }
            }
            if matches!(self.peek(), TokenKind::Word(word) if word == "function") {
                let mut method = self.function_declaration(is_static, is_abstract, is_final, visibility)?;
                method.attributes = member_attributes;
                methods.push(method);
                continue;
            }
            let nullable = self.eat(&TokenKind::Question);
            let has_type = (matches!(self.peek(), TokenKind::Word(_)) && matches!(self.tokens.get(self.cursor + 1).map(|token| &token.kind), Some(TokenKind::Var(_) | TokenKind::Pipe | TokenKind::Ampersand))) || matches!(self.peek(), TokenKind::Lp);
            let type_name = if has_type { Some(self.parse_union_type(nullable)?) } else if nullable { return Err(self.error("Expected anonymous class property type after ?")); } else { None };
            let property = match self.take() {
                TokenKind::Var(property) => property,
                token => return Err(self.error(&format!("Expected anonymous class method or property, got {token:?}"))),
            };
            let default = if self.eat(&TokenKind::Eq) { Some(self.expression(1)?) } else { None };
            self.expect(TokenKind::Semi, "Expected ; after anonymous class property")?;
            properties.push(crate::ast::PropertyDecl { name: property, attributes: member_attributes, default, is_static, is_readonly, visibility, set_visibility: None, type_name, get_hook: None, set_hook: None });
        }
        self.anonymous_classes.push(crate::ast::ClassDecl {
            name: name.clone(), anonymous_source: Some((None, source_line)), attributes, is_enum: false, enum_backing_type: None, is_abstract: false, is_final: false, is_readonly,
            parent, interfaces, traits, trait_adaptations, properties,
            constants: Vec::new(), methods,
        });
        Ok(name)
    }
}

fn increment_target(expression: crate::ast::Expr) -> Option<IncrementTarget> {
    match expression {
        crate::ast::Expr::Variable(variable) => Some(IncrementTarget::Variable(variable)),
        crate::ast::Expr::DynamicVariable(name) => Some(IncrementTarget::DynamicVariable { name: Box::new(crate::ast::Expr::Variable(name)), keys: Vec::new() }),
        crate::ast::Expr::DynamicVariableExpr(name) => Some(IncrementTarget::DynamicVariable { name, keys: Vec::new() }),
        crate::ast::Expr::Index(array, key) => {
            let target = increment_target(*array)?;
            match target {
                IncrementTarget::Variable(variable) => Some(IncrementTarget::ArrayPath { variable, keys: vec![*key] }),
                IncrementTarget::ArrayPath { variable, mut keys } => { keys.push(*key); Some(IncrementTarget::ArrayPath { variable, keys }) }
                IncrementTarget::DynamicVariable { name, mut keys } => { keys.push(*key); Some(IncrementTarget::DynamicVariable { name, keys }) }
                IncrementTarget::Property { receiver, property } => Some(IncrementTarget::PropertyArrayPath { receiver, property, keys: vec![*key] }),
                IncrementTarget::StaticProperty { class, property } => Some(IncrementTarget::StaticPropertyArrayPath { class, property, keys: vec![*key] }),
                IncrementTarget::DynamicProperty { receiver, property } => Some(IncrementTarget::DynamicPropertyArrayPath { receiver, property, keys: vec![*key] }),
                IncrementTarget::DynamicStaticPropertyArrayPath { class, property, mut keys } => { keys.push(*key); Some(IncrementTarget::DynamicStaticPropertyArrayPath { class, property, keys }) }
                IncrementTarget::PropertyArrayPath { receiver, property, mut keys } => { keys.push(*key); Some(IncrementTarget::PropertyArrayPath { receiver, property, keys }) }
                IncrementTarget::StaticPropertyArrayPath { class, property, mut keys } => { keys.push(*key); Some(IncrementTarget::StaticPropertyArrayPath { class, property, keys }) }
                IncrementTarget::DynamicPropertyArrayPath { receiver, property, mut keys } => { keys.push(*key); Some(IncrementTarget::DynamicPropertyArrayPath { receiver, property, keys }) }
            }
        }
        crate::ast::Expr::Property(receiver, property) => Some(IncrementTarget::Property { receiver, property }),
        crate::ast::Expr::DynamicProperty(receiver, property) => Some(IncrementTarget::DynamicProperty { receiver, property }),
        crate::ast::Expr::DynamicStaticProperty(class, property) => Some(IncrementTarget::DynamicStaticPropertyArrayPath { class, property: Box::new(crate::ast::Expr::String(property)), keys: Vec::new() }),
        crate::ast::Expr::DynamicStaticPropertyExpr(class, property) => Some(IncrementTarget::DynamicStaticPropertyArrayPath { class, property, keys: Vec::new() }),
        crate::ast::Expr::StaticProperty(class, property) => Some(IncrementTarget::StaticProperty { class, property }),
        _ => None,
    }
}

fn assignment_array_path(expression: crate::ast::Expr) -> Option<(String, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::Index(array, key) => {
            let mut path = match *array {
                crate::ast::Expr::Variable(variable) => (variable, Vec::new()),
                nested => assignment_array_path(nested)?,
            };
            path.1.push(*key);
            Some(path)
        }
        _ => None,
    }
}

fn property_assignment_path(expression: crate::ast::Expr) -> Option<(crate::ast::Expr, String, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::Index(array, key) => {
            let (receiver, property, mut keys) = match *array {
                crate::ast::Expr::Property(receiver, property) => (*receiver, property, Vec::new()),
                nested => property_assignment_path(nested)?,
            };
            keys.push(*key);
            Some((receiver, property, keys))
        }
        _ => None,
    }
}

fn static_property_assignment_path(expression: crate::ast::Expr) -> Option<(String, String, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::Index(array, key) => {
            let (class, property, mut keys) = match *array {
                crate::ast::Expr::StaticProperty(class, property) => (class, property, Vec::new()),
                nested => static_property_assignment_path(nested)?,
            };
            keys.push(*key);
            Some((class, property, keys))
        }
        _ => None,
    }
}

fn dynamic_static_property_assignment_path(expression: crate::ast::Expr) -> Option<(crate::ast::Expr, crate::ast::Expr, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::Index(array, key) => {
            let (class, property, mut keys) = match *array {
                crate::ast::Expr::DynamicStaticPropertyExpr(class, property) => (*class, *property, Vec::new()),
                nested => dynamic_static_property_assignment_path(nested)?,
            };
            keys.push(*key);
            Some((class, property, keys))
        }
        _ => None,
    }
}

fn property_append_path(expression: crate::ast::Expr) -> Option<(crate::ast::Expr, String, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::Property(receiver, property) => Some((*receiver, property, Vec::new())),
        crate::ast::Expr::Index(array, key) => { let (receiver, property, mut keys) = property_append_path(*array)?; keys.push(*key); Some((receiver, property, keys)) }
        _ => None,
    }
}

fn dynamic_property_append_path(expression: crate::ast::Expr) -> Option<(crate::ast::Expr, crate::ast::Expr, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::DynamicProperty(receiver, property) => Some((*receiver, *property, Vec::new())),
        crate::ast::Expr::Index(array, key) => { let (receiver, property, mut keys) = dynamic_property_append_path(*array)?; keys.push(*key); Some((receiver, property, keys)) }
        _ => None,
    }
}

fn expression_append_path(expression: crate::ast::Expr) -> (crate::ast::Expr, Vec<crate::ast::Expr>) {
    match expression {
        crate::ast::Expr::Index(array, key) => { let (base, mut keys) = expression_append_path(*array); keys.push(*key); (base, keys) }
        other => (other, Vec::new()),
    }
}

fn variable_append_path(expression: crate::ast::Expr) -> Option<(String, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::Variable(variable) => Some((variable, Vec::new())),
        crate::ast::Expr::Index(array, key) => { let (variable, mut keys) = variable_append_path(*array)?; keys.push(*key); Some((variable, keys)) }
        _ => None,
    }
}

fn static_property_append_path(expression: crate::ast::Expr) -> Option<(String, String, Vec<crate::ast::Expr>)> {
    match expression {
        crate::ast::Expr::StaticProperty(class, property) => Some((class, property, Vec::new())),
        crate::ast::Expr::Index(array, key) => { let (class, property, mut keys) = static_property_append_path(*array)?; keys.push(*key); Some((class, property, keys)) }
        _ => None,
    }
}

impl Parser<'_> {
    fn first_class_callable_marker(&mut self) -> bool {
        if self.peek() == &TokenKind::Ellipsis
            && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Rp)
        {
            self.take();
            self.take();
            true
        } else {
            false
        }
    }

    pub(super) fn call_argument(&mut self) -> Result<Expr, String> {
        if self.eat(&TokenKind::Ellipsis) {
            return Ok(Expr::UnpackArgument(Box::new(self.grouped_expression(1)?)));
        }
        if let TokenKind::Word(name) = self.peek().clone() {
            if self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Colon) {
                let offset = self.tokens[self.cursor].offset;
                let original_name = self.source[offset..].char_indices()
                    .take_while(|(_, character)| character.is_alphanumeric() || *character == '_')
                    .map(|(_, character)| character)
                    .collect::<String>();
                self.take();
                self.take();
                return Ok(Expr::NamedArgument(if original_name.is_empty() { name } else { original_name }, Box::new(self.grouped_expression(1)?)));
            }
        }
        self.grouped_expression(1)
    }

    fn interpolated_string(value: String) -> Result<Expr, String> {
        let bytes = value.as_bytes();
        let mut parts = Vec::new();
        let mut literal_start = 0usize;
        let mut index = 0usize;
        while index < bytes.len() {
            if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{')
                && bytes.get(index + 2).is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            {
                let start = index + 2;
                let mut end = start + 1;
                while bytes.get(end).is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_') { end += 1; }
                if bytes.get(end) == Some(&b'}') {
                    let name = &value[start..end];
                    parts.push(Expr::String(value[literal_start..index].to_string()));
                    parts.push(Expr::DeprecatedInterpolation(
                        Box::new(Expr::Variable(name.to_string())),
                        format!("Using ${{{name}}} in strings is deprecated, use {{${name}}} instead"),
                    ));
                    index = end + 1;
                    literal_start = index;
                    continue;
                }
            }
            if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
                if let Some(relative_end) = value[index + 2..].find('}') {
                    let end = index + 2 + relative_end;
                    let inner = value[index + 2..end].trim();
                    if inner.starts_with('$') {
                        let program = crate::parser::parse(&format!("<?php echo {inner};"))?;
                        let expression = match program.statements.into_iter().next() {
                            Some(crate::ast::Stmt::SourceLine(_, statement)) => match *statement { crate::ast::Stmt::Echo(mut expressions) if expressions.len() == 1 => expressions.remove(0), other => return Err(format!("Expected interpolation expression, got {other:?}")) },
                            Some(crate::ast::Stmt::Echo(mut expressions)) if expressions.len() == 1 => expressions.remove(0),
                            _ => return Err("Expected interpolation expression".into()),
                        };
                        parts.push(Expr::String(value[literal_start..index].to_string()));
                        parts.push(Expr::DeprecatedInterpolation(
                            Box::new(Expr::DynamicVariableExpr(Box::new(expression))),
                            "Using ${expr} (variable variables) in strings is deprecated, use {${expr}} instead".into(),
                        ));
                        index = end + 1;
                        literal_start = index;
                        continue;
                    }
                }
            }
            if bytes[index] == b'{' && bytes.get(index + 1) == Some(&b'$') {
                let mut depth = 1usize;
                let mut end = index + 1;
                let mut quote = None;
                while end < bytes.len() && depth > 0 {
                    let byte = bytes[end];
                    if let Some(q) = quote {
                        if byte == b'\\' { end += 2; continue; }
                        if byte == q { quote = None; }
                    } else if byte == b'\'' || byte == b'"' { quote = Some(byte); }
                    else if byte == b'{' { depth += 1; }
                    else if byte == b'}' { depth -= 1; }
                    end += 1;
                }
                if depth == 0 {
                    let inner = &value[index + 1..end - 1];
                    let program = crate::parser::parse(&format!("<?php echo {inner};"))?;
                    let expression = match program.statements.into_iter().next() {
                        Some(crate::ast::Stmt::SourceLine(_, statement)) => match *statement { crate::ast::Stmt::Echo(mut expressions) if expressions.len() == 1 => expressions.remove(0), other => return Err(format!("Expected interpolation expression, got {other:?}")) },
                        Some(crate::ast::Stmt::Echo(mut expressions)) if expressions.len() == 1 => expressions.remove(0),
                        _ => return Err("Expected interpolation expression".into()),
                    };
                    parts.push(Expr::String(value[literal_start..index].to_string()));
                    parts.push(expression);
                    index = end;
                    literal_start = index;
                    continue;
                }
            }
            if bytes[index] == b'\\' && bytes.get(index + 1) == Some(&b'$') {
                parts.push(Expr::String(value[literal_start..index].to_string()));
                parts.push(Expr::String("$".into()));
                index += 2;
                literal_start = index;
            } else if bytes[index] == b'$'
                && bytes.get(index + 1).is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            {
                let start = index + 1;
                index = start + 1;
                while bytes.get(index).is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_') { index += 1; }
                parts.push(Expr::String(value[literal_start..start - 1].to_string()));
                parts.push(Expr::Variable(value[start..index].to_string()));
                literal_start = index;
            } else {
                index += 1;
            }
        }
        if literal_start < value.len() || parts.is_empty() {
            parts.push(Expr::String(value[literal_start..].to_string()));
        }
        let mut parts = parts.into_iter();
        let Some(mut expression) = parts.next() else { return Ok(Expr::String(String::new())); };
        for part in parts {
            expression = Expr::Binary(crate::ast::BinaryOp::Concatenate, Box::new(expression), Box::new(part));
        }
        Ok(expression)
    }

    /// Parse an expression while respecting the requested operator precedence.
    pub(in crate::parser) fn expression(&mut self, min_precedence: u8) -> Result<Expr, String> {
        let token_offset = self.peek_offset();
        let mut left = match self.take() {
            TokenKind::Num(number, integer) => integer.map_or(Expr::Number(number), Expr::Integer),
            TokenKind::Str(value) => Expr::String(value),
            TokenKind::InterpolatedStr(value) => Self::interpolated_string(value)?,
            TokenKind::Var(name) if self.eat(&TokenKind::DoubleColon) => {
                let class = Box::new(Expr::Variable(name));
                if self.eat(&TokenKind::DynamicExprStart) {
                    let property = Box::new(self.expression(0)?);
                    self.expect(TokenKind::Rb, "Expected } after dynamic static property name")?;
                    Expr::DynamicStaticPropertyExpr(class, property)
                } else if self.eat(&TokenKind::Lb) {
                    let method = Box::new(self.grouped_expression(0)?);
                    self.expect(TokenKind::Rb, "Expected } after dynamic static method name")?;
                    if self.eat(&TokenKind::Lp) {
                        let mut arguments = Vec::new();
                        if !self.eat(&TokenKind::Rp) { loop { arguments.push(self.call_argument()?); if self.eat(&TokenKind::Rp) { break; } self.expect(TokenKind::Comma, "Expected , or ) in dynamic static method arguments")?; if self.eat(&TokenKind::Rp) { break; } } }
                        Expr::DynamicStaticCallExpr(class, method, arguments)
                    } else {
                        Expr::DynamicStaticPropertyExpr(class, method)
                    }
                } else if let TokenKind::Var(method) = self.peek().clone() {
                    self.take();
                    let method = Box::new(Expr::Variable(method));
                    if self.eat(&TokenKind::Lp) {
                        let mut arguments = Vec::new();
                        if !self.eat(&TokenKind::Rp) { loop { arguments.push(self.call_argument()?); if self.eat(&TokenKind::Rp) { break; } self.expect(TokenKind::Comma, "Expected , or ) in dynamic static method arguments")?; if self.eat(&TokenKind::Rp) { break; } } }
                        Expr::DynamicStaticCallExpr(class, method, arguments)
                    } else {
                        Expr::DynamicStaticPropertyExpr(class, method)
                    }
                } else if let TokenKind::DynamicVar(property) = self.peek().clone() {
                    self.take();
                    Expr::DynamicStaticPropertyExpr(class, Box::new(Expr::Variable(property)))
                } else {
                let method = match self.take() { TokenKind::Word(method) => method, token => return Err(self.error(&format!("Expected static method name, got {token:?}"))) };
                if method.eq_ignore_ascii_case("class") && self.peek() != &TokenKind::Lp {
                    Expr::DynamicClassName(class)
                } else if self.peek() != &TokenKind::Lp {
                    Expr::DynamicClassConstant(class, method)
                } else {
                self.expect(TokenKind::Lp, "Expected ( after dynamic static method")?;
                let mut arguments = Vec::new();
                if !self.eat(&TokenKind::Rp) {
                    loop { arguments.push(self.call_argument()?); if self.eat(&TokenKind::Rp) { break; } self.expect(TokenKind::Comma, "Expected , or ) in dynamic static method arguments")?; if self.eat(&TokenKind::Rp) { break; } }
                }
                Expr::DynamicStaticCall(class, method, arguments)
                }
                }
            }
            TokenKind::Var(name) => Expr::Variable(name),
            TokenKind::DynamicVar(name) => Expr::DynamicVariable(name),
            TokenKind::DynamicExprStart => {
                let name = self.expression(0)?;
                self.expect(TokenKind::Rb, "Expected } after dynamic variable expression")?;
                Expr::DynamicVariableExpr(Box::new(name))
            }
            TokenKind::Ls => {
                self.cursor -= 1;
                if self.short_list_assignment_ahead() {
                    self.take();
                    let targets = self.destructure_targets(TokenKind::Rs)?;
                    self.expect(TokenKind::Eq, "Expected = after list assignment targets")?;
                    Expr::ListAssignment { targets, value: Box::new(self.expression(1)?) }
                } else {
                    self.take();
                    self.array_literal(TokenKind::Rs)?
                }
            }
            TokenKind::Word(word) if word == "list" && self.eat(&TokenKind::Lp) => {
                let targets = self.destructure_targets(TokenKind::Rp)?;
                self.expect(TokenKind::Eq, "Expected = after list() assignment targets")?;
                Expr::ListAssignment { targets, value: Box::new(self.expression(1)?) }
            }
            TokenKind::Word(word) if matches!(word.as_str(), "include" | "include_once" | "require" | "require_once") => {
                Expr::Call(format!("\0php_include:{word}"), vec![self.expression(1)?])
            }
            TokenKind::Word(name) if name == "array" && self.eat(&TokenKind::Lp) => self.array_literal(TokenKind::Rp)?,
            TokenKind::Word(word) if word == "true" => Expr::Bool(true),
            TokenKind::Word(word) if word == "false" => Expr::Bool(false),
            TokenKind::Word(word) if word == "null" => Expr::Null,
            TokenKind::Word(word) if word.eq_ignore_ascii_case("match") => {
                self.expect(TokenKind::Lp, "Expected ( after match")?;
                let subject = self.grouped_expression(0)?;
                self.expect(TokenKind::Rp, "Expected ) after match subject")?;
                self.expect(TokenKind::Lb, "Expected { before match arms")?;
                let mut arms = Vec::new();
                let mut has_default = false;
                if self.eat(&TokenKind::Rb) { return Err(self.error("match expression must contain at least one arm")); }
                loop {
                    let conditions = if matches!(self.peek(), TokenKind::Word(name) if name.eq_ignore_ascii_case("default"))
                        && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Arrow)
                    {
                        self.take();
                        self.take();
                        if has_default { return Err(self.error("match expression cannot contain multiple default arms")); }
                        has_default = true;
                        Vec::new()
                    } else {
                        let mut conditions = vec![self.grouped_expression(0)?];
                        while self.eat(&TokenKind::Comma) {
                            if self.peek() == &TokenKind::Arrow { break; }
                            conditions.push(self.grouped_expression(0)?);
                        }
                        self.expect(TokenKind::Arrow, "Expected => after match conditions")?;
                        conditions
                    };
                    let value = self.grouped_expression(0)?;
                    arms.push(crate::ast::MatchArm { conditions, value });
                    if self.eat(&TokenKind::Rb) { break; }
                    self.expect(TokenKind::Comma, "Expected , or } after match arm")?;
                    if self.eat(&TokenKind::Rb) { break; }
                }
                Expr::Match { subject: Box::new(subject), arms }
            }
            TokenKind::Word(word) if word == "yield" => {
                if matches!(self.peek(), TokenKind::Word(next) if next.eq_ignore_ascii_case("from")) {
                    self.take();
                    Expr::YieldFrom(Box::new(self.grouped_expression(1)?))
                } else {
                    let value = if matches!(self.peek(), TokenKind::Semi | TokenKind::Comma | TokenKind::Rp) {
                        Expr::Null
                    } else {
                        self.grouped_expression(1)?
                    };
                    if self.eat(&TokenKind::Arrow) {
                        let yielded_value = self.grouped_expression(1)?;
                        Expr::Yield { key: Some(Box::new(value)), value: Box::new(yielded_value) }
                    } else {
                        Expr::Yield { key: None, value: Box::new(value) }
                    }
                }
            }
            TokenKind::Word(word) if word == "throw" => {
                Expr::Call("\0php_throw_expression".into(), vec![self.grouped_expression(1)?])
            }
            TokenKind::Word(word) if word == "empty" && self.eat(&TokenKind::Lp) => {
                let value = self.grouped_expression(0)?;
                self.expect(TokenKind::Rp, "Expected ) after empty() operand")?;
                Expr::Empty(Box::new(value))
            }
            // Keep isset property reads distinct so the VM can call __isset without invoking __get.
            TokenKind::Word(word) if word == "isset" && self.eat(&TokenKind::Lp) => {
                let mut values = Vec::new();
                loop {
                    let value = self.grouped_expression(0)?;
                    let value = split_isset_property_chain(value.clone())
                        .filter(|(_, properties)| properties.len() > 1 && properties.iter().any(|segment| !matches!(segment, crate::ast::IssetPropertySegment::ArrayOffset(_))))
                        .map(|(object, properties)| Expr::IssetPropertyChain(Box::new(object), properties))
                        .unwrap_or(value);
                    values.push(match value {
                        Expr::Property(object, property) => Expr::IssetProperty(object, property),
                        value @ Expr::Index(_, _) => {
                            match split_isset_property_path(value.clone()) {
                                Some((object, property, keys)) => Expr::IssetPropertyPath(Box::new(object), property, keys),
                                None => match split_isset_dynamic_property_path(value.clone()) {
                                    Some((object, property, keys)) => Expr::IssetDynamicPropertyPath(Box::new(object), Box::new(property), keys),
                                    None => value,
                                },
                            }
                        }
                        Expr::StaticProperty(class, property) => Expr::IssetStaticProperty(class, property),
                        Expr::DynamicProperty(object, property) => Expr::IssetDynamicProperty(object, property),
                        value @ (Expr::Variable(_) | Expr::DynamicVariable(_) | Expr::DynamicVariableExpr(_) | Expr::DynamicStaticPropertyExpr(_, _) | Expr::IssetProperty(_, _) | Expr::IssetPropertyChain(_, _) | Expr::IssetPropertyPath(_, _, _) | Expr::IssetStaticProperty(_, _) | Expr::IssetDynamicProperty(_, _) | Expr::IssetDynamicPropertyPath(_, _, _)) => value,
                        _ => return Err(self.error("isset() expects variables, array offsets, or object properties")),
                    });
                    if self.eat(&TokenKind::Rp) { break; }
                    self.expect(TokenKind::Comma, "Expected , or ) in isset()")?;
                    if self.eat(&TokenKind::Rp) { break; }
                }
                Expr::Isset(values)
            }
            // Clone binds tightly to its object expression, before surrounding operators.
            TokenKind::Word(word) if word == "clone" => Expr::Clone(Box::new(self.expression(15)?)),
            TokenKind::Word(word) if word == "static" && matches!(self.peek(), TokenKind::Word(next) if next == "function") => self.static_closure_expression()?,
            TokenKind::Word(word) if word == "static" && matches!(self.peek(), TokenKind::Word(next) if next == "fn") => {
                self.take();
                self.static_arrow_function_expression()?
            }
            TokenKind::Word(word) if word == "function" => self.closure_expression()?,
            TokenKind::Word(word) if word == "fn" => self.arrow_function_expression()?,
            TokenKind::Word(word) if word == "new" => {
                let source_line = self.source.get(..token_offset).unwrap_or_default().bytes().filter(|byte| *byte == b'\n').count() + 1;
                let anonymous_attributes = self.attributes()?;
                // PHP 8.2 permits `new readonly class (...) { ... }`.
                let anonymous_class_readonly = if matches!(self.peek(), TokenKind::Word(name) if name == "readonly") { self.take(); true } else { false };
                let anonymous_class = matches!(self.peek(), TokenKind::Word(name) if name == "class");
                let anonymous_class_line = if anonymous_class {
                    self.source.get(..self.peek_offset()).unwrap_or_default().bytes().filter(|byte| *byte == b'\n').count() + 1
                } else { 0 };
                if anonymous_class_readonly && !anonymous_class { return Err(self.error("The readonly modifier is only valid before an anonymous class")); }
                if !anonymous_attributes.is_empty() && !anonymous_class { return Err(self.error("Attributes after new are only valid on an anonymous class")); }
                let mut class_expression = match self.take() {
                    TokenKind::Word(name) if matches!(name.as_str(), "static" | "self" | "parent") => Ok(name),
                    TokenKind::Word(name) => Ok(self.qualify_name(name)),
                    TokenKind::Var(name) => Err(Expr::Variable(name)),
                    TokenKind::Lp => {
                        self.cursor -= 1;
                        Err(self.grouped_expression(0)?)
                    }
                    token => return Err(self.error(&format!("Expected class name after new, got {token:?}"))),
                };
                if class_expression.as_ref().is_ok_and(|name| matches!(name.as_str(), "static" | "self" | "parent")) && self.eat(&TokenKind::DoubleColon) {
                    let class_name = class_expression.as_ref().ok().unwrap().clone();
                    let property = match self.take() {
                        TokenKind::Var(name) => name,
                        TokenKind::Word(name) => name,
                        token => return Err(self.error(&format!("Expected static property after :: in dynamic new, got {token:?}"))),
                    };
                    let mut expression = Expr::StaticProperty(class_name, property);
                    while self.eat(&TokenKind::Ls) {
                        let key = self.grouped_expression(0)?;
                        self.expect(TokenKind::Rs, "Expected ] after static class array key")?;
                        expression = Expr::Index(Box::new(expression), Box::new(key));
                    }
                    class_expression = Err(expression);
                }
                if let Err(mut dynamic_class) = class_expression {
                    while self.eat(&TokenKind::ObjectArrow) {
                        match self.take() {
                            TokenKind::Word(property) => dynamic_class = Expr::Property(Box::new(dynamic_class), property),
                            token => return Err(self.error(&format!("Expected property in dynamic class name, got {token:?}"))),
                        }
                    }
                    class_expression = Err(dynamic_class);
                }
                let mut arguments = Vec::new();
                if self.eat(&TokenKind::Lp) && !self.eat(&TokenKind::Rp) {
                    loop {
                        arguments.push(self.call_argument()?);
                        if self.eat(&TokenKind::Rp) { break; }
                        self.expect(TokenKind::Comma, "Expected , or ) in constructor arguments")?;
                        if self.eat(&TokenKind::Rp) { break; }
                    }
                }
                if anonymous_class {
                    let class_name = self.anonymous_class_declaration(anonymous_attributes, anonymous_class_readonly, anonymous_class_line)?;
                    Expr::NewObject(class_name, arguments, source_line)
                } else {
                match class_expression { Ok(name) => Expr::NewObject(name, arguments, source_line), Err(class) => Expr::DynamicNewObject(Box::new(class), arguments, source_line) }
                }
            }
            TokenKind::Word(word) if word == "parent" && self.eat(&TokenKind::DoubleColon) => {
                let member_offset = self.peek_offset();
                let member = match self.take() {
                    TokenKind::Word(name) => name,
                    token => return Err(self.error(&format!("Expected parent method or constant name, got {token:?}"))),
                };
                if self.eat(&TokenKind::Lp) {
                    if self.first_class_callable_marker() {
                        Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::StaticMethod("parent".into(), member))
                    } else {
                        let mut arguments = Vec::new();
                        if !self.eat(&TokenKind::Rp) {
                            loop {
                                arguments.push(self.call_argument()?);
                                if self.eat(&TokenKind::Rp) { break; }
                                self.expect(TokenKind::Comma, "Expected , or ) in parent method arguments")?;
                                if self.eat(&TokenKind::Rp) { break; }
                            }
                        }
                        Expr::ParentMethodCall(member, arguments)
                    }
                } else {
                    Expr::ClassConstant("parent".into(), self.source_word(member_offset))
                }
            }
            TokenKind::Word(class_name) if self.eat(&TokenKind::DoubleColon) => {
                let class_name = if matches!(class_name.as_str(), "self" | "static" | "parent") { class_name } else { self.qualify_name(self.source_word(token_offset)) };
                if self.eat(&TokenKind::Lb) {
                    let method = self.grouped_expression(0)?;
                    self.expect(TokenKind::Rb, "Expected } after dynamic static method name")?;
                    let class = Box::new(Expr::String(class_name));
                    if self.eat(&TokenKind::Lp) {
                        let mut arguments = Vec::new();
                        if !self.eat(&TokenKind::Rp) {
                            loop {
                                arguments.push(self.call_argument()?);
                                if self.eat(&TokenKind::Rp) { break; }
                                self.expect(TokenKind::Comma, "Expected , or ) in dynamic static method arguments")?;
                                if self.eat(&TokenKind::Rp) { break; }
                            }
                        }
                        return Ok(Expr::DynamicStaticCallExpr(class, Box::new(method), arguments));
                    }
                    return Ok(Expr::DynamicStaticPropertyExpr(class, Box::new(method)));
                }
                let member_offset = self.peek_offset();
                let member = self.take();
                if member == TokenKind::DynamicExprStart {
                    let property = Box::new(self.expression(0)?);
                    self.expect(TokenKind::Rb, "Expected } after dynamic static property name")?;
                    Expr::DynamicStaticPropertyExpr(Box::new(Expr::String(class_name)), property)
                } else if let TokenKind::Var(property) = member {
                    Expr::StaticProperty(class_name, property)
                } else {
                let member_name = match member {
                    TokenKind::Word(name) => name,
                    token => return Err(self.error(&format!("Expected static method or property name, got {token:?}"))),
                };
                if member_name.eq_ignore_ascii_case("class") && self.peek() != &TokenKind::Lp {
                    Expr::String(class_name)
                } else if self.eat(&TokenKind::Lp) {
                    if self.first_class_callable_marker() {
                        Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::StaticMethod(class_name, member_name))
                    } else {
                        let mut arguments = Vec::new();
                        if !self.eat(&TokenKind::Rp) {
                            loop {
                                arguments.push(self.call_argument()?);
                                if self.eat(&TokenKind::Rp) { break; }
                                self.expect(TokenKind::Comma, "Expected , or ) in static method arguments")?;
                                if self.eat(&TokenKind::Rp) { break; }
                            }
                        }
                        Expr::StaticCall(class_name, member_name, arguments)
                    }
                } else {
                    Expr::ClassConstant(class_name, self.source_word(member_offset))
                }
                }
            }
            TokenKind::Word(name) if self.eat(&TokenKind::Lp) => {
                if self.first_class_callable_marker() {
                    Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::Function(self.qualify_function_name(name)))
                } else {
                    let mut arguments = Vec::new();
                    if !self.eat(&TokenKind::Rp) {
                        loop {
                            arguments.push(self.call_argument()?);
                            if self.eat(&TokenKind::Rp) { break; }
                            self.expect(TokenKind::Comma, "Expected , or ) in argument list")?;
                            if self.eat(&TokenKind::Rp) { break; }
                        }
                    }
                    Expr::Call(self.qualify_function_name(name), arguments)
                }
            },
            TokenKind::Word(name) if name == "__namespace__" => {
                Expr::String(self.namespace.clone().unwrap_or_default())
            }
            TokenKind::Word(name) if name == "__line__" => {
                let line = self.source.get(..token_offset).unwrap_or_default().bytes().filter(|byte| *byte == b'\n').count() + 1;
                Expr::Integer(line as i64)
            }
            TokenKind::Word(name) if matches!(name.as_str(), "__class__" | "__function__" | "__method__") => Expr::MagicConstant(name),
            TokenKind::Word(_) => Expr::Constant(self.qualify_constant_name(self.source_word(token_offset))),
            TokenKind::Minus => Expr::Negate(Box::new(self.expression(15)?)),
            TokenKind::Plus => Expr::Positive(Box::new(self.expression(15)?)),
            TokenKind::Not => Expr::Not(Box::new(self.expression(12)?)),
            TokenKind::ErrorSuppress => Expr::ErrorSuppress(Box::new(self.expression(15)?)),
            TokenKind::Tilde => Expr::BitwiseNot(Box::new(self.expression(15)?)),
            TokenKind::PlusPlus | TokenKind::MinusMinus => {
                let delta = if self.tokens[self.cursor - 1].kind == TokenKind::PlusPlus { 1 } else { -1 };
                let mut target = match self.take() {
                    TokenKind::Var(name) => Expr::Variable(name),
                    TokenKind::DynamicVar(name) => Expr::DynamicVariable(name),
                    TokenKind::DynamicExprStart => {
                        let name = self.expression(0)?;
                        self.expect(TokenKind::Rb, "Expected } after dynamic variable expression")?;
                        Expr::DynamicVariableExpr(Box::new(name))
                    }
                    TokenKind::Word(class) if self.eat(&TokenKind::DoubleColon) => {
                        let property = match self.take() {
                            TokenKind::Var(property) => property,
                            token => return Err(self.error(&format!("Expected static property after ::, got {token:?}"))),
                        };
                        let class = if matches!(class.as_str(), "self" | "static" | "parent") { class } else { self.qualify_name(class) };
                        Expr::StaticProperty(class, property)
                    }
                    token => return Err(self.error(&format!("Expected variable or static property after prefix increment/decrement, got {token:?}"))),
                };
                if matches!(&target, Expr::Variable(_) | Expr::DynamicVariable(_) | Expr::DynamicVariableExpr(_) | Expr::StaticProperty(_, _)) {
                loop {
                    if self.eat(&TokenKind::Ls) {
                        let key = self.grouped_expression(0)?;
                        self.expect(TokenKind::Rs, "Expected ] after array key")?;
                        target = Expr::Index(Box::new(target), Box::new(key));
                    } else if self.eat(&TokenKind::DoubleColon) {
                        let property = match self.take() {
                            TokenKind::Var(name) => Expr::Variable(name),
                            token => return Err(self.error(&format!("Expected dynamic static property after ::, got {token:?}"))),
                        };
                        target = Expr::DynamicStaticPropertyExpr(Box::new(target), Box::new(property));
                    } else if self.eat(&TokenKind::ObjectArrow) {
                        if self.eat(&TokenKind::Lb) {
                            let property = self.grouped_expression(0)?;
                            self.expect(TokenKind::Rb, "Expected } after dynamic property name")?;
                            target = Expr::DynamicProperty(Box::new(target), Box::new(property));
                        } else {
                            let property_offset = self.peek_offset();
                            match self.take() {
                                TokenKind::Word(_) => target = Expr::Property(Box::new(target), self.source_word(property_offset)),
                                TokenKind::Var(name) => target = Expr::DynamicProperty(Box::new(target), Box::new(Expr::Variable(name))),
                                token => return Err(self.error(&format!("Expected property name after ->, got {token:?}"))),
                            }
                        }
                    } else { break; }
                }
                }
                Expr::PreIncrement(increment_target(target).ok_or_else(|| self.error("Prefix increment requires a variable, array element, or property"))?, delta)
            }
            TokenKind::Lp => {
                let cast_type = match self.peek() {
                    TokenKind::Word(name) if matches!(name.as_str(), "int" | "integer") => Some(crate::ast::CastType::Integer),
                    TokenKind::Word(name) if matches!(name.as_str(), "float" | "double" | "real") => Some(crate::ast::CastType::Float),
                    TokenKind::Word(name) if matches!(name.as_str(), "string" | "binary") => Some(crate::ast::CastType::String),
                    TokenKind::Word(name) if matches!(name.as_str(), "bool" | "boolean") => Some(crate::ast::CastType::Bool),
                    TokenKind::Word(name) if name == "array" => Some(crate::ast::CastType::Array),
                    TokenKind::Word(name) if name == "object" => Some(crate::ast::CastType::Object),
                    _ => None,
                };
                if let Some(cast_type) = cast_type.filter(|_| self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Rp)) {
                    self.take();
                    self.take();
                    Expr::Cast(cast_type, Box::new(self.expression(15)?))
                } else {
                    let expression = self.grouped_expression(0)?;
                    self.expect(TokenKind::Rp, "Expected )")?;
                    expression
                }
            }
            token => return Err(self.error(&format!("Expected expression, got {token:?}"))),
        };

        // Consume postfix array/property/method access before binary operators.
        loop {
            if self.eat(&TokenKind::Ls) {
                if self.eat(&TokenKind::Rs) {
                    left = Expr::PropertyAppend(Box::new(left));
                } else {
                    let key = self.grouped_expression(0)?;
                    self.expect(TokenKind::Rs, "Expected ] after array key")?;
                    left = Expr::Index(Box::new(left), Box::new(key));
                }
            } else if self.eat(&TokenKind::DoubleColon) {
                let member = match self.take() {
                    TokenKind::Word(name) => name,
                    TokenKind::Var(name) => {
                        let method = Box::new(Expr::Variable(name));
                        if self.eat(&TokenKind::Lp) {
                            let mut arguments = Vec::new();
                            if !self.eat(&TokenKind::Rp) {
                                loop { arguments.push(self.call_argument()?); if self.eat(&TokenKind::Rp) { break; } self.expect(TokenKind::Comma, "Expected , or ) in dynamic static call")?; if self.eat(&TokenKind::Rp) { break; } }
                            }
                            left = Expr::DynamicStaticCallExpr(Box::new(left), method, arguments);
                        } else { left = Expr::DynamicStaticProperty(Box::new(left), match *method { Expr::Variable(name) => name, _ => unreachable!() }); }
                        continue;
                    },
                    token => return Err(self.error(&format!("Expected dynamic class member, got {token:?}"))),
                };
                if member.eq_ignore_ascii_case("class") && self.peek() != &TokenKind::Lp {
                    left = Expr::DynamicClassName(Box::new(left));
                } else if self.eat(&TokenKind::Lp) {
                    let mut arguments = Vec::new();
                    if !self.eat(&TokenKind::Rp) { loop { arguments.push(self.call_argument()?); if self.eat(&TokenKind::Rp) { break; } self.expect(TokenKind::Comma, "Expected , or ) in dynamic static arguments")?; if self.eat(&TokenKind::Rp) { break; } } }
                    left = Expr::DynamicStaticCall(Box::new(left), member, arguments);
                } else {
                    left = Expr::DynamicClassConstant(Box::new(left), member);
                }
            } else if self.eat(&TokenKind::ObjectArrow) || self.eat(&TokenKind::NullsafeArrow) {
                if self.eat(&TokenKind::Lb) {
                    let property = self.grouped_expression(0)?;
                    self.expect(TokenKind::Rb, "Expected } after dynamic property name")?;
                    left = Expr::DynamicProperty(Box::new(left), Box::new(property));
                    continue;
                }
                let member_offset = self.peek_offset();
                let member = match self.take() {
                    TokenKind::Word(name) => name,
                    TokenKind::Var(name) => {
                        if self.peek() != &TokenKind::Lp {
                            left = Expr::DynamicProperty(Box::new(left), Box::new(Expr::Variable(name)));
                            continue;
                        }
                        self.expect(TokenKind::Lp, "Expected ( after dynamic method name")?;
                        let mut arguments = Vec::new();
                        if !self.eat(&TokenKind::Rp) {
                            loop { arguments.push(self.call_argument()?); if self.eat(&TokenKind::Rp) { break; } self.expect(TokenKind::Comma, "Expected , or ) in dynamic method arguments")?; if self.eat(&TokenKind::Rp) { break; } }
                        }
                        left = Expr::DynamicMethodCall(Box::new(left), Box::new(Expr::Variable(name)), arguments);
                        continue;
                    }
                    token => return Err(self.error(&format!("Expected property or method name, got {token:?}"))),
                };
                if self.eat(&TokenKind::Lp) {
                    if self.first_class_callable_marker() {
                        left = Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::InstanceMethod(
                            Box::new(left), member,
                        ));
                        continue;
                    }
                    let mut arguments = Vec::new();
                    if !self.eat(&TokenKind::Rp) {
                        loop {
                            arguments.push(self.call_argument()?);
                            if self.eat(&TokenKind::Rp) { break; }
                            self.expect(TokenKind::Comma, "Expected , or ) in method arguments")?;
                            if self.eat(&TokenKind::Rp) { break; }
                        }
                    }
                    left = Expr::MethodCall(Box::new(left), member, arguments);
                } else {
                    left = Expr::Property(Box::new(left), self.source_word(member_offset));
                }
            } else if self.eat(&TokenKind::PlusPlus) {
                let target = increment_target(left).ok_or_else(|| self.error("Post-increment requires a variable, array element, or property"))?;
                left = Expr::PostIncrement(target, 1);
            } else if self.eat(&TokenKind::MinusMinus) {
                let target = increment_target(left).ok_or_else(|| self.error("Post-decrement requires a variable, array element, or property"))?;
                left = Expr::PostIncrement(target, -1);
            } else if self.eat(&TokenKind::Lp) {
                if self.first_class_callable_marker() {
                    left = Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::Value(Box::new(left)));
                    continue;
                }
                let mut arguments = Vec::new();
                if !self.eat(&TokenKind::Rp) {
                    loop {
                        arguments.push(self.call_argument()?);
                        if self.eat(&TokenKind::Rp) { break; }
                        self.expect(TokenKind::Comma, "Expected , or ) in closure call")?;
                        if self.eat(&TokenKind::Rp) { break; }
                    }
                }
                left = Expr::Invoke(Box::new(left), arguments);
            } else { break; }
        }

        // Precedence climbing makes operators left-associative without grammar duplication.
        loop {
            if matches!(self.peek(), TokenKind::Word(word) if word == "instanceof") {
                let precedence = 13;
                if precedence < min_precedence { break; }
                self.take();
                let class_expression = match self.take() {
                    TokenKind::Word(name) => Expr::String(self.qualify_type(name)),
                    TokenKind::Var(_) => {
                        self.cursor -= 1;
                        self.expression(precedence + 1)?
                    }
                    TokenKind::Lp => { self.cursor -= 1; self.grouped_expression(0)? },
                    token => return Err(self.error(&format!("Expected class name after instanceof, got {token:?}"))),
                };
                left = Expr::InstanceOf(Box::new(left), Box::new(class_expression));
                continue;
            }
            let (precedence, operation) = match self.peek() {
                TokenKind::OrOr => (2, BinaryOp::LogicalOr),
                TokenKind::AndAnd => (3, BinaryOp::LogicalAnd),
                TokenKind::Pipe => (4, BinaryOp::BitOr),
                TokenKind::Caret => (5, BinaryOp::BitXor),
                TokenKind::Ampersand => (6, BinaryOp::BitAnd),
                TokenKind::EqEq => (7, BinaryOp::Equal),
                TokenKind::NotEq => (7, BinaryOp::NotEqual),
                TokenKind::StrictEq => (7, BinaryOp::StrictEqual),
                TokenKind::StrictNotEq => (7, BinaryOp::StrictNotEqual),
                TokenKind::Lt => (7, BinaryOp::Less),
                TokenKind::Le => (7, BinaryOp::LessEqual),
                TokenKind::Gt => (7, BinaryOp::Greater),
                TokenKind::Ge => (7, BinaryOp::GreaterEqual),
                TokenKind::Spaceship => (7, BinaryOp::Spaceship),
                TokenKind::Dot => (8, BinaryOp::Concatenate),
                TokenKind::ShiftLeft => (9, BinaryOp::ShiftLeft),
                TokenKind::ShiftRight => (9, BinaryOp::ShiftRight),
                TokenKind::Plus => (10, BinaryOp::Add),
                TokenKind::Minus => (10, BinaryOp::Subtract),
                TokenKind::Star => (11, BinaryOp::Multiply),
                TokenKind::Power => (16, BinaryOp::Power),
                TokenKind::Slash => (11, BinaryOp::Divide),
                TokenKind::Percent => (11, BinaryOp::Modulo),
                _ => break,
            };
            if precedence < min_precedence { break; }
            self.take();
            let right = self.expression(if operation == BinaryOp::Power { precedence } else { precedence + 1 })?;
            left = Expr::Binary(operation, Box::new(left), Box::new(right));
        }
        // Null coalescing is right associative and short-circuits its fallback.
        if min_precedence <= 1 && self.eat(&TokenKind::Coalesce) {
            let fallback = self.expression(1)?;
            left = Expr::Coalesce(Box::new(left), Box::new(fallback));
        }
        // Ternary binds below logical operators and its else branch associates to the right.
        if min_precedence <= 1 && self.eat(&TokenKind::Question) {
            if self.eat(&TokenKind::Colon) {
                left = Expr::ShorthandTernary { condition: Box::new(left), else_value: Box::new(self.grouped_expression(1)?) };
            } else {
                let then_value = self.grouped_expression(0)?;
                self.expect(TokenKind::Colon, "Expected : in conditional expression")?;
                let else_value = self.grouped_expression(1)?;
                left = Expr::Ternary {
                    condition: Box::new(left),
                    then_value: Box::new(then_value),
                    else_value: Box::new(else_value),
                };
            }
        }
        // Assignment has low precedence and associates to the right.
        if min_precedence <= 1 || matches!(&left, Expr::Variable(_) | Expr::Index(_, _) | Expr::Property(_, _) | Expr::StaticProperty(_, _)) {
            let operation = match self.peek() {
                TokenKind::Eq => Some(None),
                TokenKind::PlusEq => Some(Some(BinaryOp::Add)),
                TokenKind::MinusEq => Some(Some(BinaryOp::Subtract)),
                TokenKind::StarEq => Some(Some(BinaryOp::Multiply)),
                TokenKind::PowerEq => Some(Some(BinaryOp::Power)),
                TokenKind::SlashEq => Some(Some(BinaryOp::Divide)),
                TokenKind::PercentEq => Some(Some(BinaryOp::Modulo)),
                TokenKind::DotEq => Some(Some(BinaryOp::Concatenate)),
                TokenKind::AmpersandEq => Some(Some(BinaryOp::BitAnd)),
                TokenKind::CaretEq => Some(Some(BinaryOp::BitXor)),
                TokenKind::PipeEq => Some(Some(BinaryOp::BitOr)),
                TokenKind::ShiftLeftEq => Some(Some(BinaryOp::ShiftLeft)),
                TokenKind::ShiftRightEq => Some(Some(BinaryOp::ShiftRight)),
                TokenKind::CoalesceEq => Some(Some(BinaryOp::Coalesce)),
                _ => None,
            };
            if let Some(operation) = operation {
                if operation.is_none() && self.tokens.get(self.cursor + 1).is_some_and(|token| token.kind == TokenKind::Ampersand) {
                    let Expr::Variable(variable) = left else {
                        return Err(self.error("Reference assignment expression requires a variable target"));
                    };
                    self.take();
                    self.take();
                    let value = self.expression(1)?;
                    left = Expr::ReferenceAssignment { variable, value: Box::new(value) };
                    return Ok(left);
                }
                if let Some((class, property, keys)) = dynamic_static_property_assignment_path(left.clone()) {
                    self.take();
                    let previous_floor = self.word_operator_floor;
                    self.word_operator_floor = 4;
                    let value = self.expression(1);
                    self.word_operator_floor = previous_floor;
                    left = Expr::DynamicStaticPropertyArrayAssignment { class: Box::new(class), property: Box::new(property), keys, operation, value: Box::new(value?) };
                    return Ok(left);
                }
                let (property_target, static_property_target, dynamic_target, assignment_target, property_array_target, property_append_target, static_array_target, dynamic_property_target, array_append_target, static_append_target, dynamic_append_target, expression_append_target) = match left {
                    Expr::Property(receiver, property) => (Some((receiver, property)), None, None, None, None, None, None, None, None, None, None, None),
                    Expr::StaticProperty(class, property) => (None, Some((class, property)), None, None, None, None, None, None, None, None, None, None),
                    Expr::DynamicVariable(variable) => (None, None, Some(Box::new(Expr::Variable(variable))), None, None, None, None, None, None, None, None, None),
                    Expr::DynamicVariableExpr(name) => (None, None, Some(name), None, None, None, None, None, None, None, None, None),
                    Expr::Variable(variable) => (None, None, None, Some((variable, Vec::new())), None, None, None, None, None, None, None, None),
                    Expr::DynamicProperty(receiver, property) => (None, None, None, None, None, None, None, Some((receiver, property)), None, None, None, None),
                    Expr::PropertyAppend(target) => {
                        if let Some((receiver, property, keys)) = property_append_path((*target).clone()) {
                            (None, None, None, None, None, Some((receiver, property, keys)), None, None, None, None, None, None)
                        } else if let Some((receiver, property, keys)) = dynamic_property_append_path((*target).clone()) {
                            (None, None, None, None, None, None, None, None, None, None, Some((receiver, property, keys)), None)
                        } else if let Some((variable, keys)) = variable_append_path((*target).clone()) {
                            (None, None, None, None, None, None, None, None, Some((variable, keys)), None, None, None)
                        } else if let Some((class, property, keys)) = static_property_append_path((*target).clone()) {
                            (None, None, None, None, None, None, None, None, None, Some((class, property, keys)), None, None)
                        } else {
                            let (array, keys) = expression_append_path((*target).clone());
                            (None, None, None, None, None, None, None, None, None, None, None, Some((array, keys)))
                        }
                    }
                    target => {
                        if let Some((receiver, property, keys)) = property_assignment_path(target.clone()) {
                            (None, None, None, None, Some((Box::new(receiver), property, keys)), None, None, None, None, None, None, None)
                        } else if let Some((class, property, keys)) = static_property_assignment_path(target.clone()) {
                            (None, None, None, None, None, None, Some((class, property, keys)), None, None, None, None, None)
                        } else {
                            (None, None, None, assignment_array_path(target), None, None, None, None, None, None, None, None)
                        }
                    },
                };
                if property_target.is_none() && static_property_target.is_none() && dynamic_target.is_none() && assignment_target.is_none() && property_array_target.is_none() && property_append_target.is_none() && static_array_target.is_none() && dynamic_property_target.is_none() && array_append_target.is_none() && static_append_target.is_none() && dynamic_append_target.is_none() && expression_append_target.is_none() {
                    return Err(self.error("Assignment expression requires a variable, array offset, or property"));
                }
                self.take();
                let previous_floor = self.word_operator_floor;
                self.word_operator_floor = 4;
                let value = self.expression(1);
                self.word_operator_floor = previous_floor;
                let value = value?;
                left = if let Some((class, property, keys)) = static_append_target {
                    if operation.is_some() { return Err(self.error("Compound assignment to a static property append target is not supported")); }
                    Expr::StaticPropertyAppendAssignment { class, property, keys, value: Box::new(value) }
                } else if let Some((variable, keys)) = array_append_target {
                    if operation.is_some() { return Err(self.error("Compound assignment to an array append target is not supported")); }
                    Expr::ArrayAppendAssignment { variable, keys, value: Box::new(value) }
                } else if let Some((receiver, property, keys)) = dynamic_append_target {
                    if operation.is_some() { return Err(self.error("Compound assignment to a dynamic property append target is not supported")); }
                    Expr::DynamicPropertyAppendAssignment { receiver: Box::new(receiver), property: Box::new(property), keys, value: Box::new(value) }
                } else if let Some((array, keys)) = expression_append_target {
                    if operation.is_some() { return Err(self.error("Compound assignment to an array expression append target is not supported")); }
                    Expr::ArrayExpressionAppendAssignment { array: Box::new(array), keys, value: Box::new(value) }
                } else if let Some((receiver, property)) = dynamic_property_target {
                    if let Some(operation) = operation {
                        Expr::DynamicPropertyCompoundAssignment { receiver, property, operation, value: Box::new(value) }
                    } else {
                        Expr::DynamicPropertyAssignment { receiver, property, value: Box::new(value) }
                    }
                } else if let Some((class, property, keys)) = static_array_target {
                    Expr::StaticPropertyArrayAssignment { class, property, keys, operation, value: Box::new(value) }
                } else if let Some((receiver, property, keys)) = property_append_target {
                    Expr::PropertyAppendAssignment { receiver: Box::new(receiver), property, keys, value: Box::new(value) }
                } else if let Some((receiver, property, keys)) = property_array_target {
                    Expr::PropertyArrayAssignment { receiver, property, keys, operation, value: Box::new(value) }
                } else if let Some((receiver, property)) = property_target {
                    Expr::PropertyAssignment { receiver, property, operation, value: Box::new(value) }
                } else if let Some((class, property)) = static_property_target {
                    Expr::StaticPropertyAssignment { class, property, operation, value: Box::new(value) }
                } else if let Some(name) = dynamic_target {
                    Expr::DynamicVariableAssignment { name, operation, value: Box::new(value) }
                } else {
                    let (variable, keys) = assignment_target.expect("validated assignment target");
                    if keys.is_empty() {
                        Expr::Assignment { variable, operation, value: Box::new(value) }
                    } else {
                        Expr::ArrayAssignment { variable, keys, operation, value: Box::new(value) }
                    }
                };
            }

            // PHP's word operators bind below assignment and have three distinct levels:
            // `and` > `xor` > `or`. Keep the floor while parsing a right operand so
            // precedence climbing remains correct without changing punctuation operators.
            loop {
                let (rank, operation) = match self.peek() {
                    TokenKind::Word(word) if word == "or" => (1, BinaryOp::LogicalOr),
                    TokenKind::Word(word) if word == "xor" => (2, BinaryOp::LogicalXor),
                    TokenKind::Word(word) if word == "and" => (3, BinaryOp::LogicalAnd),
                    _ => break,
                };
                if rank < self.word_operator_floor { break; }
                self.take();
                let previous_floor = self.word_operator_floor;
                self.word_operator_floor = rank + 1;
                let right = self.expression(1);
                self.word_operator_floor = previous_floor;
                left = Expr::Binary(operation, Box::new(left), Box::new(right?));
            }
        }
        Ok(left)
    }

}

fn split_isset_property_path(expression: Expr) -> Option<(Expr, String, Vec<Expr>)> {
    fn split(expression: Expr, keys: &mut Vec<Expr>) -> Option<(Expr, String)> {
        match expression {
            Expr::Index(array, key) => {
                let result = split(*array, keys)?;
                keys.push(*key);
                Some(result)
            }
            Expr::Property(object, property) => Some((*object, property)),
            _ => None,
        }
    }
    let mut keys = Vec::new();
    let (object, property) = split(expression, &mut keys)?;
    Some((object, property, keys))
}

fn split_isset_property_chain(expression: Expr) -> Option<(Expr, Vec<crate::ast::IssetPropertySegment>)> {
    fn split(expression: Expr, properties: &mut Vec<crate::ast::IssetPropertySegment>) -> Option<Expr> {
        match expression {
            Expr::Index(array, key) => {
                let root = split(*array, properties)?;
                properties.push(crate::ast::IssetPropertySegment::ArrayOffset(vec![*key]));
                Some(root)
            }
            Expr::Property(object, property) => {
                let root = split(*object, properties)?;
                properties.push(crate::ast::IssetPropertySegment::Fixed(property));
                Some(root)
            }
            Expr::DynamicProperty(object, property) => {
                let root = split(*object, properties)?;
                properties.push(crate::ast::IssetPropertySegment::Dynamic(property));
                Some(root)
            }
            root => Some(root),
        }
    }
    let mut properties = Vec::new();
    let root = split(expression, &mut properties)?;
    Some((root, properties))
}

fn split_isset_dynamic_property_path(expression: Expr) -> Option<(Expr, Expr, Vec<Expr>)> {
    fn split(expression: Expr, keys: &mut Vec<Expr>) -> Option<(Expr, Expr)> {
        match expression {
            Expr::Index(array, key) => {
                let result = split(*array, keys)?;
                keys.push(*key);
                Some(result)
            }
            Expr::DynamicProperty(object, property) => Some((*object, *property)),
            _ => None,
        }
    }
    let mut keys = Vec::new();
    let (object, property) = split(expression, &mut keys)?;
    Some((object, property, keys))
}
