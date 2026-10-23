//! The in-process half of oracle tier 2: proving M ⊨ φ' from a formula ψ
//! already known to hold on M, when ψ ⇒ φ'.
//!
//! Deciding ψ ⇒ φ' needs Spot. This module only decides when that call is
//! worth making. A [`Fingerprint`] records which of a fixed set of random
//! lasso words ([`Words`]) satisfy a formula. A word that satisfies ψ but not
//! φ' is a model of `ψ & !φ'`, so it refutes ψ ⇒ φ' exactly; one AND-NOT over
//! the two fingerprints finds every such word in the sample. The test is
//! necessary and not sufficient: when no sampled word separates ψ from φ',
//! the implication is likely but still unproved.
//!
//! Short random words work because most non-implications between small
//! formulas already show on a word of a few positions, and each word is cheap
//! to evaluate. The sampling follows PEREDUR's `fingerprint::prefilter`: 256
//! words, a prefix of 0 to 2 positions and a cycle of 1 to 3, each atom true
//! at each position with probability 1/2. PEREDUR capped a word at 64
//! positions to pack it in a machine word; [`Lasso::satisfies`] has no such
//! cap, so neither does this module.
//!
//! On 400 fixed-seed pairs of a random formula and a one-subformula mutant
//! of it over three atoms (`tests/fingerprint.rs`), Spot proved 188
//! implications and refuted 212. The fingerprint ruled out none of the 188
//! and refuted 208 of the 212 (98.1%).

use std::sync::Arc;

use crate::ltl::Formula;

use super::{Lasso, Valuation};

/// The number of sampled words, and so of bits in a [`Fingerprint`].
const N_WORDS: usize = 256;
const MAX_PREFIX: u64 = 2;
const MAX_CYCLE: u64 = 3;

/// A fixed sample of lasso words over the model's atoms. Two fingerprints
/// can be compared only when they come from the same sample.
#[derive(Debug)]
pub struct Words {
    words: Vec<Lasso>,
}

impl Words {
    /// Draws 256 words over `atoms`. The words depend only on the
    /// set of atoms and on `seed`. An atom of a formula that is not in
    /// `atoms` is false at every position of every word.
    pub fn sample<I>(atoms: I, seed: u64) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Arc<str>>,
    {
        let atoms: Valuation = atoms.into_iter().map(Into::into).collect();
        let mut rng = SplitMix64(seed);
        let words = (0..N_WORDS)
            .map(|_| {
                let prefix_len = rng.below(MAX_PREFIX + 1);
                let cycle_len = 1 + rng.below(MAX_CYCLE);
                let prefix = rng.positions(&atoms, prefix_len);
                let cycle = rng.positions(&atoms, cycle_len);
                Lasso::new(prefix, cycle).expect("the cycle has a position")
            })
            .collect();
        Words { words }
    }

    /// The words that satisfy `formula`, as a bitset.
    pub fn fingerprint(&self, formula: &Formula) -> Fingerprint {
        let mut bits = [0u64; N_WORDS / 64];
        for (i, word) in self.words.iter().enumerate() {
            if word.satisfies(formula) {
                bits[i / 64] |= 1 << (i % 64);
            }
        }
        Fingerprint(bits)
    }
}

/// Which words of a [`Words`] sample satisfy a formula, one bit per word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Fingerprint([u64; N_WORDS / 64]);

impl Fingerprint {
    /// False when some sampled word satisfies `self`'s formula but not
    /// `other`'s, which proves that the first does not imply the second.
    /// True means only that the sample found no such word.
    pub fn may_imply(&self, other: &Fingerprint) -> bool {
        self.0.iter().zip(&other.0).all(|(a, b)| a & !b == 0)
    }
}

/// Sebastiano Vigna's SplitMix64: small, seedable and good enough to draw
/// test words.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A draw from `0..n`. The modulo bias is negligible for small `n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// `n` positions, each atom holding at each with probability 1/2.
    fn positions(&mut self, atoms: &Valuation, n: u64) -> Vec<Valuation> {
        (0..n)
            .map(|_| {
                atoms
                    .iter()
                    .filter(|_| self.next() & 1 == 1)
                    .cloned()
                    .collect()
            })
            .collect()
    }
}

/// The formulas known to hold on M, each with its fingerprint.
///
/// Kept as an antichain of the strongest truths, but only as far as is
/// sound: a fingerprint can show that one truth does not imply another and
/// never that it does. So [`KnownTruths::insert`] keeps every new truth, and
/// a stored truth leaves only through [`KnownTruths::remove`], once Spot has
/// proved that another stored truth implies it.
#[derive(Debug)]
pub struct KnownTruths {
    words: Words,
    truths: Vec<(Formula, Fingerprint)>,
}

impl KnownTruths {
    /// No known truths yet; fingerprints are taken on `words`.
    pub fn new(words: Words) -> Self {
        KnownTruths {
            words,
            truths: Vec::new(),
        }
    }

    /// Records that M satisfies `truth`. A formula already stored is not
    /// stored twice.
    pub fn insert(&mut self, truth: Formula) {
        if self.truths.iter().all(|(f, _)| *f != truth) {
            let print = self.words.fingerprint(&truth);
            self.truths.push((truth, print));
        }
    }

    /// Forgets `implied`. The caller removes a truth only once Spot has
    /// proved that another stored truth implies it, so every formula the
    /// removed one implies is still implied by a stored one.
    pub fn remove(&mut self, implied: &Formula) {
        self.truths.retain(|(f, _)| f != implied);
    }

    /// The stored truths that may imply `candidate`: every other truth has a
    /// sampled word that satisfies it and violates `candidate`. These are the
    /// only truths worth an implication call to prove M ⊨ `candidate`.
    pub fn may_imply<'a>(&'a self, candidate: &Formula) -> impl Iterator<Item = &'a Formula> {
        let print = self.words.fingerprint(candidate);
        self.truths
            .iter()
            .filter(move |(_, p)| p.may_imply(&print))
            .map(|(f, _)| f)
    }

    /// The stored truths that `truth` may imply, which are the ones worth an
    /// implication call before [`KnownTruths::remove`] when `truth` is added.
    pub fn may_be_implied_by<'a>(&'a self, truth: &Formula) -> impl Iterator<Item = &'a Formula> {
        let print = self.words.fingerprint(truth);
        let truth = truth.clone();
        self.truths
            .iter()
            .filter(move |(f, p)| print.may_imply(p) && *f != truth)
            .map(|(f, _)| f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltl::{BinaryOp, UnaryOp};

    const ATOMS: [&str; 3] = ["a", "b", "c"];

    fn f(text: &str) -> Formula {
        text.parse().unwrap()
    }

    fn words() -> Words {
        Words::sample(ATOMS, 7)
    }

    /// Marsaglia's xorshift64, for fixed-seed random formulas.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    fn random(rng: &mut Rng, depth: usize) -> Formula {
        const UNARY: [UnaryOp; 4] = [
            UnaryOp::Not,
            UnaryOp::Next,
            UnaryOp::Eventually,
            UnaryOp::Globally,
        ];
        const BINARY: [BinaryOp; 7] = [
            BinaryOp::And,
            BinaryOp::Or,
            BinaryOp::Implies,
            BinaryOp::Iff,
            BinaryOp::Until,
            BinaryOp::WeakUntil,
            BinaryOp::Release,
        ];
        if depth == 0 || rng.below(4) == 0 {
            return Formula::atom(ATOMS[rng.below(ATOMS.len())]);
        }
        if rng.below(3) == 0 {
            Formula::unary(UNARY[rng.below(4)], random(rng, depth - 1))
        } else {
            let op = BINARY[rng.below(7)];
            Formula::binary(op, random(rng, depth - 1), random(rng, depth - 1))
        }
    }

    /// Pairs (ψ, φ) with ψ ⇒ φ valid by an LTL law, over random p and q.
    fn implied_pairs(p: &Formula, q: &Formula) -> Vec<(Formula, Formula)> {
        let and = |l: &Formula, r: &Formula| Formula::binary(BinaryOp::And, l.clone(), r.clone());
        let or = |l: &Formula, r: &Formula| Formula::binary(BinaryOp::Or, l.clone(), r.clone());
        let bin = |op, l: &Formula, r: &Formula| Formula::binary(op, l.clone(), r.clone());
        let g = |x: &Formula| Formula::unary(UnaryOp::Globally, x.clone());
        let ev = |x: &Formula| Formula::unary(UnaryOp::Eventually, x.clone());
        let x = |x: &Formula| Formula::unary(UnaryOp::Next, x.clone());
        vec![
            (and(p, q), p.clone()),
            (p.clone(), or(p, q)),
            (g(p), ev(p)),
            (g(p), p.clone()),
            (p.clone(), ev(p)),
            (g(p), x(p)),
            (bin(BinaryOp::Until, p, q), ev(q)),
            (bin(BinaryOp::Until, p, q), bin(BinaryOp::WeakUntil, p, q)),
            (g(p), bin(BinaryOp::WeakUntil, p, q)),
            (bin(BinaryOp::Release, p, q), q.clone()),
            (and(&g(p), &ev(q)), ev(&and(p, q))),
            (ev(&g(p)), g(&ev(p))),
            (p.clone(), p.clone()),
        ]
    }

    #[test]
    fn an_implied_pair_is_never_ruled_out() {
        let words = words();
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..200 {
            let (p, q) = (random(&mut rng, 3), random(&mut rng, 3));
            for (strong, weak) in implied_pairs(&p, &q) {
                assert!(
                    words
                        .fingerprint(&strong)
                        .may_imply(&words.fingerprint(&weak)),
                    "{strong} => {weak} was ruled out"
                );
            }
        }
    }

    #[test]
    fn a_non_implied_pair_is_ruled_out() {
        let words = words();
        for (strong, weak) in [("F a", "G a"), ("a", "b"), ("a | b", "a"), ("true", "a")] {
            assert!(
                !words
                    .fingerprint(&f(strong))
                    .may_imply(&words.fingerprint(&f(weak))),
                "{strong} => {weak} was not ruled out"
            );
        }
    }

    #[test]
    fn a_fixed_seed_gives_fixed_words() {
        let probe = f("a U (b & X c)");
        let print = Words::sample(ATOMS, 7).fingerprint(&probe);
        // The order of the atoms does not matter.
        assert_eq!(print, Words::sample(["c", "b", "a"], 7).fingerprint(&probe));
        assert_ne!(print, Words::sample(ATOMS, 8).fingerprint(&probe));
    }

    #[test]
    fn an_unknown_atom_is_always_false() {
        let words = words();
        assert_eq!(words.fingerprint(&f("z")), words.fingerprint(&f("false")));
    }

    #[test]
    fn known_truths_filter_by_fingerprint() {
        let mut known = KnownTruths::new(words());
        assert_eq!(known.may_imply(&f("F a")).count(), 0);
        known.insert(f("G a"));
        known.insert(f("G b"));
        known.insert(f("G a"));
        let survivors: Vec<_> = known.may_imply(&f("F a")).collect();
        assert_eq!(survivors, [&f("G a")]);
        // A new truth G (a & b) may imply both stored ones.
        let implied: Vec<_> = known.may_be_implied_by(&f("G (a & b)")).collect();
        assert_eq!(implied, [&f("G a"), &f("G b")]);
        // Once Spot proves it, the caller drops the weaker truths.
        known.insert(f("G (a & b)"));
        known.remove(&f("G a"));
        known.remove(&f("G b"));
        let survivors: Vec<_> = known.may_imply(&f("F a")).collect();
        assert_eq!(survivors, [&f("G (a & b)")]);
        assert_eq!(known.may_be_implied_by(&f("G (a & b)")).count(), 0);
    }
}
