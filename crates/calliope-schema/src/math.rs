//! `schemas/math_schema.py`: the math definition.

use std::collections::{BTreeSet, HashSet};
use std::fmt;

use calliope_yaml::Node;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use crate::error::FieldError;
use crate::join;
use crate::types::{AnyValue, AttrStr, Numeric, UniqueList};
use crate::update::{DictShape, ModelShape, Schema, Shape};

macro_rules! literal_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $var:ident = $s:literal),+ $(,)? } default $d:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            $($(#[$vm])* #[serde(rename = $s)] $var,)+
        }
        impl Default for $name {
            fn default() -> Self {
                $name::$d
            }
        }
        impl $name {
            /// The YAML spelling.
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$var => $s,)+
                }
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

literal_enum!(
    /// `Dimension.dtype`.
    DimensionDtype {
        /// Strings.
        String = "string",
        /// Datetimes.
        Datetime = "datetime",
        /// Dates.
        Date = "date",
        /// Floats.
        Float = "float",
        /// Integers.
        Integer = "integer",
    } default String
);

literal_enum!(
    /// `Lookup.dtype`.
    LookupDtype {
        /// Floats.
        Float = "float",
        /// Strings.
        String = "string",
        /// Bools.
        Bool = "bool",
        /// Datetimes.
        Datetime = "datetime",
        /// Dates.
        Date = "date",
    } default String
);

literal_enum!(
    /// `resample_method`.
    ResampleMethod {
        /// Mean over the new bin.
        Mean = "mean",
        /// Sum over the new bin.
        Sum = "sum",
        /// First value of the new bin.
        First = "first",
    } default First
);

literal_enum!(
    /// `Variable.domain`.
    Domain {
        /// Continuous.
        Real = "real",
        /// Integer.
        Integer = "integer",
    } default Real
);

literal_enum!(
    /// `Objective.sense`.
    Sense {
        /// Minimise.
        Minimise = "minimise",
        /// Maximise.
        Maximise = "maximise",
        /// Minimise (US spelling).
        Minimize = "minimize",
        /// Maximise (US spelling).
        Maximize = "maximize",
    } default Minimise
);

literal_enum!(
    /// `Check.errors`.
    CheckErrors {
        /// Raise an error.
        Raise = "raise",
        /// Warn.
        Warn = "warn",
    } default Raise
);

impl Sense {
    /// True for `minimise`/`minimize`.
    pub fn is_minimise(self) -> bool {
        matches!(self, Sense::Minimise | Sense::Minimize)
    }
}

fn default_true() -> bool {
    true
}
fn default_where() -> String {
    "True".to_owned()
}
fn nan() -> Numeric {
    Numeric::Float(f64::NAN)
}

/// `ExpressionItem`: one equation, sub-expression or slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpressionItem {
    /// Condition that determines whether the expression is built.
    #[serde(rename = "where", default = "default_where")]
    pub where_: String,
    /// The expression.
    pub expression: String,
}

/// `Equations` (a `CalliopeListModel`).
pub type Equations = Vec<ExpressionItem>;
/// `SubExpressions` (a `CalliopeDictModel` of equation lists); also used for `slices`.
pub type SubExpressions = IndexMap<AttrStr, Equations>;

/// `Dimension`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Dimension {
    /// Long name.
    pub title: String,
    /// Description.
    pub description: String,
    /// If false, the component is ignored.
    pub active: bool,
    /// Data type of the dimension items.
    pub dtype: DimensionDtype,
    /// If true, the order of items is meaningful.
    pub ordered: bool,
    /// Iterator name in LaTeX math.
    pub iterator: String,
}

impl Default for Dimension {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            active: true,
            dtype: DimensionDtype::String,
            ordered: false,
            iterator: "NEEDS_ITERATOR".to_owned(),
        }
    }
}

/// `Parameter`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Parameter {
    /// Long name.
    pub title: String,
    /// Description.
    pub description: String,
    /// If false, the component is ignored.
    pub active: bool,
    /// Default value where no data is given.
    pub default: Numeric,
    /// Aggregation when resampling.
    pub resample_method: ResampleMethod,
    /// Unit.
    pub unit: String,
}

impl Default for Parameter {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            active: true,
            default: nan(),
            resample_method: ResampleMethod::First,
            unit: String::new(),
        }
    }
}

/// `Lookup.default`: `AttrStr | float | int | bool`.
#[derive(Debug, Clone, PartialEq)]
pub enum LookupDefault {
    /// A bool.
    Bool(bool),
    /// A number.
    Number(Numeric),
    /// A name.
    Str(AttrStr),
}

impl Serialize for LookupDefault {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            LookupDefault::Bool(b) => s.serialize_bool(*b),
            LookupDefault::Number(n) => n.serialize(s),
            LookupDefault::Str(a) => a.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for LookupDefault {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use calliope_yaml::Value;
        let node = Node::deserialize(d)?;
        match node.value {
            Value::Bool(b) => Ok(LookupDefault::Bool(b)),
            Value::Int(i) => Ok(LookupDefault::Number(Numeric::Int(i))),
            Value::Float(f) => Ok(LookupDefault::Number(Numeric::Float(f))),
            Value::Str(s) => AttrStr::new(s)
                .map(LookupDefault::Str)
                .map_err(de::Error::custom),
            other => Err(de::Error::custom(format!(
                "Input should be a name, number or bool, got {} `{other}`",
                other.type_name()
            ))),
        }
    }
}

impl JsonSchema for LookupDefault {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LookupDefault".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"anyOf": [
            g.subschema_for::<AttrStr>(), {"type": "number"}, {"type": "integer"},
            {"type": "boolean"}
        ]})
    }
}

/// `Lookup`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Lookup {
    /// Long name.
    pub title: String,
    /// Description.
    pub description: String,
    /// If false, the component is ignored.
    pub active: bool,
    /// Default value where no data is given.
    pub default: LookupDefault,
    /// Data type.
    pub dtype: LookupDtype,
    /// Aggregation when resampling.
    pub resample_method: ResampleMethod,
    /// Allowed values.
    #[schemars(with = "Option<Vec<AnyValue>>")]
    pub one_of: Option<Vec<Node>>,
    /// Pivot the values into a new boolean dimension of this name.
    pub pivot_values_to_dim: Option<String>,
}

impl Default for Lookup {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            active: true,
            default: LookupDefault::Number(nan()),
            dtype: LookupDtype::String,
            resample_method: ResampleMethod::First,
            one_of: None,
            pivot_values_to_dim: None,
        }
    }
}

/// A variable bound: a parameter name or a number.
#[derive(Debug, Clone, PartialEq)]
pub enum BoundValue {
    /// Name of an input parameter.
    Param(AttrStr),
    /// A constant.
    Value(Numeric),
}

impl Serialize for BoundValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            BoundValue::Param(p) => p.serialize(s),
            BoundValue::Value(v) => v.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for BoundValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use calliope_yaml::Value;
        let node = Node::deserialize(d)?;
        match node.value {
            // `AttrStr | NumericVal`: a valid name is a parameter, otherwise lax number parsing.
            Value::Str(s) => match AttrStr::new(s.clone()) {
                Ok(a) => Ok(BoundValue::Param(a)),
                Err(e) => s
                    .trim()
                    .parse::<f64>()
                    .map(|f| BoundValue::Value(Numeric::Float(f)))
                    .map_err(|_| de::Error::custom(e)),
            },
            Value::Int(i) => Ok(BoundValue::Value(Numeric::Int(i))),
            Value::Float(f) => Ok(BoundValue::Value(Numeric::Float(f))),
            Value::Bool(b) => Ok(BoundValue::Value(Numeric::Int(i64::from(b)))),
            other => Err(de::Error::custom(format!(
                "Input should be a parameter name or a number, got {} `{other}`",
                other.type_name()
            ))),
        }
    }
}

impl JsonSchema for BoundValue {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "BoundValue".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"anyOf": [
            g.subschema_for::<AttrStr>(), {"type": "integer"}, {"type": "number"}
        ]})
    }
}

/// `Bounds` of a decision variable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Bounds {
    /// Upper bound.
    pub max: BoundValue,
    /// Lower bound.
    pub min: BoundValue,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            max: BoundValue::Value(Numeric::Float(f64::INFINITY)),
            min: BoundValue::Value(Numeric::Float(f64::NEG_INFINITY)),
        }
    }
}

/// `Variable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct Variable {
    /// Long name.
    pub title: String,
    /// Description.
    pub description: String,
    /// If false, the component is ignored.
    pub active: bool,
    /// Dimensions over which the component is built.
    pub foreach: UniqueList<AttrStr>,
    /// Top-level condition.
    #[serde(rename = "where")]
    pub where_: String,
    /// Unit.
    pub unit: String,
    /// Default value.
    pub default: Numeric,
    /// Real or integer.
    pub domain: Domain,
    /// Bounds.
    pub bounds: Bounds,
}

impl Default for Variable {
    fn default() -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            active: true,
            foreach: UniqueList::default(),
            where_: default_where(),
            unit: String::new(),
            default: nan(),
            domain: Domain::Real,
            bounds: Bounds::default(),
        }
    }
}

/// `Constraint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    /// Long name.
    #[serde(default)]
    pub title: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// If false, the component is ignored.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Equations.
    #[serde(default)]
    pub equations: Equations,
    /// Named sub-expressions.
    #[serde(default)]
    pub sub_expressions: SubExpressions,
    /// Named index slices.
    #[serde(default)]
    pub slices: SubExpressions,
    /// Dimensions over which the component is built.
    #[serde(default)]
    pub foreach: UniqueList<AttrStr>,
    /// Top-level condition.
    #[serde(rename = "where", default = "default_where")]
    pub where_: String,
}

/// `PiecewiseConstraint`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PiecewiseConstraint {
    /// Long name.
    #[serde(default)]
    pub title: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// If false, the component is ignored.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Dimensions over which the component is built.
    #[serde(default)]
    pub foreach: UniqueList<AttrStr>,
    /// Top-level condition.
    #[serde(rename = "where", default = "default_where")]
    pub where_: String,
    /// X variable.
    pub x_expression: String,
    /// Y variable.
    pub y_expression: String,
    /// X breakpoint parameter.
    pub x_values: String,
    /// Y breakpoint parameter.
    pub y_values: String,
}

/// `GlobalExpression` (and `PostprocessedExpression`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GlobalExpression {
    /// Long name.
    #[serde(default)]
    pub title: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// If false, the component is ignored.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Equations.
    #[serde(default)]
    pub equations: Equations,
    /// Named sub-expressions.
    #[serde(default)]
    pub sub_expressions: SubExpressions,
    /// Named index slices.
    #[serde(default)]
    pub slices: SubExpressions,
    /// Dimensions over which the component is built.
    #[serde(default)]
    pub foreach: UniqueList<AttrStr>,
    /// Top-level condition.
    #[serde(rename = "where", default = "default_where")]
    pub where_: String,
    /// Unit.
    #[serde(default)]
    pub unit: String,
    /// Default value.
    #[serde(default = "nan")]
    pub default: Numeric,
    /// Evaluation order relative to other global expressions.
    #[serde(default)]
    pub order: i64,
}

/// `PostprocessedExpression`.
pub type PostprocessedExpression = GlobalExpression;

/// `Objective`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Objective {
    /// Long name.
    #[serde(default)]
    pub title: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// If false, the component is ignored.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Equations.
    #[serde(default)]
    pub equations: Equations,
    /// Named sub-expressions.
    #[serde(default)]
    pub sub_expressions: SubExpressions,
    /// Named index slices.
    #[serde(default)]
    pub slices: SubExpressions,
    /// Minimise or maximise.
    pub sense: Sense,
}

/// `MathEquationComponent`: the equation part shared by constraints, global expressions and
/// objectives. Usable on its own (as in the Python tests).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MathEquationComponent {
    /// Long name.
    #[serde(default)]
    pub title: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// If false, the component is ignored.
    #[serde(default = "default_true")]
    pub active: bool,
    /// Equations.
    #[serde(default)]
    pub equations: Equations,
    /// Named sub-expressions.
    #[serde(default)]
    pub sub_expressions: SubExpressions,
    /// Named index slices.
    #[serde(default)]
    pub slices: SubExpressions,
}

/// `Check`: an input data check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// Condition that flags a problem.
    #[serde(rename = "where")]
    pub where_: String,
    /// Message shown when the condition is true somewhere.
    pub message: String,
    /// Raise or warn.
    #[serde(default)]
    pub errors: CheckErrors,
    /// If false, the check is skipped.
    #[serde(default = "default_true")]
    pub active: bool,
}

const MUST_HAVE_EQUATIONS: &str = "Must have equations defined if component is active.";

fn check_equations(active: bool, equations: &Equations, path: &str, errors: &mut Vec<FieldError>) {
    if active && equations.is_empty() {
        errors.push(FieldError::new(path, MUST_HAVE_EQUATIONS));
    }
}

/// The component groups of [`CalliopeBuildMath`], in field order.
pub const COMPONENT_GROUPS: [&str; 10] = [
    "dimensions",
    "parameters",
    "lookups",
    "variables",
    "global_expressions",
    "constraints",
    "piecewise_constraints",
    "objectives",
    "postprocessed",
    "checks",
];

/// `CalliopeBuildMath`: the math used to build the optimisation problem.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model math schema")]
pub struct CalliopeBuildMath {
    /// Dimensions.
    pub dimensions: IndexMap<AttrStr, Dimension>,
    /// Parameters.
    pub parameters: IndexMap<AttrStr, Parameter>,
    /// Lookups.
    pub lookups: IndexMap<AttrStr, Lookup>,
    /// Decision variables.
    pub variables: IndexMap<AttrStr, Variable>,
    /// Global expressions.
    pub global_expressions: IndexMap<AttrStr, GlobalExpression>,
    /// Constraints.
    pub constraints: IndexMap<AttrStr, Constraint>,
    /// Piecewise constraints.
    pub piecewise_constraints: IndexMap<AttrStr, PiecewiseConstraint>,
    /// Objectives.
    pub objectives: IndexMap<AttrStr, Objective>,
    /// Postprocessed expressions.
    pub postprocessed: IndexMap<AttrStr, PostprocessedExpression>,
    /// Input data checks.
    pub checks: IndexMap<AttrStr, Check>,
}

/// A reference to one math component, as returned by [`CalliopeBuildMath::find`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ComponentRef<'a> {
    /// A dimension.
    Dimension(&'a Dimension),
    /// A parameter.
    Parameter(&'a Parameter),
    /// A lookup.
    Lookup(&'a Lookup),
    /// A variable.
    Variable(&'a Variable),
    /// A global expression.
    GlobalExpression(&'a GlobalExpression),
    /// A constraint.
    Constraint(&'a Constraint),
    /// A piecewise constraint.
    PiecewiseConstraint(&'a PiecewiseConstraint),
    /// An objective.
    Objective(&'a Objective),
    /// A postprocessed expression.
    Postprocessed(&'a PostprocessedExpression),
    /// A check.
    Check(&'a Check),
}

/// Errors from [`CalliopeBuildMath::find`].
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FindError {
    /// No active component of that name.
    #[error("Component name `{0}` not found in math schema.")]
    NotFound(String),
    /// Active components of that name in several groups.
    #[error("Component name `{name}` found in multiple places: {groups:?}.")]
    Multiple {
        /// The name.
        name: String,
        /// The groups it was found in.
        groups: Vec<&'static str>,
    },
}

/// Names grouped by role, for parsing math strings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameGroups {
    /// Dimension names.
    pub dimensions: BTreeSet<String>,
    /// Lookup and parameter names.
    pub inputs: BTreeSet<String>,
    /// Variable and global expression names.
    pub results: BTreeSet<String>,
}

/// `parsing_components`: valid names in `where` strings and in expressions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsingComponents {
    /// Names valid in expressions (active components only).
    pub expression: NameGroups,
    /// Names valid in `where` strings (inactive inputs and results are included unless the
    /// name is active in the other role).
    pub where_: NameGroups,
}

impl CalliopeBuildMath {
    /// Names of the active components of a group.
    pub fn active_names(&self, group: &str) -> Vec<&str> {
        fn act<T>(m: &IndexMap<AttrStr, T>, f: impl Fn(&T) -> bool) -> Vec<&str> {
            m.iter()
                .filter(|(_, v)| f(v))
                .map(|(k, _)| k.as_str())
                .collect()
        }
        match group {
            "dimensions" => act(&self.dimensions, |c| c.active),
            "parameters" => act(&self.parameters, |c| c.active),
            "lookups" => act(&self.lookups, |c| c.active),
            "variables" => act(&self.variables, |c| c.active),
            "global_expressions" => act(&self.global_expressions, |c| c.active),
            "constraints" => act(&self.constraints, |c| c.active),
            "piecewise_constraints" => act(&self.piecewise_constraints, |c| c.active),
            "objectives" => act(&self.objectives, |c| c.active),
            "postprocessed" => act(&self.postprocessed, |c| c.active),
            "checks" => act(&self.checks, |c| c.active),
            _ => Vec::new(),
        }
    }

    /// Names of all components of a group, active or not.
    pub fn all_names(&self, group: &str) -> Vec<&str> {
        fn all<T>(m: &IndexMap<AttrStr, T>) -> Vec<&str> {
            m.keys().map(AttrStr::as_str).collect()
        }
        match group {
            "dimensions" => all(&self.dimensions),
            "parameters" => all(&self.parameters),
            "lookups" => all(&self.lookups),
            "variables" => all(&self.variables),
            "global_expressions" => all(&self.global_expressions),
            "constraints" => all(&self.constraints),
            "piecewise_constraints" => all(&self.piecewise_constraints),
            "objectives" => all(&self.objectives),
            "postprocessed" => all(&self.postprocessed),
            "checks" => all(&self.checks),
            _ => Vec::new(),
        }
    }

    fn get_in<'a>(&'a self, group: &str, name: &str) -> Option<ComponentRef<'a>> {
        Some(match group {
            "dimensions" => ComponentRef::Dimension(self.dimensions.get(name)?),
            "parameters" => ComponentRef::Parameter(self.parameters.get(name)?),
            "lookups" => ComponentRef::Lookup(self.lookups.get(name)?),
            "variables" => ComponentRef::Variable(self.variables.get(name)?),
            "global_expressions" => {
                ComponentRef::GlobalExpression(self.global_expressions.get(name)?)
            }
            "constraints" => ComponentRef::Constraint(self.constraints.get(name)?),
            "piecewise_constraints" => {
                ComponentRef::PiecewiseConstraint(self.piecewise_constraints.get(name)?)
            }
            "objectives" => ComponentRef::Objective(self.objectives.get(name)?),
            "postprocessed" => ComponentRef::Postprocessed(self.postprocessed.get(name)?),
            "checks" => ComponentRef::Check(self.checks.get(name)?),
            _ => return None,
        })
    }

    /// `find`: the active component of this name, searching all groups except `checks`, or
    /// only `subset`.
    pub fn find(&self, name: &str, subset: Option<&[&str]>) -> Result<ComponentRef<'_>, FindError> {
        let groups: Vec<&'static str> = COMPONENT_GROUPS
            .iter()
            .copied()
            .filter(|g| match subset {
                Some(s) => s.contains(g),
                None => *g != "checks",
            })
            .collect();
        let found: Vec<&'static str> = groups
            .into_iter()
            .filter(|g| self.active_names(g).contains(&name))
            .collect();
        match found.as_slice() {
            [] => Err(FindError::NotFound(name.to_owned())),
            [g] => self
                .get_in(g, name)
                .ok_or_else(|| FindError::NotFound(name.to_owned())),
            _ => Err(FindError::Multiple {
                name: name.to_owned(),
                groups: found,
            }),
        }
    }

    fn names_of(&self, groups: &[&str], active_only: bool) -> BTreeSet<String> {
        groups
            .iter()
            .flat_map(|g| {
                if active_only {
                    self.active_names(g)
                } else {
                    self.all_names(g)
                }
            })
            .map(str::to_owned)
            .collect()
    }

    /// `parsing_components`.
    pub fn parsing_components(&self) -> ParsingComponents {
        const INPUTS: [&str; 2] = ["lookups", "parameters"];
        const RESULTS: [&str; 2] = ["variables", "global_expressions"];
        let expression = NameGroups {
            dimensions: self.names_of(&["dimensions"], true),
            inputs: self.names_of(&INPUTS, true),
            results: self.names_of(&RESULTS, true),
        };
        let all_active: HashSet<&String> = expression
            .inputs
            .iter()
            .chain(&expression.results)
            .collect();
        let mut where_ = expression.clone();
        for n in self.names_of(&INPUTS, false) {
            if !all_active.contains(&n) {
                where_.inputs.insert(n);
            }
        }
        for n in self.names_of(&RESULTS, false) {
            if !all_active.contains(&n) {
                where_.results.insert(n);
            }
        }
        ParsingComponents { expression, where_ }
    }
}

/// `CalliopeMath`: the math attribute container (`init` holds the raw math files by name).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CalliopeMath {
    /// Raw math definitions by name (`base`, `operate`, user files, ...).
    #[schemars(with = "IndexMap<AttrStr, AnyValue>")]
    pub init: IndexMap<AttrStr, IndexMap<String, Node>>,
    /// The math used to build the problem.
    pub build: CalliopeBuildMath,
}

static EQUATIONS_DICT: DictShape = DictShape {
    name: "SubExpressions",
    entry: Shape::List,
};

macro_rules! component_shape {
    ($static:ident, $name:literal, [$($f:expr),*]) => {
        static $static: ModelShape = ModelShape { name: $name, fields: &[$($f),*], extra: Shape::Leaf };
    };
}

const EQ_FIELDS: [(&str, Shape); 3] = [
    ("equations", Shape::List),
    ("sub_expressions", Shape::Dict(&EQUATIONS_DICT)),
    ("slices", Shape::Dict(&EQUATIONS_DICT)),
];

component_shape!(DIMENSION, "Dimension", []);
component_shape!(PARAMETER, "Parameter", []);
component_shape!(LOOKUP, "Lookup", []);
component_shape!(BOUNDS, "Bounds", []);
component_shape!(VARIABLE, "Variable", [("bounds", Shape::Model(&BOUNDS))]);
component_shape!(
    CONSTRAINT,
    "Constraint",
    [EQ_FIELDS[0], EQ_FIELDS[1], EQ_FIELDS[2]]
);
component_shape!(PIECEWISE, "PiecewiseConstraint", []);
component_shape!(
    GLOBAL_EXPRESSION,
    "GlobalExpression",
    [EQ_FIELDS[0], EQ_FIELDS[1], EQ_FIELDS[2]]
);
component_shape!(
    OBJECTIVE,
    "Objective",
    [EQ_FIELDS[0], EQ_FIELDS[1], EQ_FIELDS[2]]
);
component_shape!(
    EQUATION_COMPONENT,
    "MathEquationComponent",
    [EQ_FIELDS[0], EQ_FIELDS[1], EQ_FIELDS[2]]
);
component_shape!(
    POSTPROCESSED,
    "PostprocessedExpression",
    [EQ_FIELDS[0], EQ_FIELDS[1], EQ_FIELDS[2]]
);
component_shape!(CHECK, "Check", []);

macro_rules! dict_shape {
    ($static:ident, $name:literal, $entry:ident) => {
        static $static: DictShape = DictShape {
            name: $name,
            entry: Shape::Model(&$entry),
        };
    };
}
dict_shape!(DIMENSIONS, "Dimensions", DIMENSION);
dict_shape!(PARAMETERS, "Parameters", PARAMETER);
dict_shape!(LOOKUPS, "Lookups", LOOKUP);
dict_shape!(VARIABLES, "Variables", VARIABLE);
dict_shape!(GLOBAL_EXPRESSIONS, "GlobalExpressions", GLOBAL_EXPRESSION);
dict_shape!(CONSTRAINTS, "Constraints", CONSTRAINT);
dict_shape!(PIECEWISES, "PiecewiseConstraints", PIECEWISE);
dict_shape!(OBJECTIVES, "Objectives", OBJECTIVE);
dict_shape!(POSTPROCESSEDS, "PostprocessedExpressions", POSTPROCESSED);
dict_shape!(CHECKS, "Checks", CHECK);

pub(crate) static BUILD_MATH: ModelShape = ModelShape {
    name: "CalliopeBuildMath",
    fields: &[
        ("dimensions", Shape::Dict(&DIMENSIONS)),
        ("parameters", Shape::Dict(&PARAMETERS)),
        ("lookups", Shape::Dict(&LOOKUPS)),
        ("variables", Shape::Dict(&VARIABLES)),
        ("global_expressions", Shape::Dict(&GLOBAL_EXPRESSIONS)),
        ("constraints", Shape::Dict(&CONSTRAINTS)),
        ("piecewise_constraints", Shape::Dict(&PIECEWISES)),
        ("objectives", Shape::Dict(&OBJECTIVES)),
        ("postprocessed", Shape::Dict(&POSTPROCESSEDS)),
        ("checks", Shape::Dict(&CHECKS)),
    ],
    extra: Shape::Leaf,
};

/// `CalliopeInputMath` entries are dict models, which `CalliopeDictModel.update` replaces.
static INPUT_MATH: DictShape = DictShape {
    name: "CalliopeInputMath",
    entry: Shape::Leaf,
};

pub(crate) static MATH: ModelShape = ModelShape {
    name: "CalliopeMath",
    fields: &[
        ("init", Shape::Dict(&INPUT_MATH)),
        ("build", Shape::Model(&BUILD_MATH)),
    ],
    extra: Shape::Leaf,
};

impl CalliopeBuildMath {
    fn validate_inner(&self, path: &str, errors: &mut Vec<FieldError>) {
        for (group, map) in [
            ("constraints", &self.constraints as &dyn EqMap),
            ("global_expressions", &self.global_expressions),
            ("objectives", &self.objectives),
            ("postprocessed", &self.postprocessed),
        ] {
            for (name, active, eqs) in map.eq_items() {
                check_equations(active, eqs, &join(&join(path, group), name), errors);
            }
        }
        // `unique_component_names`: names may appear in only one group among active ones.
        let mut seen: HashSet<&str> = HashSet::new();
        let mut duplicates: BTreeSet<&str> = BTreeSet::new();
        let mut groups: Vec<HashSet<&str>> = COMPONENT_GROUPS
            .iter()
            .map(|g| self.active_names(g).into_iter().collect())
            .collect();
        groups.sort_by_key(HashSet::len);
        for names in groups {
            duplicates.extend(names.intersection(&seen).copied());
            seen.extend(names);
        }
        if !duplicates.is_empty() {
            let list: Vec<String> = duplicates.iter().map(|d| format!("'{d}'")).collect();
            errors.push(FieldError::new(
                path,
                format!(
                    "Non-unique names in math components: [{}].",
                    list.join(", ")
                ),
            ));
        }
    }
}

trait EqMap {
    fn eq_items(&self) -> Vec<(&str, bool, &Equations)>;
}

macro_rules! eq_map {
    ($t:ty) => {
        impl EqMap for IndexMap<AttrStr, $t> {
            fn eq_items(&self) -> Vec<(&str, bool, &Equations)> {
                self.iter()
                    .map(|(k, v)| (k.as_str(), v.active, &v.equations))
                    .collect()
            }
        }
    };
}
eq_map!(Constraint);
eq_map!(GlobalExpression);
eq_map!(Objective);

impl Schema for CalliopeBuildMath {
    const TITLE: &'static str = "Model math schema";
    fn shape() -> Shape {
        Shape::Model(&BUILD_MATH)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.validate_inner(path, errors);
    }
}

impl Schema for CalliopeMath {
    const TITLE: &'static str = "CalliopeMath";
    fn shape() -> Shape {
        Shape::Model(&MATH)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.build.validate_inner(&join(path, "build"), errors);
    }
}

macro_rules! component_schema {
    ($t:ty, $title:literal, $shape:ident) => {
        impl Schema for $t {
            const TITLE: &'static str = $title;
            fn shape() -> Shape {
                Shape::Model(&$shape)
            }
        }
    };
    ($t:ty, $title:literal, $shape:ident, equations) => {
        impl Schema for $t {
            const TITLE: &'static str = $title;
            fn shape() -> Shape {
                Shape::Model(&$shape)
            }
            fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
                check_equations(self.active, &self.equations, path, errors);
            }
        }
    };
}

component_schema!(Dimension, "Dimension", DIMENSION);
component_schema!(Parameter, "Parameter", PARAMETER);
component_schema!(Lookup, "Lookup", LOOKUP);
component_schema!(Variable, "Variable", VARIABLE);
component_schema!(Bounds, "Bounds", BOUNDS);
component_schema!(Constraint, "Constraint", CONSTRAINT, equations);
component_schema!(PiecewiseConstraint, "PiecewiseConstraint", PIECEWISE);
component_schema!(
    GlobalExpression,
    "GlobalExpression",
    GLOBAL_EXPRESSION,
    equations
);
component_schema!(Objective, "Objective", OBJECTIVE, equations);
component_schema!(
    MathEquationComponent,
    "MathEquationComponent",
    EQUATION_COMPONENT,
    equations
);
component_schema!(Check, "Check", CHECK);
