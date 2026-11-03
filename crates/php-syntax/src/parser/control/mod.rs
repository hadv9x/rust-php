//! Parsing for switch statements and case/default arms.

use crate::{ast::{Stmt, SwitchCase}, lexer::token::TokenKind};
use super::Parser;

impl Parser<'_> {
    /// Parse a switch body after the `switch` keyword has been consumed.
    pub(in crate::parser) fn switch_statement(&mut self) -> Result<Stmt, String> {
        self.expect(TokenKind::Lp, "Expected ( after switch")?;
        let value = self.expression(0)?;
        self.expect(TokenKind::Rp, "Expected ) after switch value")?;
        let alternate_syntax = if self.eat(&TokenKind::Lb) {
            false
        } else if self.eat(&TokenKind::Colon) {
            true
        } else {
            return Err(self.error("Expected { or : before switch cases"));
        };

        let mut cases = Vec::new();
        let mut has_default = false;
        loop {
            let finished = if alternate_syntax {
                matches!(self.peek(), TokenKind::Word(word) if word == "endswitch")
            } else {
                self.eat(&TokenKind::Rb)
            };
            if finished { break; }
            if self.peek() == &TokenKind::Eof { return Err(self.error("Unterminated switch statement")); }
            let condition = match self.take() {
                TokenKind::Word(word) if word == "case" => Some(self.expression(0)?),
                TokenKind::Word(word) if word == "default" => {
                    if has_default { return Err(self.error("Switch statement may contain only one default arm")); }
                    has_default = true;
                    None
                }
                token => return Err(self.error(&format!("Expected case or default, got {token:?}"))),
            };
            self.expect(TokenKind::Colon, "Expected : after switch case")?;
            let braced_case_body = !alternate_syntax && self.eat(&TokenKind::Lb);

            let mut body = Vec::new();
            while !(alternate_syntax && matches!(self.peek(), TokenKind::Word(word) if word == "endswitch"))
                && !(!alternate_syntax && !braced_case_body && matches!(self.peek(), TokenKind::Rb))
                && !(braced_case_body && matches!(self.peek(), TokenKind::Rb))
                && !matches!(self.peek(), TokenKind::Word(word) if word == "case" || word == "default")
            {
                if self.peek() == &TokenKind::Eof { return Err(self.error("Unterminated switch statement")); }
                body.push(self.statement()?);
            }
            if braced_case_body { self.expect(TokenKind::Rb, "Expected } after braced switch case")?; }
            cases.push(SwitchCase { condition, body });
        }
        if alternate_syntax {
            self.expect(TokenKind::Word("endswitch".into()), "Expected endswitch")?;
            self.expect(TokenKind::Semi, "Expected ; after endswitch")?;
        }
        Ok(Stmt::Switch { value, cases })
    }
}
