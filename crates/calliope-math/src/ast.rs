//! The parsed form of math strings.
//!
//! One tree type covers `where` strings and expressions. [`std::fmt::Display`] reproduces
//! the `repr` of Calliope's pyparsing parse results (`NUM:1`, `INPUT:foo`,
//! `(COMPONENT:a + NUM:1)`, ...), which is how the parser is checked against Python.

use std::fmt;
use std::ops::Range;

/// A byte range in the parsed string.
pub type Span = Range<usize>;

/// A node with its location in the parsed string.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// What the node is.
    pub kind: NodeKind,
    /// Where it is.
    pub span: Span,
}

/// Which language a string is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// `where` strings (and `checks`).
    Where,
    /// Equations, global expressions, objectives, sub-expressions and slices.
    Expression,
}

/// What a name refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    /// A dimension (`techs`, `timesteps`, ...).
    Dimension,
    /// A parameter or lookup.
    Input,
    /// A decision variable or global expression.
    Result,
    /// A postprocessed expression.
    Postprocessed,
}

/// A number literal.
#[derive(Debug, Clone, PartialEq)]
pub struct Number {
    /// Its value.
    pub value: f64,
    /// True if written as an integer (Python `int`).
    pub is_int: bool,
}

impl fmt::Display for Number {
    /// Python's `str()` of the `int` or `float` pyparsing converts the literal to.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_int {
            #[allow(clippy::cast_possible_truncation)]
            let i = self.value as i64;
            write!(f, "{i}")
        } else {
            f.write_str(&calliope_yaml::python_float(self.value))
        }
    }
}

/// Unary sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    /// `+x`
    Plus,
    /// `-x`
    Minus,
}

/// Binary arithmetic operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `**`
    Pow,
}

impl ArithOp {
    /// The operator as written.
    pub fn as_str(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
            ArithOp::Pow => "**",
        }
    }
}

/// `and` / `or` (one precedence level, left to right, as in Calliope).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    /// `and`
    And,
    /// `or`
    Or,
}

/// Comparison operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `==`
    Eq,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `<` (`where` only)
    Lt,
    /// `>` (`where` only)
    Gt,
}

impl CmpOp {
    /// The operator as written.
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Eq => "==",
            CmpOp::Le => "<=",
            CmpOp::Ge => ">=",
            CmpOp::Lt => "<",
            CmpOp::Gt => ">",
        }
    }
}

/// The node types.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    /// A number (`NUM:1`).
    Number(Number),
    /// `True` / `False` in `where` strings (`BOOL:true`).
    Bool(bool),
    /// A generic string, e.g. a dimension item (`STRING:pv`).
    Str(String),
    /// A model component (`COMPONENT:x` in expressions; `DIM:`, `INPUT:`, `RESULT:` in `where`).
    Ref {
        /// The name.
        name: String,
        /// What it refers to.
        role: Role,
        /// The language it was parsed in (decides the `repr`).
        lang: Lang,
    },
    /// `config.<key>` (`CONFIG:key`).
    Config(String),
    /// `$name` in an equation (`SUB_EXPRESSION:name`).
    SubExpr(String),
    /// `$name` as a slice (`REFERENCE:name`).
    SliceRef(String),
    /// `name[dim=slicer, ...]` (`SLICED_COMPONENT:name[dim=..., ...]`).
    Sliced {
        /// The sliced component.
        name: String,
        /// Its role.
        role: Role,
        /// `(dimension, slicer)` pairs, in order.
        slices: Vec<(String, Node)>,
    },
    /// `[a, b, 1]`.
    List(Vec<Node>),
    /// `-x` / `+x`.
    Sign(Sign, Box<Node>),
    /// `not x`.
    Not(Box<Node>),
    /// `a op b op c ...` at one precedence level.
    Arith(Box<Node>, Vec<(ArithOp, Node)>),
    /// `a and b or c ...`.
    Logic(Box<Node>, Vec<(LogicOp, Node)>),
    /// A helper function call.
    Call {
        /// Function name.
        name: String,
        /// Span of the name.
        name_span: Span,
        /// Positional arguments.
        args: Vec<Node>,
        /// Keyword arguments, in order.
        kwargs: Vec<(String, Node)>,
    },
    /// `lhs op rhs`.
    Compare {
        /// Left-hand side.
        lhs: Box<Node>,
        /// Operator.
        op: CmpOp,
        /// Right-hand side.
        rhs: Box<Node>,
        /// `where` comparison (`a==b`) or equation (`a == b`).
        lang: Lang,
    },
    /// `[a, b] in dim` (`SUBSET:DIM:dim[...]`).
    Subset {
        /// The items.
        items: Vec<Node>,
        /// The dimension.
        dim: String,
    },
}

impl Node {
    /// A node.
    pub fn new(kind: NodeKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Visits this node and all nodes below it, parents first.
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        match &self.kind {
            NodeKind::Sliced { slices, .. } => slices.iter().for_each(|(_, n)| n.walk(f)),
            NodeKind::List(items) | NodeKind::Subset { items, .. } => {
                items.iter().for_each(|n| n.walk(f));
            }
            NodeKind::Sign(_, n) | NodeKind::Not(n) => n.walk(f),
            NodeKind::Arith(first, rest) => {
                first.walk(f);
                rest.iter().for_each(|(_, n)| n.walk(f));
            }
            NodeKind::Logic(first, rest) => {
                first.walk(f);
                rest.iter().for_each(|(_, n)| n.walk(f));
            }
            NodeKind::Call { args, kwargs, .. } => {
                args.iter().for_each(|n| n.walk(f));
                kwargs.iter().for_each(|(_, n)| n.walk(f));
            }
            NodeKind::Compare { lhs, rhs, .. } => {
                lhs.walk(f);
                rhs.walk(f);
            }
            _ => {}
        }
    }

    /// Names of the `$sub_expressions` referenced, in order of appearance (unique).
    pub fn sub_expression_refs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.walk(&mut |n| {
            if let NodeKind::SubExpr(s) = &n.kind
                && !out.contains(s)
            {
                out.push(s.clone());
            }
        });
        out
    }

    /// Names of the `$slices` referenced, in order of appearance (unique).
    pub fn slice_refs(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        self.walk(&mut |n| {
            if let NodeKind::SliceRef(s) = &n.kind
                && !out.contains(s)
            {
                out.push(s.clone());
            }
        });
        out
    }
}

fn join<'a>(f: &mut fmt::Formatter<'_>, items: impl Iterator<Item = &'a Node>) -> fmt::Result {
    for (i, n) in items.enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{n}")?;
    }
    Ok(())
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            NodeKind::Number(n) => write!(f, "NUM:{n}"),
            NodeKind::Bool(b) => write!(f, "BOOL:{b}"),
            NodeKind::Str(s) => write!(f, "STRING:{s}"),
            NodeKind::Ref { name, role, lang } => {
                let prefix = match (lang, role) {
                    (Lang::Expression, _) => "COMPONENT",
                    (Lang::Where, Role::Dimension) => "DIM",
                    (Lang::Where, Role::Input) => "INPUT",
                    (Lang::Where, Role::Result | Role::Postprocessed) => "RESULT",
                };
                write!(f, "{prefix}:{name}")
            }
            NodeKind::Config(k) => write!(f, "CONFIG:{k}"),
            NodeKind::SubExpr(s) => write!(f, "SUB_EXPRESSION:{s}"),
            NodeKind::SliceRef(s) => write!(f, "REFERENCE:{s}"),
            NodeKind::Sliced { name, slices, .. } => {
                write!(f, "SLICED_COMPONENT:{name}[")?;
                for (i, (dim, n)) in slices.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{dim}={n}")?;
                }
                f.write_str("]")
            }
            NodeKind::List(items) => {
                f.write_str("[")?;
                join(f, items.iter())?;
                f.write_str("]")
            }
            NodeKind::Sign(s, n) => {
                let s = match s {
                    Sign::Plus => "+",
                    Sign::Minus => "-",
                };
                write!(f, "({s}){n}")
            }
            NodeKind::Not(n) => write!(f, "(not){n}"),
            NodeKind::Arith(first, rest) => {
                write!(f, "({first}")?;
                for (op, n) in rest {
                    write!(f, " {} {n}", op.as_str())?;
                }
                f.write_str(")")
            }
            NodeKind::Logic(first, rest) => {
                write!(f, "({first}")?;
                for (op, n) in rest {
                    let op = match op {
                        LogicOp::And => "and",
                        LogicOp::Or => "or",
                    };
                    write!(f, " {op} {n}")?;
                }
                f.write_str(")")
            }
            NodeKind::Call {
                name, args, kwargs, ..
            } => {
                write!(f, "{name}(args=[")?;
                join(f, args.iter())?;
                f.write_str("], kwargs={")?;
                for (i, (k, n)) in kwargs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{k}={n}")?;
                }
                f.write_str("})")
            }
            NodeKind::Compare { lhs, op, rhs, lang } => match lang {
                Lang::Where => write!(f, "{lhs}{}{rhs}", op.as_str()),
                Lang::Expression => write!(f, "{lhs} {} {rhs}", op.as_str()),
            },
            NodeKind::Subset { items, dim } => {
                write!(f, "SUBSET:DIM:{dim}[")?;
                join(f, items.iter())?;
                f.write_str("]")
            }
        }
    }
}
