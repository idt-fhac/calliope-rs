//! The `calliope` command-line interface (parity with Python's `cli.py`, plus `check`,
//! `lp-diff`, `inspect` and `bench`).

pub mod bench;
pub mod check;
pub mod cli;
pub mod error;
pub mod facade;
pub mod generate;
pub mod inspect;
pub mod lp_diff;
pub mod new_cmd;
