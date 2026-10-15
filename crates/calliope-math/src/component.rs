//! Parsing whole math components and expanding their equations
//! (`calliope.backend.parsing.ParsedBackendComponent`).
//!
//! A component's `equations` are expanded with the product of the `sub_expressions` they
//! reference, then with the product of the `slices` referenced by the result. Each
//! expanded equation is named as Calliope names it: `constraints:foo:0`, then
//! `constraints:foo:0-bar:1` for sub-expression `bar` item 1, then
//! `constraints:foo:0-bar:1-baz:0` for slice `baz` item 0.

use indexmap::IndexMap;

use calliope_schema::{CalliopeBuildMath, ExpressionItem};

use crate::ast::{Node, NodeKind};
use crate::diagnostic::{MathDiagnostic, StringPath};
use crate::names::NameContext;
use crate::parse::{StringKind, parse};

/// The component groups, in the order Calliope builds them, then postprocessed and checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    /// `variables`
    Variables,
    /// `global_expressions`
    GlobalExpressions,
    /// `constraints`
    Constraints,
    /// `piecewise_constraints`
    PiecewiseConstraints,
    /// `objectives`
    Objectives,
    /// `postprocessed`
    Postprocessed,
    /// `checks`
    Checks,
}

impl Group {
    /// The group's key in the math.
    pub fn as_str(self) -> &'static str {
        match self {
            Group::Variables => "variables",
            Group::GlobalExpressions => "global_expressions",
            Group::Constraints => "constraints",
            Group::PiecewiseConstraints => "piecewise_constraints",
            Group::Objectives => "objectives",
            Group::Postprocessed => "postprocessed",
            Group::Checks => "checks",
        }
    }

    /// The grammar of the group's equations (`ParsedBackendComponent.PARSERS`).
    pub fn equation_kind(self) -> StringKind {
        match self {
            Group::Constraints => StringKind::Equation,
            _ => StringKind::Arithmetic,
        }
    }
}

/// A parsed math string and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedString {
    /// Its location in the math.
    pub path: StringPath,
    /// The string.
    pub source: String,
    /// Its parse tree.
    pub node: Node,
}

/// One entry of `equations`, a `sub_expressions` list or a `slices` list.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedItem {
    /// Calliope's name: `<group>:<component>:<i>` for equations, `<name>:<i>` otherwise.
    pub name: String,
    /// Its `where` string.
    pub where_: ParsedString,
    /// Its expression.
    pub expression: ParsedString,
}

/// One equation after expansion.
#[derive(Debug, Clone, PartialEq)]
pub struct Equation {
    /// Calliope's name for it (`constraints:foo:0-bar:1`).
    pub name: String,
    /// The equation expression.
    pub expression: ParsedString,
    /// The `where` strings that apply on top of the component's: the equation's, then
    /// those of the chosen sub-expressions and slices.
    pub wheres: Vec<ParsedString>,
    /// The chosen item of each referenced sub-expression.
    pub sub_expressions: IndexMap<String, ParsedString>,
    /// The chosen item of each referenced slice.
    pub slices: IndexMap<String, ParsedString>,
}

/// The `x`/`y` part of a piecewise constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct PiecewiseAxes {
    /// `x_expression`.
    pub x_expression: ParsedString,
    /// `x_values` (a parameter name).
    pub x_values: String,
    /// `y_expression`.
    pub y_expression: ParsedString,
    /// `y_values` (a parameter name).
    pub y_values: String,
}

/// A parsed component.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedComponent {
    /// The group.
    pub group: Group,
    /// The name.
    pub name: String,
    /// `foreach` dimensions.
    pub foreach: Vec<String>,
    /// The top-level `where`.
    pub where_: ParsedString,
    /// The equations, unexpanded.
    pub equation_items: Vec<ParsedItem>,
    /// The sub-expressions, unexpanded.
    pub sub_expression_items: IndexMap<String, Vec<ParsedItem>>,
    /// The slices, unexpanded.
    pub slice_items: IndexMap<String, Vec<ParsedItem>>,
    /// The expanded equations, in Calliope's order.
    pub equations: Vec<Equation>,
    /// Piecewise constraint axes.
    pub piecewise: Option<PiecewiseAxes>,
}

/// Which components to parse and with which names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    /// Active components with the build names (`backend_model`).
    Build,
    /// All components with all names, active or not (`pre_validate_math_strings`).
    PreValidate,
}

struct Parser<'a> {
    names: &'a NameContext,
    diagnostics: Vec<MathDiagnostic>,
}

impl Parser<'_> {
    fn parse_string(
        &mut self,
        kind: StringKind,
        path: StringPath,
        source: &str,
        postprocessed: bool,
    ) -> Option<ParsedString> {
        let names = self.names.names(kind.lang(), postprocessed);
        match parse(kind, source, &names) {
            Ok(node) => Some(ParsedString {
                path,
                source: source.to_owned(),
                node,
            }),
            Err(e) => {
                self.diagnostics.push(
                    MathDiagnostic::error("math::parse", path, source, e.span, e.message)
                        .with_label("parsing stopped here"),
                );
                None
            }
        }
    }

    fn items(
        &mut self,
        kind: StringKind,
        base: &StringPath,
        field: &str,
        prefix: &str,
        items: &[ExpressionItem],
        postprocessed: bool,
    ) -> Option<Vec<ParsedItem>> {
        let mut out = Vec::new();
        let mut ok = true;
        for (i, item) in items.iter().enumerate() {
            let p = |s: &str| StringPath {
                field: format!("{field}[{i}].{s}"),
                ..base.clone()
            };
            let w = self.parse_string(StringKind::Where, p("where"), &item.where_, postprocessed);
            let e = self.parse_string(kind, p("expression"), &item.expression, postprocessed);
            match (w, e) {
                (Some(where_), Some(expression)) => out.push(ParsedItem {
                    name: format!("{prefix}:{i}"),
                    where_,
                    expression,
                }),
                _ => ok = false,
            }
        }
        ok.then_some(out)
    }
}

/// Parses every math string of the math and expands the equations. Components with parse
/// errors are left out; the errors are in the returned diagnostics.
pub fn parse_math(
    math: &CalliopeBuildMath,
    mode: ParseMode,
) -> (Vec<ParsedComponent>, Vec<MathDiagnostic>) {
    let names = match mode {
        ParseMode::Build => NameContext::from_math(math),
        ParseMode::PreValidate => NameContext::all_from_math(math),
    };
    let mut p = Parser {
        names: &names,
        diagnostics: Vec::new(),
    };
    let all = mode == ParseMode::PreValidate;
    let mut out = Vec::new();

    for (name, v) in &math.variables {
        if v.active || all {
            out.extend(component(
                &mut p,
                Group::Variables,
                name.as_str(),
                &v.foreach
                    .iter()
                    .map(|d| d.as_str().to_owned())
                    .collect::<Vec<_>>(),
                &v.where_,
                None,
            ));
        }
    }
    let mut globals: Vec<_> = math.global_expressions.iter().collect();
    globals.sort_by_key(|(_, g)| g.order);
    for (name, g) in globals {
        if g.active || all {
            out.extend(component(
                &mut p,
                Group::GlobalExpressions,
                name.as_str(),
                &g.foreach
                    .iter()
                    .map(|d| d.as_str().to_owned())
                    .collect::<Vec<_>>(),
                &g.where_,
                Some((&g.equations, &g.sub_expressions, &g.slices)),
            ));
        }
    }
    for (name, c) in &math.constraints {
        if c.active || all {
            out.extend(component(
                &mut p,
                Group::Constraints,
                name.as_str(),
                &c.foreach
                    .iter()
                    .map(|d| d.as_str().to_owned())
                    .collect::<Vec<_>>(),
                &c.where_,
                Some((&c.equations, &c.sub_expressions, &c.slices)),
            ));
        }
    }
    for (name, c) in &math.piecewise_constraints {
        if !(c.active || all) {
            continue;
        }
        let foreach: Vec<String> = c.foreach.iter().map(|d| d.as_str().to_owned()).collect();
        let Some(mut comp) = component(
            &mut p,
            Group::PiecewiseConstraints,
            name.as_str(),
            &foreach,
            &c.where_,
            None,
        ) else {
            continue;
        };
        let path = |f: &str| StringPath {
            group: "piecewise_constraints".into(),
            component: name.to_string(),
            field: f.into(),
        };
        let x = p.parse_string(
            StringKind::Arithmetic,
            path("x_expression"),
            &c.x_expression,
            false,
        );
        let y = p.parse_string(
            StringKind::Arithmetic,
            path("y_expression"),
            &c.y_expression,
            false,
        );
        if let (Some(x_expression), Some(y_expression)) = (x, y) {
            comp.piecewise = Some(PiecewiseAxes {
                x_expression,
                x_values: c.x_values.clone(),
                y_expression,
                y_values: c.y_values.clone(),
            });
            out.push(comp);
        }
    }
    for (name, o) in &math.objectives {
        if o.active || all {
            out.extend(component(
                &mut p,
                Group::Objectives,
                name.as_str(),
                &[],
                "True",
                Some((&o.equations, &o.sub_expressions, &o.slices)),
            ));
        }
    }
    if mode == ParseMode::Build {
        let mut post: Vec<_> = math.postprocessed.iter().collect();
        post.sort_by_key(|(_, g)| g.order);
        for (name, g) in post {
            if g.active {
                out.extend(component(
                    &mut p,
                    Group::Postprocessed,
                    name.as_str(),
                    &g.foreach
                        .iter()
                        .map(|d| d.as_str().to_owned())
                        .collect::<Vec<_>>(),
                    &g.where_,
                    Some((&g.equations, &g.sub_expressions, &g.slices)),
                ));
            }
        }
        for (name, c) in &math.checks {
            if c.active {
                out.extend(component(
                    &mut p,
                    Group::Checks,
                    name.as_str(),
                    &[],
                    &c.where_,
                    None,
                ));
            }
        }
    }
    (out, p.diagnostics)
}

type EquationDefs<'a> = (
    &'a Vec<ExpressionItem>,
    &'a calliope_schema::SubExpressions,
    &'a calliope_schema::SubExpressions,
);

fn component(
    p: &mut Parser<'_>,
    group: Group,
    name: &str,
    foreach: &[String],
    where_: &str,
    equations: Option<EquationDefs<'_>>,
) -> Option<ParsedComponent> {
    let pp = group == Group::Postprocessed;
    let base = StringPath {
        group: group.as_str().to_owned(),
        component: name.to_owned(),
        field: String::new(),
    };
    let top_where = p.parse_string(
        StringKind::Where,
        StringPath {
            field: "where".into(),
            ..base.clone()
        },
        where_,
        pp,
    );
    let mut comp = ParsedComponent {
        group,
        name: name.to_owned(),
        foreach: foreach.to_vec(),
        where_: top_where?,
        equation_items: Vec::new(),
        sub_expression_items: IndexMap::new(),
        slice_items: IndexMap::new(),
        equations: Vec::new(),
        piecewise: None,
    };
    let Some((eqs, subs, slices)) = equations else {
        return Some(comp);
    };
    let prefix = format!("{}:{name}", group.as_str());
    let mut ok = true;
    match p.items(group.equation_kind(), &base, "equations", &prefix, eqs, pp) {
        Some(items) => comp.equation_items = items,
        None => ok = false,
    }
    let sub_kind = StringKind::SubExpression;
    for (sub_name, items) in subs {
        let field = format!("sub_expressions.{sub_name}");
        match p.items(sub_kind, &base, &field, sub_name.as_str(), items, pp) {
            Some(items) => {
                comp.sub_expression_items
                    .insert(sub_name.to_string(), items);
            }
            None => ok = false,
        }
    }
    for (slice_name, items) in slices {
        let field = format!("slices.{slice_name}");
        match p.items(
            StringKind::Slice,
            &base,
            &field,
            slice_name.as_str(),
            items,
            pp,
        ) {
            Some(items) => {
                comp.slice_items.insert(slice_name.to_string(), items);
            }
            None => ok = false,
        }
    }
    if !ok {
        return None;
    }
    match expand(&comp) {
        Ok(eqs) => comp.equations = eqs,
        Err(d) => {
            p.diagnostics.push(*d);
            return None;
        }
    }
    Some(comp)
}

/// Expands a component's equations with its sub-expressions and slices
/// (`ParsedBackendComponent.parse_equations`).
///
/// The product runs over the referenced names in order of first appearance. Calliope
/// iterates a Python `set` of names here, so its order (and the order of the name parts)
/// can differ between runs when an equation references more than one sub-expression or
/// slice; the set of equations is the same.
pub fn expand(comp: &ParsedComponent) -> Result<Vec<Equation>, Box<MathDiagnostic>> {
    let mut out = Vec::new();
    for item in &comp.equation_items {
        let eq = Equation {
            name: item.name.clone(),
            expression: item.expression.clone(),
            wheres: vec![item.where_.clone()],
            sub_expressions: IndexMap::new(),
            slices: IndexMap::new(),
        };
        let refs = item.expression.node.sub_expression_refs();
        for with_subs in product(
            comp,
            eq,
            &refs,
            &comp.sub_expression_items,
            "sub_expressions",
        )? {
            let mut slice_refs = with_subs.expression.node.slice_refs();
            for s in with_subs.sub_expressions.values() {
                for r in s.node.slice_refs() {
                    if !slice_refs.contains(&r) {
                        slice_refs.push(r);
                    }
                }
            }
            out.extend(product(
                comp,
                with_subs,
                &slice_refs,
                &comp.slice_items,
                "slices",
            )?);
        }
    }
    Ok(out)
}

fn product(
    comp: &ParsedComponent,
    eq: Equation,
    refs: &[String],
    items: &IndexMap<String, Vec<ParsedItem>>,
    group: &str,
) -> Result<Vec<Equation>, Box<MathDiagnostic>> {
    if refs.is_empty() {
        return Ok(vec![eq]);
    }
    let missing: Vec<&String> = refs.iter().filter(|r| !items.contains_key(*r)).collect();
    if let Some(first) = missing.first() {
        let span = find_ref_span(&eq, first).unwrap_or(0..0);
        let list: Vec<String> = missing.iter().map(|m| format!("'{m}'")).collect();
        return Err(Box::new(MathDiagnostic::error(
            "math::undefined-reference",
            eq.expression.path.clone(),
            &eq.expression.source,
            span,
            format!(
                "{}:{}: Undefined {group} found in equation: {{{}}}",
                comp.group.as_str(),
                comp.name,
                list.join(", ")
            ),
        )));
    }
    let mut combos: Vec<Vec<&ParsedItem>> = vec![Vec::new()];
    for r in refs {
        combos = combos
            .into_iter()
            .flat_map(|c| {
                items[r].iter().map(move |it| {
                    let mut c = c.clone();
                    c.push(it);
                    c
                })
            })
            .collect();
    }
    Ok(combos
        .into_iter()
        .map(|combo| {
            let mut e = eq.clone();
            let parts: Vec<&str> = combo.iter().map(|it| it.name.as_str()).collect();
            e.name = format!("{}-{}", eq.name, parts.join("-"));
            for it in combo {
                e.wheres.push(it.where_.clone());
                let key = it.name.split(':').next().unwrap_or("").to_owned();
                let target = if group == "slices" {
                    &mut e.slices
                } else {
                    &mut e.sub_expressions
                };
                target.insert(key, it.expression.clone());
            }
            e
        })
        .collect())
}

fn find_ref_span(eq: &Equation, name: &str) -> Option<crate::ast::Span> {
    let mut found = None;
    eq.expression.node.walk(&mut |n| match &n.kind {
        NodeKind::SubExpr(s) | NodeKind::SliceRef(s) if s == name && found.is_none() => {
            found = Some(n.span.clone());
        }
        _ => {}
    });
    found
}

/// Which `$` references Calliope's `_find_items_in_expression` finds.
///
/// It only descends into arithmetic chains and helper calls (and sliced components, for
/// slices), so a reference under a unary sign, e.g. `-$x` or `-y[techs=$t]`, is not found
/// and the equation is not expanded with it; evaluating it then fails with a `KeyError`.
/// Compare with [`Node::sub_expression_refs`] / [`Node::slice_refs`] to detect this.
pub fn calliope_visible_refs(
    expression: &Node,
    sub_expressions: &[&Node],
    slices: bool,
) -> Vec<String> {
    fn element(n: &Node, slices: bool, out: &mut Vec<String>) {
        match &n.kind {
            NodeKind::SubExpr(s) if !slices => out.push(s.clone()),
            NodeKind::SliceRef(s) if slices => out.push(s.clone()),
            NodeKind::Arith(..) | NodeKind::Call { .. } => children(n, slices, out),
            NodeKind::Sliced { .. } if slices => children(n, slices, out),
            _ => {}
        }
    }
    fn children(n: &Node, slices: bool, out: &mut Vec<String>) {
        match &n.kind {
            NodeKind::Arith(first, rest) => {
                element(first, slices, out);
                rest.iter().for_each(|(_, x)| element(x, slices, out));
            }
            NodeKind::Call { args, kwargs, .. } => {
                args.iter().for_each(|x| element(x, slices, out));
                kwargs.iter().for_each(|(_, x)| element(x, slices, out));
            }
            NodeKind::Sliced { slices: s, .. } => {
                s.iter().for_each(|(_, x)| element(x, slices, out))
            }
            NodeKind::Compare { lhs, rhs, .. } => {
                element(lhs, slices, out);
                element(rhs, slices, out);
            }
            NodeKind::Sign(_, x) => element(x, slices, out),
            _ => {}
        }
    }
    let mut out = Vec::new();
    // The top-level element is searched through its tokens, whatever its type.
    if !slices && matches!(expression.kind, NodeKind::SubExpr(_)) {
        element(expression, slices, &mut out);
    } else {
        children(expression, slices, &mut out);
    }
    if slices {
        for s in sub_expressions {
            element(s, slices, &mut out);
        }
    }
    let mut unique = Vec::new();
    for r in out {
        if !unique.contains(&r) {
            unique.push(r);
        }
    }
    unique
}
