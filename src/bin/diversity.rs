//! The pairwise behavioural distances between formulas of one instance, for
//! `bench/diversity_stats.py`.
//!
//! ```text
//! diversity [--atoms a,b,...] [--semantics infinite|finite] [--seed N]
//!           [--traces N] < formulas
//! ```
//!
//! Reads one formula per line, samples the traces of `gawain::diversity`
//! over the given atoms and every atom of the formulas (default seed 1,
//! 65,536 traces, infinite semantics), and prints one JSON object: the
//! atoms, the number of traces, how many satisfy each formula, and the
//! distance matrix in input order. Each distance is printed as Rust's
//! shortest round-trip form, so Python reads back the exact `f64`. One run
//! is one instance on one core; parallelism is across instances.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::{BufRead, Write as _};
use std::process::ExitCode;
use std::sync::Arc;

use gawain::diversity::{distance, Traces};
use gawain::ltl::Formula;
use gawain::oracle::Semantics;

const USAGE: &str = "usage: diversity [--atoms a,b,...] [--semantics infinite|finite] \
                     [--seed N] [--traces N] < formulas";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("diversity: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut atoms: BTreeSet<Arc<str>> = BTreeSet::new();
    let mut semantics = Semantics::Infinite;
    let (mut seed, mut count) = (1u64, 1usize << 16);
    let mut given = std::env::args().skip(1);
    while let Some(flag) = given.next() {
        let value = given.next().ok_or(format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--atoms" => atoms.extend(value.split(',').filter(|a| !a.is_empty()).map(Arc::from)),
            "--semantics" => {
                semantics = match value.as_str() {
                    "infinite" => Semantics::Infinite,
                    "finite" => Semantics::Finite,
                    _ => return Err(format!("unknown semantics {value}")),
                }
            }
            "--seed" => seed = value.parse().map_err(|e| format!("--seed: {e}"))?,
            "--traces" => count = value.parse().map_err(|e| format!("--traces: {e}"))?,
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    let mut formulas = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let f: Formula = line.parse().map_err(|e| format!("{line:?}: {e}"))?;
        atoms.extend(f.atoms());
        formulas.push(f);
    }
    let traces = Traces::sample(atoms.clone(), semantics, seed, count);
    let sets: Vec<Vec<u64>> = formulas.iter().map(|f| traces.satisfying(f)).collect();

    let mut out = String::from("{\"atoms\": [");
    for (i, a) in atoms.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        write!(
            out,
            "{sep}\"{}\"",
            a.replace('\\', "\\\\").replace('"', "\\\"")
        )
        .unwrap();
    }
    write!(out, "], \"traces\": {}, \"satisfying\": [", traces.len()).unwrap();
    for (i, s) in sets.iter().enumerate() {
        let ones: u32 = s.iter().map(|w| w.count_ones()).sum();
        write!(out, "{}{ones}", if i == 0 { "" } else { ", " }).unwrap();
    }
    out.push_str("], \"distance\": [");
    for (i, a) in sets.iter().enumerate() {
        out.push_str(if i == 0 { "[" } else { ",\n[" });
        for (j, b) in sets.iter().enumerate() {
            let d = if i == j { 0.0 } else { distance(a, b) };
            write!(out, "{}{d:?}", if j == 0 { "" } else { ", " }).unwrap();
        }
        out.push(']');
    }
    out.push_str("]}\n");
    std::io::stdout()
        .lock()
        .write_all(out.as_bytes())
        .map_err(|e| e.to_string())
}
