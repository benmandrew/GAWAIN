//! A front end for RERS parallel-track Promela models: it reads one `.pml`
//! file and explores its reachable states into a [`Kripke`] structure.
//!
//! # Input
//!
//! The parser accepts only the shapes that the RERS 2016, 2017 and 2019
//! parallel files use, and rejects any other line with its line number:
//!
//! - one `mtype = { ... };` list of actions, which includes `nop`;
//! - rendezvous channels, `chan c = [0] of {mtype};`, one of them `act`;
//! - `mtype lastAction = nop;`;
//! - an `Environment` process whose `do` loop offers `c ! m` on its channels;
//! - a `Listener` process that receives on `act` into `lastAction`;
//! - component processes with one local `int state`, written as
//!   `:: state == k -> atomic { if :: c ? m -> act ! m; state = j; ... fi }`,
//!   where a branch is a receive or a send, and `act ! m;` is optional;
//! - atoms `#define p (lastAction == m)`;
//! - `ltl` lines and one-line comments, which are skipped.
//!
//! The RERS 2018 files keep the component states in global variables and
//! guard each branch on the states of other components. That is a different
//! shape, and the parser rejects it.
//!
//! # Semantics
//!
//! A state is the local state of every component plus the last action. A
//! step is one of:
//!
//! - a component takes a receive branch on a channel where the environment
//!   offers that message;
//! - one component takes a send branch `c ! m` and another takes a receive
//!   branch `c ? m` together, as a rendezvous.
//!
//! If the step sends on `act`, the last action becomes that message;
//! otherwise it stays. Atom `p` holds at a state exactly when its
//! `lastAction == m` holds. A state with no step gets a self-loop, as SPIN's
//! stutter extension does.
//!
//! The first step of every component receives `nop` and sends nothing on
//! `act`. These steps are taken before exploration starts, so the initial
//! state is the one after all of them. Each such step is always enabled, is
//! the only branch of its state, records nothing, and commutes with every
//! other step, so moving it to the front of a run changes the run only by
//! stutter.
//!
//! The Listener process, the `atomic` blocks and the rendezvous hand-over
//! between the two sides of a step are not modelled. In SPIN these add
//! intermediate states where `lastAction` has not changed yet. An LTL
//! formula without `X` cannot tell a run with these stutter steps from one
//! without, so the structure keeps exactly what such a formula observes.
//! `docs/dev/rers-kripke.md` gives the reasons and the measured sizes.

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

use crate::kripke::{Kripke, StateId};

/// Why a Promela file was not read or not explored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// The 1-based line of the problem, or 0 for the file as a whole.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for Error {}

/// One branch of a component's `if`.
#[derive(Clone, Copy, Debug)]
struct Branch {
    send: bool,
    chan: u32,
    msg: u32,
    /// The message sent on `act`, if any.
    act: Option<u32>,
    target: u32,
}

/// A component process. Local states are numbered densely from 0.
#[derive(Debug)]
struct Component {
    initial: u32,
    /// The branches of each local state.
    branches: Vec<Vec<Branch>>,
}

/// A parsed RERS parallel model: its components, the environment's offers
/// and the atoms.
#[derive(Debug)]
pub struct Model {
    actions: Vec<String>,
    initial_action: u32,
    /// The (channel, message) pairs the environment offers.
    offers: Vec<(u32, u32)>,
    components: Vec<Component>,
    /// Each atom with the action it tests.
    atoms: Vec<(Arc<str>, u32)>,
}

fn error<T>(line: usize, message: impl Into<String>) -> Result<T, Error> {
    Err(Error {
        line,
        message: message.into(),
    })
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The source lines that matter, as (line number, text with each run of
/// whitespace made one space).
fn significant_lines(source: &str) -> Result<Vec<(usize, String)>, Error> {
    let mut lines = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        let mut text = raw.to_string();
        while let Some(start) = text.find("/*") {
            let Some(len) = text[start..].find("*/") else {
                return error(i + 1, "a comment that spans lines is not supported");
            };
            text.replace_range(start..start + len + 2, " ");
        }
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() || text.starts_with("ltl ") {
            continue;
        }
        lines.push((i + 1, text));
    }
    Ok(lines)
}

/// The names declared so far, and the parse position.
struct Parser {
    lines: Vec<(usize, String)>,
    pos: usize,
    actions: HashMap<String, u32>,
    chans: HashMap<String, u32>,
}

impl Parser {
    fn peek(&self) -> Option<&str> {
        self.lines.get(self.pos).map(|(_, t)| t.as_str())
    }

    fn line(&self) -> usize {
        self.lines.get(self.pos).map_or(0, |(n, _)| *n)
    }

    fn next(&mut self) -> Result<(usize, String), Error> {
        let Some(item) = self.lines.get(self.pos).cloned() else {
            return error(0, "unexpected end of file");
        };
        self.pos += 1;
        Ok(item)
    }

    fn expect(&mut self, want: &str) -> Result<(), Error> {
        let (n, text) = self.next()?;
        if text != want {
            return error(n, format!("expected `{want}`, found `{text}`"));
        }
        Ok(())
    }

    fn action(&self, line: usize, name: &str) -> Result<u32, Error> {
        match self.actions.get(name) {
            Some(&a) => Ok(a),
            None => error(line, format!("`{name}` is not a declared mtype")),
        }
    }

    fn chan(&self, line: usize, name: &str) -> Result<u32, Error> {
        match self.chans.get(name) {
            Some(&c) => Ok(c),
            None => error(line, format!("`{name}` is not a declared channel")),
        }
    }

    /// Parses `c ! m` or `c ? m` as (send, channel, message), refusing `act`.
    fn message(&self, line: usize, text: &str) -> Result<(bool, u32, u32), Error> {
        let parts: Vec<&str> = text.split(' ').collect();
        let [chan, op @ ("!" | "?"), msg] = parts[..] else {
            return error(line, format!("unsupported Promela: `{text}`"));
        };
        if chan == "act" {
            return error(line, "a branch cannot use `act` as its channel");
        }
        Ok((op == "!", self.chan(line, chan)?, self.action(line, msg)?))
    }
}

impl Model {
    /// Reads a RERS parallel Promela model, refusing any shape outside those
    /// listed in the module header.
    pub fn parse(source: &str) -> Result<Model, Error> {
        let mut p = Parser {
            lines: significant_lines(source)?,
            pos: 0,
            actions: HashMap::new(),
            chans: HashMap::new(),
        };
        let mut actions: Option<Vec<String>> = None;
        let mut initial_action = None;
        let mut offers = None;
        let mut listener = false;
        let mut components = Vec::new();
        let mut atoms = Vec::new();
        while p.peek().is_some() {
            let (n, text) = p.next()?;
            if let Some(list) = text
                .strip_prefix("mtype = {")
                .and_then(|t| t.strip_suffix("};"))
            {
                if actions.is_some() {
                    return error(n, "a second mtype list");
                }
                let names: Vec<String> = list.split(',').map(|s| s.trim().to_string()).collect();
                for (i, name) in names.iter().enumerate() {
                    if !is_ident(name) || p.actions.insert(name.clone(), i as u32).is_some() {
                        return error(n, format!("bad or repeated mtype `{name}`"));
                    }
                }
                actions = Some(names);
            } else if let Some(rest) = text.strip_prefix("chan ") {
                let Some(name) = rest.strip_suffix(" = [0] of {mtype};") else {
                    return error(n, format!("only rendezvous mtype channels: `{text}`"));
                };
                let id = p.chans.len() as u32;
                if !is_ident(name) || p.chans.insert(name.to_string(), id).is_some() {
                    return error(n, format!("bad or repeated channel `{name}`"));
                }
            } else if let Some(rest) = text.strip_prefix("mtype lastAction = ") {
                let Some(name) = rest.strip_suffix(';') else {
                    return error(n, format!("unsupported Promela: `{text}`"));
                };
                initial_action = Some(p.action(n, name)?);
            } else if let Some(rest) = text.strip_prefix("#define ") {
                let Some((name, test)) = rest.split_once(' ') else {
                    return error(n, format!("unsupported #define: `{text}`"));
                };
                let test = test.replace(' ', "");
                let Some(action) = test
                    .strip_prefix("(lastAction==")
                    .and_then(|t| t.strip_suffix(')'))
                else {
                    return error(n, format!("an atom must be `(lastAction == m)`: `{text}`"));
                };
                if !is_ident(name) {
                    return error(n, format!("bad atom name `{name}`"));
                }
                atoms.push((Arc::from(name), p.action(n, action)?));
            } else if text == "active proctype Environment()" {
                if offers.is_some() {
                    return error(n, "a second Environment");
                }
                offers = Some(parse_environment(&mut p)?);
            } else if text == "active proctype Listener()" {
                if listener {
                    return error(n, "a second Listener");
                }
                for want in [
                    "{",
                    "atomic {",
                    "do",
                    ":: act ? lastAction ->",
                    "step: skip",
                    "od",
                    "}",
                    "}",
                ] {
                    p.expect(want)?;
                }
                listener = true;
            } else if text.starts_with("active proctype ") && text.ends_with("()") {
                components.push(parse_component(&mut p)?);
            } else {
                return error(n, format!("unsupported Promela: `{text}`"));
            }
        }
        let (Some(actions), Some(initial_action), Some(offers), true) =
            (actions, initial_action, offers, listener)
        else {
            return error(
                0,
                "the model needs an mtype list, lastAction, an Environment and a Listener",
            );
        };
        if !p.chans.contains_key("act") {
            return error(0, "the model has no `act` channel");
        }
        if components.is_empty() {
            return error(0, "the model has no component process");
        }
        Ok(Model {
            actions,
            initial_action,
            offers,
            components,
            atoms,
        })
    }
}

fn parse_environment(p: &mut Parser) -> Result<Vec<(u32, u32)>, Error> {
    p.expect("{")?;
    p.expect("do")?;
    let mut offers = Vec::new();
    while let Some(text) = p.peek().and_then(|t| t.strip_prefix(":: ")) {
        let (send, chan, msg) = p.message(p.line(), text)?;
        if !send {
            return error(p.line(), "the Environment only sends");
        }
        offers.push((chan, msg));
        p.pos += 1;
    }
    p.expect("od")?;
    p.expect("}")?;
    offers.sort_unstable();
    offers.dedup();
    Ok(offers)
}

/// Reads `int state = N;` and the `do` loop of one component.
fn parse_component(p: &mut Parser) -> Result<Component, Error> {
    p.expect("{")?;
    let (n, text) = p.next()?;
    let Some(initial) = text
        .strip_prefix("int state = ")
        .and_then(|t| t.strip_suffix(';'))
        .and_then(|t| t.parse::<u32>().ok())
    else {
        return error(n, format!("expected `int state = N;`, found `{text}`"));
    };
    p.expect("do")?;
    // Branches by the value of `state`, before dense numbering.
    let mut blocks: Vec<(u32, Vec<Branch>)> = Vec::new();
    while p.peek() != Some("od") {
        let (n, text) = p.next()?;
        let Some(k) = text
            .strip_prefix(":: state == ")
            .and_then(|t| t.strip_suffix(" ->"))
            .and_then(|t| t.parse::<u32>().ok())
        else {
            return error(n, format!("expected `:: state == N ->`, found `{text}`"));
        };
        if blocks.iter().any(|(b, _)| *b == k) {
            return error(n, format!("a second block for state {k}"));
        }
        p.expect("atomic {")?;
        p.expect("if")?;
        let mut branches = Vec::new();
        while p.peek() != Some("fi") {
            branches.push(parse_branch(p)?);
        }
        p.expect("fi")?;
        p.expect("}")?;
        blocks.push((k, branches));
    }
    p.expect("od")?;
    p.expect("}")?;

    let mut values = vec![initial];
    for (k, branches) in &blocks {
        values.push(*k);
        values.extend(branches.iter().map(|b| b.target));
    }
    values.sort_unstable();
    values.dedup();
    let dense = |v: u32| values.binary_search(&v).unwrap() as u32;
    let mut dense_branches = vec![Vec::new(); values.len()];
    for (k, mut branches) in blocks {
        for b in &mut branches {
            b.target = dense(b.target);
        }
        dense_branches[dense(k) as usize] = branches;
    }
    Ok(Component {
        initial: dense(initial),
        branches: dense_branches,
    })
}

/// Reads `:: c ? m ->`, an optional `act ! a;`, and `state = j;`. The target
/// is the raw value of `state`.
fn parse_branch(p: &mut Parser) -> Result<Branch, Error> {
    let (n, text) = p.next()?;
    let Some(inner) = text.strip_prefix(":: ").and_then(|t| t.strip_suffix(" ->")) else {
        return error(
            n,
            format!("expected a branch `:: c ? m ->`, found `{text}`"),
        );
    };
    let (send, chan, msg) = p.message(n, inner)?;
    let (mut n, mut text) = p.next()?;
    let mut act = None;
    if let Some(name) = text
        .strip_prefix("act ! ")
        .and_then(|t| t.strip_suffix(';'))
    {
        act = Some(p.action(n, name)?);
        (n, text) = p.next()?;
    }
    let Some(target) = text
        .strip_prefix("state = ")
        .and_then(|t| t.strip_suffix(';'))
        .and_then(|t| t.parse::<u32>().ok())
    else {
        return error(n, format!("expected `state = N;`, found `{text}`"));
    };
    Ok(Branch {
        send,
        chan,
        msg,
        act,
        target,
    })
}

/// A cheap multiplicative hash for the packed state keys.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(5) ^ n).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }

    fn write_u128(&mut self, n: u128) {
        self.write_u64(n as u64);
        self.write_u64((n >> 64) as u64);
    }
}

/// A global state packed into one integer: each component's local state in
/// its own bit field, then the last action.
type Key = u128;

/// Where each field of a [`Key`] sits.
struct Layout {
    shifts: Vec<u32>,
    masks: Vec<Key>,
    action_shift: u32,
}

impl Layout {
    fn new(network: &Model) -> Result<Layout, Error> {
        let bits = |n: usize| usize::BITS - n.saturating_sub(1).leading_zeros();
        let mut shift = 0;
        let mut shifts = Vec::new();
        let mut masks = Vec::new();
        for c in &network.components {
            let width = bits(c.branches.len());
            shifts.push(shift);
            masks.push((1 << width) - 1);
            shift += width;
        }
        if shift + bits(network.actions.len()) > Key::BITS {
            return error(0, "the state does not fit in 128 bits");
        }
        Ok(Layout {
            shifts,
            masks,
            action_shift: shift,
        })
    }

    fn pack(&self, locals: &[u32], action: u32) -> Key {
        let mut key = (action as Key) << self.action_shift;
        for (i, &l) in locals.iter().enumerate() {
            key |= (l as Key) << self.shifts[i];
        }
        key
    }

    fn unpack(&self, key: Key, locals: &mut [u32]) -> u32 {
        for (i, l) in locals.iter_mut().enumerate() {
            *l = ((key >> self.shifts[i]) & self.masks[i]) as u32;
        }
        (key >> self.action_shift) as u32
    }
}

impl Model {
    /// The local state each component reaches by taking its silent `nop`
    /// receive, if that is its only branch, else its initial state.
    fn start(&self) -> Vec<u32> {
        let nop = self.actions.iter().position(|a| a == "nop");
        self.components
            .iter()
            .map(|c| match c.branches[c.initial as usize][..] {
                [b] if !b.send
                    && b.act.is_none()
                    && Some(b.msg as usize) == nop
                    && self.offers.binary_search(&(b.chan, b.msg)).is_ok() =>
                {
                    b.target
                }
                _ => c.initial,
            })
            .collect()
    }

    /// Explores the reachable states. Fails only when a state does not fit
    /// in 128 bits or there are more states than a [`StateId`] can number.
    pub fn explore(&self) -> Result<Kripke, Error> {
        let layout = Layout::new(self)?;
        let mut atom_of = vec![None; self.actions.len()];
        for (i, (_, action)) in self.atoms.iter().enumerate() {
            atom_of[*action as usize].get_or_insert(i);
        }
        let mut kripke = Kripke::new(self.atoms.iter().map(|(a, _)| a.clone()).collect());

        let mut index: HashMap<Key, StateId, BuildHasherDefault<KeyHasher>> = HashMap::default();
        let mut queue: Vec<Key> = Vec::new();
        let first = layout.pack(&self.start(), self.initial_action);
        index.insert(first, 0);
        queue.push(first);

        let mut locals = vec![0; self.components.len()];
        let mut next = locals.clone();
        let mut successors: Vec<StateId> = Vec::new();
        let mut head = 0;
        while head < queue.len() {
            let action = layout.unpack(queue[head], &mut locals);
            successors.clear();
            let mut add = |next: &[u32], act: Option<u32>| -> Result<(), Error> {
                let key = layout.pack(next, act.unwrap_or(action));
                let id = match index.get(&key) {
                    Some(&id) => id,
                    None => {
                        let Ok(id) = StateId::try_from(queue.len()) else {
                            return error(0, "more states than a state number can hold");
                        };
                        index.insert(key, id);
                        queue.push(key);
                        id
                    }
                };
                successors.push(id);
                Ok(())
            };
            for (i, c) in self.components.iter().enumerate() {
                for b in &c.branches[locals[i] as usize] {
                    if !b.send {
                        if self.offers.binary_search(&(b.chan, b.msg)).is_ok() {
                            next.copy_from_slice(&locals);
                            next[i] = b.target;
                            add(&next, b.act)?;
                        }
                        continue;
                    }
                    for (j, d) in self.components.iter().enumerate() {
                        if j == i {
                            continue;
                        }
                        for r in &d.branches[locals[j] as usize] {
                            if !r.send && r.chan == b.chan && r.msg == b.msg {
                                next.copy_from_slice(&locals);
                                next[i] = b.target;
                                next[j] = r.target;
                                add(&next, r.act.or(b.act))?;
                            }
                        }
                    }
                }
            }
            successors.sort_unstable();
            successors.dedup();
            if successors.is_empty() {
                successors.push(head as StateId);
            }
            kripke.push_state(atom_of[action as usize], &successors);
            head += 1;
        }
        Ok(kripke)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn explore(source: &str) -> Result<Kripke, Error> {
        Model::parse(source)?.explore()
    }

    /// Two components. Proc0 loops on `a` from the environment and hands
    /// `h` to Proc1, which then records `h`; Proc1 then deadlocks.
    const SMALL: &str = "\
/* Actions */
mtype = { nop, a, h };
chan p0 = [0] of {mtype};
chan p1 = [0] of {mtype};
chan p0_1 = [0] of {mtype};
active proctype Environment()
{
  do
  :: p0 ! a
  :: p0 ! nop
  :: p1 ! nop
  od
}
chan act = [0] of {mtype};
mtype lastAction = nop;
active proctype Listener()
{
  atomic {
    do
    :: act ? lastAction ->
  step: skip
    od
  }
}
active proctype Proc0()
{
  int state = 0;
  do
  :: state == 0 ->
    atomic {
      if
      :: p0 ? nop ->

        state = 1;
      fi
    }
  :: state == 1 ->
    atomic {
      if
      :: p0 ? a ->
        act ! a;
        state = 1;
      :: p0_1 ! h ->

        state = 2;
      fi
    }
  od
}
active proctype Proc1()
{
  int state = 5;
  do
  :: state == 5 ->
    atomic {
      if
      :: p1 ? nop ->

        state = 6;
      fi
    }
  :: state == 6 ->
    atomic {
      if
      :: p0_1 ? h ->
        act ! h;
        state = 7;
      fi
    }
  od
}
#define p_a\t (lastAction == a)
#define p_h\t (lastAction == h)
ltl p1\t { [] p_a }
";

    fn atom(k: &Kripke, s: StateId) -> Option<&str> {
        k.atom(s).map(|a| &**a)
    }

    #[test]
    fn explores_local_and_rendezvous_steps() {
        let k = explore(SMALL).unwrap();
        // (1, 6, nop) -> (1, 6, a) by the loop, or (2, 7, h) by the
        // rendezvous; (1, 6, a) loops or hands over; (2, 7, h) deadlocks.
        assert_eq!(k.state_count(), 3);
        assert_eq!(k.transition_count(), 5);
        assert_eq!(atom(&k, 0), None);
        let [loop_a, handed] = k.successors(0) else {
            panic!("two successors expected");
        };
        assert_eq!(atom(&k, *loop_a), Some("p_a"));
        assert_eq!(atom(&k, *handed), Some("p_h"));
        assert_eq!(k.successors(*loop_a), &[*loop_a, *handed]);
        assert_eq!(k.successors(*handed), &[*handed], "deadlock self-loop");
    }

    #[test]
    fn a_silent_step_keeps_the_last_action() {
        // Proc1 receives `h` without recording it, so the state after the
        // hand-over still shows `a`.
        let silent = SMALL.replace("act ! h;\n", "");
        let k = explore(&silent).unwrap();
        let loop_a = k.successors(0)[0];
        let handed = *k.successors(loop_a).last().unwrap();
        assert_eq!(atom(&k, handed), Some("p_a"));
    }

    #[test]
    fn rejects_unsupported_shapes() {
        // Each edit, the message words it should give, and the line it is on.
        let cases = [
            ("chan p1 = [0]", "chan p1 = [2]", "rendezvous"),
            ("int state = 5;", "int x = 5;", "int state"),
            ("act ! h;", "act ! zz;", "not a declared mtype"),
            (
                "#define p_h\t (lastAction == h)",
                "#define p_h (x > 1)",
                "lastAction",
            ),
            ("/* Actions */", "int state_0 = 16;", "unsupported Promela"),
        ];
        for (from, to, words) in cases {
            let line = 1 + SMALL[..SMALL.find(from).unwrap()].matches('\n').count();
            let e = explore(&SMALL.replace(from, to)).unwrap_err();
            assert_eq!(e.line, line, "{e}");
            assert!(e.message.contains(words), "{e}");
        }
        let e = explore("mtype = { nop };").unwrap_err();
        assert!(e.message.contains("needs"), "{e}");
    }
}
