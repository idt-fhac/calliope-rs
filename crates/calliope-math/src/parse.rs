//! Parsing math strings with the pest grammar, then resolving identifiers against the names.
//!
//! The resolution step rejects exactly what Calliope's name-specific pyparsing grammar would
//! not have matched, so that a string parses here if and only if it parses in Calliope
//! (checked against the Python parser on the whole math corpus and on mutants of it, see
//! `tests/parse_corpus.rs`).

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;

use crate::ast::{ArithOp, CmpOp, Lang, LogicOp, Node, NodeKind, Number, Role, Sign, Span};
use crate::names::Names;

#[derive(Parser)]
#[grammar = "grammar.pest"]
struct MathParser;

/// Which grammar a string is parsed with (`ParsedBackendComponent.PARSERS` and friends).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StringKind {
    /// A `where` string (`generate_where_string_parser`).
    Where,
    /// A constraint equation `lhs <=|>=|== rhs` (`generate_equation_parser`).
    Equation,
    /// A global expression, objective or piecewise expression (`generate_arithmetic_parser`).
    Arithmetic,
    /// A sub-expression (`generate_sub_expression_parser`).
    SubExpression,
    /// A slice (`generate_slice_parser`).
    Slice,
}

impl StringKind {
    /// The language of the string.
    pub fn lang(self) -> Lang {
        match self {
            StringKind::Where => Lang::Where,
            _ => Lang::Expression,
        }
    }
}

/// A string that does not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ParseError {
    /// What is wrong.
    pub message: String,
    /// Where (a byte range of the string).
    pub span: Span,
}

impl ParseError {
    fn at(span: Span, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

type Result<T> = std::result::Result<T, ParseError>;

/// Parses a math string. `names` are the component names valid in it; for `where` strings
/// of postprocessed expressions, `names.postprocessed` holds the postprocessed names.
pub fn parse(kind: StringKind, text: &str, names: &Names) -> Result<Node> {
    let rule = match kind {
        StringKind::Where => Rule::where_string,
        StringKind::Equation => Rule::equation_string,
        StringKind::Arithmetic => Rule::arithmetic_string,
        StringKind::SubExpression => Rule::sub_expression_string,
        StringKind::Slice => Rule::slice_string,
    };
    let mut pairs = MathParser::parse(rule, text).map_err(|e| pest_error(&e, text))?;
    let top = pairs.next().expect("entry rule");
    let b = Builder { text, names };
    let mut inner = top.into_inner().filter(|p| p.as_rule() != Rule::EOI);
    let first = inner.next().expect("entry content");
    let mut node = match kind {
        StringKind::Where => b.where_expr(first),
        StringKind::Equation => {
            let op = inner.next().expect("operator");
            let rhs = inner.next().expect("rhs");
            let lhs = b.arith(first, Ctx::Equation)?;
            let rhs = b.arith(rhs, Ctx::Equation)?;
            let span = lhs.span.start..rhs.span.end;
            Ok(Node::new(
                NodeKind::Compare {
                    lhs: Box::new(lhs),
                    op: cmp_op(op.as_str()),
                    rhs: Box::new(rhs),
                    lang: Lang::Expression,
                },
                span,
            ))
        }
        StringKind::Arithmetic => b.arith(first, Ctx::Equation),
        StringKind::SubExpression => b.arith(first, Ctx::SubExpression),
        StringKind::Slice => b.slice_value(first),
    }?;
    trim_spans(&mut node, text);
    Ok(node)
}

/// pest spans of repeated rules can end in the whitespace before a failed repetition.
fn trim_spans(node: &mut Node, text: &str) {
    let trimmed = text[node.span.clone()].trim_end().len();
    node.span.end = node.span.start + trimmed;
    match &mut node.kind {
        NodeKind::Sliced { slices, .. } => slices.iter_mut().for_each(|(_, n)| trim_spans(n, text)),
        NodeKind::List(items) | NodeKind::Subset { items, .. } => {
            items.iter_mut().for_each(|n| trim_spans(n, text));
        }
        NodeKind::Sign(_, n) | NodeKind::Not(n) => trim_spans(n, text),
        NodeKind::Arith(first, rest) => {
            trim_spans(first, text);
            rest.iter_mut().for_each(|(_, n)| trim_spans(n, text));
        }
        NodeKind::Logic(first, rest) => {
            trim_spans(first, text);
            rest.iter_mut().for_each(|(_, n)| trim_spans(n, text));
        }
        NodeKind::Call { args, kwargs, .. } => {
            args.iter_mut().for_each(|n| trim_spans(n, text));
            kwargs.iter_mut().for_each(|(_, n)| trim_spans(n, text));
        }
        NodeKind::Compare { lhs, rhs, .. } => {
            trim_spans(lhs, text);
            trim_spans(rhs, text);
        }
        _ => {}
    }
}

fn pest_error(e: &pest::error::Error<Rule>, text: &str) -> ParseError {
    let pos = match e.location {
        pest::error::InputLocation::Pos(p) => p,
        pest::error::InputLocation::Span((s, _)) => s,
    };
    let found = text[pos..]
        .chars()
        .next()
        .map_or_else(|| "end of string".to_owned(), |c| format!("`{c}`"));
    let expected = match &e.variant {
        pest::error::ErrorVariant::ParsingError { positives, .. } => {
            let mut names: Vec<&str> = positives.iter().filter_map(|r| rule_name(*r)).collect();
            names.sort_unstable();
            names.dedup();
            names
        }
        pest::error::ErrorVariant::CustomError { .. } => Vec::new(),
    };
    let message = if expected.is_empty() {
        format!("Unexpected {found}")
    } else {
        format!("Expected {}, found {found}", expected.join(" or "))
    };
    ParseError::at(pos..pos, message)
}

fn rule_name(r: Rule) -> Option<&'static str> {
    Some(match r {
        Rule::ident | Rule::call_open | Rule::sliced_open => "a name",
        Rule::number | Rule::float | Rule::int | Rule::inf => "a number",
        Rule::add_op | Rule::mul_op | Rule::pow_op | Rule::sign_op => "an operator",
        Rule::cmp_op | Rule::eq_op => "a comparison operator",
        Rule::logic_op | Rule::kw_and | Rule::kw_or => "`and`/`or`",
        Rule::kw_not => "`not`",
        Rule::boolean => "a boolean",
        Rule::sub_expr | Rule::slice_ref => "a `$` reference",
        Rule::id_list => "a list",
        Rule::config => "`config.<option>`",
        Rule::subset_in => "`] in <dimension>`",
        Rule::EOI => "end of string",
        _ => return None,
    })
}

fn cmp_op(s: &str) -> CmpOp {
    match s {
        "==" => CmpOp::Eq,
        "<=" => CmpOp::Le,
        ">=" => CmpOp::Ge,
        "<" => CmpOp::Lt,
        ">" => CmpOp::Gt,
        _ => unreachable!("grammar only allows comparison operators"),
    }
}

fn span(p: &Pair<'_, Rule>) -> Span {
    p.as_span().start()..p.as_span().end()
}

/// The arithmetic grammars, which differ in their operands and in how names resolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// Equations and global expressions.
    Equation,
    /// Sub-expressions.
    SubExpression,
    /// The left-hand side of a `where` comparison.
    Comparison,
}

/// Which list grammar a `[...]` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListCtx {
    /// `helper_func_list`: numbers, generic strings, components.
    HelperArg,
    /// `slice_list`: numbers and generic strings.
    Slice,
    /// The `where` `id_list`: numbers, generic strings, dimensions and inputs.
    Where,
}

struct Builder<'a> {
    text: &'a str,
    names: &'a Names,
}

impl Builder<'_> {
    fn err<T>(&self, span: Span, message: impl Into<String>) -> Result<T> {
        Err(ParseError::at(span, message))
    }

    /// pyparsing keywords also need a non-identifier character before them.
    fn check_keyword_start(&self, p: &Pair<'_, Rule>) -> Result<()> {
        let start = p.as_span().start();
        if let Some(c) = self.text[..start].chars().next_back()
            && (c.is_ascii_alphanumeric() || c == '_' || c == '$')
        {
            return self.err(
                span(p),
                format!("Keyword `{}` must not follow `{c}` directly", p.as_str()),
            );
        }
        Ok(())
    }

    // ---- where strings -------------------------------------------------------------

    fn where_expr(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        let mut inner = p.into_inner();
        let first = self.where_not(inner.next().expect("operand"))?;
        let mut rest = Vec::new();
        while let Some(op) = inner.next() {
            let kw = op.into_inner().next().expect("keyword");
            self.check_keyword_start(&kw)?;
            let op = match kw.as_rule() {
                Rule::kw_and => LogicOp::And,
                _ => LogicOp::Or,
            };
            rest.push((op, self.where_not(inner.next().expect("operand"))?));
        }
        if rest.is_empty() {
            Ok(first)
        } else {
            Ok(Node::new(NodeKind::Logic(Box::new(first), rest), sp))
        }
    }

    fn where_not(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        let mut inner = p.into_inner();
        let first = inner.next().expect("content");
        if first.as_rule() == Rule::kw_not {
            self.check_keyword_start(&first)?;
            let operand = self.where_not(inner.next().expect("operand"))?;
            return Ok(Node::new(NodeKind::Not(Box::new(operand)), sp));
        }
        self.where_atom(first)
    }

    fn where_atom(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        match p.as_rule() {
            Rule::boolean => self.boolean(p),
            Rule::comparison => self.comparison(p),
            Rule::call_w => self.call(p, CallCtx::Where),
            Rule::subset => self.subset(p),
            Rule::where_expr => self.where_expr(p),
            Rule::ident => {
                let name = p.as_str();
                match self.names.role(name) {
                    Some(role @ (Role::Input | Role::Result | Role::Postprocessed)) => {
                        Ok(self.where_ref(name, role, sp))
                    }
                    Some(Role::Dimension) => self.err(
                        sp,
                        format!("Dimension `{name}` cannot be used on its own in a `where` string"),
                    ),
                    None => self.err(sp, format!("Unknown input or result `{name}`")),
                }
            }
            r => unreachable!("where atom {r:?}"),
        }
    }

    fn where_ref(&self, name: &str, role: Role, sp: Span) -> Node {
        Node::new(
            NodeKind::Ref {
                name: name.to_owned(),
                role,
                lang: Lang::Where,
            },
            sp,
        )
    }

    fn boolean(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        let kw = p.into_inner().next().expect("keyword");
        self.check_keyword_start(&kw)?;
        Ok(Node::new(
            NodeKind::Bool(kw.as_rule() == Rule::bool_true),
            sp,
        ))
    }

    fn comparison(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        let mut inner = p.into_inner();
        let lhs = self.arith(inner.next().expect("lhs"), Ctx::Comparison)?;
        let op = cmp_op(inner.next().expect("op").as_str());
        let r = inner.next().expect("rhs");
        let rhs = match r.as_rule() {
            Rule::call_c => self.call(r, CallCtx::Comparison)?,
            Rule::boolean => self.boolean(r)?,
            Rule::number => self.number(r),
            Rule::ident => Node::new(NodeKind::Str(r.as_str().to_owned()), span(&r)),
            other => unreachable!("comparison rhs {other:?}"),
        };
        Ok(Node::new(
            NodeKind::Compare {
                lhs: Box::new(lhs),
                op,
                rhs: Box::new(rhs),
                lang: Lang::Where,
            },
            sp,
        ))
    }

    fn subset(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        let mut items = Vec::new();
        let mut dim = None;
        for item in p.into_inner() {
            match item.as_rule() {
                Rule::config => items.push(self.config(item)),
                Rule::number => items.push(self.number(item)),
                Rule::ident if dim.is_none() => {
                    // Either a subset item or the dimension after `in`; the dimension comes
                    // last, after `subset_in`.
                    items.push(Node::new(
                        NodeKind::Str(item.as_str().to_owned()),
                        span(&item),
                    ));
                }
                Rule::subset_in => dim = Some(()),
                Rule::ident => {
                    let name = item.as_str();
                    if self.names.role(name) != Some(Role::Dimension) {
                        return self.err(span(&item), format!("Unknown dimension `{name}`"));
                    }
                    return Ok(Node::new(
                        NodeKind::Subset {
                            items,
                            dim: name.to_owned(),
                        },
                        sp,
                    ));
                }
                r => unreachable!("subset {r:?}"),
            }
        }
        unreachable!("subset without dimension")
    }

    fn config(&self, p: Pair<'_, Rule>) -> Node {
        let sp = span(&p);
        let key = p.into_inner().next().expect("key").as_str().to_owned();
        Node::new(NodeKind::Config(key), sp)
    }

    fn number(&self, p: Pair<'_, Rule>) -> Node {
        let sp = span(&p);
        let tok = p.into_inner().next().expect("number token");
        let s = tok.as_str();
        let n = match tok.as_rule() {
            Rule::inf => Number {
                value: f64::INFINITY,
                is_int: false,
            },
            Rule::int => Number {
                #[allow(clippy::cast_precision_loss)]
                value: s
                    .parse::<i64>()
                    .map_or_else(|_| s.parse().unwrap_or(f64::NAN), |i| i as f64),
                is_int: true,
            },
            _ => Number {
                value: s.parse().unwrap_or(f64::NAN),
                is_int: false,
            },
        };
        Node::new(NodeKind::Number(n), sp)
    }

    // ---- arithmetic ----------------------------------------------------------------

    fn arith(&self, p: Pair<'_, Rule>, ctx: Ctx) -> Result<Node> {
        let sp = span(&p);
        match p.as_rule() {
            Rule::arith_e
            | Rule::arith_s
            | Rule::arith_c
            | Rule::term_e
            | Rule::term_s
            | Rule::term_c
            | Rule::pow_e
            | Rule::pow_s
            | Rule::pow_c => {
                let mut inner = p.into_inner();
                let first = self.arith(inner.next().expect("operand"), ctx)?;
                let mut rest = Vec::new();
                while let Some(op) = inner.next() {
                    let op = match op.as_str() {
                        "+" => ArithOp::Add,
                        "-" => ArithOp::Sub,
                        "*" => ArithOp::Mul,
                        "/" => ArithOp::Div,
                        _ => ArithOp::Pow,
                    };
                    rest.push((op, self.arith(inner.next().expect("operand"), ctx)?));
                }
                if rest.is_empty() {
                    Ok(first)
                } else {
                    Ok(Node::new(NodeKind::Arith(Box::new(first), rest), sp))
                }
            }
            Rule::sign_e | Rule::sign_s | Rule::sign_c => {
                let mut inner = p.into_inner();
                let first = inner.next().expect("content");
                if first.as_rule() == Rule::sign_op {
                    let sign = if first.as_str() == "-" {
                        Sign::Minus
                    } else {
                        Sign::Plus
                    };
                    let operand = self.arith(inner.next().expect("operand"), ctx)?;
                    Ok(Node::new(NodeKind::Sign(sign, Box::new(operand)), sp))
                } else {
                    self.arith(first, ctx)
                }
            }
            Rule::number => Ok(self.number(p)),
            Rule::config => Ok(self.config(p)),
            Rule::sub_expr => {
                let name = p.into_inner().next().expect("name").as_str().to_owned();
                Ok(Node::new(NodeKind::SubExpr(name), sp))
            }
            Rule::sliced => self.sliced(p, true),
            Rule::call_e => self.call(p, CallCtx::Equation),
            Rule::call_s => self.call(p, CallCtx::SubExpression),
            Rule::call_c => self.call(p, CallCtx::Comparison),
            Rule::ident => self.arith_ident(&p, ctx),
            r => unreachable!("arithmetic {r:?}"),
        }
    }

    /// An identifier operand in arithmetic: it must be a component.
    fn arith_ident(&self, p: &Pair<'_, Rule>, ctx: Ctx) -> Result<Node> {
        let name = p.as_str();
        let sp = span(p);
        let role = self.names.role(name);
        match ctx {
            Ctx::Equation | Ctx::SubExpression => match role {
                Some(role) => Ok(Node::new(
                    NodeKind::Ref {
                        name: name.to_owned(),
                        role,
                        lang: Lang::Expression,
                    },
                    sp,
                )),
                None => self.err(sp, format!("Unknown component `{name}`")),
            },
            Ctx::Comparison => match role {
                Some(r @ (Role::Dimension | Role::Input)) => Ok(self.where_ref(name, r, sp)),
                // `results_parser` is an arithmetic element only for postprocessed math.
                Some(r @ (Role::Result | Role::Postprocessed))
                    if !self.names.postprocessed.is_empty() =>
                {
                    Ok(self.where_ref(name, r, sp))
                }
                Some(_) => self.err(
                    sp,
                    format!("Result `{name}` cannot be compared in a `where` string"),
                ),
                None => self.err(sp, format!("Unknown dimension or input `{name}`")),
            },
        }
    }

    // ---- helper function calls ------------------------------------------------------

    fn call(&self, p: Pair<'_, Rule>, ctx: CallCtx) -> Result<Node> {
        let sp = span(&p);
        let mut inner = p.into_inner();
        let open = inner.next().expect("call_open");
        let name_pair = open.into_inner().next().expect("name");
        let name = name_pair.as_str().to_owned();
        let name_span = span(&name_pair);
        let mut args = Vec::new();
        let mut kwargs: Vec<(String, Node)> = Vec::new();
        let call_args = inner.next().expect("call args");
        for part in call_args.into_inner() {
            match part.as_rule() {
                Rule::args_e | Rule::args_s | Rule::args_w | Rule::args_c | Rule::args_slice => {
                    for a in part.into_inner() {
                        args.push(self.arg(a, ctx)?);
                    }
                }
                _ => {
                    for kw in part.into_inner() {
                        let mut kv = kw.into_inner();
                        let key = kv.next().expect("key").as_str().to_owned();
                        let value = self.arg(kv.next().expect("value"), ctx)?;
                        // `dict_of`: a repeated key keeps its first position, last value.
                        if let Some(slot) = kwargs.iter_mut().find(|(k, _)| *k == key) {
                            slot.1 = value;
                        } else {
                            kwargs.push((key, value));
                        }
                    }
                }
            }
        }
        Ok(Node::new(
            NodeKind::Call {
                name,
                name_span,
                args,
                kwargs,
            },
            sp,
        ))
    }

    fn arg(&self, p: Pair<'_, Rule>, ctx: CallCtx) -> Result<Node> {
        let sp = span(&p);
        let value = p.into_inner().next().expect("arg value");
        match ctx {
            CallCtx::Equation | CallCtx::SubExpression => {
                if value.as_rule() == Rule::id_list {
                    return self.list(value, ListCtx::HelperArg);
                }
                // `MatchFirst([arithmetic, helper_func_list, evaluatable_identifier])`: an
                // identifier that is not a component is a generic string.
                if let Some(id) = bare_ident(&value)
                    && !self.names.contains(id.as_str())
                {
                    return Ok(Node::new(NodeKind::Str(id.as_str().to_owned()), span(&id)));
                }
                let actx = if ctx == CallCtx::Equation {
                    Ctx::Equation
                } else {
                    Ctx::SubExpression
                };
                self.arith(value, actx)
            }
            CallCtx::Where => match value.as_rule() {
                Rule::ident => Ok(self.classify_where(&value)),
                Rule::number => Ok(self.number(value)),
                Rule::id_list => self.list(value, ListCtx::Where),
                Rule::config => Ok(self.config(value)),
                r => unreachable!("where arg {r:?}"),
            },
            CallCtx::Comparison => {
                // `MatchFirst([unique_evaluatable_string, number, id_list, arithmetic])`: an
                // argument that starts with a non-component identifier is that identifier.
                let text = &self.text[sp.clone()];
                if let Some(word) = leading_ident(text)
                    && !self.names.contains(word)
                {
                    if text.trim_end() == word {
                        return Ok(Node::new(
                            NodeKind::Str(word.to_owned()),
                            sp.start..sp.start + word.len(),
                        ));
                    }
                    return self.err(
                        sp.start..sp.start + word.len(),
                        format!("`{word}` is not a component, so it must be a whole argument"),
                    );
                }
                match value.as_rule() {
                    Rule::number => Ok(self.number(value)),
                    Rule::id_list => self.list(value, ListCtx::Where),
                    _ => self.arith(value, Ctx::Comparison),
                }
            }
            CallCtx::Slice => self.slice_value(value),
        }
    }

    /// A `where` helper argument identifier: dimension, input, result or generic string.
    fn classify_where(&self, p: &Pair<'_, Rule>) -> Node {
        let name = p.as_str();
        match self.names.role(name) {
            Some(role) => self.where_ref(name, role, span(p)),
            None => Node::new(NodeKind::Str(name.to_owned()), span(p)),
        }
    }

    fn list(&self, p: Pair<'_, Rule>, ctx: ListCtx) -> Result<Node> {
        let sp = span(&p);
        let mut items = Vec::new();
        for item in p.into_inner() {
            if item.as_rule() == Rule::number {
                items.push(self.number(item));
                continue;
            }
            let name = item.as_str();
            let isp = span(&item);
            let role = self.names.role(name);
            items.push(match (ctx, role) {
                (_, None) => Node::new(NodeKind::Str(name.to_owned()), isp),
                (ListCtx::HelperArg, Some(role)) => Node::new(
                    NodeKind::Ref {
                        name: name.to_owned(),
                        role,
                        lang: Lang::Expression,
                    },
                    isp,
                ),
                (ListCtx::Where, Some(r @ (Role::Dimension | Role::Input))) => {
                    self.where_ref(name, r, isp)
                }
                (ListCtx::Where, Some(_)) => {
                    return self.err(isp, format!("Result `{name}` cannot be a list item here"));
                }
                (ListCtx::Slice, Some(_)) => {
                    return self.err(
                        isp,
                        format!("Component `{name}` cannot be a slice list item"),
                    );
                }
            });
        }
        Ok(Node::new(NodeKind::List(items), sp))
    }

    // ---- slices --------------------------------------------------------------------

    fn sliced(&self, p: Pair<'_, Rule>, refs: bool) -> Result<Node> {
        let sp = span(&p);
        let mut inner = p.into_inner();
        let open = inner.next().expect("sliced_open");
        let name_pair = open.into_inner().next().expect("name");
        let name = name_pair.as_str().to_owned();
        let Some(role) = self.names.role(&name) else {
            return self.err(span(&name_pair), format!("Unknown component `{name}`"));
        };
        let mut slices = Vec::new();
        for item in inner {
            let mut kv = item.into_inner();
            let dim = kv.next().expect("dim").as_str().to_owned();
            let v = kv.next().expect("slicer");
            let vsp = span(&v);
            let slicer = match v.as_rule() {
                Rule::slice_ref if refs => {
                    let n = v.into_inner().next().expect("name").as_str().to_owned();
                    Node::new(NodeKind::SliceRef(n), vsp)
                }
                Rule::number => self.number(v),
                Rule::id_list => self.list(v, ListCtx::Slice)?,
                Rule::ident => {
                    let s = v.as_str();
                    if self.names.contains(s) {
                        return self.err(
                            vsp,
                            format!("Component `{s}` cannot be a slice; use a `$slice` reference"),
                        );
                    }
                    Node::new(NodeKind::Str(s.to_owned()), vsp)
                }
                r => unreachable!("slicer {r:?}"),
            };
            slices.push((dim, slicer));
        }
        Ok(Node::new(NodeKind::Sliced { name, role, slices }, sp))
    }

    /// A `slices` expression, or an argument of a helper in one.
    fn slice_value(&self, p: Pair<'_, Rule>) -> Result<Node> {
        let sp = span(&p);
        match p.as_rule() {
            Rule::call_slice => self.call(p, CallCtx::Slice),
            Rule::sliced_noref => self.sliced(p, false),
            Rule::number => Ok(self.number(p)),
            Rule::id_list => self.list(p, ListCtx::Slice),
            Rule::ident => {
                let name = p.as_str();
                Ok(match self.names.role(name) {
                    Some(role) => Node::new(
                        NodeKind::Ref {
                            name: name.to_owned(),
                            role,
                            lang: Lang::Expression,
                        },
                        sp,
                    ),
                    None => Node::new(NodeKind::Str(name.to_owned()), sp),
                })
            }
            r => unreachable!("slice value {r:?}"),
        }
    }
}

/// Where a helper function is called; decides its argument grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallCtx {
    Equation,
    SubExpression,
    Where,
    Comparison,
    Slice,
}

/// The identifier an arithmetic pair consists of, if it is a lone identifier (not in
/// parentheses, no sign or operator).
fn bare_ident<'i>(p: &Pair<'i, Rule>) -> Option<Pair<'i, Rule>> {
    let mut cur = p.clone();
    let mut ariths = 0;
    loop {
        match cur.as_rule() {
            Rule::ident => return Some(cur),
            Rule::arith_e | Rule::arith_s => {
                ariths += 1;
                if ariths > 1 {
                    return None;
                }
            }
            Rule::term_e
            | Rule::term_s
            | Rule::pow_e
            | Rule::pow_s
            | Rule::sign_e
            | Rule::sign_s => {}
            _ => return None,
        }
        let mut inner = cur.into_inner();
        let only = inner.next()?;
        if inner.next().is_some() {
            return None;
        }
        cur = only;
    }
}

/// The identifier a string starts with (`Word(alphas, alphanums + "_")`, not `inf`).
fn leading_ident(s: &str) -> Option<&str> {
    let first = s.chars().next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    let word = &s[..end];
    let after = s[end..].chars().next();
    let is_inf = word.eq_ignore_ascii_case("inf") && !matches!(after, Some('$'));
    if is_inf { None } else { Some(word) }
}
