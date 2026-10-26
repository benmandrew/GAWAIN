//! End-to-end tests of the trace-set back end: the LTLf evaluator agrees
//! with Spot's `--from-ltlf` on random formulas and words, LTLf implication
//! differs from LTL implication where it should, and the `weaken` binary
//! repairs a small trace set.
//!
//! Each test skips, with a reason on stderr, when `ltlfilt` is missing.

mod common;

use std::io::Write;
use std::process::Command;
use std::time::Duration;

use common::{formula, have, parse, Rng};
use gawain::ltl::{BinaryOp, Formula, UnaryOp};
use gawain::oracle::process::Limits;
use gawain::oracle::{implications, Lasso, Semantics, Valuation};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};
const ATOMS: &[&str] = &["a", "b", "c"];
const CASES: usize = 300;

/// The formula whose only model over `ATOMS` is `word`: each position's
/// literals, then strong next to the rest, and weak `X false` at the last
/// position, which holds only there.
fn describe(word: &[Valuation]) -> Formula {
    let literals = |v: &Valuation| {
        ATOMS
            .iter()
            .map(|&a| {
                let atom = Formula::atom(a);
                if v.contains(a) {
                    atom
                } else {
                    Formula::unary(UnaryOp::Not, atom)
                }
            })
            .reduce(|l, r| Formula::binary(BinaryOp::And, l, r))
            .expect("there are atoms")
    };
    let not = |f| Formula::unary(UnaryOp::Not, f);
    let last = Formula::unary(UnaryOp::Next, Formula::ff());
    let (end, rest) = word.split_last().expect("a word has a position");
    let tail = Formula::binary(BinaryOp::And, literals(end), last);
    rest.iter().rev().fold(tail, |next, v| {
        let strong_next = not(Formula::unary(UnaryOp::Next, not(next)));
        Formula::binary(BinaryOp::And, literals(v), strong_next)
    })
}

#[test]
fn the_ltlf_evaluator_agrees_with_spot() {
    if !have(&["ltlfilt"], "the_ltlf_evaluator_agrees_with_spot") {
        return;
    }
    let mut rng = Rng(0x5eed_f1a7);
    let mut words = Vec::new();
    let mut formulas = Vec::new();
    for _ in 0..CASES {
        let length = 1 + rng.below(5);
        let word: Vec<Valuation> = (0..length)
            .map(|_| {
                ATOMS
                    .iter()
                    .filter(|_| rng.below(2) == 1)
                    .map(|&a| a.into())
                    .collect()
            })
            .collect();
        words.push(word);
        formulas.push(formula(&mut rng, 4, ATOMS));
    }
    let described: Vec<Formula> = words.iter().map(|w| describe(w)).collect();
    let pairs: Vec<(&Formula, &Formula)> = described.iter().zip(&formulas).collect();
    let spot = implications(&pairs, Semantics::Finite, LIMITS).expect("the batch answers");
    let mut disagreements = Vec::new();
    let mut held = 0;
    for ((word, phi), spot) in words.into_iter().zip(&formulas).zip(spot) {
        let ours = Lasso::finite(word.clone()).unwrap().satisfies(phi);
        held += usize::from(ours);
        if ours != spot {
            disagreements.push(format!("{phi} on {word:?}: ours {ours}, Spot {spot}"));
        }
    }
    let _ = writeln!(
        std::io::stderr(),
        "{held} of {CASES} words satisfy their formula"
    );
    assert!(disagreements.is_empty(), "{disagreements:#?}");
    assert!(held > CASES / 5 && held < CASES * 4 / 5, "{held} hold");
}

#[test]
fn finite_implication_is_not_infinite_implication() {
    if !have(
        &["ltlfilt"],
        "finite_implication_is_not_infinite_implication",
    ) {
        return;
    }
    let cases = [
        // Weak next holds at the last position, where F a may not.
        ("X a", "F a", true, false),
        // Both read "b at the last position" over finite words.
        ("G F b", "F G b", false, true),
        // Strong next fails on a one-position word.
        ("true", "X[!] true", true, false),
        ("G a", "a", true, true),
    ];
    for (psi, phi, infinite, finite) in cases {
        let (psi, phi) = (parse(psi), parse(phi));
        for (semantics, want) in [(Semantics::Infinite, infinite), (Semantics::Finite, finite)] {
            let got = implications(&[(&psi, &phi)], semantics, LIMITS).expect("answers");
            assert_eq!(got, [want], "{psi} => {phi} under {semantics:?}");
        }
    }
}

/// Every positive has b eventually after a, but not always the step after;
/// G (a -> X[!] b) fails, on the second positive only, and G (a -> F b) is
/// a weakening that holds. With weak X, G (a -> X b) does not imply
/// G (a -> F b): a at the last position satisfies the first and not the
/// second.
const SET: &str = "\
semantics finite
atoms a b
+ 10 01 00
+ 10 00 01
+ 00 10 01
+ 00 00
- 10 00 00
";

#[test]
fn weaken_repairs_a_trace_set() {
    if !have(&["ltlfilt"], "weaken_repairs_a_trace_set") {
        return;
    }
    let text = SET;
    let dir = std::env::temp_dir().join(format!("gawain-traces-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("set.trace");
    std::fs::write(&path, text).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_weaken"))
        .args(["--traces", path.to_str().unwrap()])
        .args(["--ltl", "G (a -> X[!] b)", "--seed", "1"])
        .args(["--generations", "15", "--population", "20"])
        .output()
        .expect("weaken runs");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _ = writeln!(std::io::stderr(), "{stdout}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let repairs: Vec<Formula> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("repair\t"))
        .map(|l| parse(l.rsplit('\t').next().unwrap()))
        .collect();
    assert!(!repairs.is_empty(), "no repair");
    let set = gawain::oracle::traces::TraceSet::parse(text).unwrap();
    let phi = parse("G (a -> X[!] b)");
    for repair in &repairs {
        assert!(
            set.positives().iter().all(|t| t.satisfies(repair)),
            "{repair} fails on a positive"
        );
        let proved = implications(&[(&phi, repair)], Semantics::Finite, LIMITS).unwrap();
        assert_eq!(proved, [true], "φ does not imply {repair}");
    }
}

#[test]
fn the_graded_search_grades_against_the_positives() {
    if !have(
        &["ltlfilt", "ltl2tgba"],
        "the_graded_search_grades_against_the_positives",
    ) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("gawain-graded-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("set.trace");
    std::fs::write(&path, SET).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_search_bench"))
        .args(["--backend", "traces", "--model", path.to_str().unwrap()])
        .args(["--ltl", "G (a -> X[!] b)", "--seed", "1"])
        .args(["--status", "graded"])
        .args(["--generations", "15", "--population", "20"])
        .output()
        .expect("search_bench runs");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _ = writeln!(std::io::stderr(), "{stdout}");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The grader reads all four positives, one of which violates φ, and
    // the oracle stores no lasso of its own.
    for field in [
        "\"lassos\":4,",
        "\"refuting\":1,",
        "\"lasso_hits\":0,",
        "\"spot_calls\":0,",
    ] {
        assert!(stdout.contains(field), "no {field}");
    }
    assert!(!stdout.contains("\"rescores\":0,"), "never graded");
    assert!(!stdout.contains("\"repairs\":[]"), "no repair");
}
