//! Parser state and shared token-consumption helpers.

use crate::{diagnostics::format_diagnostic, lexer::{lex, token::{Token, TokenKind}}};
use std::collections::HashMap;

mod declarations;
mod arrays;
mod expressions;
mod functions;
mod loops;
mod control;
mod statements;

/// Parser state containing the source tokens and current cursor.
struct Parser<'a> {
    pub(super) source: &'a str,
    pub(super) tokens: Vec<Token>,
    pub(super) cursor: usize,
    /// Active namespace prefix for names parsed in this file.
    pub(super) namespace: Option<String>,
    /// Imported class aliases in the current file namespace.
    pub(super) class_imports: HashMap<String, String>,
    /// Imported function aliases in the current file namespace.
    pub(super) function_imports: HashMap<String, String>,
    /// Imported constant aliases in the current file namespace (case-sensitive).
    pub(super) constant_imports: HashMap<String, String>,
    /// Class declarations created by `new class` expressions in this source unit.
    pub(super) anonymous_classes: Vec<crate::ast::ClassDecl>,
    pub(super) next_anonymous_class_id: usize,
    /// Minimum precedence rank for PHP's word-form logical operators while parsing a right operand.
    pub(super) word_operator_floor: u8,
    /// File-level scalar type checking mode set by `declare(strict_types=...)`.
    pub(super) strict_types: bool,
}

impl Parser<'_> {
    /// Inspect the current token without advancing the parser.
    fn peek(&self) -> &TokenKind { &self.tokens[self.cursor].kind }

    fn peek_offset(&self) -> usize { self.tokens[self.cursor].offset }

    /// Parse a nested expression with the normal word-operator precedence floor.
    pub(super) fn grouped_expression(&mut self, min_precedence: u8) -> Result<crate::ast::Expr, String> {
        let previous_floor = self.word_operator_floor;
        self.word_operator_floor = 1;
        let expression = self.expression(min_precedence);
        self.word_operator_floor = previous_floor;
        expression
    }

    /// Recover an identifier's original spelling (the lexer lowercases words for keyword matching).
    fn source_word(&self, offset: usize) -> String {
        let tail = self.source.get(offset..).unwrap_or_default();
        let length = tail.bytes().take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'\\').count();
        tail.get(..length).unwrap_or_default().to_string()
    }

    /// Consume the current token and advance, clamping at the EOF token.
    fn take(&mut self) -> TokenKind {
        let token = self.tokens[self.cursor].kind.clone();
        self.cursor += 1;
        token
    }

    /// Consume a token only when it matches the requested kind.
    fn eat(&mut self, expected: &TokenKind) -> bool {
        if self.peek() == expected {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    /// Format a parser diagnostic at the current token position.
    fn error(&self, message: &str) -> String {
        let offset = self.tokens.get(self.cursor).or_else(|| self.tokens.last()).map(|token| token.offset).unwrap_or(0);
        format_diagnostic(self.source, offset, message)
    }

    /// Require a token kind or return a positioned parser error.
    fn expect(&mut self, expected: TokenKind, message: &str) -> Result<(), String> {
        if self.eat(&expected) { Ok(()) } else { Err(self.error(message)) }
    }

    /// Prefix an unqualified class/function name with the active namespace.
    pub(super) fn qualify_name(&self, name: String) -> String {
        self.qualify_imported_name(name, &self.class_imports)
    }

    /// Prefix a declaration name with the namespace without applying import aliases.
    pub(super) fn qualify_declaration_name(&self, name: String) -> String {
        self.namespace.as_ref().map_or(name.clone(), |namespace| format!("{namespace}\\{name}"))
    }

    /// Recover identifier spelling for metadata whose PHP reflection API is case-preserving.
    pub(super) fn previous_word_spelling(&self, fallback: &str) -> String {
        let Some(token) = self.cursor.checked_sub(1).and_then(|index| self.tokens.get(index)) else { return fallback.to_string(); };
        let source = &self.source[token.offset..];
        let spelling = source.chars().take_while(|character| character.is_alphanumeric() || matches!(*character, '_' | '\\')).collect::<String>();
        if spelling.is_empty() { fallback.to_string() } else { spelling }
    }

    /// Resolve a function reference using function imports and the active namespace.
    pub(super) fn qualify_function_name(&self, name: String) -> String {
        self.qualify_imported_name(name, &self.function_imports)
    }

    /// Prefix unqualified constant references with the active namespace.
    pub(super) fn qualify_constant_name(&self, name: String) -> String {
        if name.starts_with('\\') { return name.trim_start_matches('\\').to_string(); }
        let mut segments = name.splitn(2, '\\');
        let first = segments.next().unwrap_or_default();
        if let Some(imported) = self.constant_imports.get(first) {
            return match segments.next() { Some(suffix) => format!("{imported}\\{suffix}"), None => imported.clone() };
        }
        self.namespace.as_ref().map_or(name.clone(), |namespace| format!("{namespace}\\{name}"))
    }

    fn qualify_imported_name(&self, name: String, imports: &HashMap<String, String>) -> String {
        if name.starts_with('\\') { return name.trim_start_matches('\\').to_string(); }
        let mut segments = name.splitn(2, '\\');
        let first = segments.next().unwrap_or_default();
        let lowered_first = first.to_ascii_lowercase();
        if let Some(imported) = imports.get(first).or_else(|| imports.get(&lowered_first)) {
            return match segments.next() {
                Some(suffix) => format!("{imported}\\{suffix}"),
                None => imported.clone(),
            };
        }
        self.namespace.as_ref().map_or(name.clone(), |namespace| format!("{namespace}\\{name}"))
    }

    /// Qualify class types while leaving PHP builtin and contextual types unchanged.
    pub(super) fn qualify_type(&self, name: String) -> String {
        let nullable = name.starts_with('?');
        let bare = name.trim_start_matches('?');
        if bare.contains('|') || bare.contains('&') {
            let members = bare.split('|').map(|union_member| union_member.split('&').map(|member| self.qualify_type(member.to_string())).collect::<Vec<_>>().join("&")).collect::<Vec<_>>().join("|");
            return if nullable { format!("?{members}") } else { members };
        }
        if matches!(bare.to_ascii_lowercase().as_str(), "int" | "integer" | "float" | "double" | "string" | "bool" | "boolean" | "array" | "iterable" | "object" | "mixed" | "void" | "never" | "null" | "false" | "true" | "self" | "parent" | "static") {
            return name;
        }
        let qualified = self.qualify_name(bare.to_string());
        if nullable { format!("?{qualified}") } else { qualified }
    }

}

/// Parse source text into the syntax tree, reporting errors with source locations.
pub fn parse(source: &str) -> Result<crate::Program, String> {
    parse_with_anonymous_class_id(source, 0)
}

/// Parse a source unit while continuing the anonymous-class sequence used by an enclosing compilation.
pub fn parse_with_anonymous_class_id(source: &str, first_anonymous_class_id: usize) -> Result<crate::Program, String> {
    Parser { source, tokens: lex(source)?, cursor: 0, namespace: None, class_imports: HashMap::new(), function_imports: HashMap::new(), constant_imports: HashMap::new(), anonymous_classes: Vec::new(), next_anonymous_class_id: first_anonymous_class_id, word_operator_floor: 1, strict_types: false }.program()
}
