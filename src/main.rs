//! `weaken`: searches for weakenings of an LTL property that a nuXmv model,
//! or a set of traces, satisfies.
//!
//! ```text
//! weaken --model m.smv --atoms a,b,... --ltl 'φ' --seed N --generations G --population P
//!        [--exclusive a,b,...]...
//! weaken --traces t.trace --ltl 'φ' --seed N --generations G --population P
//!        [--exclusive a,b,...]...
//! ```
//!
//! `--atoms` lists the model's boolean variables or DEFINEs that formulas
//! may mention, and must cover the atoms of φ. With `--traces` in place of
//! `--model`, M is the file's positive traces (`oracle::traces`), the atoms
//! are the file's, and formulas read under the file's semantics, LTLf for
//! finite traces. Groups of mutually exclusive atoms come from the model's
//! DEFINEs (`p := v = c`) and from each `--exclusive`, and print at
//! start-up; a candidate valid modulo them is trivial and drops. So does a
//! candidate that M's first letter alone makes true: `instance::initial`
//! finds Init, the letters over φ's atoms that start a run of M, which
//! prints as `initial: …`, and the invariant becomes I ∧ Init ("Triviality
//! at the initial state" in `docs/dev/search.md`). If Init is not found, a
//! warning goes to stderr and the search keeps I alone. A candidate that
//! holds but that φ is not proved to imply scores status 0, since the gate
//! could never report it ("Keeping the search on weakenings" in
//! `docs/dev/search.md`). Each
//! repair prints on one line as
//! `repair<TAB>similarity<TAB>semantic<TAB>rejection<TAB>vacuity<TAB>formula`,
//! where similarity is syntactic, semantic is the model-counting similarity
//! (over finite words under LTLf), rejection is the rejection strength
//! (`Counter::rejections`: ⊤ scores 0 and φ 1), and vacuity names the subformulae in
//! which the repair is vacuous on M, separated by `; `, `?` for an unknown
//! verdict, or `-`. When Spot's simplifier shortens the repair, its
//! simplified form follows as a seventh column. With `--traces`, a line
//! `negatives<TAB>rejected/total<TAB>formula` follows each repair. Then come
//! the oracle counters (search and gate together), the cost of the model
//! counts and the cost of the vacuity flags.
//! Before searching, `instance::classify` checks φ and ¬φ on M.
//! Exits 0 after a search, 1 when M already satisfies φ, 2 on a usage
//! error, and 3 when the instance is degenerate (M satisfies ¬φ). If
//! either check has no verdict, a warning goes to stderr and the search
//! runs.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gawain::count::Counter;
use gawain::instance::{classify, initial, Instance};
use gawain::ltl::{BinaryOp, Formula};
use gawain::oracle::nuxmv::{exclusive_groups, Nuxmv};
use gawain::oracle::process::Limits;
use gawain::oracle::traces::TraceSet;
use gawain::oracle::{Backend, Oracle, Words};
use gawain::search::{gate, invariant, search, Params, Rng};

const USAGE: &str = "usage: weaken (--model m.smv --atoms a,b,... | --traces t.trace) --ltl 'φ' --seed N --generations G --population P [--exclusive a,b,...]...";

/// Each nuXmv check.
const NUXMV_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(600),
    memory_bytes: 8 << 30,
};
/// Each `ltlfilt` batch of the oracle.
const SPOT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};
/// Each Spot exec of the search's validity filter, of the counter and of
/// the gate's pruning of dominated repairs. A
/// batch that fails is retried one formula per exec, so one hard formula
/// costs two of these limits and stays unfiltered or uncounted, and the
/// rest are still decided; see "Slow implication batches" in
/// `docs/dev/trace-sets.md`.
const SEARCH_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(2),
    memory_bytes: 4 << 30,
};
/// Each `ltlfilt` exec of the output gate's proof of φ ⇒ φ', which runs
/// once, and whose undecided pair drops a repair.
const GATE_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(10),
    memory_bytes: 4 << 30,
};

/// What M is.
enum Model {
    /// A nuXmv file and the atoms formulas may mention.
    Smv { path: String, atoms: Vec<String> },
    /// A trace-set file.
    Traces(String),
}

struct Args {
    model: Model,
    exclusive: Vec<Vec<String>>,
    ltl: Formula,
    seed: u64,
    params: Params,
}

fn parse_args() -> Result<Args, String> {
    let mut model = None;
    let mut traces = None;
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
            "--traces" => traces = Some(value.clone()),
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
    let model = match (model, traces, atoms) {
        (Some(path), None, Some(atoms)) => Model::Smv { path, atoms },
        (Some(_), None, None) => return Err(missing("--atoms")),
        (None, Some(path), None) => Model::Traces(path),
        (None, Some(_), Some(_)) => return Err("--traces takes its atoms from the file".into()),
        (Some(_), Some(_), _) => return Err("give --model or --traces, not both".into()),
        (None, None, _) => return Err(missing("--model or --traces")),
    };
    let args = Args {
        model,
        exclusive,
        ltl: ltl.ok_or_else(|| missing("--ltl"))?,
        seed: seed.ok_or_else(|| missing("--seed"))?,
        params: Params::new(
            generations.ok_or_else(|| missing("--generations"))?,
            population.ok_or_else(|| missing("--population"))?,
        ),
    };
    if args.params.population == 0 {
        return Err("--population must be at least 1".into());
    }
    Ok(args)
}

/// A lasso records only the listed atoms, and reads any other as false, so
/// an unlisted atom would make the lasso tier unsound.
fn check_atoms(phi: &Formula, atoms: &[String]) -> Result<(), String> {
    let unlisted: Vec<String> = phi
        .atoms()
        .iter()
        .filter(|a| !atoms.iter().any(|b| b == &***a))
        .map(|a| a.to_string())
        .collect();
    if unlisted.is_empty() {
        Ok(())
    } else {
        Err(format!("the atoms lack {}", unlisted.join(", ")))
    }
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
    let read = |path: &str| {
        std::fs::read_to_string(path).map_err(|e| {
            eprintln!("weaken: {path}: {e}");
            ExitCode::from(2)
        })
    };
    let (atoms, groups, backend) = match &args.model {
        Model::Smv { path, atoms } => {
            let source = match read(path) {
                Ok(source) => source,
                Err(code) => return code,
            };
            let backend = Nuxmv::new(path, atoms, NUXMV_LIMITS);
            (atoms.clone(), exclusive_groups(&source), Run::Smv(backend))
        }
        Model::Traces(path) => {
            let set = match read(path).map(|text| TraceSet::parse(&text)) {
                Ok(Ok(set)) => set,
                Ok(Err(e)) => {
                    eprintln!("weaken: {path}: {e}");
                    return ExitCode::from(2);
                }
                Err(code) => return code,
            };
            let atoms = set.atoms().iter().map(|a| a.to_string()).collect();
            (atoms, Vec::new(), Run::Traces(set))
        }
    };
    if let Err(e) = check_atoms(&args.ltl, &atoms) {
        eprintln!("weaken: {e}\n{USAGE}");
        return ExitCode::from(2);
    }
    // Only listed atoms can appear in a candidate, so a group keeps those.
    let listed = |name: &str| atoms.iter().any(|a| a == name);
    let groups: Vec<Vec<Arc<str>>> = groups
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
    let words = Words::sample(atoms.iter().map(String::as_str), args.seed);
    match backend {
        Run::Smv(backend) => run(
            &args,
            Oracle::new(backend, words, SPOT_LIMITS),
            &groups,
            None,
            start,
        ),
        Run::Traces(set) => {
            // A trace set answers in microseconds, faster than tiers 1 and 2
            // (docs/dev/oracle.md, "Trace sets").
            let negatives = set.clone();
            let oracle = Oracle::memo_only(set);
            run(&args, oracle, &groups, Some(&negatives), start)
        }
    }
}

/// The back end chosen by the flags.
enum Run {
    Smv(Nuxmv),
    Traces(TraceSet),
}

/// Checks φ, searches, gates and prints. `traces`, when given, counts the
/// negative traces each repair rejects.
fn run<B: Backend>(
    args: &Args,
    mut oracle: Oracle<B>,
    groups: &[Vec<Arc<str>>],
    traces: Option<&TraceSet>,
    start: Instant,
) -> ExitCode {
    let mut invariant = invariant(groups);
    let phi = &args.ltl;
    let check_start = Instant::now();
    let instance = classify(phi, &mut oracle);
    let check_wall = check_start.elapsed().as_secs_f64();
    match instance {
        Instance::Satisfied => {
            eprintln!("weaken: the model already satisfies {phi}");
            return ExitCode::from(1);
        }
        Instance::Degenerate => {
            eprintln!("weaken: degenerate: {phi} fails on every run of the model");
            return ExitCode::from(3);
        }
        Instance::Proper { unknown } => {
            for reason in unknown {
                eprintln!("weaken: warning: no verdict for {reason}; searching anyway");
            }
        }
    }
    println!("input: {phi}");
    println!("instance: proper time={check_wall:.2}s");
    match initial(&phi.atoms(), &mut oracle) {
        Ok(init) => {
            println!("initial: {}", init.simplify());
            invariant = Formula::binary(BinaryOp::And, invariant, init).simplify();
        }
        Err(reason) => eprintln!("weaken: warning: no initial-state predicate: {reason}"),
    }
    let mut rng = Rng::new(args.seed);
    let mut counter = Counter::new(phi, oracle.semantics(), SEARCH_LIMITS);
    let (population, _, _) = search(
        phi,
        &mut oracle,
        &mut counter,
        &invariant,
        args.params,
        SEARCH_LIMITS,
        &mut rng,
    );
    let search_counters = oracle.counters();
    let gate_start = Instant::now();
    let outcome = gate(
        phi,
        &population,
        &invariant,
        &mut oracle,
        GATE_LIMITS,
        SEARCH_LIMITS,
    );
    let gate_wall = gate_start.elapsed().as_secs_f64();
    for error in &outcome.errors {
        eprintln!("weaken: output gate: {error}");
    }
    let formulas: Vec<Formula> = outcome.repairs.iter().map(|r| r.formula.clone()).collect();
    let rejections = counter.rejections(&formulas);
    for (repair, rejection) in outcome.repairs.iter().zip(rejections) {
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
        let formula = repair.formula.to_string();
        let simplified = if repair.simplified == formula {
            String::new()
        } else {
            format!("\t{}", repair.simplified)
        };
        println!(
            "repair\t{:.4}\t{:.4}\t{rejection:.4}\t{flags}\t{formula}{simplified}",
            repair.similarity, repair.semantic
        );
        if let Some(traces) = traces {
            println!(
                "negatives\t{}/{}\t{}",
                traces.rejected(&repair.formula),
                traces.negatives().len(),
                repair.formula
            );
        }
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
    let k = counter.counts();
    println!(
        "count: queries={} hits={} translations={} formulas={} ganak_calls={} failures={} time={:.2}s tool_time={:.2}s wait={:.2}s",
        k.queries,
        k.hits,
        k.translations,
        k.formulas,
        k.ganak_calls,
        k.failures,
        k.time.as_secs_f64(),
        k.tool_time.as_secs_f64(),
        k.wait.as_secs_f64()
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
