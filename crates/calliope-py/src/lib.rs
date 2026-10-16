//! Python bindings: the native module `calliope_rs._core` behind the `calliope_rs` package.
//!
//! The Rust side exchanges plain Python objects with the package's Python layer
//! (`python/calliope_rs/__init__.py`), which turns them into `xarray`/`pandas` objects:
//!
//! - a **dataset** is `{"coords": {dim: (kind, labels)}, "vars": [(name, dims, dtype,
//!   payload)]}`;
//! - an **array** is `{"coords": {dim: (kind, labels)}, "dims": [...], "dtype": ..,
//!   "data": payload}`;
//! - `kind` is `"datetime"` (labels are nanoseconds since the epoch) or `"object"`
//!   (Python `str`/`int`/`float` labels);
//! - `dtype` is a numpy type string: `f8`, `i8` and `?` (payload: the C-order values as
//!   little-endian bytes), `M8[ns]` (int64 bytes, NaT = `i64::MIN`) or `O` (payload: a
//!   flat list of Python objects, NaN for missing);
//! - an in-memory **data table** (`data_table_dfs`) is `{"index_names": [...],
//!   "index": [(kind, labels) per level], "column_names": [...], "columns": [(kind,
//!   labels) per level], "data": [(dtype, payload) per column]}`.
//!
//! Config, definition and math cross as nested `dict`/`list`/scalars.
//!
//! Building (abi3 wheel, Python >= 3.11) and testing, from this directory, in a virtual
//! environment with numpy, pandas, xarray, netCDF4 and pytest:
//!
//! ```sh
//! maturin develop        # or: maturin build --release
//! pytest                 # tests/: needs ../calliope (or $CALLIOPE_REF) and golden/
//! ```

use std::path::PathBuf;

use calliope_array::{Coord, LArray, Label};
use calliope_io::AttrValue;
use calliope_math::Group;
use calliope_math::eval::Value;
use calliope_model::{BuildOptions, Error, Model, Results, SolveOptions};
use calliope_preprocess::ReadOptions;
use calliope_preprocess::table::Frame;
use calliope_preprocess::xr::{Data, Dataset, Obj, Var};
use calliope_yaml::{AttrTree, Node, Value as YValue};
use indexmap::IndexMap;
use pyo3::IntoPyObjectExt;
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyKeyError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyInt, PyList, PySet, PyString, PyTuple};

create_exception!(
    _core,
    ModelError,
    PyException,
    "calliope.exceptions.ModelError"
);
create_exception!(
    _core,
    BackendError,
    PyException,
    "calliope.exceptions.BackendError"
);

fn to_py_err(e: Error) -> PyErr {
    match e {
        Error::Model(m) => ModelError::new_err(m),
        Error::Backend(m) => BackendError::new_err(m),
        Error::Key(m) => PyKeyError::new_err(m),
        Error::Value(m) => PyValueError::new_err(m),
        Error::Io(e) => PyOSError::new_err(e.to_string()),
    }
}

trait OrPy<T> {
    fn py(self) -> PyResult<T>;
}

impl<T> OrPy<T> for Result<T, Error> {
    fn py(self) -> PyResult<T> {
        self.map_err(to_py_err)
    }
}

impl<T> OrPy<T> for Result<T, calliope_array::ArrayError> {
    fn py(self) -> PyResult<T> {
        self.map_err(|e| PyValueError::new_err(e.to_string()))
    }
}

// ---- YAML-like trees -------------------------------------------------------------------

fn node_to_py<'py>(py: Python<'py>, n: &Node) -> PyResult<Bound<'py, PyAny>> {
    match &n.value {
        YValue::Null => Ok(py.None().into_bound(py)),
        YValue::Bool(b) => b.into_bound_py_any(py),
        YValue::Int(i) => i.into_bound_py_any(py),
        YValue::Float(f) => f.into_bound_py_any(py),
        YValue::Str(s) | YValue::Date(s) | YValue::DateTime(s) => s.into_bound_py_any(py),
        YValue::Seq(items) => {
            let l = PyList::empty(py);
            for i in items {
                l.append(node_to_py(py, i)?)?;
            }
            Ok(l.into_any())
        }
        YValue::Map(t) => Ok(tree_to_py(py, t)?.into_any()),
    }
}

fn tree_to_py<'py>(py: Python<'py>, t: &AttrTree) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for (k, v) in t.iter() {
        d.set_item(k, node_to_py(py, v)?)?;
    }
    Ok(d)
}

fn py_to_node(obj: &Bound<'_, PyAny>) -> PyResult<Node> {
    let v = if obj.is_none() {
        YValue::Null
    } else if obj.is_instance_of::<PyBool>() {
        YValue::Bool(obj.extract()?)
    } else if obj.is_instance_of::<PyInt>() {
        YValue::Int(obj.extract()?)
    } else if obj.is_instance_of::<PyFloat>() {
        YValue::Float(obj.extract()?)
    } else if obj.is_instance_of::<PyString>() {
        YValue::Str(obj.extract()?)
    } else if let Ok(d) = obj.cast::<PyDict>() {
        YValue::Map(py_to_tree(d)?)
    } else if obj.is_instance_of::<PyList>()
        || obj.is_instance_of::<PyTuple>()
        || obj.is_instance_of::<PySet>()
    {
        let mut items = Vec::new();
        for i in obj.try_iter()? {
            items.push(py_to_node(&i?)?);
        }
        YValue::Seq(items)
    } else if obj.hasattr("__fspath__")? {
        YValue::Str(obj.call_method0("__fspath__")?.extract()?)
    } else if let Ok(b) = obj.extract::<bool>()
        && obj.get_type().name()?.to_string().starts_with("bool")
    {
        YValue::Bool(b)
    } else if let Ok(i) = obj.extract::<i64>() {
        YValue::Int(i)
    } else if let Ok(f) = obj.extract::<f64>() {
        YValue::Float(f)
    } else {
        return Err(PyTypeError::new_err(format!(
            "Cannot convert {} to a configuration value",
            obj.repr()?
        )));
    };
    Ok(Node::new(v))
}

/// A Python mapping as a tree; dotted keys are nested, as `AttrDict` does.
fn py_to_tree(d: &Bound<'_, PyDict>) -> PyResult<AttrTree> {
    let mut t = AttrTree::new();
    for (k, v) in d.iter() {
        let key: String = k.str()?.extract()?;
        t.set_key(&key, py_to_node(&v)?)
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
    }
    Ok(t)
}

fn opt_tree(d: Option<&Bound<'_, PyDict>>) -> PyResult<Option<AttrTree>> {
    d.map(py_to_tree).transpose()
}

fn attr_to_py<'py>(py: Python<'py>, a: &AttrValue) -> PyResult<Bound<'py, PyAny>> {
    match a {
        AttrValue::Str(s) => s.into_bound_py_any(py),
        AttrValue::Int(i) => i.into_bound_py_any(py),
        AttrValue::Float(f) => f.into_bound_py_any(py),
        AttrValue::Bool(b) => b.into_bound_py_any(py),
        AttrValue::None => Ok(py.None().into_bound(py)),
        AttrValue::List(l) => Ok(PyList::new(py, l)?.into_any()),
        AttrValue::Set(l) => Ok(PySet::new(py, l)?.into_any()),
        AttrValue::Dict(t) => Ok(tree_to_py(py, t)?.into_any()),
        AttrValue::Floats(f) => Ok(PyList::new(py, f)?.into_any()),
        AttrValue::Ints(i) => Ok(PyList::new(py, i)?.into_any()),
    }
}

// ---- labels, coordinates and data ------------------------------------------------------

fn label_to_py<'py>(py: Python<'py>, l: &Label) -> PyResult<Bound<'py, PyAny>> {
    match l {
        Label::Str(s) => (**s).into_bound_py_any(py),
        Label::Int(i) | Label::Datetime(i) => i.into_bound_py_any(py),
        Label::Float(f) => f.into_bound_py_any(py),
    }
}

/// `(kind, labels)` of a coordinate.
fn labels_to_py<'py>(py: Python<'py>, labels: &[Label]) -> PyResult<Bound<'py, PyTuple>> {
    let datetime = !labels.is_empty() && labels.iter().all(|l| matches!(l, Label::Datetime(_)));
    let kind = if datetime { "datetime" } else { "object" };
    let values = PyList::empty(py);
    for l in labels {
        values.append(label_to_py(py, l)?)?;
    }
    PyTuple::new(py, [kind.into_bound_py_any(py)?, values.into_any()])
}

fn py_to_labels(obj: &Bound<'_, PyAny>) -> PyResult<Vec<Label>> {
    let (kind, values): (String, Bound<'_, PyAny>) = obj.extract()?;
    let mut out = Vec::new();
    for v in values.try_iter()? {
        let v = v?;
        out.push(if kind == "datetime" {
            Label::Datetime(v.extract()?)
        } else {
            py_to_label(&v)?
        });
    }
    Ok(out)
}

fn py_to_label(v: &Bound<'_, PyAny>) -> PyResult<Label> {
    if v.is_instance_of::<PyString>() {
        Ok(Label::str(&v.extract::<String>()?))
    } else if v.is_instance_of::<PyBool>() {
        Ok(Label::str(if v.extract::<bool>()? {
            "True"
        } else {
            "False"
        }))
    } else if let Ok(i) = v.extract::<i64>()
        && !v.is_instance_of::<PyFloat>()
    {
        Ok(Label::Int(i))
    } else if let Ok(f) = v.extract::<f64>() {
        Ok(Label::Float(f))
    } else {
        Ok(Label::str(&v.str()?.extract::<String>()?))
    }
}

fn coords_to_py<'py>(py: Python<'py>, coords: &[&Coord]) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    for c in coords {
        d.set_item(c.name(), labels_to_py(py, c.labels())?)?;
    }
    Ok(d)
}

fn py_to_coords(d: &Bound<'_, PyAny>) -> PyResult<IndexMap<String, Coord>> {
    let d = d.cast::<PyDict>()?;
    let mut out = IndexMap::new();
    for (k, v) in d.iter() {
        let name: String = k.extract()?;
        let c = Coord::new(&name, py_to_labels(&v)?).py()?;
        out.insert(name, c);
    }
    Ok(out)
}

fn bytes_of<T: Copy, const N: usize>(v: &[T], f: impl Fn(T) -> [u8; N]) -> Vec<u8> {
    v.iter().flat_map(|x| f(*x)).collect()
}

fn obj_to_py<'py>(py: Python<'py>, o: &Obj) -> PyResult<Bound<'py, PyAny>> {
    match o {
        Obj::Nan => f64::NAN.into_bound_py_any(py),
        Obj::Bool(b) => b.into_bound_py_any(py),
        Obj::Int(i) => i.into_bound_py_any(py),
        Obj::Float(f) => f.into_bound_py_any(py),
        Obj::Str(s) => (**s).into_bound_py_any(py),
        Obj::Datetime(_) => o.to_py_str().into_bound_py_any(py),
    }
}

/// `(dtype, payload)` of data values.
fn data_to_py<'py>(py: Python<'py>, d: &Data) -> PyResult<(&'static str, Bound<'py, PyAny>)> {
    let bytes = |b: Vec<u8>| PyBytes::new(py, &b).into_any();
    Ok(match d {
        Data::Float(v) => ("f8", bytes(bytes_of(v, f64::to_le_bytes))),
        Data::Sparse(s) => ("f8", bytes(bytes_of(&s.to_dense(), f64::to_le_bytes))),
        Data::Int(v) => ("i8", bytes(bytes_of(v, i64::to_le_bytes))),
        Data::Bool(v) => ("?", bytes(v.iter().map(|b| u8::from(*b)).collect())),
        Data::Datetime(v) => ("M8[ns]", bytes(bytes_of(v, i64::to_le_bytes))),
        Data::Object(v) => {
            let l = PyList::empty(py);
            for o in v {
                l.append(obj_to_py(py, o)?)?;
            }
            ("O", l.into_any())
        }
    })
}

fn chunks<const N: usize>(b: &[u8]) -> PyResult<impl Iterator<Item = [u8; N]> + '_> {
    let (whole, rest) = b.as_chunks::<N>();
    if !rest.is_empty() {
        return Err(PyValueError::new_err("data buffer has a partial element"));
    }
    Ok(whole.iter().copied())
}

fn py_to_obj(v: &Bound<'_, PyAny>) -> PyResult<Obj> {
    Ok(if v.is_none() {
        Obj::Nan
    } else if v.is_instance_of::<PyString>() {
        Obj::str(&v.extract::<String>()?)
    } else if v.is_instance_of::<PyBool>() {
        Obj::Bool(v.extract()?)
    } else if v.is_instance_of::<PyInt>() {
        Obj::Int(v.extract()?)
    } else if let Ok(f) = v.extract::<f64>() {
        if f.is_nan() { Obj::Nan } else { Obj::Float(f) }
    } else {
        Obj::str(&v.str()?.extract::<String>()?)
    })
}

fn py_to_data(dtype: &str, payload: &Bound<'_, PyAny>) -> PyResult<Data> {
    if dtype == "O" {
        let mut out = Vec::new();
        for v in payload.try_iter()? {
            out.push(py_to_obj(&v?)?);
        }
        return Ok(Data::Object(out));
    }
    let b: Vec<u8> = payload.extract()?;
    Ok(match dtype {
        "f8" => Data::Float(chunks::<8>(&b)?.map(f64::from_le_bytes).collect()),
        "i8" => Data::Int(chunks::<8>(&b)?.map(i64::from_le_bytes).collect()),
        "?" => Data::Bool(b.iter().map(|x| *x != 0).collect()),
        "M8[ns]" => Data::Datetime(chunks::<8>(&b)?.map(i64::from_le_bytes).collect()),
        other => return Err(PyTypeError::new_err(format!("unsupported dtype {other}"))),
    })
}

fn dataset_to_py<'py>(py: Python<'py>, ds: &Dataset) -> PyResult<Bound<'py, PyDict>> {
    let out = PyDict::new(py);
    out.set_item(
        "coords",
        coords_to_py(py, &ds.coords.values().collect::<Vec<_>>())?,
    )?;
    let vars = PyList::empty(py);
    for (name, v) in &ds.vars {
        let (dtype, payload) = data_to_py(py, &v.data)?;
        vars.append((name, &v.dims, dtype, payload))?;
    }
    out.set_item("vars", vars)?;
    Ok(out)
}

fn py_to_dataset(d: &Bound<'_, PyAny>) -> PyResult<Dataset> {
    let coords = py_to_coords(&d.get_item("coords")?)?;
    let mut vars = IndexMap::new();
    for v in d.get_item("vars")?.try_iter()? {
        let (name, dims, dtype, payload): (String, Vec<String>, String, Bound<'_, PyAny>) =
            v?.extract()?;
        let data = py_to_data(&dtype, &payload)?;
        let n: usize = dims
            .iter()
            .map(|dim| coords.get(dim).map_or(0, Coord::len))
            .product();
        if let Some(dim) = dims.iter().find(|d| !coords.contains_key(*d)) {
            return Err(PyValueError::new_err(format!(
                "`{name}`: dimension `{dim}` has no coordinate"
            )));
        }
        if n != data.len() {
            return Err(PyValueError::new_err(format!(
                "`{name}`: {} values for shape of size {n}",
                data.len()
            )));
        }
        vars.insert(name, Var { dims, data });
    }
    Ok(Dataset { coords, vars })
}

fn array_dict<'py>(
    py: Python<'py>,
    dims: &[Coord],
    dtype: &str,
    payload: Bound<'py, PyAny>,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item(
        "coords",
        coords_to_py(py, &dims.iter().collect::<Vec<_>>())?,
    )?;
    d.set_item("dims", dims.iter().map(Coord::name).collect::<Vec<_>>())?;
    d.set_item("dtype", dtype)?;
    d.set_item("data", payload)?;
    Ok(d)
}

fn num_to_py<'py>(py: Python<'py>, a: &LArray<f64>) -> PyResult<Bound<'py, PyDict>> {
    let (dtype, payload) = data_to_py(py, &Data::Float(a.to_dense()))?;
    array_dict(py, a.dims(), dtype, payload)
}

fn value_to_py<'py>(py: Python<'py>, v: &Value) -> PyResult<Bound<'py, PyAny>> {
    Ok(match v {
        Value::Num(a) => num_to_py(py, a)?.into_any(),
        Value::Bool(a) => {
            let (dtype, payload) = data_to_py(py, &Data::Bool(a.to_dense()))?;
            array_dict(py, a.dims(), dtype, payload)?.into_any()
        }
        Value::Label(a) => {
            let cells = a.to_dense();
            let datetime = cells.iter().flatten().count() > 0
                && cells
                    .iter()
                    .flatten()
                    .all(|l| matches!(l, Label::Datetime(_)));
            let data = if datetime {
                Data::Datetime(
                    cells
                        .iter()
                        .map(|l| match l {
                            Some(Label::Datetime(t)) => *t,
                            _ => i64::MIN,
                        })
                        .collect(),
                )
            } else {
                Data::Object(
                    cells
                        .iter()
                        .map(|l| l.as_ref().map_or(Obj::Nan, Obj::from_label))
                        .collect(),
                )
            };
            let (dtype, payload) = data_to_py(py, &data)?;
            array_dict(py, a.dims(), dtype, payload)?.into_any()
        }
        Value::Str(s) => s.into_bound_py_any(py)?,
        Value::Expr(_) | Value::List(_) => {
            return Err(PyTypeError::new_err("not an input array"));
        }
    })
}

/// An array dict as (dims, data).
fn py_to_array(d: &Bound<'_, PyAny>) -> PyResult<(Vec<Coord>, Data)> {
    let coords = py_to_coords(&d.get_item("coords")?)?;
    let dims: Vec<String> = d.get_item("dims")?.extract()?;
    let dtype: String = d.get_item("dtype")?.extract()?;
    let data = py_to_data(&dtype, &d.get_item("data")?)?;
    let dims =
        dims.iter()
            .map(|n| {
                coords.get(n).cloned().ok_or_else(|| {
                    PyValueError::new_err(format!("dimension `{n}` has no coordinate"))
                })
            })
            .collect::<PyResult<Vec<_>>>()?;
    Ok((dims, data))
}

fn py_to_value(d: &Bound<'_, PyAny>) -> PyResult<Value> {
    let (dims, data) = py_to_array(d)?;
    Ok(match data {
        Data::Bool(v) => Value::Bool(LArray::from_dense(dims, v).py()?),
        Data::Datetime(v) => Value::Label(
            LArray::from_dense(
                dims,
                v.into_iter()
                    .map(|t| (t != i64::MIN).then_some(Label::Datetime(t)))
                    .collect(),
            )
            .py()?,
        ),
        Data::Object(v)
            if v.iter()
                .any(|o| matches!(o, Obj::Str(_) | Obj::Datetime(_))) =>
        {
            Value::Label(
                LArray::from_dense(
                    dims,
                    v.iter()
                        .map(|o| (!o.is_null()).then(|| o.to_label()))
                        .collect(),
                )
                .py()?,
            )
        }
        other => Value::Num(LArray::from_dense(dims, num_values(&other)?).py()?),
    })
}

fn num_values(d: &Data) -> PyResult<Vec<f64>> {
    d.as_f64_vec()
        .ok_or_else(|| PyTypeError::new_err("expected numeric values"))
}

fn py_to_num(d: &Bound<'_, PyAny>) -> PyResult<LArray<f64>> {
    let (dims, data) = py_to_array(d)?;
    LArray::from_dense(dims, num_values(&data)?).py()
}

fn py_to_bool(d: &Bound<'_, PyAny>) -> PyResult<LArray<bool>> {
    let (dims, data) = py_to_array(d)?;
    let v = match data {
        Data::Bool(v) => v,
        other => num_values(&other)?
            .into_iter()
            .map(|x| x != 0.0 && !x.is_nan())
            .collect(),
    };
    LArray::from_dense(dims, v).py()
}

/// A `data_table_dfs` entry as a [`Frame`].
fn py_to_frame(d: &Bound<'_, PyAny>) -> PyResult<Frame> {
    let index_names: Vec<Option<String>> = d.get_item("index_names")?.extract()?;
    let column_names: Vec<Option<String>> = d.get_item("column_names")?.extract()?;
    let levels = |key: &str| -> PyResult<Vec<Vec<Label>>> {
        d.get_item(key)?
            .try_iter()?
            .map(|l| py_to_labels(&l?))
            .collect()
    };
    let transpose = |levels: Vec<Vec<Label>>| -> Vec<Vec<Label>> {
        let n = levels.first().map_or(0, Vec::len);
        (0..n)
            .map(|i| levels.iter().map(|l| l[i].clone()).collect())
            .collect()
    };
    let index = transpose(levels("index")?);
    let columns = transpose(levels("columns")?);
    let mut data = Vec::new();
    for c in d.get_item("data")?.try_iter()? {
        let (dtype, payload): (String, Bound<'_, PyAny>) = c?.extract()?;
        data.push(py_to_data(&dtype, &payload)?);
    }
    if data.len() != columns.len() || data.iter().any(|c| c.len() != index.len()) {
        return Err(PyValueError::new_err("inconsistent data table shape"));
    }
    Ok(Frame {
        index_names,
        index,
        column_names,
        columns,
        data,
    })
}

fn group_of(name: &str) -> PyResult<Group> {
    Ok(match name {
        "variables" => Group::Variables,
        "global_expressions" => Group::GlobalExpressions,
        "constraints" => Group::Constraints,
        "piecewise_constraints" => Group::PiecewiseConstraints,
        "objectives" => Group::Objectives,
        "postprocessed" => Group::Postprocessed,
        other => {
            return Err(PyValueError::new_err(format!(
                "Unknown component type: {other}"
            )));
        }
    })
}

/// `(lb, ub, body)` of a constraint.
type ConstraintDicts<'py> = (
    Bound<'py, PyDict>,
    Bound<'py, PyDict>,
    Option<Bound<'py, PyDict>>,
);

// ---- the model -------------------------------------------------------------------------

/// A calliope-rs model (`calliope.Model`), wrapped by `calliope_rs.Model`.
#[pyclass(unsendable, name = "RsModel", module = "calliope_rs._core")]
#[derive(Debug)]
struct PyModel {
    inner: Model,
}

impl PyModel {
    fn backend(&self) -> PyResult<&calliope_model::Backend> {
        self.inner
            .backend()
            .ok_or_else(|| ModelError::new_err("The model has not been built yet."))
    }

    fn backend_mut(&mut self) -> PyResult<&mut calliope_model::Backend> {
        self.inner
            .backend_mut()
            .ok_or_else(|| ModelError::new_err("The model has not been built yet."))
    }
}

#[pymethods]
impl PyModel {
    /// `model.inputs` as a dataset dict.
    fn inputs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        dataset_to_py(py, &self.inner.inputs)
    }

    /// `model.results` as a dataset dict.
    fn results<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        dataset_to_py(py, &self.inner.results.to_dataset().py()?)
    }

    /// `model.dump_all_attrs()`.
    fn dump_all_attrs<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for (k, v) in &self.inner.dump_all_attrs() {
            d.set_item(k, attr_to_py(py, v)?)?;
        }
        Ok(d)
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name().map(str::to_owned)
    }

    #[getter]
    fn is_built(&self) -> bool {
        self.inner.is_built()
    }

    #[getter]
    fn is_solved(&self) -> bool {
        self.inner.is_solved()
    }

    #[getter]
    fn warnings(&self) -> Vec<String> {
        self.inner.warnings.clone()
    }

    fn info(&self) -> String {
        self.inner.info()
    }

    fn set_random_seed(&mut self, seed: u32) {
        self.inner.set_random_seed(seed);
    }

    #[pyo3(signature = (force=false, overrides=None))]
    fn build(&mut self, force: bool, overrides: Option<&Bound<'_, PyDict>>) -> PyResult<()> {
        let opts = BuildOptions {
            force,
            overrides: opt_tree(overrides)?.unwrap_or_default(),
        };
        self.inner.build(&opts).py()
    }

    #[pyo3(signature = (force=false, warmstart=false, overrides=None))]
    fn solve(
        &mut self,
        force: bool,
        warmstart: bool,
        overrides: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<()> {
        let opts = SolveOptions {
            force,
            warmstart,
            overrides: opt_tree(overrides)?.unwrap_or_default(),
        };
        self.inner.solve(&opts).py()
    }

    fn to_netcdf(&self, path: PathBuf) -> PyResult<()> {
        self.inner.to_netcdf(&path).py()
    }

    #[pyo3(signature = (path, dropna=true, allow_overwrite=false))]
    fn to_csv(&self, path: PathBuf, dropna: bool, allow_overwrite: bool) -> PyResult<()> {
        self.inner.to_csv(&path, dropna, allow_overwrite).py()
    }

    /// `calliope.Model(inputs, attrs=self.all_attrs(), results, **init)`.
    #[pyo3(signature = (inputs, results=None, init=None))]
    fn with_inputs(
        &self,
        inputs: &Bound<'_, PyAny>,
        results: Option<&Bound<'_, PyAny>>,
        init: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<PyModel> {
        let inputs = py_to_dataset(inputs)?;
        let results = results
            .map(|r| {
                let ds = py_to_dataset(r)?;
                Results::from_dataset(&ds, String::new()).py()
            })
            .transpose()?;
        let init = opt_tree(init)?.unwrap_or_default();
        Ok(PyModel {
            inner: self.inner.with_inputs(inputs, results, &init).py()?,
        })
    }

    // ---- backend interface -------------------------------------------------------------

    fn backend_get_parameter<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<Bound<'py, PyAny>> {
        value_to_py(py, self.backend()?.get_parameter(name).py()?)
    }

    fn backend_update_input(&mut self, name: &str, new: &Bound<'_, PyAny>) -> PyResult<()> {
        let v = py_to_value(new)?;
        self.backend_mut()?.update_input(name, &v).py()
    }

    fn backend_get_variable<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        num_to_py(py, &self.backend()?.get_variable(name).py()?)
    }

    fn backend_get_variable_bounds<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<(Bound<'py, PyDict>, Bound<'py, PyDict>)> {
        let (lb, ub) = self.backend()?.get_variable_bounds(name).py()?;
        Ok((num_to_py(py, &lb)?, num_to_py(py, &ub)?))
    }

    #[pyo3(signature = (name, min=None, max=None))]
    fn backend_update_variable_bounds(
        &mut self,
        name: &str,
        min: Option<&Bound<'_, PyAny>>,
        max: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let min = min.map(py_to_num).transpose()?;
        let max = max.map(py_to_num).transpose()?;
        self.backend_mut()?
            .update_variable_bounds(name, min.as_ref(), max.as_ref())
            .py()
    }

    #[pyo3(signature = (name, where_=None))]
    fn backend_fix_variable(
        &mut self,
        name: &str,
        where_: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let w = where_.map(py_to_bool).transpose()?;
        self.backend_mut()?.fix_variable(name, w.as_ref()).py()
    }

    #[pyo3(signature = (name, where_=None))]
    fn backend_unfix_variable(
        &mut self,
        name: &str,
        where_: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<()> {
        let w = where_.map(py_to_bool).transpose()?;
        self.backend_mut()?.unfix_variable(name, w.as_ref()).py()
    }

    fn backend_get_global_expression<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<Bound<'py, PyDict>> {
        num_to_py(py, &self.backend()?.get_global_expression(name).py()?)
    }

    /// `(lb, ub, body or None)`.
    fn backend_get_constraint<'py>(
        &self,
        py: Python<'py>,
        name: &str,
    ) -> PyResult<ConstraintDicts<'py>> {
        let c = self.backend()?.get_constraint(name).py()?;
        Ok((
            num_to_py(py, &c.lb)?,
            num_to_py(py, &c.ub)?,
            c.body.as_ref().map(|b| num_to_py(py, b)).transpose()?,
        ))
    }

    fn backend_get_objective(&self, name: &str) -> PyResult<f64> {
        self.backend()?.get_objective(name).py()
    }

    fn backend_set_objective(&mut self, name: &str) -> PyResult<()> {
        self.backend_mut()?.set_objective(name).py()
    }

    fn backend_objective(&self) -> PyResult<String> {
        Ok(self.backend()?.objective().to_owned())
    }

    fn backend_set_constraint_active(&mut self, name: &str, active: bool) -> PyResult<()> {
        self.backend_mut()?.set_constraint_active(name, active).py()
    }

    fn backend_delete_component(&mut self, name: &str, component_type: &str) -> PyResult<()> {
        let g = group_of(component_type)?;
        self.backend_mut()?.delete_component(name, g).py()
    }

    fn backend_verbose_strings(&mut self) -> PyResult<()> {
        self.backend_mut()?.verbose_strings();
        Ok(())
    }

    fn backend_to_lp(&self, path: PathBuf) -> PyResult<()> {
        self.backend()?.to_lp(&path).py()
    }

    fn backend_lp_string(&self) -> PyResult<String> {
        Ok(self.backend()?.lp_string())
    }

    fn backend_has_integer_or_binary_variables(&self) -> PyResult<bool> {
        Ok(self.backend()?.has_integer_or_binary_variables())
    }

    /// Component names of a group in the built math.
    fn backend_component_names(&self, component_type: &str) -> PyResult<Vec<String>> {
        let math = self.backend()?.math();
        let names: Vec<String> = match group_of(component_type)? {
            Group::Variables => math.variables.keys().map(ToString::to_string).collect(),
            Group::GlobalExpressions => math
                .global_expressions
                .keys()
                .map(ToString::to_string)
                .collect(),
            Group::Constraints => math.constraints.keys().map(ToString::to_string).collect(),
            Group::PiecewiseConstraints => math
                .piecewise_constraints
                .keys()
                .map(ToString::to_string)
                .collect(),
            Group::Objectives => math.objectives.keys().map(ToString::to_string).collect(),
            Group::Postprocessed | Group::Checks => Vec::new(),
        };
        Ok(names)
    }
}

fn read_options(
    scenario: Option<String>,
    override_dict: Option<&Bound<'_, PyDict>>,
    math_dict: Option<&Bound<'_, PyDict>>,
    data_table_dfs: Option<&Bound<'_, PyDict>>,
    init_kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<ReadOptions> {
    let mut dfs = IndexMap::new();
    if let Some(d) = data_table_dfs {
        for (k, v) in d.iter() {
            dfs.insert(k.extract::<String>()?, py_to_frame(&v)?);
        }
    }
    Ok(ReadOptions {
        scenario,
        override_dict: opt_tree(override_dict)?,
        math_dict: opt_tree(math_dict)?,
        init_kwargs: opt_tree(init_kwargs)?.unwrap_or_default(),
        data_table_dfs: dfs,
    })
}

/// `calliope.read_yaml`.
#[pyfunction]
#[pyo3(signature = (path, scenario=None, override_dict=None, math_dict=None, data_table_dfs=None, init_kwargs=None))]
fn read_yaml(
    path: PathBuf,
    scenario: Option<String>,
    override_dict: Option<&Bound<'_, PyDict>>,
    math_dict: Option<&Bound<'_, PyDict>>,
    data_table_dfs: Option<&Bound<'_, PyDict>>,
    init_kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<PyModel> {
    let opts = read_options(
        scenario,
        override_dict,
        math_dict,
        data_table_dfs,
        init_kwargs,
    )?;
    Ok(PyModel {
        inner: Model::from_yaml(&path, &opts).py()?,
    })
}

/// `calliope.read_dict`.
#[pyfunction]
#[pyo3(signature = (model_definition, scenario=None, override_dict=None, math_dict=None, data_table_dfs=None, definition_path=None, init_kwargs=None))]
fn read_dict(
    model_definition: &Bound<'_, PyDict>,
    scenario: Option<String>,
    override_dict: Option<&Bound<'_, PyDict>>,
    math_dict: Option<&Bound<'_, PyDict>>,
    data_table_dfs: Option<&Bound<'_, PyDict>>,
    definition_path: Option<PathBuf>,
    init_kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<PyModel> {
    let def = py_to_tree(model_definition)?;
    let opts = read_options(
        scenario,
        override_dict,
        math_dict,
        data_table_dfs,
        init_kwargs,
    )?;
    Ok(PyModel {
        inner: Model::from_dict(&def, definition_path.as_deref(), &opts).py()?,
    })
}

/// `calliope.read_netcdf`.
#[pyfunction]
#[pyo3(signature = (path, init_kwargs=None))]
fn read_netcdf(path: PathBuf, init_kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<PyModel> {
    let m = Model::from_netcdf(&path).py()?;
    let init = opt_tree(init_kwargs)?.unwrap_or_default();
    let inner = if init.is_empty() {
        m
    } else {
        m.with_inputs(m.inputs.clone(), Some(m.results.clone()), &init)
            .py()?
    };
    Ok(PyModel { inner })
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyModel>()?;
    m.add_function(wrap_pyfunction!(read_yaml, m)?)?;
    m.add_function(wrap_pyfunction!(read_dict, m)?)?;
    m.add_function(wrap_pyfunction!(read_netcdf, m)?)?;
    m.add("ModelError", m.py().get_type::<ModelError>())?;
    m.add("BackendError", m.py().get_type::<BackendError>())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
