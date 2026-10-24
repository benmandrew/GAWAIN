//! Scores given formulas against φ, as the search scores its repairs, for
//! the baselines in `bench/notes/baselines.md`: ⊤ and CEGIW's output.
//!
//! ```text
//! score [--timeout SECS] < pairs.tsv
//! ```
//!
//! Each input line is `φ<TAB>φ'`. Each output line, in input order, is
//! `semantic<TAB>syntactic`: the model-counting similarity of φ' to φ over
//! infinite words (`count::Counter`) and the syntactic similarity
//! (`Formula::syntactic_similarity`). A count that fails scores 0, as in
//! the search. `--timeout` bounds each Spot and Ganak exec (default
//! 120 s). Lines that share φ share one counter, so its counts are cached.

use std::collections::HashMap;
use std::io::{BufRead, Write as _};
use std::process::ExitCode;
use std::time::Duration;

use gawain::count::Counter;
use gawain::ltl::Formula;
use gawain::oracle::process::Limits;
use gawain::oracle::Semantics;

const USAGE: &str = "usage: score [--timeout SECS] < lines of φ<TAB>φ'";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("score: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut timeout = 120;
    let mut given = std::env::args().skip(1);
    while let Some(flag) = given.next() {
        match flag.as_str() {
            "--timeout" => {
                let value = given.next().ok_or("--timeout needs a value")?;
                timeout = value.parse().map_err(|e| format!("--timeout: {e}"))?;
            }
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    let limits = Limits {
        timeout: Duration::from_secs(timeout),
        memory_bytes: 4 << 30,
    };
    let parse = |text: &str| {
        text.parse::<Formula>()
            .map_err(|e| format!("{text:?}: {e}"))
    };
    let mut pairs = Vec::new();
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let (phi, candidate) = line.split_once('\t').ok_or(format!("no tab in {line:?}"))?;
        pairs.push((parse(phi)?, parse(candidate)?));
    }
    // Score each φ's candidates in one call, then print in input order.
    let mut by_phi: HashMap<&Formula, Vec<usize>> = HashMap::new();
    for (i, (phi, _)) in pairs.iter().enumerate() {
        by_phi.entry(phi).or_default().push(i);
    }
    let mut semantic = vec![0.0; pairs.len()];
    for (phi, indices) in by_phi {
        let candidates: Vec<Formula> = indices.iter().map(|&i| pairs[i].1.clone()).collect();
        let scores = Counter::new(phi, Semantics::Infinite, limits).similarities(&candidates);
        for (&i, score) in indices.iter().zip(scores) {
            semantic[i] = score;
        }
    }
    let mut out = std::io::stdout().lock();
    for ((phi, candidate), score) in pairs.iter().zip(semantic) {
        let syntactic = candidate.syntactic_similarity(phi);
        writeln!(out, "{score:.6}\t{syntactic:.6}").map_err(|e| e.to_string())?;
    }
    Ok(())
}
