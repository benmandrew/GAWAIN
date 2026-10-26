//! End-to-end tests of the model-counting similarity: `ltl2tgba` builds the
//! automata (`ltlf2dfa` under LTLf), and Ganak counts the labels too wide
//! for a truth table. Each expected score comes from a closed-form count of
//! the lassos (u, v) with |u| + |v| = [`BOUND`] whose word u v^ω is in the
//! language, or of the finite words of length 1 to [`BOUND`].
//!
//! Each test skips, with a reason on stderr, when a tool is not on `PATH`.

mod common;

use std::time::Duration;

use common::{have, parse};
use gawain::count::{similarity_from_logs, Counter, BOUND};
use gawain::ltl::{Formula, UnaryOp};
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
    let mut counter = Counter::new(&phi, Semantics::Infinite, LIMITS);
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
    // φ = a0 & … & a20 over 21 atoms: its label mentions all 21, past the
    // truth table's 20. φ fixes the first letter, so each split has
    // 2^(21 (k-1)) lassos, and the candidate a0 & … & a19, which φ
    // implies, has twice as many.
    let all = |n: usize| -> Formula {
        let body = (0..n)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join(" & ");
        parse(&body)
    };
    let phi = all(21);
    let mut counter = Counter::new(&phi, Semantics::Infinite, LIMITS);
    let found = counter.similarities(&[all(20)]);
    let narrow = splits(21.0 * (BOUND as f64 - 1.0));
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
        let mut counter = Counter::new(&parse(phi), Semantics::Infinite, LIMITS);
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
    let mut counter = Counter::new(&parse("G (req -> X ack)"), Semantics::Infinite, LIMITS);
    let found = counter.similarities(&[
        parse("G (req -> F ack)"),
        parse("G F req -> G F ack"),
        Formula::tt(),
    ]);
    assert!(1.0 > found[0], "{found:?}");
    assert!(found[0] > found[1], "{found:?}");
    assert!(found[1] > found[2], "{found:?}");
}

#[test]
fn rejections_match_closed_forms_over_one_atom() {
    if !have(
        &["ltl2tgba", "ltlf2dfa"],
        "rejections_match_closed_forms_over_one_atom",
    ) {
        return;
    }
    // φ = G a over the one atom a. Under LTL each split has 2^k lassos, of
    // which ¬G a has all but one and ¬a the 2^(k-1) that start with !a.
    // ¬G F a = F G !a needs a loop of !a only, so a split with |v| = j has
    // 2^(k-j) lassos, 2^k - 1 in all.
    let k = BOUND as i32;
    let candidates = [Formula::tt(), parse("G a"), parse("a"), parse("G F a")];
    let mut counter = Counter::new(&parse("G a"), Semantics::Infinite, LIMITS);
    let found = counter.rejections(&candidates);
    let not_phi = splits((2f64.powi(k) - 1.0).log2());
    let expected = [
        0.0,
        1.0,
        splits((k - 1) as f64) / not_phi,
        (2f64.powi(k) - 1.0).log2() / not_phi,
    ];
    for ((candidate, found), expected) in candidates.iter().zip(found).zip(expected) {
        assert!(
            (found - expected).abs() < 1e-12,
            "{candidate}: {found} against {expected}"
        );
    }
    // Under LTLf, of the 2^n words of length n, ¬G a has all but one and
    // ¬a half.
    let sum = |term: &dyn Fn(i32) -> f64| (1..=k).map(term).sum::<f64>().log2();
    let mut counter = Counter::new(&parse("G a"), Semantics::Finite, LIMITS);
    let found = counter.rejections(&[Formula::tt(), parse("a")]);
    let expected = sum(&|n| 2f64.powi(n - 1)) / sum(&|n| 2f64.powi(n) - 1.0);
    assert_eq!(found[0], 0.0);
    assert!((found[1] - expected).abs() < 1e-12, "{found:?} {expected}");
}

/// 1,200 pairs (ψ, χ) with ψ ⇒ χ, from the monotone rewrite of random
/// depth-4 formulas over three atoms, as in `tests/search.rs`. Each
/// implication holds under LTL, as Spot must prove. Under LTLf the rewrites
/// are not all monotone (weak `X a` does not imply `F a`), so only the
/// pairs Spot proves under `semantics` are kept.
fn implied_pairs(semantics: Semantics) -> Vec<(Formula, Formula)> {
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
    let proved = implications(&refs, semantics, LIMITS).expect("the batch answers");
    pairs
        .into_iter()
        .zip(proved)
        .filter(|&(_, proved)| proved)
        .map(|(pair, _)| pair)
        .collect()
}

#[test]
fn counts_are_monotone_under_implication() {
    let test = "counts_are_monotone_under_implication";
    if have(&["ltl2tgba", "ltlfilt"], test) {
        // ⊤ counts every lasso.
        let every = 3.0 * BOUND as f64 + (BOUND as f64).log2();
        assert_monotone(test, Semantics::Infinite, every);
    }
}

#[test]
fn finite_counts_are_monotone_under_ltlf_implication() {
    let test = "finite_counts_are_monotone_under_ltlf_implication";
    if have(&["ltlf2dfa", "ltlfilt"], test) {
        // ⊤ counts every word of length 1 to k over three atoms.
        let every = (1..=BOUND as i32).map(|n| 8f64.powi(n)).sum::<f64>().log2();
        assert_monotone(test, Semantics::Finite, every);
    }
}

/// Counts both sides of every implied pair under `semantics` and checks
/// that none exceeds its consequence, or `every`, the count of ⊤.
fn assert_monotone(test: &str, semantics: Semantics, every: f64) {
    let pairs = implied_pairs(semantics);
    // φ fixes the universe at the three atoms.
    let mut counter = Counter::new(&parse("a & b & c"), semantics, LIMITS);
    // Each pair, then each pair negated and swapped, since ψ ⇒ χ is
    // ¬χ ⇒ ¬ψ: the counts of the rejection strength.
    let not = |f: &Formula| Formula::unary(UnaryOp::Not, f.clone());
    let formulas: Vec<Formula> = pairs
        .iter()
        .flat_map(|(p, q)| [p.clone(), q.clone()])
        .chain(pairs.iter().flat_map(|(p, q)| [not(q), not(p)]))
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
    assert!((top - every).abs() < 1e-9, "⊤ counts {top}, not {every}");
    let above_top = logs.iter().filter(|&&l| l > top + 1e-9).count();
    let (positive, negated) = logs.split_at(2 * pairs.len());
    for (logs, what) in [
        (positive, "count(ψ) > count(χ)"),
        (negated, "count(¬χ) > count(¬ψ)"),
    ] {
        let violations: Vec<_> = pairs
            .iter()
            .zip(logs.chunks(2))
            .filter(|(_, l)| l[0] > l[1] + 1e-9)
            .collect();
        let strict = logs.chunks(2).filter(|l| l[0] < l[1] - 1e-9).count();
        eprintln!(
            "{test}: {} of {} pairs have {what}; {strict} pairs strictly ordered",
            violations.len(),
            pairs.len()
        );
        for ((p, q), l) in violations.iter().take(5) {
            eprintln!("  {p}  =>  {q}: {:.12} > {:.12}", l[0], l[1]);
        }
        assert!(violations.is_empty());
    }
    eprintln!("{test}: {above_top} counts exceed ⊤");
    assert_eq!(above_top, 0);
}

#[test]
fn finite_counts_match_closed_forms_and_differ_from_lasso_counts() {
    if !have(
        &["ltlf2dfa", "ltl2tgba"],
        "finite_counts_match_closed_forms",
    ) {
        return;
    }
    // Over the one atom a, with n the word length from 1 to k: every word
    // gives 2^n per length, a gives 2^(n-1), G a one word, F a all but one,
    // weak X a every word of length 1 and half the longer ones, strong
    // X[!] true every word but those of length 1, and weak X false only
    // those of length 1.
    let k = BOUND as i32;
    let pow = |n: i32| 2f64.powi(n);
    let sum = |from: i32, term: &dyn Fn(i32) -> f64| (from..=k).map(term).sum::<f64>().log2();
    let cases: [(&str, f64); 7] = [
        ("a | !a", sum(1, &|n| pow(n))),
        ("a", sum(1, &|n| pow(n - 1))),
        ("G a", (k as f64).log2()),
        ("F a", sum(1, &|n| pow(n) - 1.0)),
        (
            "X a",
            (2.0 + (2..=k).map(|n| pow(n - 1)).sum::<f64>()).log2(),
        ),
        ("X[!] (a | !a)", sum(2, &|n| pow(n))),
        ("X false", 1.0),
    ];
    let mut counter = Counter::new(&parse("a"), Semantics::Finite, LIMITS);
    let formulas: Vec<Formula> = cases.iter().map(|(f, _)| parse(f)).collect();
    let found = counter.log_counts(&formulas);
    for ((text, expected), found) in cases.iter().zip(&found) {
        let found = found.unwrap_or_else(|| panic!("no count for {text}"));
        assert!(
            (found - expected).abs() < 1e-9,
            "{text}: {found} against {expected}"
        );
    }
    // Under LTL, X[!] true is true, so the lasso count cannot tell them
    // apart, and weak X false is false.
    let mut lassos = Counter::new(&parse("a"), Semantics::Infinite, LIMITS);
    let infinite = lassos.log_counts(&[
        formulas[0].clone(),
        formulas[5].clone(),
        formulas[6].clone(),
    ]);
    assert_eq!(infinite[0], infinite[1]);
    assert_eq!(infinite[2], Some(f64::NEG_INFINITY));
}
