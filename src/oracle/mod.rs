//! The model-checking oracle: does the model M satisfy a candidate φ'?
//!
//! Every back end answers with a [`Verdict`]. A failing verdict carries a
//! [`Lasso`], a run of M that violates the formula, which the oracle keeps
//! to refute later candidates without another model-checking call. See
//! `docs/dev/oracle-performance.md` for the tiered design and its costs.

mod implication;
mod lasso;
pub mod nuxmv;
pub mod process;

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::ltl::Formula;

pub use implication::{Fingerprint, KnownTruths, Words};
pub use lasso::LassoCache;

/// The atoms that hold at one position of a run.
pub type Valuation = BTreeSet<Arc<str>>;

/// An infinite run `prefix · cycle^ω` of the model.
///
/// Each valuation lists every atom of the model's interface that holds at
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

    /// The positions before the cycle.
    pub fn prefix(&self) -> &[Valuation] {
        &self.prefix
    }

    /// The positions that repeat forever. Never empty.
    pub fn cycle(&self) -> &[Valuation] {
        &self.cycle
    }
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
    }
}
