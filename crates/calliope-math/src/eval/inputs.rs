//! The input data the math is evaluated on (`BackendModel.inputs`).

use calliope_array::{Coord, LArray, Label};
use calliope_schema::{CalliopeBuildMath, LookupDefault, Numeric};
use indexmap::IndexMap;

use super::value::Value;

/// Model inputs as the backend sees them (`BackendModel._add_inputs`): one array per math
/// parameter and lookup, and the dimension coordinates.
///
/// Parameters missing from the data are a NaN scalar and lookups missing from the data are
/// their default (a scalar); data not named in the math is dropped.
#[derive(Debug, Clone, Default)]
pub struct Inputs {
    /// Dimension coordinates, by name.
    pub coords: IndexMap<String, Coord>,
    /// Parameters and lookups: [`Value::Num`], [`Value::Bool`] or [`Value::Label`].
    pub arrays: IndexMap<String, Value>,
}

impl Inputs {
    /// `_add_inputs`: keeps the arrays the math defines as parameters or lookups and adds
    /// scalars for those missing from `data`.
    pub fn new(
        coords: IndexMap<String, Coord>,
        mut data: IndexMap<String, Value>,
        math: &CalliopeBuildMath,
    ) -> Self {
        let mut arrays = IndexMap::new();
        for name in math.parameters.keys() {
            let v = data
                .shift_remove(name.as_str())
                .unwrap_or_else(|| Value::num(f64::NAN));
            arrays.insert(name.to_string(), v);
        }
        for (name, l) in &math.lookups {
            let v = data
                .shift_remove(name.as_str())
                .unwrap_or_else(|| lookup_default(&l.default));
            arrays.insert(name.to_string(), v);
        }
        Self { coords, arrays }
    }

    /// The input `name`, if the math defines it.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.arrays.get(name)
    }
}

/// A lookup default as a scalar value.
pub fn lookup_default(d: &LookupDefault) -> Value {
    match d {
        LookupDefault::Bool(b) => Value::Bool(LArray::scalar(*b)),
        LookupDefault::Number(n) => Value::num(numeric(n)),
        LookupDefault::Str(s) => Value::Label(LArray::scalar(Some(Label::str(s.as_str())))),
    }
}

/// A schema number as `f64`.
pub fn numeric(n: &Numeric) -> f64 {
    match n {
        #[allow(clippy::cast_precision_loss)]
        Numeric::Int(i) => *i as f64,
        Numeric::Float(f) => *f,
    }
}
