//! Checks MCC LTL formulas of one P/T instance with the TAPAAL back end,
//! and the negation of each, for the subset selection in `bench/notes/mcc.md`.
//!
//! ```text
//! mcc_check --instance DIR --formulas ID,ID,... [--timeout SECS] [--memory-mb MB]
//! ```
//!
//! `DIR` is an unpacked instance archive with `model.pnml` and the
//! `LTLCardinality.xml` and `LTLFireability.xml` files; each `ID` is a full
//! property id such as `Model-PT-01-LTLFireability-03`. For each formula φ
//! the back end checks φ and then ¬φ, the degeneracy filter of `PLAN.md`.
//! One CSV row per formula goes to standard output, with the verdicts as
//! `TRUE`, `FALSE` or `UNKNOWN`, the wall time of each check, the lasso's
//! prefix and cycle lengths for φ, and the reason for an unknown verdict.
//! A failing verdict has already been replayed and checked against its
//! formula by the back end.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use gawain::ltl::{Formula, UnaryOp};
use gawain::mcc::{properties, Net, Property};
use gawain::oracle::process::Limits;
use gawain::oracle::tapaal::Tapaal;
use gawain::oracle::{Backend, Verdict};

const USAGE: &str =
    "usage: mcc_check --instance DIR --formulas ID,ID,... [--timeout SECS] [--memory-mb MB]";

const HEADER: &str =
    "instance,formula,atoms,verdict,seconds,prefix,cycle,neg_verdict,neg_seconds,reason";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mcc_check: {e}\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), String> {
    let mut instance = None;
    let mut formulas = None;
    let mut timeout = 60;
    let mut memory_mb = 8192;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or(format!("{flag} needs a value"))?;
        let number = || {
            value
                .parse::<u64>()
                .map_err(|e| format!("{flag} {value}: {e}"))
        };
        match flag.as_str() {
            "--instance" => instance = Some(PathBuf::from(&value)),
            "--formulas" => formulas = Some(value.split(',').map(String::from).collect::<Vec<_>>()),
            "--timeout" => timeout = number()?,
            "--memory-mb" => memory_mb = number()?,
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    let dir = instance.ok_or("--instance is required")?;
    let formulas = formulas.ok_or("--formulas is required")?;
    let limits = Limits {
        timeout: Duration::from_secs(timeout),
        memory_bytes: memory_mb << 20,
    };
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("bad instance directory")?
        .to_owned();
    let model = dir.join("model.pnml");
    let read =
        |path: &PathBuf| fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()));
    let net = Net::parse(&read(&model)?)?;
    let mut by_exam: HashMap<String, Vec<Property>> = HashMap::new();
    println!("{HEADER}");
    for id in formulas {
        let exam = id
            .rsplit('-')
            .nth(1)
            .ok_or(format!("{id}: not an MCC property id"))?
            .to_owned();
        if !by_exam.contains_key(&exam) {
            let props = properties(&read(&dir.join(format!("{exam}.xml")))?, &net)?;
            by_exam.insert(exam.clone(), props);
        }
        let Some(property) = by_exam[&exam].iter().find(|p| p.id == id) else {
            return Err(format!("{id}: not in {exam}.xml"));
        };
        let mut backend = Tapaal::new(&model, net.clone(), property.atoms.clone(), limits);
        let (verdict, seconds) = timed(&mut backend, &property.formula);
        let negation = Formula::unary(UnaryOp::Not, property.formula.clone());
        let (neg_verdict, neg_seconds) = timed(&mut backend, &negation);
        let (prefix, cycle) = match &verdict {
            Verdict::Fails(lasso) => (
                lasso.prefix().len().to_string(),
                lasso.cycle().len().to_string(),
            ),
            _ => (String::new(), String::new()),
        };
        let reason: Vec<&str> = [&verdict, &neg_verdict]
            .into_iter()
            .filter_map(|v| match v {
                Verdict::Unknown(r) => Some(r.as_str()),
                _ => None,
            })
            .collect();
        let reason = reason.join(" | ").replace(['\n', ','], " ");
        println!(
            "{name},{id},{},{},{seconds:.3},{prefix},{cycle},{},{neg_seconds:.3},{reason}",
            property.atoms.len(),
            label(&verdict),
            label(&neg_verdict),
        );
    }
    Ok(())
}

fn timed(backend: &mut Tapaal, formula: &Formula) -> (Verdict, f64) {
    let start = Instant::now();
    let verdict = backend.check(formula);
    (verdict, start.elapsed().as_secs_f64())
}

fn label(verdict: &Verdict) -> &'static str {
    match verdict {
        Verdict::Holds => "TRUE",
        Verdict::Fails(_) => "FALSE",
        Verdict::Unknown(_) => "UNKNOWN",
    }
}
