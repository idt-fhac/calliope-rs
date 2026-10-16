//! What each math component refers to: the dependency index behind `update_input`.
//!
//! Calliope records the names an expression touched while it is evaluated (`references`)
//! and, when an input changes, rebuilds every component that refers to it and, in turn,
//! everything that refers to those (`_find_all_references`, `_rebuild_references`). Here
//! the references are read off the lowered IR once, so the set of affected components is
//! known before anything is evaluated.

use std::collections::BTreeSet;

use calliope_schema::{BoundValue, CalliopeBuildMath};
use indexmap::IndexMap;

use crate::component::{Equation, Group, ParseMode, ParsedComponent, parse_math};
use crate::ir::{Body, Ir, lower};

/// The names one component refers to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComponentRefs {
    /// Inputs (parameters and lookups), including `active`, which every component's mask
    /// reads.
    pub inputs: BTreeSet<String>,
    /// Variables, global expressions and postprocessed arrays.
    pub results: BTreeSet<String>,
}

/// The references of every active component, by `(group, name)`, in build order.
#[derive(Debug, Clone, Default)]
pub struct Dependencies {
    /// Per component.
    pub components: IndexMap<(Group, String), ComponentRefs>,
}

fn walk(ir: &Ir, refs: &mut ComponentRefs) {
    match ir {
        Ir::Input { name, .. } | Ir::InputPresence(name) => {
            refs.inputs.insert(name.clone());
        }
        Ir::Result(name) | Ir::ResultPresence(name) => {
            refs.results.insert(name.clone());
        }
        Ir::List(items) => items.iter().for_each(|i| walk(i, refs)),
        Ir::Select { array, slicer, .. } => {
            walk(array, refs);
            walk(slicer, refs);
        }
        Ir::Mask(x) | Ir::Neg(x) | Ir::Not(x) => walk(x, refs),
        Ir::Arith(_, a, b) | Ir::Compare(_, a, b) | Ir::And(a, b) | Ir::Or(a, b) => {
            walk(a, refs);
            walk(b, refs);
        }
        Ir::Isin { items, .. } => items.iter().for_each(|i| walk(i, refs)),
        Ir::Helper {
            params, dim_kwargs, ..
        } => {
            params.iter().for_each(|(_, p)| walk(p, refs));
            dim_kwargs.iter().for_each(|(_, p)| walk(p, refs));
        }
        Ir::Const(_) | Ir::Bool(_) | Ir::Str(_) | Ir::Config(_) | Ir::Dim(_) => {}
    }
}

fn refs_of(comp: &ParsedComponent, math: &CalliopeBuildMath) -> ComponentRefs {
    let mut refs = ComponentRefs::default();
    refs.inputs.insert("active".to_owned());
    let low = lower(comp, math);
    walk(&low.mask.where_, &mut refs);
    for eq in &low.equations {
        eq.wheres.iter().for_each(|w| walk(w, &mut refs));
        match &eq.body {
            Body::Expression(e) => walk(e, &mut refs),
            Body::Constraint { lhs, rhs, .. } => {
                walk(lhs, &mut refs);
                walk(rhs, &mut refs);
            }
        }
    }
    match comp.group {
        Group::Variables => {
            let def = &math.variables[comp.name.as_str()];
            for b in [&def.bounds.min, &def.bounds.max] {
                if let BoundValue::Param(p) = b {
                    refs.inputs.insert(p.to_string());
                }
            }
        }
        Group::PiecewiseConstraints => {
            if let Some(axes) = &comp.piecewise {
                refs.inputs.insert(axes.x_values.clone());
                refs.inputs.insert(axes.y_values.clone());
                for expr in [&axes.x_expression, &axes.y_expression] {
                    let mut c = comp.clone();
                    c.equations = vec![Equation {
                        name: String::new(),
                        expression: expr.clone(),
                        wheres: Vec::new(),
                        sub_expressions: IndexMap::new(),
                        slices: IndexMap::new(),
                    }];
                    if let Some(Body::Expression(e)) =
                        lower(&c, math).equations.first().map(|e| &e.body)
                    {
                        walk(e, &mut refs);
                    }
                }
            }
        }
        _ => {}
    }
    refs
}

impl Dependencies {
    /// Reads the references of every active component of `math`.
    pub fn new(math: &CalliopeBuildMath) -> Self {
        let (comps, _) = parse_math(math, ParseMode::Build);
        let components = comps
            .iter()
            .filter(|c| c.group != Group::Checks)
            .map(|c| ((c.group, c.name.clone()), refs_of(c, math)))
            .collect();
        Self { components }
    }

    /// Every component that depends on input `name`, directly or through the variables,
    /// global expressions and postprocessed arrays that do (`_find_all_references`), in
    /// build order.
    pub fn affected_by_input(&self, name: &str) -> Vec<(Group, String)> {
        let mut hit: BTreeSet<String> = BTreeSet::new();
        let mut out = Vec::new();
        // Build order is dependency order, so one pass suffices.
        for ((group, comp), refs) in &self.components {
            if refs.inputs.contains(name) || refs.results.iter().any(|r| hit.contains(r)) {
                hit.insert(comp.clone());
                out.push((*group, comp.clone()));
            }
        }
        out
    }

    /// True if anything in the optimisation problem (not postprocessing) depends on input
    /// `name`.
    pub fn problem_depends_on(&self, name: &str) -> bool {
        self.affected_by_input(name)
            .iter()
            .any(|(g, _)| *g != Group::Postprocessed)
    }
}
