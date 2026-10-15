//! The evaluation IR: parsed components lowered to vectorised operations, with Calliope's
//! implicit evaluation rules made explicit nodes.
//!
//! Calliope spreads these rules over `expression_parser.py`, `where_parser.py`, `parsing.py`
//! and `backend_model.py`; here they are decided once, at lowering time:
//!
//! - **Defaults.** A component referenced by name has its NaN cells filled with the math
//!   default ([`Ir::Input`]`.default`; `EvalArrayOrMath._apply_default`, and
//!   `InputArrayParser` inside `where` comparisons). Derived arrays are never filled.
//! - **Presence.** In a `where` condition (outside comparisons) an input becomes "defined,
//!   not NaN and not ±inf" ([`Ir::InputPresence`]; bool lookups are taken as they are) and a
//!   result becomes "defined" ([`Ir::ResultPresence`]).
//! - **Masking.** Each operand of a binary arithmetic operation is masked with the
//!   equation's `where` ([`Ir::Mask`]; `_apply_where_array`), and so is the final
//!   expression, except inside the arguments of helpers with `ignore_where` (`roll`,
//!   `where`, `select_from_lookup_arrays`, `map_dim`), whose operands see unmasked data.
//! - **Component masks** ([`ComponentMask`]): `active` × "item defined" for every other
//!   `foreach` dimension × top-level `where`, reduced to the `foreach` dimensions with
//!   "any"; each equation then ANDs its own `where` strings and reduces again
//!   (`backend_model._eval_equations`).
//!
//! The evaluator (next step) interprets [`Ir`] over `LArray`s, with affine expressions as a
//! further value kind, and must check what `_eval_equations` checks: equation masks are
//! disjoint ("trying to set two equations for the same index"), and the expression is
//! defined wherever its mask is true ("Missing a linear expression for some coordinates
//! selected by 'where'").

use calliope_schema::{CalliopeBuildMath, LookupDefault, Numeric};

use crate::ast::Lang;
use crate::ast::{ArithOp, CmpOp, LogicOp, Node, NodeKind, Role, Sign};
use crate::component::{Equation, Group, ParsedComponent, ParsedString};
use crate::helpers::{Helper, bind};

/// A lowered expression or condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Ir {
    /// A number.
    Const(f64),
    /// A boolean.
    Bool(bool),
    /// A string (dimension item, helper option).
    Str(String),
    /// A list literal.
    List(Vec<Ir>),
    /// `config.<key>` from the build configuration.
    Config(String),
    /// An input's values, NaN filled with `default` if it has one.
    Input {
        /// The input.
        name: String,
        /// The math default, if not NaN.
        default: Option<f64>,
    },
    /// An input as a condition: defined, not NaN, not ±inf (bool lookups as they are;
    /// an input missing from the data is false).
    InputPresence(String),
    /// A decision variable or global expression (postprocessing: its values).
    Result(String),
    /// A result as a condition: defined.
    ResultPresence(String),
    /// A dimension's items as an array.
    Dim(String),
    /// `array.sel(dim=slicer)`: a scalar drops the dimension, a list keeps it, an array
    /// indexes vectorised.
    Select {
        /// The array.
        array: Box<Ir>,
        /// The dimension.
        dim: String,
        /// The selection.
        slicer: Box<Ir>,
    },
    /// The array broadcast over the equation's `where` and NaN where it is false.
    Mask(Box<Ir>),
    /// `-x`.
    Neg(Box<Ir>),
    /// `a op b` (`/` is multiplication by the reciprocal, as in Calliope).
    Arith(ArithOp, Box<Ir>, Box<Ir>),
    /// `a op b` in a `where` string.
    Compare(CmpOp, Box<Ir>, Box<Ir>),
    /// `not x`.
    Not(Box<Ir>),
    /// `a and b`.
    And(Box<Ir>, Box<Ir>),
    /// `a or b`.
    Or(Box<Ir>, Box<Ir>),
    /// `[items] in dim`.
    Isin {
        /// The dimension.
        dim: String,
        /// The items.
        items: Vec<Ir>,
    },
    /// A helper call with its arguments bound to its parameters.
    Helper {
        /// The helper.
        helper: Helper,
        /// Named parameters in signature order.
        params: Vec<(&'static str, Ir)>,
        /// `**kwargs` by dimension.
        dim_kwargs: Vec<(String, Ir)>,
    },
}

/// Where a component is built: `active` × defined `foreach` items × top-level `where`,
/// reduced to `foreach` with "any" over other dimensions.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentMask {
    /// The `foreach` dimensions, in order (the order of LP rows/columns).
    pub foreach: Vec<String>,
    /// The top-level `where` condition.
    pub where_: Ir,
}

/// What an equation builds.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// A global expression, objective or postprocessed array.
    Expression(Ir),
    /// A constraint `lhs op rhs`.
    Constraint {
        /// Left-hand side (masked).
        lhs: Ir,
        /// `<=`, `>=` or `==`.
        op: CmpOp,
        /// Right-hand side (masked).
        rhs: Ir,
    },
}

/// One expanded equation.
#[derive(Debug, Clone, PartialEq)]
pub struct LoweredEquation {
    /// Calliope's equation name.
    pub name: String,
    /// The equation's own `where` conditions (equation, sub-expressions, slices), ANDed with
    /// the component mask.
    pub wheres: Vec<Ir>,
    /// What it builds.
    pub body: Body,
}

/// A lowered component.
#[derive(Debug, Clone, PartialEq)]
pub struct LoweredComponent {
    /// The group.
    pub group: Group,
    /// The name.
    pub name: String,
    /// Where it is built.
    pub mask: ComponentMask,
    /// Its equations (empty for variables and checks).
    pub equations: Vec<LoweredEquation>,
}

/// Lowers a parsed component. The component must have parsed and typechecked; helper
/// argument errors are reported by the typecheck and lowered as `Ir::Const(NaN)` here.
pub fn lower(comp: &ParsedComponent, math: &CalliopeBuildMath) -> LoweredComponent {
    let l = Lowerer { math };
    let equations = comp.equations.iter().map(|eq| l.equation(eq)).collect();
    LoweredComponent {
        group: comp.group,
        name: comp.name.clone(),
        mask: ComponentMask {
            foreach: comp.foreach.clone(),
            where_: l.where_(&comp.where_),
        },
        equations,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Expressions: operands are masked.
    Masked,
    /// Inside an `ignore_where` helper: no masking.
    Unmasked,
    /// `where` condition: names are presence tests.
    Presence,
    /// Inside a `where` comparison: names are values.
    Values,
}

struct Lowerer<'a> {
    math: &'a CalliopeBuildMath,
}

impl Lowerer<'_> {
    fn where_(&self, src: &ParsedString) -> Ir {
        self.node(&src.node, None, Scope::Presence)
    }

    fn equation(&self, eq: &Equation) -> LoweredEquation {
        let wheres = eq.wheres.iter().map(|w| self.where_(w)).collect();
        let body = match &eq.expression.node.kind {
            NodeKind::Compare { lhs, op, rhs, .. } => Body::Constraint {
                lhs: Ir::Mask(Box::new(self.node(lhs, Some(eq), Scope::Masked))),
                op: *op,
                rhs: Ir::Mask(Box::new(self.node(rhs, Some(eq), Scope::Masked))),
            },
            _ => Body::Expression(Ir::Mask(Box::new(self.node(
                &eq.expression.node,
                Some(eq),
                Scope::Masked,
            )))),
        };
        LoweredEquation {
            name: eq.name.clone(),
            wheres,
            body,
        }
    }

    fn default(&self, name: &str) -> Option<f64> {
        let v = if let Some(p) = self.math.parameters.get(name) {
            numeric(&p.default)
        } else if let Some(l) = self.math.lookups.get(name) {
            match &l.default {
                LookupDefault::Number(n) => numeric(n),
                LookupDefault::Bool(b) => f64::from(u8::from(*b)),
                LookupDefault::Str(_) => f64::NAN,
            }
        } else if let Some(g) = self.math.global_expressions.get(name) {
            numeric(&g.default)
        } else if let Some(v) = self.math.variables.get(name) {
            numeric(&v.default)
        } else {
            f64::NAN
        };
        (!v.is_nan()).then_some(v)
    }

    fn operand(&self, n: &Node, eq: Option<&Equation>, scope: Scope) -> Ir {
        let ir = self.node(n, eq, scope);
        if scope == Scope::Masked {
            Ir::Mask(Box::new(ir))
        } else {
            ir
        }
    }

    #[allow(clippy::too_many_lines)]
    fn node(&self, n: &Node, eq: Option<&Equation>, scope: Scope) -> Ir {
        match &n.kind {
            NodeKind::Number(x) => Ir::Const(x.value),
            NodeKind::Bool(b) => Ir::Bool(*b),
            NodeKind::Str(s) => Ir::Str(s.clone()),
            NodeKind::Config(k) => Ir::Config(k.clone()),
            NodeKind::List(items) => {
                Ir::List(items.iter().map(|i| self.node(i, eq, scope)).collect())
            }
            NodeKind::Ref { name, role, .. } => match (role, scope) {
                (Role::Dimension, _) => Ir::Dim(name.clone()),
                (Role::Input, Scope::Presence) => Ir::InputPresence(name.clone()),
                (Role::Input, _) => Ir::Input {
                    name: name.clone(),
                    default: self.default(name),
                },
                (Role::Result | Role::Postprocessed, Scope::Presence) => {
                    Ir::ResultPresence(name.clone())
                }
                (Role::Result | Role::Postprocessed, _) => Ir::Result(name.clone()),
            },
            NodeKind::SubExpr(name) => match eq.and_then(|e| e.sub_expressions.get(name)) {
                Some(s) => self.node(&s.node, eq, scope),
                None => Ir::Const(f64::NAN),
            },
            NodeKind::SliceRef(name) => match eq.and_then(|e| e.slices.get(name)) {
                // Slices are evaluated as values (`as_values=True`), never masked.
                Some(s) => self.node(&s.node, eq, Scope::Unmasked),
                None => Ir::Const(f64::NAN),
            },
            NodeKind::Sliced { name, role, slices } => {
                let base = Node::new(
                    NodeKind::Ref {
                        name: name.clone(),
                        role: *role,
                        lang: Lang::Expression,
                    },
                    n.span.clone(),
                );
                let mut ir = self.node(&base, eq, scope);
                for (dim, slicer) in slices {
                    ir = Ir::Select {
                        array: Box::new(ir),
                        dim: dim.clone(),
                        slicer: Box::new(self.node(slicer, eq, Scope::Unmasked)),
                    };
                }
                ir
            }
            NodeKind::Sign(Sign::Plus, x) => self.node(x, eq, scope),
            NodeKind::Sign(Sign::Minus, x) => Ir::Neg(Box::new(self.node(x, eq, scope))),
            NodeKind::Arith(first, rest) => {
                let mut acc = self.operand(first, eq, scope);
                for (op, x) in rest {
                    acc = Ir::Arith(*op, Box::new(acc), Box::new(self.operand(x, eq, scope)));
                }
                acc
            }
            NodeKind::Not(x) => Ir::Not(Box::new(self.node(x, eq, scope))),
            NodeKind::Logic(first, rest) => {
                let mut acc = self.node(first, eq, scope);
                for (op, x) in rest {
                    let rhs = Box::new(self.node(x, eq, scope));
                    acc = match op {
                        LogicOp::And => Ir::And(Box::new(acc), rhs),
                        LogicOp::Or => Ir::Or(Box::new(acc), rhs),
                    };
                }
                acc
            }
            NodeKind::Compare { lhs, op, rhs, .. } => Ir::Compare(
                *op,
                Box::new(self.node(lhs, eq, Scope::Values)),
                Box::new(self.node(rhs, eq, Scope::Values)),
            ),
            NodeKind::Subset { items, dim } => Ir::Isin {
                dim: dim.clone(),
                items: items
                    .iter()
                    .map(|i| self.node(i, eq, Scope::Values))
                    .collect(),
            },
            NodeKind::Call {
                name, args, kwargs, ..
            } => {
                let lang = if matches!(scope, Scope::Presence | Scope::Values) {
                    Lang::Where
                } else {
                    Lang::Expression
                };
                let Some(helper) = Helper::lookup(name, lang) else {
                    return Ir::Const(f64::NAN);
                };
                let inner = if helper.ignore_where() && scope == Scope::Masked {
                    Scope::Unmasked
                } else {
                    scope
                };
                let lowered_args = args.iter().map(|a| self.node(a, eq, inner)).collect();
                let lowered_kwargs = kwargs
                    .iter()
                    .map(|(k, v)| (k.clone(), self.node(v, eq, inner)))
                    .collect();
                match bind(helper, lowered_args, lowered_kwargs) {
                    Ok(b) => Ir::Helper {
                        helper,
                        params: b.params,
                        dim_kwargs: b.dim_kwargs,
                    },
                    Err(_) => Ir::Const(f64::NAN),
                }
            }
        }
    }
}

fn numeric(n: &Numeric) -> f64 {
    match n {
        #[allow(clippy::cast_precision_loss)]
        Numeric::Int(i) => *i as f64,
        Numeric::Float(f) => *f,
    }
}
