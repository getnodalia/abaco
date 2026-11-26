// SPDX-License-Identifier: MIT OR Apache-2.0
//! Colouring for the cell editor: which part of a line is a number, a unit, a function, a
//! comment. It reads text and returns ranges; the window picks the colours.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Space,
    Comment,
    Number,
    /// A name that is a unit where a unit is expected (after a number, after `->`).
    Unit,
    /// A built-in function, or a name followed by `(`.
    Function,
    Keyword,
    Name,
    String,
    Punct,
}

/// The byte offset after the longest run of chars from `from` that satisfy `keep`.
fn run(text: &str, from: usize, keep: impl Fn(char) -> bool) -> usize {
    let rest = text.get(from..).unwrap_or("");
    rest.char_indices()
        .find(|&(_, c)| !keep(c))
        .map_or(text.len(), |(i, _)| from + i)
}

fn string_end(text: &str, start: usize) -> usize {
    // Up to the closing quote, or the end of the line when there is none.
    let body = start + 1;
    let rest = text.get(body..).unwrap_or("");
    for (i, c) in rest.char_indices() {
        if c == '"' {
            return body + i + 1;
        }
        if c == '\n' {
            return body + i;
        }
    }
    text.len()
}

fn number_end(text: &str, start: usize) -> usize {
    let mut end = run(text, start, |c| c.is_ascii_digit() || c == '.' || c == '_');
    // An exponent: `1.5e-3`, but `1e` followed by a name is `1*e`.
    let rest = text.get(end..).unwrap_or("");
    let mut it = rest.chars();
    if matches!(it.next(), Some('e' | 'E')) {
        let mut at = end + 1;
        if matches!(it.clone().next(), Some('+' | '-')) {
            it.next();
            at += 1;
        }
        if it.next().is_some_and(|d| d.is_ascii_digit()) {
            end = run(text, at, |c| c.is_ascii_digit() || c == '_');
        }
    }
    end
}

/// The tokens of `text`, covering it completely and in order, split at char boundaries.
pub fn tokens(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut out: Vec<(Range<usize>, Token)> = Vec::new();
    let mut at = 0;
    // A unit may follow: after a number, a unit, `->`, `to`, or an operator inside a unit.
    let mut expect_unit = false;
    let mut after_unit = false;
    while let Some(c) = text.get(at..).and_then(|r| r.chars().next()) {
        let start = at;
        let next_char = text
            .get(start + c.len_utf8()..)
            .and_then(|r| r.chars().next());
        let (end, kind) = if c == '#' {
            (run(text, start, |n| n != '\n'), Token::Comment)
        } else if c.is_whitespace() {
            (run(text, start, char::is_whitespace), Token::Space)
        } else if c == '"' {
            (string_end(text, start), Token::String)
        } else if c.is_ascii_digit() || (c == '.' && next_char.is_some_and(|n| n.is_ascii_digit()))
        {
            (number_end(text, start), Token::Number)
        } else if c.is_alphabetic() || c == '_' || c == '°' {
            let end = run(text, start, |n| n.is_alphanumeric() || n == '_' || n == '°');
            let word = text.get(start..end).unwrap_or("");
            let call = text.get(end..).is_some_and(|r| r.starts_with('('));
            let kind = if abaco_expr::KEYWORDS.contains(&word) {
                Token::Keyword
            } else if expect_unit && nodalia_units::is_unit_name(word) {
                Token::Unit
            } else if abaco_expr::is_builtin(word) || call {
                Token::Function
            } else {
                Token::Name
            };
            (end, kind)
        } else if c == '-' && next_char == Some('>') {
            (start + 2, Token::Punct)
        } else {
            (start + c.len_utf8(), Token::Punct)
        };
        at = end.max(start + c.len_utf8());
        let word = text.get(start..at).unwrap_or("");
        match kind {
            Token::Space | Token::Comment => {}
            Token::Number => {
                expect_unit = true;
                after_unit = false;
            }
            Token::Unit => {
                expect_unit = false;
                after_unit = true;
            }
            Token::Punct if word == "->" => {
                expect_unit = true;
                after_unit = false;
            }
            Token::Keyword if word == "to" => {
                expect_unit = true;
                after_unit = false;
            }
            Token::Punct if matches!(word, "*" | "/" | "^") && after_unit => {
                expect_unit = true;
            }
            _ => {
                expect_unit = false;
                after_unit = false;
            }
        }
        out.push((start..at, kind));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, Token)> {
        tokens(text)
            .into_iter()
            .filter(|(_, k)| *k != Token::Space)
            .map(|(r, k)| (&text[r], k))
            .collect()
    }

    #[test]
    fn numbers_followed_by_units_and_conversions() {
        assert_eq!(
            kinds("10 kg * 9.81 m/s^2 -> N"),
            [
                ("10", Token::Number),
                ("kg", Token::Unit),
                ("*", Token::Punct),
                ("9.81", Token::Number),
                ("m", Token::Unit),
                ("/", Token::Punct),
                ("s", Token::Unit),
                ("^", Token::Punct),
                ("2", Token::Number),
                ("->", Token::Punct),
                ("N", Token::Unit),
            ]
        );
    }

    #[test]
    fn a_variable_called_like_a_unit_is_not_coloured_as_one() {
        let k = kinds("h = 80 mm");
        assert_eq!(k[0], ("h", Token::Name));
        assert_eq!(k[3], ("mm", Token::Unit));
    }

    #[test]
    fn functions_keywords_and_comments() {
        let k = kinds("T(len) = sin(x) # a note\nx to m");
        assert_eq!(k[0], ("T", Token::Function));
        assert!(k.contains(&("sin", Token::Function)));
        assert!(k.contains(&("# a note", Token::Comment)));
        assert!(k.contains(&("to", Token::Keyword)));
        assert!(k.contains(&("m", Token::Unit)));
    }

    #[test]
    fn exponents_and_strings() {
        assert_eq!(kinds("1.5e-3")[0], ("1.5e-3", Token::Number));
        assert_eq!(kinds("1e")[0], ("1", Token::Number));
        assert_eq!(kinds("\"hi there\"")[0], ("\"hi there\"", Token::String));
        assert_eq!(kinds("\"open")[0].1, Token::String);
    }

    #[test]
    fn tokens_cover_the_text_exactly_whatever_it_holds() {
        for text in [
            "",
            "a",
            "α = 3 µm  # ünï",
            "1 +\n2 *\n(",
            "\"\n#\n->->- >",
            "😀 = 1",
            "1e+",
            ".5.5.5",
            "°C",
        ] {
            let toks = tokens(text);
            let mut at = 0;
            for (range, _) in &toks {
                assert_eq!(range.start, at, "gap in {text:?}");
                assert!(text.is_char_boundary(range.start) && text.is_char_boundary(range.end));
                assert!(range.end > range.start);
                at = range.end;
            }
            assert_eq!(at, text.len(), "{text:?}");
        }
    }
}
