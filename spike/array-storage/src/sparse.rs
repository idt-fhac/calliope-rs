//! Candidate C (hybrid): sparse over the non-time dimensions, dense over time.
//!
//! An array is a sorted list of coordinate tuples over its non-time dims (only defined
//! entries), each with either one value or a dense vector over `timesteps`. Masks and affine
//! expressions use the same layout, so a `[nodes, techs, carriers, timesteps]` variable costs
//! `active (node, tech, carrier) × T`, never the full product.

use std::collections::HashMap;
use std::time::Instant;

use crate::{Csc, Model};

type Dim = &'static str;

#[derive(Clone, Debug)]
pub struct SArray {
    /// Non-time dimensions, in order.
    pub dims: Vec<Dim>,
    /// Length of the time axis, if the array has one.
    pub time: Option<usize>,
    /// Coordinate tuples (sorted, unique).
    pub keys: Vec<Vec<u32>>,
    /// `keys.len() × time.unwrap_or(1)` values; NaN = missing.
    pub values: Vec<f64>,
}

impl SArray {
    fn width(&self) -> usize {
        self.time.unwrap_or(1)
    }

    fn index(&self) -> HashMap<&[u32], usize> {
        self.keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.as_slice(), i))
            .collect()
    }

    /// Positions of `self.dims` within `dims` (all must be present).
    fn project(&self, dims: &[Dim]) -> Vec<usize> {
        self.dims
            .iter()
            .map(|d| dims.iter().position(|x| x == d).expect("dim present"))
            .collect()
    }

    /// `sum(dim, min_count=1)` over a non-time dimension.
    pub fn sum(&self, dim: Dim) -> SArray {
        let pos = self
            .dims
            .iter()
            .position(|d| *d == dim)
            .expect("dim present");
        let mut out = self.clone();
        out.dims.remove(pos);
        for k in &mut out.keys {
            k.remove(pos);
        }
        out.aggregate()
    }

    /// Merges entries with equal keys by NaN-skipping summation (all-NaN gives NaN).
    fn aggregate(self) -> SArray {
        let w = self.width();
        let mut acc: std::collections::BTreeMap<Vec<u32>, Vec<(f64, u32)>> = Default::default();
        for (i, k) in self.keys.into_iter().enumerate() {
            let slot = acc.entry(k).or_insert_with(|| vec![(0.0, 0); w]);
            for (s, &v) in slot.iter_mut().zip(&self.values[i * w..(i + 1) * w]) {
                if !v.is_nan() {
                    s.0 += v;
                    s.1 += 1;
                }
            }
        }
        let mut out = SArray {
            dims: self.dims,
            time: self.time,
            keys: vec![],
            values: vec![],
        };
        for (k, slot) in acc {
            out.keys.push(k);
            out.values
                .extend(slot.iter().map(|&(s, n)| if n == 0 { f64::NAN } else { s }));
        }
        out
    }

    /// `where(mask)` with a non-time boolean mask whose dims are a subset of ours.
    pub fn where_(&self, mask: &SArray) -> SArray {
        let proj = mask.project(&self.dims);
        let idx = mask.index();
        let w = self.width();
        let mut out = SArray {
            dims: self.dims.clone(),
            time: self.time,
            keys: vec![],
            values: vec![],
        };
        for (i, k) in self.keys.iter().enumerate() {
            let mk: Vec<u32> = proj.iter().map(|&p| k[p]).collect();
            if idx
                .get(mk.as_slice())
                .is_some_and(|&j| mask.values[j] != 0.0)
            {
                out.keys.push(k.clone());
                out.values
                    .extend_from_slice(&self.values[i * w..(i + 1) * w]);
            }
        }
        out
    }

    /// `roll(timesteps=shift)`.
    pub fn roll_time(&self, shift: isize) -> SArray {
        let w = self.width() as isize;
        let mut out = self.clone();
        for (i, _) in self.keys.iter().enumerate() {
            let src = &self.values[i * w as usize..(i + 1) * w as usize];
            for t in 0..w {
                out.values[i * w as usize + t as usize] = src[(t - shift).rem_euclid(w) as usize];
            }
        }
        out
    }

    /// `group_sum` over a non-time dimension.
    pub fn group_sum(&self, dim: Dim, groups: &[u32], new_dim: Dim) -> SArray {
        let pos = self
            .dims
            .iter()
            .position(|d| *d == dim)
            .expect("dim present");
        let mut out = self.clone();
        for k in &mut out.keys {
            k[pos] = groups[k[pos] as usize];
        }
        out.dims[pos] = new_dim;
        out.aggregate()
    }
}

/// Affine terms in struct-of-arrays form; each term points at a non-time key.
pub struct Aff {
    pub dims: Vec<Dim>,
    pub time: usize,
    pub keys: Vec<Vec<u32>>,
    pub key: Vec<u32>,
    pub t: Vec<u32>,
    pub col: Vec<u32>,
    pub coef: Vec<f64>,
}

impl Aff {
    /// A variable over a non-time mask × time, numbered in C order.
    pub fn variable(mask: &SArray, time: usize, first_col: u32) -> Aff {
        let active: Vec<Vec<u32>> = mask
            .keys
            .iter()
            .zip(&mask.values)
            .filter(|(_, m)| **m != 0.0)
            .map(|(k, _)| k.clone())
            .collect();
        let n = active.len() * time;
        let key = (0..active.len() as u32)
            .flat_map(|k| std::iter::repeat_n(k, time))
            .collect();
        let t = (0..active.len()).flat_map(|_| 0..time as u32).collect();
        Aff {
            dims: mask.dims.clone(),
            time,
            keys: active,
            key,
            t,
            col: (first_col..first_col + n as u32).collect(),
            coef: vec![1.0; n],
        }
    }

    /// Multiplies by `fillna(param, default)`, looked up once per non-time key.
    pub fn mul_param(mut self, p: &SArray, default: f64) -> Aff {
        let proj = p.project(&self.dims);
        let idx = p.index();
        let rows: Vec<Option<usize>> = self
            .keys
            .iter()
            .map(|k| {
                let pk: Vec<u32> = proj.iter().map(|&i| k[i]).collect();
                idx.get(pk.as_slice()).copied()
            })
            .collect();
        let w = p.width();
        for ((k, t), coef) in self.key.iter().zip(&self.t).zip(self.coef.iter_mut()) {
            let v = rows[*k as usize].map_or(f64::NAN, |r| {
                p.values[r * w + if w > 1 { *t as usize } else { 0 }]
            });
            *coef *= if v.is_nan() { default } else { v };
        }
        self
    }

    pub fn div_param(self, p: &SArray, default: f64) -> Aff {
        let inv = SArray {
            values: p.values.iter().map(|v| 1.0 / v).collect(),
            ..p.clone()
        };
        self.mul_param(&inv, 1.0 / default)
    }

    /// `sum(over=dim)`: drops the dim from the keys and merges equal keys.
    pub fn sum(mut self, dim: Dim) -> Aff {
        let pos = self
            .dims
            .iter()
            .position(|d| *d == dim)
            .expect("dim present");
        let mut new_keys: Vec<Vec<u32>> = Vec::new();
        let mut seen: HashMap<Vec<u32>, u32> = HashMap::new();
        let remap: Vec<u32> = self
            .keys
            .iter()
            .map(|k| {
                let mut nk = k.clone();
                nk.remove(pos);
                *seen.entry(nk.clone()).or_insert_with(|| {
                    new_keys.push(nk);
                    new_keys.len() as u32 - 1
                })
            })
            .collect();
        for k in &mut self.key {
            *k = remap[*k as usize];
        }
        self.keys = new_keys;
        self.dims.remove(pos);
        self
    }

    pub fn sub(mut self, other: Aff) -> Aff {
        assert_eq!(self.dims, other.dims);
        let mut idx: HashMap<Vec<u32>, u32> = self
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), i as u32))
            .collect();
        let remap: Vec<u32> = other
            .keys
            .into_iter()
            .map(|k| {
                let n = self.keys.len() as u32;
                *idx.entry(k.clone()).or_insert_with(|| {
                    self.keys.push(k);
                    n
                })
            })
            .collect();
        self.key
            .extend(other.key.into_iter().map(|k| remap[k as usize]));
        self.t.extend(other.t);
        self.col.extend(other.col);
        self.coef.extend(other.coef.into_iter().map(|c| -c));
        self
    }

    pub fn to_csc(&self, rows: &SArray, n_cols: usize) -> Csc {
        let idx = rows.index();
        let mut row_of = vec![u32::MAX; rows.keys.len()];
        let mut n = 0;
        for (i, &m) in rows.values.iter().enumerate() {
            if m != 0.0 {
                row_of[i] = n;
                n += 1;
            }
        }
        let key_row: Vec<u32> = self
            .keys
            .iter()
            .map(|k| idx.get(k.as_slice()).map_or(u32::MAX, |&i| row_of[i]))
            .collect();
        let triples = (0..self.key.len())
            .filter_map(|i| {
                let r = key_row[self.key[i] as usize];
                (r != u32::MAX).then_some((
                    r * self.time as u32 + self.t[i],
                    self.col[i],
                    self.coef[i],
                ))
            })
            .collect();
        Csc::from_triples(n as usize * self.time, n_cols, triples)
    }
}

fn mask(dims: Vec<Dim>, mut keys: Vec<Vec<u32>>) -> SArray {
    keys.sort_unstable();
    keys.dedup();
    let values = vec![1.0; keys.len()];
    SArray {
        dims,
        time: None,
        keys,
        values,
    }
}

pub fn build(m: &Model) -> Csc {
    let t0 = Instant::now();
    let nt = |v: &[(usize, usize, usize)]| {
        v.iter()
            .map(|&(n, t, c)| vec![n as u32, t as u32, c as u32])
            .collect()
    };
    let carrier_out = mask(vec!["nodes", "techs", "carriers"], nt(&m.carrier_out));
    let carrier_in = mask(vec!["nodes", "techs", "carriers"], nt(&m.carrier_in));
    let mut eff_rows: Vec<(Vec<u32>, f64)> = m
        .eff
        .iter()
        .map(|&(n, t, v)| (vec![n as u32, t as u32], v))
        .collect();
    eff_rows.sort_by(|a, b| a.0.cmp(&b.0));
    let eff = SArray {
        dims: vec!["nodes", "techs"],
        time: None,
        keys: eff_rows.iter().map(|r| r.0.clone()).collect(),
        values: eff_rows.iter().map(|r| r.1).collect(),
    };
    let mut ts_rows: Vec<&(usize, usize, Vec<f64>)> = m.ts.iter().collect();
    ts_rows.sort_by_key(|r| (r.0, r.1));
    let ts = SArray {
        dims: vec!["nodes", "techs"],
        time: Some(m.hours),
        keys: ts_rows
            .iter()
            .map(|r| vec![r.0 as u32, r.1 as u32])
            .collect(),
        values: ts_rows.iter().flat_map(|r| r.2.iter().copied()).collect(),
    };
    eprintln!("sparse: inputs {:.3}s", t0.elapsed().as_secs_f64());
    let flow_out = Aff::variable(&carrier_out, m.hours, 0);
    let n_out = flow_out.col.len() as u32;
    let flow_in = Aff::variable(&carrier_in, m.hours, n_out);
    let n_cols = flow_out.col.len() + flow_in.col.len();
    let expr = flow_out
        .mul_param(&ts, 1.0)
        .div_param(&eff, 1.0)
        .sum("techs")
        .sub(flow_in.sum("techs"));
    // rows: any(carrier_in, over=techs) OR any(carrier_out, over=techs)
    let mut nc: Vec<Vec<u32>> = carrier_in
        .keys
        .iter()
        .chain(&carrier_out.keys)
        .map(|k| vec![k[0], k[2]])
        .collect();
    nc.sort_unstable();
    nc.dedup();
    let rows = mask(vec!["nodes", "carriers"], nc);
    expr.to_csc(&rows, n_cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_where_roll_group_sum() {
        let x = SArray {
            dims: vec!["a", "b"],
            time: None,
            keys: vec![vec![0, 0], vec![0, 1], vec![1, 0]],
            values: vec![1.0, 2.0, f64::NAN],
        };
        let s = x.sum("b");
        assert_eq!(s.values[0], 3.0);
        assert!(s.values[1].is_nan());
        let m = mask(vec!["b"], vec![vec![0]]);
        assert_eq!(x.where_(&m).keys.len(), 2);
        let g = x.group_sum("a", &[0, 0], "g");
        assert_eq!(g.values, vec![1.0, 2.0]);
        let t = SArray {
            dims: vec![],
            time: Some(3),
            keys: vec![vec![]],
            values: vec![0.0, 1.0, 2.0],
        };
        assert_eq!(t.roll_time(1).values, vec![2.0, 0.0, 1.0]);
    }
}
