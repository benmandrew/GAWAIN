//! `weaken`: searches for weakenings of an LTL property that a nuXmv model
//! satisfies.
//!
//! ```text
//! weaken --model m.smv --atoms a,b,... --ltl 'φ' --seed N --generations G --population P
//! ```
//!
//! `--atoms` lists the model's boolean variables or DEFINEs that formulas
//! may mention, and must cover the atoms of φ. Each repair prints on one
//! line as `repair<TAB>similarity<TAB>formula`, then the oracle counters.
//! Exits 0 after a search, 1 when the model already satisfies φ, and 2 on a
//! usage error or when φ's own verdict is unknown.

use std::process::ExitCode;
use std::time::{Duration, Instant};

use aragon::ltl::Formula;
use aragon::oracle::nuxmv::Nuxmv;
use aragon::oracle::process::Limits;
use aragon::oracle::{Oracle, Verdict, Words};
use aragon::search::{gate, search, Params, Rng};

const USAGE: &str =
    "usage: weaken --model m.smv --atoms a,b,... --ltl 'φ' --seed N --generations G --population P";

/// Each nuXmv check.
const NUXMV_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(600),
    memory_bytes: 8 << 30,
};
/// Each `ltlfilt` batch.
const SPOT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};

struct Args {
    model: String,
    atoms: Vec<String>,
    ltl: Formula,
    seed: u64,
    params: Params,
}

fn parse_args() -> Result<Args, String> {
    let mut model = None;
    let mut atoms = None;
    let mut ltl = None;
    let mut seed = None;
    let mut generations = None;
    let mut population = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|e| format!("{flag} {value}: {e}"))
        };
        match flag.as_str() {
            "--model" => model = Some(value.clone()),
            "--atoms" => {
                atoms = Some(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|a| !a.is_empty())
                        .map(String::from)
                        .collect::<Vec<_>>(),
                )
            }
            "--ltl" => {
                let formula: Formula = value.parse().map_err(|e| format!("--ltl: {e}"))?;
                ltl = Some(formula);
            }
            "--seed" => seed = Some(number()?),
            "--generations" => generations = Some(number()? as usize),
            "--population" => population = Some(number()? as usize),
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    let missing = |name: &str| format!("missing {name}");
    let args = Args {
        model: model.ok_or_else(|| missing("--model"))?,
        atoms: atoms.ok_or_else(|| missing("--atoms"))?,
        ltl: ltl.ok_or_else(|| missing("--ltl"))?,
        seed: seed.ok_or_else(|| missing("--seed"))?,
        params: Params {
            generations: generations.ok_or_else(|| missing("--generations"))?,
            population: population.ok_or_else(|| missing("--population"))?,
        },
    };
    if args.params.population == 0 {
        return Err("--population must be at least 1".into());
    }
    // A lasso records only the listed atoms, and reads any other as false,
    // so an unlisted atom would make the lasso tier unsound.
    let unlisted: Vec<String> = args
        .ltl
        .atoms()
        .iter()
        .filter(|a| !args.atoms.iter().any(|b| b == &***a))
        .map(|a| a.to_string())
        .collect();
    if !unlisted.is_empty() {
        return Err(format!("--atoms lacks {}", unlisted.join(", ")));
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("weaken: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let start = Instant::now();
    let backend = Nuxmv::new(&args.model, &args.atoms, NUXMV_LIMITS);
    let words = Words::sample(args.atoms.iter().map(String::as_str), args.seed);
    let mut oracle = Oracle::new(backend, words, SPOT_LIMITS);
    let phi = args.ltl;
    match oracle.check(&phi) {
        Verdict::Holds => {
            eprintln!("weaken: the model already satisfies {phi}");
            return ExitCode::from(1);
        }
        Verdict::Unknown(reason) => {
            eprintln!("weaken: no verdict for {phi}: {reason}");
            return ExitCode::from(2);
        }
        Verdict::Fails(_) => println!("input fails: {phi}"),
    }
    let mut rng = Rng::new(args.seed);
    let population = search(&phi, &mut oracle, args.params, SPOT_LIMITS, &mut rng);
    let outcome = gate(&phi, &population, SPOT_LIMITS);
    for error in &outcome.errors {
        eprintln!("weaken: output gate: {error}");
    }
    for repair in &outcome.repairs {
        println!("repair\t{:.4}\t{}", repair.similarity, repair.formula);
    }
    let c = oracle.counters();
    println!("repairs: {}", outcome.repairs.len());
    println!(
        "oracle: queries={} memo_hits={} lasso_hits={} implication_hits={} spot_calls={} spot_errors={} backend_calls={} backend_unknowns={}",
        c.queries,
        c.memo_hits,
        c.lasso_hits,
        c.implication_hits,
        c.spot_calls,
        c.spot_errors,
        c.backend_calls,
        c.backend_unknowns
    );
    println!("wall: {:.2}s", start.elapsed().as_secs_f64());
    ExitCode::SUCCESS
}
