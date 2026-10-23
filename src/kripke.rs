//! An explicit Kripke structure held in memory: the reachable states of a
//! model, their successors, and the atom that holds at each one.
//!
//! States are numbered from 0, and state 0 is the initial state. The
//! successor lists are stored as one flat array with an offset per state.
//! Every state has at least one successor, so every finite path extends to
//! an infinite run.
//!
//! At most one atom holds at each state. This fits the models built so far
//! (see [`crate::rers`]), whose atoms all read the same variable, and it
//! keeps a label to one integer per state.

use std::sync::Arc;

/// A state number.
pub type StateId = u32;

/// The marker for a state where no atom holds.
const NO_ATOM: u32 = u32::MAX;

/// A finite, total transition system with one optional atom per state.
#[derive(Clone, Debug)]
pub struct Kripke {
    atoms: Vec<Arc<str>>,
    labels: Vec<u32>,
    offsets: Vec<usize>,
    successors: Vec<StateId>,
}

impl Kripke {
    /// Starts an empty structure over the given atoms. States are then added
    /// in number order with [`Kripke::push_state`].
    pub(crate) fn new(atoms: Vec<Arc<str>>) -> Self {
        Kripke {
            atoms,
            labels: Vec::new(),
            offsets: vec![0],
            successors: Vec::new(),
        }
    }

    /// Adds the next state, with the index into the atom list of the atom
    /// that holds there and its successors, which must not be empty.
    pub(crate) fn push_state(&mut self, atom: Option<usize>, successors: &[StateId]) {
        debug_assert!(!successors.is_empty(), "every state needs a successor");
        debug_assert!(atom.is_none_or(|a| a < self.atoms.len()));
        self.labels.push(atom.map_or(NO_ATOM, |a| a as u32));
        self.successors.extend_from_slice(successors);
        self.offsets.push(self.successors.len());
    }

    /// The number of states.
    pub fn state_count(&self) -> usize {
        self.labels.len()
    }

    /// The number of transitions, counting each self-loop once.
    pub fn transition_count(&self) -> usize {
        self.successors.len()
    }

    /// The successors of `state`. Never empty.
    pub fn successors(&self, state: StateId) -> &[StateId] {
        let s = state as usize;
        &self.successors[self.offsets[s]..self.offsets[s + 1]]
    }

    /// Every atom, in index order.
    pub fn atoms(&self) -> &[Arc<str>] {
        &self.atoms
    }

    /// The index into [`Kripke::atoms`] of the atom that holds at `state`,
    /// if any.
    pub fn atom_index(&self, state: StateId) -> Option<usize> {
        match self.labels[state as usize] {
            NO_ATOM => None,
            a => Some(a as usize),
        }
    }

    /// The atom that holds at `state`, if any. No other atom holds there.
    pub fn atom(&self, state: StateId) -> Option<&Arc<str>> {
        match self.labels[state as usize] {
            NO_ATOM => None,
            a => Some(&self.atoms[a as usize]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_keep_their_successors_and_atom() {
        let mut k = Kripke::new(vec!["a".into(), "b".into()]);
        k.push_state(None, &[1, 2]);
        k.push_state(Some(1), &[1]);
        k.push_state(Some(0), &[0]);
        assert_eq!(k.state_count(), 3);
        assert_eq!(k.transition_count(), 4);
        assert_eq!(k.successors(0), &[1, 2]);
        assert_eq!(k.successors(2), &[0]);
        assert_eq!(k.atom(0), None);
        assert_eq!(k.atom(1).map(|a| &**a), Some("b"));
        assert_eq!(k.atom_index(1), Some(1));
        assert_eq!(k.atoms().len(), 2);
    }
}
