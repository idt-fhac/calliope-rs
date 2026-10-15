//! Port of `tests/backend/helper_functions_test.py` (`TestAsArray`), on the conftest
//! `dummy_model_data` fixture. LaTeX output (`TestAsMathString`) is not ported.

use calliope_array::{Coord, LArray, Label};
use calliope_math::ast::Lang;
use calliope_math::helpers::array::{self as h, GroupDtype, HelperError, Value};
use calliope_math::helpers::{Helper, bind};

const NAN: f64 = f64::NAN;
const INF: f64 = f64::INFINITY;
const HOUR: i64 = 3_600_000_000_000;
const Y2000: i64 = 946_684_800_000_000_000;

fn nodes() -> Coord {
    Coord::strings("nodes", &["foo", "bar"]).unwrap()
}
fn techs() -> Coord {
    Coord::strings("techs", &["foobar", "foobaz", "barfoo", "bazfoo"]).unwrap()
}
fn carriers() -> Coord {
    Coord::strings("carriers", &["foo", "bar"]).unwrap()
}
fn timesteps() -> Coord {
    Coord::new(
        "timesteps",
        (0..4).map(|i| Label::Datetime(Y2000 + i * HOUR)).collect(),
    )
    .unwrap()
}

fn float(dims: Vec<Coord>, v: &[f64]) -> LArray<f64> {
    LArray::from_dense(dims, v.to_vec()).unwrap()
}
fn boolean(dims: Vec<Coord>, v: &[bool]) -> LArray<bool> {
    LArray::from_dense(dims, v.to_vec()).unwrap()
}
fn labels(dims: Vec<Coord>, v: &[Option<&str>]) -> LArray<Option<Label>> {
    LArray::from_dense(dims, v.iter().map(|s| s.map(Label::str)).collect()).unwrap()
}

fn with_inf() -> LArray<f64> {
    float(
        vec![nodes(), techs()],
        &[1.0, NAN, 1.0, 3.0, INF, 2.0, 1.0, NAN],
    )
}
fn only_techs() -> LArray<f64> {
    float(vec![techs()], &[NAN, 1.0, 2.0, 3.0])
}
fn with_inf_as_bool() -> LArray<bool> {
    boolean(
        vec![nodes(), techs()],
        &[true, false, true, true, false, true, true, false],
    )
}
fn carrier_out() -> LArray<bool> {
    boolean(
        vec![nodes(), techs(), carriers()],
        &[
            true, true, true, true, false, true, true, true, //
            true, true, true, true, true, true, true, true,
        ],
    )
}

/// Equal with NaN == NaN, same dims in any order.
fn assert_close(got: &LArray<f64>, dims: &[&str], expected: &[f64]) {
    let got = got.transpose(dims).unwrap();
    let values = got.to_dense();
    assert_eq!(values.len(), expected.len(), "{got:?}");
    for (g, e) in values.iter().zip(expected) {
        assert!(
            (g.is_nan() && e.is_nan()) || g == e,
            "{values:?} != {expected:?}"
        );
    }
}

#[test]
fn registry() {
    // `_registry`: one name per language, `ALLOWED_IN` decides where it is found.
    assert_eq!(Helper::lookup("any", Lang::Where), Some(Helper::Any));
    assert_eq!(Helper::lookup("any", Lang::Expression), None);
    assert_eq!(Helper::lookup("sum", Lang::Where), Some(Helper::Sum));
    assert_eq!(Helper::lookup("roll", Lang::Where), None);
    assert_eq!(
        Helper::lookup("select_from_lookup_arrays", Lang::Expression),
        Some(Helper::SelectFromLookupArrays)
    );
    let mut names: Vec<&str> = Helper::ALL.iter().map(|h| h.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Helper::ALL.len());
}

#[test]
fn any_not_bool() {
    let ts = LArray::from_coord(&timesteps());
    for (name, v) in [
        ("with_inf", Value::Float(with_inf())),
        ("no_dims", Value::Float(LArray::scalar(2.0))),
        ("timesteps", Value::Label(ts)),
    ] {
        let err = h::any(&v, name, &["techs"]).unwrap_err();
        assert!(
            err.to_string()
                .contains("Input to `any` must be a boolean array"),
            "{err}"
        );
    }
}

#[test]
fn any_exists() {
    let got = h::any(&Value::Bool(with_inf_as_bool()), "x", &["techs"]).unwrap();
    assert_eq!(got, boolean(vec![nodes()], &[true, true]));
    let all_false = boolean(vec![nodes(), techs()], &[false; 8]);
    let got = h::any(&Value::Bool(all_false), "x", &["techs"]).unwrap();
    assert_eq!(got, boolean(vec![nodes()], &[false, false]));
}

#[test]
fn sum_one_dim() {
    let got = h::sum(&Value::Float(only_techs()), &["techs"]).unwrap();
    assert!(got.dims().is_empty());
    assert_eq!(got.to_dense(), [6.0]);
}

#[test]
fn sum_one_of_two_dims() {
    let got = h::sum(&Value::Float(with_inf()), &["techs"]).unwrap();
    assert_close(&got, &["nodes"], &[5.0, INF]);
    // Dimensions the array lacks are ignored.
    let got = h::sum(&Value::Float(with_inf()), &["techs", "costs"]).unwrap();
    assert_close(&got, &["nodes"], &[5.0, INF]);
}

#[test]
fn sum_two_dims_bool() {
    let got = h::sum(&Value::Bool(with_inf_as_bool()), &["nodes", "techs"]).unwrap();
    assert_eq!(got.to_dense(), [5.0]);
}

#[test]
fn reduce_carrier_dim() {
    let all_true_carriers = h::bool_to_float(&boolean(vec![carriers(), techs()], &[true; 8]));
    let reduced = h::reduce_carrier_dim(&all_true_carriers, &carrier_out()).unwrap();
    let total = h::sum(&Value::Float(reduced.clone()), &["nodes", "techs"]).unwrap();
    assert_eq!(total.to_dense(), [15.0]);
    let mut dims = reduced.dim_names();
    dims.sort_unstable();
    assert_eq!(dims, ["nodes", "techs"]);
}

fn lookup_techs() -> LArray<Option<Label>> {
    labels(vec![techs()], &[Some("foobar"), None, Some("foobaz"), None])
}

#[test]
fn select_from_lookup_arrays() {
    let lt = lookup_techs();
    let got = h::select_from_lookup_arrays(
        &with_inf(),
        "with_inf",
        &[("techs".into(), "lookup_techs".into(), &lt)],
    )
    .unwrap();
    assert_close(
        &got,
        &["nodes", "techs"],
        &[1.0, NAN, NAN, NAN, INF, NAN, 2.0, NAN],
    );
    assert_eq!(got.coord("techs").unwrap(), &techs());

    let ln = labels(
        vec![nodes(), techs()],
        &[
            Some("bar"),
            None,
            Some("bar"),
            None,
            Some("foo"),
            None,
            None,
            None,
        ],
    );
    let ltm = labels(
        vec![nodes(), techs()],
        &[
            Some("foobar"),
            None,
            Some("foobaz"),
            None,
            Some("bazfoo"),
            None,
            None,
            None,
        ],
    );
    let got = h::select_from_lookup_arrays(
        &with_inf(),
        "with_inf",
        &[
            ("nodes".into(), "lookup_multi_dim_nodes".into(), &ln),
            ("techs".into(), "lookup_multi_dim_techs".into(), &ltm),
        ],
    )
    .unwrap();
    assert_close(
        &got,
        &["nodes", "techs"],
        &[INF, NAN, 2.0, NAN, 3.0, NAN, NAN, NAN],
    );
}

#[test]
fn select_from_lookup_arrays_errors() {
    let lt = lookup_techs();
    let nodes_true = h::bool_to_float(&boolean(vec![nodes()], &[true, true]));
    let err = h::select_from_lookup_arrays(
        &nodes_true,
        "nodes_true",
        &[("techs".into(), "lookup_techs".into(), &lt)],
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "Cannot select items from `nodes_true` on the dimensions {'techs'} since the array is not indexed over the dimensions {'techs'}"
    );

    let ln = labels(vec![nodes(), techs()], &[None; 8]);
    let err = h::select_from_lookup_arrays(
        &with_inf(),
        "with_inf",
        &[
            ("techs".into(), "lookup_techs".into(), &lt),
            ("nodes".into(), "lookup_multi_dim_nodes".into(), &ln),
        ],
    )
    .unwrap_err();
    let msg = err.to_string();
    for part in [
        "lookup arrays used to select items from `with_inf",
        "'techs'",
        "'nodes'",
    ] {
        assert!(msg.contains(part), "{msg}");
    }

    let no_match = labels(vec![techs()], &[Some("foo"), None, Some("bar"), None]);
    let err = h::select_from_lookup_arrays(
        &with_inf(),
        "with_inf",
        &[("techs".into(), "lookup_techs_no_match".into(), &no_match)],
    )
    .unwrap_err();
    assert!(matches!(err, HelperError::Index(_)));
    assert!(err.to_string().contains(
        "Trying to select items on the dimension techs from the lookup_techs_no_match lookup array, but no matches found."
    ));
}

#[test]
fn get_val_at_index() {
    for (i, hours) in [(0, 0), (1, 1), (-1, 3)] {
        assert_eq!(
            h::get_val_at_index(&timesteps(), i).unwrap(),
            Label::Datetime(Y2000 + hours * HOUR)
        );
    }
}

#[test]
fn get_val_at_index_not_one_mapping() {
    for kwargs in [vec![], vec![("foo".to_owned(), 1), ("bar".to_owned(), 1)]] {
        let err = bind(Helper::GetValAtIndex, vec![], kwargs).unwrap_err();
        assert_eq!(err, "Supply one (and only one) dimension:index mapping");
    }
}

#[test]
fn roll() {
    for shift in [1.0, -3.0] {
        let rolled = h::roll(&with_inf(), &[("techs".into(), shift)]).unwrap();
        assert_eq!(rolled.get(&[0, 0]), 3.0);
    }
}

#[test]
fn expression_where_techs_only() {
    let cond = boolean(vec![techs()], &[true, true, false, false]);
    let got = h::where_(&only_techs(), &Value::Bool(cond));
    assert_close(&got, &["techs"], &[NAN, 1.0, NAN, NAN]);
}

#[test]
fn expression_where_techs_add_nodes() {
    let cond = boolean(
        vec![nodes(), techs()],
        &[true, true, false, false, false, false, true, true],
    );
    let got = h::where_(&only_techs(), &Value::Bool(cond));
    assert_eq!(got.coord("nodes").unwrap(), &nodes());
    assert_close(
        &got,
        &["nodes", "techs"],
        &[NAN, 1.0, NAN, NAN, NAN, NAN, 2.0, 3.0],
    );
}

#[test]
fn expression_where_techs_add_new_dim() {
    let new_dim = Coord::strings("new_dim", &["a", "b"]).unwrap();
    let cond = boolean(
        vec![new_dim, techs()],
        &[true, true, false, false, false, false, true, true],
    );
    let got = h::where_(&only_techs(), &Value::Bool(cond));
    assert_close(
        &got,
        &["new_dim", "techs"],
        &[NAN, 1.0, NAN, NAN, NAN, NAN, 2.0, 3.0],
    );
}

#[test]
fn expression_where_no_shared_dim() {
    let new_dim = Coord::strings("new_dim", &["a", "b"]).unwrap();
    let cond = boolean(vec![new_dim], &[true, false]);
    let got = h::where_(&only_techs(), &Value::Bool(cond));
    assert_close(
        &got,
        &["new_dim", "techs"],
        &[NAN, 1.0, 2.0, 3.0, NAN, NAN, NAN, NAN],
    );
}

#[test]
fn expression_where_no_initial_dim() {
    let cond = boolean(vec![nodes()], &[true, false]);
    let got = h::where_(&LArray::scalar(2.0), &Value::Bool(cond));
    assert_close(&got, &["nodes"], &[2.0, NAN]);
}

#[test]
fn group_sum_one_dim() {
    for (groups, expected) in [
        (
            [Some("group_1"), None, Some("group_1"), Some("group_2")],
            [2.0, 3.0, INF, NAN],
        ),
        (
            [
                Some("group_1"),
                Some("group_1"),
                Some("group_2"),
                Some("group_2"),
            ],
            [1.0, 4.0, INF, 1.0],
        ),
    ] {
        let groupby = labels(vec![techs()], &groups);
        let got = h::group_sum(&with_inf(), &groupby, "new_dim").unwrap();
        assert_eq!(
            got.coord("new_dim").unwrap(),
            &Coord::strings("new_dim", &["group_1", "group_2"]).unwrap()
        );
        assert_close(&got, &["nodes", "new_dim"], &expected);
    }
}

#[test]
fn group_sum_multi_dim() {
    let groupby = labels(
        vec![nodes(), techs()],
        &[
            Some("group_1"),
            None,
            Some("group_1"),
            Some("group_2"),
            None,
            Some("group_1"),
            Some("group_1"),
            Some("group_2"),
        ],
    );
    let got = h::group_sum(&with_inf(), &groupby, "new_dim").unwrap();
    assert_close(&got, &["new_dim"], &[5.0, 3.0]);
}

fn timeseries() -> LArray<f64> {
    float(vec![timesteps()], &[1.0; 4])
}

#[test]
fn group_datetime() {
    let got = h::group_datetime(&timeseries(), "timesteps", "date", GroupDtype::String).unwrap();
    assert_eq!(
        got.coord("date").unwrap().labels(),
        [Label::str("2000-01-01")]
    );
    assert_close(&got, &["date"], &[4.0]);

    let got = h::group_datetime(&timeseries(), "timesteps", "time", GroupDtype::String).unwrap();
    let times: Vec<String> = got
        .coord("time")
        .unwrap()
        .labels()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(times, ["00:00:00", "01:00:00", "02:00:00", "03:00:00"]);
    assert_close(&got, &["time"], &[1.0; 4]);

    let got = h::group_datetime(&timeseries(), "timesteps", "hour", GroupDtype::Integer).unwrap();
    assert_eq!(
        got.coord("hour").unwrap().labels(),
        [Label::Int(0), Label::Int(1), Label::Int(2), Label::Int(3)]
    );
    assert_close(&got, &["hour"], &[1.0; 4]);

    let ts_nodes = float(vec![nodes(), timesteps()], &[1.0; 8]);
    let got = h::group_datetime(&ts_nodes, "timesteps", "hour", GroupDtype::Integer).unwrap();
    assert_close(&got, &["nodes", "hour"], &[1.0; 8]);
}

#[test]
fn sum_next_n() {
    let got = h::sum_next_n(&only_techs(), "techs", 2.0).unwrap();
    assert_close(&got, &["techs"], &[1.0, 3.0, 5.0, 3.0]);
    let got = h::sum_next_n(&with_inf(), "techs", 2.0).unwrap();
    assert_close(
        &got,
        &["nodes", "techs"],
        &[1.0, 1.0, 4.0, 3.0, INF, 3.0, 1.0, NAN],
    );
}

#[test]
fn map_dim() {
    let mapper = labels(
        vec![techs()],
        &[Some("foo"), None, Some("bar"), Some("foo")],
    );
    let got = h::map_dim(&nodes(), &mapper);
    let got = got.transpose(&["nodes", "techs"]).unwrap();
    assert_eq!(
        got.to_dense(),
        [true, false, false, true, false, false, true, false]
    );
}
