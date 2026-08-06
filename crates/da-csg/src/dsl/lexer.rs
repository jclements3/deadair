//! Tokenizer for the vali modeling language (`.vim` scripts).
//!
//! Comments start with `#` or, VimL-style, a `"` at the start of a statement.
//! A `"` elsewhere (e.g. a call argument like `rotate("z", 90)`) is a string
//! literal. Newlines separate statements, but are suppressed inside parentheses
//! so calls can span lines. Numbers, identifiers, strings, and the single-char
//! symbols `+ - * / , = . ( )` are the whole vocabulary.

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Sym(char),
    Newline,
    Eof,
}

pub fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let mut paren_depth = 0i32;
    // True at the start of a logical statement — where a `"` means a VimL
    // comment rather than a string literal. Reset by any emitted token.
    let mut at_line_start = true;

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' => i += 1,
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            // VimL-style comment: a `"` that opens a statement runs to EOL.
            '"' if at_line_start => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '\n' => {
                if paren_depth <= 0 {
                    // Collapse runs of blank lines into a single separator.
                    if !matches!(out.last(), Some(Tok::Newline) | None) {
                        out.push(Tok::Newline);
                    }
                    at_line_start = true;
                }
                i += 1;
            }
            '(' => {
                paren_depth += 1;
                out.push(Tok::Sym('('));
                at_line_start = false;
                i += 1;
            }
            ')' => {
                paren_depth -= 1;
                out.push(Tok::Sym(')'));
                at_line_start = false;
                i += 1;
            }
            '+' | '-' | '*' | '/' | ',' | '=' | '.' => {
                // A '.' that is part of a number (e.g. ".5") is handled below.
                if c == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit() {
                    let (tok, ni) = lex_number(&chars, i)?;
                    out.push(tok);
                    i = ni;
                } else {
                    out.push(Tok::Sym(c));
                    i += 1;
                }
                at_line_start = false;
            }
            '"' => {
                let mut s = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    s.push(chars[i]);
                    i += 1;
                }
                if i >= chars.len() {
                    return Err("unterminated string literal".into());
                }
                i += 1; // closing quote
                out.push(Tok::Str(s));
                at_line_start = false;
            }
            c if c.is_ascii_digit() => {
                let (tok, ni) = lex_number(&chars, i)?;
                out.push(tok);
                i = ni;
                at_line_start = false;
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Tok::Ident(chars[start..i].iter().collect()));
                at_line_start = false;
            }
            other => return Err(format!("unexpected character '{other}'")),
        }
    }
    out.push(Tok::Eof);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viml_comment_at_line_start() {
        // A `"` opening a line is a comment; the `cube` line still tokenizes.
        let toks = lex("\" a comment\ncube(2)\n").unwrap();
        assert!(toks.iter().any(|t| matches!(t, Tok::Ident(s) if s == "cube")));
        // No stray string token from the comment.
        assert!(!toks.iter().any(|t| matches!(t, Tok::Str(_))));
    }

    #[test]
    fn quote_in_args_is_a_string() {
        // A `"` inside an expression (call arg) is still a string literal.
        let toks = lex("box(1,1,1).rotate(\"z\", 90)").unwrap();
        assert!(toks.iter().any(|t| matches!(t, Tok::Str(s) if s == "z")));
    }

    #[test]
    fn hash_comment_still_works() {
        let toks = lex("# hash comment\nsphere(3)").unwrap();
        assert!(toks.iter().any(|t| matches!(t, Tok::Ident(s) if s == "sphere")));
    }
}

fn lex_number(chars: &[char], start: usize) -> Result<(Tok, usize), String> {
    let mut i = start;
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
        i += 1;
    }
    let s: String = chars[start..i].iter().collect();
    let n: f64 = s
        .parse()
        .map_err(|_| format!("invalid number literal '{s}'"))?;
    Ok((Tok::Num(n), i))
}
