#![allow(dead_code)]
//! Phase 2 storage spike (ADR 0001): dense `ndarray` (A) vs Polars long format (B).
//!
//! Both build the same heavy constraint end to end into CSC triples, on a synthetic scale-up
//! of `national_scale` (`bench/synthetic.py`: copies of the 5-node system joined in a ring):
//!
//! ```text
//! foreach: [nodes, carriers, timesteps]
//! where:   any(carrier_in, over=techs) OR any(carrier_out, over=techs)
//! sum(flow_out * fillna(ts_factor, 1) / fillna(flow_out_eff, 1), over=techs)
//!   - sum(flow_in, over=techs) == 0
//! ```
//!
//! This is `system_balance` with the time-varying, NaN-holed coefficient of
//! `flow_out_inc_eff`-style terms, so that broadcasting a `[nodes, techs, timesteps]`
//! parameter, `fillna`, `where` and `sum(over)` are all on the hot path.
//!
//! Usage: `array-storage-spike <dense|polars|reference> <copies> <hours>`; prints JSON with
//! wall time, peak RSS and a checksum of the CSC matrix.

use std::time::Instant;

/// Synthetic inputs, with labels as integer codes.
pub struct Model {
    pub n_nodes: usize,
    pub n_techs: usize,
    pub n_carriers: usize,
    pub hours: usize,
    /// `carrier_out[nodes, techs, carriers]` (true entries).
    pub carrier_out: Vec<(usize, usize, usize)>,
    /// `carrier_in[nodes, techs, carriers]` (true entries).
    pub carrier_in: Vec<(usize, usize, usize)>,
    /// `flow_out_eff[nodes, techs]` (defined entries).
    pub eff: Vec<(usize, usize, f64)>,
    /// `ts_factor[nodes, techs, timesteps]` (defined (node, tech) pairs; NaN holes inside).
    pub ts: Vec<(usize, usize, Vec<f64>)>,
}

pub fn synthetic(copies: usize, hours: usize) -> Model {
    // Techs: 4 global ones, then 4 links per copy, then ring links between copies.
    const BATTERY: usize = 0;
    const CCGT: usize = 1;
    const CSP: usize = 2;
    const DEMAND: usize = 3;
    let n_nodes = 5 * copies;
    let node = |k: usize, i: usize| 5 * k + i; // 0 region1, 1 region2, 2..4 region1_x
    let mut links: Vec<(usize, usize)> = Vec::new();
    for k in 0..copies {
        for j in 1..5 {
            links.push((node(k, 0), node(k, j)));
        }
    }
    if copies > 1 {
        for k in 0..copies {
            links.push((node(k, 0), node((k + 1) % copies, 0)));
            if copies == 2 {
                break;
            }
        }
    }
    let n_techs = 4 + links.len();
    let mut m = Model {
        n_nodes,
        n_techs,
        n_carriers: 1,
        hours,
        carrier_out: Vec::new(),
        carrier_in: Vec::new(),
        eff: Vec::new(),
        ts: Vec::new(),
    };
    let series = |seed: usize, scale: f64| -> Vec<f64> {
        (0..hours)
            .map(|h| {
                if (h + seed) % 97 == 0 {
                    f64::NAN
                } else {
                    scale * (1.1 + ((h as f64) * 0.2618 + seed as f64).sin())
                }
            })
            .collect()
    };
    for k in 0..copies {
        let f = 0.8 + 0.4 * (((k * 7) % 11) as f64) / 10.0;
        // region1: ccgt + demand; region2: battery + demand; region1_x: csp + demand
        let r1 = node(k, 0);
        m.carrier_out.push((r1, CCGT, 0));
        m.eff.push((r1, CCGT, 0.5));
        let r2 = node(k, 1);
        m.carrier_out.push((r2, BATTERY, 0));
        m.carrier_in.push((r2, BATTERY, 0));
        m.eff.push((r2, BATTERY, 0.95));
        for j in 2..5 {
            let n = node(k, j);
            m.carrier_out.push((n, CSP, 0));
            m.ts.push((n, CSP, series(n, f)));
        }
        for j in 0..5 {
            let n = node(k, j);
            m.carrier_in.push((n, DEMAND, 0));
            m.ts.push((n, DEMAND, series(n + 1000, f)));
        }
    }
    for (i, (a, b)) in links.iter().enumerate() {
        for n in [*a, *b] {
            m.carrier_out.push((n, 4 + i, 0));
            m.carrier_in.push((n, 4 + i, 0));
            m.eff.push((n, 4 + i, 0.98));
        }
    }
    m.carrier_out.sort_unstable();
    m.carrier_in.sort_unstable();
    m
}

/// A CSC matrix: column pointers, row indices, values.
#[derive(Debug, Default, PartialEq)]
pub struct Csc {
    pub n_rows: usize,
    pub col_ptr: Vec<usize>,
    pub row_idx: Vec<u32>,
    pub values: Vec<f64>,
}

impl Csc {
    /// Builds from unsorted triples; duplicate (row, col) entries are summed.
    pub fn from_triples(n_rows: usize, n_cols: usize, mut t: Vec<(u32, u32, f64)>) -> Self {
        t.sort_unstable_by_key(|&(r, c, _)| (c, r));
        let mut csc = Csc {
            n_rows,
            col_ptr: vec![0; n_cols + 1],
            ..Default::default()
        };
        let mut last: Option<(u32, u32)> = None;
        for (r, c, v) in t {
            if last == Some((r, c)) {
                *csc.values.last_mut().expect("entry") += v;
                continue;
            }
            last = Some((r, c));
            csc.row_idx.push(r);
            csc.values.push(v);
            csc.col_ptr[c as usize + 1] += 1;
        }
        for i in 0..n_cols {
            csc.col_ptr[i + 1] += csc.col_ptr[i];
        }
        csc
    }

    pub fn checksum(&self) -> f64 {
        let mut s = 0.0;
        for c in 0..self.col_ptr.len() - 1 {
            for k in self.col_ptr[c]..self.col_ptr[c + 1] {
                s += self.values[k] * (1.0 + (self.row_idx[k] as f64 * 31.0 + c as f64) % 1009.0);
            }
        }
        s
    }
}

mod dense;
mod long;
mod reference;
mod sparse;

fn peak_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|v| v.parse::<f64>().ok())
            })
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let variant = args.get(1).map_or("dense", String::as_str);
    let copies: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let hours: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8760);
    let m = synthetic(copies, hours);
    let base_rss = peak_rss_mb();
    let t0 = Instant::now();
    let csc = match variant {
        "dense" => dense::build(&m),
        "polars" => long::build(&m),
        "reference" => reference::build(&m),
        "sparse" => sparse::build(&m),
        other => panic!("unknown variant {other}"),
    };
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "{{\"variant\": \"{variant}\", \"nodes\": {}, \"techs\": {}, \"hours\": {hours}, \
         \"rows\": {}, \"cols\": {}, \"nnz\": {}, \"seconds\": {secs:.3}, \
         \"inputs_rss_mb\": {base_rss:.0}, \"peak_rss_mb\": {:.0}, \"checksum\": {:.6e}}}",
        m.n_nodes,
        m.n_techs,
        csc.n_rows,
        csc.col_ptr.len() - 1,
        csc.values.len(),
        peak_rss_mb(),
        csc.checksum()
    );
}
