//! End-to-end tests: the batched Spot implication check gives the verdicts
//! of known pairs, agrees with one `ltlfilt` validity check per pair, and
//! reports its cost per pair.
//!
//! The per-pair reference asks whether `ψ -> φ` is equivalent to `true`.
//! Spot 2.15.1's `--imply` filter is not used there: it wrongly rejects
//! `ltlfilt -f false --imply=true`, which a mutant pair hit.
//!
//! Each test skips, with a reason on stderr, when `ltlfilt` is not on
//! `PATH`.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{have, mutant, parse, Rng};
use gawain::ltl::Formula;
use gawain::oracle::process::Limits;
use gawain::oracle::{implications, syntactic_implications, Semantics};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 2 << 30,
};
const ATOMS: &[&str] = &["a", "b", "c"];

fn batch(pairs: &[(Formula, Formula)]) -> Vec<bool> {
    let refs: Vec<_> = pairs.iter().map(|(p, q)| (p, q)).collect();
    implications(&refs, Semantics::Infinite, LIMITS).unwrap_or_else(|e| panic!("batch failed: {e}"))
}

/// Spot's verdict on one pair: is `ψ -> φ` valid?
fn imply_one(psi: &Formula, phi: &Formula) -> bool {
    let output = Command::new("timeout")
        .args(["60", "ltlfilt", "-q", "-f"])
        .arg(format!("({psi}) -> ({phi})"))
        .arg("--equivalent-to=true")
        .stdin(Stdio::null())
        .output()
        .expect("cannot run ltlfilt");
    match output.status.code() {
        Some(0) => true,
        Some(1) => false,
        code => panic!("ltlfilt on {psi} => {phi} exited {code:?}"),
    }
}

#[test]
fn known_pairs_get_their_verdicts() {
    if !have(&["ltlfilt"], "known_pairs_get_their_verdicts") {
        return;
    }
    let cases = [
        ("G a", "F a", true),
        ("F a", "G a", false),
        ("a & b", "a", true),
        ("a", "a | b", true),
        ("a", "b", false),
        ("a U b", "F b", true),
        ("a U b", "a W b", true),
        ("a W b", "a U b", false),
        ("G (a & b)", "G a & G b", true),
        ("F G a", "G F a", true),
        ("G F a", "F G a", false),
        ("false", "a", true),
        ("a", "true", true),
        ("true", "a", false),
        ("false", "true", true),
        (
            "G (\"x y\" -> X \"Grant\")",
            "G (\"x y\" -> F \"Grant\")",
            true,
        ),
    ];
    let pairs: Vec<_> = cases.iter().map(|(p, q, _)| (parse(p), parse(q))).collect();
    let proved = batch(&pairs);
    for ((psi, phi, expected), got) in cases.iter().zip(proved) {
        assert_eq!(got, *expected, "{psi} => {phi}");
    }
}

#[test]
fn a_timeout_proves_nothing() {
    if !have(&["ltlfilt"], "a_timeout_proves_nothing") {
        return;
    }
    let limits = Limits {
        timeout: Duration::from_millis(1),
        ..LIMITS
    };
    let pairs = vec![(parse("G a"), parse("F a")); 2000];
    let refs: Vec<_> = pairs.iter().map(|(p, q)| (p, q)).collect();
    assert!(implications(&refs, Semantics::Infinite, limits).is_err());
}

/// Random formulas and mutants, as in `tests/fingerprint.rs`.
fn mutant_pairs(n: usize, seed: u64) -> Vec<(Formula, Formula)> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|_| {
            let psi = common::formula(&mut rng, 4, ATOMS);
            let phi = mutant(&mut rng, &psi, 2, ATOMS);
            (psi, phi)
        })
        .collect()
}

#[test]
fn the_batch_agrees_with_one_call_per_pair() {
    if !have(&["ltlfilt"], "the_batch_agrees_with_one_call_per_pair") {
        return;
    }
    let pairs = mutant_pairs(200, 0x9E37_79B9_7F4A_7C15);
    let proved = batch(&pairs);
    for ((psi, phi), got) in pairs.iter().zip(&proved) {
        assert_eq!(*got, imply_one(psi, phi), "{psi} => {phi}");
    }
    let n_proved = proved.iter().filter(|&&p| p).count();
    assert!(n_proved > 10 && n_proved < 190, "{n_proved} of 200 proved");
}

#[test]
fn report_cost_per_pair() {
    if !have(&["ltlfilt"], "report_cost_per_pair") {
        return;
    }
    let pairs = mutant_pairs(2000, 0x2545_F491_4F6C_DD1D);
    let time = |pairs: &[(Formula, Formula)]| {
        let start = Instant::now();
        batch(pairs);
        start.elapsed()
    };
    let one = (0..5).map(|_| time(&pairs[..1])).min().expect("five runs");
    let all = (0..3).map(|_| time(&pairs)).min().expect("three runs");
    let per_pair = all.saturating_sub(one) / (pairs.len() as u32 - 1);
    let _ = writeln!(
        std::io::stderr(),
        "spot implication: one pair {one:?}, {} pairs {all:?}, {per_pair:?} per extra pair",
        pairs.len()
    );
}

#[test]
fn the_rewriting_rules_prove_only_what_the_complete_check_proves() {
    if !have(
        &["ltlfilt"],
        "the_rewriting_rules_prove_only_what_the_complete_check_proves",
    ) {
        return;
    }
    let pairs = mutant_pairs(200, 0x9E37_79B9_7F4A_7C15);
    let refs: Vec<_> = pairs.iter().map(|(p, q)| (p, q)).collect();
    let complete = batch(&pairs);
    let rules =
        syntactic_implications(&refs, Semantics::Infinite, LIMITS).expect("the batch answers");
    for ((psi, phi), (rule, full)) in pairs.iter().zip(rules.iter().zip(&complete)) {
        assert!(
            !rule || *full,
            "{psi} => {phi}: rules prove it, Spot does not"
        );
    }
    let n_rules = rules.iter().filter(|&&p| p).count();
    let n_full = complete.iter().filter(|&&p| p).count();
    assert!(n_rules > 10 && n_rules <= n_full, "{n_rules} of {n_full}");
}

/// A chain of 13 nested `U`, as in LMCS production-cell `p3`, with
/// `innermost` as the left operand of the innermost `U`.
fn long_chain(innermost: &str) -> Formula {
    let mut text = format!("(true U {innermost})");
    for i in 1..13 {
        text = format!("({text} U (a{i} & b{i}))");
    }
    parse(&text)
}

#[test]
fn the_rewriting_rules_prove_a_long_chain_at_once() {
    if !have(
        &["ltlfilt"],
        "the_rewriting_rules_prove_a_long_chain_at_once",
    ) {
        return;
    }
    // Spot's complete check ran past 60 s on such pairs.
    let strong = long_chain("(a0 & !b0)");
    let weak = long_chain("!b0");
    let start = Instant::now();
    let proved = syntactic_implications(
        &[(&strong, &weak), (&weak, &strong)],
        Semantics::Infinite,
        LIMITS,
    );
    assert_eq!(proved, Ok(vec![true, false]));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
}
