//! A front end for RERS parallel-track Promela models: it reads one `.pml`
//! file and explores its reachable states into a [`Kripke`] structure.
//!
//! # Input
//!
//! The parser accepts only the shapes that the RERS 2016 to 2019 parallel
//! files use, and rejects any other line with its line number:
//!
//! - one `mtype = { ... };` list of actions, which includes `nop`;
//! - rendezvous channels, `chan c = [0] of {mtype};`, one of them `act`;
//! - `mtype lastAction = nop;`;
//! - an `Environment` process whose `do` loop offers `c ! m` on its channels;
//! - a `Listener` process that receives on `act` into `lastAction`;
//! - component processes, each a `do` loop of `:: state == k -> atomic { if
//!   ... fi }` blocks over one state variable;
//! - atoms `#define p (lastAction == m)`;
//! - `ltl` lines and one-line comments, which are skipped.
//!
//! The 2016, 2017 and 2019 files give each component a local `int state`.
//! A branch is a receive `c ? m ->` or a send `c ! m ->`, then an optional
//! `act ! m;` and `state = j;`.
//!
//! The 2018 files declare `int state_K = N;` globals, one per component in
//! the order of the processes. A branch is one of:
//!
//! - a receive with no output, `c ? m -> state_K = j`;
//! - a receive that records or passes on, `c ? m -> {state_K = j; act ! m}`
//!   or `c ? m -> {state_K = j; d ! m}`;
//! - a guarded send, `G -> {state_K = j; d ! m}`, where `G` is a conjunction
//!   of disjunctions of `state_I==N` over other components.
//!
//! # Semantics
//!
//! A state is the local state of every component plus the last action. A
//! step starts in one component, with a receive branch on a channel where
//! the environment offers that message, or with a send branch whose guard
//! holds. A send hands its message to a component with a matching receive
//! branch, which may send in turn, so one step can pass along a chain of
//! components. Every component on the chain takes its branch.
//!
//! If the step sends on `act`, the last action becomes that message;
//! otherwise it stays. Atom `p` holds at a state exactly when its
//! `lastAction == m` holds. A state with no step gets a self-loop, as SPIN's
//! stutter extension does.
//!
//! A send that begins its branch, as in 2016, is enabled only when some
//! component can receive it. A send after an assignment, as in 2018, would
//! block in SPIN with the assignment made. The guards are meant to rule this
//! out, and the explorer fails if a chain ever finds no receiver.
//!
//! The first step of every component receives `nop` and sends nothing on
//! `act`. These steps are taken before exploration starts, so the initial
//! state is the one after all of them. Each such step is always enabled, is
//! the only branch of its state, records nothing, and commutes with every
//! other step, so moving it to the front of a run changes the run only by
//! stutter.
//!
//! The Listener process, the `atomic` blocks and the rendezvous hand-over
//! between the components of a step are not modelled. In SPIN these add
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

/// One branch of a component's `if`. A branch with neither `recv` nor
/// `guard` begins with its send.
#[derive(Clone, Debug)]
struct Branch {
    /// The (channel, message) received first, if any.
    recv: Option<(u32, u32)>,
    /// For a 2018 send branch, each listed component must be in one of its
    /// listed local states, sorted.
    guard: Vec<(u32, Vec<u32>)>,
    /// The (channel, message) sent to another component, if any.
    send: Option<(u32, u32)>,
    /// The message sent on `act`, if any.
    act: Option<u32>,
    target: u32,
}

/// A component process. Local states are numbered densely from 0.
#[derive(Debug)]
struct Component {
    initial: u32,
    /// The value of the state variable in each local state, ascending.
    values: Vec<u32>,
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
    /// The 2018 `int state_K = N;` globals, as (name, initial value).
    globals: Vec<(String, u32)>,
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

    /// Parses a 2018 guard, `((state_1==4 || state_1==7) && state_2==3)`
    /// and the like, as (global index, values) pairs.
    fn guard(&self, line: usize, text: &str) -> Result<Vec<(u32, Vec<u32>)>, Error> {
        let bad = || error(line, format!("unsupported guard: `{text}`"));
        let text = text.replace(' ', "");
        let inner = text
            .strip_prefix('(')
            .and_then(|t| t.strip_suffix(')'))
            .unwrap_or(&text);
        let mut guard = Vec::new();
        for conjunct in inner.split("&&") {
            let conjunct = conjunct
                .strip_prefix('(')
                .and_then(|t| t.strip_suffix(')'))
                .unwrap_or(conjunct);
            let mut var = None;
            let mut values = Vec::new();
            for term in conjunct.split("||") {
                let Some((name, value)) = term.split_once("==") else {
                    return bad();
                };
                let Some(k) = self.globals.iter().position(|(g, _)| g == name) else {
                    return bad();
                };
                if var.is_some_and(|v| v != k) {
                    return bad();
                }
                var = Some(k);
                let Ok(value) = value.parse() else {
                    return bad();
                };
                values.push(value);
            }
            values.sort_unstable();
            guard.push((var.unwrap() as u32, values));
        }
        Ok(guard)
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
            globals: Vec::new(),
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
            } else if let Some((name, value)) = text
                .strip_prefix("int ")
                .and_then(|t| t.strip_suffix(';'))
                .and_then(|t| t.split_once(" = "))
            {
                let Ok(value) = value.parse() else {
                    return error(n, format!("unsupported Promela: `{text}`"));
                };
                if !is_ident(name) || p.globals.iter().any(|(g, _)| g == name) {
                    return error(n, format!("bad or repeated variable `{name}`"));
                }
                p.globals.push((name.to_string(), value));
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
                let index = components.len();
                components.push(parse_component(&mut p, index)?);
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
        if !p.globals.is_empty() && p.globals.len() != components.len() {
            return error(0, "the model needs one state variable per component");
        }
        // Guards name raw values; number them as the components do. A
        // value that is no state of its component never holds.
        let values: Vec<Vec<u32>> = components.iter().map(|c| c.values.clone()).collect();
        for c in &mut components {
            for b in c.branches.iter_mut().flatten() {
                for (k, allowed) in &mut b.guard {
                    let of = &values[*k as usize];
                    *allowed = (allowed.iter())
                        .filter_map(|v| of.binary_search(v).ok().map(|d| d as u32))
                        .collect();
                }
            }
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

/// Reads the state variable and the `do` loop of the component at `index`:
/// a local `int state = N;`, or else the `index`th global.
fn parse_component(p: &mut Parser, index: usize) -> Result<Component, Error> {
    p.expect("{")?;
    let local = p
        .peek()
        .and_then(|t| t.strip_prefix("int state = "))
        .and_then(|t| t.strip_suffix(';'))
        .and_then(|t| t.parse::<u32>().ok());
    let (var, initial) = match (local, p.globals.get(index)) {
        (Some(initial), _) => {
            p.pos += 1;
            ("state".to_string(), initial)
        }
        (None, Some(global)) => global.clone(),
        (None, None) => {
            let (n, text) = p.next()?;
            return error(n, format!("expected `int state = N;`, found `{text}`"));
        }
    };
    p.expect("do")?;
    // Branches by the value of the state variable, before dense numbering.
    let mut blocks: Vec<(u32, Vec<Branch>)> = Vec::new();
    while p.peek() != Some("od") {
        let (n, text) = p.next()?;
        let Some(k) = text
            .strip_prefix(&format!(":: {var} == "))
            .and_then(|t| t.strip_suffix(" ->"))
            .and_then(|t| t.parse::<u32>().ok())
        else {
            return error(n, format!("expected `:: {var} == N ->`, found `{text}`"));
        };
        if blocks.iter().any(|(b, _)| *b == k) {
            return error(n, format!("a second block for state {k}"));
        }
        p.expect("atomic {")?;
        p.expect("if")?;
        let mut branches = Vec::new();
        while p.peek() != Some("fi") {
            branches.push(parse_branch(p, &var)?);
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
        values,
    })
}

/// Parses `var = N` as N.
fn assignment(text: &str, var: &str) -> Option<u32> {
    text.strip_prefix(var)?.strip_prefix(" = ")?.parse().ok()
}

/// Reads one branch in either shape of the module header. The target is
/// the raw value of `var`, and guard values are raw too.
fn parse_branch(p: &mut Parser, var: &str) -> Result<Branch, Error> {
    let (n, text) = p.next()?;
    let Some(text) = text.strip_prefix(":: ") else {
        return error(n, format!("expected a branch `:: ...`, found `{text}`"));
    };
    let mut branch = Branch {
        recv: None,
        guard: Vec::new(),
        send: None,
        act: None,
        target: 0,
    };
    // 2016: `c ? m ->` alone, then lines.
    if let Some(head) = text.strip_suffix(" ->").filter(|h| !h.starts_with('(')) {
        let (send, chan, msg) = p.message(n, head)?;
        if send {
            branch.send = Some((chan, msg));
        } else {
            branch.recv = Some((chan, msg));
        }
        let (mut n, mut text) = p.next()?;
        if let Some(name) = text
            .strip_prefix("act ! ")
            .and_then(|t| t.strip_suffix(';'))
        {
            branch.act = Some(p.action(n, name)?);
            (n, text) = p.next()?;
        }
        let Some(target) = text.strip_suffix(';').and_then(|t| assignment(t, var)) else {
            return error(n, format!("expected `{var} = N;`, found `{text}`"));
        };
        branch.target = target;
        return Ok(branch);
    }
    // 2018: a receive and its body on one line, or a guard over one or more
    // lines and its body on the next.
    let (n, body) = if text.starts_with('(') {
        let mut guard = text.to_string();
        while !guard.ends_with(" ->") {
            guard.push(' ');
            guard.push_str(&p.next()?.1);
        }
        branch.guard = p.guard(n, guard.strip_suffix(" ->").unwrap())?;
        p.next()?
    } else if let Some((head, body)) = text.split_once(" -> ") {
        let (send, chan, msg) = p.message(n, head)?;
        if send {
            return error(n, format!("a 2018 send needs a guard: `{text}`"));
        }
        branch.recv = Some((chan, msg));
        (n, body.to_string())
    } else {
        return error(n, format!("unsupported branch: `{text}`"));
    };
    let (assign, out) = match body.strip_prefix('{').and_then(|b| b.strip_suffix('}')) {
        Some(inner) => match inner.split_once("; ") {
            Some((assign, out)) => (assign, Some(out)),
            None => return error(n, format!("unsupported branch body: `{body}`")),
        },
        None => (body.as_str(), None),
    };
    let Some(target) = assignment(assign, var) else {
        return error(n, format!("expected `{var} = N`, found `{assign}`"));
    };
    branch.target = target;
    match out {
        Some(out) => {
            if let Some(name) = out.strip_prefix("act ! ") {
                branch.act = Some(p.action(n, name)?);
            } else {
                let (send, chan, msg) = p.message(n, out)?;
                if !send {
                    return error(n, format!("a body may only send: `{body}`"));
                }
                branch.send = Some((chan, msg));
            }
        }
        None if branch.recv.is_none() => {
            return error(n, format!("a guarded branch must send: `{body}`"));
        }
        None => {}
    }
    Ok(branch)
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

/// Adds the state with these local states and this last action, if any.
type AddState<'a> = dyn FnMut(&[u32], Option<u32>) -> Result<(), Error> + 'a;

impl Model {
    /// The local state each component reaches by taking its silent `nop`
    /// receive, if that is its only branch, else its initial state.
    fn start(&self) -> Vec<u32> {
        let nop = self.actions.iter().position(|a| a == "nop");
        self.components
            .iter()
            .map(|c| match &c.branches[c.initial as usize][..] {
                [b] if b.send.is_none()
                    && b.act.is_none()
                    && b.recv.is_some_and(|(chan, msg)| {
                        Some(msg as usize) == nop && self.offers.binary_search(&(chan, msg)).is_ok()
                    }) =>
                {
                    b.target
                }
                _ => c.initial,
            })
            .collect()
    }

    /// Hands `message` from component `from` to each branch that can
    /// receive it in `next`, follows any send that branch makes in turn, and
    /// adds each state where the chain ends. Returns whether any branch
    /// received it; fails when a send after the first finds no receiver.
    fn deliver(
        &self,
        from: usize,
        message: (u32, u32),
        act: Option<u32>,
        next: &mut [u32],
        add: &mut AddState,
    ) -> Result<bool, Error> {
        let mut received = false;
        for (j, d) in self.components.iter().enumerate() {
            if j == from {
                continue;
            }
            let local = next[j];
            for r in &d.branches[local as usize] {
                if r.recv != Some(message) {
                    continue;
                }
                received = true;
                next[j] = r.target;
                let act = r.act.or(act);
                match r.send {
                    None => add(next, act)?,
                    Some(onward) => self.deliver_or_block(j, onward, act, next, add)?,
                }
                next[j] = local;
            }
        }
        Ok(received)
    }

    /// [`Model::deliver`] for a send that follows an assignment, which
    /// would block in SPIN without a receiver.
    fn deliver_or_block(
        &self,
        from: usize,
        message: (u32, u32),
        act: Option<u32>,
        next: &mut [u32],
        add: &mut AddState,
    ) -> Result<(), Error> {
        if self.deliver(from, message, act, next, add)? {
            return Ok(());
        }
        let (chan, msg) = message;
        error(
            0,
            format!(
                "component {from} blocks sending `{}` on channel {chan}",
                self.actions[msg as usize]
            ),
        )
    }

    /// Explores the reachable states. Fails when a state does not fit in
    /// 128 bits, when there are more states than a [`StateId`] can number,
    /// or when a send inside a step finds no receiver.
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
                    let enabled = match b.recv {
                        Some(offer) => self.offers.binary_search(&offer).is_ok(),
                        None => (b.guard.iter()).all(|(k, allowed)| {
                            allowed.binary_search(&locals[*k as usize]).is_ok()
                        }),
                    };
                    if !enabled {
                        continue;
                    }
                    next.copy_from_slice(&locals);
                    next[i] = b.target;
                    match b.send {
                        None => add(&next, b.act)?,
                        // A send that begins its branch waits for a receiver.
                        Some(message) if b.recv.is_none() && b.guard.is_empty() => {
                            self.deliver(i, message, b.act, &mut next, &mut add)?;
                        }
                        Some(message) => {
                            self.deliver_or_block(i, message, b.act, &mut next, &mut add)?
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
            ("/* Actions */", "int state_0 = x;", "unsupported Promela"),
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

    /// The 2018 shape. Proc1 loops on `a` from the environment. When Proc1
    /// is in state 11, Proc0 starts `h`, which Proc1 passes on to Proc2,
    /// which records it; then all three deadlock.
    const SMALL_2018: &str = "\
/* State variables */
int state_0 = 0;
int state_1 = 10;
int state_2 = 20;
mtype = { nop, a, h };
chan p0_1 = [0] of {mtype};
chan p1_2 = [0] of {mtype};
chan p0 = [0] of {mtype};
chan p1 = [0] of {mtype};
chan p2 = [0] of {mtype};
active proctype Environment()
{
  do
  :: p1 ! a
  :: p0 ! nop
  :: p1 ! nop
  :: p2 ! nop
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
  do
  :: state_0 == 0 ->
    atomic {
      if
      :: p0 ? nop -> state_0 = 1
      fi
    }
  :: state_0 == 1 ->
    atomic {
      if
      :: ((state_1==11 || state_1==99)
         && state_2==21) -> \n           {state_0 = 2; p0_1 ! h}
      fi
    }
  od
}
active proctype Proc1()
{
  do
  :: state_1 == 10 ->
    atomic {
      if
      :: p1 ? nop -> state_1 = 11
      fi
    }
  :: state_1 == 11 ->
    atomic {
      if
      :: p1 ? a -> {state_1 = 11; act ! a}
      :: p0_1 ? h -> {state_1 = 12; p1_2 ! h}
      fi
    }
  od
}
active proctype Proc2()
{
  do
  :: state_2 == 20 ->
    atomic {
      if
      :: p2 ? nop -> state_2 = 21
      fi
    }
  :: state_2 == 21 ->
    atomic {
      if
      :: p1_2 ? h -> {state_2 = 22; act ! h}
      fi
    }
  od
}
#define p_a\t (lastAction == a)
#define p_h\t (lastAction == h)
";

    #[test]
    fn a_guarded_send_passes_along_a_chain() {
        let k = explore(SMALL_2018).unwrap();
        // (1, 11, 21, nop) and (1, 11, 21, a) each loop on `a` or take the
        // chain to (2, 12, 22, h), which deadlocks.
        assert_eq!((k.state_count(), k.transition_count()), (3, 5));
        // Proc0 comes first, so the chain's state is numbered first.
        let [chain, loop_a] = k.successors(0) else {
            panic!("two successors expected");
        };
        assert_eq!(atom(&k, *loop_a), Some("p_a"));
        assert_eq!(atom(&k, *chain), Some("p_h"));
        assert_eq!(k.successors(*loop_a), &[*chain, *loop_a]);
        assert_eq!(k.successors(*chain), &[*chain]);
    }

    #[test]
    fn a_false_guard_disables_the_chain() {
        let k = explore(&SMALL_2018.replace("state_2==21", "state_2==22")).unwrap();
        assert_eq!((k.state_count(), k.transition_count()), (2, 2));
    }

    #[test]
    fn a_chain_with_no_receiver_fails() {
        let e = explore(&SMALL_2018.replace("p1_2 ? h", "p1_2 ? a")).unwrap_err();
        assert!(e.message.contains("blocks sending `h`"), "{e}");
    }

    #[test]
    fn rejects_unsupported_2018_shapes() {
        let cases = [
            ("state_2==21)", "state_9==21)", "unsupported guard"),
            ("state_2==21)", "state_2>21)", "unsupported guard"),
            (
                "{state_1 = 12; p1_2 ! h}",
                "{state_0 = 12; p1_2 ! h}",
                "state_1 = N",
            ),
            (
                "{state_1 = 12; p1_2 ! h}",
                "{state_1 = 12; p1_2 ? h}",
                "only send",
            ),
        ];
        for (from, to, words) in cases {
            let e = explore(&SMALL_2018.replace(from, to)).unwrap_err();
            assert!(e.message.contains(words), "{e}");
        }
    }
}
