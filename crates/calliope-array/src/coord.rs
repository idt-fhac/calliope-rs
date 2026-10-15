//! Coordinate labels and named dimensions.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// A coordinate label. Datetimes are nanoseconds since the Unix epoch (`datetime64[ns]`).
#[derive(Debug, Clone)]
pub enum Label {
    /// A string.
    Str(Arc<str>),
    /// An integer.
    Int(i64),
    /// A float (compared by total order, so NaN labels are allowed and equal to themselves).
    Float(f64),
    /// A datetime in nanoseconds since the epoch.
    Datetime(i64),
}

impl Label {
    /// A string label.
    pub fn str(s: &str) -> Self {
        Label::Str(Arc::from(s))
    }

    fn rank(&self) -> u8 {
        match self {
            Label::Int(_) | Label::Float(_) => 0,
            Label::Datetime(_) => 1,
            Label::Str(_) => 2,
        }
    }
}

impl PartialEq for Label {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Label {}

impl PartialOrd for Label {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Numbers compare numerically (`1 == 1.0`, as in pandas), then datetimes, then strings.
impl Ord for Label {
    fn cmp(&self, other: &Self) -> Ordering {
        use Label::*;
        match (self, other) {
            (Int(a), Int(b)) => a.cmp(b),
            (Datetime(a), Datetime(b)) => a.cmp(b),
            (Str(a), Str(b)) => a.cmp(b),
            #[allow(clippy::cast_precision_loss)]
            (Int(a), Float(b)) => (*a as f64).total_cmp(b),
            #[allow(clippy::cast_precision_loss)]
            (Float(a), Int(b)) => a.total_cmp(&(*b as f64)),
            (Float(a), Float(b)) => a.total_cmp(b),
            (a, b) => a.rank().cmp(&b.rank()),
        }
    }
}

impl Hash for Label {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Label::Str(s) => {
                2u8.hash(state);
                s.hash(state);
            }
            Label::Datetime(d) => {
                1u8.hash(state);
                d.hash(state);
            }
            #[allow(clippy::cast_precision_loss)]
            Label::Int(i) => {
                0u8.hash(state);
                (*i as f64).to_bits().hash(state);
            }
            Label::Float(f) => {
                0u8.hash(state);
                f.to_bits().hash(state);
            }
        }
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Label::Str(s) => f.write_str(s),
            Label::Int(i) => write!(f, "{i}"),
            Label::Float(x) => write!(f, "{x}"),
            Label::Datetime(ns) => write!(f, "{ns}ns"),
        }
    }
}

impl From<&str> for Label {
    fn from(s: &str) -> Self {
        Label::str(s)
    }
}

impl From<i64> for Label {
    fn from(i: i64) -> Self {
        Label::Int(i)
    }
}

/// A named dimension with ordered, unique labels. Cheap to clone (shared).
#[derive(Clone)]
pub struct Coord(Arc<CoordInner>);

struct CoordInner {
    name: Arc<str>,
    labels: Vec<Label>,
    index: HashMap<Label, u32>,
}

impl fmt::Debug for Coord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{:?}", self.name(), self.labels())
    }
}

impl PartialEq for Coord {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            || (self.name() == other.name() && self.labels() == other.labels())
    }
}

/// Errors from building coordinates and arrays.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ArrayError {
    /// A label appears twice in a coordinate.
    #[error("Duplicate label `{label}` in dimension `{dim}`")]
    DuplicateLabel {
        /// The dimension.
        dim: String,
        /// The label.
        label: String,
    },
    /// A label is not in a coordinate.
    #[error("Label `{label}` not found in dimension `{dim}`")]
    MissingLabel {
        /// The dimension.
        dim: String,
        /// The label.
        label: String,
    },
    /// A dimension is not in the array.
    #[error("Dimension `{0}` not found")]
    MissingDim(String),
    /// Shapes or dimensions do not fit.
    #[error("{0}")]
    Shape(String),
}

impl Coord {
    /// A coordinate from labels, which must be unique.
    pub fn new(name: &str, labels: Vec<Label>) -> Result<Self, ArrayError> {
        let mut index = HashMap::with_capacity(labels.len());
        for (i, l) in labels.iter().enumerate() {
            let code = u32::try_from(i).map_err(|_| ArrayError::Shape("too many labels".into()))?;
            if index.insert(l.clone(), code).is_some() {
                return Err(ArrayError::DuplicateLabel {
                    dim: name.to_owned(),
                    label: l.to_string(),
                });
            }
        }
        Ok(Self(Arc::new(CoordInner {
            name: Arc::from(name),
            labels,
            index,
        })))
    }

    /// A coordinate of string labels.
    pub fn strings(name: &str, labels: &[&str]) -> Result<Self, ArrayError> {
        Self::new(name, labels.iter().map(|s| Label::str(s)).collect())
    }

    /// The dimension name.
    pub fn name(&self) -> &str {
        &self.0.name
    }

    /// The labels, in order.
    pub fn labels(&self) -> &[Label] {
        &self.0.labels
    }

    /// Number of labels.
    pub fn len(&self) -> usize {
        self.0.labels.len()
    }

    /// True if there are no labels.
    pub fn is_empty(&self) -> bool {
        self.0.labels.is_empty()
    }

    /// The code (position) of a label.
    pub fn code(&self, label: &Label) -> Option<u32> {
        self.0.index.get(label).copied()
    }

    /// The label at a code.
    pub fn label(&self, code: u32) -> &Label {
        &self.0.labels[code as usize]
    }

    /// True if all labels are datetimes (the dimension is a time axis).
    pub fn is_datetime(&self) -> bool {
        !self.is_empty()
            && self
                .labels()
                .iter()
                .all(|l| matches!(l, Label::Datetime(_)))
    }

    /// The same labels under a new name.
    pub fn renamed(&self, name: &str) -> Self {
        Self::new(name, self.labels().to_vec()).expect("labels are unique")
    }

    /// Joins two coordinates of the same dimension as xarray aligns indexes: equal
    /// coordinates are kept; otherwise `inner` keeps the left labels that are also on the
    /// right (left order) and `outer` takes the sorted union.
    pub fn join(&self, other: &Coord, join: Join) -> Coord {
        if self == other {
            return self.clone();
        }
        let labels = match join {
            Join::Inner => self
                .labels()
                .iter()
                .filter(|l| other.code(l).is_some())
                .cloned()
                .collect(),
            Join::Outer => {
                let mut all: Vec<Label> = self.labels().to_vec();
                all.extend(
                    other
                        .labels()
                        .iter()
                        .filter(|l| self.code(l).is_none())
                        .cloned(),
                );
                all.sort();
                all
            }
            Join::Left => self.labels().to_vec(),
        };
        Coord::new(self.name(), labels).expect("joined labels are unique")
    }
}

/// How coordinates are aligned in a binary operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    /// Labels on both sides (xarray's default for arithmetic).
    Inner,
    /// Labels on either side (sorted union).
    Outer,
    /// The left array's labels.
    Left,
}
