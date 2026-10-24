//! Replays the lasso counts of one search, for "Lasso count time" in
//! `docs/dev/search.md`.
//!
//! ```text
//! count_bench [--timeout SECS] DUMP
//! ```
//!
//! `DUMP` holds what one search asked its [`Counter`]: an optional `#`
//! comment line, φ, then each call's candidates one per line, each call
//! ended by a `---` line (`bench/notes/count-dump.patch` writes it). Each
//! call is replayed in order on one counter, as the search makes it, so
//! the cache hits are the same. The counts are asked of
//! [`Counter::log_counts`], which counts φ, φ' and `φ & φ'` over the same
//! universe as [`Counter::similarities`] does, since every candidate's
//! atoms are φ's.
//!
//! Stdout gets one line per formula counted, `log2<TAB>formula`, with the
//! log2 count printed exactly (`None` for a failed count), so two builds can
//! be diffed. Stderr gets one summary line: the calls, the formulas, the
//! failures, the wall time of the counts and the part spent in `ltl2tgba`.
//! `--timeout` bounds each `ltl2tgba` exec (default 2 s, as in the search).

use std::collections::HashSet;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use gawain::count::Counter;
use gawain::ltl::{BinaryOp, Formula};
use gawain::oracle::process::Limits;
use gawain::oracle::Semantics;

const USAGE: &str = "usage: count_bench [--timeout SECS] DUMP";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("count_bench: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut timeout = 2;
    let mut path = None;
    let mut given = std::env::args().skip(1);
    while let Some(arg) = given.next() {
        match arg.as_str() {
            "--timeout" => {
                let value = given.next().ok_or("--timeout needs a value")?;
                timeout = value.parse().map_err(|e| format!("--timeout: {e}"))?;
            }
            _ if path.is_none() => path = Some(arg),
            _ => return Err(format!("unexpected argument {arg}")),
        }
    }
    let path = path.ok_or("no dump given")?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let parse = |line: &str| {
        line.parse::<Formula>()
            .map_err(|e| format!("{line:?}: {e}"))
    };
    let mut lines = text.lines().filter(|l| !l.starts_with('#'));
    let phi = parse(lines.next().ok_or("empty dump")?)?.canonical();
    let mut calls: Vec<Vec<Formula>> = vec![Vec::new()];
    for line in lines {
        if line == "---" {
            calls.push(Vec::new());
        } else {
            calls.last_mut().expect("one call").push(parse(line)?);
        }
    }
    calls.retain(|call| !call.is_empty());
    let limits = Limits {
        timeout: Duration::from_secs(timeout),
        memory_bytes: 4 << 30,
    };
    let mut counter = Counter::new(&phi, Semantics::Infinite, limits);
    let mut counted: Vec<Formula> = Vec::new();
    let mut seen = HashSet::new();
    let mut time = Duration::ZERO;
    for call in &calls {
        // φ, φ' and φ & φ' for each candidate, in the order the search
        // asks for them.
        let mut formulas = Vec::new();
        for candidate in call.iter().map(Formula::canonical) {
            if candidate == phi {
                continue;
            }
            let shared = Formula::binary(BinaryOp::And, phi.clone(), candidate.clone()).canonical();
            formulas.extend([phi.clone(), candidate, shared]);
        }
        for formula in &formulas {
            if seen.insert(formula.clone()) {
                counted.push(formula.clone());
            }
        }
        let start = Instant::now();
        counter.log_counts(&formulas);
        time += start.elapsed();
    }
    let logs = counter.log_counts(&counted);
    for (formula, log) in counted.iter().zip(logs) {
        println!("{log:?}\t{formula}");
    }
    let k = counter.counts();
    eprintln!(
        "{path}: {} calls, {} formulas, {} failures, count {:.3} s, ltl2tgba {:.3} s",
        calls.len(),
        counted.len(),
        k.failures,
        time.as_secs_f64(),
        k.tool_time.as_secs_f64()
    );
    Ok(())
}
