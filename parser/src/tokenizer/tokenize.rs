use std::convert::TryFrom;

use regex::Regex;

use crate::span::Span;
use crate::string_utils::{
    lines_with_endings,
    lstrip_slice,
    rstrip_slice,
};
use crate::symbol::Symbol;
use crate::tokenizer::regex::{
    compile_anchored,
    get_pseudotoken_pattern,
    get_single_quote_set,
    get_triple_quote_set,
    DOUBLE,
    DOUBLE3,
    SINGLE,
    SINGLE3,
};
use crate::tokenizer::types::{
    Token,
    TokenKind,
    TokenKind::*,
};

const TABSIZE: usize = 8;

#[inline]
fn is_identifier_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || c.is_digit(10)
}

#[derive(Debug, PartialEq)]
pub struct TokenizeError {
    pub msg: &'static str,
    pub offset: usize,
}

/// Parse a source string into a list of tokens.
///
/// Arguments:
///
/// * `input` - The source string to be parsed.
///
/// Returns:
///
/// A vector of tokens.
#[allow(clippy::cognitive_complexity)]
#[allow(clippy::trivial_regex)]
pub fn tokenize(input: &str) -> Result<Box<[Token]>, TokenizeError> {
    // Static values/helpers
    let pseudo_token_re = compile_anchored(&get_pseudotoken_pattern());

    let triple_quoted = get_triple_quote_set();
    let single_quoted = get_single_quote_set();

    let double3_re = Regex::new(DOUBLE3).unwrap();
    let single3_re = Regex::new(SINGLE3).unwrap();
    let double_re = Regex::new(DOUBLE).unwrap();
    let single_re = Regex::new(SINGLE).unwrap();

    // The ordering of checks matters here.  We need to eliminate the possibility of
    // triple quote delimiters before looking for single quote delimiters.
    let get_contstr_end_re = |token: &str| -> &Regex {
        let token_stripped = lstrip_slice(token, "bBrRuUfF");

        if token_stripped.starts_with("\"\"\"") {
            &double3_re
        } else if token_stripped.starts_with("'''") {
            &single3_re
        } else if token_stripped.starts_with('"') {
            &double_re
        } else {
            // This arm of the if statement is equivalent to the following check:
            // `else if token_stripped.starts_with('\'')`
            //
            // This is because any string in `token` has already been matched against a
            // regex that ensures it begins with """, ''', ", or ' after
            // stripping of any leading prefix codes
            &single_re
        }
    };

    // Token list result
    let mut result: Vec<Token> = Vec::new();

    // State vars
    let mut parenlev: usize = 0;
    let mut continued: bool = false;
    let mut indents: Vec<usize> = vec![0];

    let mut contstr_start: Option<usize> = None;
    let mut contstr_end_re: Option<&Regex> = None;
    let mut needcont: bool = false;

    // Token generation loop.  We use the `loop` keyword here (instead of `for
    // (line, line_num) in ...`) so we can hold onto the iterator vars after the
    // loop finishes.
    let mut line = &input[..0];
    let mut lines = lines_with_endings(input);

    loop {
        let next = lines.next();
        // We use this guard style of exiting to avoid indenting the entire loop body
        if next.is_none() {
            break;
        }

        // Get current line and line offsets
        let next_unwrap = next.unwrap();
        line = next_unwrap.0;
        let line_start = next_unwrap.1;
        let line_end = next_unwrap.2;

        // Set parsing position relative to this line
        let mut line_pos: usize = 0;
        let line_len: usize = line.len();

        if let Some(contstr_start_val) = contstr_start {
            // Continued string
            if let Some(endmatch) = contstr_end_re.unwrap().find(line) {
                let tok_end = endmatch.end();
                line_pos = tok_end;

                result.push(Token {
                    kind: Str,
                    span: Span::new(contstr_start_val, line_start + tok_end),
                });

                contstr_start = None;

                needcont = false;
            } else if needcont && !line.ends_with("\\\n") && !line.ends_with("\\\r\n") {
                result.push(Token {
                    kind: ErrorToken,
                    span: Span::new(contstr_start_val, line_end),
                });

                contstr_start = None;

                continue;
            } else {
                continue;
            }
        } else if parenlev == 0 && !continued {
            // New statement
            let mut column: usize = 0;

            // Measure leading whitespace
            for c in line.chars() {
                match c {
                    ' ' => {
                        column += 1;
                    }
                    '\t' => {
                        column = (column / TABSIZE + 1) * TABSIZE;
                    }
                    '\x0c' => {
                        // Form feed ("\f" in python)
                        column = 0;
                    }
                    _ => {
                        // Break if we encounter anything that's not part of indentation
                        break;
                    }
                }
                line_pos += c.len_utf8();
            }

            if line_pos == line_len {
                // If no more chars in line (not even newline, carriage return, etc.), we're at
                // EOF.  Break out of the token loop.
                break;
            }

            {
                let c = line[line_pos..].chars().next().unwrap();
                if c == '#' || c == '\r' || c == '\n' {
                    if c == '#' {
                        let comment_token = rstrip_slice(&line[line_pos..], "\r\n");
                        let comment_token_len = comment_token.len();

                        result.push(Token {
                            kind: Comment,
                            span: Span::new(
                                line_start + line_pos,
                                line_start + line_pos + comment_token_len,
                            ),
                        });

                        line_pos += comment_token_len;
                    }

                    result.push(Token {
                        kind: WhitespaceNewline,
                        span: Span::new(line_start + line_pos, line_end),
                    });

                    continue;
                }
            }

            let rest_off = line_start + line_pos;

            if column > *indents.last().unwrap() {
                indents.push(column);
                result.push(Token {
                    kind: Indent,
                    span: Span::new(line_start, rest_off),
                });
            }

            if !indents.contains(&column) {
                return Err(TokenizeError {
                    msg: "unindent does not match any outer indentation level",
                    offset: rest_off,
                });
            }

            while column < *indents.last().unwrap() {
                indents.pop();
                result.push(Token {
                    kind: Dedent,
                    span: Span::new(rest_off, rest_off),
                });
            }
        } else {
            continued = false;
        }

        while line_pos < line_len {
            if let Some(pseudomatch) = pseudo_token_re.captures(&line[line_pos..]) {
                let capture = pseudomatch.get(1).unwrap();
                let tok_start = line_pos + capture.start();
                let tok_end = line_pos + capture.end();

                let soff = line_start + tok_start;
                let eoff = line_start + tok_end;
                line_pos = tok_end;

                if tok_start == tok_end {
                    continue;
                }

                let token = &line[tok_start..tok_end];
                let initial = line[tok_start..].chars().next().unwrap();

                if initial.is_ascii_digit() || (initial == '.' && token != "." && token != "...") {
                    result.push(Token {
                        kind: Num,
                        span: Span::new(soff, eoff),
                    });
                } else if initial == '\r' || initial == '\n' {
                    result.push(Token {
                        kind: if parenlev > 0 {
                            WhitespaceNewline
                        } else {
                            Newline
                        },
                        span: Span::new(soff, eoff),
                    });
                } else if initial == '#' {
                    result.push(Token {
                        kind: Comment,
                        span: Span::new(soff, eoff),
                    });
                } else if triple_quoted.contains(token) {
                    contstr_end_re = Some(get_contstr_end_re(token));

                    if let Some(endmatch) = contstr_end_re.unwrap().find_at(line, line_pos) {
                        line_pos = endmatch.end();

                        result.push(Token {
                            kind: Str,
                            span: Span::new(soff, line_start + line_pos),
                        });
                    } else {
                        contstr_start = Some(line_start + tok_start);
                        break;
                    }
                } else if single_quoted.contains(&initial.to_string())
                    || single_quoted.contains(&token.chars().take(2).collect::<String>())
                    || single_quoted.contains(&token.chars().take(3).collect::<String>())
                {
                    if token.ends_with('\n') {
                        contstr_end_re = Some(get_contstr_end_re(token));
                        contstr_start = Some(line_start + tok_start);

                        needcont = true;
                    } else {
                        result.push(Token {
                            kind: Str,
                            span: Span::new(soff, eoff),
                        });
                    }
                } else if is_identifier_char(initial) {
                    result.push(Token {
                        kind: Name(Symbol::new(token)),
                        span: Span::new(soff, eoff),
                    });
                } else if initial == '\\' {
                    continued = true;
                } else {
                    if initial == '(' || initial == '[' || initial == '{' {
                        parenlev += 1;
                    } else if initial == ')' || initial == ']' || initial == '}' {
                        parenlev -= 1;
                    }
                    result.push(Token {
                        kind: TokenKind::try_from(token).unwrap(),
                        span: Span::new(soff, eoff),
                    });
                }
            } else {
                #[allow(clippy::range_plus_one)]
                result.push(Token {
                    kind: ErrorToken,
                    span: Span::new(line_start + line_pos, line_start + line_pos + 1),
                });
                line_pos += 1;
            }
        }
    }

    // We use this zero-length slice as the ending content for remaining tokens.
    // This is *just in case* anyone actually cares that the location of the
    // pointer makes any kind of sense.
    let input_len = input.len();

    if contstr_start.is_some() {
        return Err(TokenizeError {
            msg: "EOF in multi-line string",
            offset: input_len,
        });
    }

    if continued {
        return Err(TokenizeError {
            msg: "EOF in multi-line statement",
            offset: input_len,
        });
    }

    if !line.is_empty() {
        let last_char = line.chars().last().unwrap();
        if last_char != '\r' && last_char != '\n' {
            result.push(Token {
                kind: if line.trim().is_empty() {
                    WhitespaceNewline
                } else {
                    Newline
                },
                span: Span::new(
                    input_len,
                    // Python's stdlib tokenize module fudges the end position of this virtual
                    // token and says it's one character beyond the actual
                    // content of the string.  Since this could potentially
                    // lead someone access invalid memory, we differ slightly
                    // here and just act like the token has a length of zero.
                    input_len,
                ),
            });
        }
    }
    for _ in indents.iter().skip(1) {
        result.push(Token {
            kind: Dedent,
            span: Span::new(input_len, input_len),
        });
    }
    result.push(Token {
        kind: EndMarker,
        span: Span::new(input_len, input_len),
    });

    Ok(result.into_boxed_slice())
}
