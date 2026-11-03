//! Token kinds and source offsets produced by the lexer.

#[derive(Clone, Debug, PartialEq)]
/// Token categories emitted by the lexer and consumed by the parser.
pub(crate) enum TokenKind {
    Word(String), Var(String), DynamicVar(String), DynamicExprStart, InlineHtml(String), Num(f64, Option<i64>), Str(String),
    Plus, Minus, PlusPlus, MinusMinus, Star, Power, Slash, Percent, Dot, Eq, PlusEq, MinusEq, StarEq, PowerEq, SlashEq, PercentEq, DotEq, AmpersandEq, PipeEq, CaretEq, ShiftLeftEq, ShiftRightEq, CoalesceEq, EqEq, StrictEq, Arrow, ObjectArrow, NullsafeArrow, DoubleColon, Ampersand, Pipe, Caret, ShiftLeft, ShiftRight, Tilde, Not, NotEq, StrictNotEq, AndAnd, OrOr, Lt, Le, Gt, Ge, Spaceship,
    Semi, Colon, Lp, Rp, Lb, Rb, Comma, Ls, Rs, AttributeStart, Question, Coalesce, ErrorSuppress, Ellipsis, Eof,
}

#[derive(Clone, Debug, PartialEq)]
/// A token kind paired with its byte offset in the source text.
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    pub(crate) offset: usize,
}
