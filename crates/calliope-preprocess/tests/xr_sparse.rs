//! Sparse `float64` data and the typed fast paths behave like the dense, generic paths.

use calliope_array::Coord;
use calliope_preprocess::xr::{Compat, DType, Data, DataArray, Dataset, Join, Obj, SparseF64};

fn coord(name: &str, labels: &[&str]) -> Coord {
    Coord::strings(name, labels).unwrap()
}

/// Values with NaN equal to NaN, and the dtype.
fn values(d: &Data) -> (DType, Vec<Obj>) {
    (d.dtype(), d.objs())
}

fn same(a: &DataArray, b: &DataArray) {
    assert_eq!(a.coords, b.coords);
    assert_eq!(values(&a.data), values(&b.data));
}

/// A mostly empty (t, n, k) array, dense and sparse.
fn pair() -> (DataArray, DataArray) {
    let coords = vec![
        coord("t", &["t0", "t1", "t2"]),
        coord("n", &["a", "b"]),
        coord("k", &["x", "y", "z"]),
    ];
    let mut v = vec![f64::NAN; 18];
    v[1] = 1.5;
    v[7] = -2.0;
    v[16] = 4.0;
    let dense = DataArray::new(coords.clone(), Data::Float(v.clone()));
    let sparse = DataArray::new(coords, Data::Sparse(SparseF64::from_dense(&v)));
    (dense, sparse)
}

#[test]
fn sparse_storage_only_for_sparse_floats() {
    let (d, _) = pair();
    assert!(matches!(
        d.data.clone().into_sparse_if_sparse(),
        Data::Sparse(_)
    ));
    let full = Data::Float(vec![1.0, 2.0, f64::NAN]);
    assert!(matches!(full.into_sparse_if_sparse(), Data::Float(_)));
    let bools = Data::Bool(vec![false; 8]);
    assert!(matches!(bools.into_sparse_if_sparse(), Data::Bool(_)));
}

#[test]
fn conform_reorders_reindexes_and_broadcasts() {
    let (d, s) = pair();
    let dst = vec![
        coord("c", &["c0", "c1"]),
        coord("k", &["z", "x", "w"]),
        coord("t", &["t2", "t1", "t0"]),
        coord("n", &["b", "a"]),
    ];
    let a = d.conform(&dst, DType::Float);
    let b = s.conform(&dst, DType::Float);
    assert!(matches!(b.data, Data::Sparse(_)));
    same(&a, &b);
    same(
        &d.reindex(&coord("n", &["a"])),
        &s.reindex(&coord("n", &["a"])),
    );
}

#[test]
fn where_matches_the_generic_path() {
    let (d, s) = pair();
    let cond = DataArray::new(
        vec![coord("k", &["x", "y", "z"]), coord("n", &["a", "b"])],
        Data::Bool(vec![true, false, false, true, true, true]),
    );
    let a = d.where_(&cond);
    let b = s.where_(&cond);
    assert!(matches!(b.data, Data::Sparse(_)));
    same(&a, &b);
    // The dense in-place path against a reference built cell by cell.
    let expect: Vec<Obj> = (0..18)
        .map(|i| {
            let (n, k) = ((i / 3) % 2, i % 3);
            let keep = [[true, true], [false, true], [false, true]][k][n];
            if keep { d.data.get(i) } else { Obj::Nan }
        })
        .collect();
    assert_eq!(a.data.objs(), expect);
    // Bool data becomes float64, as numpy does when values go missing.
    let flags = DataArray::new(vec![coord("n", &["a", "b"])], Data::Bool(vec![true, false]));
    let mask = DataArray::new(vec![coord("n", &["a", "b"])], Data::Bool(vec![false, true]));
    let w = flags.where_(&mask);
    assert_eq!(
        values(&w.data),
        (DType::Float, vec![Obj::Nan, Obj::Float(0.0)])
    );
    let all = flags.where_(&DataArray::scalar(Obj::Bool(true)));
    assert_eq!(w.dtype(), all.dtype());
}

#[test]
fn fillna_combinations_agree() {
    let (d, s) = pair();
    let other_v: Vec<f64> = (0..18)
        .map(|i| if i % 4 == 0 { f64::NAN } else { f64::from(i) })
        .collect();
    let od = DataArray::new(d.coords.clone(), Data::Float(other_v.clone()));
    let os = DataArray::new(
        d.coords.clone(),
        Data::Sparse(SparseF64::from_dense(&other_v)),
    );
    let expect = d.fillna(&od);
    for (a, b) in [(&d, &os), (&s, &od), (&s, &os)] {
        same(&expect, &a.fillna(b));
    }
    // Filling from a smaller array (left join, broadcast).
    let small = DataArray::new(vec![coord("n", &["b", "a"])], Data::Float(vec![9.0, 8.0]));
    same(&d.fillna(&small), &s.fillna(&small));
}

#[test]
fn merge_over_equals_merge_then_fillna() {
    let (d, s) = pair();
    for ts in [s, d] {
        let mut old = Dataset::new();
        old.set("ts", ts);
        old.set(
            "flag",
            DataArray::new(vec![coord("n", &["a", "b"])], Data::Bool(vec![true, false])),
        );
        old.set(
            "name",
            DataArray::new(
                vec![coord("k", &["x", "y", "z"])],
                Data::Object(vec![Obj::str("p"), Obj::Nan, Obj::str("q")]),
            ),
        );
        let mut new = Dataset::new();
        new.set(
            "ts",
            DataArray::new(
                vec![coord("n", &["c", "a"]), coord("k", &["x"])],
                Data::Float(vec![1.0, f64::NAN]),
            ),
        );
        new.set(
            "name",
            DataArray::new(
                vec![coord("k", &["w", "y"])],
                Data::Object(vec![Obj::str("r"), Obj::str("s")]),
            ),
        );
        new.set("scalar", DataArray::scalar(Obj::Float(1.0)));
        let expect = Dataset::merge(&[&new, &old], Join::Outer, Compat::Override)
            .unwrap()
            .fillna(&old);
        let mut got = old.clone();
        got.merge_over(new);
        assert_eq!(got.coords, expect.coords);
        assert_eq!(got.names(), expect.names());
        for name in expect.names() {
            same(&got.get(&name).unwrap(), &expect.get(&name).unwrap());
        }
    }
}

#[test]
fn zip_with_typed_results() {
    let a = DataArray::new(vec![coord("n", &["a", "b"])], Data::Bool(vec![true, false]));
    let b = DataArray::new(
        vec![coord("n", &["a", "b"])],
        Data::Float(vec![f64::NAN, 2.0]),
    );
    let or = a.zip_with(&b, |x, y| Obj::Bool(x.truthy() || y.truthy()), DType::Bool);
    assert_eq!(or.data, Data::Bool(vec![true, true]));
    let mul = a.zip_with(
        &b,
        |x, y| match (x.as_f64(), y.as_f64()) {
            (Some(p), Some(q)) => Obj::float(p * q),
            _ => Obj::Nan,
        },
        DType::Float,
    );
    assert_eq!(
        values(&mul.data),
        (DType::Float, vec![Obj::Nan, Obj::Float(0.0)])
    );
    // Non-numeric results keep the generic conversion (object data).
    let s = a.zip_with(&b, |_, _| Obj::str("x"), DType::Float);
    assert_eq!(s.dtype(), DType::Object);
}
