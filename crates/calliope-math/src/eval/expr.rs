//! Interpretation of [`Ir`] over the inputs and the components built so far.

use calliope_array::{Coord, Element, LArray, Label, Selection};
use calliope_schema::CalliopeBuildMath;
use indexmap::IndexMap;

use super::EvalError;
use super::inputs::{Inputs, lookup_default, numeric};
use super::lin::Lin;
use super::value::{Value, all_true, arith, compare, map_array, notnull};
use crate::helpers::Helper;
use crate::helpers::array as h;
use crate::ir::Ir;

/// Everything an expression or `where` string can refer to.
#[derive(Debug, Clone, Copy)]
pub struct Evaluator<'a> {
    /// Model inputs.
    pub inputs: &'a Inputs,
    /// The math (for defaults).
    pub math: &'a CalliopeBuildMath,
    /// The build configuration as JSON (`config.<key>`).
    pub config: &'a serde_json::Value,
    /// Variables and global expressions built so far.
    pub results: &'a IndexMap<String, Value>,
}

type Result<T> = std::result::Result<T, EvalError>;

fn full_size(dims: &[Coord]) -> usize {
    dims.iter().map(Coord::len).product()
}

/// `isnull().all()` → a NaN scalar, as `add_parameter`/`add_lookup` store empty inputs.
fn collapse(v: &Value) -> Value {
    let empty = match v {
        Value::Num(a) => a.count() == 0 && !a.dims().is_empty(),
        Value::Label(a) => a.count() == 0 && !a.dims().is_empty(),
        Value::Expr(a) => a.count() == 0 && !a.dims().is_empty(),
        _ => false,
    };
    if empty {
        Value::num(f64::NAN)
    } else {
        v.clone()
    }
}

/// `_apply_default`: fills missing cells (over the full product of the array's dims) with
/// the math default.
fn fill_default(v: Value, default: Option<&Value>) -> Value {
    let Some(d) = default else { return v };
    match (v, d) {
        (Value::Num(a), Value::Num(d)) if a.count() < full_size(a.dims()) => {
            let x = d.get(&[]);
            if x.is_nan() {
                Value::Num(a)
            } else {
                Value::Num(a.fillna(x))
            }
        }
        (Value::Expr(a), Value::Num(d)) if a.count() < full_size(a.dims()) => {
            let x = d.get(&[]);
            if x.is_nan() {
                Value::Expr(a)
            } else {
                Value::Expr(a.fillna(Lin::num(x)))
            }
        }
        (Value::Label(a), Value::Label(d)) if a.count() < full_size(a.dims()) => match d.get(&[]) {
            Some(l) => Value::Label(a.fillna(Some(l))),
            None => Value::Label(a),
        },
        (Value::Num(a), Value::Label(d))
            if a.count() == 0 && a.dims().is_empty() && d.get(&[]).is_some() =>
        {
            Value::Label(d.clone())
        }
        (v, _) => v,
    }
}

/// Names given as `over=dim` or `over=[dim, ...]`.
pub(crate) fn names_of(ir: &Ir) -> Vec<String> {
    match ir {
        Ir::Dim(n) | Ir::Str(n) | Ir::Result(n) | Ir::ResultPresence(n) | Ir::InputPresence(n) => {
            vec![n.clone()]
        }
        Ir::Input { name, .. } => vec![name.clone()],
        Ir::List(items) => items.iter().flat_map(names_of).collect(),
        _ => Vec::new(),
    }
}

impl Evaluator<'_> {
    /// The math default of an input or result, as a scalar value.
    fn default_of(&self, name: &str) -> Option<Value> {
        if let Some(p) = self.math.parameters.get(name) {
            Some(Value::num(numeric(&p.default)))
        } else if let Some(l) = self.math.lookups.get(name) {
            Some(lookup_default(&l.default))
        } else if let Some(v) = self.math.variables.get(name) {
            Some(Value::num(numeric(&v.default)))
        } else if let Some(g) = self.math.global_expressions.get(name) {
            Some(Value::num(numeric(&g.default)))
        } else {
            // `math.find` also searches postprocessed arrays (referenced by later ones).
            self.math
                .postprocessed
                .get(name)
                .map(|g| Value::num(numeric(&g.default)))
        }
    }

    /// An input as an expression operand: empty → NaN scalar, then the default filled in.
    pub fn input_values(&self, name: &str) -> Result<Value> {
        let v = self.inputs.get(name).ok_or_else(|| {
            EvalError(format!(
                "Trying to access a math component that is not yet defined: {name}."
            ))
        })?;
        Ok(fill_default(collapse(v), self.default_of(name).as_ref()))
    }

    /// A variable or global expression as an operand, the default filled in.
    pub fn result_values(&self, name: &str) -> Result<Value> {
        let v = self.results.get(name).ok_or_else(|| {
            EvalError(format!(
                "Trying to access a math component that is not yet defined: {name}. If the referenced component is a global expression, set its `order` to have it defined first."
            ))
        })?;
        Ok(fill_default(v.clone(), self.default_of(name).as_ref()))
    }

    fn config(&self, key: &str) -> Result<Value> {
        let mut node = self.config;
        for part in key.split('.') {
            node = node.get(part).unwrap_or(&serde_json::Value::Null);
        }
        match node {
            serde_json::Value::Bool(b) => Ok(Value::Bool(LArray::scalar(*b))),
            serde_json::Value::Number(n) => Ok(Value::num(n.as_f64().unwrap_or(f64::NAN))),
            serde_json::Value::String(s) => Ok(Value::Label(LArray::scalar(Some(Label::str(s))))),
            other => Err(EvalError(format!(
                "where string | Configuration option resolves to invalid type `{}`, expected a number, string, or boolean.",
                match other {
                    serde_json::Value::Null => "NoneType",
                    serde_json::Value::Array(_) => "list",
                    _ => "dict",
                }
            ))),
        }
    }

    /// Evaluates a node. `mask` is the equation's `where` (None: everywhere).
    pub fn eval(&self, ir: &Ir, mask: Option<&LArray<bool>>) -> Result<Value> {
        match ir {
            Ir::Const(x) => Ok(Value::num(*x)),
            Ir::Bool(b) => Ok(Value::Bool(LArray::scalar(*b))),
            Ir::Str(s) => Ok(Value::Str(s.clone())),
            Ir::List(items) => Ok(Value::List(
                items
                    .iter()
                    .map(|i| self.eval(i, mask))
                    .collect::<Result<_>>()?,
            )),
            Ir::Config(key) => self.config(key),
            Ir::Input { name, .. } => self.input_values(name),
            Ir::InputPresence(name) => Ok(Value::Bool(match self.inputs.get(name) {
                None => LArray::scalar(false),
                Some(Value::Bool(a)) => a.clone(),
                Some(Value::Num(a)) => a.map(|v| v.is_finite()).compact(),
                Some(Value::Label(a)) => notnull(a),
                Some(Value::Expr(a)) => notnull(a),
                Some(_) => LArray::scalar(true),
            })),
            Ir::Result(name) => self.result_values(name),
            Ir::ResultPresence(name) => Ok(Value::Bool(match self.results.get(name) {
                Some(Value::Expr(a)) => notnull(a),
                Some(Value::Num(a)) => notnull(a),
                _ => LArray::scalar(false),
            })),
            Ir::Dim(name) => Ok(match self.inputs.coords.get(name) {
                Some(c) => Value::Label(LArray::from_coord(c)),
                None => Value::num(f64::NAN),
            }),
            Ir::Select { array, dim, slicer } => {
                let a = self.eval(array, mask)?;
                let s = self.eval(slicer, None)?;
                select(&a, dim, &s)
            }
            Ir::Mask(x) => {
                let v = self.eval(x, mask)?;
                match mask {
                    Some(m) => v.mask(m),
                    None => Ok(v),
                }
            }
            Ir::Neg(x) => self.eval(x, mask)?.neg(),
            Ir::Arith(op, a, b) => arith(*op, &self.eval(a, mask)?, &self.eval(b, mask)?),
            Ir::Compare(op, a, b) => Ok(Value::Bool(compare(
                *op,
                &self.eval(a, mask)?,
                &self.eval(b, mask)?,
            )?)),
            Ir::Not(x) => Ok(Value::Bool(self.eval(x, mask)?.to_bool()?.not())),
            Ir::And(a, b) => {
                let (x, y) = (
                    self.eval(a, mask)?.to_bool()?,
                    self.eval(b, mask)?.to_bool()?,
                );
                Ok(Value::Bool(x.and(&y)))
            }
            Ir::Or(a, b) => {
                let (x, y) = (
                    self.eval(a, mask)?.to_bool()?,
                    self.eval(b, mask)?.to_bool()?,
                );
                Ok(Value::Bool(x.or(&y)))
            }
            Ir::Isin { dim, items } => {
                let labels: Vec<Label> = items
                    .iter()
                    .map(|i| self.eval(i, None))
                    .collect::<Result<Vec<_>>>()?
                    .iter()
                    .filter_map(Value::scalar_label)
                    .collect();
                Ok(Value::Bool(match self.inputs.coords.get(dim) {
                    Some(c) => LArray::from_coord(c).isin(&labels),
                    None => LArray::scalar(false),
                }))
            }
            Ir::Helper {
                helper,
                params,
                dim_kwargs,
            } => self.helper(*helper, params, dim_kwargs, mask),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn helper(
        &self,
        helper: Helper,
        params: &[(&'static str, Ir)],
        dim_kwargs: &[(String, Ir)],
        mask: Option<&LArray<bool>>,
    ) -> Result<Value> {
        let param = |n: &str| -> &Ir {
            params
                .iter()
                .find(|(p, _)| *p == n)
                .map(|(_, v)| v)
                .expect("bound by the signature")
        };
        let arg = |n: &str| self.eval(param(n), mask);
        let int_arg = |ir: &Ir| -> Result<f64> {
            self.eval(ir, mask)?
                .scalar_num()
                .ok_or_else(|| EvalError(format!("`{}` expects a number", helper.name())))
        };
        match helper {
            Helper::Any => {
                let a = arg("array")?;
                let Value::Bool(b) = &a else {
                    return Err(EvalError(format!(
                        "Input to `any` must be a boolean array. Received {} of dtype {}",
                        names_of(param("array")).join(""),
                        a.dtype()
                    )));
                };
                let over = present(&a, &names_of(param("over")));
                let refs: Vec<&str> = over.iter().map(String::as_str).collect();
                Ok(Value::Bool(b.any(&refs).map_err(err)?))
            }
            Helper::Sum => sum(&arg("array")?, &names_of(param("over"))),
            Helper::ReduceCarrierDim => {
                let a = arg("array")?;
                let dir = names_of(param("flow_direction")).join("");
                let carrier = match self.inputs.get(&format!("carrier_{dir}")) {
                    Some(c) => c.to_bool()?,
                    None => LArray::scalar(false),
                };
                sum(&a.mask(&carrier)?, &["carriers".to_owned()])
            }
            Helper::SelectFromLookupArrays => {
                let a = arg("array")?;
                let array_name = names_of(param("array")).join("");
                let mut lookups = Vec::new();
                for (dim, ir) in dim_kwargs {
                    let lname = names_of(ir).join("");
                    let l = match ir {
                        Ir::Input { name, .. } => self
                            .inputs
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| Value::num(f64::NAN)),
                        other => self.eval(other, mask)?,
                    };
                    lookups.push((dim.clone(), lname, l.to_label()?));
                }
                let refs: Vec<(String, String, &LArray<Option<Label>>)> = lookups
                    .iter()
                    .map(|(d, n, l)| (d.clone(), n.clone(), l))
                    .collect();
                map_array!(&a, |x| h::select_from_lookup_arrays(x, &array_name, &refs)
                    .map_err(|e| EvalError(e.to_string()))?)
            }
            Helper::GetValAtIndex => {
                let (dim, ir) = &dim_kwargs[0];
                #[allow(clippy::cast_possible_truncation)]
                let idx = int_arg(ir)?.trunc() as i64;
                let coord = self.inputs.coords.get(dim).ok_or_else(|| {
                    EvalError(format!("Unknown dimension `{dim}` in `get_val_at_index`"))
                })?;
                let l = h::get_val_at_index(coord, idx).map_err(|e| EvalError(e.to_string()))?;
                Ok(Value::Label(LArray::scalar(Some(l))))
            }
            Helper::Roll => {
                let a = arg("array")?;
                let shifts: Vec<(String, f64)> = dim_kwargs
                    .iter()
                    .map(|(d, ir)| Ok((d.clone(), int_arg(ir)?)))
                    .collect::<Result<_>>()?;
                map_array!(&a, |x| h::roll(x, &shifts)
                    .map_err(|e| EvalError(e.to_string()))?)
            }
            Helper::Where => {
                let a = arg("array")?;
                let cond = arg("condition")?.to_bool()?;
                match &a {
                    Value::Bool(_) => Ok(Value::Num(a.to_num()?.where_(&cond))),
                    v => map_array!(v, |x| x.where_(&cond)),
                }
            }
            Helper::MapDim => {
                let dim = names_of(param("dim")).join("");
                let coord =
                    self.inputs.coords.get(&dim).ok_or_else(|| {
                        EvalError(format!("Unknown dimension `{dim}` in `map_dim`"))
                    })?;
                let mapper = arg("mapper")?.to_label()?;
                Ok(Value::Bool(h::map_dim(coord, &mapper)))
            }
            Helper::GroupSum => {
                let a = arg("array")?;
                let groupby = arg("groupby")?.to_label()?;
                let group_dim = names_of(param("group_dim")).join("");
                match a {
                    Value::Expr(x) => Ok(Value::Expr(group_sum_expr(&x, &groupby, &group_dim)?)),
                    other => Ok(Value::Num(
                        h::group_sum(&other.to_num()?, &groupby, &group_dim)
                            .map_err(|e| EvalError(e.to_string()))?,
                    )),
                }
            }
            Helper::GroupDatetime => {
                let a = arg("array")?;
                let over = names_of(param("over")).join("");
                let group = names_of(param("group")).join("");
                let dtype = match self.math.dimensions.get(group.as_str()).map(|d| d.dtype) {
                    Some(calliope_schema::DimensionDtype::Integer) => h::GroupDtype::Integer,
                    _ => h::GroupDtype::String,
                };
                let coord = a
                    .dims()
                    .into_iter()
                    .find(|c| c.name() == over)
                    .ok_or_else(|| EvalError(format!("Dimension `{over}` not found")))?;
                let groupby = h::datetime_groupby(&coord, &group, dtype)
                    .map_err(|e| EvalError(e.to_string()))?;
                match a {
                    Value::Expr(x) => Ok(Value::Expr(group_sum_expr(&x, &groupby, &group)?)),
                    other => Ok(Value::Num(
                        h::group_sum(&other.to_num()?, &groupby, &group)
                            .map_err(|e| EvalError(e.to_string()))?,
                    )),
                }
            }
            Helper::SumNextN => {
                let a = arg("array")?;
                let over = names_of(param("over")).join("");
                let n = int_arg(param("N"))?;
                match a {
                    Value::Expr(x) => Ok(Value::Expr(sum_next_n_expr(&x, &over, n)?)),
                    other => Ok(Value::Num(
                        h::sum_next_n(&other.to_num()?, &over, n)
                            .map_err(|e| EvalError(e.to_string()))?,
                    )),
                }
            }
        }
    }

    /// The `where` mask of a component over its `foreach` dimensions
    /// (`combine_active_and_foreach`).
    pub fn foreach_mask(&self, foreach: &[String]) -> Result<LArray<bool>> {
        let active = match self.inputs.get("active") {
            Some(v) => v.to_bool()?,
            None => LArray::scalar(true),
        };
        let have: Vec<String> = active.dims().iter().map(|c| c.name().to_owned()).collect();
        let mut out = active;
        for d in foreach.iter().filter(|d| !have.contains(d)) {
            match self.inputs.coords.get(d) {
                Some(c) => out = out.and(&all_true(c)),
                None => return Ok(LArray::scalar(false)),
            }
        }
        Ok(out)
    }
}

fn err(e: calliope_array::ArrayError) -> EvalError {
    EvalError(e.to_string())
}

fn present(v: &Value, over: &[String]) -> Vec<String> {
    let dims = v.dim_names();
    over.iter().filter(|o| dims.contains(o)).cloned().collect()
}

/// `sum(array, over=...)`: over the dims the array has; NaN skipped; a group without values
/// is missing.
pub fn sum(v: &Value, over: &[String]) -> Result<Value> {
    let over = present(v, over);
    let refs: Vec<&str> = over.iter().map(String::as_str).collect();
    match v {
        Value::Expr(a) => Ok(Value::Expr(
            a.reduce(&refs, |cells| Lin::sum(cells.iter().copied()))
                .map_err(err)?,
        )),
        Value::Num(a) => Ok(Value::Num(a.sum(&refs, 1).map_err(err)?)),
        Value::Bool(_) => Ok(Value::Num(v.to_num()?.sum(&refs, 1).map_err(err)?)),
        other => Err(EvalError(format!("Cannot sum a {}", other.kind()))),
    }
}

/// `array.sel(dim=slicer)`.
pub fn select(a: &Value, dim: &str, slicer: &Value) -> Result<Value> {
    match slicer {
        Value::List(items) => {
            let labels: Vec<Label> = items.iter().filter_map(Value::scalar_label).collect();
            let sel = Selection::Labels(labels);
            map_array!(a, |x| x.sel(dim, &sel).map_err(err)?)
        }
        Value::Bool(mask) => {
            // Boolean indexing: the labels where the mask is true.
            let Some(coord) = a.dims().into_iter().find(|c| c.name() == dim) else {
                return Err(EvalError(format!("Dimension `{dim}` not found")));
            };
            let on: Vec<Label> = match mask.dims() {
                [m] if m.name() == dim => coord
                    .labels()
                    .iter()
                    .filter(|l| m.code(l).is_some_and(|c| mask.get(&[c])))
                    .cloned()
                    .collect(),
                _ => {
                    return Err(EvalError(format!(
                        "A boolean slice of `{dim}` must be indexed over `{dim}` only"
                    )));
                }
            };
            let sel = Selection::Labels(on);
            map_array!(a, |x| x.sel(dim, &sel).map_err(err)?)
        }
        s if s.dims().is_empty() => {
            let l = s
                .scalar_label()
                .ok_or_else(|| EvalError(format!("Cannot select `{dim}` with a missing value")))?;
            map_array!(a, |x| x.sel_one(dim, &l).map_err(err)?)
        }
        s => {
            let idx = s.to_label()?;
            map_array!(a, |x| sel_vectorized(x, dim, &idx)?)
        }
    }
}

/// Vectorised `sel`: the dimension is replaced by the indexer's dimensions.
fn sel_vectorized<T: Element>(
    a: &LArray<T>,
    dim: &str,
    idx: &LArray<Option<Label>>,
) -> Result<LArray<T>> {
    let ax = a.axis(dim).map_err(err)?;
    let coord = a.dims()[ax].clone();
    let mut dims: Vec<Coord> = a
        .dims()
        .iter()
        .filter(|c| c.name() != dim)
        .cloned()
        .collect();
    let shared: Vec<(usize, usize)> = idx
        .dims()
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            dims.iter()
                .position(|d| d.name() == c.name())
                .map(|p| (i, p))
        })
        .collect();
    let new: Vec<usize> = (0..idx.dims().len())
        .filter(|i| !shared.iter().any(|(s, _)| s == i))
        .collect();
    let base = dims.len();
    dims.extend(new.iter().map(|&i| idx.dims()[i].clone()));
    let mut by_code: std::collections::HashMap<u32, Vec<(Vec<u32>, T)>> = Default::default();
    for (mut codes, v) in a.cells() {
        let c = codes.remove(ax);
        by_code.entry(c).or_default().push((codes, v.clone()));
    }
    let mut cells = Vec::new();
    for (icodes, l) in idx.cells() {
        let Some(l) = l else { continue };
        let c = coord.code(l).ok_or_else(|| {
            EvalError(format!(
                "KeyError: `{l}` is not a label of dimension `{dim}`"
            ))
        })?;
        for (rest, v) in by_code.get(&c).into_iter().flatten() {
            if shared.iter().any(|&(i, p)| icodes[i] != rest[p]) {
                continue;
            }
            let mut out = rest.clone();
            out.resize(base, 0);
            out.extend(new.iter().map(|&i| icodes[i]));
            cells.push((out, v.clone()));
        }
    }
    LArray::from_cells(dims, cells).map_err(err)
}

fn group_sum_expr(
    a: &LArray<Lin>,
    groupby: &LArray<Option<Label>>,
    group_dim: &str,
) -> Result<LArray<Lin>> {
    let g_axes: Vec<usize> = groupby
        .dims()
        .iter()
        .map(|d| a.axis(d.name()))
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    let mut labels: Vec<Label> = groupby.cells().filter_map(|(_, v)| v.clone()).collect();
    labels.sort();
    labels.dedup();
    let gcoord = Coord::new(group_dim, labels).map_err(err)?;
    let keep: Vec<usize> = (0..a.dims().len())
        .filter(|i| !g_axes.contains(i))
        .collect();
    let mut groups: std::collections::BTreeMap<Vec<u32>, Lin> = Default::default();
    for (codes, v) in a.cells() {
        let gcodes: Vec<u32> = g_axes.iter().map(|&ax| codes[ax]).collect();
        let Some(label) = groupby.get(&gcodes) else {
            continue;
        };
        let g = gcoord.code(&label).expect("collected");
        let mut key = vec![g];
        key.extend(keep.iter().map(|&i| codes[i]));
        let e = groups.entry(key).or_insert_with(|| Lin::num(0.0));
        *e = e.add(v);
    }
    let mut dims = vec![gcoord];
    dims.extend(keep.iter().map(|&i| a.dims()[i].clone()));
    LArray::from_cells(dims, groups).map_err(err)
}

fn sum_next_n_expr(a: &LArray<Lin>, over: &str, n: f64) -> Result<LArray<Lin>> {
    let ax = a.axis(over).map_err(err)?;
    let len = a.dims()[ax].len();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = if n <= 0.0 { 0 } else { n.trunc() as usize };
    let mut rows: std::collections::BTreeMap<Vec<u32>, Vec<Option<Lin>>> = Default::default();
    for (codes, v) in a.cells() {
        let mut key = codes.clone();
        key.remove(ax);
        rows.entry(key).or_insert_with(|| vec![None; len])[codes[ax] as usize] = Some(v.clone());
    }
    let mut cells = Vec::new();
    for (key, row) in rows {
        for i in 0..len {
            let window: Vec<&Lin> = row[i..(i + n).min(len)].iter().flatten().collect();
            if !window.is_empty() {
                let mut codes = key.clone();
                codes.insert(ax, u32::try_from(i).expect("fits"));
                cells.push((codes, Lin::sum(window)));
            }
        }
    }
    LArray::from_cells(a.dims().to_vec(), cells).map_err(err)
}
