//! Times preprocessing and building of a model over its full time series.
//!
//! `cargo run --release -p calliope-model --example build_timing -- <model.yaml> [scenario]`
//! (default: the national_scale example of `../calliope`).

use std::time::Instant;

use calliope_preprocess::{ReadOptions, read_yaml};
use calliope_yaml::{AttrTree, Node, Value};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().cloned().unwrap_or_else(|| {
        "../calliope/src/calliope/example_models/national_scale/model.yaml".to_owned()
    });
    let mut over = AttrTree::new();
    over.set_key("config.init.subset.timesteps", Node::new(Value::Null))
        .unwrap();
    let opts = ReadOptions {
        scenario: args.get(1).cloned(),
        override_dict: Some(over),
        ..ReadOptions::default()
    };
    let t = Instant::now();
    let m = read_yaml(path.as_ref(), &opts).expect("preprocess");
    let pre = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let built = calliope_model::build(&m).expect("build");
    let build = t.elapsed().as_secs_f64();
    println!(
        "timesteps {}  preprocess {pre:.2} s  build {build:.2} s  cols {}  rows {}  nnz {}",
        m.inputs.coord("timesteps").map_or(0, |c| c.len()),
        built.lp.num_cols(),
        built.lp.num_rows(),
        built.lp.num_nz()
    );
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(l) = status.lines().find(|l| l.starts_with("VmHWM"))
    {
        println!("peak RSS: {}", l.trim_start_matches("VmHWM:").trim());
    }
}
