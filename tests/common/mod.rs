//! Helpers shared by the end-to-end tests: tool detection and fixed-seed
//! random formulas and mutants.

#![allow(dead_code)]

use std::io::Write;

use gawain::ltl::{BinaryOp, Formula, Node, UnaryOp};

pub fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// True when every tool is on `PATH`; otherwise prints why `test` skips.
/// Written to stderr directly, which the test harness does not capture, so a
/// skip is visible in a normal `cargo test` run.
pub fn have(tools: &[&str], test: &str) -> bool {
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

pub fn parse(text: &str) -> Formula {
    text.parse()
        .unwrap_or_else(|e| panic!("parse {text:?}: {e}"))
}

/// Marsaglia's xorshift64, enough for a fixed-seed test.
pub struct Rng(pub u64);

impl Rng {
    pub fn below(&mut self, n: usize) -> usize {
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

/// A random formula over `atoms` of depth at most `depth`.
pub fn formula(rng: &mut Rng, depth: usize, atoms: &[&str]) -> Formula {
    if depth == 0 || rng.below(4) == 0 {
        return match rng.below(8) {
            0 => Formula::tt(),
            1 => Formula::ff(),
            _ => Formula::atom(atoms[rng.below(atoms.len())]),
        };
    }
    if rng.below(3) == 0 {
        let op = UNARY[rng.below(UNARY.len())];
        Formula::unary(op, formula(rng, depth - 1, atoms))
    } else {
        let op = BINARY[rng.below(BINARY.len())];
        let lhs = formula(rng, depth - 1, atoms);
        Formula::binary(op, lhs, formula(rng, depth - 1, atoms))
    }
}

/// `f` with its subformula at pre-order index `target` replaced by `new`.
pub fn replace(f: &Formula, target: usize, new: &Formula) -> Formula {
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

/// A mutant of `f` as a GA offspring would be: one random subformula
/// replaced by a random formula of depth at most `depth`.
pub fn mutant(rng: &mut Rng, f: &Formula, depth: usize, atoms: &[&str]) -> Formula {
    let target = rng.below(f.n_subformulae());
    replace(f, target, &formula(rng, depth, atoms))
}
