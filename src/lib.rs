//! Specification weakening against a model, by genetic search over LTL
//! formulas. `ltl` holds the formulas, `oracle` decides whether a model
//! satisfies one, `instance` drops instances not worth weakening, `search`
//! breeds weakenings and gates them, and `rers` with `kripke` builds
//! explicit models.

#![deny(missing_docs)]

pub mod count;
pub mod instance;
pub mod kripke;
pub mod ltl;
pub mod oracle;
pub mod rers;
pub mod search;
