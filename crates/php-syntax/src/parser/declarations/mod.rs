//! Parsing for class, interface, and trait declarations and top-level program assembly.

use super::Parser;
use crate::{
    ast::{
        ClassConstantDecl, ClassDecl, Expr, InterfaceDecl, Program, PropertyDecl, TraitAdaptation,
        TraitDecl, Visibility,
    },
    lexer::token::TokenKind,
};

impl Parser<'_> {
    pub(super) fn attributes(&mut self) -> Result<Vec<crate::ast::AttributeDecl>, String> {
        let mut attributes = Vec::new();
        while self.eat(&TokenKind::AttributeStart) {
            loop {
                let name_offset = self.peek_offset();
                let name = match self.take() {
                    TokenKind::Word(_) => self.qualify_name(self.source_word(name_offset)),
                    token => {
                        return Err(self.error(&format!("Expected attribute name, got {token:?}")))
                    }
                };
                let mut arguments = Vec::new();
                if self.eat(&TokenKind::Lp) && !self.eat(&TokenKind::Rp) {
                    loop {
                        arguments.push(self.call_argument()?);
                        if self.eat(&TokenKind::Rp) {
                            break;
                        }
                        self.expect(TokenKind::Comma, "Expected , or ) in attribute arguments")?;
                        if self.eat(&TokenKind::Rp) {
                            break;
                        }
                    }
                }
                attributes.push(crate::ast::AttributeDecl { name, arguments });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(TokenKind::Rs, "Expected ] after attribute group")?;
        }
        Ok(attributes)
    }

    fn property_hooks(
        &mut self,
    ) -> Result<
        (
            Option<Vec<crate::ast::Stmt>>,
            Option<crate::ast::PropertySetHook>,
        ),
        String,
    > {
        let mut get_hook = None;
        let mut set_hook = None;
        if !self.eat(&TokenKind::Lb) {
            return Ok((get_hook, set_hook));
        }
        while !self.eat(&TokenKind::Rb) {
            if self.peek() == &TokenKind::Eof {
                return Err(self.error("Unterminated property hook block"));
            }
            if matches!(self.peek(), TokenKind::Word(word) if matches!(word.as_str(), "public" | "protected" | "private"))
            {
                self.take();
            }
            let hook = match self.take() {
                TokenKind::Word(name) if name == "get" || name == "set" => name,
                token => {
                    return Err(
                        self.error(&format!("Expected get or set property hook, got {token:?}"))
                    )
                }
            };
            let parameter = if hook == "set" && self.eat(&TokenKind::Lp) {
                let name = match self.take() {
                    TokenKind::Var(name) => name,
                    token => {
                        return Err(self.error(&format!("Expected setter parameter, got {token:?}")))
                    }
                };
                self.expect(TokenKind::Rp, "Expected ) after setter parameter")?;
                name
            } else {
                "value".to_string()
            };
            let body = self.block()?;
            if hook == "get" {
                if get_hook.replace(body).is_some() {
                    return Err(self.error("Property cannot declare more than one get hook"));
                }
            } else if set_hook
                .replace(crate::ast::PropertySetHook { parameter, body })
                .is_some()
            {
                return Err(self.error("Property cannot declare more than one set hook"));
            }
        }
        if get_hook.is_none() && set_hook.is_none() {
            return Err(self.error("Property hook block must declare get or set"));
        }
        Ok((get_hook, set_hook))
    }

    fn register_namespace_import(
        &mut self,
        function_import: bool,
        constant_import: bool,
        alias: String,
        target: String,
    ) -> Result<(), String> {
        let duplicate = if constant_import {
            self.constant_imports.insert(alias, target).is_some()
        } else if function_import {
            self.function_imports
                .insert(alias.to_ascii_lowercase(), target)
                .is_some()
        } else {
            self.class_imports
                .insert(alias.to_ascii_lowercase(), target)
                .is_some()
        };
        if duplicate {
            Err(self.error("Import alias is already defined"))
        } else {
            Ok(())
        }
    }

    /// Parse top-level statements and declarations into the program AST.
    pub(in crate::parser) fn program(mut self) -> Result<Program, String> {
        let mut statements = Vec::new();
        let mut functions = Vec::new();
        let mut classes = Vec::new();
        let mut interfaces = Vec::new();
        let mut traits = Vec::new();
        let mut constants = Vec::new();
        let mut namespace_style = None::<bool>;
        let mut inside_namespace_block = false;
        while self.peek() != &TokenKind::Eof {
            if inside_namespace_block && self.peek() == &TokenKind::Rb {
                self.take();
                inside_namespace_block = false;
                self.namespace = None;
                self.class_imports.clear();
                self.function_imports.clear();
                self.constant_imports.clear();
                continue;
            }
            if matches!(self.peek(), TokenKind::Word(word) if word == "namespace") {
                self.take();
                let namespace_name = if matches!(self.peek(), TokenKind::Word(_)) {
                    let offset = self.peek_offset();
                    self.take();
                    Some(self.source_word(offset))
                } else {
                    None
                };
                let is_block = self.peek() == &TokenKind::Lb;
                if namespace_style.is_some_and(|style| style != is_block) {
                    return Err(
                        self.error("Cannot mix bracketed and semicolon namespace declarations")
                    );
                }
                namespace_style = Some(is_block);
                self.namespace = namespace_name;
                self.class_imports.clear();
                self.function_imports.clear();
                self.constant_imports.clear();
                if is_block {
                    self.take();
                    inside_namespace_block = true;
                } else {
                    if self.namespace.is_none() {
                        return Err(self.error("Expected namespace name before ;"));
                    }
                    self.expect(TokenKind::Semi, "Expected ; after namespace declaration")?;
                }
                continue;
            }
            if namespace_style == Some(true) && !inside_namespace_block {
                return Err(self.error("Code outside a bracketed namespace block is not allowed"));
            }
            let class_attributes = self.attributes()?;
            if matches!(self.peek(), TokenKind::Word(word) if word == "use") {
                self.take();
                let is_function_import = if matches!(self.peek(), TokenKind::Word(word) if word == "function")
                {
                    self.take();
                    true
                } else {
                    false
                };
                let is_constant_import = !is_function_import
                    && if matches!(self.peek(), TokenKind::Word(word) if word == "const") {
                        self.take();
                        true
                    } else {
                        false
                    };
                let target_offset = self.peek_offset();
                let target_source = match self.take() {
                    TokenKind::Word(_) => self
                        .source_word(target_offset)
                        .trim_start_matches('\\')
                        .to_string(),
                    token => {
                        return Err(self.error(&format!("Expected imported name, got {token:?}")))
                    }
                };
                let target = if is_constant_import {
                    target_source.clone()
                } else {
                    target_source.to_ascii_lowercase()
                };
                if self.eat(&TokenKind::Lb) {
                    let prefix = target_source.trim_end_matches('\\').to_string();
                    let mut grouped = Vec::new();
                    loop {
                        let mut item_is_function = is_function_import;
                        let mut item_is_constant = is_constant_import;
                        if matches!(self.peek(), TokenKind::Word(word) if word == "function" || word == "const")
                        {
                            item_is_function = self.eat(&TokenKind::Word("function".into()));
                            item_is_constant = if item_is_function {
                                false
                            } else {
                                self.eat(&TokenKind::Word("const".into()))
                            };
                            if !item_is_function && !item_is_constant {
                                return Err(
                                    self.error("Expected function or const import modifier")
                                );
                            }
                        }
                        let name_offset = self.peek_offset();
                        let name = match self.take() {
                            TokenKind::Word(_) => self.source_word(name_offset),
                            token => {
                                return Err(self.error(&format!(
                                    "Expected grouped import name, got {token:?}"
                                )))
                            }
                        };
                        let full_name = if name.starts_with('\\') {
                            name.trim_start_matches('\\').to_string()
                        } else {
                            format!("{prefix}\\{name}")
                        };
                        let full_name = if item_is_constant {
                            full_name
                        } else {
                            full_name.to_ascii_lowercase()
                        };
                        let alias = if matches!(self.peek(), TokenKind::Word(word) if word == "as")
                        {
                            self.take();
                            let alias_offset = self.peek_offset();
                            match self.take() {
                                TokenKind::Word(alias) => {
                                    if item_is_constant {
                                        self.source_word(alias_offset)
                                    } else {
                                        alias
                                    }
                                }
                                token => {
                                    return Err(self
                                        .error(&format!("Expected import alias, got {token:?}")))
                                }
                            }
                        } else {
                            name.rsplit('\\').next().unwrap_or(&name).to_string()
                        };
                        grouped.push((item_is_function, item_is_constant, alias, full_name));
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                        if self.peek() == &TokenKind::Rb {
                            break;
                        }
                    }
                    self.expect(TokenKind::Rb, "Expected } after grouped imports")?;
                    self.expect(TokenKind::Semi, "Expected ; after grouped imports")?;
                    for (item_is_function, item_is_constant, alias, full_name) in grouped {
                        self.register_namespace_import(
                            item_is_function,
                            item_is_constant,
                            alias,
                            full_name,
                        )?;
                    }
                    continue;
                }
                let alias = if matches!(self.peek(), TokenKind::Word(word) if word == "as") {
                    self.take();
                    let alias_offset = self.peek_offset();
                    match self.take() {
                        TokenKind::Word(alias) => {
                            if is_constant_import {
                                self.source_word(alias_offset)
                            } else {
                                alias
                            }
                        }
                        token => {
                            return Err(self.error(&format!("Expected import alias, got {token:?}")))
                        }
                    }
                } else {
                    target.rsplit('\\').next().unwrap_or(&target).to_string()
                };
                let mut imports = vec![(alias, target)];
                while self.eat(&TokenKind::Comma) {
                    let offset = self.peek_offset();
                    let source = match self.take() {
                        TokenKind::Word(_) => self
                            .source_word(offset)
                            .trim_start_matches('\\')
                            .to_string(),
                        token => {
                            return Err(self.error(&format!(
                                "Expected imported name after comma, got {token:?}"
                            )))
                        }
                    };
                    let imported = if is_constant_import {
                        source.clone()
                    } else {
                        source.to_ascii_lowercase()
                    };
                    let alias = if matches!(self.peek(), TokenKind::Word(word) if word == "as") {
                        self.take();
                        match self.take() {
                            TokenKind::Word(alias) => {
                                if is_constant_import {
                                    alias
                                } else {
                                    alias.to_ascii_lowercase()
                                }
                            }
                            token => {
                                return Err(
                                    self.error(&format!("Expected import alias, got {token:?}"))
                                )
                            }
                        }
                    } else {
                        imported
                            .rsplit('\\')
                            .next()
                            .unwrap_or(&imported)
                            .to_string()
                    };
                    imports.push((alias, imported));
                }
                for (alias, target) in imports {
                    self.register_namespace_import(
                        is_function_import,
                        is_constant_import,
                        alias,
                        target,
                    )?;
                }
                self.expect(TokenKind::Semi, "Expected ; after import")?;
                continue;
            }
            if matches!(self.peek(), TokenKind::Word(word) if word == "const") {
                self.take();
                loop {
                    let name_offset = self.peek_offset();
                    match self.take() {
                        TokenKind::Word(_) => {}
                        token => {
                            return Err(
                                self.error(&format!("Expected constant name, got {token:?}"))
                            )
                        }
                    }
                    let name = self.qualify_declaration_name(self.source_word(name_offset));
                    self.expect(TokenKind::Eq, "Expected = in constant declaration")?;
                    let value = self.expression(1)?;
                    constants.push(crate::ast::ConstantDecl {
                        name,
                        attributes: class_attributes.clone(),
                        value,
                    });
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(TokenKind::Semi, "Expected ; after constant declaration")?;
                continue;
            }
            let mut class_is_abstract = false;
            let mut class_is_final = false;
            let mut class_is_readonly = false;
            while matches!(self.peek(), TokenKind::Word(word) if matches!(word.as_str(), "abstract" | "final" | "readonly"))
            {
                match self.take() {
                    TokenKind::Word(modifier) if modifier == "abstract" => class_is_abstract = true,
                    TokenKind::Word(modifier) if modifier == "final" => class_is_final = true,
                    TokenKind::Word(_) => class_is_readonly = true,
                    _ => unreachable!(),
                }
            }
            if matches!(self.peek(), TokenKind::Word(word) if word == "function") {
                let mut function =
                    self.function_declaration(false, false, false, Visibility::Public)?;
                function.attributes = class_attributes;
                function.name = self.qualify_declaration_name(function.name);
                functions.push(function);
            } else if matches!(self.peek(), TokenKind::Word(word) if word == "class" || word == "enum")
            {
                let is_enum = matches!(self.peek(), TokenKind::Word(word) if word == "enum");
                self.take();
                let name = match self.take() {
                    TokenKind::Word(name) => {
                        self.qualify_declaration_name(self.previous_word_spelling(&name))
                    }
                    token => return Err(self.error(&format!("Expected class name, got {token:?}"))),
                };
                let enum_backing_type = if is_enum && self.eat(&TokenKind::Colon) {
                    // Backed enum scalar type is syntax metadata; case values are retained below.
                    Some(match self.take() {
                        TokenKind::Word(kind) if kind == "string" || kind == "int" => kind,
                        token => {
                            return Err(
                                self.error(&format!("Expected enum backing type, got {token:?}"))
                            )
                        }
                    })
                } else {
                    None
                };
                let parent = if !is_enum
                    && matches!(self.peek(), TokenKind::Word(word) if word == "extends")
                {
                    self.take();
                    match self.take() {
                        TokenKind::Word(name) => Some(self.qualify_name(name)),
                        token => {
                            return Err(
                                self.error(&format!("Expected parent class name, got {token:?}"))
                            )
                        }
                    }
                } else {
                    None
                };
                let mut implemented_interfaces = Vec::new();
                if matches!(self.peek(), TokenKind::Word(word) if word == "implements") {
                    self.take();
                    loop {
                        match self.take() {
                            TokenKind::Word(name) => {
                                implemented_interfaces.push(self.qualify_name(name))
                            }
                            token => {
                                return Err(
                                    self.error(&format!("Expected interface name, got {token:?}"))
                                )
                            }
                        }
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::Lb, "Expected { after class name")?;
                let mut properties = Vec::new();
                let mut constants = Vec::new();
                let mut methods = Vec::new();
                let mut used_traits = Vec::new();
                let mut trait_adaptations = Vec::new();
                // Class members are parsed in source order; trait uses are expanded later.
                while !self.eat(&TokenKind::Rb) {
                    if self.peek() == &TokenKind::Eof {
                        return Err(self.error("Unterminated class body"));
                    }
                    let member_attributes = self.attributes()?;
                    if is_enum && matches!(self.peek(), TokenKind::Word(word) if word == "case") {
                        self.take();
                        let case_name_offset = self.peek_offset();
                        let case_name = match self.take() {
                            TokenKind::Word(_) => self.source_word(case_name_offset),
                            token => {
                                return Err(
                                    self.error(&format!("Expected enum case name, got {token:?}"))
                                )
                            }
                        };
                        let enum_case_has_value = self.eat(&TokenKind::Eq);
                        let value = if enum_case_has_value {
                            self.expression(1)?
                        } else {
                            Expr::String(case_name.clone())
                        };
                        self.expect(TokenKind::Semi, "Expected ; after enum case")?;
                        constants.push(ClassConstantDecl {
                            name: case_name,
                            attributes: member_attributes,
                            value,
                            type_name: None,
                            visibility: Visibility::Public,
                            is_final: true,
                            is_enum_case: true,
                            enum_case_has_value,
                        });
                        continue;
                    }
                    if matches!(self.peek(), TokenKind::Word(word) if word == "use") {
                        self.take();
                        loop {
                            match self.take() {
                                TokenKind::Word(name) => used_traits
                                    .push(self.qualify_name(self.previous_word_spelling(&name))),
                                token => {
                                    return Err(
                                        self.error(&format!("Expected trait name, got {token:?}"))
                                    )
                                }
                            }
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                        if self.eat(&TokenKind::Semi) {
                            continue;
                        }
                        self.expect(TokenKind::Lb, "Expected ; or { after trait use")?;
                        while !self.eat(&TokenKind::Rb) {
                            let first_name = match self.take() {
                                TokenKind::Word(name) => self.previous_word_spelling(&name),
                                token => {
                                    return Err(self.error(&format!(
                                    "Expected trait or method name in adaptation, got {token:?}"
                                )))
                                }
                            };
                            let (trait_name, method_name) = if self.eat(&TokenKind::DoubleColon) {
                                let method = match self.take() {
                                    TokenKind::Word(name) => self.previous_word_spelling(&name),
                                    token => {
                                        return Err(self.error(&format!(
                                            "Expected trait method name, got {token:?}"
                                        )))
                                    }
                                };
                                (self.qualify_name(first_name), method)
                            } else {
                                let trait_name = used_traits.last().cloned().ok_or_else(|| {
                                    self.error("Unqualified trait adaptation requires a trait use")
                                })?;
                                (trait_name, first_name)
                            };
                            let mut excluded_traits = Vec::new();
                            let mut alias = None;
                            let mut adaptation_visibility = None;
                            let mut is_final = false;
                            match self.take() {
                                TokenKind::Word(word) if word == "insteadof" => loop {
                                    match self.take() {
                                        TokenKind::Word(name) => excluded_traits.push(
                                            self.qualify_name(self.previous_word_spelling(&name)),
                                        ),
                                        token => {
                                            return Err(self.error(&format!(
                                                "Expected excluded trait name, got {token:?}"
                                            )))
                                        }
                                    }
                                    if !self.eat(&TokenKind::Comma) {
                                        break;
                                    }
                                },
                                TokenKind::Word(word) if word == "as" => {
                                    while let TokenKind::Word(word) = self.peek() {
                                        match word.as_str() {
                                            "public" => {
                                                adaptation_visibility = Some(Visibility::Public);
                                                self.take();
                                            }
                                            "protected" => {
                                                adaptation_visibility = Some(Visibility::Protected);
                                                self.take();
                                            }
                                            "private" => {
                                                adaptation_visibility = Some(Visibility::Private);
                                                self.take();
                                            }
                                            "final" => {
                                                is_final = true;
                                                self.take();
                                            }
                                            _ => break,
                                        }
                                    }
                                    if let TokenKind::Word(name) = self.peek() {
                                        let fallback = name.clone();
                                        self.take();
                                        alias = Some(self.previous_word_spelling(&fallback));
                                    }
                                    if alias.is_none()
                                        && adaptation_visibility.is_none()
                                        && !is_final
                                    {
                                        return Err(self.error("Trait alias must specify a new name, visibility, or final modifier"));
                                    }
                                }
                                token => {
                                    return Err(self.error(&format!(
                                        "Expected insteadof or as, got {token:?}"
                                    )))
                                }
                            }
                            self.expect(TokenKind::Semi, "Expected ; after trait adaptation")?;
                            trait_adaptations.push(TraitAdaptation {
                                trait_name,
                                method_name,
                                excluded_traits,
                                alias,
                                visibility: adaptation_visibility,
                                is_final,
                            });
                        }
                        continue;
                    }
                    let (mut is_static, mut is_abstract, mut is_final) = (false, false, false);
                    let mut is_readonly = class_is_readonly;
                    let mut visibility = Visibility::Public;
                    let mut set_visibility = None;
                    while let TokenKind::Word(word) = self.peek() {
                        if matches!(word.as_str(), "public" | "private" | "protected")
                            && self
                                .tokens
                                .get(self.cursor + 1)
                                .is_some_and(|token| token.kind == TokenKind::Lp)
                            && self
                                .tokens
                                .get(self.cursor + 2)
                                .is_some_and(|token| token.kind == TokenKind::Word("set".into()))
                        {
                            set_visibility = Some(match self.take() {
                                TokenKind::Word(name) if name == "public" => Visibility::Public,
                                TokenKind::Word(name) if name == "protected" => {
                                    Visibility::Protected
                                }
                                _ => Visibility::Private,
                            });
                            self.expect(TokenKind::Lp, "Expected ( before setter visibility")?;
                            self.expect(
                                TokenKind::Word("set".into()),
                                "Expected set in asymmetric visibility",
                            )?;
                            self.expect(TokenKind::Rp, "Expected ) after asymmetric visibility")?;
                            continue;
                        }
                        if matches!(
                            word.as_str(),
                            "public"
                                | "private"
                                | "protected"
                                | "static"
                                | "abstract"
                                | "final"
                                | "readonly"
                        ) {
                            match word.as_str() {
                                "public" => visibility = Visibility::Public,
                                "protected" => visibility = Visibility::Protected,
                                "private" => visibility = Visibility::Private,
                                "static" => is_static = true,
                                "abstract" => is_abstract = true,
                                "final" => is_final = true,
                                "readonly" => is_readonly = true,
                                _ => unreachable!(),
                            }
                            self.take();
                        } else {
                            break;
                        }
                    }
                    if matches!(self.peek(), TokenKind::Word(word) if word == "function") {
                        let mut method = self.function_declaration(
                            is_static,
                            is_abstract,
                            is_final,
                            visibility,
                        )?;
                        method.attributes = member_attributes;
                        methods.push(method);
                    } else if matches!(self.peek(), TokenKind::Word(word) if word == "const") {
                        if is_static || is_abstract {
                            return Err(self.error("Invalid modifier on class constant"));
                        }
                        self.take();
                        let nullable = self.eat(&TokenKind::Question);
                        let type_name = if nullable
                            || matches!(self.peek(), TokenKind::Word(_))
                                && matches!(
                                    self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                                    Some(
                                        TokenKind::Word(_) | TokenKind::Pipe | TokenKind::Ampersand
                                    )
                                ) {
                            Some(self.parse_union_type(nullable)?)
                        } else {
                            None
                        };
                        loop {
                            let const_name_offset = self.peek_offset();
                            let const_name = match self.take() {
                                TokenKind::Word(_) => self.source_word(const_name_offset),
                                token => {
                                    return Err(self.error(&format!(
                                        "Expected class constant name, got {token:?}"
                                    )))
                                }
                            };
                            self.expect(TokenKind::Eq, "Expected = in class constant declaration")?;
                            let value = self.expression(1)?;
                            constants.push(ClassConstantDecl {
                                name: const_name,
                                attributes: member_attributes.clone(),
                                value,
                                type_name: type_name.clone(),
                                visibility,
                                is_final,
                                is_enum_case: false,
                                enum_case_has_value: false,
                            });
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                        self.expect(TokenKind::Semi, "Expected ; after class constant")?;
                    } else {
                        let nullable = self.eat(&TokenKind::Question);
                        let has_type = (matches!(self.peek(), TokenKind::Word(_))
                            && matches!(
                                self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                                Some(TokenKind::Var(_) | TokenKind::Pipe | TokenKind::Ampersand)
                            ))
                            || matches!(self.peek(), TokenKind::Lp);
                        let type_name = if has_type {
                            Some(self.parse_union_type(nullable)?)
                        } else if nullable {
                            return Err(self.error("Expected property type after ?"));
                        } else {
                            None
                        };
                        let property = match self.take() {
                            TokenKind::Var(name) => name,
                            token => {
                                return Err(self.error(&format!(
                                    "Expected class property or method, got {token:?}"
                                )))
                            }
                        };
                        let default = if self.eat(&TokenKind::Eq) {
                            Some(self.expression(1)?)
                        } else {
                            None
                        };
                        let (get_hook, set_hook) = if self.peek() == &TokenKind::Lb {
                            self.property_hooks()?
                        } else {
                            (None, None)
                        };
                        if get_hook.is_some() || set_hook.is_some() {
                            if is_static {
                                return Err(self.error(
                                    "Property hooks cannot be declared on static properties",
                                ));
                            }
                        } else {
                            self.expect(TokenKind::Semi, "Expected ; after class property")?;
                        }
                        properties.push(PropertyDecl {
                            name: property,
                            attributes: member_attributes,
                            default,
                            is_static,
                            is_readonly,
                            visibility,
                            set_visibility,
                            type_name,
                            get_hook,
                            set_hook,
                        });
                    }
                }
                classes.push(ClassDecl {
                    name,
                    anonymous_source: None,
                    attributes: class_attributes,
                    is_enum,
                    enum_backing_type,
                    is_abstract: class_is_abstract,
                    is_final: class_is_final || is_enum,
                    is_readonly: class_is_readonly,
                    parent,
                    interfaces: implemented_interfaces,
                    traits: used_traits,
                    trait_adaptations,
                    properties,
                    constants,
                    methods,
                });
            } else if matches!(self.peek(), TokenKind::Word(word) if word == "interface") {
                self.take();
                let name = match self.take() {
                    TokenKind::Word(name) => {
                        self.qualify_declaration_name(self.previous_word_spelling(&name))
                    }
                    token => {
                        return Err(self.error(&format!("Expected interface name, got {token:?}")))
                    }
                };
                let mut parents = Vec::new();
                if matches!(self.peek(), TokenKind::Word(word) if word == "extends") {
                    self.take();
                    loop {
                        match self.take() {
                            TokenKind::Word(name) => parents.push(self.qualify_name(name)),
                            token => {
                                return Err(self.error(&format!(
                                    "Expected parent interface name, got {token:?}"
                                )))
                            }
                        }
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::Lb, "Expected { after interface name")?;
                let mut methods = Vec::new();
                let mut constants = Vec::new();
                while !self.eat(&TokenKind::Rb) {
                    let member_attributes = self.attributes()?;
                    let mut constant_visibility = Visibility::Public;
                    let mut constant_final = false;
                    while let TokenKind::Word(word) = self.peek() {
                        match word.as_str() {
                            "public" => constant_visibility = Visibility::Public,
                            "protected" => constant_visibility = Visibility::Protected,
                            "private" => constant_visibility = Visibility::Private,
                            "final" => constant_final = true,
                            _ => break,
                        }
                        self.take();
                    }
                    if matches!(self.peek(), TokenKind::Word(word) if word == "const") {
                        self.take();
                        let nullable = self.eat(&TokenKind::Question);
                        let type_name = if nullable
                            || matches!(self.peek(), TokenKind::Word(_))
                                && matches!(
                                    self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                                    Some(
                                        TokenKind::Word(_) | TokenKind::Pipe | TokenKind::Ampersand
                                    )
                                ) {
                            Some(self.parse_union_type(nullable)?)
                        } else {
                            None
                        };
                        let const_name_offset = self.peek_offset();
                        let const_name = match self.take() {
                            TokenKind::Word(_) => self.source_word(const_name_offset),
                            token => {
                                return Err(self.error(&format!(
                                    "Expected interface constant name, got {token:?}"
                                )))
                            }
                        };
                        self.expect(TokenKind::Eq, "Expected = in interface constant")?;
                        let value = self.expression(1)?;
                        self.expect(TokenKind::Semi, "Expected ; after interface constant")?;
                        constants.push(ClassConstantDecl {
                            name: const_name,
                            attributes: member_attributes,
                            value,
                            type_name,
                            visibility: constant_visibility,
                            is_final: constant_final,
                            is_enum_case: false,
                            enum_case_has_value: false,
                        });
                        continue;
                    }
                    let mut is_static = false;
                    while let TokenKind::Word(word) = self.peek() {
                        if matches!(word.as_str(), "public" | "static") {
                            is_static |= word == "static";
                            self.take();
                        } else {
                            break;
                        }
                    }
                    if !matches!(self.peek(), TokenKind::Word(word) if word == "function") {
                        return Err(self.error("Expected interface method"));
                    }
                    let mut method = self.interface_method(is_static)?;
                    method.attributes = member_attributes;
                    methods.push(method);
                }
                interfaces.push(InterfaceDecl {
                    name,
                    attributes: class_attributes,
                    parents,
                    constants,
                    methods,
                });
            } else if matches!(self.peek(), TokenKind::Word(word) if word == "trait") {
                self.take();
                let name = match self.take() {
                    TokenKind::Word(name) => {
                        self.qualify_declaration_name(self.previous_word_spelling(&name))
                    }
                    token => return Err(self.error(&format!("Expected trait name, got {token:?}"))),
                };
                self.expect(TokenKind::Lb, "Expected { after trait name")?;
                let mut properties = Vec::new();
                let mut trait_constants = Vec::new();
                let mut methods = Vec::new();
                let mut nested_traits = Vec::new();
                let mut nested_trait_adaptations = Vec::new();
                while !self.eat(&TokenKind::Rb) {
                    let member_attributes = self.attributes()?;
                    if matches!(self.peek(), TokenKind::Word(word) if word == "use") {
                        self.take();
                        loop {
                            match self.take() {
                                TokenKind::Word(name) => nested_traits
                                    .push(self.qualify_name(self.previous_word_spelling(&name))),
                                token => {
                                    return Err(self.error(&format!(
                                        "Expected trait name in trait use, got {token:?}"
                                    )))
                                }
                            }
                            if !self.eat(&TokenKind::Comma) {
                                break;
                            }
                        }
                        if self.eat(&TokenKind::Semi) {
                            continue;
                        }
                        self.expect(TokenKind::Lb, "Expected ; or { after nested trait use")?;
                        while !self.eat(&TokenKind::Rb) {
                            let first = match self.take() {
                                TokenKind::Word(name) => self.previous_word_spelling(&name),
                                token => {
                                    return Err(self.error(&format!(
                                        "Expected trait method adaptation, got {token:?}"
                                    )))
                                }
                            };
                            let (trait_name, method_name) = if self.eat(&TokenKind::DoubleColon) {
                                let method = match self.take() {
                                    TokenKind::Word(name) => self.previous_word_spelling(&name),
                                    token => {
                                        return Err(self.error(&format!(
                                            "Expected trait method name, got {token:?}"
                                        )))
                                    }
                                };
                                (self.qualify_name(first), method)
                            } else {
                                (
                                    nested_traits.last().cloned().ok_or_else(|| {
                                        self.error(
                                            "Unqualified trait adaptation requires a trait use",
                                        )
                                    })?,
                                    first,
                                )
                            };
                            let mut excluded_traits = Vec::new();
                            let mut alias = None;
                            let mut visibility = None;
                            let mut is_final = false;
                            match self.take() {
                                TokenKind::Word(word) if word == "insteadof" => loop {
                                    match self.take() {
                                        TokenKind::Word(name) => excluded_traits.push(
                                            self.qualify_name(self.previous_word_spelling(&name)),
                                        ),
                                        token => {
                                            return Err(self.error(&format!(
                                                "Expected excluded trait, got {token:?}"
                                            )))
                                        }
                                    }
                                    if !self.eat(&TokenKind::Comma) {
                                        break;
                                    }
                                },
                                TokenKind::Word(word) if word == "as" => {
                                    while let TokenKind::Word(word) = self.peek() {
                                        match word.as_str() {
                                            "public" => {
                                                visibility = Some(Visibility::Public);
                                                self.take();
                                            }
                                            "protected" => {
                                                visibility = Some(Visibility::Protected);
                                                self.take();
                                            }
                                            "private" => {
                                                visibility = Some(Visibility::Private);
                                                self.take();
                                            }
                                            "final" => {
                                                is_final = true;
                                                self.take();
                                            }
                                            _ => break,
                                        }
                                    }
                                    if let TokenKind::Word(name) = self.peek() {
                                        let fallback = name.clone();
                                        self.take();
                                        alias = Some(self.previous_word_spelling(&fallback));
                                    }
                                    if alias.is_none() && visibility.is_none() && !is_final {
                                        return Err(self.error("Trait alias must specify a name, visibility, or final modifier"));
                                    }
                                }
                                token => {
                                    return Err(self.error(&format!(
                                        "Expected insteadof or as, got {token:?}"
                                    )))
                                }
                            }
                            self.expect(TokenKind::Semi, "Expected ; after trait adaptation")?;
                            nested_trait_adaptations.push(TraitAdaptation {
                                trait_name,
                                method_name,
                                excluded_traits,
                                alias,
                                visibility,
                                is_final,
                            });
                        }
                        continue;
                    }
                    let (mut is_static, mut is_abstract, mut is_final, mut is_readonly) =
                        (false, false, false, false);
                    let mut visibility = Visibility::Public;
                    let mut set_visibility = None;
                    while let TokenKind::Word(word) = self.peek() {
                        if matches!(word.as_str(), "public" | "private" | "protected")
                            && self
                                .tokens
                                .get(self.cursor + 1)
                                .is_some_and(|token| token.kind == TokenKind::Lp)
                            && self
                                .tokens
                                .get(self.cursor + 2)
                                .is_some_and(|token| token.kind == TokenKind::Word("set".into()))
                        {
                            set_visibility = Some(match self.take() {
                                TokenKind::Word(name) if name == "public" => Visibility::Public,
                                TokenKind::Word(name) if name == "protected" => {
                                    Visibility::Protected
                                }
                                _ => Visibility::Private,
                            });
                            self.expect(TokenKind::Lp, "Expected ( before setter visibility")?;
                            self.expect(
                                TokenKind::Word("set".into()),
                                "Expected set in asymmetric visibility",
                            )?;
                            self.expect(TokenKind::Rp, "Expected ) after asymmetric visibility")?;
                            continue;
                        }
                        if matches!(
                            word.as_str(),
                            "public"
                                | "private"
                                | "protected"
                                | "static"
                                | "abstract"
                                | "final"
                                | "readonly"
                        ) {
                            match word.as_str() {
                                "public" => visibility = Visibility::Public,
                                "protected" => visibility = Visibility::Protected,
                                "private" => visibility = Visibility::Private,
                                "static" => is_static = true,
                                "abstract" => is_abstract = true,
                                "final" => is_final = true,
                                "readonly" => is_readonly = true,
                                _ => unreachable!(),
                            }
                            self.take();
                        } else {
                            break;
                        }
                    }
                    if matches!(self.peek(), TokenKind::Word(word) if word == "function") {
                        let mut method = self.function_declaration(
                            is_static,
                            is_abstract,
                            is_final,
                            visibility,
                        )?;
                        method.attributes = member_attributes;
                        methods.push(method);
                    } else if matches!(self.peek(), TokenKind::Word(word) if word == "const") {
                        if is_static || is_abstract || is_readonly {
                            return Err(self.error("Invalid modifier on trait constant"));
                        }
                        self.take();
                        let nullable = self.eat(&TokenKind::Question);
                        let type_name = if nullable
                            || matches!(self.peek(), TokenKind::Word(_))
                                && matches!(
                                    self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                                    Some(
                                        TokenKind::Word(_) | TokenKind::Pipe | TokenKind::Ampersand
                                    )
                                ) {
                            Some(self.parse_union_type(nullable)?)
                        } else {
                            None
                        };
                        let const_name_offset = self.peek_offset();
                        let const_name = match self.take() {
                            TokenKind::Word(_) => self.source_word(const_name_offset),
                            token => {
                                return Err(self.error(&format!(
                                    "Expected trait constant name, got {token:?}"
                                )))
                            }
                        };
                        self.expect(TokenKind::Eq, "Expected = in trait constant")?;
                        let value = self.expression(1)?;
                        self.expect(TokenKind::Semi, "Expected ; after trait constant")?;
                        trait_constants.push(ClassConstantDecl {
                            name: const_name,
                            attributes: member_attributes,
                            value,
                            type_name,
                            visibility,
                            is_final,
                            is_enum_case: false,
                            enum_case_has_value: false,
                        });
                    } else {
                        let nullable = self.eat(&TokenKind::Question);
                        let has_type = (matches!(self.peek(), TokenKind::Word(_))
                            && matches!(
                                self.tokens.get(self.cursor + 1).map(|token| &token.kind),
                                Some(TokenKind::Var(_) | TokenKind::Pipe | TokenKind::Ampersand)
                            ))
                            || matches!(self.peek(), TokenKind::Lp);
                        let type_name = if has_type {
                            Some(self.parse_union_type(nullable)?)
                        } else if nullable {
                            return Err(self.error("Expected property type after ?"));
                        } else {
                            None
                        };
                        let property = match self.take() {
                            TokenKind::Var(name) => name,
                            token => {
                                return Err(self.error(&format!(
                                    "Expected trait property or method, got {token:?}"
                                )))
                            }
                        };
                        let default = if self.eat(&TokenKind::Eq) {
                            Some(self.expression(1)?)
                        } else {
                            None
                        };
                        let (get_hook, set_hook) = if self.peek() == &TokenKind::Lb {
                            self.property_hooks()?
                        } else {
                            (None, None)
                        };
                        if get_hook.is_some() || set_hook.is_some() {
                            if is_static {
                                return Err(self.error(
                                    "Property hooks cannot be declared on static properties",
                                ));
                            }
                        } else {
                            self.expect(TokenKind::Semi, "Expected ; after trait property")?;
                        }
                        properties.push(PropertyDecl {
                            name: property,
                            attributes: member_attributes,
                            default,
                            is_static,
                            is_readonly,
                            visibility,
                            set_visibility,
                            type_name,
                            get_hook,
                            set_hook,
                        });
                    }
                }
                traits.push(TraitDecl {
                    name,
                    attributes: class_attributes,
                    traits: nested_traits,
                    trait_adaptations: nested_trait_adaptations,
                    properties,
                    constants: trait_constants,
                    methods,
                });
            } else {
                statements.push(self.statement()?);
            }
        }
        if inside_namespace_block {
            return Err(self.error("Unterminated namespace block"));
        }
        classes.extend(self.anonymous_classes);
        Ok(Program {
            strict_types: self.strict_types,
            statements,
            functions,
            classes,
            interfaces,
            traits,
            constants,
        })
    }
}
