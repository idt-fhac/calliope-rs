//! Developer tasks (`cargo run -p xtask -- <task>`). Placeholder until Phase 1 of `PLAN.md`.

fn main() {
    let task = std::env::args().nth(1);
    eprintln!("xtask: unknown task {task:?}; no tasks are implemented yet");
    std::process::exit(2);
}
