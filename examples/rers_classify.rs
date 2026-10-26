//! Checks φ and ¬φ with the explicit back end for the properties of one
//! RERS parallel Promela file, for the degeneracy filter of the 2018 track
//! (`bench/results/rers-2018-explicit.csv`).
//!
//! Usage: `cargo run --release --example rers_classify -- FILE.pml TIMEOUT
//! [N...]`. It explores the model once, then checks the `ltl pN` lines
//! numbered N, or all of them, each check bounded by TIMEOUT seconds. It
//! prints the exploration on stderr and one CSV line per property on
//! stdout: number, the verdict on φ and on ¬φ (`holds`, `violated` or
//! `unknown`), and the seconds both took.

use std::time::{Duration, Instant};

use gawain::ltl::Formula;
use gawain::oracle::explicit::Explicit;
use gawain::oracle::process::Limits;
use gawain::oracle::{Backend, Verdict};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [path, timeout, numbers @ ..] = &args[..] else {
        panic!("usage: rers_classify FILE.pml TIMEOUT [N...]");
    };
    let limits = Limits {
        timeout: Duration::from_secs(timeout.parse().expect("TIMEOUT")),
        memory_bytes: 8 << 30,
    };
    let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let start = Instant::now();
    let kripke = gawain::rers::Model::parse(&source)
        .and_then(|m| m.explore())
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    eprintln!(
        "{path}: {} states, {} transitions, {:.1} s",
        kripke.state_count(),
        kripke.transition_count(),
        start.elapsed().as_secs_f64()
    );
    let mut backend = Explicit::new(kripke, limits);
    println!("prop,phi,negphi,seconds");
    for line in source.lines() {
        let Some(rest) = line.strip_prefix("ltl p") else {
            continue;
        };
        let (number, body) = rest.split_once(char::is_whitespace).expect("ltl pN { … }");
        if !numbers.is_empty() && !numbers.iter().any(|n| n == number) {
            continue;
        }
        let body = body.trim().trim_start_matches('{').trim_end_matches('}');
        let phi: Formula = body.parse().unwrap_or_else(|e| panic!("p{number}: {e}"));
        let start = Instant::now();
        let mut verdict = |text: String| match backend.check(&text.parse().unwrap()) {
            Verdict::Holds => "holds".to_string(),
            Verdict::Fails(_) => "violated".to_string(),
            Verdict::Unknown(reason) => format!("unknown ({})", reason.replace(',', ";")),
        };
        let on_phi = verdict(phi.to_string());
        let on_neg = verdict(format!("!({phi})"));
        println!(
            "{number},{on_phi},{on_neg},{:.1}",
            start.elapsed().as_secs_f64()
        );
    }
}
