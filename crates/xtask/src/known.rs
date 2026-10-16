//! Classified parity differences: fixtures whose output differs from Calliope's for a known
//! and recorded reason (docs/findings.md, bench/BUGS.md). A classified difference is
//! reported as such and does not fail `xtask parity`, but only within what its entry
//! allows; anything beyond that still fails.

/// What a classification allows to differ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Allow {
    /// Result arrays (an alternative optimal vertex); objectives and termination match.
    Arrays,
    /// Result arrays and objective values within this relative tolerance; termination
    /// matches.
    Objective(f64),
    /// The run fails like Calliope's, with a different error message.
    ErrorMessage,
}

/// One classified fixture part.
#[derive(Debug, Clone, Copy)]
pub struct Known {
    /// Fixture name.
    pub fixture: &'static str,
    /// Kind of difference: `degenerate`, `mip-gap`, `solver`, `calliope-bug`, `deviation`.
    pub class: &'static str,
    /// What may differ.
    pub allow: Allow,
    /// Why (with the record it refers to).
    pub reason: &'static str,
}

/// The classified differences.
pub const KNOWN: &[Known] = &[
    Known {
        fixture: "urban_scale",
        class: "degenerate",
        allow: Allow::Arrays,
        reason: "alternative optimum: same LP (canonical diff equal, same column/row order) and objective; highspy merges duplicate terms with a cumsum difference and orients rows by its own rules, so some coefficients differ in the last bits and some rows are negated, and HiGHS (1.15.0 vs 1.15.1) ends on another optimal vertex (docs/findings.md R1)",
    },
    Known {
        fixture: "docs_piecewise",
        class: "solver",
        allow: Allow::Arrays,
        reason: "the golden run uses Pyomo + CBC (SOS2); the port solves with HiGHS (SOS2 as binaries): same objective, another optimal vertex (docs/findings.md R2)",
    },
    Known {
        fixture: "national_scale_check_feasibility",
        class: "calliope-bug",
        allow: Allow::Objective(f64::INFINITY),
        reason: "bench/BUGS.md #14: the scenario's objective `check_feasibility` does not exist, so both solve a zero objective; any feasible point is optimal and the reported `min_cost_optimisation` depends on it (docs/findings.md R3)",
    },
    Known {
        fixture: "national_scale_spores_evolving_average",
        class: "degenerate",
        allow: Allow::Objective(1e-4),
        reason: "SPORE 1 has the same objective but another optimal vertex (flow_cap differs by 0.1%); evolving-average scores depend on the capacities, so SPORE 2's `min_spores` differs by 1.4e-5 (docs/findings.md R1)",
    },
    Known {
        fixture: "urban_scale_milp",
        class: "mip-gap",
        allow: Allow::Objective(0.05),
        reason: "MILP solved to mip_rel_gap 0.05: both incumbents are within the gap (901.51 vs 901.67) (docs/findings.md R4)",
    },
    Known {
        fixture: "national_scale_cold_fusion",
        class: "deviation",
        allow: Allow::ErrorMessage,
        reason: "bench/BUGS.md #12: rejected in preprocessing like Calliope, with the port's message (`Template cost_dim_setter is not defined`) instead of a bare KeyError",
    },
    Known {
        fixture: "national_scale_maximize_utility",
        class: "deviation",
        allow: Allow::ErrorMessage,
        reason: "bench/BUGS.md #12: rejected in preprocessing like Calliope; the port fails when merging the override (a nested key into a non-dict), Calliope later in schema validation",
    },
    Known {
        fixture: "national_scale_minimize_emissions",
        class: "deviation",
        allow: Allow::ErrorMessage,
        reason: "bench/BUGS.md #12: rejected in preprocessing like Calliope; the port fails when merging the override (a nested key into a non-dict), Calliope later in schema validation",
    },
];

/// The classification of a fixture, if any.
pub fn lookup(fixture: &str) -> Option<&'static Known> {
    KNOWN.iter().find(|k| k.fixture == fixture)
}
