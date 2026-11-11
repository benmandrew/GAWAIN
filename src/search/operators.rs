//! Mutation and crossover of one LTL formula, ported from PEREDUR's TLSF
//! path: `tlsf_mutate` and its helpers (`src/tlsf/mutation.cpp`), the
//! propositional `mutate_formula` (`src/genetic/mutation.cpp`), the monotone
//! rewrite (`src/genetic/monotone.cpp`) and `tlsf_crossover`
//! (`src/tlsf/crossover.cpp`).
//!
//! PEREDUR mutates one conjunct of one section of a specification. Here the
//! individual is a single formula, so the slot, side and section draws are
//! gone and every operator acts on the whole formula. What was left out, and
//! why, is in `docs/dev/search.md`.
//!
//! Every draw is sequenced into its own local, in the order PEREDUR draws.

use std::sync::Arc;

use super::Rng;
use crate::ltl::{BinaryOp, Formula, Node, UnaryOp};

/// The chance that a non-monotone mutation may change the temporal skeleton,
/// PEREDUR's `tlsf_p_temporal` default.
const P_TEMPORAL: f64 = 0.2;

/// Mutates `formula` once, drawing new atoms from `atoms`. With no atoms the
/// formula is returned unchanged, as PEREDUR does for an empty pool.
///
/// A share `p_monotone` of mutations (PEREDUR's default is a quarter) are a
/// [`monotone_rewrite`] in a direction drawn on a fair coin, or always in
/// `forced` when it is given, drawing from `rules`. Of the
/// rest, a fifth may add, drop or swap temporal operators and the others
/// rewrite only the propositional subtrees.
pub fn mutate(
    formula: &Formula,
    atoms: &[Arc<str>],
    p_monotone: f64,
    forced: Option<Direction>,
    rules: RuleSet,
    rng: &mut Rng,
) -> Formula {
    if atoms.is_empty() {
        return formula.clone();
    }
    if rng.chance(p_monotone) {
        // Both directions, as PEREDUR: a search that only weakens cannot
        // recover from an ancestor that weakened past the best repair.
        // The coin is drawn either way, so `forced` keeps the stream.
        let coin = if rng.coin() {
            Direction::Weaken
        } else {
            Direction::Strengthen
        };
        let direction = forced.unwrap_or(coin);
        return monotone_rewrite(formula, direction, atoms, rules, rng);
    }
    if rng.chance(P_TEMPORAL) {
        mutate_temporal(formula, atoms, rng)
    } else {
        mutate_propositional_parts(formula, atoms, rng)
    }
}

/// Recombines two formulas, as one side of PEREDUR's `tlsf_crossover`. On a
/// fair coin, a subformula of `into` is replaced by one of `from`, or joined
/// with one under `&`, `|`, `U` or `W`. Graft sites are the distinct
/// temporal subformulae, or the whole formula when there are none.
pub fn crossover(into: &Formula, from: &Formula, rng: &mut Rng) -> Formula {
    let replace = rng.coin();
    if replace {
        replace_subformula(into, from, rng)
    } else {
        combine_subformula(into, from, rng)
    }
}

/// Which way along the implication order a [`monotone_rewrite`] moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// The result is implied by its input.
    Weaken,
    /// The result implies its input.
    Strengthen,
}

/// Which monotone rules a [`monotone_rewrite`] draws from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RuleSet {
    /// PEREDUR's rules, as ported. The frozen configuration and the trace
    /// benchmark generator use these.
    #[default]
    Peredur,
    /// PEREDUR's rules and the order-guided rules of
    /// `docs/dev/order-search.md`, which reach more covering edges of the
    /// implication order in one step. `proofs/Gawain/Rules.lean` proves
    /// every one sound.
    Ordered,
}

/// Rewrites one node of `formula` so that the result is comparable to it:
/// [`Direction::Weaken`] returns a formula that `formula` implies,
/// [`Direction::Strengthen`] one that implies `formula`.
///
/// The site is drawn uniformly over the nodes of determinate polarity, and
/// then one rule applicable there. A weaker subformula weakens the whole
/// only at a positive occurrence, so under `!` and in the antecedent of `->`
/// the dual rule fires, and a node under `<->` is never a site. `atoms` must
/// not be empty: `AddOperand` draws a literal from it.
pub fn monotone_rewrite(
    formula: &Formula,
    direction: Direction,
    atoms: &[Arc<str>],
    rules: RuleSet,
    rng: &mut Rng,
) -> Formula {
    assert!(!atoms.is_empty(), "a monotone rewrite needs an atom pool");
    let mut polarities = Vec::new();
    collect_polarities(formula, Polarity::Positive, &mut polarities);
    let sites: Vec<usize> = (0..polarities.len())
        .filter(|&i| polarities[i] != Polarity::Indeterminate)
        .collect();
    if sites.is_empty() {
        return formula.clone();
    }
    let site = sites[rng.below(sites.len())];
    let mut next = 0;
    let site = Site {
        target: site,
        want_weaker: direction == Direction::Weaken,
        atoms,
        rules,
    };
    site.rewrite(formula, Polarity::Positive, &mut next, rng)
}

/// Every formula that [`monotone_rewrite`] can return for `formula`, with
/// `literal` as the drawn literal, over every site, rule and coin outcome,
/// in pre-order of the sites. `bench/check_lean_orders.py` compares this
/// list with the Lean transcription in `proofs/Gawain/Rules.lean`.
pub fn monotone_outcomes(
    formula: &Formula,
    direction: Direction,
    rules: RuleSet,
    literal: &Formula,
) -> Vec<Formula> {
    let want_weaker = direction == Direction::Weaken;
    let mut out = Vec::new();
    outcomes_below(
        formula,
        Polarity::Positive,
        want_weaker,
        rules,
        literal,
        &mut |f| f,
        &mut out,
    );
    out
}

fn outcomes_below(
    formula: &Formula,
    polarity: Polarity,
    want_weaker: bool,
    rules: RuleSet,
    literal: &Formula,
    wrap: &mut dyn FnMut(Formula) -> Formula,
    out: &mut Vec<Formula>,
) {
    if polarity != Polarity::Indeterminate {
        let weaken_here = (polarity == Polarity::Positive) == want_weaker;
        for rule in rules_at(formula, weaken_here, rules) {
            for outcome in outcomes(formula, rule, weaken_here, literal) {
                out.push(wrap(outcome));
            }
        }
    }
    let (first, second) = child_polarities(formula, polarity);
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => {}
        Node::Unary(op, child) => {
            let op = *op;
            let mut inner = |c| wrap(Formula::unary(op, c));
            outcomes_below(child, first, want_weaker, rules, literal, &mut inner, out);
        }
        Node::Binary(op, lhs, rhs) => {
            let op = *op;
            let mut left = |l| wrap(Formula::binary(op, l, rhs.clone()));
            outcomes_below(lhs, first, want_weaker, rules, literal, &mut left, out);
            let mut right = |r| wrap(Formula::binary(op, lhs.clone(), r));
            outcomes_below(rhs, second, want_weaker, rules, literal, &mut right, out);
        }
    }
}

/// The vacuity witnesses of `formula`: for each subformula occurrence ψ of
/// determinate polarity, other than the whole formula and the constants,
/// `formula[ψ ← false]` where ψ is positive and `formula[ψ ← true]` where it
/// is negative, simplified, as pairs (ψ, witness) in pre-order. Each
/// witness implies `formula`, so a model that satisfies one satisfies
/// `formula` whatever ψ says (Beer, Ben-David, Eisner and Rodeh; Kupferman
/// and Vardi). A witness that folds to `false` is left out, since no model
/// with a run satisfies it.
pub fn vacuity_witnesses(formula: &Formula) -> Vec<(Formula, Formula)> {
    let mut polarities = Vec::new();
    collect_polarities(formula, Polarity::Positive, &mut polarities);
    let mut nodes = Vec::new();
    pre_order(formula, &mut nodes);
    let mut witnesses = Vec::new();
    for (target, &polarity) in polarities.iter().enumerate().skip(1) {
        let constant = match polarity {
            Polarity::Positive => Formula::ff(),
            Polarity::Negative => Formula::tt(),
            Polarity::Indeterminate => continue,
        };
        let mut next = 0;
        let Some(witness) = replace_at(formula, target, &constant, &mut next) else {
            continue;
        };
        let witness = witness.simplify();
        if !matches!(witness.node(), Node::False) {
            witnesses.push((nodes[target].clone(), witness));
        }
    }
    witnesses
}

/// Every subformula occurrence of `formula`, in pre-order.
fn pre_order(formula: &Formula, out: &mut Vec<Formula>) {
    out.push(formula.clone());
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => {}
        Node::Unary(_, child) => pre_order(child, out),
        Node::Binary(_, lhs, rhs) => {
            pre_order(lhs, out);
            pre_order(rhs, out);
        }
    }
}

/// `formula` with the node at pre-order index `target` replaced by `with`,
/// or `None` when that node is a constant.
fn replace_at(
    formula: &Formula,
    target: usize,
    with: &Formula,
    next: &mut usize,
) -> Option<Formula> {
    let index = *next;
    *next += 1;
    if index == target {
        return match formula.node() {
            Node::True | Node::False => None,
            _ => Some(with.clone()),
        };
    }
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => Some(formula.clone()),
        Node::Unary(op, child) => {
            replace_at(child, target, with, next).map(|c| Formula::unary(*op, c))
        }
        Node::Binary(op, lhs, rhs) => {
            let left = replace_at(lhs, target, with, next)?;
            let right = replace_at(rhs, target, with, next)?;
            Some(Formula::binary(*op, left, right))
        }
    }
}

fn is_temporal(formula: &Formula) -> bool {
    matches!(
        formula.node(),
        Node::Unary(UnaryOp::Next | UnaryOp::Eventually | UnaryOp::Globally, _)
            | Node::Binary(
                BinaryOp::Until | BinaryOp::WeakUntil | BinaryOp::Release,
                _,
                _
            )
    )
}

fn is_propositional(formula: &Formula) -> bool {
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => true,
        _ if is_temporal(formula) => false,
        Node::Unary(_, child) => is_propositional(child),
        Node::Binary(_, lhs, rhs) => is_propositional(lhs) && is_propositional(rhs),
    }
}

fn pick_unary(rng: &mut Rng) -> UnaryOp {
    [
        UnaryOp::Not,
        UnaryOp::Next,
        UnaryOp::Eventually,
        UnaryOp::Globally,
    ][rng.below(4)]
}

/// The temporal path's binary operators. `<->` stays out, as in PEREDUR.
fn pick_binary(rng: &mut Rng) -> BinaryOp {
    [
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Until,
        BinaryOp::Release,
        BinaryOp::WeakUntil,
        BinaryOp::Implies,
    ][rng.below(6)]
}

/// The connective that grafts an anchor atom onto a unary node.
fn pick_connective(rng: &mut Rng) -> BinaryOp {
    [
        BinaryOp::Until,
        BinaryOp::WeakUntil,
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Implies,
    ][rng.below(5)]
}

/// The propositional path's binary operators.
fn pick_boolean(rng: &mut Rng) -> BinaryOp {
    [
        BinaryOp::And,
        BinaryOp::Or,
        BinaryOp::Implies,
        BinaryOp::Iff,
    ][rng.below(4)]
}

fn random_atom(atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    Formula::atom(&*atoms[rng.below(atoms.len())])
}

fn draw_literal(atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    let atom = random_atom(atoms, rng);
    let negate = rng.coin();
    if negate {
        Formula::unary(UnaryOp::Not, atom)
    } else {
        atom
    }
}

fn not(formula: Formula) -> Formula {
    Formula::unary(UnaryOp::Not, formula)
}

/// A constant flips at no draw. An atom becomes a pool atom, a distinct one
/// when the pool has another.
fn flip_or_replace(leaf: &Formula, atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    match leaf.node() {
        Node::True => Formula::ff(),
        Node::False => Formula::tt(),
        Node::Atom(name) => {
            let mut index = rng.below(atoms.len());
            if atoms[index] == *name && atoms.len() > 1 {
                index = (index + 1) % atoms.len();
            }
            Formula::atom(&*atoms[index])
        }
        _ => unreachable!("only a leaf is renamed"),
    }
}

/// Brizzio et al.'s mutation, which may add, drop or swap temporal
/// operators, with PEREDUR's connective-keeping arm at binary nodes.
fn mutate_temporal(formula: &Formula, atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => {
            if rng.below(3) == 0 {
                let op = pick_unary(rng);
                return Formula::unary(op, formula.clone());
            }
            flip_or_replace(formula, atoms, rng)
        }
        Node::Unary(op, child) => {
            let mutated = mutate_temporal(child, atoms, rng);
            match rng.below(4) {
                0 => mutated,
                1 => {
                    let new_op = pick_unary(rng);
                    Formula::unary(new_op, mutated)
                }
                2 => {
                    let outer = pick_unary(rng);
                    Formula::unary(outer, Formula::unary(*op, mutated))
                }
                _ => {
                    let anchor = random_atom(atoms, rng);
                    let inner_op = pick_unary(rng);
                    let connective = pick_connective(rng);
                    Formula::binary(connective, anchor, Formula::unary(inner_op, mutated))
                }
            }
        }
        Node::Binary(op, lhs, rhs) => match rng.below(4) {
            0 => {
                let left = rng.coin();
                let chosen = if left { lhs } else { rhs };
                mutate_temporal(chosen, atoms, rng)
            }
            1 => {
                let new_op = pick_binary(rng);
                let left = mutate_temporal(lhs, atoms, rng);
                let right = mutate_temporal(rhs, atoms, rng);
                Formula::binary(new_op, left, right)
            }
            2 => {
                let outer = pick_unary(rng);
                let inner = pick_binary(rng);
                let left = mutate_temporal(lhs, atoms, rng);
                let right = mutate_temporal(rhs, atoms, rng);
                Formula::unary(outer, Formula::binary(inner, left, right))
            }
            _ => {
                let left = mutate_temporal(lhs, atoms, rng);
                let right = mutate_temporal(rhs, atoms, rng);
                Formula::binary(*op, left, right)
            }
        },
    }
}

/// Mutates the maximal propositional subtrees and rebuilds every temporal
/// node around them, so the temporal skeleton is kept exactly.
fn mutate_propositional_parts(formula: &Formula, atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    if is_propositional(formula) {
        return mutate_boolean(formula, atoms, rng);
    }
    match formula.node() {
        Node::Unary(op, child) => {
            Formula::unary(*op, mutate_propositional_parts(child, atoms, rng))
        }
        Node::Binary(op, lhs, rhs) => {
            let left = mutate_propositional_parts(lhs, atoms, rng);
            let right = mutate_propositional_parts(rhs, atoms, rng);
            Formula::binary(*op, left, right)
        }
        _ => unreachable!("a leaf is propositional"),
    }
}

/// PEREDUR's `mutate_formula`: each node, bottom-up, is rewritten with
/// probability one in the formula's size. PEREDUR then removes double
/// negations; here the breeding step's `simplify` does that.
fn mutate_boolean(formula: &Formula, atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    let size = formula.n_subformulae();
    formula.rewrite_post_order(|sub| {
        if rng.below(size) != 0 {
            return None;
        }
        match sub.node() {
            Node::True | Node::False | Node::Atom(_) => Some(mutate_leaf(sub, atoms, rng)),
            Node::Unary(UnaryOp::Not, child) => Some(mutate_negation(child, atoms, rng)),
            Node::Binary(_, lhs, rhs) => Some(mutate_connective(lhs, rhs, rng)),
            Node::Unary(..) => None,
        }
    })
}

/// Rename the leaf, negate it, or graft a drawn literal onto it under a
/// drawn connective, on either side.
fn mutate_leaf(leaf: &Formula, atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    match rng.below(3) {
        0 => flip_or_replace(leaf, atoms, rng),
        1 => not(leaf.clone()),
        _ => {
            let anchor = draw_literal(atoms, rng);
            let op = pick_boolean(rng);
            let anchor_first = rng.coin();
            if anchor_first {
                Formula::binary(op, anchor, leaf.clone())
            } else {
                Formula::binary(op, leaf.clone(), anchor)
            }
        }
    }
}

/// At `!child`: drop the negation, keep it, or join a drawn atom to it.
fn mutate_negation(child: &Formula, atoms: &[Arc<str>], rng: &mut Rng) -> Formula {
    match rng.below(3) {
        0 => child.clone(),
        1 => not(child.clone()),
        _ => {
            let anchor = random_atom(atoms, rng);
            let op = pick_boolean(rng);
            Formula::binary(op, anchor, not(child.clone()))
        }
    }
}

/// At a boolean connective: keep one operand, or swap the connective and
/// perhaps negate the result.
fn mutate_connective(lhs: &Formula, rhs: &Formula, rng: &mut Rng) -> Formula {
    let swap = rng.coin();
    if !swap {
        let left = rng.coin();
        return if left { lhs.clone() } else { rhs.clone() };
    }
    let op = pick_boolean(rng);
    let combined = Formula::binary(op, lhs.clone(), rhs.clone());
    let negate = rng.coin();
    if negate {
        not(combined)
    } else {
        combined
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Polarity {
    Positive,
    Negative,
    Indeterminate,
}

impl Polarity {
    fn flip(self) -> Polarity {
        match self {
            Polarity::Positive => Polarity::Negative,
            Polarity::Negative => Polarity::Positive,
            Polarity::Indeterminate => Polarity::Indeterminate,
        }
    }
}

/// The polarity of each child of `formula`, given the formula's own.
fn child_polarities(formula: &Formula, polarity: Polarity) -> (Polarity, Polarity) {
    match formula.node() {
        Node::Unary(UnaryOp::Not, _) => (polarity.flip(), polarity.flip()),
        Node::Binary(BinaryOp::Implies, ..) => (polarity.flip(), polarity),
        Node::Binary(BinaryOp::Iff, ..) => (Polarity::Indeterminate, Polarity::Indeterminate),
        // And, Or, X, F, G, U, W and R are monotone in every argument.
        _ => (polarity, polarity),
    }
}

/// Every node's polarity, in pre-order.
fn collect_polarities(formula: &Formula, polarity: Polarity, out: &mut Vec<Polarity>) {
    out.push(polarity);
    let (first, second) = child_polarities(formula, polarity);
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => {}
        Node::Unary(_, child) => collect_polarities(child, first, out),
        Node::Binary(_, lhs, rhs) => {
            collect_polarities(lhs, first, out);
            collect_polarities(rhs, second, out);
        }
    }
}

/// One monotone rule, named for what it does at the node it fires at.
#[derive(Clone, Copy, Debug)]
enum Rule {
    /// φ to true, or to false.
    Constant,
    /// φ ∧ ψ to one operand (weaken), φ ∨ ψ to one (strengthen).
    DropOperand,
    /// φ to φ ∨ ℓ (weaken) or φ ∧ ℓ (strengthen), ℓ a drawn literal.
    AddOperand,
    /// G φ to F φ.
    GloballyToEventually,
    /// G φ to G F φ.
    GloballyToInfinitelyOften,
    /// F φ to G φ.
    EventuallyToGlobally,
    /// φ U ψ to φ W ψ.
    UntilToWeakUntil,
    /// φ W ψ to φ U ψ.
    WeakUntilToUntil,
    /// φ ↔ ψ to φ → ψ or ψ → φ.
    IffToImplies,
    /// φ ↔ ψ to φ ∧ ψ or ¬φ ∧ ¬ψ.
    IffToConjunction,
    /// φ R ψ to ψ.
    ReleaseToConsequent,
    /// φ R ψ to G ψ.
    ReleaseToGlobally,
    /// X φ to F φ.
    NextToEventually,
    /// X φ to G φ.
    NextToGlobally,
    /// φ to F φ (weaken) or G φ (strengthen).
    Introduce,
    /// G φ to φ (weaken), F φ to φ (strengthen).
    Strip,
    /// F G φ to G F φ (weaken), G F φ to F G φ (strengthen).
    ExchangeFG,
    /// φ U ψ to F ψ.
    UntilToEventually,
    /// φ U ψ or φ W ψ to ψ.
    UntilToConsequent,
    /// φ W ψ to φ ∨ ψ.
    WeakUntilToDisjunction,
    /// φ W ψ to G φ.
    WeakUntilToGlobally,
    /// φ R ψ to φ ∧ ψ.
    ReleaseToConjunction,
    /// φ R ψ to φ ∨ X ψ.
    ReleaseToNext,
    /// φ ∨ X ψ to φ R ψ.
    NextToRelease,
    /// φ ∧ X ψ to φ U ψ.
    NextToUntil,
    /// φ U ψ to φ ∧ X ψ.
    UntilToNext,
    /// F (φ ∧ ψ) to F φ ∧ F ψ.
    SplitEventually,
    /// F φ ∧ F ψ to F (φ ∧ ψ).
    JoinEventually,
    /// G φ ∧ F ψ, or F φ ∧ G ψ, to F (φ ∧ ψ).
    MeetEventually,
    /// F (φ ∧ ψ) to G φ ∧ F ψ or F φ ∧ G ψ.
    SplitMeet,
    /// F (φ R ψ) to φ R F ψ, F (φ W ψ) to F φ W F ψ.
    EventuallyInward,
    /// φ R F ψ to F (φ R ψ), F φ W F ψ to F (φ W ψ).
    EventuallyOutward,
    /// φ U G ψ to G (φ U ψ), φ W G ψ to G (φ W ψ).
    GloballyOutward,
    /// G (φ U ψ) to φ U G ψ, G (φ W ψ) to φ W G ψ.
    GloballyInward,
}

/// The rules of `set` that move `formula` the asked way. `Constant` and
/// `AddOperand` are sound at every node, so the list is never empty.
fn rules_at(formula: &Formula, weaken: bool, set: RuleSet) -> Vec<Rule> {
    let mut rules = vec![Rule::Constant, Rule::AddOperand];
    let extra: &[Rule] = match (formula.node(), weaken) {
        (Node::Binary(BinaryOp::And, ..), true) | (Node::Binary(BinaryOp::Or, ..), false) => {
            &[Rule::DropOperand]
        }
        (Node::Unary(UnaryOp::Globally, _), true) => {
            &[Rule::GloballyToEventually, Rule::GloballyToInfinitelyOften]
        }
        (Node::Unary(UnaryOp::Eventually, _), false) => &[Rule::EventuallyToGlobally],
        (Node::Binary(BinaryOp::Until, ..), true) => &[Rule::UntilToWeakUntil],
        (Node::Binary(BinaryOp::WeakUntil, ..), false) => &[Rule::WeakUntilToUntil],
        (Node::Binary(BinaryOp::Iff, ..), true) => &[Rule::IffToImplies],
        (Node::Binary(BinaryOp::Iff, ..), false) => &[Rule::IffToConjunction],
        (Node::Binary(BinaryOp::Release, ..), true) => &[Rule::ReleaseToConsequent],
        (Node::Binary(BinaryOp::Release, ..), false) => &[Rule::ReleaseToGlobally],
        (Node::Unary(UnaryOp::Next, _), true) => &[Rule::NextToEventually],
        (Node::Unary(UnaryOp::Next, _), false) => &[Rule::NextToGlobally],
        _ => &[],
    };
    rules.extend_from_slice(extra);
    if set == RuleSet::Ordered {
        ordered_rules(formula, weaken, &mut rules);
    }
    rules
}

fn is_unary(formula: &Formula, op: UnaryOp) -> bool {
    matches!(formula.node(), Node::Unary(o, _) if *o == op)
}

fn is_binary(formula: &Formula, op: BinaryOp) -> bool {
    matches!(formula.node(), Node::Binary(o, ..) if *o == op)
}

/// The order-guided rules at `formula`, mirrored in `proposedAt` of
/// `proofs/Gawain/Rules.lean`.
fn ordered_rules(formula: &Formula, weaken: bool, rules: &mut Vec<Rule>) {
    use BinaryOp::{And, Or, Release, Until, WeakUntil};
    use UnaryOp::{Eventually, Globally, Next};
    rules.push(Rule::Introduce);
    match formula.node() {
        Node::Unary(op, child) => {
            if (*op == Globally) == weaken && matches!(op, Globally | Eventually) {
                rules.push(Rule::Strip);
            }
            let (outer, inner) = if weaken {
                (Eventually, Globally)
            } else {
                (Globally, Eventually)
            };
            if *op == outer && is_unary(child, inner) {
                rules.push(Rule::ExchangeFG);
            }
            if *op == Eventually && is_binary(child, And) {
                rules.push(if weaken {
                    Rule::SplitEventually
                } else {
                    Rule::SplitMeet
                });
            }
            if weaken
                && *op == Eventually
                && (is_binary(child, Release) || is_binary(child, WeakUntil))
            {
                rules.push(Rule::EventuallyInward);
            }
            if !weaken
                && *op == Globally
                && (is_binary(child, Until) || is_binary(child, WeakUntil))
            {
                rules.push(Rule::GloballyInward);
            }
        }
        Node::Binary(op, lhs, rhs) => match (op, weaken) {
            (Until, true) => {
                rules.push(Rule::UntilToEventually);
                if is_unary(rhs, Globally) {
                    rules.push(Rule::GloballyOutward);
                }
            }
            (Until, false) => rules.extend([Rule::UntilToConsequent, Rule::UntilToNext]),
            (WeakUntil, true) => {
                rules.push(Rule::WeakUntilToDisjunction);
                if is_unary(rhs, Globally) {
                    rules.push(Rule::GloballyOutward);
                }
            }
            (WeakUntil, false) => {
                rules.extend([Rule::UntilToConsequent, Rule::WeakUntilToGlobally]);
                if is_unary(lhs, Eventually) && is_unary(rhs, Eventually) {
                    rules.push(Rule::EventuallyOutward);
                }
            }
            (Release, true) => rules.push(Rule::ReleaseToNext),
            (Release, false) => {
                rules.push(Rule::ReleaseToConjunction);
                if is_unary(rhs, Eventually) {
                    rules.push(Rule::EventuallyOutward);
                }
            }
            (And, true) => {
                if is_unary(rhs, Next) {
                    rules.push(Rule::NextToUntil);
                }
                if (is_unary(lhs, Globally) && is_unary(rhs, Eventually))
                    || (is_unary(lhs, Eventually) && is_unary(rhs, Globally))
                {
                    rules.push(Rule::MeetEventually);
                }
            }
            (And, false) => {
                if is_unary(lhs, Eventually) && is_unary(rhs, Eventually) {
                    rules.push(Rule::JoinEventually);
                }
            }
            (Or, false) if is_unary(rhs, Next) => rules.push(Rule::NextToRelease),
            _ => {}
        },
        Node::True | Node::False | Node::Atom(_) => {}
    }
}

fn unary_operand(formula: &Formula) -> &Formula {
    match formula.node() {
        Node::Unary(_, child) => child,
        _ => unreachable!("rules_at files this rule under a unary node"),
    }
}

fn binary_operands(formula: &Formula) -> (&Formula, &Formula) {
    match formula.node() {
        Node::Binary(_, lhs, rhs) => (lhs, rhs),
        _ => unreachable!("rules_at files this rule under a binary node"),
    }
}

fn apply_rule(
    formula: &Formula,
    rule: Rule,
    weaken: bool,
    atoms: &[Arc<str>],
    rng: &mut Rng,
) -> Formula {
    // The literal and then one coin, only where a rule needs them, as
    // PEREDUR draws.
    let literal = match rule {
        Rule::AddOperand => draw_literal(atoms, rng),
        _ => Formula::tt(),
    };
    let mut choices = outcomes(formula, rule, weaken, &literal);
    let first = choices.len() == 1 || rng.coin();
    choices.swap_remove(if first { 0 } else { 1 })
}

/// What `rule` builds from `formula`, one formula per coin outcome, the
/// outcome of a true coin first. `literal` is the drawn literal of
/// `AddOperand`.
fn outcomes(formula: &Formula, rule: Rule, weaken: bool, literal: &Formula) -> Vec<Formula> {
    use BinaryOp::{And, Implies, Or, Release, Until, WeakUntil};
    use UnaryOp::{Eventually, Globally, Next};
    let un = |op, x: &Formula| Formula::unary(op, x.clone());
    let bin = |op, x: &Formula, y: &Formula| Formula::binary(op, x.clone(), y.clone());
    match rule {
        Rule::Constant if weaken => vec![Formula::tt()],
        Rule::Constant => vec![Formula::ff()],
        Rule::DropOperand => {
            let (lhs, rhs) = binary_operands(formula);
            vec![lhs.clone(), rhs.clone()]
        }
        Rule::AddOperand => {
            let op = if weaken { Or } else { And };
            vec![bin(op, formula, literal)]
        }
        Rule::GloballyToEventually | Rule::NextToEventually => {
            vec![un(Eventually, unary_operand(formula))]
        }
        // PEREDUR builds `G` of the whole node here, `G G φ`, which is
        // equivalent to its input. The rule's documented `G F φ` is built
        // instead.
        Rule::GloballyToInfinitelyOften => {
            vec![un(Globally, &un(Eventually, unary_operand(formula)))]
        }
        Rule::EventuallyToGlobally | Rule::NextToGlobally => {
            vec![un(Globally, unary_operand(formula))]
        }
        Rule::UntilToWeakUntil => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(WeakUntil, lhs, rhs)]
        }
        Rule::WeakUntilToUntil => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(Until, lhs, rhs)]
        }
        Rule::IffToImplies => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(Implies, lhs, rhs), bin(Implies, rhs, lhs)]
        }
        Rule::IffToConjunction => {
            let (lhs, rhs) = binary_operands(formula);
            vec![
                bin(And, lhs, rhs),
                bin(And, &not(lhs.clone()), &not(rhs.clone())),
            ]
        }
        Rule::ReleaseToConsequent | Rule::UntilToConsequent => {
            vec![binary_operands(formula).1.clone()]
        }
        Rule::ReleaseToGlobally => vec![un(Globally, binary_operands(formula).1)],
        Rule::Introduce => vec![un(if weaken { Eventually } else { Globally }, formula)],
        Rule::Strip => vec![unary_operand(formula).clone()],
        Rule::ExchangeFG => {
            let (outer, inner) = if weaken {
                (Globally, Eventually)
            } else {
                (Eventually, Globally)
            };
            let x = unary_operand(unary_operand(formula));
            vec![un(outer, &un(inner, x))]
        }
        Rule::UntilToEventually => vec![un(Eventually, binary_operands(formula).1)],
        Rule::WeakUntilToDisjunction => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(Or, lhs, rhs)]
        }
        Rule::WeakUntilToGlobally => vec![un(Globally, binary_operands(formula).0)],
        Rule::ReleaseToConjunction => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(And, lhs, rhs)]
        }
        Rule::ReleaseToNext => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(Or, lhs, &un(Next, rhs))]
        }
        Rule::NextToRelease => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(Release, lhs, unary_operand(rhs))]
        }
        Rule::NextToUntil => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(Until, lhs, unary_operand(rhs))]
        }
        Rule::UntilToNext => {
            let (lhs, rhs) = binary_operands(formula);
            vec![bin(And, lhs, &un(Next, rhs))]
        }
        Rule::SplitEventually => {
            let (x, y) = binary_operands(unary_operand(formula));
            vec![bin(And, &un(Eventually, x), &un(Eventually, y))]
        }
        Rule::JoinEventually | Rule::MeetEventually => {
            let (lhs, rhs) = binary_operands(formula);
            vec![un(
                Eventually,
                &bin(And, unary_operand(lhs), unary_operand(rhs)),
            )]
        }
        Rule::SplitMeet => {
            let (x, y) = binary_operands(unary_operand(formula));
            vec![
                bin(And, &un(Globally, x), &un(Eventually, y)),
                bin(And, &un(Eventually, x), &un(Globally, y)),
            ]
        }
        Rule::EventuallyInward => {
            let inner = unary_operand(formula);
            let (x, y) = binary_operands(inner);
            if is_binary(inner, Release) {
                vec![bin(Release, x, &un(Eventually, y))]
            } else {
                vec![bin(WeakUntil, &un(Eventually, x), &un(Eventually, y))]
            }
        }
        Rule::EventuallyOutward => {
            let (lhs, rhs) = binary_operands(formula);
            if is_binary(formula, Release) {
                vec![un(Eventually, &bin(Release, lhs, unary_operand(rhs)))]
            } else {
                let w = bin(WeakUntil, unary_operand(lhs), unary_operand(rhs));
                vec![un(Eventually, &w)]
            }
        }
        Rule::GloballyOutward => {
            let (lhs, rhs) = binary_operands(formula);
            let op = if is_binary(formula, Until) {
                Until
            } else {
                WeakUntil
            };
            vec![un(Globally, &bin(op, lhs, unary_operand(rhs)))]
        }
        Rule::GloballyInward => {
            let inner = unary_operand(formula);
            let (x, y) = binary_operands(inner);
            let op = if is_binary(inner, Until) {
                Until
            } else {
                WeakUntil
            };
            vec![bin(op, x, &un(Globally, y))]
        }
    }
}

/// The node a monotone rewrite fires at, by pre-order index.
struct Site<'a> {
    target: usize,
    want_weaker: bool,
    atoms: &'a [Arc<str>],
    rules: RuleSet,
}

impl Site<'_> {
    /// Rebuilds `formula` with one rule applied at the target. The walk
    /// numbers nodes in the pre-order of [`collect_polarities`] and stops
    /// descending at the target.
    fn rewrite(
        &self,
        formula: &Formula,
        polarity: Polarity,
        next: &mut usize,
        rng: &mut Rng,
    ) -> Formula {
        let index = *next;
        *next += 1;
        if index == self.target {
            let weaken_here = (polarity == Polarity::Positive) == self.want_weaker;
            let rules = rules_at(formula, weaken_here, self.rules);
            let rule = rules[rng.below(rules.len())];
            return apply_rule(formula, rule, weaken_here, self.atoms, rng);
        }
        let (first, second) = child_polarities(formula, polarity);
        match formula.node() {
            Node::True | Node::False | Node::Atom(_) => formula.clone(),
            Node::Unary(op, child) => Formula::unary(*op, self.rewrite(child, first, next, rng)),
            Node::Binary(op, lhs, rhs) => {
                let left = self.rewrite(lhs, first, next, rng);
                let right = self.rewrite(rhs, second, next, rng);
                Formula::binary(*op, left, right)
            }
        }
    }
}

/// The distinct temporal subformulae in post-order, or the whole formula
/// when it has none.
fn graft_sites(formula: &Formula) -> Vec<Formula> {
    fn walk(formula: &Formula, out: &mut Vec<Formula>) {
        match formula.node() {
            Node::True | Node::False | Node::Atom(_) => {}
            Node::Unary(_, child) => walk(child, out),
            Node::Binary(_, lhs, rhs) => {
                walk(lhs, out);
                walk(rhs, out);
            }
        }
        if is_temporal(formula) && !out.contains(formula) {
            out.push(formula.clone());
        }
    }
    let mut sites = Vec::new();
    walk(formula, &mut sites);
    if sites.is_empty() {
        sites.push(formula.clone());
    }
    sites
}

/// `subject` with its first occurrence of `pattern`, in post-order, replaced.
fn replace_first(subject: &Formula, pattern: &Formula, replacement: &Formula) -> Formula {
    let mut replaced = false;
    subject.rewrite_post_order(|sub| {
        if replaced || sub != pattern {
            return None;
        }
        replaced = true;
        Some(replacement.clone())
    })
}

fn replace_subformula(into: &Formula, from: &Formula, rng: &mut Rng) -> Formula {
    let sites = graft_sites(into);
    let donors = graft_sites(from);
    let site = rng.below(sites.len());
    let donor = rng.below(donors.len());
    replace_first(into, &sites[site], &donors[donor])
}

fn combine_subformula(into: &Formula, from: &Formula, rng: &mut Rng) -> Formula {
    let sites = graft_sites(into);
    let donors = graft_sites(from);
    let site = &sites[rng.below(sites.len())];
    let donor = &donors[rng.below(donors.len())];
    let join = rng.below(4);
    let joined = if join < 2 {
        let op = if join == 0 {
            BinaryOp::And
        } else {
            BinaryOp::Or
        };
        Formula::binary(op, site.clone(), donor.clone())
    } else {
        let op = if join == 2 {
            BinaryOp::Until
        } else {
            BinaryOp::WeakUntil
        };
        let donor_on_the_right = rng.coin();
        if donor_on_the_right {
            Formula::binary(op, site.clone(), donor.clone())
        } else {
            Formula::binary(op, donor.clone(), site.clone())
        }
    };
    replace_first(into, site, &joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ltl::tests::{f, random_formulas};

    fn pool(names: &[&str]) -> Vec<Arc<str>> {
        names.iter().map(|&n| Arc::from(n)).collect()
    }

    #[test]
    fn same_seed_same_offspring() {
        let atoms = pool(&["a", "b", "c"]);
        let run = |seed| {
            let mut rng = Rng::new(seed);
            random_formulas(100, 11)
                .iter()
                .map(|x| {
                    let m = mutate(x, &atoms, 0.25, None, RuleSet::Peredur, &mut rng);
                    crossover(&m, x, &mut rng)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }

    #[test]
    fn mutations_draw_atoms_from_the_pool_only() {
        let atoms = pool(&["a", "b"]);
        let mut rng = Rng::new(3);
        let phi = f("G (a -> X b)");
        let mut changed = 0;
        for _ in 0..500 {
            let child = mutate(&phi, &atoms, 0.25, None, RuleSet::Peredur, &mut rng);
            assert!(child.atoms().iter().all(|a| atoms.contains(a)), "{child}");
            changed += usize::from(child != phi);
        }
        assert!(changed > 400, "{changed} of 500 changed");
    }

    #[test]
    fn an_empty_pool_leaves_the_formula_alone() {
        let mut rng = Rng::new(3);
        let phi = f("G true");
        assert_eq!(
            mutate(&phi, &[], 0.25, None, RuleSet::Peredur, &mut rng),
            phi
        );
    }

    #[test]
    fn a_forced_direction_keeps_the_coin_draw() {
        // At share 1 every mutation is a monotone rewrite, and a forced one
        // is that direction's rewrite after the same coin draw.
        let atoms = pool(&["a", "b", "c"]);
        for direction in [Direction::Weaken, Direction::Strengthen] {
            for (seed, x) in random_formulas(200, 4).iter().enumerate() {
                let mut rng = Rng::new(seed as u64);
                let m = mutate(x, &atoms, 1.0, Some(direction), RuleSet::Peredur, &mut rng);
                let mut rng = Rng::new(seed as u64);
                assert!(rng.chance(1.0));
                rng.coin();
                assert_eq!(
                    m,
                    monotone_rewrite(x, direction, &atoms, RuleSet::Peredur, &mut rng),
                    "{x}"
                );
            }
        }
    }

    #[test]
    fn the_propositional_path_keeps_the_temporal_skeleton() {
        fn skeleton(formula: &Formula) -> Vec<String> {
            let mut out = Vec::new();
            fn walk(formula: &Formula, out: &mut Vec<String>) {
                if is_temporal(formula) {
                    let op = match formula.node() {
                        Node::Unary(op, _) => format!("{op:?}"),
                        Node::Binary(op, ..) => format!("{op:?}"),
                        _ => unreachable!(),
                    };
                    out.push(op);
                }
                match formula.node() {
                    Node::Unary(_, c) => walk(c, out),
                    Node::Binary(_, l, r) => {
                        walk(l, out);
                        walk(r, out);
                    }
                    _ => {}
                }
            }
            walk(formula, &mut out);
            out
        }
        let atoms = pool(&["a", "b"]);
        let mut rng = Rng::new(8);
        let phi = f("G (a -> X (b U a)) & F !b");
        for _ in 0..300 {
            let child = mutate_propositional_parts(&phi, &atoms, &mut rng);
            assert_eq!(skeleton(&child), skeleton(&phi), "{child}");
        }
    }

    #[test]
    fn polarity_flips_under_negation_and_antecedents() {
        let mut out = Vec::new();
        collect_polarities(&f("!a -> (b <-> c)"), Polarity::Positive, &mut out);
        use Polarity::*;
        assert_eq!(
            out,
            [
                Positive,
                Negative,
                Positive,
                Positive,
                Indeterminate,
                Indeterminate
            ]
        );
    }

    #[test]
    fn rules_follow_the_node_and_the_direction() {
        let names = |formula: &str, weaken| -> Vec<String> {
            rules_at(&f(formula), weaken, RuleSet::Peredur)
                .iter()
                .map(|r| format!("{r:?}"))
                .collect()
        };
        assert_eq!(names("a", true), ["Constant", "AddOperand"]);
        assert_eq!(
            names("G a", true),
            [
                "Constant",
                "AddOperand",
                "GloballyToEventually",
                "GloballyToInfinitelyOften"
            ]
        );
        assert_eq!(names("G a", false), ["Constant", "AddOperand"]);
        assert_eq!(names("a | b", false)[2], "DropOperand");
        assert_eq!(names("a & b", true)[2], "DropOperand");
        assert_eq!(names("X a", false)[2], "NextToGlobally");
    }

    #[test]
    fn the_ordered_set_adds_rules_and_keeps_peredur_first() {
        let names = |formula: &str, weaken| -> Vec<String> {
            rules_at(&f(formula), weaken, RuleSet::Ordered)
                .iter()
                .map(|r| format!("{r:?}"))
                .collect()
        };
        assert_eq!(
            names("G a", true),
            [
                "Constant",
                "AddOperand",
                "GloballyToEventually",
                "GloballyToInfinitelyOften",
                "Introduce",
                "Strip"
            ]
        );
        assert_eq!(
            names("F (a & b)", false)[3..],
            ["Introduce", "Strip", "SplitMeet"]
        );
        assert_eq!(
            names("G a & F b", true)[3..],
            ["Introduce", "MeetEventually"]
        );
        assert_eq!(names("a | X b", false)[3..], ["Introduce", "NextToRelease"]);
        assert_eq!(names("a", true), ["Constant", "AddOperand", "Introduce"]);
    }

    #[test]
    fn outcomes_list_every_coin_and_site() {
        let literal = f("c");
        let weaker = monotone_outcomes(&f("a & b"), Direction::Weaken, RuleSet::Peredur, &literal);
        assert_eq!(
            weaker,
            [
                f("true"),
                f("(a & b) | c"),
                f("a"),
                f("b"),
                f("true & b"),
                f("(a | c) & b"),
                f("a & true"),
                f("a & (b | c)"),
            ]
        );
        // The same draws as apply_rule: DropOperand on a true coin keeps the
        // left operand.
        let both = outcomes(&f("F (a & b)"), Rule::SplitMeet, false, &literal);
        assert_eq!(both, [f("G a & F b"), f("F a & G b")]);
    }

    #[test]
    fn globally_to_infinitely_often_builds_g_f() {
        let mut rng = Rng::new(0);
        let rewritten = apply_rule(
            &f("G a"),
            Rule::GloballyToInfinitelyOften,
            true,
            &pool(&["a"]),
            &mut rng,
        );
        assert_eq!(rewritten, f("G F a"));
    }

    #[test]
    fn a_formula_under_a_biconditional_only_rewrites_the_root() {
        let atoms = pool(&["a"]);
        let mut rng = Rng::new(2);
        let phi = f("a <-> b");
        for _ in 0..100 {
            let child =
                monotone_rewrite(&phi, Direction::Weaken, &atoms, RuleSet::Peredur, &mut rng);
            let at_root = [
                f("true"),
                f("(a <-> b) | a"),
                f("(a <-> b) | !a"),
                f("a -> b"),
                f("b -> a"),
            ];
            assert!(at_root.contains(&child), "{child}");
        }
    }

    #[test]
    fn vacuity_witnesses_replace_each_determinate_site() {
        // `a & b`, `a` and `b` are negative and become true; `X c` and `c`
        // are positive and become false, both giving `G !(a & b)`. The
        // implication itself gives `G false`, which is left out.
        let pairs = |list: &[(&str, &str)]| -> Vec<(Formula, Formula)> {
            list.iter().map(|&(s, w)| (f(s), f(w))).collect()
        };
        assert_eq!(
            vacuity_witnesses(&f("G (a & b -> X c)")),
            pairs(&[
                ("a & b", "G X c"),
                ("a", "G (b -> X c)"),
                ("b", "G (a -> X c)"),
                ("X c", "G !(a & b)"),
                ("c", "G !(a & b)"),
            ])
        );
        // Under <-> nothing is a site but the root, which is skipped.
        assert!(vacuity_witnesses(&f("a <-> b")).is_empty());
        assert!(vacuity_witnesses(&f("G true")).is_empty());
    }

    #[test]
    fn graft_sites_are_distinct_temporal_subformulae() {
        let sites = graft_sites(&f("G a & (F b | G a)"));
        assert_eq!(sites, [f("G a"), f("F b")]);
        assert_eq!(graft_sites(&f("a & b")), [f("a & b")]);
    }

    #[test]
    fn replace_first_replaces_one_occurrence() {
        assert_eq!(
            replace_first(&f("G a & G a"), &f("G a"), &f("F b")),
            f("F b & G a")
        );
    }

    #[test]
    fn crossover_moves_material_from_the_donor() {
        let mut rng = Rng::new(4);
        let into = f("G (a -> X b)");
        let from = f("F c");
        let mut took = 0;
        for _ in 0..100 {
            let child = crossover(&into, &from, &mut rng);
            took += usize::from(child.atoms().contains("c"));
        }
        assert_eq!(took, 100);
    }
}
