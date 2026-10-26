//! Runs one weakening search on any back end and prints one JSON line, for
//! the binary-against-graded ablation in `docs/dev/search.md` ("Measured
//! runs"). The `weaken` binary is the user's entry point; this one adds the
//! other back ends and the status flag.
//!
//! ```text
//! search_bench --backend nuxmv|explicit|spin|tapaal|traces --model PATH
//!     [--atoms a,b,...] [--ltl 'φ'] [--property ID]
//!     --seed N --generations G --population P
//!     --status binary|graded|graded-feasible|graded-reserve-D
//!     [--timeout SECS] [--memory-mb MB] [--degenerate skip|search]
//!     [--initial ignore|flag|filter] [--search ga|random]
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
//! - `traces`: `--model` is a trace-set file (`oracle::traces`) and `--ltl`
//!   φ. M is its positive traces, read under its semantics, behind
//!   `Oracle::memo_only` as in `weaken --traces`.
//!
//! Every group keeps only φ's atoms, since mutation draws no other.
//! `--timeout` and `--memory-mb` bound each back-end call (default 120 s
//! and 4096 MB). A degenerate instance (M ⊨ ¬φ) is classified and not
//! searched, unless `--degenerate search` is given: CEGIW's case studies
//! include degenerate ones, and `bench/baselines_cegiw.py` compares the
//! two tools there too. `--initial flag` finds the initial-state
//! predicate Init of M over φ's atoms ([`initial`]) and marks each repair
//! that I ∧ Init implies; `--initial filter` also puts I ∧ Init in place of
//! I in the search's validity filter and the gate, so those repairs drop
//! ("Triviality at the initial state" in `docs/dev/search.md`). `filter`
//! is the default; `ignore` asks nothing. `--search random` runs the
//! random-mutation baseline ([`random_search`]) in place of the genetic
//! search, the default `ga`. The JSON line holds the instance class, the repairs with
//! their similarities, the search that ran, the candidates it bred and
//! scored, and the counters of the oracle, the counter and the
//! grader. It is read by `bench/graded_stats.py`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gawain::count::Counter;
use gawain::instance::{classify, initial, Instance};
use gawain::ltl::{BinaryOp, Formula};
use gawain::mcc::{properties, Net};
use gawain::oracle::explicit::Explicit;
use gawain::oracle::nuxmv::{exclusive_groups, Nuxmv};
use gawain::oracle::process::Limits;
use gawain::oracle::spin::Spin;
use gawain::oracle::tapaal::Tapaal;
use gawain::oracle::traces::TraceSet;
use gawain::oracle::{implications_each, Backend, Oracle, Words};
use gawain::rers::Model;
use gawain::search::{gate, invariant, random_search, search, Params, Rng, Status};

const USAGE: &str = "usage: search_bench --backend nuxmv|explicit|spin|tapaal|traces --model PATH [--atoms a,b,...] [--ltl 'φ'] [--property ID] --seed N --generations G --population P --status binary|graded|graded-feasible|graded-reserve-D [--timeout SECS] [--memory-mb MB] [--degenerate skip|search] [--initial ignore|flag|filter] [--search ga|random]";

/// Each `ltlfilt` and `ltl2tgba` batch, as in `weaken`.
const SPOT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};
/// Each Spot exec of the search and of the gate's pruning, as in `weaken`.
const SEARCH_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(2),
    memory_bytes: 4 << 30,
};
/// Each exec of the gate's proof of φ ⇒ φ', as in `weaken`.
const GATE_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(10),
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
    search_degenerate: bool,
    initial: Initial,
    random: bool,
}

/// What `--initial` asks of the initial-state predicate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Initial {
    /// Nothing.
    Ignore,
    /// Find it and mark the repairs it makes trivial.
    Flag,
    /// Also conjoin it to the invariant, so those repairs drop.
    #[default]
    Filter,
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
                    "graded-feasible" => Status::GradedFeasible,
                    other => match other.strip_prefix("graded-reserve-").map(str::parse) {
                        Some(Ok(divisor)) if divisor >= 1 => Status::GradedReserve(divisor),
                        _ => {
                            return Err(format!(
                            "--status {value}: binary, graded, graded-feasible or graded-reserve-D"
                        ))
                        }
                    },
                })
            }
            "--timeout" => args.timeout = number()?,
            "--memory-mb" => args.memory_mb = number()?,
            "--degenerate" => {
                args.search_degenerate = match value.as_str() {
                    "search" => true,
                    "skip" => false,
                    _ => return Err(format!("--degenerate {value}: skip or search")),
                }
            }
            "--initial" => {
                args.initial = match value.as_str() {
                    "ignore" => Initial::Ignore,
                    "flag" => Initial::Flag,
                    "filter" => Initial::Filter,
                    _ => return Err(format!("--initial {value}: ignore, flag or filter")),
                }
            }
            "--search" => {
                args.random = match value.as_str() {
                    "random" => true,
                    "ga" => false,
                    _ => return Err(format!("--search {value}: ga or random")),
                }
            }
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
            search_on(&args, backend, phi, &args.atoms, groups, start)
        }
        "explicit" => {
            let phi = need_phi()?;
            let model = Model::parse(&read(&args.model)?).map_err(|e| e.to_string())?;
            let kripke = model.explore().map_err(|e| e.to_string())?;
            let atoms: Vec<String> = kripke.atoms().iter().map(|a| a.to_string()).collect();
            let groups = vec![kripke.atoms().to_vec()];
            let backend = Explicit::new(kripke, limits);
            search_on(&args, backend, phi, &atoms, groups, start)
        }
        "spin" => {
            let phi = need_phi()?;
            let backend = Spin::stepwise(&args.model, &args.atoms, limits)?;
            search_on(&args, backend, phi, &args.atoms, Vec::new(), start)
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
            search_on(&args, backend, property.formula, &atoms, Vec::new(), start)
        }
        "traces" => {
            let phi = need_phi()?;
            let set =
                TraceSet::parse(&read(&args.model)?).map_err(|e| format!("{}: {e}", args.model))?;
            let atoms: Vec<String> = set.atoms().iter().map(|a| a.to_string()).collect();
            search_on(&args, set, phi, &atoms, Vec::new(), start)
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
    start: Instant,
) -> Result<(), String> {
    let used = phi.atoms();
    if let Some(missing) = used.iter().find(|a| !atoms.iter().any(|b| b == &***a)) {
        return Err(format!("the atoms lack {missing}"));
    }
    let groups: Vec<Vec<Arc<str>>> = groups
        .into_iter()
        .map(|g| {
            g.into_iter()
                .filter(|a| used.contains(a))
                .collect::<Vec<_>>()
        })
        .filter(|g| g.len() > 1)
        .collect();
    let invariant = invariant(&groups);
    let words = Words::sample(atoms.iter().map(String::as_str), args.seed);
    // A back end that lists its runs is a trace set, which answers faster
    // than tiers 1 and 2, as in `weaken --traces`.
    let mut oracle = match backend.runs() {
        Some(_) => Oracle::memo_only(backend),
        None => Oracle::new(backend, words, SPOT_LIMITS),
    };
    let mut out = String::from("{");
    field(&mut out, "phi", &json(&phi.to_string()));
    field(&mut out, "seed", &args.seed.to_string());
    let status = match args.status {
        Some(Status::Graded) => "graded".to_string(),
        Some(Status::GradedFeasible) => "graded-feasible".to_string(),
        Some(Status::GradedReserve(divisor)) => format!("graded-reserve-{divisor}"),
        _ => "binary".to_string(),
    };
    field(&mut out, "status", &json(&status));
    let which = if args.random { "random" } else { "ga" };
    field(&mut out, "search", &json(which));
    let class = match classify(&phi, &mut oracle) {
        Instance::Satisfied => "satisfied".to_string(),
        Instance::Degenerate => "degenerate".to_string(),
        Instance::Proper { unknown } => format!("proper{}", "?".repeat(unknown.len())),
    };
    field(&mut out, "instance", &json(&class));
    // I ∧ Init, when Init was asked for and found.
    let mut trivial_at_start = None;
    let mut invariant = invariant;
    if args.initial != Initial::Ignore {
        let asked = Instant::now();
        let calls = oracle.counters().backend_calls;
        let found = initial(&used, &mut oracle);
        let calls = oracle.counters().backend_calls - calls;
        field(&mut out, "initial_calls", &calls.to_string());
        let time = asked.elapsed().as_secs_f64();
        field(&mut out, "initial_time", &format!("{time:.3}"));
        match found {
            Ok(init) => {
                field(&mut out, "initial", &json(&init.simplify().to_string()));
                let both = Formula::binary(BinaryOp::And, invariant.clone(), init).simplify();
                if args.initial == Initial::Filter {
                    invariant = both.clone();
                }
                trivial_at_start = Some(both);
            }
            Err(reason) => field(&mut out, "initial_error", &json(&reason)),
        }
    }
    let mut repairs = String::from("[");
    let mut grading = Default::default();
    let mut search_wall = 0.0;
    let mut gate_wall = 0.0;
    if class.starts_with("proper") || (args.search_degenerate && class == "degenerate") {
        let params = Params {
            generations: args.generations,
            population: args.population,
            status: args.status.unwrap_or_default(),
        };
        let mut rng = Rng::new(args.seed);
        let mut counter = Counter::new(&phi, oracle.semantics(), SEARCH_LIMITS);
        let searched = Instant::now();
        let run = if args.random { random_search } else { search };
        let (population, counts, budget) = run(
            &phi,
            &mut oracle,
            &mut counter,
            &invariant,
            params,
            SEARCH_LIMITS,
            &mut rng,
        );
        search_wall = searched.elapsed().as_secs_f64();
        grading = counts;
        field(&mut out, "bred", &budget.bred.to_string());
        field(&mut out, "scored", &budget.scored.to_string());
        let k = counter.counts();
        field(
            &mut out,
            "count_time",
            &format!("{:.3}", k.time.as_secs_f64()),
        );
        field(
            &mut out,
            "count_wait",
            &format!("{:.3}", k.wait.as_secs_f64()),
        );
        field(&mut out, "count_failures", &k.failures.to_string());
        let best = population
            .iter()
            .filter(|s| !s.holds())
            .map(|s| s.objectives[0])
            .fold(0.0, f64::max);
        field(&mut out, "best_failing_status", &format!("{best:.6}"));
        let holding = population.iter().filter(|s| s.holds()).count();
        field(&mut out, "holding", &holding.to_string());
        let gated = Instant::now();
        let outcome = gate(
            &phi,
            &population,
            &invariant,
            &mut oracle,
            GATE_LIMITS,
            SEARCH_LIMITS,
        );
        gate_wall = gated.elapsed().as_secs_f64();
        // Whether I ∧ Init implies each repair, in one batch.
        let flags: Vec<Option<bool>> = match &trivial_at_start {
            Some(both) => {
                let pairs: Vec<(&Formula, &Formula)> =
                    outcome.repairs.iter().map(|r| (both, &r.formula)).collect();
                let (proved, _) = implications_each(&pairs, oracle.semantics(), GATE_LIMITS);
                proved.into_iter().map(Some).collect()
            }
            None => vec![None; outcome.repairs.len()],
        };
        for (i, (repair, flag)) in outcome.repairs.iter().zip(flags).enumerate() {
            if i > 0 {
                repairs.push(',');
            }
            let _ = write!(
                repairs,
                "{{\"formula\":{},\"similarity\":{:.6},\"semantic\":{:.6},\"vacuous\":{}",
                json(&repair.formula.to_string()),
                repair.similarity,
                repair.semantic,
                repair.vacuity.subformulae.len()
            );
            if let Some(flag) = flag {
                let _ = write!(repairs, ",\"initial_trivial\":{flag}");
            }
            repairs.push('}');
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
