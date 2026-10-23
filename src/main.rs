//! `weaken`: searches for weakenings of an LTL property that a nuXmv model
//! satisfies.
//!
//! ```text
//! weaken --model m.smv --atoms a,b,... --ltl 'φ' --seed N --generations G --population P
//!        [--exclusive a,b,...]...
//! ```
//!
//! `--atoms` lists the model's boolean variables or DEFINEs that formulas
//! may mention, and must cover the atoms of φ. Groups of mutually exclusive
//! atoms come from the model's DEFINEs (`p := v = c`) and from each
//! `--exclusive`, and print at start-up; a candidate valid modulo them is
//! trivial and drops. Each repair prints on one line as
//! `repair<TAB>similarity<TAB>vacuity<TAB>formula`, where vacuity names the
//! subformulae in which the repair is vacuous on M, separated by `; `, `?`
//! for an unknown verdict, or `-`. Then come the oracle counters (search
//! and gate together) and the cost of the vacuity flags.
//! Exits 0 after a search, 1 when the model already satisfies φ, and 2 on a
//! usage error or when φ's own verdict is unknown.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aragon::ltl::Formula;
use aragon::oracle::nuxmv::{exclusive_groups, Nuxmv};
use aragon::oracle::process::Limits;
use aragon::oracle::{Oracle, Verdict, Words};
use aragon::search::{gate, invariant, search, Params, Rng};

const USAGE: &str =
    "usage: weaken --model m.smv --atoms a,b,... --ltl 'φ' --seed N --generations G --population P [--exclusive a,b,...]...";

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
    exclusive: Vec<Vec<String>>,
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
    let mut exclusive = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|e| format!("{flag} {value}: {e}"))
        };
        let list = || -> Vec<String> {
            value
                .split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .map(String::from)
                .collect()
        };
        match flag.as_str() {
            "--model" => model = Some(value.clone()),
            "--atoms" => atoms = Some(list()),
            "--exclusive" => exclusive.push(list()),
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
        exclusive,
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
    let source = match std::fs::read_to_string(&args.model) {
        Ok(source) => source,
        Err(e) => {
            eprintln!("weaken: {}: {e}", args.model);
            return ExitCode::from(2);
        }
    };
    // Only listed atoms can appear in a candidate, so a group keeps those.
    let listed = |name: &str| args.atoms.iter().any(|a| a == name);
    let groups: Vec<Vec<Arc<str>>> = exclusive_groups(&source)
        .into_iter()
        .chain(
            args.exclusive
                .iter()
                .map(|group| group.iter().map(|a| Arc::from(a.as_str())).collect()),
        )
        .map(|group: Vec<Arc<str>>| group.into_iter().filter(|a| listed(a)).collect::<Vec<_>>())
        .filter(|group| group.len() > 1)
        .collect();
    for group in &groups {
        println!("exclusive: {}", group.join(", "));
    }
    let invariant = invariant(&groups);
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
    let population = search(
        &phi,
        &mut oracle,
        &invariant,
        args.params,
        SPOT_LIMITS,
        &mut rng,
    );
    let search_counters = oracle.counters();
    let gate_start = Instant::now();
    let outcome = gate(&phi, &population, &invariant, &mut oracle, SPOT_LIMITS);
    let gate_wall = gate_start.elapsed().as_secs_f64();
    for error in &outcome.errors {
        eprintln!("weaken: output gate: {error}");
    }
    for repair in &outcome.repairs {
        let mut flags: Vec<String> = repair
            .vacuity
            .subformulae
            .iter()
            .map(Formula::to_string)
            .collect();
        if repair.vacuity.unknown {
            flags.push("?".into());
        }
        let flags = if flags.is_empty() {
            "-".into()
        } else {
            flags.join("; ")
        };
        println!(
            "repair\t{:.4}\t{flags}\t{}",
            repair.similarity, repair.formula
        );
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
    let v = outcome.vacuity;
    println!(
        "vacuity: queries={} unknowns={} backend_calls={}",
        v.queries,
        v.unknowns,
        c.backend_calls - search_counters.backend_calls
    );
    println!(
        "wall: {:.2}s gate={gate_wall:.2}s",
        start.elapsed().as_secs_f64()
    );
    ExitCode::SUCCESS
}
