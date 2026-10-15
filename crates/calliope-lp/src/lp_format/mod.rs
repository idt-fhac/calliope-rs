//! CPLEX LP reader, canonicaliser and differ for Pyomo and HiGHS flavoured files.
//!
//! Pyomo writes names (`component(name)(i__j)`); HiGHS writes anonymous `c<N>`/`r<N>` names,
//! so a Pyomo-vs-HiGHS comparison is structural (colour refinement) rather than by name.

mod canon;
mod diff;
mod parse;

pub use canon::{CRow, CSos, CVar, CanonOptions, Canonical, Kind, ONE_VAR_CONSTANT, canonicalise};
pub use diff::{DiffOptions, LpDiff, Mode, close, diff_canonical, diff_lp, fold_singleton_rows};
pub use parse::{LpError, RawLp, RawRow, RawSos, Sense, parse_lp};

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(a: &str, b: &str) -> LpDiff {
        diff_lp(a, b, &DiffOptions::default()).unwrap()
    }

    const PYOMO: &str = "\\* Source Pyomo model name=None *\\\n\nmin\nobj:\n+2.0 ONE_VAR_CONSTANT\n+3 variables(x)(b__a)\n\ns.t.\n\nc_l_constraints(r)(b__a)_:\n+1 variables(x)(b__a)\n+1 variables(y)(a)\n>= 5.0\n\nc_e_constraints(e)(a)_:\n+1 variables(y)(a)\n= 1.0\n\nbounds\n   1 <= ONE_VAR_CONSTANT <= 1\n   0.0 <= variables(x)(b__a) <= 10\n   -inf <= variables(y)(a) <= +inf\ngeneral\n  variables(x)(b__a)\nend\n";

    #[test]
    fn parses_pyomo_and_sorts_index() {
        let c = canonicalise(&parse_lp(PYOMO).unwrap(), CanonOptions::default());
        assert_eq!(c.offset, 2.0);
        assert_eq!(c.vars[0].name, "variables(x)(a__b)");
        assert_eq!(c.vars[0].kind, Kind::Integer);
        assert_eq!(c.vars[1].lb, f64::NEG_INFINITY);
        assert_eq!(c.rows[0].name, "constraints(e)(a)");
        assert_eq!((c.rows[1].lo, c.rows[1].hi), (5.0, f64::INFINITY));
    }

    #[test]
    fn row_order_term_order_and_noise_do_not_matter() {
        let other = PYOMO
            .replace(
                "+1 variables(x)(b__a)\n+1 variables(y)(a)\n>= 5.0",
                "+1 variables(y)(a)\n+1.0000000000001 variables(x)(a__b)\n>= 5.0",
            )
            .replace("variables(x)(b__a)", "variables(x)(a__b)");
        assert!(diff(PYOMO, &other).is_equal());
    }

    #[test]
    fn detects_perturbations() {
        assert!(!diff(PYOMO, &PYOMO.replace(">= 5.0", "<= 5.0")).is_equal());
        assert!(!diff(PYOMO, &PYOMO.replace(">= 5.0", ">= 5.001")).is_equal());
        assert!(!diff(PYOMO, &PYOMO.replace("(r)(b__a)", "(r)(c__a)")).is_equal());
        assert!(!diff(PYOMO, &PYOMO.replace("+3 variables", "+3.1 variables")).is_equal());
        assert!(!diff(PYOMO, &PYOMO.replace("+2.0 ONE", "+2.5 ONE")).is_equal());
        assert!(!diff(PYOMO, &PYOMO.replace("<= 10", "<= 11")).is_equal());
        assert!(!diff(PYOMO, &PYOMO.replace("min\n", "max\n")).is_equal());
    }

    #[test]
    fn highs_flavour_is_anonymous_and_matches_by_structure() {
        let highs = "\\ File written by HiGHS .lp file handler\nmin\n obj: +3 c7 +2\nst\n r0: +1 c3 +1 c7 >= +5\n r1: +1 c3 = +1\nbounds\n c7 <= 10\n c3 free\ngen\n c7\nend\n";
        let r = diff(PYOMO, highs);
        assert_eq!(r.mode, Mode::Structural);
        assert!(r.is_equal(), "{:?}", r.differences);
        // flipped orientation is the same row
        let flipped = highs.replace("r0: +1 c3 +1 c7 >= +5", "r0: -1 c3 -1 c7 <= -5");
        assert!(diff(PYOMO, &flipped).is_equal());
        assert!(!diff(PYOMO, &highs.replace(">= +5", ">= +6")).is_equal());
        assert!(!diff(PYOMO, &highs.replace("+3 c7", "+4 c7")).is_equal());
    }

    #[test]
    fn sos_and_ranges() {
        let lp = "min\nobj:\n+1 x\ns.t.\nr:\n1 <= +1 x + 1 y <= 4\nbounds\n 0 <= x <= 3\n 0 <= y <= 3\nSOS\ns_s: S2::\n  x:1\n  y:2\nend\n";
        let c = canonicalise(&parse_lp(lp).unwrap(), CanonOptions::default());
        assert_eq!((c.rows[0].lo, c.rows[0].hi), (1.0, 4.0));
        assert_eq!(c.sos.len(), 1);
        assert!(diff(lp, lp).is_equal());
        assert!(!diff(lp, &lp.replace("y:2", "y:3")).is_equal());
    }
}
