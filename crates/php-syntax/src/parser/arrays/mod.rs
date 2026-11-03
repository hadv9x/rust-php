//! Parsing for PHP array literals.

use crate::{ast::{ArrayEntry, Expr}, lexer::token::TokenKind};
use super::Parser;

impl Parser<'_> {
    /// Parse keyed and unkeyed array entries up to the supplied closing token.
    pub(in crate::parser) fn array_literal(&mut self, closing: TokenKind) -> Result<Expr, String> {
        let mut items = Vec::new();
        if self.eat(&closing) { return Ok(Expr::Array(items)); }

        loop {
            if self.eat(&TokenKind::Ellipsis) {
                if self.peek() == &TokenKind::Ampersand { return Err(self.error("Array unpacking cannot use a reference marker")); }
                items.push(ArrayEntry { key: None, value: self.grouped_expression(1)?, unpack: true, by_reference: false });
                if self.eat(&closing) { break; }
                self.expect(TokenKind::Comma, "Expected , or closing bracket after unpacked array")?;
                if self.eat(&closing) { break; }
                continue;
            }
            // Parsing the key first lets `=>` distinguish keyed from packed entries.
            let packed_reference = self.eat(&TokenKind::Ampersand);
            let first = self.grouped_expression(1)?;
            let entry = if self.eat(&TokenKind::Arrow) {
                if packed_reference { return Err(self.error("A keyed array entry's key cannot be a reference")); }
                let by_reference = self.eat(&TokenKind::Ampersand);
                let value = self.grouped_expression(1)?;
                if by_reference && !is_reference_source(&value) {
                    return Err(self.error("Array reference value must be a variable or array element"));
                }
                ArrayEntry { key: Some(first), value, unpack: false, by_reference }
            } else {
                if packed_reference && !is_reference_source(&first) {
                    return Err(self.error("Array reference value must be a variable or array element"));
                }
                ArrayEntry { key: None, value: first, unpack: false, by_reference: packed_reference }
            };
            items.push(entry);

            if self.eat(&closing) { break; }
            self.expect(TokenKind::Comma, "Expected , or closing bracket in array")?;
            if self.eat(&closing) { break; }
        }
        Ok(Expr::Array(items))
    }
}

fn is_reference_source(expression: &Expr) -> bool {
    match expression {
        Expr::Variable(_) | Expr::Property(_, _) => true,
        Expr::Index(array, _) => is_reference_source(array),
        _ => false,
    }
}
