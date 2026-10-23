//! Parser for Spot's LTL syntax, restricted to the TLSF operator set.
//!
//! Precedence follows Spot, loosest first: `->` and `<->` (right-associative,
//! one level), `|`, `&`, then `U`, `W`, `R` (right-associative, one level),
//! then the unary operators. SPIN's spellings (`[]`, `<>`, `V`, `&&`, `||`)
//! and nuXmv's constants (`TRUE`, `FALSE`) are accepted too, so the output of
//! every printer in this module parses back.

use std::fmt;
use std::str::FromStr;

use super::{BinaryOp, Formula, UnaryOp};

/// Why a string is not a formula, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Byte offset into the input.
    pub offset: usize,
    /// What was wrong there.
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Formula {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let tokens = lex(text)?;
        let mut parser = Parser {
            tokens,
            position: 0,
            end: text.len(),
        };
        let formula = parser.expression()?;
        match parser.peek() {
            None => Ok(formula),
            Some((offset, token)) => Err(ParseError {
                offset,
                message: format!("unexpected {token:?} after a complete formula"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    True,
    False,
    Atom(String),
    Unary(UnaryOp),
    Binary(BinaryOp),
    LParen,
    RParen,
}

const SYMBOLS: &[(&str, Token)] = &[
    ("<-->", Token::Binary(BinaryOp::Iff)),
    ("<->", Token::Binary(BinaryOp::Iff)),
    ("<=>", Token::Binary(BinaryOp::Iff)),
    ("<>", Token::Unary(UnaryOp::Eventually)),
    ("[]", Token::Unary(UnaryOp::Globally)),
    ("-->", Token::Binary(BinaryOp::Implies)),
    ("->", Token::Binary(BinaryOp::Implies)),
    ("=>", Token::Binary(BinaryOp::Implies)),
    ("&&", Token::Binary(BinaryOp::And)),
    ("/\\", Token::Binary(BinaryOp::And)),
    ("&", Token::Binary(BinaryOp::And)),
    ("||", Token::Binary(BinaryOp::Or)),
    ("\\/", Token::Binary(BinaryOp::Or)),
    ("|", Token::Binary(BinaryOp::Or)),
    ("!", Token::Unary(UnaryOp::Not)),
    ("~", Token::Unary(UnaryOp::Not)),
    ("(", Token::LParen),
    (")", Token::RParen),
];

fn is_identifier_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_identifier_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn lex(text: &str) -> Result<Vec<(usize, Token)>, ParseError> {
    let error = |offset: usize, message: String| ParseError { offset, message };
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let rest = &text[offset..];
        let c = rest.chars().next().expect("offset is inside the text");
        if c.is_whitespace() {
            offset += c.len_utf8();
            continue;
        }
        if let Some((symbol, token)) = SYMBOLS.iter().find(|(s, _)| rest.starts_with(s)) {
            tokens.push((offset, token.clone()));
            offset += symbol.len();
            continue;
        }
        if c == '"' {
            let close = rest[1..]
                .find('"')
                .ok_or_else(|| error(offset, "unterminated quoted atom".into()))?;
            tokens.push((offset, Token::Atom(rest[1..1 + close].to_string())));
            offset += close + 2;
            continue;
        }
        if c.is_ascii_digit() {
            let len = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let token = match &rest[..len] {
                "0" => Token::False,
                "1" => Token::True,
                other => return Err(error(offset, format!("unexpected number {other:?}"))),
            };
            tokens.push((offset, token));
            offset += len;
            continue;
        }
        if is_identifier_start(c) {
            let len = rest
                .find(|c: char| !is_identifier_char(c))
                .unwrap_or(rest.len());
            lex_word(&rest[..len], offset, &mut tokens)?;
            offset += len;
            continue;
        }
        return Err(error(offset, format!("unexpected character {c:?}")));
    }
    Ok(tokens)
}

fn lex_word(word: &str, offset: usize, tokens: &mut Vec<(usize, Token)>) -> Result<(), ParseError> {
    let token = match word {
        "true" | "TRUE" => Token::True,
        "false" | "FALSE" => Token::False,
        "U" => Token::Binary(BinaryOp::Until),
        "W" => Token::Binary(BinaryOp::WeakUntil),
        "R" | "V" => Token::Binary(BinaryOp::Release),
        "M" | "xor" => {
            return Err(ParseError {
                offset,
                message: format!("operator {word:?} is outside the TLSF operator set"),
            })
        }
        _ => {
            // Spot reads a word that opens with F, G or X as that run of
            // unary operators applied to the rest: `GFa` is G(F(a)) and
            // `Grant` is G(rant). An atom of that shape must be quoted.
            let prefix = word
                .find(|c| !matches!(c, 'F' | 'G' | 'X'))
                .unwrap_or(word.len());
            if prefix == 0 {
                Token::Atom(word.to_string())
            } else {
                for (i, c) in word[..prefix].char_indices() {
                    let op = match c {
                        'F' => UnaryOp::Eventually,
                        'G' => UnaryOp::Globally,
                        _ => UnaryOp::Next,
                    };
                    tokens.push((offset + i, Token::Unary(op)));
                }
                let remainder = &word[prefix..];
                if remainder.is_empty() {
                    return Ok(());
                }
                let token = match remainder {
                    "true" | "TRUE" => Token::True,
                    "false" | "FALSE" => Token::False,
                    _ => Token::Atom(remainder.to_string()),
                };
                tokens.push((offset + prefix, token));
                return Ok(());
            }
        }
    };
    tokens.push((offset, token));
    Ok(())
}

struct Parser {
    tokens: Vec<(usize, Token)>,
    position: usize,
    end: usize,
}

impl Parser {
    fn peek(&self) -> Option<(usize, &Token)> {
        self.tokens.get(self.position).map(|(o, t)| (*o, t))
    }

    fn peek_binary(&self, accept: &[BinaryOp]) -> Option<BinaryOp> {
        match self.peek() {
            Some((_, Token::Binary(op))) if accept.contains(op) => Some(*op),
            _ => None,
        }
    }

    fn expression(&mut self) -> Result<Formula, ParseError> {
        let lhs = self.disjunction()?;
        if let Some(op) = self.peek_binary(&[BinaryOp::Implies, BinaryOp::Iff]) {
            self.position += 1;
            let rhs = self.expression()?;
            return Ok(Formula::binary(op, lhs, rhs));
        }
        Ok(lhs)
    }

    fn disjunction(&mut self) -> Result<Formula, ParseError> {
        let mut lhs = self.conjunction()?;
        while self.peek_binary(&[BinaryOp::Or]).is_some() {
            self.position += 1;
            let rhs = self.conjunction()?;
            lhs = Formula::binary(BinaryOp::Or, lhs, rhs);
        }
        Ok(lhs)
    }

    fn conjunction(&mut self) -> Result<Formula, ParseError> {
        let mut lhs = self.temporal()?;
        while self.peek_binary(&[BinaryOp::And]).is_some() {
            self.position += 1;
            let rhs = self.temporal()?;
            lhs = Formula::binary(BinaryOp::And, lhs, rhs);
        }
        Ok(lhs)
    }

    fn temporal(&mut self) -> Result<Formula, ParseError> {
        let lhs = self.unary()?;
        let ops = [BinaryOp::Until, BinaryOp::WeakUntil, BinaryOp::Release];
        if let Some(op) = self.peek_binary(&ops) {
            self.position += 1;
            let rhs = self.temporal()?;
            return Ok(Formula::binary(op, lhs, rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<Formula, ParseError> {
        let Some((offset, token)) = self.peek() else {
            return Err(ParseError {
                offset: self.end,
                message: "unexpected end of formula".into(),
            });
        };
        let token = token.clone();
        self.position += 1;
        match token {
            Token::True => Ok(Formula::tt()),
            Token::False => Ok(Formula::ff()),
            Token::Atom(name) => Ok(Formula::atom(name)),
            Token::Unary(op) => Ok(Formula::unary(op, self.unary()?)),
            Token::LParen => {
                let inner = self.expression()?;
                match self.peek() {
                    Some((_, Token::RParen)) => {
                        self.position += 1;
                        Ok(inner)
                    }
                    _ => Err(ParseError {
                        offset,
                        message: "unclosed parenthesis".into(),
                    }),
                }
            }
            other => Err(ParseError {
                offset,
                message: format!("expected an operand, found {other:?}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ltl::tests::f;
    use crate::ltl::{BinaryOp::*, Formula, UnaryOp::*};

    fn a(name: &str) -> Formula {
        Formula::atom(name)
    }

    #[test]
    fn constants_in_every_spelling() {
        for text in ["true", "TRUE", "1"] {
            assert!(f(text).is_true(), "{text}");
        }
        for text in ["false", "FALSE", "0"] {
            assert!(f(text).is_false(), "{text}");
        }
        assert_eq!(f("\"true\""), a("true"));
    }

    #[test]
    fn precedence_matches_spot() {
        let b_until_c = Formula::binary(Until, a("b"), a("c"));
        assert_eq!(f("a & b U c"), Formula::binary(And, a("a"), b_until_c));
        let b_and_c = Formula::binary(And, a("b"), a("c"));
        assert_eq!(f("a | b & c"), Formula::binary(Or, a("a"), b_and_c));
        let b_or_c = Formula::binary(Or, a("b"), a("c"));
        assert_eq!(f("a -> b | c"), Formula::binary(Implies, a("a"), b_or_c));
        let fa = Formula::unary(Eventually, a("a"));
        assert_eq!(f("F a U b"), Formula::binary(Until, fa, a("b")));
        let not_a = Formula::unary(Not, a("a"));
        assert_eq!(f("!a U b"), Formula::binary(Until, not_a, a("b")));
    }

    #[test]
    fn associativity_matches_spot() {
        for (text, outer, inner) in [
            ("a U b U c", Until, Until),
            ("a W b R c", WeakUntil, Release),
            ("a -> b -> c", Implies, Implies),
            ("a <-> b -> c", Iff, Implies),
        ] {
            let right = Formula::binary(inner, a("b"), a("c"));
            assert_eq!(f(text), Formula::binary(outer, a("a"), right), "{text}");
        }
        for (text, op) in [("a & b & c", And), ("a | b | c", Or)] {
            let left = Formula::binary(op, a("a"), a("b"));
            assert_eq!(f(text), Formula::binary(op, left, a("c")), "{text}");
        }
    }

    #[test]
    fn letter_operators_split_like_spot() {
        let fa = Formula::unary(Eventually, a("a"));
        assert_eq!(f("GFa"), Formula::unary(Globally, fa));
        assert_eq!(f("Grant"), Formula::unary(Globally, a("rant")));
        assert_eq!(f("FOO"), Formula::unary(Eventually, a("OO")));
        let fab = Formula::unary(Eventually, a("ab"));
        assert_eq!(f("XFab"), Formula::unary(Next, fab));
        assert_eq!(f("Ftrue"), Formula::unary(Eventually, Formula::tt()));
        assert_eq!(f("\"Grant\""), a("Grant"));
        assert_eq!(f("Uab"), a("Uab"));
        assert_eq!(f("aUb"), a("aUb"));
        assert!("FG".parse::<Formula>().is_err());
    }

    #[test]
    fn spin_and_nuxmv_spellings() {
        assert_eq!(f("[]<>a"), f("G F a"));
        assert_eq!(f("a V b"), f("a R b"));
        assert_eq!(f("a && b || !c"), f("a & b | !c"));
        assert_eq!(f("a /\\ b \\/ c => d <=> e"), f("a & b | c -> d <-> e"));
        assert_eq!(f("~a"), f("!a"));
        assert_eq!(f("TRUE -> FALSE"), f("true -> false"));
    }

    #[test]
    fn quoted_atoms_may_hold_any_text() {
        assert_eq!(f("F\"a b\""), Formula::unary(Eventually, a("a b")));
        assert_eq!(f("\"U\" U \"W\""), Formula::binary(Until, a("U"), a("W")));
    }

    #[test]
    fn errors_carry_an_offset() {
        let err = "a & (b | c".parse::<Formula>().unwrap_err();
        assert_eq!(err.offset, 4);
        let err = "a &".parse::<Formula>().unwrap_err();
        assert_eq!(err.offset, 3);
        for bad in ["a b", "a xor b", "a M b", "a ^ b", "\"open", "", "12"] {
            assert!(bad.parse::<Formula>().is_err(), "{bad:?}");
        }
    }
}
