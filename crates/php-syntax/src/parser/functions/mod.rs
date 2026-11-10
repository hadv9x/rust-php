//! Parsing for function and method declarations and their signatures.

use super::Parser;
use crate::{
    ast::{Expr, FunctionDecl, PropertyDecl, Stmt, Visibility},
    lexer::token::TokenKind,
};

impl Parser<'_> {
    /// Parse an arrow function and infer its implicit by-value captures.
    pub(in crate::parser) fn arrow_function_expression(&mut self) -> Result<Expr, String> {
        self.arrow_function_expression_with_static(false)
    }

    /// Parse `static fn (...) => ...`, which captures lexical values but has no `$this` binding.
    pub(in crate::parser) fn static_arrow_function_expression(&mut self) -> Result<Expr, String> {
        self.arrow_function_expression_with_static(true)
    }

    fn arrow_function_expression_with_static(&mut self, is_static: bool) -> Result<Expr, String> {
        self.expect(TokenKind::Lp, "Expected ( after fn")?;
        let mut parameters = Vec::new();
        let mut parameter_attributes = Vec::new();
        let mut parameter_types = Vec::new();
        let mut parameter_by_reference = Vec::new();
        let mut parameter_variadic = Vec::new();
        let mut parameter_defaults = Vec::new();
        let mut optional_parameter_seen = false;
        if !self.eat(&TokenKind::Rp) {
            loop {
                let attributes = self.attributes()?;
                let by_reference_before_type = self.eat(&TokenKind::Ampersand);
                let variadic_before_type = self.eat(&TokenKind::Ellipsis);
                let nullable = self.eat(&TokenKind::Question);
                let has_type = (matches!(self.peek(), TokenKind::Word(_))
                    && matches!(
                        self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                        Some(
                            TokenKind::Var(_)
                                | TokenKind::Ampersand
                                | TokenKind::Ellipsis
                                | TokenKind::Pipe
                        )
                    ))
                    || matches!(self.peek(), TokenKind::Lp);
                let type_name = if has_type {
                    Some(self.parse_union_type(nullable)?)
                } else if nullable {
                    return Err(self.error("Expected type after ? in arrow-function parameter"));
                } else {
                    None
                };
                let reference_before_variadic = self.peek() == &TokenKind::Ampersand
                    && self
                        .tokens
                        .get(self.cursor + 1)
                        .is_some_and(|token| token.kind == TokenKind::Ellipsis);
                if reference_before_variadic {
                    self.take();
                }
                let is_variadic = variadic_before_type || self.eat(&TokenKind::Ellipsis);
                let is_by_reference = by_reference_before_type
                    || reference_before_variadic
                    || self.eat(&TokenKind::Ampersand);
                if parameter_variadic.iter().any(|variadic| *variadic) {
                    return Err(self.error("A variadic parameter must be last"));
                }
                let name = match self.take() {
                    TokenKind::Var(name) => name,
                    token => {
                        return Err(self
                            .error(&format!("Expected arrow-function parameter, got {token:?}")))
                    }
                };
                let default = if self.eat(&TokenKind::Eq) {
                    if is_variadic {
                        return Err(self.error("A variadic parameter cannot have a default value"));
                    }
                    optional_parameter_seen = true;
                    Some(self.expression(0)?)
                } else {
                    if optional_parameter_seen && !is_variadic {
                        return Err(
                            self.error("Required parameter cannot follow an optional parameter")
                        );
                    }
                    None
                };
                parameters.push(name);
                parameter_attributes.push(attributes);
                parameter_types.push(type_name);
                parameter_by_reference.push(is_by_reference);
                parameter_variadic.push(is_variadic);
                parameter_defaults.push(default);
                if self.eat(&TokenKind::Rp) {
                    break;
                }
                self.expect(
                    TokenKind::Comma,
                    "Expected , or ) in arrow-function parameters",
                )?;
                if self.eat(&TokenKind::Rp) {
                    break;
                }
            }
        }
        let return_type = self.parse_return_type()?;
        self.expect(
            TokenKind::Arrow,
            "Expected => after arrow-function signature",
        )?;
        let expression = self.expression(0)?;
        let mut captures = Vec::new();
        collect_arrow_captures(&expression, &parameters, &mut captures);
        if is_static && captures.iter().any(|(name, _)| name == "this") {
            return Err(self.error("Static arrow functions cannot capture $this"));
        }
        let declaration = FunctionDecl {
            name: "{arrow}".into(),
            attributes: Vec::new(),
            is_static,
            is_abstract: false,
            is_final: false,
            returns_by_reference: false,
            visibility: Visibility::Public,
            parameters,
            parameter_types,
            parameter_by_reference,
            parameter_attributes,
            parameter_variadic,
            parameter_defaults,
            return_type,
            promoted_properties: Vec::new(),
            body: vec![Stmt::Return(Some(expression))],
        };
        Ok(Expr::Closure {
            declaration: Box::new(declaration),
            captures,
        })
    }

    /// Parse an anonymous function with typed parameters and explicit captures.
    pub(in crate::parser) fn closure_expression(&mut self) -> Result<Expr, String> {
        self.closure_expression_with_static(false)
    }

    /// Parse `static function (...) use (...) { ... }`.
    pub(in crate::parser) fn static_closure_expression(&mut self) -> Result<Expr, String> {
        self.expect(
            TokenKind::Word("function".into()),
            "Expected function after static",
        )?;
        self.closure_expression_with_static(true)
    }

    fn closure_expression_with_static(&mut self, is_static: bool) -> Result<Expr, String> {
        let returns_by_reference = self.eat(&TokenKind::Ampersand);
        self.expect(TokenKind::Lp, "Expected ( after function")?;
        let mut parameters = Vec::new();
        let mut parameter_attributes = Vec::new();
        let mut parameter_types = Vec::new();
        let mut parameter_by_reference = Vec::new();
        let mut parameter_variadic = Vec::new();
        let mut parameter_defaults = Vec::new();
        if !self.eat(&TokenKind::Rp) {
            loop {
                let attributes = self.attributes()?;
                let by_reference_before_type = self.eat(&TokenKind::Ampersand);
                let variadic_before_type = self.eat(&TokenKind::Ellipsis);
                let nullable = self.eat(&TokenKind::Question);
                let has_type = (matches!(self.peek(), TokenKind::Word(_))
                    && matches!(
                        self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                        Some(
                            TokenKind::Var(_)
                                | TokenKind::Ampersand
                                | TokenKind::Ellipsis
                                | TokenKind::Pipe
                        )
                    ))
                    || matches!(self.peek(), TokenKind::Lp);
                let type_name = if has_type {
                    Some(self.parse_union_type(nullable)?)
                } else if nullable {
                    return Err(self.error("Expected type after ? in closure parameter"));
                } else {
                    None
                };
                let reference_before_variadic = self.peek() == &TokenKind::Ampersand
                    && self
                        .tokens
                        .get(self.cursor + 1)
                        .is_some_and(|token| token.kind == TokenKind::Ellipsis);
                if reference_before_variadic {
                    self.take();
                }
                let is_variadic = variadic_before_type || self.eat(&TokenKind::Ellipsis);
                let is_by_reference = by_reference_before_type
                    || reference_before_variadic
                    || self.eat(&TokenKind::Ampersand);
                if parameter_variadic.iter().any(|variadic| *variadic) {
                    return Err(self.error("A variadic parameter must be last"));
                }
                let name = match self.take() {
                    TokenKind::Var(name) => name,
                    token => {
                        return Err(
                            self.error(&format!("Expected closure parameter, got {token:?}"))
                        )
                    }
                };
                let default = if self.eat(&TokenKind::Eq) {
                    if is_variadic {
                        return Err(self.error("A variadic parameter cannot have a default value"));
                    }
                    Some(self.expression(0)?)
                } else {
                    None
                };
                parameters.push(name);
                parameter_attributes.push(attributes);
                parameter_types.push(type_name);
                parameter_by_reference.push(is_by_reference);
                parameter_variadic.push(is_variadic);
                parameter_defaults.push(default);
                if self.eat(&TokenKind::Rp) {
                    break;
                }
                self.expect(TokenKind::Comma, "Expected , or ) in closure parameters")?;
                if self.eat(&TokenKind::Rp) {
                    break;
                }
            }
        }

        let mut captures = Vec::new();
        if matches!(self.peek(), TokenKind::Word(word) if word == "use") {
            self.take();
            self.expect(TokenKind::Lp, "Expected ( after closure use")?;
            if !self.eat(&TokenKind::Rp) {
                loop {
                    let by_reference = self.eat(&TokenKind::Ampersand);
                    match self.take() {
                        TokenKind::Var(name) => captures.push((name, by_reference)),
                        token => {
                            return Err(
                                self.error(&format!("Expected captured variable, got {token:?}"))
                            )
                        }
                    }
                    if self.eat(&TokenKind::Rp) {
                        break;
                    }
                    self.expect(TokenKind::Comma, "Expected , or ) in closure capture list")?;
                    if self.eat(&TokenKind::Rp) {
                        break;
                    }
                }
            }
        }
        if is_static && captures.iter().any(|(name, _)| name == "this") {
            return Err(self.error("Static closures cannot capture $this"));
        }

        let return_type = self.parse_return_type()?;
        let body_start = self.cursor;
        let body = self.block()?;
        if is_static
            && self.tokens[body_start..self.cursor]
                .iter()
                .any(|token| token.kind == TokenKind::Var("this".into()))
        {
            return Err(self.error("Static closures cannot use $this"));
        }
        let declaration = FunctionDecl {
            name: "{closure}".into(),
            attributes: Vec::new(),
            is_static,
            is_abstract: false,
            is_final: false,
            returns_by_reference,
            visibility: Visibility::Public,
            parameter_by_reference,
            parameter_variadic,
            parameters,
            parameter_attributes,
            parameter_types,
            parameter_defaults,
            return_type,
            promoted_properties: Vec::new(),
            body,
        };
        Ok(Expr::Closure {
            declaration: Box::new(declaration),
            captures,
        })
    }

    /// Parse a function or method, including typed and promoted parameters.
    pub(in crate::parser) fn function_declaration(
        &mut self,
        is_static: bool,
        is_abstract: bool,
        is_final: bool,
        visibility: Visibility,
    ) -> Result<FunctionDecl, String> {
        self.take(); // Consume the function keyword.
        let returns_by_reference = self.eat(&TokenKind::Ampersand);
        let name = match self.take() {
            TokenKind::Word(_) => self.previous_word_spelling(""),
            token => return Err(self.error(&format!("Expected function name, got {token:?}"))),
        };
        self.expect(TokenKind::Lp, "Expected ( after function name")?;
        let mut parameters = Vec::new();
        let mut parameter_attributes = Vec::new();
        let mut parameter_types = Vec::new();
        let mut parameter_by_reference = Vec::new();
        let mut parameter_variadic = Vec::new();
        let mut parameter_defaults = Vec::new();
        let mut promoted_properties = Vec::new();
        let mut optional_parameter_seen = false;
        if !self.eat(&TokenKind::Rp) {
            loop {
                let attributes = self.attributes()?;
                let mut visibility = Visibility::Public;
                let mut has_visibility = false;
                let mut is_readonly = false;
                loop {
                    match self.peek() {
                        TokenKind::Word(word)
                            if matches!(word.as_str(), "public" | "protected" | "private") =>
                        {
                            visibility = match word.as_str() {
                                "public" => Visibility::Public,
                                "protected" => Visibility::Protected,
                                _ => Visibility::Private,
                            };
                            has_visibility = true;
                            self.take();
                        }
                        TokenKind::Word(word) if word == "readonly" => {
                            is_readonly = true;
                            self.take();
                        }
                        _ => break,
                    }
                }
                let by_reference_before_type = self.eat(&TokenKind::Ampersand);
                let variadic_before_type = self.eat(&TokenKind::Ellipsis);
                let nullable = self.eat(&TokenKind::Question);
                let has_type = (matches!(self.peek(), TokenKind::Word(_))
                    && matches!(
                        self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                        Some(
                            TokenKind::Var(_)
                                | TokenKind::Ampersand
                                | TokenKind::Ellipsis
                                | TokenKind::Pipe
                        )
                    ))
                    || matches!(self.peek(), TokenKind::Lp);
                let type_name = if has_type {
                    Some(self.parse_union_type(nullable)?)
                } else if nullable {
                    return Err(self.error("Expected type after ? in parameter"));
                } else {
                    None
                };
                let reference_before_variadic = self.peek() == &TokenKind::Ampersand
                    && self
                        .tokens
                        .get(self.cursor + 1)
                        .is_some_and(|token| token.kind == TokenKind::Ellipsis);
                if reference_before_variadic {
                    self.take();
                }
                let is_variadic = variadic_before_type || self.eat(&TokenKind::Ellipsis);
                let is_by_reference = by_reference_before_type
                    || reference_before_variadic
                    || self.eat(&TokenKind::Ampersand);
                if parameter_variadic.iter().any(|variadic| *variadic) {
                    return Err(self.error("A variadic parameter must be last"));
                }
                let name = match self.take() {
                    TokenKind::Var(name) => name,
                    token => {
                        return Err(
                            self.error(&format!("Expected parameter variable, got {token:?}"))
                        )
                    }
                };
                let default = if self.eat(&TokenKind::Eq) {
                    if is_variadic {
                        return Err(self.error("A variadic parameter cannot have a default value"));
                    }
                    optional_parameter_seen = true;
                    Some(self.expression(0)?)
                } else {
                    if optional_parameter_seen && !is_variadic {
                        return Err(
                            self.error("Required parameter cannot follow an optional parameter")
                        );
                    }
                    None
                };
                parameter_types.push(type_name.clone());
                parameter_by_reference.push(is_by_reference);
                parameter_variadic.push(is_variadic);
                parameter_defaults.push(default);
                parameter_attributes.push(attributes);
                if has_visibility || is_readonly {
                    if is_variadic {
                        return Err(self.error("A variadic parameter cannot promote a property"));
                    }
                    if !has_visibility {
                        return Err(self.error("Promoted property must declare visibility"));
                    }
                    promoted_properties.push(PropertyDecl {
                        name: name.clone(),
                        attributes: Vec::new(),
                        default: None,
                        is_static: false,
                        is_readonly,
                        visibility,
                        set_visibility: None,
                        type_name,
                        get_hook: None,
                        set_hook: None,
                    });
                }
                parameters.push(name);
                if self.eat(&TokenKind::Rp) {
                    break;
                }
                self.expect(TokenKind::Comma, "Expected , or ) in parameter list")?;
                if self.eat(&TokenKind::Rp) {
                    break;
                }
            }
        }
        let return_type = self.parse_return_type()?;
        let body = if is_abstract {
            self.expect(
                TokenKind::Semi,
                "Expected ; after abstract method declaration",
            )?;
            Vec::new()
        } else {
            self.block()?
        };
        Ok(FunctionDecl {
            name,
            attributes: Vec::new(),
            is_static,
            is_abstract,
            is_final,
            returns_by_reference,
            visibility,
            parameters,
            parameter_attributes,
            parameter_types,
            parameter_by_reference,
            parameter_variadic,
            parameter_defaults,
            return_type,
            promoted_properties,
            body,
        })
    }

    /// Parse an optional scalar, class, or nullable return type after `:`.
    fn parse_return_type(&mut self) -> Result<Option<String>, String> {
        if !self.eat(&TokenKind::Colon) {
            return Ok(None);
        }
        let nullable = self.eat(&TokenKind::Question);
        Ok(Some(self.parse_union_type(nullable)?))
    }

    pub(super) fn parse_union_type(&mut self, nullable: bool) -> Result<String, String> {
        fn intersection(parser: &mut Parser, parenthesized: bool) -> Result<String, String> {
            let mut members = Vec::new();
            loop {
                match parser.take() {
                    TokenKind::Word(name) => members.push(parser.qualify_type(name)),
                    token => {
                        return Err(parser.error(&format!("Expected type name, got {token:?}")))
                    }
                }
                if parser.peek() != &TokenKind::Ampersand
                    || !matches!(
                        parser
                            .tokens
                            .get(parser.cursor + 1)
                            .map(|token| &token.kind),
                        Some(TokenKind::Word(_))
                    )
                {
                    break;
                }
                parser.take();
            }
            if parenthesized && members.len() < 2 {
                return Err(
                    parser.error("Parenthesized type in a DNF declaration must be an intersection")
                );
            }
            Ok(members.join("&"))
        }

        let mut members = Vec::new();
        let mut parenthesized_members = Vec::new();
        loop {
            let parenthesized = self.eat(&TokenKind::Lp);
            members.push(intersection(self, parenthesized)?);
            parenthesized_members.push(parenthesized);
            if parenthesized {
                self.expect(TokenKind::Rp, "Expected ) after intersection type")?;
            }
            if !self.eat(&TokenKind::Pipe) {
                break;
            }
            if nullable {
                return Err(self.error("Nullable shorthand cannot be combined with a union type"));
            }
        }
        if members.iter().any(|member| member.contains('&')) && nullable {
            return Err(
                self.error("Nullable shorthand cannot be combined with an intersection type")
            );
        }
        if members.len() > 1
            && members
                .iter()
                .zip(parenthesized_members)
                .any(|(member, parenthesized)| member.contains('&') && !parenthesized)
        {
            return Err(self.error("Intersection members of a union type must be parenthesized"));
        }
        Ok(if nullable {
            format!("?{}", members[0])
        } else {
            members.join("|")
        })
    }

    /// Parse an interface method signature, which has no executable body.
    pub(in crate::parser) fn interface_method(
        &mut self,
        is_static: bool,
    ) -> Result<FunctionDecl, String> {
        self.take(); // Consume the function keyword.
        let returns_by_reference = self.eat(&TokenKind::Ampersand);
        let name = match self.take() {
            TokenKind::Word(name) => name,
            token => return Err(self.error(&format!("Expected method name, got {token:?}"))),
        };
        self.expect(TokenKind::Lp, "Expected ( after method name")?;
        let mut parameters = Vec::new();
        let mut parameter_attributes = Vec::new();
        let mut parameter_types = Vec::new();
        let mut parameter_by_reference = Vec::new();
        let mut parameter_variadic = Vec::new();
        let mut parameter_defaults = Vec::new();
        let mut optional_parameter_seen = false;
        if !self.eat(&TokenKind::Rp) {
            loop {
                let attributes = self.attributes()?;
                let by_reference_before_type = self.eat(&TokenKind::Ampersand);
                let variadic_before_type = self.eat(&TokenKind::Ellipsis);
                let nullable = self.eat(&TokenKind::Question);
                let has_type = (matches!(self.peek(), TokenKind::Word(_))
                    && matches!(
                        self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                        Some(
                            TokenKind::Var(_)
                                | TokenKind::Ampersand
                                | TokenKind::Ellipsis
                                | TokenKind::Pipe
                        )
                    ))
                    || matches!(self.peek(), TokenKind::Lp);
                let type_name = if has_type {
                    Some(self.parse_union_type(nullable)?)
                } else if nullable {
                    return Err(self.error("Expected parameter type after ?"));
                } else {
                    None
                };
                let reference_before_variadic = self.peek() == &TokenKind::Ampersand
                    && self
                        .tokens
                        .get(self.cursor + 1)
                        .is_some_and(|token| token.kind == TokenKind::Ellipsis);
                if reference_before_variadic {
                    self.take();
                }
                let is_variadic = variadic_before_type || self.eat(&TokenKind::Ellipsis);
                let is_by_reference = by_reference_before_type
                    || reference_before_variadic
                    || self.eat(&TokenKind::Ampersand);
                if parameter_variadic.iter().any(|variadic| *variadic) {
                    return Err(self.error("A variadic parameter must be last"));
                }
                let name = match self.take() {
                    TokenKind::Var(name) => name,
                    token => {
                        return Err(
                            self.error(&format!("Expected parameter variable, got {token:?}"))
                        )
                    }
                };
                let default = if self.eat(&TokenKind::Eq) {
                    if is_variadic {
                        return Err(self.error("A variadic parameter cannot have a default value"));
                    }
                    optional_parameter_seen = true;
                    Some(self.expression(0)?)
                } else {
                    if optional_parameter_seen && !is_variadic {
                        return Err(
                            self.error("Required parameter cannot follow an optional parameter")
                        );
                    }
                    None
                };
                parameters.push(name);
                parameter_attributes.push(attributes);
                parameter_types.push(type_name);
                parameter_by_reference.push(is_by_reference);
                parameter_variadic.push(is_variadic);
                parameter_defaults.push(default);
                if self.eat(&TokenKind::Rp) {
                    break;
                }
                self.expect(TokenKind::Comma, "Expected , or ) in method signature")?;
                if self.eat(&TokenKind::Rp) {
                    break;
                }
            }
        }
        let return_type = self.parse_return_type()?;
        self.expect(
            TokenKind::Semi,
            "Expected ; after interface method signature",
        )?;
        Ok(FunctionDecl {
            name,
            attributes: Vec::new(),
            is_static,
            is_abstract: true,
            is_final: false,
            returns_by_reference,
            visibility: Visibility::Public,
            parameters,
            parameter_attributes,
            parameter_types,
            parameter_by_reference,
            parameter_variadic,
            parameter_defaults,
            return_type,
            promoted_properties: Vec::new(),
            body: Vec::new(),
        })
    }
}

fn collect_arrow_captures(
    expression: &Expr,
    parameters: &[String],
    captures: &mut Vec<(String, bool)>,
) {
    fn add(name: &str, parameters: &[String], captures: &mut Vec<(String, bool)>) {
        if !parameters.iter().any(|parameter| parameter == name)
            && !captures.iter().any(|(captured, _)| captured == name)
        {
            captures.push((name.to_string(), false));
        }
    }
    fn visit(expression: &Expr, parameters: &[String], captures: &mut Vec<(String, bool)>) {
        match expression {
            Expr::Variable(name) | Expr::DynamicVariable(name) => add(name, parameters, captures),
            Expr::DynamicVariableExpr(name) => visit(name, parameters, captures),
            Expr::PostIncrement(target, _) | Expr::PreIncrement(target, _) => match target {
                crate::ast::IncrementTarget::Variable(name) => add(name, parameters, captures),
                crate::ast::IncrementTarget::DynamicVariable { name, keys } => {
                    visit(name, parameters, captures);
                    for key in keys {
                        visit(key, parameters, captures);
                    }
                }
                crate::ast::IncrementTarget::ArrayPath { variable, keys } => {
                    add(variable, parameters, captures);
                    for key in keys {
                        visit(key, parameters, captures);
                    }
                }
                crate::ast::IncrementTarget::PropertyArrayPath { receiver, keys, .. } => {
                    visit(receiver, parameters, captures);
                    for key in keys {
                        visit(key, parameters, captures);
                    }
                }
                crate::ast::IncrementTarget::StaticPropertyArrayPath { keys, .. } => {
                    for key in keys {
                        visit(key, parameters, captures);
                    }
                }
                crate::ast::IncrementTarget::DynamicPropertyArrayPath {
                    receiver,
                    property,
                    keys,
                } => {
                    visit(receiver, parameters, captures);
                    visit(property, parameters, captures);
                    for key in keys {
                        visit(key, parameters, captures);
                    }
                }
                crate::ast::IncrementTarget::DynamicStaticPropertyArrayPath {
                    class,
                    property,
                    keys,
                } => {
                    visit(class, parameters, captures);
                    visit(property, parameters, captures);
                    for key in keys {
                        visit(key, parameters, captures);
                    }
                }
                crate::ast::IncrementTarget::Property { receiver, .. } => {
                    visit(receiver, parameters, captures)
                }
                crate::ast::IncrementTarget::DynamicProperty { receiver, property } => {
                    visit(receiver, parameters, captures);
                    visit(property, parameters, captures);
                }
                crate::ast::IncrementTarget::StaticProperty { .. } => {}
            },
            Expr::Array(entries) => {
                for entry in entries {
                    if let Some(key) = &entry.key {
                        visit(key, parameters, captures);
                    }
                    visit(&entry.value, parameters, captures);
                }
            }
            Expr::Index(array, key) => {
                visit(array, parameters, captures);
                visit(key, parameters, captures);
            }
            Expr::Call(_, arguments)
            | Expr::NewObject(_, arguments, _)
            | Expr::ParentMethodCall(_, arguments) => {
                for argument in arguments {
                    visit(argument, parameters, captures);
                }
            }
            Expr::DynamicNewObject(class, arguments, _) => {
                visit(class, parameters, captures);
                for argument in arguments {
                    visit(argument, parameters, captures);
                }
            }
            Expr::NamedArgument(_, value)
            | Expr::UnpackArgument(value)
            | Expr::Clone(value)
            | Expr::Empty(value)
            | Expr::Negate(value)
            | Expr::Positive(value)
            | Expr::Not(value)
            | Expr::ErrorSuppress(value)
            | Expr::BitwiseNot(value)
            | Expr::YieldFrom(value)
            | Expr::DeprecatedInterpolation(value, _) => visit(value, parameters, captures),
            Expr::Cast(_, value) => visit(value, parameters, captures),
            Expr::MethodCall(object, _, arguments) => {
                visit(object, parameters, captures);
                for argument in arguments {
                    visit(argument, parameters, captures);
                }
            }
            Expr::DynamicMethodCall(object, method, arguments) => {
                visit(object, parameters, captures);
                visit(method, parameters, captures);
                for argument in arguments {
                    visit(argument, parameters, captures);
                }
            }
            Expr::StaticCall(_, _, arguments)
            | Expr::DynamicStaticCall(_, _, arguments)
            | Expr::Invoke(_, arguments) => {
                for argument in arguments {
                    visit(argument, parameters, captures);
                }
                if let Expr::DynamicStaticCall(class, _, _) = expression {
                    visit(class, parameters, captures);
                }
                if let Expr::Invoke(callee, _) = expression {
                    visit(callee, parameters, captures);
                }
            }
            Expr::DynamicStaticCallExpr(class, method, arguments) => {
                visit(class, parameters, captures);
                visit(method, parameters, captures);
                for argument in arguments {
                    visit(argument, parameters, captures);
                }
            }
            Expr::DynamicClassName(value) => visit(value, parameters, captures),
            Expr::DynamicClassConstant(class, _) => visit(class, parameters, captures),
            Expr::DynamicStaticProperty(class, _) => visit(class, parameters, captures),
            Expr::DynamicStaticPropertyExpr(class, property) => {
                visit(class, parameters, captures);
                visit(property, parameters, captures);
            }
            Expr::Property(object, _) | Expr::IssetProperty(object, _) => {
                visit(object, parameters, captures)
            }
            Expr::IssetPropertyChain(object, properties) => {
                visit(object, parameters, captures);
                for property in properties {
                    match property {
                        crate::ast::IssetPropertySegment::Dynamic(expression) => {
                            visit(expression, parameters, captures)
                        }
                        crate::ast::IssetPropertySegment::ArrayOffset(keys) => {
                            for key in keys {
                                visit(key, parameters, captures);
                            }
                        }
                        crate::ast::IssetPropertySegment::Fixed(_) => {}
                    }
                }
            }
            Expr::IssetPropertyPath(object, _, keys) => {
                visit(object, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
            }
            Expr::IssetDynamicPropertyPath(object, name, keys) => {
                visit(object, parameters, captures);
                visit(name, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
            }
            Expr::DynamicProperty(object, name) => {
                visit(object, parameters, captures);
                visit(name, parameters, captures);
            }
            Expr::IssetDynamicProperty(object, name) => {
                visit(object, parameters, captures);
                visit(name, parameters, captures);
            }
            Expr::DynamicPropertyAssignment {
                receiver,
                property,
                value,
            } => {
                visit(receiver, parameters, captures);
                visit(property, parameters, captures);
                visit(value, parameters, captures);
            }
            Expr::Isset(values) => {
                for value in values {
                    visit(value, parameters, captures);
                }
            }
            Expr::Binary(_, left, right)
            | Expr::Coalesce(left, right)
            | Expr::InstanceOf(left, right) => {
                visit(left, parameters, captures);
                visit(right, parameters, captures);
            }
            Expr::Assignment {
                variable, value, ..
            } => {
                add(variable, parameters, captures);
                visit(value, parameters, captures);
            }
            Expr::ReferenceAssignment { variable, value } => {
                add(variable, parameters, captures);
                visit(value, parameters, captures);
            }
            Expr::DynamicVariableAssignment { name, value, .. } => {
                visit(name, parameters, captures);
                visit(value, parameters, captures);
            }
            Expr::ArrayAssignment {
                variable,
                keys,
                value,
                ..
            } => {
                add(variable, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::ListAssignment { targets, value } => {
                let mut pending = targets.iter().collect::<Vec<_>>();
                while let Some(target) = pending.pop() {
                    if let Some(variable) = &target.variable {
                        add(variable, parameters, captures);
                    }
                    if let Some(variable) = &target.array_variable {
                        add(variable, parameters, captures);
                    }
                    for key in &target.array_keys {
                        visit(key, parameters, captures);
                    }
                    if let Some(receiver) = &target.property_receiver {
                        visit(receiver, parameters, captures);
                    }
                    if let Some(nested) = &target.nested {
                        pending.extend(nested);
                    }
                }
                visit(value, parameters, captures);
            }
            Expr::PropertyAssignment {
                receiver, value, ..
            } => {
                visit(receiver, parameters, captures);
                visit(value, parameters, captures);
            }
            Expr::PropertyArrayAssignment {
                receiver,
                keys,
                value,
                ..
            } => {
                visit(receiver, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::PropertyAppendAssignment {
                receiver,
                keys,
                value,
                ..
            } => {
                visit(receiver, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::DynamicPropertyAppendAssignment {
                receiver,
                property,
                keys,
                value,
            } => {
                visit(receiver, parameters, captures);
                visit(property, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::DynamicPropertyCompoundAssignment {
                receiver,
                property,
                value,
                ..
            } => {
                visit(receiver, parameters, captures);
                visit(property, parameters, captures);
                visit(value, parameters, captures);
            }
            Expr::PropertyAppend(receiver) => visit(receiver, parameters, captures),
            Expr::StaticPropertyArrayAssignment { keys, value, .. } => {
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::DynamicStaticPropertyArrayAssignment {
                class,
                property,
                keys,
                value,
                ..
            } => {
                visit(class, parameters, captures);
                visit(property, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::ArrayAppendAssignment {
                variable,
                keys,
                value,
            } => {
                add(variable, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::ArrayExpressionAppendAssignment { array, keys, value } => {
                visit(array, parameters, captures);
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::StaticPropertyAppendAssignment { keys, value, .. } => {
                for key in keys {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::StaticPropertyAssignment { value, .. } => visit(value, parameters, captures),
            Expr::Ternary {
                condition,
                then_value,
                else_value,
            } => {
                visit(condition, parameters, captures);
                visit(then_value, parameters, captures);
                visit(else_value, parameters, captures);
            }
            Expr::Match { subject, arms } => {
                visit(subject, parameters, captures);
                for arm in arms {
                    for condition in &arm.conditions {
                        visit(condition, parameters, captures);
                    }
                    visit(&arm.value, parameters, captures);
                }
            }
            Expr::ShorthandTernary {
                condition,
                else_value,
            } => {
                visit(condition, parameters, captures);
                visit(else_value, parameters, captures);
            }
            Expr::Closure {
                captures: nested, ..
            } => {
                for (name, _) in nested {
                    add(name, parameters, captures);
                }
            }
            Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::InstanceMethod(
                receiver,
                _,
            )) => visit(receiver, parameters, captures),
            Expr::FirstClassCallable(crate::ast::FirstClassCallableTarget::Value(value)) => {
                visit(value, parameters, captures)
            }
            Expr::FirstClassCallable(
                crate::ast::FirstClassCallableTarget::Function(_)
                | crate::ast::FirstClassCallableTarget::StaticMethod(_, _),
            ) => {}
            Expr::Yield { key, value } => {
                if let Some(key) = key {
                    visit(key, parameters, captures);
                }
                visit(value, parameters, captures);
            }
            Expr::Integer(_)
            | Expr::Number(_)
            | Expr::String(_)
            | Expr::Bool(_)
            | Expr::Null
            | Expr::Constant(_)
            | Expr::MagicConstant(_)
            | Expr::StaticProperty(_, _)
            | Expr::IssetStaticProperty(_, _)
            | Expr::ClassConstant(_, _) => {}
        }
    }
    visit(expression, parameters, captures);
}
