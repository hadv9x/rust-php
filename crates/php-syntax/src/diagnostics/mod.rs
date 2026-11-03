//! Source-position formatting for lexer and parser errors.

pub(crate) fn format_diagnostic(source: &str, offset: usize, message: &str) -> String {
    let mut safe_offset = offset.min(source.len());
    while !source.is_char_boundary(safe_offset) { safe_offset -= 1; }
    let prefix = &source[..safe_offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    let line_text = source.lines().nth(line - 1).unwrap_or("");
    format!("{message} at line {line}, column {column}\n{line_text}\n{}^", " ".repeat(column - 1))
}
