//! `trace_bench`: checks the trace-set benchmarks against their own labels,
//! and builds weakening instances from them.
//!
//! ```text
//! trace_bench check DIR...
//! trace_bench instances --seed N DIR...
//! ```
//!
//! Both walk each `DIR` for `*.trace` files (`bench/traces_fetch.py` writes
//! them), or take it as one such file, and print one CSV row per file to
//! stdout.
//!
//! `check` asks whether the source formula ψ accepts every positive trace
//! and rejects every negative one, under the file's semantics, and times
//! one back-end check of ψ (every positive evaluated, since ψ holds) as the
//! median of 21 runs, in microseconds.
//!
//! `instances` builds a weakening instance from each file whose ψ accepts
//! every positive: φ is a strengthening of ψ by one or two
//! [`monotone_rewrite`]s that fails on some positive trace (M ⊭ φ) and holds
//! on another (M ⊭ ¬φ, so φ is not degenerate). Spot must prove φ ⇒ ψ under
//! the file's semantics, so ψ is a known weakening of φ that holds on M.
//! Up to 20 drafts are tried per file, and Spot gets 30 s per file; a file
//! with none left is reported with an empty φ and the reason.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gawain::ltl::Formula;
use gawain::oracle::process::Limits;
use gawain::oracle::traces::TraceSet;
use gawain::oracle::{implications, Backend, Semantics, Verdict};
use gawain::search::{monotone_rewrite, Direction, Rng};

const USAGE: &str = "usage: trace_bench check DIR... | trace_bench instances --seed N DIR...";
const RUNS: usize = 21;
const DRAFTS: usize = 20;
const SPOT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(30),
    memory_bytes: 4 << 30,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check") => check(&args[1..]),
        Some("instances") => match args.get(1..3) {
            Some([flag, seed]) if flag == "--seed" => match seed.parse() {
                Ok(seed) => instances(seed, &args[3..]),
                Err(e) => Err(format!("--seed {seed}: {e}")),
            },
            _ => Err("instances needs --seed N".into()),
        },
        _ => Err("unknown command".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("trace_bench: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Every `*.trace` file under `dirs`, or named by one, sorted.
fn files(dirs: &[String]) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                walk(&path, out)?;
            } else if path.extension().is_some_and(|x| x == "trace") {
                out.push(path);
            }
        }
        Ok(())
    }
    if dirs.is_empty() {
        return Err("no directory".into());
    }
    let mut out = Vec::new();
    for dir in dirs.iter().map(Path::new) {
        if dir.is_file() {
            out.push(dir.to_path_buf());
        } else {
            walk(dir, &mut out)?;
        }
    }
    out.sort();
    Ok(out)
}

/// The set in `path` and its source formula, or why there is none.
fn load(path: &Path) -> Result<(TraceSet, Formula), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let set = TraceSet::parse(&text)?;
    let psi = set.formula().cloned().ok_or("no formula")?;
    let unknown: Vec<String> = psi
        .atoms()
        .iter()
        .filter(|a| !set.atoms().contains(a))
        .map(|a| a.to_string())
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "formula atoms {} are not traced",
            unknown.join(" ")
        ));
    }
    Ok((set, psi))
}

fn semantics_name(semantics: Semantics) -> &'static str {
    match semantics {
        Semantics::Infinite => "infinite",
        Semantics::Finite => "finite",
    }
}

fn check(dirs: &[String]) -> Result<(), String> {
    println!("file,semantics,positives,negatives,max_length,size,accepted,rejected,check_us,error");
    for path in files(dirs)? {
        let (mut set, psi) = match load(&path) {
            Ok(loaded) => loaded,
            Err(e) => {
                println!("{},,,,,,,,,{}", path.display(), e.replace(',', ";"));
                continue;
            }
        };
        let accepted = set.positives().len() - set.failing(&psi);
        let rejected = set.rejected(&psi);
        let mut times: Vec<Duration> = (0..RUNS)
            .map(|_| {
                let start = Instant::now();
                std::hint::black_box(set.check(std::hint::black_box(&psi)));
                start.elapsed()
            })
            .collect();
        times.sort();
        let max_length = set
            .positives()
            .iter()
            .chain(set.negatives())
            .map(|t| t.prefix().len() + t.cycle().len())
            .max()
            .unwrap_or(0);
        println!(
            "{},{},{},{},{max_length},{},{accepted},{rejected},{:.2},",
            path.display(),
            semantics_name(set.semantics()),
            set.positives().len(),
            set.negatives().len(),
            psi.n_subformulae(),
            times[RUNS / 2].as_secs_f64() * 1e6,
        );
    }
    Ok(())
}

fn instances(seed: u64, dirs: &[String]) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    println!(
        "file,semantics,positives,negatives,psi,phi,phi_accepted,phi_rejected_negatives,error"
    );
    for path in files(dirs)? {
        let name = path.display();
        match load(&path).and_then(|(set, psi)| instance(set, &psi, &mut rng).map(|i| (psi, i))) {
            Ok((psi, (set, phi, accepted, rejected))) => println!(
                "{name},{},{},{},{},{},{accepted},{rejected},",
                semantics_name(set.semantics()),
                set.positives().len(),
                set.negatives().len(),
                quote(&psi),
                quote(&phi),
            ),
            Err(e) => println!("{name},,,,,,,,{}", e.replace(',', ";")),
        }
    }
    Ok(())
}

/// Draws up to [`DRAFTS`] strengthenings of `psi`, keeps those that fail
/// on some but not all positives of `set`, and returns the first one Spot
/// proves implies `psi`, with the positives it accepts and the negatives it
/// rejects. One `ltlfilt` exec per file, so a formula Spot cannot decide in
/// time costs only its own file.
fn instance(
    mut set: TraceSet,
    psi: &Formula,
    rng: &mut Rng,
) -> Result<(TraceSet, Formula, usize, usize), String> {
    if set.check(psi) != Verdict::Holds {
        return Err("ψ rejects a positive".into());
    }
    let positives = set.positives().len();
    let atoms: Vec<Arc<str>> = psi.atoms().into_iter().collect();
    let mut phis: Vec<(Formula, usize)> = Vec::new();
    for _ in 0..DRAFTS {
        let mut phi = monotone_rewrite(psi, Direction::Strengthen, &atoms, rng);
        if rng.coin() {
            phi = monotone_rewrite(&phi, Direction::Strengthen, &atoms, rng);
        }
        let phi = phi.simplify();
        let failing = set.failing(&phi);
        let fresh = phi.canonical() != psi.canonical()
            && phis.iter().all(|(p, _)| p.canonical() != phi.canonical());
        if failing > 0 && failing < positives && fresh {
            phis.push((phi, positives - failing));
        }
    }
    if phis.is_empty() {
        return Err("no draft fails on some but not all positives".into());
    }
    let pairs: Vec<(&Formula, &Formula)> = phis.iter().map(|(phi, _)| (phi, psi)).collect();
    let proved = implications(&pairs, set.semantics(), SPOT_LIMITS)
        .map_err(|e| format!("Spot gave no answer: {e}"))?;
    let (phi, accepted) = phis
        .into_iter()
        .zip(proved)
        .find(|(_, proved)| *proved)
        .map(|(draft, _)| draft)
        .ok_or("Spot proved no draft implies ψ")?;
    let rejected = set.rejected(&phi);
    Ok((set, phi, accepted, rejected))
}

/// A formula as a CSV field.
fn quote(formula: &Formula) -> String {
    format!("\"{}\"", formula.to_string().replace('"', "\"\""))
}
