//! Memory probe: RSS per preprocessing stage and the size of the dataset after each.
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use calliope_preprocess::data_tables::DataTable;
use calliope_preprocess::model_data::{ModelDataBuilder, ModelDataCleaner};
use calliope_preprocess::model_math::{build_math, get_math_priority};
use calliope_preprocess::xr::{Data, Dataset};
use calliope_preprocess::{ReadOptions, prepare_model_definition_with};

fn rss_mb() -> f64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    let l = s.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    l.split_whitespace().nth(1).unwrap().parse::<f64>().unwrap() / 1024.0
}

fn bytes(d: &Data) -> (usize, &'static str, usize) {
    match d {
        Data::Bool(v) => (v.len(), "bool", v.len()),
        Data::Int(v) => (v.len(), "int", v.len() * 8),
        Data::Float(v) => (v.len(), "float", v.len() * 8),
        Data::Datetime(v) => (v.len(), "datetime", v.len() * 8),
        Data::Object(v) => (v.len(), "object", v.len() * std::mem::size_of_val(&v[0])),
        Data::Sparse(s) => (s.len, "sparse", s.nnz() * 16),
    }
}

fn report(label: &str, ds: &Dataset) {
    let mut rows: Vec<(usize, String, String, &str, usize)> = ds
        .vars
        .iter()
        .map(|(k, v)| {
            let (n, t, b) = bytes(&v.data);
            (b, k.clone(), v.dims.join(","), t, n)
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let tot: usize = rows.iter().map(|r| r.0).sum();
    println!(
        "  [{label}] {} vars, {:.0} MB of values",
        rows.len(),
        tot as f64 / 1e6
    );
    for r in rows.iter().take(6) {
        println!(
            "    {:30} ({:28}) {:7} cells={:>11} {:>8.0} MB",
            r.1,
            r.2,
            r.3,
            r.4,
            r.0 as f64 / 1e6
        );
    }
}

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let samples: Arc<Mutex<Vec<(f64, f64)>>> = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let t0 = Instant::now();
    let (s2, st2) = (samples.clone(), stop.clone());
    let h = std::thread::spawn(move || {
        while !st2.load(Ordering::Relaxed) {
            s2.lock()
                .unwrap()
                .push((t0.elapsed().as_secs_f64(), rss_mb()));
            std::thread::sleep(Duration::from_millis(10));
        }
    });
    let mut marks: Vec<(String, f64)> = vec![];
    let mut mark = |name: &str| {
        let t = t0.elapsed().as_secs_f64();
        println!("{t:8.2}s  rss={:8.0} MB  end of {name}", rss_mb());
        marks.push((name.to_owned(), t));
    };
    let opts = ReadOptions::default();
    let def = calliope_yaml::read_yaml_file(path.as_str(), false).unwrap();
    let prepared = prepare_model_definition_with(
        &def,
        None,
        None,
        None,
        Some(Path::new(&path)),
        &opts.init_kwargs,
    )
    .unwrap();
    let init = &prepared.config.init;
    let math = build_math(&get_math_priority(init), &prepared.math_init, None).unwrap();
    mark("definition+math");
    let mut tables = Vec::new();
    for (name, t) in &prepared.definition.data_tables.0 {
        tables
            .push(DataTable::load(name, t, &opts.data_table_dfs, Some(Path::new(&path))).unwrap());
    }
    mark("load data tables (CSV)");
    for t in &tables {
        report(&format!("table {}", t.name), &t.dataset);
    }
    let mut builder =
        ModelDataBuilder::new(init, prepared.definition.clone(), &math, &mut tables).unwrap();
    mark("ModelDataBuilder::new (init_from_data_tables)");
    drop(tables);
    builder.build().unwrap();
    mark("ModelDataBuilder::build");
    report("after build", &builder.dataset);
    let mut cleaner = ModelDataCleaner::new(init, builder.dataset, &math, Default::default());
    cleaner.clean().unwrap();
    mark("ModelDataCleaner::clean");
    report("cleaned inputs", &cleaner.dataset);
    stop.store(true, Ordering::Relaxed);
    h.join().unwrap();
    let s = samples.lock().unwrap();
    let mut prev = 0.0;
    println!("peak RSS per stage:");
    for (name, t) in &marks {
        let pk = s
            .iter()
            .filter(|(ts, _)| *ts >= prev && *ts <= *t)
            .map(|x| x.1)
            .fold(0.0, f64::max);
        println!("  {name:48} {:6.1}s {pk:8.0} MB", t - prev);
        prev = *t;
    }
}
