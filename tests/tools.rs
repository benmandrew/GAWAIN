//! End-to-end tests: Spot, SPIN and nuXmv read what the printers write, and
//! the nuXmv back end answers queries on a real model.
//!
//! Each test skips, with a reason on stderr, when a tool it needs is not on
//! `PATH`. Every tool call runs under `timeout`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use std::time::Duration;

use aragon::ltl::Formula;
use aragon::oracle::nuxmv::Nuxmv;
use aragon::oracle::process::Limits;
use aragon::oracle::{Backend, Lasso, Valuation, Verdict};

const TIMEOUT_SECS: &str = "60";

/// Spot syntax, covering every operator, both associativities and the
/// precedence levels the printers have to respect.
const SAMPLES: &[&str] = &[
    "G (req -> F grant)",
    "a & b U c | !d",
    "(a W b) W c",
    "a -> b -> c <-> d",
    "!(a R X b) U G F c",
    "X X a & (true W false)",
    "(a U b) U (c U d)",
    "a <-> b <-> c",
    "G !a W (b & X c)",
    "F G (a | !b) R (c -> X d)",
    "false U (a W !a)",
];

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

// Written to stderr directly, which the test harness does not capture, so a
// skip is visible in a normal `cargo test` run.
fn have(tools: &[&str], test: &str) -> bool {
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !on_path(t)).collect();
    if missing.is_empty() {
        return true;
    }
    let _ = writeln!(
        std::io::stderr(),
        "skipping {test}: {} not on PATH",
        missing.join(", ")
    );
    false
}

fn run(tool: &str, args: &[&str]) -> Output {
    let output = Command::new("timeout")
        .arg(TIMEOUT_SECS)
        .arg(tool)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {tool}: {e}"));
    assert_ne!(output.status.code(), Some(124), "{tool} {args:?} timed out");
    output
}

fn scratch(name: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(name)
}

fn parse(text: &str) -> Formula {
    text.parse()
        .unwrap_or_else(|e| panic!("parse {text:?}: {e}"))
}

fn assert_spot_equivalent(printed: &str, original: &str) {
    let output = run(
        "ltlfilt",
        &["-q", "-f", printed, "--equivalent-to", original],
    );
    assert!(
        output.status.success(),
        "Spot: {printed:?} is not equivalent to {original:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn spot_reads_every_printing_as_the_input() {
    if !have(&["ltlfilt"], "spot_reads_every_printing_as_the_input") {
        return;
    }
    // SPIN's and nuXmv's spellings are Spot syntax too, so this also checks
    // the W expansion is an equivalence.
    for text in SAMPLES {
        let formula = parse(text);
        assert_spot_equivalent(&formula.to_string(), text);
        assert_spot_equivalent(&formula.to_spin(), text);
        assert_spot_equivalent(&formula.to_nuxmv(), text);
    }
    let quoted = "G(\"Grant\" -> F \"x y\")";
    let printed = parse(quoted).to_string();
    assert_eq!(printed, quoted);
    assert_spot_equivalent(&printed, "G(\"Grant\" -> F\"x y\")");
}

#[test]
fn spin_never_claim_matches_spot() {
    let test = "spin_never_claim_matches_spot";
    if !have(&["spin", "ltl2tgba", "autfilt"], test) {
        return;
    }
    for (i, text) in SAMPLES.iter().enumerate() {
        let formula = parse(text);
        let negated = format!("!({})", formula.to_spin());
        let never = run("spin", &["-f", &negated]);
        let claim = String::from_utf8_lossy(&never.stdout);
        assert!(
            never.status.success() && claim.contains("never"),
            "spin rejected {negated:?}: {claim}{}",
            String::from_utf8_lossy(&never.stderr)
        );
        let claim_path = scratch(&format!("claim{i}.never"));
        std::fs::write(&claim_path, claim.as_bytes()).expect("write never claim");

        let reference = run("ltl2tgba", &[&format!("!({text})")]);
        assert!(reference.status.success(), "ltl2tgba failed on {text:?}");
        let reference_path = scratch(&format!("reference{i}.hoa"));
        std::fs::write(&reference_path, &reference.stdout).expect("write automaton");

        let equivalent = format!("--equivalent-to={}", reference_path.display());
        let check = run(
            "autfilt",
            &["-q", claim_path.to_str().expect("utf-8 path"), &equivalent],
        );
        assert!(
            check.status.success(),
            "SPIN's never claim for {negated:?} differs from Spot's for !({text})"
        );
    }
}

/// x alternates from false; y turns true one step after x first does and
/// stays true. So x, y run (F,F) (T,F) (F,T) (T,T) (F,T) ...
const MODEL: &str = "MODULE main
VAR x : boolean; y : boolean;
ASSIGN
  init(x) := FALSE; next(x) := !x;
  init(y) := FALSE; next(y) := y | x;
";

const VERDICTS: &[(&str, bool)] = &[
    ("G F x", true),
    ("G x", false),
    ("F G y", true),
    ("X x & X X y", true),
    ("!y W x", true),
    ("x W y", false),
    ("true W false", true),
    ("G (y -> X y)", true),
    ("x R !y", true),
    ("y R x", false),
    ("X x U y", false),
    ("!x U y", false),
];

#[test]
fn nuxmv_gives_known_verdicts() {
    if !have(&["nuXmv"], "nuxmv_gives_known_verdicts") {
        return;
    }
    // One run per formula: nuXmv does not report specifications in file
    // order.
    for (i, (text, expected)) in VERDICTS.iter().enumerate() {
        let spec = parse(text).to_nuxmv();
        let path = scratch(&format!("verdict{i}.smv"));
        std::fs::write(&path, format!("{MODEL}LTLSPEC {spec}\n")).expect("write model");
        let output = run("nuXmv", &[path.to_str().expect("utf-8 path")]);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let verdicts: Vec<bool> = stdout
            .lines()
            .filter(|line| line.starts_with("-- specification"))
            .map(|line| line.trim_end().ends_with("is true"))
            .collect();
        assert_eq!(
            verdicts,
            [*expected],
            "nuXmv on {spec:?}:\n{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The same run as `MODEL`, with a DEFINE and an integer the formulas do not
/// mention, so a lasso must report atoms beyond the formula's.
const INTERFACE_MODEL: &str = "MODULE main
VAR x : boolean; y : boolean; n : 0..3;
DEFINE both := x & y;
ASSIGN
  init(x) := FALSE; next(x) := !x;
  init(y) := FALSE; next(y) := y | x;
  init(n) := 0; next(n) := case n < 3 : n + 1; TRUE : 0; esac;
";

/// x is false at the start and true forever after.
const LATCH_MODEL: &str = "MODULE main
VAR x : boolean;
ASSIGN init(x) := FALSE; next(x) := TRUE;
";

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 1 << 30,
};

fn nuxmv(name: &str, model: &str, atoms: &[&str], limits: Limits) -> Nuxmv {
    let path = scratch(name);
    std::fs::write(&path, model).expect("write model");
    Nuxmv::new(path, atoms, limits)
}

fn check(backend: &mut Nuxmv, text: &str) -> Verdict {
    backend.check(&parse(text))
}

/// The first `n` positions of `prefix · cycle^ω`, as sorted atom names.
fn unroll(lasso: &Lasso, n: usize) -> Vec<Vec<String>> {
    let names = |v: &Valuation| v.iter().map(|a| a.to_string()).collect();
    lasso
        .prefix()
        .iter()
        .chain(lasso.cycle().iter().cycle())
        .take(n)
        .map(names)
        .collect()
}

fn expect_lasso(verdict: Verdict) -> Lasso {
    match verdict {
        Verdict::Fails(lasso) => lasso,
        other => panic!("expected a counterexample, got {other:?}"),
    }
}

#[test]
fn nuxmv_backend_proves_a_holding_property() {
    if !have(&["nuXmv"], "nuxmv_backend_proves_a_holding_property") {
        return;
    }
    let mut backend = nuxmv("holds.smv", INTERFACE_MODEL, &["x", "y", "both"], LIMITS);
    assert_eq!(check(&mut backend, "G F both"), Verdict::Holds);
    assert_eq!(check(&mut backend, "F G y"), Verdict::Holds);
}

#[test]
fn nuxmv_backend_returns_the_run_that_fails() {
    if !have(&["nuXmv"], "nuxmv_backend_returns_the_run_that_fails") {
        return;
    }
    let mut backend = nuxmv("fails.smv", INTERFACE_MODEL, &["x", "y", "both"], LIMITS);
    let lasso = expect_lasso(check(&mut backend, "G !(x & y)"));
    // The model has one run: x alternates from false, y holds from step 2.
    let expected: Vec<Vec<String>> = (0..12)
        .map(|i| {
            let x = i % 2 == 1;
            let y = i >= 2;
            [("both", x && y), ("x", x), ("y", y)]
                .iter()
                .filter(|(_, holds)| *holds)
                .map(|(name, _)| name.to_string())
                .collect()
        })
        .collect();
    assert_eq!(unroll(&lasso, 12), expected, "{lasso:?}");
}

#[test]
fn nuxmv_backend_keeps_the_prefix() {
    if !have(&["nuXmv"], "nuxmv_backend_keeps_the_prefix") {
        return;
    }
    let mut backend = nuxmv("prefix.smv", LATCH_MODEL, &["x"], LIMITS);
    let lasso = expect_lasso(check(&mut backend, "G !x"));
    assert!(!lasso.prefix().is_empty(), "{lasso:?}");
    assert!(lasso.prefix()[0].is_empty(), "{lasso:?}");
    assert!(lasso.cycle().iter().all(|v| v.contains("x")), "{lasso:?}");
    let mut expected = vec![vec![]];
    expected.extend(std::iter::repeat_n(vec!["x".to_string()], 7));
    assert_eq!(unroll(&lasso, 8), expected);
}

#[test]
fn nuxmv_backend_reports_unknown_on_timeout_or_error() {
    if !have(
        &["nuXmv"],
        "nuxmv_backend_reports_unknown_on_timeout_or_error",
    ) {
        return;
    }
    let hurried = Limits {
        timeout: Duration::from_millis(1),
        ..LIMITS
    };
    let mut backend = nuxmv("timeout.smv", INTERFACE_MODEL, &["x"], hurried);
    match check(&mut backend, "G F x") {
        Verdict::Unknown(reason) => assert!(reason.contains("timed out"), "{reason}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
    let mut backend = nuxmv("undefined.smv", INTERFACE_MODEL, &["x"], LIMITS);
    match check(&mut backend, "G undefined") {
        Verdict::Unknown(reason) => assert!(reason.contains("undefined"), "{reason}"),
        other => panic!("expected an error, got {other:?}"),
    }
}
