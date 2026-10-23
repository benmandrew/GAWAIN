//! End-to-end tests of the model-counting similarity: `ltl2tgba` builds the
//! automata, and Ganak counts the labels too wide for a truth table. Each
//! expected score comes from a closed-form count of the lassos (u, v) with
//! |u| + |v| = [`BOUND`] whose word u v^ω is in the language.
//!
//! Each test skips, with a reason on stderr, when a tool is not on `PATH`.

mod common;

use std::time::Duration;

use common::{have, parse};
use gawain::count::{similarity_from_logs, Counter, BOUND};
use gawain::ltl::Formula;
use gawain::oracle::process::Limits;
use gawain::oracle::{implications, Semantics};
use gawain::search::{monotone_rewrite, Direction, Rng};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 4 << 30,
};

/// log2 of k times `per_split`, the log2 count of each of the k splits.
fn splits(per_split: f64) -> f64 {
    per_split + (BOUND as f64).log2()
}

#[test]
fn counts_match_closed_forms_over_a_truth_table() {
    if !have(
        &["ltl2tgba"],
        "counts_match_closed_forms_over_a_truth_table",
    ) {
        return;
    }
    // Over two atoms, each split of G a has the 2^k lassos with a in
    // every letter, G a | G b has 2^(k+1) - 1, and G a implies G a | G b,
    // so it is the shared count. F a | F b misses only the one lasso with
    // neither atom anywhere, so it has 4^k - 1.
    let k = BOUND as i32;
    let phi = parse("G a");
    let mut counter = Counter::new(&phi, LIMITS);
    let found = counter.similarities(&[parse("G a | G b"), phi.clone(), parse("F a | F b")]);
    let ga = splits(k as f64);
    let either = splits((2f64.powi(k + 1) - 1.0).log2());
    let expected = similarity_from_logs(ga, either, ga);
    assert!((found[0] - expected).abs() < 1e-9, "{found:?} {expected}");
    assert!(found[0] < 1.0);
    assert_eq!(found[1], 1.0);
    let some = splits((4f64.powi(k) - 1.0).log2());
    let expected = similarity_from_logs(ga, some, ga);
    assert!((found[2] - expected).abs() < 1e-9, "{found:?} {expected}");
    let counts = counter.counts();
    assert_eq!((counts.translations, counts.formulas), (1, 5));
    assert_eq!((counts.ganak_calls, counts.failures), (0, 0));
    // Asked again, it is a cache hit with no exec.
    let again = counter.similarities(&[parse("G b | G a")]);
    assert_eq!(again, [found[0]]);
    assert_eq!(counter.counts().translations, 1);
}

#[test]
fn ganak_counts_a_label_wider_than_the_truth_table() {
    if !have(
        &["ltl2tgba", "ganak"],
        "ganak_counts_a_label_wider_than_the_truth_table",
    ) {
        return;
    }
    // φ = a0 & … & a16 over 17 atoms: its label mentions all 17, past the
    // truth table's 16. φ fixes the first letter, so each split has
    // 2^(17 (k-1)) lassos, and the candidate a0 & … & a15, which φ
    // implies, has twice as many.
    let all = |n: usize| -> Formula {
        let body = (0..n)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(" & ");
        parse(&body)
    };
    let phi = all(17);
    let mut counter = Counter::new(&phi, LIMITS);
    let found = counter.similarities(&[all(16)]);
    let narrow = splits(17.0 * (BOUND as f64 - 1.0));
    let expected = similarity_from_logs(narrow, narrow + 1.0, narrow);
    let counts = counter.counts();
    assert_eq!(counts.failures, 0, "{counts:?}");
    assert!(counts.ganak_calls >= 1, "{counts:?}");
    assert!((found[0] - expected).abs() < 1e-9, "{found:?} {expected}");
}

#[test]
fn equivalent_formulas_score_one() {
    if !have(&["ltl2tgba"], "equivalent_formulas_score_one") {
        return;
    }
    for (phi, same) in [
        ("a U b", "b | a & X (a U b)"),
        ("G a", "a & X G a"),
        ("F G a", "F G a & G F a"),
        ("a W b", "a U b | G a"),
        ("G (a -> F b)", "G (!a | F b) & G F (b | !a)"),
    ] {
        let mut counter = Counter::new(&parse(phi), LIMITS);
        let found = counter.similarities(&[parse(same)]);
        assert!(
            (found[0] - 1.0).abs() < 1e-9,
            "{phi} against {same}: {found:?}"
        );
    }
}

#[test]
fn liveness_weakenings_score_between_phi_and_top() {
    if !have(
        &["ltl2tgba"],
        "liveness_weakenings_score_between_phi_and_top",
    ) {
        return;
    }
    // On ack.smv, G(req -> F ack) weakens φ to liveness, and G F req ->
    // G F ack weakens it further. A finite prefix cannot tell either from ⊤,
    // but a lasso can.
    let mut counter = Counter::new(&parse("G (req -> X ack)"), LIMITS);
    let found = counter.similarities(&[
        parse("G (req -> F ack)"),
        parse("G F req -> G F ack"),
        Formula::tt(),
    ]);
    assert!(1.0 > found[0], "{found:?}");
    assert!(found[0] > found[1], "{found:?}");
    assert!(found[1] > found[2], "{found:?}");
}

/// 1,200 pairs (ψ, χ) with ψ ⇒ χ, from the monotone rewrite of random
/// depth-4 formulas over three atoms, as in `tests/search.rs`, each
/// implication proved by Spot.
fn implied_pairs() -> Vec<(Formula, Formula)> {
    let names = ["a", "b", "c"];
    let atoms: Vec<std::sync::Arc<str>> = names.iter().map(|&n| n.into()).collect();
    let mut gen = common::Rng(0x005E_ED0F_0DD5);
    let mut rng = Rng::new(17);
    let mut pairs = Vec::new();
    for i in 0..1200 {
        let subject = common::formula(&mut gen, 4, &names);
        if i % 2 == 0 {
            let child = monotone_rewrite(&subject, Direction::Weaken, &atoms, &mut rng);
            pairs.push((subject, child));
        } else {
            let child = monotone_rewrite(&subject, Direction::Strengthen, &atoms, &mut rng);
            pairs.push((child, subject));
        }
    }
    let refs: Vec<_> = pairs.iter().map(|(p, q)| (p, q)).collect();
    let proved = implications(&refs, Semantics::Infinite, LIMITS).expect("the batch answers");
    assert!(proved.iter().all(|&p| p), "a rewrite moved the wrong way");
    pairs
}

#[test]
fn counts_are_monotone_under_implication() {
    let test = "counts_are_monotone_under_implication";
    if !have(&["ltl2tgba", "ltlfilt"], test) {
        return;
    }
    let pairs = implied_pairs();
    // φ fixes the universe at the three atoms.
    let mut counter = Counter::new(&parse("a & b & c"), LIMITS);
    let formulas: Vec<Formula> = pairs
        .iter()
        .flat_map(|(p, q)| [p.clone(), q.clone()])
        .chain([Formula::tt()])
        .collect();
    let start = std::time::Instant::now();
    let logs = counter.log_counts(&formulas);
    eprintln!(
        "{test}: {} counts in {:.2?}",
        formulas.len(),
        start.elapsed()
    );
    assert!(logs.iter().all(Option::is_some), "a count failed");
    let logs: Vec<f64> = logs.into_iter().map(Option::unwrap).collect();
    let (top, logs) = logs.split_last().unwrap();
    let top = *top;
    let every = 3.0 * BOUND as f64 + (BOUND as f64).log2();
    assert!((top - every).abs() < 1e-9, "⊤ counts every lasso: {top}");
    let above_top = logs.iter().filter(|&&l| l > top + 1e-9).count();
    let violations: Vec<_> = pairs
        .iter()
        .zip(logs.chunks(2))
        .filter(|(_, l)| l[0] > l[1] + 1e-9)
        .collect();
    let strict = logs.chunks(2).filter(|l| l[0] < l[1] - 1e-9).count();
    eprintln!(
        "{test}: {} of {} pairs have count(ψ) > count(χ); {above_top} counts exceed ⊤; {strict} pairs strictly ordered",
        violations.len(),
        pairs.len()
    );
    for ((p, q), l) in violations.iter().take(5) {
        eprintln!("  {p}  =>  {q}: {:.12} > {:.12}", l[0], l[1]);
    }
    assert!(violations.is_empty());
    assert_eq!(above_top, 0);
}
