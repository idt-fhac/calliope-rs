//! Writes the problem of a model as HiGHS sees it (HiGHS' LP writer, anonymous names), for
//! comparison with the `highs.lp` of a golden fixture (same column and row order means the
//! same problem is handed to the solver).
//!
//! `cargo run -p calliope-model --example highs_lp -- <model.yaml> <out.lp> [scenario]`

use calliope_model::{BuildOptions, Model};
use calliope_preprocess::ReadOptions;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = ReadOptions {
        scenario: args.get(2).cloned(),
        ..ReadOptions::default()
    };
    let mut m = Model::from_yaml(args[0].as_ref(), &opts).expect("read");
    m.build(&BuildOptions::default()).expect("build");
    m.backend_mut()
        .expect("built")
        .write_solver_model(args[1].as_ref())
        .expect("write");
}
