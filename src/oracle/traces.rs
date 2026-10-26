//! A set of traces in place of a model: M ⊨ φ' iff every positive trace
//! satisfies φ'.
//!
//! The traces are all lassos, read under LTL, or all finite words, read
//! under LTLf ([`Semantics`]). Runs of at most 64 positions, which is every
//! run in the three sources, are packed into machine words and evaluated by
//! a compiled program (`oracle::packed`); longer ones fall back to
//! [`Lasso::satisfies`], which has the same semantics. The negative traces are not part of M. They give an optional check that a
//! repair still rejects something ([`TraceSet::rejected`]).
//!
//! A set is read from a text file, one line per item; `#` starts a comment
//! line:
//!
//! ```text
//! semantics finite            (or infinite)
//! atoms a b c
//! formula F (a & X b)         (optional: the source's separating formula)
//! + 100 010 001               a positive trace, one bit string per position
//! - 000 | 110 011             a negative lasso: prefix | cycle
//! ```
//!
//! Each position is one `0` or `1` per atom, in the order of `atoms`. A
//! finite trace has no `|`; a lasso has one, and a non-empty cycle after
//! it. See `docs/dev/oracle.md` for which oracle tiers stay sound here.

use std::sync::Arc;

use super::packed::{Packed, Program};
use super::spot::ALIVE;
use super::{Backend, Lasso, Semantics, Valuation, Verdict};
use crate::ltl::Formula;

/// The positive and negative traces of one instance.
#[derive(Clone, Debug)]
pub struct TraceSet {
    semantics: Semantics,
    atoms: Vec<Arc<str>>,
    formula: Option<Formula>,
    positives: Vec<Lasso>,
    negatives: Vec<Lasso>,
    /// The positives and negatives packed, when every run fits.
    packed: Option<(Vec<Packed>, Vec<Packed>)>,
}

impl TraceSet {
    /// Reads a set in the format of the module header. There must be at
    /// least one positive trace, since an empty M satisfies every formula.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut semantics = None;
        let mut atoms: Option<Vec<Arc<str>>> = None;
        let mut formula = None;
        let mut positives = Vec::new();
        let mut negatives = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: String| format!("line {}: {e}", number + 1);
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            let rest = rest.trim();
            match key {
                "semantics" => {
                    semantics = Some(match rest {
                        "finite" => Semantics::Finite,
                        "infinite" => Semantics::Infinite,
                        _ => return Err(at(format!("unknown semantics {rest:?}"))),
                    })
                }
                "atoms" => {
                    let names: Vec<Arc<str>> = rest.split_whitespace().map(Arc::from).collect();
                    if names.iter().any(|a| &**a == ALIVE) {
                        return Err(at(format!("the atom {ALIVE} is reserved")));
                    }
                    atoms = Some(names);
                }
                "formula" => {
                    formula = Some(rest.parse::<Formula>().map_err(|e| at(e.to_string()))?)
                }
                "+" | "-" => {
                    let atoms = atoms
                        .as_deref()
                        .ok_or_else(|| at("a trace before atoms".into()))?;
                    let semantics =
                        semantics.ok_or_else(|| at("a trace before semantics".into()))?;
                    let trace = trace(rest, atoms, semantics).map_err(at)?;
                    if key == "+" {
                        positives.push(trace);
                    } else {
                        negatives.push(trace);
                    }
                }
                _ => return Err(at(format!("unknown line {key:?}"))),
            }
        }
        if positives.is_empty() {
            return Err("no positive trace".into());
        }
        let atoms = atoms.ok_or("no atoms line")?;
        let pack = |runs: &[Lasso]| -> Option<Vec<Packed>> {
            runs.iter().map(|run| Packed::new(run, &atoms)).collect()
        };
        let packed = pack(&positives).zip(pack(&negatives));
        Ok(TraceSet {
            semantics: semantics.ok_or("no semantics line")?,
            atoms,
            formula,
            positives,
            negatives,
            packed,
        })
    }

    /// The atoms, in the order of the bit strings.
    pub fn atoms(&self) -> &[Arc<str>] {
        &self.atoms
    }

    /// The source's separating formula, when the file gives one.
    pub fn formula(&self) -> Option<&Formula> {
        self.formula.as_ref()
    }

    /// The traces of M.
    pub fn positives(&self) -> &[Lasso] {
        &self.positives
    }

    /// The traces outside M.
    pub fn negatives(&self) -> &[Lasso] {
        &self.negatives
    }

    /// How many negative traces violate `formula`.
    pub fn rejected(&self, formula: &Formula) -> usize {
        self.violations(formula, false).count()
    }

    /// How many positive traces violate `formula`.
    pub fn failing(&self, formula: &Formula) -> usize {
        self.violations(formula, true).count()
    }

    /// The index of each positive (or negative) trace that violates
    /// `formula`, in order, computed lazily.
    fn violations<'a>(
        &'a self,
        formula: &'a Formula,
        positive: bool,
    ) -> Box<dyn Iterator<Item = usize> + 'a> {
        let runs = if positive {
            &self.positives
        } else {
            &self.negatives
        };
        match &self.packed {
            Some((pos, neg)) => {
                let packed = if positive { pos } else { neg };
                let program = Program::new(formula, &self.atoms);
                let mut stack = Vec::new();
                Box::new(
                    (0..packed.len()).filter(move |&i| !program.satisfies(&packed[i], &mut stack)),
                )
            }
            None => Box::new((0..runs.len()).filter(move |&i| !runs[i].satisfies(formula))),
        }
    }
}

impl Backend for TraceSet {
    /// Holds iff every positive trace satisfies `formula`; otherwise fails
    /// with the first positive trace that violates it. Never `Unknown`.
    fn check(&mut self, formula: &Formula) -> Verdict {
        let first = self.violations(formula, true).next();
        match first {
            Some(i) => Verdict::Fails(self.positives[i].clone()),
            None => Verdict::Holds,
        }
    }

    fn semantics(&self) -> Semantics {
        self.semantics
    }

    /// The positive traces.
    fn runs(&self) -> Option<&[Lasso]> {
        Some(&self.positives)
    }
}

/// One trace: positions separated by spaces, with `|` before a lasso's
/// cycle.
fn trace(text: &str, atoms: &[Arc<str>], semantics: Semantics) -> Result<Lasso, String> {
    let positions = |part: &str| -> Result<Vec<Valuation>, String> {
        part.split_whitespace()
            .map(|bits| position(bits, atoms))
            .collect()
    };
    match (text.split_once('|'), semantics) {
        (None, Semantics::Finite) => {
            Lasso::finite(positions(text)?).ok_or_else(|| "an empty trace".into())
        }
        (Some((prefix, cycle)), Semantics::Infinite) => {
            Lasso::new(positions(prefix)?, positions(cycle)?)
                .ok_or_else(|| "a lasso with an empty cycle".into())
        }
        (None, Semantics::Infinite) => Err("an infinite trace needs a cycle after |".into()),
        (Some(_), Semantics::Finite) => Err("a finite trace has no cycle".into()),
    }
}

fn position(bits: &str, atoms: &[Arc<str>]) -> Result<Valuation, String> {
    if bits.len() != atoms.len() {
        return Err(format!(
            "{bits:?} has {} bits for {} atoms",
            bits.len(),
            atoms.len()
        ));
    }
    atoms
        .iter()
        .zip(bits.chars())
        .filter_map(|(atom, bit)| match bit {
            '1' => Some(Ok(atom.clone())),
            '0' => None,
            _ => Some(Err(format!("{bits:?} is not a bit string"))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(text: &str) -> Formula {
        text.parse().unwrap()
    }

    const FINITE: &str = "\
# two atoms
semantics finite
atoms a b
formula F b
+ 10 01
+ 01
- 10 10
";

    #[test]
    fn parses_a_finite_set() {
        let set = TraceSet::parse(FINITE).unwrap();
        assert_eq!(set.atoms().len(), 2);
        assert_eq!(set.formula(), Some(&f("F b")));
        assert_eq!(set.positives().len(), 2);
        assert_eq!(set.negatives().len(), 1);
        assert!(set.positives().iter().all(Lasso::is_finite));
        assert_eq!(set.positives()[0].prefix()[0], ["a".into()].into());
    }

    #[test]
    fn checks_every_positive_and_counts_rejected_negatives() {
        let mut set = TraceSet::parse(FINITE).unwrap();
        assert_eq!(set.semantics(), Semantics::Finite);
        assert_eq!(set.check(&f("F b")), Verdict::Holds);
        // The second positive has no a.
        let Verdict::Fails(trace) = set.check(&f("F a")) else {
            panic!("F a fails on the second positive");
        };
        assert_eq!(&trace, &set.positives()[1]);
        assert_eq!(set.rejected(&f("F b")), 1);
        assert_eq!(set.rejected(&f("true")), 0);
        // Weak next holds at the last position of the one-position trace.
        assert_eq!(set.check(&f("b -> X a")), Verdict::Holds);
    }

    #[test]
    fn parses_lassos_under_ltl() {
        let text = "semantics infinite\natoms a\n+ 1 | 0\n+ | 1 0\n- | 1\n";
        let mut set = TraceSet::parse(text).unwrap();
        assert_eq!(set.semantics(), Semantics::Infinite);
        assert_eq!(set.check(&f("F !a")), Verdict::Holds);
        assert!(matches!(set.check(&f("G F a")), Verdict::Fails(_)));
        assert_eq!(set.rejected(&f("F !a")), 1);
    }

    #[test]
    fn rejects_malformed_sets() {
        for bad in [
            "semantics finite\natoms a\n- 1\n",
            "semantics finite\natoms a b\n+ 1\n",
            "semantics finite\natoms a\n+ 2\n",
            "semantics finite\natoms a\n+ 1 | 1\n",
            "semantics infinite\natoms a\n+ 1\n",
            "semantics infinite\natoms a\n+ 1 |\n",
            "atoms a\n+ 1\n",
            "semantics finite\natoms gawain_alive\n+ 1\n",
            "semantics finite\natoms a\nformula F (\n+ 1\n",
            "semantics sometimes\n",
        ] {
            assert!(TraceSet::parse(bad).is_err(), "{bad:?}");
        }
    }
}
