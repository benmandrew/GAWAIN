//! The model-checking oracle: does the model M satisfy a candidate φ'?
//!
//! Every back end answers with a [`Verdict`]. A failing verdict carries a
//! [`Lasso`], a run of M that violates the formula, which the oracle keeps
//! to refute later candidates without another model-checking call. See
//! `docs/dev/oracle-performance.md` for the tiered design and its costs.

pub mod explicit;
pub(crate) mod hoa;
mod implication;
mod lasso;
pub mod nuxmv;
mod packed;
pub mod process;
pub mod spin;
mod spot;
pub mod tapaal;
mod tiered;
pub mod traces;

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::ltl::Formula;

pub use implication::{Fingerprint, KnownTruths, Words};
pub use lasso::LassoCache;
pub use spot::{implications, implications_each, syntactic_implications, ALIVE};
pub use tiered::{Counters, Oracle};

/// The atoms that hold at one position of a run.
pub type Valuation = BTreeSet<Arc<str>>;

/// A run of the model: an infinite lasso `prefix · cycle^ω`, or a finite
/// word, which is `prefix` alone with an empty cycle.
///
/// A finite word is read under LTLf semantics (see [`Lasso::satisfies`]),
/// so only a back end whose [`Backend::semantics`] is
/// [`Semantics::Finite`] returns one. Each valuation lists every atom of the model's interface that holds at
/// that position, not only the atoms of the formula that produced the run,
/// so the run can be replayed against any later candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lasso {
    prefix: Vec<Valuation>,
    cycle: Vec<Valuation>,
}

impl Lasso {
    /// Builds a lasso, or returns `None` when `cycle` is empty, since an
    /// empty cycle describes a finite run.
    pub fn new(prefix: Vec<Valuation>, cycle: Vec<Valuation>) -> Option<Self> {
        if cycle.is_empty() {
            return None;
        }
        Some(Lasso { prefix, cycle })
    }

    /// A finite word, or `None` when `word` is empty, since LTLf has no
    /// empty word.
    pub fn finite(word: Vec<Valuation>) -> Option<Self> {
        if word.is_empty() {
            return None;
        }
        Some(Lasso {
            prefix: word,
            cycle: Vec::new(),
        })
    }

    /// Whether this is a finite word rather than a lasso.
    pub fn is_finite(&self) -> bool {
        self.cycle.is_empty()
    }

    /// The positions before the cycle, or the whole of a finite word.
    pub fn prefix(&self) -> &[Valuation] {
        &self.prefix
    }

    /// The positions that repeat forever. Empty only for a finite word.
    pub fn cycle(&self) -> &[Valuation] {
        &self.cycle
    }
}

/// Which words a formula is read over: the semantics of a back end's runs,
/// and so of every Spot call made about them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Semantics {
    /// LTL over infinite words, as for a Kripke structure.
    #[default]
    Infinite,
    /// LTLf over finite non-empty words, with `X` as weak next, as Spot's
    /// `--from-ltlf` reads it.
    Finite,
}

/// The answer to one query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// M satisfies the formula.
    Holds,
    /// M violates the formula, as the lasso shows.
    Fails(Lasso),
    /// No verdict: a timeout, a memory cap or a tool error, with the reason.
    /// Never cached and never read as either verdict.
    Unknown(String),
}

/// A complete model checker for one fixed model.
pub trait Backend {
    /// Decides whether the model satisfies `formula`.
    fn check(&mut self, formula: &Formula) -> Verdict;

    /// The semantics of the model's runs. A model checker reads LTL over
    /// infinite runs; a set of finite traces reads LTLf.
    fn semantics(&self) -> Semantics {
        Semantics::Infinite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lasso_needs_a_cycle() {
        assert!(Lasso::new(vec![Valuation::new()], vec![]).is_none());
        let lasso = Lasso::new(vec![], vec![Valuation::new()]).unwrap();
        assert_eq!(lasso.cycle().len(), 1);
        assert!(lasso.prefix().is_empty());
        assert!(!lasso.is_finite());
        assert!(Lasso::finite(vec![]).is_none());
        assert!(Lasso::finite(vec![Valuation::new()]).unwrap().is_finite());
    }
}
