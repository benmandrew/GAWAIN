//! Counting the models of one wide edge label with Ganak, ported from
//! PEREDUR's `run_ganak_on_formula` and its Tseitin encoder (`cnf.cpp`).
//!
//! The label becomes a CNF in DIMACS form on Ganak's stdin. Every operator
//! gets the full bi-implication, not the one direction plain SAT needs, so
//! each auxiliary variable is functionally determined by the label's own
//! variables and the CNF has exactly the label's model count.

use std::process::Command;

use crate::oracle::hoa::Guard;
use crate::oracle::process::{self, Limits};

/// The number of assignments to APs `0..n_vars` that satisfy `guard`, which
/// mentions no AP at or above `n_vars`, from one Ganak exec.
pub fn count(guard: &Guard, n_vars: usize, limits: Limits) -> Result<f64, String> {
    let mut command = Command::new("ganak");
    let output = process::run(&mut command, &dimacs(guard, n_vars), limits)
        .map_err(|e| format!("ganak: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "ganak exited with {}: {}",
            output.status,
            output.stderr.trim()
        ));
    }
    parse(&output.stdout)
}

/// The CNF of `guard`: variable `i + 1` is AP `i`, auxiliaries follow, and
/// a unit clause asserts the root.
pub fn dimacs(guard: &Guard, n_vars: usize) -> String {
    let mut encoder = Tseitin {
        next: n_vars + 1,
        clauses: Vec::new(),
    };
    let root = encoder.encode(guard);
    encoder.clauses.push(vec![root]);
    let mut text = format!("p cnf {} {}\n", encoder.next - 1, encoder.clauses.len());
    for clause in &encoder.clauses {
        for literal in clause {
            text.push_str(&literal.to_string());
            text.push(' ');
        }
        text.push_str("0\n");
    }
    text
}

struct Tseitin {
    next: usize,
    clauses: Vec<Vec<i64>>,
}

impl Tseitin {
    fn fresh(&mut self) -> i64 {
        let var = self.next as i64;
        self.next += 1;
        var
    }

    fn encode(&mut self, guard: &Guard) -> i64 {
        match guard {
            Guard::Ap(i) => *i as i64 + 1,
            Guard::True | Guard::False => {
                let var = self.fresh();
                let value = if matches!(guard, Guard::True) {
                    var
                } else {
                    -var
                };
                self.clauses.push(vec![value]);
                var
            }
            Guard::Not(g) => {
                let child = self.encode(g);
                let var = self.fresh();
                self.clauses.push(vec![-var, -child]);
                self.clauses.push(vec![var, child]);
                var
            }
            Guard::And(g, h) => {
                let (l, r) = (self.encode(g), self.encode(h));
                let var = self.fresh();
                self.clauses.push(vec![-var, l]);
                self.clauses.push(vec![-var, r]);
                self.clauses.push(vec![var, -l, -r]);
                var
            }
            Guard::Or(g, h) => {
                let (l, r) = (self.encode(g), self.encode(h));
                let var = self.fresh();
                self.clauses.push(vec![var, -l]);
                self.clauses.push(vec![var, -r]);
                self.clauses.push(vec![-var, l, r]);
                var
            }
        }
    }
}

/// Reads `c s exact arb int N`, or 0 from `s UNSATISFIABLE`. The count may
/// exceed `u64`, so it is read as an `f64`; it only ever weights a ratio
/// of logarithms.
fn parse(stdout: &str) -> Result<f64, String> {
    if let Some(line) = stdout
        .lines()
        .find_map(|l| l.strip_prefix("c s exact arb int "))
    {
        return line
            .trim()
            .parse()
            .map_err(|_| format!("ganak: unreadable count {line:?}"));
    }
    if stdout.lines().any(|l| l.trim() == "s UNSATISFIABLE") {
        return Ok(0.0);
    }
    Err(format!("ganak: no count in {stdout:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(i: usize) -> Box<Guard> {
        Box::new(Guard::Ap(i))
    }

    #[test]
    fn the_root_is_asserted_after_its_definitions() {
        // a | !b over two variables: 3 = Not(b), 4 = Or(a, 3).
        let guard = Guard::Or(ap(0), Box::new(Guard::Not(ap(1))));
        assert_eq!(
            dimacs(&guard, 2),
            "p cnf 4 6\n-3 -2 0\n3 2 0\n4 -1 0\n4 -3 0\n-4 1 3 0\n4 0\n"
        );
    }

    #[test]
    fn counts_and_unsatisfiable_answers_are_read() {
        let out = "c o noise\ns SATISFIABLE\nc s exact arb int 123456789012345678901234\n";
        assert_eq!(parse(out), Ok(123456789012345678901234.0));
        assert_eq!(parse("s UNSATISFIABLE\n"), Ok(0.0));
        assert!(parse("c o nothing\n").is_err());
        assert!(parse("c s exact arb int x\n").is_err());
    }
}
