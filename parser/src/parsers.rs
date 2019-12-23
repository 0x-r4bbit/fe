use std::convert::TryFrom;

use nom::branch::alt;
use nom::combinator::{
    map,
    opt,
    verify,
};
use nom::error::{
    ErrorKind,
    ParseError,
};
use nom::multi::{
    many0,
    many1,
};
use nom::sequence::{
    pair,
    preceded,
    separated_pair,
    terminated,
};
use nom::IResult;

use crate::ast::ModuleStmt::*;
use crate::ast::*;
use crate::errors::make_error;
use crate::span::{
    Span,
    Spanned,
};
use crate::tokenizer::tokenize::{
    tokenize,
    TokenizeError,
};
use crate::tokenizer::types::{
    Token,
    TokenKind,
    TokenKind::*,
};

pub type TokenSlice<'a> = &'a [Token<'a>];
pub type TokenResult<'a, O, E> = IResult<TokenSlice<'a>, O, E>;

/// Tokenize the given source code in `source` and filter out tokens not
/// relevant to parsing.
pub fn get_parse_tokens<'a>(source: &'a str) -> Result<Vec<Token<'a>>, TokenizeError> {
    let tokens = tokenize(source)?;

    Ok(tokens
        .into_iter()
        .filter(|t| match t.kind {
            Comment(_) => false,
            WhitespaceNewline => false,
            _ => true,
        })
        .collect())
}

/// Parse a single token from a token slice.
pub fn one_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    match input.iter().next() {
        None => make_error(input, ErrorKind::Eof),
        Some(token) => Ok((&input[1..], token)),
    }
}

/// Parse a token of a specific kind from a token slice.
pub fn token<'a, E>(kind: TokenKind) -> impl Fn(TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    verify(one_token, move |t: &Token| t.kind == kind)
}

/// Parse a name token from a token slice.
pub fn name_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    verify(one_token, move |t: &Token| match t.kind {
        Name(_) => true,
        _ => false,
    })(input)
}

/// Parse a name token containing a specific string from a token slice.
pub fn name_string<'a, E>(string: &'a str) -> impl Fn(TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    verify(one_token, move |t: &Token| match &t.kind {
        Name(s) if s == string => true,
        _ => false,
    })
}

/// Parse a number token from a token slice.
pub fn num_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    verify(one_token, move |t: &Token| match t.kind {
        Num(_) => true,
        _ => false,
    })(input)
}

/// Parse a string token from a token slice.
pub fn str_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    verify(one_token, move |t: &Token| match t.kind {
        Str(_) => true,
        _ => false,
    })(input)
}

/// Parse an indent token from a token slice.
pub fn indent_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    token(Indent)(input)
}

/// Parse a dedent token from a token slice.
pub fn dedent_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    token(Dedent)(input)
}

/// Parse a grammatically significant newline token from a token slice.
pub fn newline_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    token(Newline)(input)
}

/// Parse an endmarker token from a token slice.
pub fn endmarker_token<'a, E>(input: TokenSlice<'a>) -> TokenResult<&Token, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    token(EndMarker)(input)
}

/// Parse a module definition.
pub fn file_input<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<Module>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    alt((empty_file_input, non_empty_file_input))(input)
}

/// Parse an empty module definition.
pub fn empty_file_input<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<Module>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    // ENDMARKER
    let (input, end_tok) = endmarker_token(input)?;

    Ok((
        input,
        Spanned {
            node: Module { body: vec![] },
            span: end_tok.span,
        },
    ))
}

/// Parse a non-empty module definition.
pub fn non_empty_file_input<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<Module>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    // module_stmt+
    let (input, body) = many1(module_stmt)(input)?;

    // ENDMARKER
    let (input, _) = endmarker_token(input)?;

    let span = {
        let first = body.first().unwrap();
        let last = body.last().unwrap();

        Span::from_pair(first, last)
    };

    Ok((
        input,
        Spanned {
            node: Module { body },
            span,
        },
    ))
}

/// Parse a module statement, such as a contract definition.
pub fn module_stmt<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    alt((import_stmt, contract_def))(input)
}

/// Parse an import statement.
pub fn import_stmt<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    terminated(alt((simple_import, from_import)), newline_token)(input)
}

/// Parse an import statement beginning with the "import" keyword.
pub fn simple_import<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, import_kw) = name_string("import")(input)?;
    let (input, first_name) = simple_import_name(input)?;
    let (input, mut other_names) = many0(preceded(token(Comma), simple_import_name))(input)?;

    let mut result = vec![first_name];
    result.append(&mut other_names);

    let span = {
        let last = result.last().unwrap();
        Span::from_pair(import_kw, last)
    };

    Ok((
        input,
        Spanned {
            node: SimpleImport { names: result },
            span,
        },
    ))
}

pub fn simple_import_name<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<SimpleImportName>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, path) = dotted_name(input)?;
    let (input, alias) = opt(preceded(name_string("as"), name_token))(input)?;

    let span = {
        match alias {
            Some(alias_tok) => Span::from_pair(&path, alias_tok),
            None => path.span,
        }
    };

    Ok((
        input,
        Spanned {
            node: SimpleImportName {
                path: path.node,
                alias: alias.map(|t| t.string.to_string()),
            },
            span,
        },
    ))
}

/// Parse an import statement beginning with the "from" keyword.
pub fn from_import<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    alt((from_import_parent_alt, from_import_sub_alt))(input)
}

/// Parse a "from" import with a path that contains only parent module
/// components.
pub fn from_import_parent_alt<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, from_kw) = name_string("from")(input)?;
    let (input, parent_level) = dots_to_int(input)?;
    let (input, _) = name_string("import")(input)?;
    let (input, names) = from_import_names(input)?;

    let path = Spanned {
        node: FromImportPath::Relative {
            parent_level: parent_level.node,
            path: vec![],
        },
        span: parent_level.span,
    };
    let span = Span::from_pair(from_kw, names.span);

    Ok((
        input,
        Spanned {
            node: FromImport { path, names },
            span,
        },
    ))
}

/// Parse a "from" import with a path that contains sub module components.
pub fn from_import_sub_alt<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, from_kw) = name_string("from")(input)?;
    let (input, path) = from_import_sub_path(input)?;
    let (input, _) = name_string("import")(input)?;
    let (input, names) = from_import_names(input)?;

    let span = Span::from_pair(from_kw, names.span);

    Ok((
        input,
        Spanned {
            node: FromImport { path, names },
            span,
        },
    ))
}

/// Parse a path containing sub module components in a "from" import statement.
pub fn from_import_sub_path<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<FromImportPath>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, opt_parent_level) = opt(dots_to_int)(input)?;
    let (input, dotted_name) = dotted_name(input)?;

    let result = match opt_parent_level {
        Some(parent_level) => {
            let span = Span::from_pair(&parent_level, &dotted_name);
            Spanned {
                node: FromImportPath::Relative {
                    parent_level: parent_level.node,
                    path: dotted_name.node,
                },
                span,
            }
        }
        None => Spanned {
            node: FromImportPath::Absolute {
                path: dotted_name.node,
            },
            span: dotted_name.span,
        },
    };

    Ok((input, result))
}

/// Parse the names to be imported by a "from" import statement.
pub fn from_import_names<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<FromImportNames>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    alt((
        from_import_names_star,
        from_import_names_parens,
        from_import_names_list,
    ))(input)
}

/// Parse a wildcard token ("*") in a "from" import statement.
pub fn from_import_names_star<'a, E>(
    input: TokenSlice<'a>,
) -> TokenResult<Spanned<FromImportNames>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, star) = token(Star)(input)?;

    Ok((
        input,
        Spanned {
            node: FromImportNames::Star,
            span: star.span,
        },
    ))
}

/// Parse a parenthesized list of names to be imported by a "from" import
/// statement.
pub fn from_import_names_parens<'a, E>(
    input: TokenSlice<'a>,
) -> TokenResult<Spanned<FromImportNames>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, o_paren) = token(OpenParen)(input)?;
    let (input, names) = from_import_names_list(input)?;
    let (input, c_paren) = token(CloseParen)(input)?;

    Ok((
        input,
        Spanned {
            node: names.node,
            span: Span::from_pair(o_paren, c_paren),
        },
    ))
}

/// Parse a list of names to be imported by a "from" import statement.
pub fn from_import_names_list<'a, E>(
    input: TokenSlice<'a>,
) -> TokenResult<Spanned<FromImportNames>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, first_name) = from_import_name(input)?;
    let (input, mut other_names) = many0(preceded(token(Comma), from_import_name))(input)?;
    let (input, comma_tok) = opt(token(Comma))(input)?;

    let mut names = vec![first_name];
    names.append(&mut other_names);

    let span = {
        let first = names.first().unwrap();
        match comma_tok {
            Some(tok) => Span::from_pair(first, tok),
            None => {
                let last = names.last().unwrap();
                Span::from_pair(first, last)
            }
        }
    };

    Ok((
        input,
        Spanned {
            node: FromImportNames::List(names),
            span,
        },
    ))
}

/// Parse an import name with an optional alias in a "from" import statement.
pub fn from_import_name<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<FromImportName>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, name) = name_token(input)?;
    let (input, alias) = opt(preceded(name_string("as"), name_token))(input)?;

    let span = match alias {
        Some(alias_tok) => Span::from_pair(name, alias_tok),
        None => name.span,
    };

    Ok((
        input,
        Spanned {
            node: FromImportName {
                name: name.string.to_string(),
                alias: alias.map(|t| t.string.to_string()),
            },
            span,
        },
    ))
}

/// Parse a dotted import name.
pub fn dotted_name<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<Vec<String>>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, first_part) = name_token(input)?;
    let (input, other_parts) = many0(preceded(token(Dot), name_token))(input)?;

    let mut path = vec![first_part.string.to_string()];
    path.extend(other_parts.iter().map(|t| t.string.to_string()));

    let span = if other_parts.is_empty() {
        first_part.span
    } else {
        let last_part = other_parts.last().unwrap();
        Span::from_pair(first_part, *last_part)
    };

    Ok((input, Spanned { node: path, span }))
}

/// Parse preceding dots used to indicate parent module imports in import
/// statements.
pub fn dots_to_int<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<usize>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, toks) = many1(alt((token(Dot), token(Ellipsis))))(input)?;

    let value = toks
        .iter()
        .map(|t| if t.kind == Dot { 1 } else { 3 })
        .sum::<usize>()
        - 1;

    let span = {
        let first = toks.first().unwrap();
        let last = toks.last().unwrap();

        Span::from_pair(*first, *last)
    };

    Ok((input, Spanned { node: value, span }))
}

/// Parse a contract definition statement.
pub fn contract_def<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ModuleStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    // "contract" name ":" NEWLINE
    let (input, contract_kw) = name_string("contract")(input)?;
    let (input, name) = name_token(input)?;
    let (input, _) = token(Colon)(input)?;
    let (input, _) = newline_token(input)?;

    // INDENT contract_stmt+ DEDENT
    let (input, _) = indent_token(input)?;
    let (input, body) = many1(contract_stmt)(input)?;
    let (input, _) = dedent_token(input)?;

    let last_stmt = body.last().unwrap();
    let span = Span::from_pair(contract_kw, last_stmt);

    Ok((
        input,
        Spanned {
            node: ContractDef {
                name: name.string.to_string(),
                body,
            },
            span,
        },
    ))
}

/// Parse a contract statement.
pub fn contract_stmt<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ContractStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    event_def(input)
}

/// Parse an event definition statement.
pub fn event_def<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ContractStmt>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    // "event" name ":" NEWLINE
    let (input, event_kw) = name_string("event")(input)?;
    let (input, name) = name_token(input)?;
    let (input, _) = token(Colon)(input)?;
    let (input, _) = newline_token(input)?;

    // INDENT event_field+ DEDENT
    let (input, _) = indent_token(input)?;
    let (input, fields) = many1(event_field)(input)?;
    let (input, _) = dedent_token(input)?;

    let last_field = fields.last().unwrap();
    let span = Span::from_pair(event_kw, last_field);

    Ok((
        input,
        Spanned {
            node: ContractStmt::EventDef {
                name: name.string.to_string(),
                fields,
            },
            span,
        },
    ))
}

/// Parse an event field definition.
pub fn event_field<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<EventField>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, name) = name_token(input)?;
    let (input, _) = token(Colon)(input)?;
    let (input, typ) = name_token(input)?;
    let (input, _) = newline_token(input)?;

    let span = Span::from_pair(name, typ);

    Ok((
        input,
        Spanned {
            node: EventField {
                name: name.string.to_string(),
                typ: typ.into(),
            },
            span,
        },
    ))
}

/// Parse a constant expression that can be evaluated at compile-time.
pub fn const_expr<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ConstExpr>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, head) = const_term(input)?;
    let (input, tail) = many0(alt((
        pair(token(Plus), const_term),
        pair(token(Minus), const_term),
    )))(input)?;

    let mut left_expr = head;
    for (op_tok, right_expr) in tail {
        let span = Span::from_pair(&left_expr, &right_expr);

        left_expr = Spanned {
            node: ConstExpr::BinOp {
                left: Box::new(left_expr),
                op: Operator::try_from(op_tok.string).unwrap(),
                right: Box::new(right_expr),
            },
            span,
        };
    }

    Ok((input, left_expr))
}

/// Parse a constant term that may appear as the operand of an addition or
/// subtraction.
pub fn const_term<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ConstExpr>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, head) = const_factor(input)?;
    let (input, tail) = many0(alt((
        pair(token(Star), const_factor),
        pair(token(Slash), const_factor),
        pair(token(Percent), const_factor),
    )))(input)?;

    let mut left_expr = head;
    for (op_tok, right_expr) in tail {
        let span = Span::from_pair(&left_expr, &right_expr);

        left_expr = Spanned {
            node: ConstExpr::BinOp {
                left: Box::new(left_expr),
                op: Operator::try_from(op_tok.string).unwrap(),
                right: Box::new(right_expr),
            },
            span,
        };
    }

    Ok((input, left_expr))
}

/// Parse a constant factor that may appear as the operand of a multiplication,
/// division, modulus, or unary op or as the exponent of a power expression.
pub fn const_factor<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ConstExpr>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let unary_op = map(
        pair(alt((token(Plus), token(Minus), token(Tilde))), const_factor),
        |res| {
            let (op_tok, operand) = res;
            let span = Span::from_pair(op_tok, &operand);

            Spanned {
                node: ConstExpr::UnaryOp {
                    op: UnaryOp::try_from(op_tok.string).unwrap(),
                    operand: Box::new(operand),
                },
                span,
            }
        },
    );

    alt((unary_op, const_power))(input)
}

/// Parse a constant power expression that may appear in the position of a
/// constant factor.
pub fn const_power<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ConstExpr>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let bin_op = map(
        separated_pair(const_atom, token(StarStar), const_factor),
        |res| {
            let (left, right) = res;
            let span = Span::from_pair(&left, &right);

            Spanned {
                node: ConstExpr::BinOp {
                    left: Box::new(left),
                    op: Operator::Pow,
                    right: Box::new(right),
                },
                span,
            }
        },
    );

    alt((bin_op, const_atom))(input)
}

/// Parse a constant atom expression that may appear in the position of a
/// constant power or as the base of a constant power expression.
pub fn const_atom<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ConstExpr>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    alt((
        const_group,
        map(name_token, |t| Spanned {
            node: ConstExpr::Name {
                name: t.string.to_string(),
            },
            span: t.span,
        }),
        map(num_token, |t| Spanned {
            node: ConstExpr::Num {
                num: t.string.to_string(),
            },
            span: t.span,
        }),
    ))(input)
}

/// Parse a parenthesized constant group that may appear in the position of a
/// constant atom.
pub fn const_group<'a, E>(input: TokenSlice<'a>) -> TokenResult<Spanned<ConstExpr>, E>
where
    E: ParseError<TokenSlice<'a>>,
{
    let (input, o_paren) = token(OpenParen)(input)?;
    let (input, spanned_expr) = const_expr(input)?;
    let (input, c_paren) = token(CloseParen)(input)?;

    Ok((
        input,
        Spanned {
            node: spanned_expr.node,
            span: Span::from_pair(o_paren, c_paren),
        },
    ))
}
