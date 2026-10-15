//! Static checks of the math before anything is built. Calliope has no counterpart: it
//! finds these problems while building (if at all), one at a time.
//!
//! Every sub-expression is given a [`Ty`]: a [`Kind`] (constant, parameter data, affine
//! expression in the decision variables, non-linear, boolean mask, string, datetime) and
//! the dimensions it is indexed over. Parameter and lookup dimensions are not part of the
//! math; without model data they are *open* (any dimensions), and checks that need them
//! are skipped. [`InputSchema`] supplies them from model data.
//!
//! Errors are what makes Calliope fail (or would make it fail) at build time; warnings are
//! things that build but almost certainly do not do what was meant, such as summing over a
//! dimension name that does not exist (Calliope silently ignores it).

use std::collections::{BTreeMap, BTreeSet};

use calliope_schema::{BoundValue, CalliopeBuildMath, DimensionDtype, LookupDtype};

use crate::ast::{ArithOp, CmpOp, Lang, LogicOp, Node, NodeKind, Role, Span};
use crate::component::{
    Equation, Group, ParseMode, ParsedComponent, ParsedString, calliope_visible_refs, parse_math,
};
use crate::diagnostic::{MathDiagnostic, StringPath};
use crate::helpers::{DATETIME_GROUPS, Helper, bind};

/// What a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A number literal.
    Number,
    /// Numeric data (parameters, numeric lookups, results in postprocessing).
    Param,
    /// Linear in the decision variables.
    Affine,
    /// Not linear in the decision variables.
    NonLinear,
    /// A boolean mask or boolean data.
    Bool,
    /// Strings (literals, string lookups, dimension items).
    Str,
    /// Datetimes.
    Datetime,
    /// A list literal.
    List,
    /// Unknown after an error (suppresses follow-up errors).
    Unknown,
}

impl Kind {
    fn describe(self) -> &'static str {
        match self {
            Kind::Number => "a number",
            Kind::Param => "numeric data",
            Kind::Affine => "a linear expression",
            Kind::NonLinear => "a non-linear expression",
            Kind::Bool => "a boolean",
            Kind::Str => "a string",
            Kind::Datetime => "a datetime",
            Kind::List => "a list",
            Kind::Unknown => "unknown",
        }
    }

    fn is_numeric(self) -> bool {
        matches!(
            self,
            Kind::Number | Kind::Param | Kind::Affine | Kind::NonLinear | Kind::Bool
        )
    }

    fn has_variables(self) -> bool {
        matches!(self, Kind::Affine | Kind::NonLinear)
    }
}

/// The dimensions a value is indexed over.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dims {
    /// Dimensions it certainly has.
    pub known: BTreeSet<String>,
    /// True if it may have further, unknown dimensions (input data without a schema).
    pub open: bool,
}

impl Dims {
    /// No dimensions.
    pub fn scalar() -> Self {
        Self::default()
    }
    /// Exactly these dimensions.
    pub fn of<I: IntoIterator<Item = S>, S: Into<String>>(dims: I) -> Self {
        Self {
            known: dims.into_iter().map(Into::into).collect(),
            open: false,
        }
    }
    /// Unknown dimensions.
    pub fn unknown() -> Self {
        Self {
            known: BTreeSet::new(),
            open: true,
        }
    }
    fn union(&self, other: &Dims) -> Dims {
        Dims {
            known: self.known.union(&other.known).cloned().collect(),
            open: self.open || other.open,
        }
    }
    fn without<'a>(&self, dims: impl IntoIterator<Item = &'a str>) -> Dims {
        let mut d = self.clone();
        for x in dims {
            d.known.remove(x);
        }
        d
    }
    /// True if `dim` is certainly absent.
    fn lacks(&self, dim: &str) -> bool {
        !self.open && !self.known.contains(dim)
    }
}

/// A value's type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ty {
    /// What it is.
    pub kind: Kind,
    /// Its dimensions.
    pub dims: Dims,
}

impl Ty {
    fn new(kind: Kind, dims: Dims) -> Self {
        Self { kind, dims }
    }
    fn scalar(kind: Kind) -> Self {
        Self::new(kind, Dims::scalar())
    }
    fn unknown() -> Self {
        Self::new(Kind::Unknown, Dims::unknown())
    }
}

/// The dimensions of input data (parameters and lookups) in a model. Inputs that are not
/// listed have unknown dimensions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputSchema {
    /// Input name to its dimensions.
    pub dims: BTreeMap<String, Vec<String>>,
}

/// The result of the typecheck.
#[derive(Debug, Clone, Default)]
pub struct TypecheckReport {
    /// Parsed components, in build order.
    pub components: Vec<ParsedComponent>,
    /// The type of each expanded equation, by component and equation name.
    pub equation_types: BTreeMap<(String, String), Vec<(String, Ty)>>,
    /// Errors and warnings, sorted by location.
    pub diagnostics: Vec<MathDiagnostic>,
}

impl TypecheckReport {
    /// The errors.
    pub fn errors(&self) -> impl Iterator<Item = &MathDiagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }
    /// The warnings.
    pub fn warnings(&self) -> impl Iterator<Item = &MathDiagnostic> {
        self.diagnostics.iter().filter(|d| !d.is_error())
    }
}

#[derive(Debug, Clone)]
struct ResultInfo {
    kind: Kind,
    dims: Dims,
    /// Position in build order; `usize::MAX` for variables (always defined first).
    position: usize,
    group: Group,
}

struct Env {
    dimensions: BTreeMap<String, DimensionDtype>,
    inputs: BTreeMap<String, (Kind, Dims)>,
    parameters: BTreeSet<String>,
    lookups: BTreeSet<String>,
    results: BTreeMap<String, ResultInfo>,
    config: BTreeMap<String, Kind>,
}

fn lookup_kind(dtype: LookupDtype) -> Kind {
    match dtype {
        LookupDtype::Float => Kind::Param,
        LookupDtype::Bool => Kind::Bool,
        LookupDtype::String => Kind::Str,
        LookupDtype::Datetime | LookupDtype::Date => Kind::Datetime,
    }
}

fn dim_kind(dtype: DimensionDtype) -> Kind {
    match dtype {
        DimensionDtype::String => Kind::Str,
        DimensionDtype::Datetime | DimensionDtype::Date => Kind::Datetime,
        DimensionDtype::Float | DimensionDtype::Integer => Kind::Param,
    }
}

/// The `build` configuration options (`config.<option>` in `where`) and their kinds.
fn build_config_kinds() -> BTreeMap<String, Kind> {
    [
        ("backend", Kind::Str),
        ("ensure_feasibility", Kind::Bool),
        ("objective", Kind::Str),
        // `operate` is a mapping: Calliope rejects it as a `where` value.
        ("operate", Kind::Unknown),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// Typechecks the math (as built: active components with the build names).
pub fn typecheck(math: &CalliopeBuildMath, schema: Option<&InputSchema>) -> TypecheckReport {
    let (components, mut diagnostics) = parse_math(math, ParseMode::Build);
    let mut env = Env {
        dimensions: math
            .dimensions
            .iter()
            .map(|(k, v)| (k.to_string(), v.dtype))
            .collect(),
        inputs: BTreeMap::new(),
        parameters: math.parameters.keys().map(ToString::to_string).collect(),
        lookups: math.lookups.keys().map(ToString::to_string).collect(),
        results: BTreeMap::new(),
        config: build_config_kinds(),
    };
    let input_dims = |name: &str| match schema {
        // An input missing from the data is a scalar default in Calliope, but the strings
        // that use it are then usually masked out by their `where`; treat it as unknown.
        Some(s) => s
            .dims
            .get(name)
            .map_or_else(Dims::unknown, |d| Dims::of(d.clone())),
        None => Dims::unknown(),
    };
    for name in math.parameters.keys() {
        env.inputs
            .insert(name.to_string(), (Kind::Param, input_dims(name.as_str())));
    }
    for (name, l) in &math.lookups {
        env.inputs.insert(
            name.to_string(),
            (lookup_kind(l.dtype), input_dims(name.as_str())),
        );
    }
    // Variables are defined before anything references them; inactive ones still have an
    // (empty) entry that `where` strings may test.
    for (name, v) in &math.variables {
        env.results.insert(
            name.to_string(),
            ResultInfo {
                kind: Kind::Affine,
                dims: Dims::of(v.foreach.iter().map(|d| d.as_str().to_owned())),
                position: usize::MAX,
                group: Group::Variables,
            },
        );
    }
    let position: BTreeMap<(Group, &str), usize> = components
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.group, c.name.as_str()), i))
        .collect();
    for (name, g) in &math.global_expressions {
        env.results.insert(
            name.to_string(),
            ResultInfo {
                kind: Kind::Affine,
                dims: Dims::of(g.foreach.iter().map(|d| d.as_str().to_owned())),
                position: position
                    .get(&(Group::GlobalExpressions, name.as_str()))
                    .copied()
                    .unwrap_or(usize::MAX - 1),
                group: Group::GlobalExpressions,
            },
        );
    }
    for (name, g) in &math.postprocessed {
        env.results.insert(
            name.to_string(),
            ResultInfo {
                kind: Kind::Param,
                dims: Dims::of(g.foreach.iter().map(|d| d.as_str().to_owned())),
                position: position
                    .get(&(Group::Postprocessed, name.as_str()))
                    .copied()
                    .unwrap_or(usize::MAX - 1),
                group: Group::Postprocessed,
            },
        );
    }

    let mut equation_types = BTreeMap::new();
    for (i, comp) in components.iter().enumerate() {
        let mut c = Checker {
            env: &env,
            comp,
            position: i,
            diags: Vec::new(),
        };
        let types = c.component(math);
        let diags = c.diags;
        diagnostics.extend(diags);
        if comp.group == Group::GlobalExpressions && !types.is_empty() {
            let kind = types
                .iter()
                .map(|(_, t)| t.kind)
                .filter(|k| k.is_numeric())
                .max()
                .unwrap_or(Kind::Param);
            if let Some(r) = env.results.get_mut(&comp.name) {
                r.kind = if kind.has_variables() {
                    kind
                } else {
                    Kind::Param
                };
            }
        }
        equation_types.insert((comp.group.as_str().to_owned(), comp.name.clone()), types);
    }

    // De-duplicate (expanded equations share their strings) and sort by location.
    let mut seen = BTreeSet::new();
    diagnostics.retain(|d| {
        seen.insert((
            d.path.clone(),
            d.span.start,
            d.span.end,
            d.code,
            d.message.clone(),
        ))
    });
    diagnostics.sort_by(|a, b| (&a.path, a.span.start).cmp(&(&b.path, b.span.start)));
    TypecheckReport {
        components,
        equation_types,
        diagnostics,
    }
}

struct Checker<'a> {
    env: &'a Env,
    comp: &'a ParsedComponent,
    position: usize,
    diags: Vec<MathDiagnostic>,
}

/// How names evaluate in a `where` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Expressions: components are their values (inputs filled with defaults).
    Expression,
    /// `where` conditions: inputs and results become "is defined" masks.
    Presence,
    /// Inside a `where` comparison: inputs are values.
    Values,
}

impl Checker<'_> {
    fn error(&mut self, code: &'static str, src: &ParsedString, span: Span, msg: String) {
        self.diags.push(MathDiagnostic::error(
            code,
            src.path.clone(),
            &src.source,
            span,
            msg,
        ));
    }

    fn warn(&mut self, code: &'static str, src: &ParsedString, span: Span, msg: String) {
        self.diags.push(MathDiagnostic::warning(
            code,
            src.path.clone(),
            &src.source,
            span,
            msg,
        ));
    }

    fn foreach(&self) -> Dims {
        Dims::of(self.comp.foreach.iter().cloned())
    }

    fn component(&mut self, math: &CalliopeBuildMath) -> Vec<(String, Ty)> {
        let comp = self.comp;
        for d in &comp.foreach {
            if !self.env.dimensions.contains_key(d) {
                let src = &comp.where_;
                self.diags.push(MathDiagnostic::error(
                    "math::foreach::unknown-dimension",
                    StringPath {
                        field: "foreach".into(),
                        ..src.path.clone()
                    },
                    d,
                    0..d.len(),
                    format!(
                        "`foreach` has `{d}`, which is not a dimension; Calliope skips the component without an error"
                    ),
                ));
            }
        }
        self.where_string(&comp.where_);
        if comp.group == Group::Variables {
            self.variable_bounds(math);
        }
        let mut types = Vec::new();
        for eq in &comp.equations {
            for w in &eq.wheres {
                self.where_string(w);
            }
            let ty = self.equation(eq);
            self.never_matches(eq);
            types.push((eq.name.clone(), ty));
        }
        self.unused_and_hidden_references();
        if let Some(pw) = &comp.piecewise {
            for (src, values) in [
                (&pw.x_expression, &pw.x_values),
                (&pw.y_expression, &pw.y_values),
            ] {
                let ty = self.expr(&src.node, src, None, Mode::Expression);
                self.check_result(&src.node, src, &ty);
                if !self.env.parameters.contains(values) {
                    self.error(
                        "math::piecewise::values",
                        src,
                        0..src.source.len(),
                        format!("Piecewise breakpoints `{values}` is not a parameter"),
                    );
                }
            }
        }
        types
    }

    fn variable_bounds(&mut self, math: &CalliopeBuildMath) {
        let Some(v) = math.variables.get(self.comp.name.as_str()) else {
            return;
        };
        for (which, b) in [("min", &v.bounds.min), ("max", &v.bounds.max)] {
            if let BoundValue::Param(p) = b
                && !self.env.parameters.contains(p.as_str())
            {
                let src = &self.comp.where_;
                self.diags.push(MathDiagnostic::error(
                    "math::bounds::unknown-parameter",
                    StringPath {
                        field: format!("bounds.{which}"),
                        ..src.path.clone()
                    },
                    p.as_str(),
                    0..p.as_str().len(),
                    format!("Bound `{p}` is not a parameter"),
                ));
            }
        }
    }

    // ---- where strings -----------------------------------------------------------

    fn where_string(&mut self, src: &ParsedString) {
        let ty = self.expr(&src.node, src, None, Mode::Presence);
        if !matches!(ty.kind, Kind::Bool | Kind::Unknown) {
            self.warn(
                "math::where::not-boolean",
                src,
                src.node.span.clone(),
                format!(
                    "`where` condition is {}, not a boolean; Calliope treats non-zero values as true",
                    ty.kind.describe()
                ),
            );
        }
        src.node.walk(&mut |n| {
            if let NodeKind::Logic(_, rest) = &n.kind {
                let and = rest.iter().any(|(op, _)| *op == LogicOp::And);
                let or = rest.iter().any(|(op, _)| *op == LogicOp::Or);
                if and && or {
                    self.diags.push(
                        MathDiagnostic::warning(
                            "math::where::and-or-precedence",
                            src.path.clone(),
                            &src.source,
                            n.span.clone(),
                            "`and` and `or` have the same precedence and apply left to right",
                        )
                        .with_help("add parentheses to make the grouping explicit"),
                    );
                }
            }
        });
    }

    // ---- equations ---------------------------------------------------------------

    fn equation(&mut self, eq: &Equation) -> Ty {
        let src = &eq.expression;
        if let NodeKind::Compare { lhs, rhs, .. } = &src.node.kind {
            let l = self.expr(lhs, src, Some(eq), Mode::Expression);
            let r = self.expr(rhs, src, Some(eq), Mode::Expression);
            self.check_result(lhs, src, &l);
            self.check_result(rhs, src, &r);
            if l.kind.is_numeric()
                && r.kind.is_numeric()
                && !l.kind.has_variables()
                && !r.kind.has_variables()
            {
                self.warn(
                    "math::constraint::no-variables",
                    src,
                    src.node.span.clone(),
                    "Neither side of the constraint has a decision variable".into(),
                );
            }
            let kind = l.kind.max(r.kind);
            return Ty::new(kind, l.dims.union(&r.dims));
        }
        let ty = self.expr(&src.node, src, Some(eq), Mode::Expression);
        self.check_result(&src.node, src, &ty);
        ty
    }

    /// Checks the type of a whole expression (or one side of a constraint).
    fn check_result(&mut self, node: &Node, src: &ParsedString, ty: &Ty) {
        // Non-linearity is reported where it arises (in the arithmetic).
        match ty.kind {
            Kind::Str | Kind::Datetime | Kind::List => self.error(
                "math::dtype",
                src,
                node.span.clone(),
                format!("The expression is {}, not a number", ty.kind.describe()),
            ),
            _ => {}
        }
        let foreach = self.foreach();
        let extra: Vec<&String> = ty.dims.known.difference(&foreach.known).collect();
        if !extra.is_empty() && ty.kind != Kind::Unknown {
            let list: Vec<String> = extra.iter().map(|d| format!("'{d}'")).collect();
            self.error(
                "math::dims::not-in-foreach",
                src,
                node.span.clone(),
                format!(
                    "The expression is indexed over dimensions not present in `foreach`: {{{}}}",
                    list.join(", ")
                ),
            );
        }
    }

    // ---- expressions -------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn expr(&mut self, n: &Node, src: &ParsedString, eq: Option<&Equation>, mode: Mode) -> Ty {
        match &n.kind {
            NodeKind::Number(_) => Ty::scalar(Kind::Number),
            NodeKind::Bool(_) => Ty::scalar(Kind::Bool),
            NodeKind::Str(_) => Ty::scalar(Kind::Str),
            NodeKind::List(_) => Ty::scalar(Kind::List),
            NodeKind::Config(key) => {
                match self.env.config.get(key) {
                    Some(Kind::Unknown) => {
                        self.error(
                        "math::config::type",
                        src,
                        n.span.clone(),
                        format!("Configuration option `config.{key}` is not a number, string or boolean"),
                    );
                        Ty::unknown()
                    }
                    Some(k) => Ty::scalar(*k),
                    None => {
                        self.error(
                            "math::config::unknown",
                            src,
                            n.span.clone(),
                            format!("`config.{key}` is not a build configuration option"),
                        );
                        Ty::unknown()
                    }
                }
            }
            NodeKind::Ref { name, role, .. } => self.reference(name, *role, n, src, mode),
            NodeKind::SubExpr(name) => match eq.and_then(|e| e.sub_expressions.get(name)) {
                Some(sub) => self.expr(&sub.node, sub, eq, mode),
                None => Ty::unknown(),
            },
            NodeKind::SliceRef(name) => match eq.and_then(|e| e.slices.get(name)) {
                Some(s) => self.expr(&s.node, s, eq, mode),
                None => Ty::unknown(),
            },
            NodeKind::Sliced { name, role, slices } => {
                let base = self.reference(name, *role, n, src, mode);
                let mut dims = base.dims.clone();
                for (dim, slicer) in slices {
                    if !self.env.dimensions.contains_key(dim) {
                        self.error(
                            "math::slice::unknown-dimension",
                            src,
                            n.span.clone(),
                            format!("`{dim}` is not a dimension"),
                        );
                        continue;
                    }
                    if base.dims.lacks(dim) {
                        self.error(
                            "math::slice::missing-dimension",
                            src,
                            n.span.clone(),
                            format!(
                                "`{name}` is not indexed over `{dim}`, so it cannot be sliced on it"
                            ),
                        );
                    }
                    let st = self.expr(slicer, src, eq, Mode::Expression);
                    match st.kind {
                        Kind::List => {}
                        _ => {
                            dims = dims.without([dim.as_str()]).union(&st.dims);
                        }
                    }
                }
                Ty::new(base.kind, dims)
            }
            NodeKind::Sign(_, x) => {
                let t = self.expr(x, src, eq, mode);
                self.numeric_operand(x, src, &t);
                Ty::new(promote_bool(t.kind), t.dims)
            }
            NodeKind::Arith(first, rest) => {
                let mut acc = self.expr(first, src, eq, mode);
                self.numeric_operand(first, src, &acc);
                for (op, x) in rest {
                    let t = self.expr(x, src, eq, mode);
                    self.numeric_operand(x, src, &t);
                    let (a, b) = (promote_bool(acc.kind), promote_bool(t.kind));
                    let kind = if a == Kind::Unknown || b == Kind::Unknown {
                        Kind::Unknown
                    } else {
                        match op {
                            ArithOp::Add | ArithOp::Sub => a.max(b),
                            ArithOp::Mul if a.has_variables() && b.has_variables() => {
                                Kind::NonLinear
                            }
                            ArithOp::Div if b.has_variables() => Kind::NonLinear,
                            ArithOp::Pow if a.has_variables() || b.has_variables() => {
                                Kind::NonLinear
                            }
                            _ => a.max(b),
                        }
                    };
                    if kind == Kind::NonLinear && a != Kind::NonLinear && b != Kind::NonLinear {
                        self.error(
                            "math::nonlinear",
                            src,
                            n.span.clone(),
                            format!(
                                "`{}` between {} and {} is not linear",
                                op.as_str(),
                                a.describe(),
                                b.describe()
                            ),
                        );
                    }
                    acc = Ty::new(kind, acc.dims.union(&t.dims));
                }
                acc
            }
            NodeKind::Not(x) => {
                let t = self.expr(x, src, eq, mode);
                Ty::new(Kind::Bool, t.dims)
            }
            NodeKind::Logic(first, rest) => {
                let mut dims = self.expr(first, src, eq, mode).dims;
                for (_, x) in rest {
                    dims = dims.union(&self.expr(x, src, eq, mode).dims);
                }
                Ty::new(Kind::Bool, dims)
            }
            NodeKind::Compare { lhs, op, rhs, .. } => {
                let l = self.expr(lhs, src, eq, Mode::Values);
                let r = self.expr(rhs, src, eq, Mode::Values);
                self.comparison(n, src, &l, *op, &r, rhs);
                Ty::new(Kind::Bool, l.dims.union(&r.dims))
            }
            NodeKind::Subset { items, dim } => {
                let dkind = self.env.dimensions.get(dim).map(|d| dim_kind(*d));
                for it in items {
                    let ik = match it.kind {
                        NodeKind::Number(_) => Kind::Param,
                        NodeKind::Str(_) => Kind::Str,
                        _ => Kind::Unknown,
                    };
                    if let Some(dk) = dkind
                        && ik != Kind::Unknown
                        && dk != Kind::Datetime
                        && ik != dk
                    {
                        self.warn(
                            "math::where::subset-dtype",
                            src,
                            it.span.clone(),
                            format!(
                                "`{dim}` items are {}, this item is {}; it never matches",
                                dk.describe(),
                                ik.describe()
                            ),
                        );
                    }
                }
                Ty::new(Kind::Bool, Dims::of([dim.clone()]))
            }
            NodeKind::Call {
                name,
                name_span,
                args,
                kwargs,
            } => self.call(n, name, name_span, args, kwargs, src, eq, mode),
        }
    }

    fn numeric_operand(&mut self, n: &Node, src: &ParsedString, t: &Ty) {
        if matches!(t.kind, Kind::Str | Kind::Datetime | Kind::List) {
            self.error(
                "math::dtype",
                src,
                n.span.clone(),
                format!("Arithmetic on {}", t.kind.describe()),
            );
        }
    }

    fn reference(
        &mut self,
        name: &str,
        role: Role,
        n: &Node,
        src: &ParsedString,
        mode: Mode,
    ) -> Ty {
        match role {
            Role::Dimension => {
                let kind = self
                    .env
                    .dimensions
                    .get(name)
                    .map_or(Kind::Str, |d| dim_kind(*d));
                Ty::new(kind, Dims::of([name.to_owned()]))
            }
            Role::Input => {
                let (kind, dims) = self
                    .env
                    .inputs
                    .get(name)
                    .cloned()
                    .unwrap_or((Kind::Unknown, Dims::unknown()));
                if mode == Mode::Presence {
                    Ty::new(Kind::Bool, dims)
                } else {
                    Ty::new(kind, dims)
                }
            }
            Role::Result | Role::Postprocessed => {
                let Some(info) = self.env.results.get(name).cloned() else {
                    return Ty::unknown();
                };
                let same_group = info.group == self.comp.group;
                if same_group && info.position >= self.position && info.position != usize::MAX {
                    let what = if info.position == self.position {
                        "itself"
                    } else {
                        "a component that is built later"
                    };
                    self.error(
                        "math::order",
                        src,
                        n.span.clone(),
                        format!(
                            "`{name}` refers to {what}: Calliope fails with \"Trying to access a math component that is not yet defined\""
                        ),
                    );
                }
                let kind = if self.comp.group == Group::Postprocessed {
                    Kind::Param
                } else {
                    info.kind
                };
                if mode == Mode::Presence {
                    Ty::new(Kind::Bool, info.dims)
                } else {
                    Ty::new(kind, info.dims)
                }
            }
        }
    }

    fn comparison(&mut self, n: &Node, src: &ParsedString, l: &Ty, op: CmpOp, r: &Ty, rhs: &Node) {
        use Kind::{Bool, Datetime, Number, Param, Str, Unknown};
        let (a, b) = (l.kind, r.kind);
        if a == Unknown || b == Unknown {
            return;
        }
        let numeric = |k: Kind| matches!(k, Number | Param);
        let compatible = match (a, b) {
            (x, y) if numeric(x) && numeric(y) => true,
            (Bool, y) | (y, Bool) if numeric(y) || y == Bool => true,
            (Str, Str) | (Datetime, Datetime) => true,
            // A string compared with a datetime is parsed as a datetime by pandas.
            (Datetime, Str) => true,
            _ => false,
        };
        if !compatible {
            let what = if matches!(rhs.kind, NodeKind::Str(_)) {
                "the string"
            } else {
                "the value"
            };
            self.error(
                "math::where::comparison-dtype",
                src,
                n.span.clone(),
                format!(
                    "Comparing {} with {what} `{}`, which is {}: never true",
                    a.describe(),
                    &src.source[rhs.span.clone()],
                    b.describe()
                ),
            );
        } else if matches!(op, CmpOp::Lt | CmpOp::Gt | CmpOp::Le | CmpOp::Ge)
            && (a == Bool || a == Str)
        {
            self.warn(
                "math::where::ordering",
                src,
                n.span.clone(),
                format!("Ordering comparison on {}", a.describe()),
            );
        }
    }

    // ---- helper calls --------------------------------------------------------------

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn call(
        &mut self,
        n: &Node,
        name: &str,
        name_span: &Span,
        args: &[Node],
        kwargs: &[(String, Node)],
        src: &ParsedString,
        eq: Option<&Equation>,
        mode: Mode,
    ) -> Ty {
        let lang = if mode == Mode::Expression {
            Lang::Expression
        } else {
            Lang::Where
        };
        let Some(helper) = Helper::lookup(name, lang) else {
            let other = match lang {
                Lang::Where => Lang::Expression,
                Lang::Expression => Lang::Where,
            };
            let msg = if Helper::lookup(name, other).is_some() {
                format!(
                    "Helper function `{name}` is not allowed in {} strings",
                    if lang == Lang::Where {
                        "`where`"
                    } else {
                        "expression"
                    }
                )
            } else {
                format!("Invalid helper function defined: {name}")
            };
            self.error("math::helper::unknown", src, name_span.clone(), msg);
            return Ty::unknown();
        };
        let bound = match bind(
            helper,
            args.iter().collect(),
            kwargs.iter().map(|(k, v)| (k.clone(), v)).collect(),
        ) {
            Ok(b) => b,
            Err(e) => {
                self.error("math::helper::arguments", src, n.span.clone(), e);
                return Ty::unknown();
            }
        };
        let arg = |p: &str| *bound.get(p).expect("bound");
        // `ignore_where` helpers see unmasked data; for typing, that changes nothing.
        let ty_of = |c: &mut Self, node: &Node| c.expr(node, src, eq, mode);
        match helper {
            Helper::Any | Helper::Sum => {
                let t = ty_of(self, arg("array"));
                let over = self.dimension_list(arg("over"), src);
                for (d, span) in &over {
                    if t.dims.lacks(d) {
                        self.warn(
                            "math::helper::sum-missing-dimension",
                            src,
                            span.clone(),
                            format!(
                                "The array does not have dimension `{d}`; `{}` ignores it",
                                helper.name()
                            ),
                        );
                    }
                }
                let dims = t.dims.without(over.iter().map(|(d, _)| d.as_str()));
                if helper == Helper::Any {
                    if mode == Mode::Values && t.kind != Kind::Bool && t.kind != Kind::Unknown {
                        self.error(
                            "math::helper::any-dtype",
                            src,
                            n.span.clone(),
                            format!(
                                "Input to `any` must be a boolean array, not {}",
                                t.kind.describe()
                            ),
                        );
                    }
                    Ty::new(Kind::Bool, dims)
                } else {
                    Ty::new(promote_bool(t.kind), dims)
                }
            }
            Helper::ReduceCarrierDim => {
                let t = ty_of(self, arg("array"));
                let dir = arg("flow_direction");
                let carrier = match &dir.kind {
                    NodeKind::Str(s) if s == "in" || s == "out" => Some(format!("carrier_{s}")),
                    _ => {
                        self.error(
                            "math::helper::flow-direction",
                            src,
                            dir.span.clone(),
                            "`flow_direction` must be `in` or `out`".into(),
                        );
                        None
                    }
                };
                let cdims = carrier
                    .and_then(|c| self.env.inputs.get(&c).map(|(_, d)| d.clone()))
                    .unwrap_or_else(Dims::unknown);
                Ty::new(t.kind, t.dims.union(&cdims).without(["carriers"]))
            }
            Helper::SelectFromLookupArrays => {
                let t = ty_of(self, arg("array"));
                let mut dims = t.dims.clone();
                for (d, v) in &bound.dim_kwargs {
                    self.check_dimension(d, v, src);
                    if t.dims.lacks(d) {
                        self.error(
                            "math::helper::lookup-dimension",
                            src,
                            v.span.clone(),
                            format!(
                                "Cannot select items on `{d}`: the array is not indexed over it"
                            ),
                        );
                    }
                    let lt = ty_of(self, v);
                    self.lookup_arg(v, src);
                    for other in bound.dim_kwargs.iter().map(|(k, _)| k) {
                        if lt.dims.lacks(other) {
                            self.error(
                                "math::helper::lookup-dimension",
                                src,
                                v.span.clone(),
                                format!("Lookup arrays must be indexed over all selected dimensions; this one lacks `{other}`"),
                            );
                        }
                    }
                    dims = dims.union(&lt.dims);
                }
                Ty::new(t.kind, dims)
            }
            Helper::GetValAtIndex => {
                let (d, v) = &bound.dim_kwargs[0];
                self.check_dimension(d, v, src);
                self.integer_arg(v, src);
                let kind = self
                    .env
                    .dimensions
                    .get(d)
                    .map_or(Kind::Unknown, |x| dim_kind(*x));
                Ty::scalar(kind)
            }
            Helper::Roll => {
                let t = ty_of(self, arg("array"));
                for (d, v) in &bound.dim_kwargs {
                    self.check_dimension(d, v, src);
                    self.integer_arg(v, src);
                    if t.dims.lacks(d) {
                        self.error(
                            "math::helper::roll-dimension",
                            src,
                            v.span.clone(),
                            format!("Cannot roll over `{d}`: the array is not indexed over it"),
                        );
                    }
                }
                t
            }
            Helper::Where => {
                let t = ty_of(self, arg("array"));
                let c = ty_of(self, arg("condition"));
                if c.kind.has_variables() {
                    self.error(
                        "math::helper::where-condition",
                        src,
                        arg("condition").span.clone(),
                        "The condition of `where` cannot contain decision variables".into(),
                    );
                }
                Ty::new(t.kind, t.dims.union(&c.dims))
            }
            Helper::MapDim => {
                let d = arg("dim");
                let dims = match &d.kind {
                    NodeKind::Ref {
                        name,
                        role: Role::Dimension,
                        ..
                    } => Dims::of([name.clone()]),
                    _ => {
                        self.error(
                            "math::helper::dimension",
                            src,
                            d.span.clone(),
                            "`map_dim` needs a dimension as its first argument".into(),
                        );
                        Dims::unknown()
                    }
                };
                let m = ty_of(self, arg("mapper"));
                self.lookup_arg(arg("mapper"), src);
                Ty::new(Kind::Bool, dims.union(&m.dims))
            }
            Helper::GroupSum => {
                let t = ty_of(self, arg("array"));
                let g = ty_of(self, arg("groupby"));
                self.lookup_arg(arg("groupby"), src);
                let gd = self.dimension_list(arg("group_dim"), src);
                let removed: Vec<String> = g.dims.known.iter().cloned().collect();
                let mut dims = t.dims.without(removed.iter().map(String::as_str));
                dims.open |= g.dims.open;
                Ty::new(
                    t.kind,
                    dims.union(&Dims::of(gd.into_iter().map(|(d, _)| d))),
                )
            }
            Helper::GroupDatetime => {
                let t = ty_of(self, arg("array"));
                let over = self.dimension_list(arg("over"), src);
                for (d, span) in &over {
                    if self
                        .env
                        .dimensions
                        .get(d)
                        .is_some_and(|x| dim_kind(*x) != Kind::Datetime)
                    {
                        self.error(
                            "math::helper::datetime-dimension",
                            src,
                            span.clone(),
                            format!("`{d}` is not a datetime dimension"),
                        );
                    }
                }
                let group = self.dimension_list(arg("group"), src);
                for (g, span) in &group {
                    if !DATETIME_GROUPS.contains(&g.as_str()) {
                        self.warn(
                            "math::helper::datetime-group",
                            src,
                            span.clone(),
                            format!("`{g}` is not a datetime attribute pandas can group by"),
                        );
                    }
                }
                let dims = t
                    .dims
                    .without(over.iter().map(|(d, _)| d.as_str()))
                    .union(&Dims::of(group.into_iter().map(|(d, _)| d)));
                Ty::new(t.kind, dims)
            }
            Helper::SumNextN => {
                let t = ty_of(self, arg("array"));
                let over = self.dimension_list(arg("over"), src);
                for (d, span) in &over {
                    if t.dims.lacks(d) {
                        self.error(
                            "math::helper::sum-next-n-dimension",
                            src,
                            span.clone(),
                            format!("The array is not indexed over `{d}`"),
                        );
                    }
                }
                self.integer_arg(arg("N"), src);
                t
            }
        }
    }

    /// The dimension names in an `over`-style argument (a name or a list of names). Names
    /// that are not dimensions are reported: Calliope ignores them silently.
    fn dimension_list(&mut self, n: &Node, src: &ParsedString) -> Vec<(String, Span)> {
        let items: Vec<&Node> = match &n.kind {
            NodeKind::List(items) => items.iter().collect(),
            _ => vec![n],
        };
        let mut out = Vec::new();
        for it in items {
            let name = match &it.kind {
                NodeKind::Ref { name, .. } | NodeKind::Str(name) => name.clone(),
                _ => {
                    self.error(
                        "math::helper::dimension",
                        src,
                        it.span.clone(),
                        "Expected a dimension name".into(),
                    );
                    continue;
                }
            };
            if !self.env.dimensions.contains_key(&name) {
                self.error(
                    "math::helper::unknown-dimension",
                    src,
                    it.span.clone(),
                    format!("`{name}` is not a dimension; Calliope silently ignores it"),
                );
                continue;
            }
            out.push((name, it.span.clone()));
        }
        out
    }

    fn check_dimension(&mut self, d: &str, v: &Node, src: &ParsedString) {
        if !self.env.dimensions.contains_key(d) {
            self.error(
                "math::helper::unknown-dimension",
                src,
                v.span.clone(),
                format!("`{d}` is not a dimension"),
            );
        }
    }

    fn integer_arg(&mut self, v: &Node, src: &ParsedString) {
        let ok = match &v.kind {
            NodeKind::Number(x) => x.value.is_finite() && x.value.fract() == 0.0,
            NodeKind::Sign(_, x) => {
                matches!(&x.kind, NodeKind::Number(x) if x.value.is_finite() && x.value.fract() == 0.0)
            }
            _ => false,
        };
        if !ok {
            self.error(
                "math::helper::integer",
                src,
                v.span.clone(),
                "Expected an integer".into(),
            );
        }
    }

    fn lookup_arg(&mut self, v: &Node, src: &ParsedString) {
        let ok = match &v.kind {
            NodeKind::Ref { name, role, .. } => {
                *role == Role::Input && self.env.lookups.contains(name.as_str())
            }
            _ => false,
        };
        if !ok {
            self.error(
                "math::helper::lookup",
                src,
                v.span.clone(),
                "Expected the name of a lookup array".into(),
            );
        }
    }

    // ---- whole-equation checks ----------------------------------------------------

    /// Warns about expanded equations whose `where` strings exclude each other.
    fn never_matches(&mut self, eq: &Equation) {
        let mut facts: Vec<Literal> = Vec::new();
        for w in std::iter::once(&self.comp.where_).chain(&eq.wheres) {
            for c in conjuncts(&w.node) {
                facts.push(literal(c));
            }
        }
        let mut conflict = None;
        for (i, (neg_a, a, eq_a)) in facts.iter().enumerate() {
            for (neg_b, b, eq_b) in &facts[i + 1..] {
                if a == b && neg_a != neg_b {
                    conflict = Some(format!("`{a}` and `not {a}`"));
                }
                if let (false, false, Some((x, v)), Some((y, w))) = (neg_a, neg_b, eq_a, eq_b)
                    && x == y
                    && v != w
                {
                    conflict = Some(format!("`{x}=={v}` and `{y}=={w}`"));
                }
            }
        }
        if let Some(what) = conflict {
            let src = &eq.expression;
            self.warn(
                "math::never-matches",
                src,
                src.node.span.clone(),
                format!(
                    "`{}` never applies: its `where` strings require {what}",
                    eq.name
                ),
            );
        }
    }

    fn unused_and_hidden_references(&mut self) {
        let comp = self.comp;
        let mut used_subs = BTreeSet::new();
        let mut used_slices = BTreeSet::new();
        for item in &comp.equation_items {
            used_subs.extend(item.expression.node.sub_expression_refs());
            used_slices.extend(item.expression.node.slice_refs());
        }
        for items in comp.sub_expression_items.values() {
            for it in items {
                used_slices.extend(it.expression.node.slice_refs());
            }
        }
        for (kind, defined, used) in [
            ("sub_expressions", &comp.sub_expression_items, &used_subs),
            ("slices", &comp.slice_items, &used_slices),
        ] {
            for (name, items) in defined {
                if !used.contains(name)
                    && let Some(first) = items.first()
                {
                    let src = &first.expression;
                    self.warn(
                        "math::unused",
                        src,
                        src.node.span.clone(),
                        format!("{kind} `{name}` is defined but never used"),
                    );
                }
            }
        }
        // References Calliope's own search does not find (under a unary sign).
        for eq in &comp.equations {
            let node = &eq.expression.node;
            let subs: Vec<&Node> = eq.sub_expressions.values().map(|s| &s.node).collect();
            let all_subs = node.sub_expression_refs();
            let mut all_slices = node.slice_refs();
            for s in &subs {
                all_slices.extend(s.slice_refs());
            }
            let seen_subs = calliope_visible_refs(node, &[], false);
            let seen_slices = calliope_visible_refs(node, &subs, true);
            let hidden: Vec<&String> = all_subs
                .iter()
                .filter(|r| !seen_subs.contains(r))
                .chain(all_slices.iter().filter(|r| !seen_slices.contains(r)))
                .collect();
            if let Some(h) = hidden.first() {
                let src = &eq.expression;
                self.warn(
                    "math::calliope::hidden-reference",
                    src,
                    src.node.span.clone(),
                    format!(
                        "Calliope does not find `${h}` here (it is under a unary sign) and fails to build `{}`",
                        eq.name
                    ),
                );
            }
        }
    }
}

fn promote_bool(k: Kind) -> Kind {
    if k == Kind::Bool { Kind::Param } else { k }
}

/// The top-level `and` terms of a `where` condition (a chain with `or` is one term).
fn conjuncts(n: &Node) -> Vec<&Node> {
    match &n.kind {
        NodeKind::Logic(first, rest) if rest.iter().all(|(op, _)| *op == LogicOp::And) => {
            let mut out = conjuncts(first);
            for (_, x) in rest {
                out.extend(conjuncts(x));
            }
            out
        }
        _ => vec![n],
    }
}

/// A condition as (negated, canonical text, `name==value` if it is one).
type Literal = (bool, String, Option<(String, String)>);

fn literal(n: &Node) -> Literal {
    match &n.kind {
        NodeKind::Not(x) => {
            let (neg, s, eq) = literal(x);
            (!neg, s, if neg { eq } else { None })
        }
        NodeKind::Compare {
            lhs,
            op: CmpOp::Eq,
            rhs,
            ..
        } => {
            let eq = match (&lhs.kind, &rhs.kind) {
                (
                    NodeKind::Ref { name, .. } | NodeKind::Config(name),
                    NodeKind::Str(_) | NodeKind::Number(_) | NodeKind::Bool(_),
                ) => Some((name.clone(), rhs.to_string())),
                _ => None,
            };
            (false, n.to_string(), eq)
        }
        _ => (false, n.to_string(), None),
    }
}
