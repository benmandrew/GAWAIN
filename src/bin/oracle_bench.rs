//! Measures the tiered oracle on a stream of candidates per property.
//!
//! For each property φ the stream is φ, then a seeded shuffle of `ltlgrind`
//! mutants of φ with one and with two mutations, repeats kept, which stands
//! in for the offspring a GA run proposes. Each stream goes through a fresh
//! [`Oracle`], as one GA run would, and every query becomes one CSV row on
//! standard output: which tier answered, the verdict, the query's wall time
//! and, for a back-end call, the back end's time alone. The analysis is
//! `bench/oracle_stats.py`; the method and results are in
//! `docs/dev/oracle-measurements.md`.
//!
//! ```text
//! oracle_bench --backend spin|nuxmv --model M --atoms FILE --properties FILE
//!     --bench LABEL [--one N] [--two N] [--two-pool N] [--seed S]
//!     [--timeout SECS] [--memory-mb MB]
//! ```
//!
//! `--atoms` lists one atom per line. `--properties` has one property per
//! line, `name<TAB>formula`, in any syntax [`Formula`] parses. `--one` and
//! `--two` are how many one- and two-mutation mutants to draw (default 40
//! each). `ltlgrind` lists two-mutation mutants in a fixed order and there
//! can be millions, so the draw is from its first `--two-pool` (default
//! 5000). `--timeout` and `--memory-mb` bound each back-end tool call.

use std::cell::Cell;
use std::fs;
use std::io::{self, Write};
use std::process::{self as std_process, Command};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gawain::ltl::Formula;
use gawain::oracle::nuxmv::Nuxmv;
use gawain::oracle::process::{self, Limits};
use gawain::oracle::spin::Spin;
use gawain::oracle::{Backend, Counters, Oracle, Verdict, Words};

/// Bounds for each `ltlgrind` and `ltlfilt` exec.
const SPOT_LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 2 << 30,
};

/// The CSV header; [`row`] writes the fields in this order.
const HEADER: &str =
    "bench,property,seq,source,tier,verdict,query_s,backend_s,spot_calls,candidate,reason";

/// A back end that records how long its last call took.
struct Timed {
    inner: Box<dyn Backend>,
    last: Rc<Cell<Option<Duration>>>,
}

impl Backend for Timed {
    fn check(&mut self, formula: &Formula) -> Verdict {
        let start = Instant::now();
        let verdict = self.inner.check(formula);
        self.last.set(Some(start.elapsed()));
        verdict
    }
}

#[derive(Debug, PartialEq)]
struct Args {
    backend: String,
    model: String,
    atoms: String,
    properties: String,
    bench: String,
    one: usize,
    two: usize,
    two_pool: usize,
    seed: u64,
    timeout: u64,
    memory_mb: u64,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        backend: String::new(),
        model: String::new(),
        atoms: String::new(),
        properties: String::new(),
        bench: String::new(),
        one: 40,
        two: 40,
        two_pool: 5000,
        seed: 1,
        timeout: 120,
        memory_mb: 4096,
    };
    let mut pairs = args.chunks(2);
    for pair in pairs.by_ref() {
        let [flag, value] = pair else {
            return Err(format!("{} needs a value", pair[0]));
        };
        let number = || value.parse().map_err(|e| format!("{flag} {value}: {e}"));
        match flag.as_str() {
            "--backend" => parsed.backend = value.clone(),
            "--model" => parsed.model = value.clone(),
            "--atoms" => parsed.atoms = value.clone(),
            "--properties" => parsed.properties = value.clone(),
            "--bench" => parsed.bench = value.clone(),
            "--one" => parsed.one = number()? as usize,
            "--two" => parsed.two = number()? as usize,
            "--two-pool" => parsed.two_pool = number()? as usize,
            "--seed" => parsed.seed = number()?,
            "--timeout" => parsed.timeout = number()?,
            "--memory-mb" => parsed.memory_mb = number()?,
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    let required = [
        ("--backend", &parsed.backend),
        ("--model", &parsed.model),
        ("--atoms", &parsed.atoms),
        ("--properties", &parsed.properties),
        ("--bench", &parsed.bench),
    ];
    if let Some((flag, _)) = required.iter().find(|(_, v)| v.is_empty()) {
        return Err(format!("{flag} is required"));
    }
    if !matches!(parsed.backend.as_str(), "spin" | "nuxmv") {
        return Err(format!("unknown back end {}", parsed.backend));
    }
    Ok(parsed)
}

/// SplitMix64, for a seeded shuffle.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Fisher–Yates.
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }
}

/// `ltlgrind`'s mutants of `formula` with `mutations` mutations, at most
/// `max` of them, in its own order. A line that does not parse is skipped.
fn mutants(formula: &Formula, mutations: u32, max: Option<usize>) -> Result<Vec<Formula>, String> {
    let mut command = Command::new("ltlgrind");
    command.args(["-F", "-", "-m", &mutations.to_string()]);
    if let Some(max) = max {
        command.args(["-n", &max.to_string()]);
    }
    let output = process::run(&mut command, &format!("{formula}\n"), SPOT_LIMITS)
        .map_err(|e| format!("ltlgrind: {e}"))?;
    if !output.status.success() {
        return Err(format!("ltlgrind failed: {}", output.stderr.trim()));
    }
    Ok(output
        .stdout
        .lines()
        .filter_map(|line| line.parse().ok())
        .collect())
}

/// φ first, then `one` and `two` mutants drawn from `ltlgrind`'s lists,
/// shuffled together. Each candidate carries its source: `phi`, `m1` or
/// `m2`.
fn stream(
    phi: &Formula,
    args: &Args,
    rng: &mut Rng,
) -> Result<Vec<(&'static str, Formula)>, String> {
    let mut drawn = Vec::new();
    for (source, mutations, pool, take) in [
        ("m1", 1, None, args.one),
        ("m2", 2, Some(args.two_pool), args.two),
    ] {
        let mut list = mutants(phi, mutations, pool)?;
        rng.shuffle(&mut list);
        drawn.extend(list.into_iter().take(take).map(|f| (source, f)));
    }
    rng.shuffle(&mut drawn);
    drawn.insert(0, ("phi", phi.clone()));
    Ok(drawn)
}

/// Which tier answered, from the counters before and after one query.
fn tier(before: &Counters, after: &Counters) -> &'static str {
    if after.memo_hits > before.memo_hits {
        "memo"
    } else if after.lasso_hits > before.lasso_hits {
        "lasso"
    } else if after.implication_hits > before.implication_hits {
        "implication"
    } else {
        "backend"
    }
}

/// A CSV field, quoted when it holds a comma, a quote or a line break,
/// with line breaks turned into spaces.
fn field(text: &str) -> String {
    let text = text.replace(['\n', '\r'], " ");
    if text.contains([',', '"']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn row(fields: &[&str]) -> String {
    fields
        .iter()
        .map(|f| field(f))
        .collect::<Vec<_>>()
        .join(",")
}

fn lines(path: &str) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

fn run(args: &Args) -> Result<(), String> {
    let atoms = lines(&args.atoms)?;
    let limits = Limits {
        timeout: Duration::from_secs(args.timeout),
        memory_bytes: args.memory_mb << 20,
    };
    let mut out = io::stdout().lock();
    let write_err = |e: io::Error| format!("cannot write: {e}");
    writeln!(out, "{HEADER}").map_err(write_err)?;
    let mut rng = Rng(args.seed);
    for line in lines(&args.properties)? {
        let (name, text) = line
            .split_once('\t')
            .ok_or_else(|| format!("no tab in property line {line:?}"))?;
        let phi: Formula = text.parse().map_err(|e| format!("property {name}: {e}"))?;
        let candidates = stream(&phi, args, &mut rng)?;
        let last = Rc::new(Cell::new(None));
        let backend: Box<dyn Backend> = match args.backend.as_str() {
            "spin" => Box::new(Spin::new(&args.model, &atoms, limits)?),
            _ => Box::new(Nuxmv::new(&args.model, &atoms, limits)),
        };
        let timed = Timed {
            inner: backend,
            last: last.clone(),
        };
        let mut oracle = Oracle::new(
            timed,
            Words::sample(atoms.iter().map(String::as_str), args.seed),
            SPOT_LIMITS,
        );
        for (seq, (source, candidate)) in candidates.iter().enumerate() {
            let before = oracle.counters();
            last.set(None);
            let start = Instant::now();
            let verdict = oracle.check(candidate);
            let query = start.elapsed();
            let after = oracle.counters();
            let (verdict, reason) = match &verdict {
                Verdict::Holds => ("holds", ""),
                Verdict::Fails(_) => ("fails", ""),
                Verdict::Unknown(reason) => ("unknown", reason.as_str()),
            };
            let backend_s = last.get().map(|d| format!("{:.6}", d.as_secs_f64()));
            let line = row(&[
                &args.bench,
                name,
                &seq.to_string(),
                source,
                tier(&before, &after),
                verdict,
                &format!("{:.6}", query.as_secs_f64()),
                backend_s.as_deref().unwrap_or(""),
                &(after.spot_calls - before.spot_calls).to_string(),
                &candidate.to_string(),
                reason,
            ]);
            writeln!(out, "{line}").map_err(write_err)?;
            out.flush().map_err(write_err)?;
        }
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = parse_args(&args).and_then(|args| run(&args)) {
        eprintln!("oracle_bench: {error}");
        std_process::exit(2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn arguments_need_the_required_flags_and_a_known_back_end() {
        let base = [
            "--backend",
            "spin",
            "--model",
            "m",
            "--atoms",
            "a",
            "--properties",
            "p",
            "--bench",
            "b",
        ];
        let args = parse_args(&strings(&base)).unwrap();
        assert_eq!(
            (args.one, args.two, args.seed, args.timeout),
            (40, 40, 1, 120)
        );
        let mut more = base.to_vec();
        more.extend(["--one", "3", "--seed", "9"]);
        let args = parse_args(&strings(&more)).unwrap();
        assert_eq!((args.one, args.seed), (3, 9));
        assert!(parse_args(&strings(&base[..8]))
            .unwrap_err()
            .contains("--bench"));
        let mut bad = base.to_vec();
        bad[1] = "z3";
        assert!(parse_args(&strings(&bad)).is_err());
        assert!(parse_args(&strings(&["--seed"]))
            .unwrap_err()
            .contains("needs a value"));
    }

    #[test]
    fn the_shuffle_is_a_seeded_permutation() {
        let mut a: Vec<u32> = (0..50).collect();
        let mut b = a.clone();
        Rng(7).shuffle(&mut a);
        Rng(7).shuffle(&mut b);
        assert_eq!(a, b);
        assert_ne!(a, (0..50).collect::<Vec<_>>());
        a.sort();
        assert_eq!(a, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn the_tier_is_the_counter_that_moved() {
        let before = Counters::default();
        let moved = |f: fn(&mut Counters)| {
            let mut after = before;
            f(&mut after);
            tier(&before, &after)
        };
        assert_eq!(moved(|c| c.memo_hits += 1), "memo");
        assert_eq!(moved(|c| c.lasso_hits += 1), "lasso");
        assert_eq!(moved(|c| c.implication_hits += 1), "implication");
        assert_eq!(moved(|c| c.backend_calls += 1), "backend");
    }

    #[test]
    fn fields_are_quoted_only_when_needed() {
        assert_eq!(
            row(&["a", "G (p, q)", "say \"x\"\nnow"]),
            "a,\"G (p, q)\",\"say \"\"x\"\" now\""
        );
    }
}
