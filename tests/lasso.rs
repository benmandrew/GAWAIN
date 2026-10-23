//! End-to-end test: `Lasso::satisfies` agrees with Spot on random formulas
//! and random lassos.
//!
//! Spot decides each case by translating the formula with `ltl2tgba` and
//! asking `autfilt --accept-word` whether the automaton accepts the lasso.
//! The test skips, with a reason on stderr, when either tool is not on
//! `PATH`. Every tool call runs under `timeout`.

use std::io::Write;
use std::process::{Command, Stdio};

use aragon::ltl::{BinaryOp, Formula, UnaryOp};
use aragon::oracle::{Lasso, Valuation};

const TIMEOUT_SECS: &str = "60";
const CASES: usize = 300;
const ATOMS: &[&str] = &["a", "b", "c"];

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// Marsaglia's xorshift64, enough for a fixed-seed test.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

const UNARY: &[UnaryOp] = &[
    UnaryOp::Not,
    UnaryOp::Next,
    UnaryOp::Eventually,
    UnaryOp::Globally,
];

const BINARY: &[BinaryOp] = &[
    BinaryOp::And,
    BinaryOp::Or,
    BinaryOp::Implies,
    BinaryOp::Iff,
    BinaryOp::Until,
    BinaryOp::WeakUntil,
    BinaryOp::Release,
];

fn formula(rng: &mut Rng, depth: usize) -> Formula {
    if depth == 0 || rng.below(4) == 0 {
        return match rng.below(8) {
            0 => Formula::tt(),
            1 => Formula::ff(),
            _ => Formula::atom(ATOMS[rng.below(ATOMS.len())]),
        };
    }
    if rng.below(3) == 0 {
        let op = UNARY[rng.below(UNARY.len())];
        Formula::unary(op, formula(rng, depth - 1))
    } else {
        let op = BINARY[rng.below(BINARY.len())];
        Formula::binary(op, formula(rng, depth - 1), formula(rng, depth - 1))
    }
}

fn positions(rng: &mut Rng, n: usize) -> Vec<Valuation> {
    (0..n)
        .map(|_| {
            ATOMS
                .iter()
                .filter(|_| rng.below(2) == 0)
                .map(|&atom| atom.into())
                .collect()
        })
        .collect()
}

/// One position as a Spot letter, naming every atom: `a&!b&!c`.
fn letter(valuation: &Valuation) -> String {
    let literals: Vec<String> = ATOMS
        .iter()
        .map(|&atom| {
            if valuation.contains(atom) {
                atom.to_string()
            } else {
                format!("!{atom}")
            }
        })
        .collect();
    literals.join("&")
}

/// The lasso in `autfilt --accept-word` syntax: `a&!b&!c; cycle{!a&b&!c}`.
fn word(lasso: &Lasso) -> String {
    let letters = |vs: &[Valuation]| vs.iter().map(letter).collect::<Vec<_>>().join("; ");
    let cycle = format!("cycle{{{}}}", letters(lasso.cycle()));
    if lasso.prefix().is_empty() {
        cycle
    } else {
        format!("{}; {cycle}", letters(lasso.prefix()))
    }
}

/// Runs `tool args` under `timeout`, with `input` on stdin; returns the exit
/// code and stdout.
fn run(tool: &str, args: &[&str], input: &[u8]) -> (Option<i32>, Vec<u8>) {
    let mut child = Command::new("timeout")
        .arg(TIMEOUT_SECS)
        .arg(tool)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run {tool}: {e}"));
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for tool");
    let code = output.status.code();
    assert_ne!(code, Some(124), "{tool} {args:?} timed out");
    assert!(
        matches!(code, Some(0 | 1)),
        "{tool} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (code, output.stdout)
}

/// Spot's verdict: does the automaton for `formula` accept `lasso`?
fn spot_accepts(formula: &Formula, lasso: &Lasso) -> bool {
    let (code, automaton) = run("ltl2tgba", &["-F", "-"], format!("{formula}\n").as_bytes());
    assert_eq!(code, Some(0), "ltl2tgba rejected {formula}");
    let accept = format!("--accept-word={}", word(lasso));
    // autfilt -q exits 0 when the automaton passes the filter, 1 otherwise.
    let (code, _) = run("autfilt", &["-q", &accept], &automaton);
    code == Some(0)
}

#[test]
fn satisfies_agrees_with_spot() {
    let missing: Vec<&str> = ["ltl2tgba", "autfilt"]
        .into_iter()
        .filter(|t| !on_path(t))
        .collect();
    if !missing.is_empty() {
        let _ = writeln!(
            std::io::stderr(),
            "skipping satisfies_agrees_with_spot: {} not on PATH",
            missing.join(", ")
        );
        return;
    }
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut accepted = 0;
    for _ in 0..CASES {
        let f = formula(&mut rng, 4);
        let prefix_len = rng.below(4);
        let cycle_len = 1 + rng.below(4);
        let lasso = Lasso::new(
            positions(&mut rng, prefix_len),
            positions(&mut rng, cycle_len),
        )
        .expect("non-empty cycle");
        let expected = spot_accepts(&f, &lasso);
        assert_eq!(
            lasso.satisfies(&f),
            expected,
            "{f} on {}: Spot says {expected}",
            word(&lasso)
        );
        accepted += usize::from(expected);
    }
    // Guards against a generator that only yields one verdict.
    assert!(
        accepted > CASES / 10 && accepted < CASES * 9 / 10,
        "{accepted} of {CASES} accepted"
    );
}
