//! Algebraic properties of `LArray` operations (no oracle needed).

use calliope_array::{Coord, Join, LArray, Label};
use proptest::prelude::*;

const HOUR: i64 = 3_600_000_000_000;

fn coord(name: &'static str, n: usize) -> Coord {
    let labels = match name {
        "t" => (0..n as i64).map(|h| Label::Datetime(h * HOUR)).collect(),
        "b" => (0..n as i64).map(Label::Int).collect(),
        _ => (0..n).map(|i| Label::str(&format!("l{i}"))).collect(),
    };
    Coord::new(name, labels).unwrap()
}

prop_compose! {
    fn arrays()(dims in proptest::sample::subsequence(vec!["a", "b", "t"], 0..=3).prop_shuffle(),
                sizes in proptest::collection::vec(1usize..5, 3))
               (values in proptest::collection::vec(
                    prop_oneof![1 => Just(f64::NAN), 3 => -5.0f64..5.0],
                    dims.iter().zip(&sizes).map(|(_, s)| *s).product::<usize>()),
                dims in Just(dims), sizes in Just(sizes)) -> LArray<f64> {
        let coords = dims.iter().zip(&sizes).map(|(d, s)| coord(d, *s)).collect();
        LArray::from_dense(coords, values).unwrap()
    }
}

fn same(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| (x.is_nan() && y.is_nan()) || (x - y).abs() < 1e-9)
}

proptest! {
    #[test]
    fn dense_roundtrip(a in arrays()) {
        let back = LArray::from_dense(a.dims().to_vec(), a.to_dense()).unwrap();
        prop_assert_eq!(&back, &a);
    }

    #[test]
    fn transpose_roundtrip(a in arrays()) {
        let names: Vec<&str> = a.dim_names();
        let mut rev = names.clone();
        rev.reverse();
        let t = a.transpose(&rev).unwrap().transpose(&names).unwrap();
        prop_assert_eq!(&t, &a);
    }

    #[test]
    fn roll_inverse(a in arrays(), shift in -7i64..7) {
        for d in a.dim_names() {
            let r = a.roll(d, shift).unwrap().roll(d, -shift).unwrap();
            prop_assert_eq!(&r, &a);
        }
    }

    #[test]
    fn total_sum(a in arrays()) {
        let all = a.dim_names();
        let s = a.sum(&all, 1).unwrap().to_dense();
        let vals: Vec<f64> = a.to_dense().into_iter().filter(|v| !v.is_nan()).collect();
        let expected = if vals.is_empty() { f64::NAN } else { vals.iter().sum() };
        prop_assert!(same(&s, &[expected]));
        let s0 = a.sum(&all, 0).unwrap().to_dense();
        let expected0 = if vals.is_empty() { 0.0 } else { expected };
        prop_assert!(same(&s0, &[expected0]));
    }

    #[test]
    fn self_addition_doubles(a in arrays()) {
        let twice = a.zip_with(&a, Join::Inner, |x, y| x + y);
        let expected: Vec<f64> = a.to_dense().iter().map(|v| 2.0 * v).collect();
        prop_assert!(same(&twice.to_dense(), &expected));
    }

    #[test]
    fn fillna_leaves_nothing_missing(a in arrays()) {
        prop_assert!(a.fillna(1.5).to_dense().iter().all(|v| !v.is_nan()));
    }

    #[test]
    fn where_true_is_identity_and_false_is_empty(a in arrays()) {
        let n = a.to_dense().len();
        let t = LArray::from_dense(a.dims().to_vec(), vec![true; n]).unwrap();
        prop_assert_eq!(&a.where_(&t), &a);
        let f = LArray::from_dense(a.dims().to_vec(), vec![false; n]).unwrap();
        prop_assert_eq!(a.where_(&f).count(), 0);
    }

    #[test]
    fn concat_then_select(a in arrays(), b in arrays()) {
        if a.dim_names() == b.dim_names() && a.dims() == b.dims() {
            let new = Coord::strings("new", &["p", "q"]).unwrap();
            let c = LArray::concat(&[a.clone(), b.clone()], new).unwrap();
            prop_assert_eq!(&c.sel_one("new", &Label::str("p")).unwrap(), &a);
            prop_assert_eq!(&c.sel_one("new", &Label::str("q")).unwrap(), &b);
        }
    }
}

#[test]
fn time_axis_is_dense_and_order_follows_dims() {
    let t = coord("t", 3);
    let a = coord("a", 2);
    let x = LArray::from_dense(vec![t, a], vec![1.0, 2.0, f64::NAN, 4.0, 5.0, 6.0]).unwrap();
    assert_eq!(x.dense_dim(), Some("t"));
    assert_eq!(x.n_keys(), 2);
    assert!(same(&x.to_dense(), &[1.0, 2.0, f64::NAN, 4.0, 5.0, 6.0]));
}
