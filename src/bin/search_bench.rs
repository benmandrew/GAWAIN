//! Runs one weakening search on any back end and prints one JSON line, for
//! the binary-against-graded ablation in `docs/dev/search.md` ("Measured
//! runs"). The `weaken` binary is the user's entry point; this one adds the
//! other back ends and the status flag.
//!
//! ```text
//! search_bench --backend nuxmv|explicit|spin|tapaal --model PATH
//!     [--atoms a,b,...] [--ltl 'φ'] [--property ID]
//!     --seed N --generations G --population P --status graded|binary
//!     [--timeout SECS] [--memory-mb MB]
//! ```
//!
//! - `nuxmv`: `--model` is an SMV file, `--atoms` its DEFINEs and `--ltl`
//!   φ; the exclusive groups come from the DEFINEs, as in `weaken`.
//! - `explicit`: `--model` is a RERS parallel Promela file, explored into a
//!   Kripke structure; the atoms are its actions, which form one exclusive
//!   group, since one action holds per state.
//! - `spin`: `--model` is a stepwise Promela file from
//!   `bench/beem/search_models.py`, `--atoms` its atom macros.
//! - `tapaal`: `--model` is an MCC instance directory and `--property` the
//!   full id of one LTL property, which gives φ and the atoms.
//!
//! Every group keeps only φ's atoms, since mutation draws no other.
//! `--timeout` and `--memory-mb` bound each back-end call (default 120 s
//! and 4096 MB). The JSON line holds the instance class, the repairs with
//! their similarities, and the counters of the oracle, the counter and the
//! grader. It is read by `bench/graded_stats.py`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gawain::count::Counter;
use gawain::instance::{classify, Instance};
use gawain::ltl::Formula;
use gawain::mcc::{properties, Net};
use gawain::oracle::explicit::Explicit;
use gawain::oracle::nuxmv::{exclusive_groups, Nuxmv};
use gawain::oracle::process::Limits;
use gawain::oracle::spin::Spin;
use gawain::oracle::tapaal::Tapaal;
use gawain::oracle::{Backend, Lasso, Oracle, Words};
use gawain::rers::Model;
use gawain::search::{gate, invariant, search, Params, Rng, Status};

const USAGE: &str = "usage: search_bench --backend nuxmv|explicit|spin|tapaal --model PATH [--atoms a,b,...] [--ltl 'φ'] [--property ID] --seed N --generations G --population P --status graded|binary [--timeout SECS] [--memory-mb MB]";

/// Each `ltlfilt` and `ltl2tgba` batch, as in `weaken`.
const SPOT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};

#[derive(Default)]
struct Args {
    backend: String,
    model: String,
    atoms: Vec<String>,
    ltl: Option<Formula>,
    property: Option<String>,
    seed: u64,
    generations: usize,
    population: usize,
    status: Option<Status>,
    timeout: u64,
    memory_mb: u64,
    random_runs: usize,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        timeout: 120,
        memory_mb: 4096,
        ..Args::default()
    };
    let mut given = std::env::args().skip(1);
    while let Some(flag) = given.next() {
        let value = given.next().ok_or(format!("{flag} needs a value"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|e| format!("{flag} {value}: {e}"))
        };
        match flag.as_str() {
            "--backend" => args.backend = value.clone(),
            "--model" => args.model = value.clone(),
            "--atoms" => {
                args.atoms = value
                    .split(',')
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .map(String::from)
                    .collect()
            }
            "--ltl" => args.ltl = Some(value.parse().map_err(|e| format!("--ltl: {e}"))?),
            "--property" => args.property = Some(value.clone()),
            "--seed" => args.seed = number()?,
            "--generations" => args.generations = number()? as usize,
            "--population" => args.population = number()? as usize,
            "--status" => {
                args.status = Some(match value.as_str() {
                    "graded" => Status::Graded,
                    "binary" => Status::Binary,
                    "states" => Status::States,
                    _ => return Err(format!("--status {value}: graded, binary or states")),
                })
            }
            "--timeout" => args.timeout = number()?,
            "--memory-mb" => args.memory_mb = number()?,
            "--random-runs" => args.random_runs = number()? as usize,
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    if args.model.is_empty() || args.population == 0 || args.status.is_none() {
        return Err("--model, --population and --status are required".into());
    }
    Ok(args)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("search_bench: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let start = Instant::now();
    let limits = Limits {
        timeout: Duration::from_secs(args.timeout),
        memory_bytes: args.memory_mb << 20,
    };
    let read = |path: &str| std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"));
    let need_phi = || args.ltl.clone().ok_or("--ltl is required");
    match args.backend.as_str() {
        "nuxmv" => {
            let phi = need_phi()?;
            let groups = exclusive_groups(&read(&args.model)?);
            let backend = Nuxmv::new(&args.model, &args.atoms, limits);
            search_on(&args, backend, phi, &args.atoms, groups, Vec::new(), start)
        }
        "explicit" => {
            let phi = need_phi()?;
            let model = Model::parse(&read(&args.model)?).map_err(|e| e.to_string())?;
            let kripke = model.explore().map_err(|e| e.to_string())?;
            let atoms: Vec<String> = kripke.atoms().iter().map(|a| a.to_string()).collect();
            let groups = vec![kripke.atoms().to_vec()];
            let backend = Explicit::new(kripke, limits);
            let mut rng = Rng::new(args.seed ^ 0x5eed);
            let runs = backend.random_lassos(args.random_runs, 10_000, &mut rng);
            search_on(&args, backend, phi, &atoms, groups, runs, start)
        }
        "spin" => {
            let phi = need_phi()?;
            let backend = Spin::stepwise(&args.model, &args.atoms, limits)?;
            search_on(&args, backend, phi, &args.atoms, Vec::new(), Vec::new(), start)
        }
        "tapaal" => {
            let dir = PathBuf::from(&args.model);
            let id = args.property.clone().ok_or("--property is required")?;
            let exam = id.rsplit('-').nth(1).ok_or("not an MCC property id")?;
            let pnml = dir.join("model.pnml");
            let net = Net::parse(&read(&pnml.to_string_lossy())?)?;
            let xml = read(&dir.join(format!("{exam}.xml")).to_string_lossy())?;
            let property = properties(&xml, &net)?
                .into_iter()
                .find(|p| p.id == id)
                .ok_or(format!("{id}: not in {exam}.xml"))?;
            let atoms: Vec<String> = property.atoms.iter().map(|(a, _)| a.to_string()).collect();
            let backend = Tapaal::new(&pnml, net, property.atoms, limits);
            search_on(&args, backend, property.formula, &atoms, Vec::new(), Vec::new(), start)
        }
        other => Err(format!("unknown back end {other}")),
    }
}

/// Classifies, searches, gates and prints the JSON line.
fn search_on<B: Backend>(
    args: &Args,
    backend: B,
    phi: Formula,
    atoms: &[String],
    groups: Vec<Vec<Arc<str>>>,
    runs: Vec<Lasso>,
    start: Instant,
) -> Result<(), String> {
    let used = phi.atoms();
    if let Some(missing) = used.iter().find(|a| !atoms.iter().any(|b| b == &***a)) {
        return Err(format!("the atoms lack {missing}"));
    }
    let groups: Vec<Vec<Arc<str>>> = groups
        .into_iter()
        .map(|g| g.into_iter().filter(|a| used.contains(a)).collect::<Vec<_>>())
        .filter(|g| g.len() > 1)
        .collect();
    let invariant = invariant(&groups);
    let words = Words::sample(atoms.iter().map(String::as_str), args.seed);
    let mut oracle = Oracle::new(backend, words, SPOT_LIMITS);
    let random_runs = runs.len();
    for run in runs {
        oracle.store(run);
    }
    let mut out = String::from("{");
    field(&mut out, "phi", &json(&phi.to_string()));
    field(&mut out, "seed", &args.seed.to_string());
    let status = match args.status {
        Some(Status::Binary) => "binary",
        Some(Status::States) => "states",
        _ => "graded",
    };
    field(&mut out, "status", &json(status));
    field(&mut out, "random_runs", &random_runs.to_string());
    let class = match classify(&phi, &mut oracle) {
        Instance::Satisfied => "satisfied".to_string(),
        Instance::Degenerate => "degenerate".to_string(),
        Instance::Proper { unknown } => format!("proper{}", "?".repeat(unknown.len())),
    };
    field(&mut out, "instance", &json(&class));
    let mut repairs = String::from("[");
    let mut grading = Default::default();
    let mut search_wall = 0.0;
    let mut gate_wall = 0.0;
    if class.starts_with("proper") {
        let params = Params {
            generations: args.generations,
            population: args.population,
            status: args.status.unwrap_or_default(),
        };
        let mut rng = Rng::new(args.seed);
        let mut counter = Counter::new(&phi, SPOT_LIMITS);
        let searched = Instant::now();
        let (population, counts) = search(
            &phi,
            &mut oracle,
            &mut counter,
            &invariant,
            params,
            SPOT_LIMITS,
            &mut rng,
        );
        search_wall = searched.elapsed().as_secs_f64();
        grading = counts;
        let k = counter.counts();
        field(&mut out, "count_time", &format!("{:.3}", k.time.as_secs_f64()));
        field(&mut out, "count_wait", &format!("{:.3}", k.wait.as_secs_f64()));
        let best = population
            .iter()
            .filter(|s| !s.holds())
            .map(|s| s.objectives[0])
            .fold(0.0, f64::max);
        field(&mut out, "best_failing_status", &format!("{best:.6}"));
        let holding = population.iter().filter(|s| s.holds()).count();
        field(&mut out, "holding", &holding.to_string());
        let gated = Instant::now();
        let outcome = gate(&phi, &population, &invariant, &mut oracle, SPOT_LIMITS);
        gate_wall = gated.elapsed().as_secs_f64();
        for (i, repair) in outcome.repairs.iter().enumerate() {
            if i > 0 {
                repairs.push(',');
            }
            let _ = write!(
                repairs,
                "{{\"formula\":{},\"similarity\":{:.6},\"semantic\":{:.6}}}",
                json(&repair.formula.to_string()),
                repair.similarity,
                repair.semantic
            );
        }
        let errors: Vec<String> = outcome.errors.iter().map(|e| json(e)).collect();
        field(&mut out, "gate_errors", &format!("[{}]", errors.join(",")));
    }
    repairs.push(']');
    field(&mut out, "repairs", &repairs);
    let c = oracle.counters();
    for (name, value) in [
        ("queries", c.queries),
        ("memo_hits", c.memo_hits),
        ("lasso_hits", c.lasso_hits),
        ("implication_hits", c.implication_hits),
        ("spot_calls", c.spot_calls),
        ("backend_calls", c.backend_calls),
        ("backend_unknowns", c.backend_unknowns),
        ("rescores", grading.rescores),
        ("evaluations", grading.evaluations),
        ("lassos", grading.lassos as u64),
        ("refuting", grading.refuting as u64),
    ] {
        field(&mut out, name, &value.to_string());
    }
    field(
        &mut out,
        "grade_time",
        &format!("{:.4}", grading.time.as_secs_f64()),
    );
    field(&mut out, "search_wall", &format!("{search_wall:.3}"));
    field(&mut out, "gate_wall", &format!("{gate_wall:.3}"));
    field(
        &mut out,
        "wall",
        &format!("{:.3}", start.elapsed().as_secs_f64()),
    );
    out.pop();
    out.push('}');
    println!("{out}");
    Ok(())
}

/// Appends `"name":value,` to a JSON object under construction.
fn field(out: &mut String, name: &str, value: &str) {
    let _ = write!(out, "\"{name}\":{value},");
}

/// `text` as a JSON string.
fn json(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
