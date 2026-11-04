//! Specification weakening against a model, by genetic search over LTL
//! formulas. `ltl` holds the formulas, `oracle` decides whether a model
//! satisfies one, `instance` drops instances not worth weakening, `search`
//! breeds weakenings and gates them, `rers` with `kripke` builds explicit
//! models, `mcc` reads Model Checking Contest nets and properties, and
//! `diversity` measures how far apart repairs lie in behaviour.

#![deny(missing_docs)]

pub mod count;
pub mod diversity;
pub mod instance;
pub mod kripke;
pub mod ltl;
pub mod mcc;
pub mod oracle;
pub mod rers;
pub mod search;
