//! Plain loops, as the correctness reference for both candidates.

use std::collections::{BTreeSet, HashMap};

use crate::{Csc, Model};

pub fn build(m: &Model) -> Csc {
    let h = m.hours;
    let eff: HashMap<(usize, usize), f64> = m.eff.iter().map(|&(n, t, v)| ((n, t), v)).collect();
    let ts: HashMap<(usize, usize), &Vec<f64>> =
        m.ts.iter().map(|(n, t, v)| ((*n, *t), v)).collect();
    let row_nc: BTreeSet<(usize, usize)> = m
        .carrier_in
        .iter()
        .chain(&m.carrier_out)
        .map(|&(n, _, c)| (n, c))
        .collect();
    let row_of: HashMap<(usize, usize), usize> =
        row_nc.iter().enumerate().map(|(i, &k)| (k, i)).collect();
    let mut triples = Vec::new();
    let mut col = 0u32;
    for &(n, t, c) in &m.carrier_out {
        let e = eff.get(&(n, t)).copied().unwrap_or(f64::NAN);
        let e = if e.is_nan() { 1.0 } else { e };
        for step in 0..h {
            let f = ts.get(&(n, t)).map_or(f64::NAN, |v| v[step]);
            let f = if f.is_nan() { 1.0 } else { f };
            let row = (row_of[&(n, c)] * h + step) as u32;
            triples.push((row, col, f / e));
            col += 1;
        }
    }
    for &(n, _t, c) in &m.carrier_in {
        for step in 0..h {
            let row = (row_of[&(n, c)] * h + step) as u32;
            triples.push((row, col, -1.0));
            col += 1;
        }
    }
    Csc::from_triples(row_nc.len() * h, col as usize, triples)
}
