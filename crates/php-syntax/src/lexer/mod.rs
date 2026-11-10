//! PHP subset lexer that emits tokens with source offsets.

use self::token::{Token, TokenKind};
use crate::diagnostics::format_diagnostic;

pub(super) mod token;

fn utf8_width(first_byte: u8) -> usize {
    match first_byte {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 1,
    }
}

fn scan_digits(bytes: &[u8], cursor: &mut usize, radix: u8) -> bool {
    let start = *cursor;
    let mut previous_was_digit = false;
    while let Some(byte) = bytes.get(*cursor).copied() {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => 255,
        };
        if digit < radix {
            previous_was_digit = true;
            *cursor += 1;
        } else if byte == b'_'
            && previous_was_digit
            && bytes.get(*cursor + 1).is_some_and(|next| {
                let next_digit = match *next {
                    b'0'..=b'9' => *next - b'0',
                    b'a'..=b'f' => *next - b'a' + 10,
                    b'A'..=b'F' => *next - b'A' + 10,
                    _ => 255,
                };
                next_digit < radix
            })
        {
            previous_was_digit = false;
            *cursor += 1;
        } else {
            break;
        }
    }
    *cursor > start && previous_was_digit
}

/// Convert PHP source into tokens while preserving byte offsets for diagnostics.
pub(crate) fn lex(source: &str) -> Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut in_php = false;

    while i < bytes.len() {
        if !in_php {
            let Some(relative) = source[i..].find("<?") else {
                if i < source.len() {
                    tokens.push(Token {
                        kind: TokenKind::InlineHtml(source[i..].to_string()),
                        offset: i,
                    });
                }
                break;
            };
            let open = i + relative;
            if open > i {
                tokens.push(Token {
                    kind: TokenKind::InlineHtml(source[i..open].to_string()),
                    offset: i,
                });
            }
            i = open;
            in_php = true;
        }
        let ch = bytes[i] as char;
        let token_start = i;
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && bytes.get(i + 1) == Some(&b'*') {
            let comment_body_start = i + 2;
            let Some(comment_end) = source[comment_body_start..].find("*/") else {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    "Unterminated block comment",
                ));
            };
            i = comment_body_start + comment_end + 2;
            continue;
        }
        if ch == '#' && bytes.get(i + 1) == Some(&b'[') {
            tokens.push(Token {
                kind: TokenKind::AttributeStart,
                offset: token_start,
            });
            i += 2;
            continue;
        }
        if ch == '#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == '<' && bytes.get(i + 1) == Some(&b'?') {
            let short_echo = bytes.get(i + 2) == Some(&b'=');
            i += 2;
            if short_echo {
                i += 1;
                tokens.push(Token {
                    kind: TokenKind::Word("echo".into()),
                    offset: token_start,
                });
            }
            while !short_echo && i < bytes.len() && (bytes[i] as char).is_whitespace() {
                i += 1;
            }
            if source.get(i..).unwrap_or("").starts_with("php") {
                i += 3;
            }
            in_php = true;
            continue;
        }
        if ch == '?' && bytes.get(i + 1) == Some(&b'>') {
            i += 2;
            tokens.push(Token {
                kind: TokenKind::Semi,
                offset: token_start,
            });
            in_php = false;
            continue;
        }
        if ch == '$' {
            if bytes.get(i + 1) == Some(&b'{') {
                i += 2;
                tokens.push(Token {
                    kind: TokenKind::DynamicExprStart,
                    offset: token_start,
                });
                continue;
            }
            if bytes.get(i + 1) == Some(&b'$') {
                i += 2;
                let start = i;
                while i < bytes.len() && ((bytes[i] as char).is_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                if start == i {
                    return Err(format_diagnostic(
                        source,
                        token_start,
                        "Expected variable name after $$",
                    ));
                }
                tokens.push(Token {
                    kind: TokenKind::DynamicVar(source[start..i].into()),
                    offset: token_start,
                });
                continue;
            }
            i += 1;
            let start = i;
            while i < bytes.len() && ((bytes[i] as char).is_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if start == i {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    "Expected variable name after $",
                ));
            }
            tokens.push(Token {
                kind: TokenKind::Var(source[start..i].into()),
                offset: token_start,
            });
            continue;
        }
        if ch == '\\'
            && bytes
                .get(i + 1)
                .is_some_and(|next| (*next as char).is_alphabetic() || *next == b'_')
        {
            let start = i;
            i += 1;
            while i < bytes.len()
                && ((bytes[i] as char).is_alphanumeric()
                    || bytes[i] == b'_'
                    || (bytes[i] == b'\\'
                        && bytes
                            .get(i + 1)
                            .is_some_and(|next| (*next as char).is_alphabetic() || *next == b'{')))
            {
                i += 1;
            }
            tokens.push(Token {
                kind: TokenKind::Word(source[start..i].to_ascii_lowercase()),
                offset: token_start,
            });
            continue;
        }
        if ch == '.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
            let start = i;
            i += 1;
            if !scan_digits(bytes, &mut i, 10) {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    "Invalid numeric literal",
                ));
            }
            if bytes.get(i).is_some_and(|byte| matches!(byte, b'e' | b'E')) {
                i += 1;
                if bytes.get(i).is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                    i += 1;
                }
                if !scan_digits(bytes, &mut i, 10) {
                    return Err(format_diagnostic(
                        source,
                        token_start,
                        "Invalid numeric exponent",
                    ));
                }
            }
            let spelling = source[start..i].replace('_', "");
            let number = spelling
                .parse()
                .map_err(|_| format_diagnostic(source, token_start, "Invalid number"))?;
            tokens.push(Token {
                kind: TokenKind::Num(number, None),
                offset: token_start,
            });
            continue;
        }
        if ch.is_ascii_digit() {
            let start = i;
            let base = if bytes
                .get(i..i + 2)
                .is_some_and(|prefix| matches!(prefix, b"0x" | b"0X"))
            {
                Some((16, 2))
            } else if bytes
                .get(i..i + 2)
                .is_some_and(|prefix| matches!(prefix, b"0b" | b"0B"))
            {
                Some((2, 2))
            } else if bytes
                .get(i..i + 2)
                .is_some_and(|prefix| matches!(prefix, b"0o" | b"0O"))
            {
                Some((8, 2))
            } else {
                None
            };
            if let Some((radix, prefix_width)) = base {
                i += prefix_width;
                let digits_start = i;
                if !scan_digits(bytes, &mut i, radix) {
                    return Err(format_diagnostic(
                        source,
                        token_start,
                        "Invalid base-prefixed integer literal",
                    ));
                }
                let digits = source[digits_start..i].replace('_', "");
                let value = u64::from_str_radix(&digits, u32::from(radix)).map_err(|_| {
                    format_diagnostic(source, token_start, "Integer literal is out of range")
                })? as i64;
                tokens.push(Token {
                    kind: TokenKind::Num(value as f64, Some(value)),
                    offset: token_start,
                });
                continue;
            }
            if !scan_digits(bytes, &mut i, 10) {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    "Invalid numeric literal",
                ));
            }
            let mut is_float = false;
            if bytes.get(i) == Some(&b'.') {
                is_float = true;
                i += 1;
                scan_digits(bytes, &mut i, 10);
            }
            if bytes.get(i).is_some_and(|byte| matches!(byte, b'e' | b'E')) {
                is_float = true;
                i += 1;
                if bytes.get(i).is_some_and(|byte| matches!(byte, b'+' | b'-')) {
                    i += 1;
                }
                if !scan_digits(bytes, &mut i, 10) {
                    return Err(format_diagnostic(
                        source,
                        token_start,
                        "Invalid numeric exponent",
                    ));
                }
            }
            let spelling = source[start..i].replace('_', "");
            let number = spelling
                .parse()
                .map_err(|_| format_diagnostic(source, token_start, "Invalid number"))?;
            let integer = if is_float {
                None
            } else {
                spelling.parse::<i64>().ok()
            };
            tokens.push(Token {
                kind: TokenKind::Num(number, integer),
                offset: token_start,
            });
            continue;
        }
        if ch == '\'' || ch == '"' {
            let quote = bytes[i];
            i += 1;
            let mut value = String::new();
            let mut interpolation_depth = 0usize;
            let mut expression_quote = None;
            while i < bytes.len() && !(interpolation_depth == 0 && bytes[i] == quote) {
                if interpolation_depth > 0 {
                    if let Some(inner_quote) = expression_quote {
                        if bytes[i] == b'\\' && i + 1 < bytes.len() {
                            value.push_str(&source[i..i + 2]);
                            i += 2;
                        } else if bytes[i] == inner_quote {
                            expression_quote = None;
                            value.push(inner_quote as char);
                            i += 1;
                        } else if (bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{'))
                            || (bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'$'))
                        {
                            interpolation_depth += 1;
                            let width = utf8_width(bytes[i]);
                            value.push_str(source.get(i..i + width).unwrap_or(""));
                            i += width;
                        } else if bytes[i] == b'}' {
                            interpolation_depth -= 1;
                            value.push('}');
                            i += 1;
                        } else {
                            let width = utf8_width(bytes[i]);
                            value.push_str(source.get(i..i + width).unwrap_or(""));
                            i += width;
                        }
                    } else if matches!(bytes[i], b'\'' | b'"') {
                        expression_quote = Some(bytes[i]);
                        value.push(bytes[i] as char);
                        i += 1;
                    } else if bytes[i] == b'{' {
                        interpolation_depth += 1;
                        value.push('{');
                        i += 1;
                    } else if bytes[i] == b'}' {
                        interpolation_depth -= 1;
                        value.push('}');
                        i += 1;
                    } else {
                        let width = utf8_width(bytes[i]);
                        value.push_str(source.get(i..i + width).unwrap_or(""));
                        i += width;
                    }
                    continue;
                }
                if quote == b'"' && bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
                    interpolation_depth = 1;
                    value.push_str("${");
                    i += 2;
                    continue;
                }
                if quote == b'"' && bytes[i] == b'{' && bytes.get(i + 1) == Some(&b'$') {
                    interpolation_depth = 1;
                    value.push('{');
                    i += 1;
                    continue;
                }
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                    let escaped = bytes[i];
                    if quote == b'\'' {
                        match escaped {
                            b'\\' | b'\'' => value.push(escaped as char),
                            _ => {
                                value.push('\\');
                                let width = utf8_width(escaped);
                                value.push_str(source.get(i..i + width).unwrap_or(""));
                                i += width;
                                continue;
                            }
                        }
                        i += 1;
                    } else {
                        match escaped {
                            b'n' => value.push('\n'),
                            b'r' => value.push('\r'),
                            b't' => value.push('\t'),
                            b'v' => value.push('\x0b'),
                            b'e' => value.push('\x1b'),
                            b'f' => value.push('\x0c'),
                            b'\\' => value.push('\\'),
                            b'"' => value.push('"'),
                            b'$' => {
                                value.push('\\');
                                value.push('$');
                            }
                            _ => {
                                value.push('\\');
                                let width = utf8_width(escaped);
                                value.push_str(source.get(i..i + width).unwrap_or(""));
                                i += width;
                                continue;
                            }
                        }
                        i += 1;
                    }
                } else {
                    let width = utf8_width(bytes[i]);
                    value.push_str(source.get(i..i + width).unwrap_or(""));
                    i += width;
                }
            }
            if i == bytes.len() {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    "Unterminated string",
                ));
            }
            i += 1;
            let kind = if quote == b'"' {
                TokenKind::InterpolatedStr(value)
            } else {
                TokenKind::Str(value)
            };
            tokens.push(Token {
                kind,
                offset: token_start,
            });
            continue;
        }

        if bytes.get(i..i + 3) == Some(&b"<<<"[..]) {
            let header_end = source[i..]
                .find('\n')
                .map(|offset| i + offset)
                .ok_or_else(|| {
                    format_diagnostic(source, token_start, "Unterminated heredoc header")
                })?;
            let header = source[i + 3..header_end]
                .trim()
                .trim_end_matches('\r')
                .trim();
            let nowdoc = header.starts_with('\'');
            let label = header.trim_matches(['\'', '"']);
            if label.is_empty()
                || !label.chars().enumerate().all(|(index, ch)| {
                    ch == '_' || ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit())
                })
            {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    "Invalid heredoc label",
                ));
            }
            let body_start = (header_end + 1).min(source.len());
            let mut line_start = body_start;
            let mut closing = None;
            while line_start < source.len() {
                let line_end = source[line_start..]
                    .find('\n')
                    .map(|offset| line_start + offset)
                    .unwrap_or(source.len());
                let line = source[line_start..line_end].trim_end_matches('\r');
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix(label) {
                    if rest.trim().is_empty()
                        || rest
                            .trim_start()
                            .chars()
                            .next()
                            .is_some_and(|ch| matches!(ch, ';' | ',' | ')' | ']' | '}'))
                    {
                        let indentation = &line[..line.len() - line.trim_start().len()];
                        if !indentation.bytes().all(|byte| matches!(byte, b' ' | b'\t')) {
                            return Err(format_diagnostic(
                                source,
                                line_start,
                                "Invalid body indentation level",
                            ));
                        }
                        closing = Some((
                            line_start,
                            line_start + indentation.len() + label.len(),
                            indentation.to_string(),
                        ));
                        break;
                    }
                }
                line_start = line_end + usize::from(line_end < source.len());
            }
            let (body_end, after_end, indentation) = closing
                .ok_or_else(|| format_diagnostic(source, token_start, "Unterminated heredoc"))?;
            let body_end =
                if body_end > body_start && source.as_bytes().get(body_end - 1) == Some(&b'\n') {
                    body_end - 1
                } else {
                    body_end
                };
            let body_end =
                if body_end > body_start && source.as_bytes().get(body_end - 1) == Some(&b'\r') {
                    body_end - 1
                } else {
                    body_end
                };
            let body = &source[body_start..body_end];
            let body = if indentation.is_empty() {
                body.to_string()
            } else {
                let mut lines = Vec::new();
                for line in body.split('\n') {
                    let line = line.strip_suffix('\r').unwrap_or(line);
                    if line.trim().is_empty() {
                        lines.push(String::new());
                        continue;
                    }
                    if !line.starts_with(&indentation) {
                        return Err(format_diagnostic(
                            source,
                            body_start,
                            "Invalid body indentation level",
                        ));
                    }
                    let remainder = &line[indentation.len()..];
                    lines.push(remainder.to_string());
                }
                lines.join("\n")
            };
            let kind = if nowdoc {
                TokenKind::Str(body)
            } else {
                TokenKind::InterpolatedStr(body)
            };
            tokens.push(Token {
                kind,
                offset: token_start,
            });
            i = after_end;
            continue;
        }

        let next = bytes.get(i + 1).copied();
        if ch == '=' && bytes.get(i + 1..i + 3) == Some(&b"=="[..]) {
            tokens.push(Token {
                kind: TokenKind::StrictEq,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '!' && bytes.get(i + 1..i + 3) == Some(&b"=="[..]) {
            tokens.push(Token {
                kind: TokenKind::StrictNotEq,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '?' && bytes.get(i + 1..i + 3) == Some(&b"?="[..]) {
            tokens.push(Token {
                kind: TokenKind::CoalesceEq,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '?' && bytes.get(i + 1..i + 3) == Some(&b"->"[..]) {
            tokens.push(Token {
                kind: TokenKind::NullsafeArrow,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '*' && bytes.get(i + 1..i + 3) == Some(&b"*="[..]) {
            tokens.push(Token {
                kind: TokenKind::PowerEq,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '<' && bytes.get(i + 1..i + 3) == Some(&b"<="[..]) {
            tokens.push(Token {
                kind: TokenKind::ShiftLeftEq,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '>' && bytes.get(i + 1..i + 3) == Some(&b">="[..]) {
            tokens.push(Token {
                kind: TokenKind::ShiftRightEq,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if matches!(ch, '&' | '|' | '^') && bytes.get(i + 1) == Some(&b'=') {
            let kind = match ch {
                '&' => TokenKind::AmpersandEq,
                '|' => TokenKind::PipeEq,
                _ => TokenKind::CaretEq,
            };
            tokens.push(Token {
                kind,
                offset: token_start,
            });
            i += 2;
            continue;
        }
        if ch == '*' && bytes.get(i + 1) == Some(&b'*') {
            tokens.push(Token {
                kind: TokenKind::Power,
                offset: token_start,
            });
            i += 2;
            continue;
        }
        if ch == '<' && bytes.get(i + 1..i + 3) == Some(&b"=>"[..]) {
            tokens.push(Token {
                kind: TokenKind::Spaceship,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        if ch == '<' && bytes.get(i + 1) == Some(&b'<') {
            tokens.push(Token {
                kind: TokenKind::ShiftLeft,
                offset: token_start,
            });
            i += 2;
            continue;
        }
        if ch == '>' && bytes.get(i + 1) == Some(&b'>') {
            tokens.push(Token {
                kind: TokenKind::ShiftRight,
                offset: token_start,
            });
            i += 2;
            continue;
        }
        if ch == '.' && bytes.get(i + 1..i + 3) == Some(&b".."[..]) {
            tokens.push(Token {
                kind: TokenKind::Ellipsis,
                offset: token_start,
            });
            i += 3;
            continue;
        }
        let (token, width) = match (ch, next) {
            ('=', Some(b'=')) => (TokenKind::EqEq, 2),
            ('+', Some(b'=')) => (TokenKind::PlusEq, 2),
            ('-', Some(b'=')) => (TokenKind::MinusEq, 2),
            ('*', Some(b'=')) => (TokenKind::StarEq, 2),
            ('/', Some(b'=')) => (TokenKind::SlashEq, 2),
            ('%', Some(b'=')) => (TokenKind::PercentEq, 2),
            ('.', Some(b'=')) => (TokenKind::DotEq, 2),
            ('=', Some(b'>')) => (TokenKind::Arrow, 2),
            ('-', Some(b'>')) => (TokenKind::ObjectArrow, 2),
            ('+', Some(b'+')) => (TokenKind::PlusPlus, 2),
            ('-', Some(b'-')) => (TokenKind::MinusMinus, 2),
            (':', Some(b':')) => (TokenKind::DoubleColon, 2),
            (':', _) => (TokenKind::Colon, 1),
            ('!', Some(b'=')) => (TokenKind::NotEq, 2),
            ('?', Some(b'?')) => (TokenKind::Coalesce, 2),
            ('!', _) => (TokenKind::Not, 1),
            ('&', Some(b'&')) => (TokenKind::AndAnd, 2),
            ('&', _) => (TokenKind::Ampersand, 1),
            ('|', Some(b'|')) => (TokenKind::OrOr, 2),
            ('|', _) => (TokenKind::Pipe, 1),
            ('^', _) => (TokenKind::Caret, 1),
            ('~', _) => (TokenKind::Tilde, 1),
            ('<', Some(b'=')) => (TokenKind::Le, 2),
            ('>', Some(b'=')) => (TokenKind::Ge, 2),
            ('=', _) => (TokenKind::Eq, 1),
            ('<', _) => (TokenKind::Lt, 1),
            ('>', _) => (TokenKind::Gt, 1),
            ('+', _) => (TokenKind::Plus, 1),
            ('-', _) => (TokenKind::Minus, 1),
            ('*', _) => (TokenKind::Star, 1),
            ('/', _) => (TokenKind::Slash, 1),
            ('%', _) => (TokenKind::Percent, 1),
            ('.', _) => (TokenKind::Dot, 1),
            (';', _) => (TokenKind::Semi, 1),
            ('(', _) => (TokenKind::Lp, 1),
            (')', _) => (TokenKind::Rp, 1),
            ('{', _) => (TokenKind::Lb, 1),
            ('}', _) => (TokenKind::Rb, 1),
            (',', _) => (TokenKind::Comma, 1),
            ('[', _) => (TokenKind::Ls, 1),
            (']', _) => (TokenKind::Rs, 1),
            ('?', _) => (TokenKind::Question, 1),
            ('@', _) => (TokenKind::ErrorSuppress, 1),
            _ if ch.is_alphabetic() || ch == '_' => {
                let start = i;
                i += 1;
                while i < bytes.len()
                    && ((bytes[i] as char).is_alphanumeric()
                        || bytes[i] == b'_'
                        || (bytes[i] == b'\\'
                            && bytes.get(i + 1).is_some_and(|next| {
                                (*next as char).is_alphabetic() || *next == b'{'
                            })))
                {
                    i += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Word(source[start..i].to_ascii_lowercase()),
                    offset: token_start,
                });
                continue;
            }
            _ => {
                return Err(format_diagnostic(
                    source,
                    token_start,
                    &format!("Unexpected character: {ch}"),
                ))
            }
        };
        tokens.push(Token {
            kind: token,
            offset: token_start,
        });
        i += width;
    }

    tokens.push(Token {
        kind: TokenKind::Eof,
        offset: source.len(),
    });
    Ok(tokens)
}
