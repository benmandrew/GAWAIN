//! End-to-end test: the fingerprint prefilter never rules out an implication
//! that Spot proves, and reports how many of Spot's refutations it finds.
//!
//! Each pair is a random formula ψ and a mutant φ' of it, made by replacing
//! one random subformula with a random formula, as a GA offspring would be.
//! Spot decides ψ ⇒ φ' with `ltlfilt -f ψ --imply=φ'`. The test skips, with
//! a reason on stderr, when `ltlfilt` is not on `PATH`. Every tool call runs
//! under `timeout`.

use std::io::Write;
use std::process::{Command, Stdio};

use aragon::ltl::{BinaryOp, Formula, Node, UnaryOp};
use aragon::oracle::Words;

const TIMEOUT_SECS: &str = "60";
const PAIRS: usize = 400;
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

/// `f` with its subformula at pre-order index `target` replaced by `new`.
fn replace(f: &Formula, target: usize, new: &Formula) -> Formula {
    if target == 0 {
        return new.clone();
    }
    match f.node() {
        Node::Unary(op, child) => Formula::unary(*op, replace(child, target - 1, new)),
        Node::Binary(op, lhs, rhs) => {
            let n = lhs.n_subformulae();
            if target <= n {
                Formula::binary(*op, replace(lhs, target - 1, new), rhs.clone())
            } else {
                Formula::binary(*op, lhs.clone(), replace(rhs, target - 1 - n, new))
            }
        }
        _ => unreachable!("a leaf has only index 0"),
    }
}

/// Spot's verdict on `psi -> phi`, valid or not.
fn spot_implies(psi: &Formula, phi: &Formula) -> bool {
    let output = Command::new("timeout")
        .arg(TIMEOUT_SECS)
        .arg("ltlfilt")
        .args(["-q", "-f", &psi.to_string(), &format!("--imply={phi}")])
        .stdin(Stdio::null())
        .output()
        .expect("cannot run ltlfilt");
    let code = output.status.code();
    assert_ne!(code, Some(124), "ltlfilt timed out on {psi} => {phi}");
    // ltlfilt -q exits 0 when the formula passes the filter, 1 otherwise.
    assert!(
        matches!(code, Some(0 | 1)),
        "ltlfilt failed on {psi} => {phi}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    code == Some(0)
}

#[test]
fn fingerprint_is_sound_against_spot() {
    if !on_path("ltlfilt") {
        let _ = writeln!(
            std::io::stderr(),
            "skipping fingerprint_is_sound_against_spot: ltlfilt not on PATH"
        );
        return;
    }
    let words = Words::sample(ATOMS.iter().copied(), 0);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut implied, mut refuted, mut caught) = (0, 0, 0);
    for _ in 0..PAIRS {
        let psi = formula(&mut rng, 4);
        let target = rng.below(psi.n_subformulae());
        let phi = replace(&psi, target, &formula(&mut rng, 2));
        let may = words.fingerprint(&psi).may_imply(&words.fingerprint(&phi));
        if spot_implies(&psi, &phi) {
            assert!(
                may,
                "the fingerprint ruled out {psi} => {phi}, which Spot proves"
            );
            implied += 1;
        } else {
            refuted += 1;
            caught += usize::from(!may);
        }
    }
    let _ = writeln!(
        std::io::stderr(),
        "fingerprint: {implied} pairs implied; refuted {caught} of {refuted} \
         Spot-refuted pairs ({:.1}%)",
        100.0 * caught as f64 / refuted as f64
    );
    // Guards against a generator that only yields one verdict.
    assert!(
        implied > PAIRS / 20 && refuted > PAIRS / 20,
        "{implied} implied, {refuted} refuted"
    );
}
