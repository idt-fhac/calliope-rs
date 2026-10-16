//! Evaluated values: labelled arrays of numbers, booleans, labels or affine expressions.

use calliope_array::{Coord, Element, Join, LArray, Label};

use super::EvalError;
use super::lin::Lin;
use crate::ast::{ArithOp, CmpOp};

/// The value of an evaluated IR node.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Numbers (`float64`, NaN = missing).
    Num(LArray<f64>),
    /// Booleans (absent = false, as numpy bool arrays have no missing state).
    Bool(LArray<bool>),
    /// Labels (strings, datetimes; `object`/`datetime64` arrays).
    Label(LArray<Option<Label>>),
    /// Affine expressions over decision variables (Pyomo object arrays).
    Expr(LArray<Lin>),
    /// A generic string (dimension item, helper option).
    Str(String),
    /// A list of values.
    List(Vec<Value>),
}

/// Applies a generic array function to whichever array variant a value holds.
macro_rules! map_array {
    ($v:expr, |$a:ident| $body:expr) => {
        match $v {
            Value::Num($a) => Ok(Value::Num($body)),
            Value::Bool($a) => Ok(Value::Bool($body)),
            Value::Label($a) => Ok(Value::Label($body)),
            Value::Expr($a) => Ok(Value::Expr($body)),
            other => Err(EvalError(format!(
                "Expected an array, got {}",
                other.kind()
            ))),
        }
    };
}
pub(crate) use map_array;

impl Value {
    /// A number.
    pub fn num(x: f64) -> Value {
        Value::Num(LArray::scalar(x))
    }

    /// What kind of value this is (for messages).
    pub fn kind(&self) -> &'static str {
        match self {
            Value::Num(_) => "numeric array",
            Value::Bool(_) => "boolean array",
            Value::Label(_) => "label array",
            Value::Expr(_) => "expression array",
            Value::Str(_) => "string",
            Value::List(_) => "list",
        }
    }

    /// The numpy dtype name Calliope would print.
    pub fn dtype(&self) -> &'static str {
        match self {
            Value::Num(_) => "float64",
            Value::Bool(_) => "bool",
            Value::Label(_) | Value::Expr(_) => "object",
            Value::Str(_) => "<U",
            Value::List(_) => "list",
        }
    }

    /// The dimensions (empty for non-arrays).
    pub fn dims(&self) -> Vec<Coord> {
        match self {
            Value::Num(a) => a.dims().to_vec(),
            Value::Bool(a) => a.dims().to_vec(),
            Value::Label(a) => a.dims().to_vec(),
            Value::Expr(a) => a.dims().to_vec(),
            Value::Str(_) | Value::List(_) => Vec::new(),
        }
    }

    /// The dimension names.
    pub fn dim_names(&self) -> Vec<String> {
        self.dims().iter().map(|c| c.name().to_owned()).collect()
    }

    /// As numbers: booleans become 0/1 (over the full product, as numpy casts them).
    pub fn to_num(&self) -> Result<LArray<f64>, EvalError> {
        match self {
            Value::Num(a) => Ok(a.clone()),
            Value::Bool(a) => Ok(crate::helpers::array::bool_to_float(a)),
            other => Err(EvalError(format!(
                "Cannot use a {} as a number",
                other.kind()
            ))),
        }
    }

    /// As expressions (numbers become constant expressions).
    pub fn to_expr(&self) -> Result<LArray<Lin>, EvalError> {
        match self {
            Value::Expr(a) => Ok(a.clone()),
            other => Ok(other.to_num()?.map(|x| Lin::num(*x))),
        }
    }

    /// As a boolean array: numbers are true where non-zero and not NaN, labels where present.
    pub fn to_bool(&self) -> Result<LArray<bool>, EvalError> {
        match self {
            Value::Bool(a) => Ok(a.clone()),
            Value::Num(a) => Ok(a.map(|v| !v.is_nan() && *v != 0.0).compact()),
            Value::Label(a) => Ok(a.map(Option::is_some).compact()),
            Value::Str(s) => Ok(LArray::scalar(!s.is_empty())),
            other => Err(EvalError(format!(
                "Cannot use a {} as a condition",
                other.kind()
            ))),
        }
    }

    /// As labels (strings and numbers become scalar labels).
    pub fn to_label(&self) -> Result<LArray<Option<Label>>, EvalError> {
        match self {
            Value::Label(a) => Ok(a.clone()),
            Value::Str(s) => Ok(LArray::scalar(Some(Label::str(s)))),
            Value::Num(a) => Ok(a.map(|v| (!v.is_nan()).then_some(Label::Float(*v)))),
            other => Err(EvalError(format!(
                "Cannot use a {} as labels",
                other.kind()
            ))),
        }
    }

    /// The single label of a scalar value (a string, number or 0-d label array).
    pub fn scalar_label(&self) -> Option<Label> {
        match self {
            Value::Str(s) => Some(Label::str(s)),
            Value::Num(a) if a.dims().is_empty() => {
                let v = a.get(&[]);
                (!v.is_nan()).then_some(Label::Float(v))
            }
            Value::Label(a) if a.dims().is_empty() => a.get(&[]),
            Value::Bool(a) if a.dims().is_empty() => Some(Label::Int(i64::from(a.get(&[])))),
            _ => None,
        }
    }

    /// The single number of a scalar value.
    pub fn scalar_num(&self) -> Option<f64> {
        match self {
            Value::Num(a) if a.dims().is_empty() => Some(a.get(&[])),
            Value::Bool(a) if a.dims().is_empty() => Some(f64::from(u8::from(a.get(&[])))),
            _ => None,
        }
    }

    /// True if any cell is defined (non-missing; for booleans: true).
    pub fn any_defined(&self) -> bool {
        match self {
            Value::Num(a) => a.count() > 0,
            Value::Bool(a) => a.count() > 0,
            Value::Label(a) => a.count() > 0,
            Value::Expr(a) => a.count() > 0,
            Value::Str(_) | Value::List(_) => true,
        }
    }

    /// `array.broadcast_like(where).where(where)` (`_apply_where_array`).
    pub fn mask(&self, mask: &LArray<bool>) -> Result<Value, EvalError> {
        match self {
            Value::Bool(_) => Ok(Value::Num(self.to_num()?.where_(mask))),
            Value::Str(_) | Value::List(_) => Ok(self.clone()),
            v => map_array!(v, |a| a.where_(mask)),
        }
    }

    /// `-1 * x`.
    pub fn neg(&self) -> Result<Value, EvalError> {
        match self {
            Value::Expr(a) => Ok(Value::Expr(a.map(|l| l.scale(-1.0)))),
            other => Ok(Value::Num(other.to_num()?.map(|x| -x))),
        }
    }
}

/// `a op b` with xarray broadcasting by name; a missing operand gives a missing result.
/// `/` multiplies by the reciprocal, as Calliope does.
pub fn arith(op: ArithOp, a: &Value, b: &Value) -> Result<Value, EvalError> {
    if !matches!(a, Value::Expr(_)) && !matches!(b, Value::Expr(_)) {
        let (x, y) = (a.to_num()?, b.to_num()?);
        let f: fn(f64, f64) -> f64 = match op {
            ArithOp::Add => |x, y| x + y,
            ArithOp::Sub => |x, y| x - y,
            ArithOp::Mul => |x, y| x * y,
            ArithOp::Div => |x, y| x * (1.0 / y),
            ArithOp::Pow => f64::powf,
        };
        return Ok(Value::Num(
            x.zip_with(&y, Join::Inner, |p, q| f(*p, *q)).compact(),
        ));
    }
    let (x, y) = (a.to_expr()?, b.to_expr()?);
    let nonlinear = std::cell::Cell::new(false);
    let out = match op {
        ArithOp::Add => x.zip_with(&y, Join::Inner, |p, q| p.add(q)),
        ArithOp::Sub => x.zip_with(&y, Join::Inner, |p, q| p.sub(q)),
        ArithOp::Mul => x.zip_with(&y, Join::Inner, |p, q| {
            p.mul(q).unwrap_or_else(|| {
                nonlinear.set(true);
                Lin::missing()
            })
        }),
        ArithOp::Div => x.zip_with(&y, Join::Inner, |p, q| {
            if q.is_expr() {
                nonlinear.set(true);
                Lin::missing()
            } else {
                p.scale(1.0 / q.constant)
            }
        }),
        ArithOp::Pow => x.zip_with(&y, Join::Inner, |p, q| {
            if p.is_expr() || q.is_expr() {
                nonlinear.set(true);
                Lin::missing()
            } else {
                Lin::num(p.constant.powf(q.constant))
            }
        }),
    };
    if nonlinear.get() {
        return Err(EvalError(format!(
            "Non-linear operation `{}` between decision variables or expressions; only linear math can be built",
            op.as_str()
        )));
    }
    Ok(Value::Expr(out.compact()))
}

/// A comparison in a `where` string: true where both sides are defined and the comparison
/// holds (NaN compares false).
pub fn compare(op: CmpOp, a: &Value, b: &Value) -> Result<LArray<bool>, EvalError> {
    let numeric = |v: &Value| matches!(v, Value::Num(_) | Value::Bool(_));
    if numeric(a) && numeric(b) {
        let (x, y) = (a.to_num()?, b.to_num()?);
        return Ok(x
            .zip_with(&y, Join::Inner, |p, q| cmp_f64(op, *p, *q))
            .compact());
    }
    let (x, y) = (as_labels(a)?, as_labels(b)?);
    Ok(x.zip_with(&y, Join::Inner, |p, q| match (p, q) {
        (Some(p), Some(q)) => cmp_label(op, p, q),
        _ => false,
    })
    .compact())
}

fn as_labels(v: &Value) -> Result<LArray<Option<Label>>, EvalError> {
    match v {
        Value::Bool(a) => Ok(a.map(|b| Some(Label::Int(i64::from(*b))))),
        other => other.to_label(),
    }
}

fn cmp_f64(op: CmpOp, a: f64, b: f64) -> bool {
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Le => a <= b,
        CmpOp::Ge => a >= b,
        CmpOp::Lt => a < b,
        CmpOp::Gt => a > b,
    }
}

fn cmp_label(op: CmpOp, a: &Label, b: &Label) -> bool {
    let comparable = matches!(
        (a, b),
        (Label::Str(_), Label::Str(_))
            | (Label::Datetime(_), Label::Datetime(_))
            | (
                Label::Int(_) | Label::Float(_),
                Label::Int(_) | Label::Float(_)
            )
    );
    if !comparable {
        return false;
    }
    match op {
        CmpOp::Eq => a == b,
        CmpOp::Le => a <= b,
        CmpOp::Ge => a >= b,
        CmpOp::Lt => a < b,
        CmpOp::Gt => a > b,
    }
}

/// A boolean array that is true over every label of `coord`.
pub fn all_true(coord: &Coord) -> LArray<bool> {
    let cells: Vec<(Vec<u32>, bool)> = (0..coord.len())
        .map(|i| (vec![u32::try_from(i).expect("fits")], true))
        .collect();
    LArray::from_cells(vec![coord.clone()], cells).expect("valid cells")
}

/// True where an array has a defined (non-missing) value.
pub fn notnull<T: Element>(a: &LArray<T>) -> LArray<bool> {
    a.map(|v| !v.is_missing()).compact()
}
