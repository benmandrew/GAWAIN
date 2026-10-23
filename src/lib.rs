//! Specification weakening against a model, by genetic search over LTL
//! formulas. `ltl` holds the formulas, `oracle` decides whether a model
//! satisfies one, and `rers` with `kripke` builds explicit models.

#![deny(missing_docs)]

pub mod kripke;
pub mod ltl;
pub mod oracle;
pub mod rers;
