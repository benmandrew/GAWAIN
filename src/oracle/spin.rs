//! The SPIN back end: batched claims against a Promela model.
//!
//! The compile of `pan.c` dominates a SPIN query, so [`Spin::check_all`]
//! builds one verifier per chunk of up to [`Spin::claims_per_build`]
//! candidates and runs `pan -a -N <claim>` once per candidate (see "Back
//! end 1: SPIN" in `docs/dev/oracle-performance.md`). A build has two
//! `spin -a` passes. The first translates each candidate, printed by
//! [`Formula::to_spin`], as an `ltl` block and keeps the never claims SPIN
//! writes to `_spin_nvr.tmp`. SPIN's translator ends a safety violation with
//! an `assert` in the claim, which gives a finite trail and no lasso, so the
//! claims are rewritten to move to an accepting self-loop instead, and the
//! second pass builds the verifier from them. Every violation is then an
//! acceptance cycle, a real infinite run of the model.
//!
//! # Atoms and observable positions
//!
//! An interface atom must be a model macro `#define p (v == c)`, with `v` a
//! global variable and `c` a constant, as in the RERS parallel models. Any
//! other atom is rejected when the back end is built. A violation's trail is
//! replayed with `spin -t -p -g -w`, which prints every global after every
//! step, so each atom's value is read from the printed value of its
//! variable.
//!
//! SPIN steps are finer than the model's observable steps. RERS sends each
//! action by rendezvous to a `Listener` process, which receives it into
//! `lastAction`; the guards, the inputs from the environment and the
//! listener's own `skip` are SPIN steps that no property can observe. So a
//! position of the returned lasso is the initial state or the state after a
//! step that receives into an atom variable (`chan?v`), which is exactly one
//! position per action, and a run with the same action twice keeps both. The
//! replay rejects a trail whose atom variable changes at any other step, and
//! a trail whose cycle does not return to its starting values. Between two
//! receives the valuation is constant, so SPIN's run is a stuttering of the
//! lasso, and for an `X`-free formula SPIN's verdict and the lasso agree.
//!
//! `X` counts SPIN steps, not positions, so on a model with unobservable
//! steps an `X` verdict describes SPIN's run, not the model's, and a wrong
//! `Holds` could not be caught. A formula with `X` is therefore answered
//! `Verdict::Unknown` without a build; the explicit Kripke back end, whose
//! steps are the model's actions, decides those. Every lasso is still checked
//! with [`Lasso::satisfies`] before it is returned, and one that satisfies
//! its formula becomes `Verdict::Unknown`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::process::{self, Limits, Output};
use super::{Backend, Lasso, Valuation, Verdict};
use crate::ltl::{Formula, Node, UnaryOp};

/// The most claims one `pan.c` may hold. Each claim is a proctype, and SPIN
/// allows 255: at 260 claims `pan` silently reported a violation of a
/// property that smaller builds verified. The model's own processes count
/// against the same limit, so a build holds this many less those.
pub const MAX_CLAIMS: usize = 200;

/// `pan`'s hash table size, 2^18 slots (2 MB). The default `-w24` costs
/// 128 MB and about 100 ms before the search starts.
const HASH_BITS: &str = "-w18";

/// `pan`'s search depth bound. Reaching it truncates the search, which is
/// then reported as `Verdict::Unknown`.
const DEPTH: &str = "-m200000";

/// Address space kept free of `-DMEMLIM` for the `pan` binary itself.
const PAN_HEADROOM_MB: u64 = 64;

/// A [`Backend`] that runs SPIN on a fixed Promela model.
pub struct Spin {
    model: String,
    atoms: Vec<Atom>,
    claims_per_build: usize,
    limits: Limits,
    dir: PathBuf,
}

/// An interface atom `name`, defined as `var == value`.
#[derive(Debug, PartialEq, Eq)]
struct Atom {
    name: Arc<str>,
    var: String,
    value: String,
}

impl Spin {
    /// A back end for the Promela model at `model`. The model's own `ltl`
    /// blocks are ignored. `atoms` are the model's interface, each a macro
    /// `#define p (v == c)`; each returned lasso records at every position
    /// which of them hold. Fails if the model cannot be read, an atom has
    /// another form, or the scratch directory cannot be made.
    pub fn new(
        model: impl AsRef<Path>,
        atoms: impl IntoIterator<Item = impl AsRef<str>>,
        limits: Limits,
    ) -> Result<Self, String> {
        let path = model.as_ref();
        let text =
            fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let atoms = atoms
            .into_iter()
            .map(|name| interface_atom(&text, name.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        let model = strip_ltl(&text);
        let claims_per_build = MAX_CLAIMS.saturating_sub(proctypes(&model)).max(1);
        Ok(Spin {
            model,
            atoms,
            claims_per_build,
            limits,
            dir: scratch_dir()?,
        })
    }

    /// How many claims one build holds: [`MAX_CLAIMS`] less the model's
    /// proctypes.
    pub fn claims_per_build(&self) -> usize {
        self.claims_per_build
    }

    /// Decides every formula, one verdict per formula in order. A formula
    /// with `X` or with an atom outside the interface is answered
    /// `Verdict::Unknown`. Each call to a tool runs under the back end's
    /// limits.
    pub fn check_all(&mut self, formulas: &[Formula]) -> Vec<Verdict> {
        let mut verdicts: Vec<Option<Verdict>> = vec![None; formulas.len()];
        let mut checked = Vec::new();
        for (i, formula) in formulas.iter().enumerate() {
            if let Some(name) = self.foreign_atom(formula) {
                let reason = format!("SPIN: atom {name} is not in the interface");
                verdicts[i] = Some(Verdict::Unknown(reason));
            } else if has_next(formula) {
                let reason = "SPIN: X counts SPIN steps, not the model's positions".to_string();
                verdicts[i] = Some(Verdict::Unknown(reason));
            } else {
                checked.push(i);
            }
        }
        for chunk in checked.chunks(self.claims_per_build) {
            let claims: Vec<&Formula> = chunk.iter().map(|&i| &formulas[i]).collect();
            for (&i, verdict) in chunk.iter().zip(self.check_chunk(&claims)) {
                verdicts[i] = Some(verdict);
            }
        }
        verdicts
            .into_iter()
            .map(|v| v.expect("every formula is answered"))
            .collect()
    }

    fn foreign_atom(&self, formula: &Formula) -> Option<Arc<str>> {
        match formula.node() {
            Node::True | Node::False => None,
            Node::Atom(name) => {
                let known = self.atoms.iter().any(|a| a.name == *name);
                (!known).then(|| name.clone())
            }
            Node::Unary(_, child) => self.foreign_atom(child),
            Node::Binary(_, lhs, rhs) => self.foreign_atom(lhs).or_else(|| self.foreign_atom(rhs)),
        }
    }

    fn check_chunk(&self, formulas: &[&Formula]) -> Vec<Verdict> {
        match self.build(formulas) {
            Ok(()) => formulas
                .iter()
                .enumerate()
                .map(|(k, formula)| self.search(k, formula))
                .collect(),
            Err(reason) => vec![Verdict::Unknown(format!("SPIN: {reason}")); formulas.len()],
        }
    }

    /// Writes `claims.pml` with claim `c<k>` for `formulas[k]`, and compiles
    /// its verifier to `pan`.
    fn build(&self, formulas: &[&Formula]) -> Result<(), String> {
        let mut ltl = self.model.clone();
        for (k, formula) in formulas.iter().enumerate() {
            let _ = writeln!(ltl, "ltl c{k} {{ {} }}", formula.to_spin());
        }
        let never = self.dir.join("_spin_nvr.tmp");
        let _ = fs::remove_file(&never);
        self.write("ltl.pml", &ltl)?;
        self.tool("spin", &["-a", "ltl.pml"])?;
        let claims = fs::read_to_string(&never).map_err(|e| format!("no never claims: {e}"))?;
        self.write(
            "claims.pml",
            &format!("{}{}", self.model, accepting(&claims)?),
        )?;
        self.tool("spin", &["-a", "claims.pml"])?;
        let memlim = (self.limits.memory_bytes >> 20).saturating_sub(PAN_HEADROOM_MB);
        let memlim = format!("-DMEMLIM={}", memlim.max(1));
        self.tool("gcc", &["-O0", &memlim, "-o", "pan", "pan.c"])?;
        Ok(())
    }

    fn search(&self, k: usize, formula: &Formula) -> Verdict {
        let unknown = |reason: String| Verdict::Unknown(format!("SPIN: {reason}"));
        let _ = fs::remove_file(self.dir.join("claims.pml.trail"));
        let pan = self.dir.join("pan");
        let claim = format!("c{k}");
        let args = ["-a", "-n", HASH_BITS, DEPTH, "-N", &claim];
        let output = match self.run(&pan.to_string_lossy(), &args) {
            Ok(output) => output,
            Err(reason) => return unknown(reason),
        };
        match search_result(&output.stdout) {
            Ok(false) => Verdict::Holds,
            Ok(true) => match self.replay() {
                Ok(lasso) if lasso.satisfies(formula) => unknown(format!(
                    "the counterexample satisfies {formula} over observable positions"
                )),
                Ok(lasso) => Verdict::Fails(lasso),
                Err(reason) => unknown(reason),
            },
            Err(reason) => unknown(reason),
        }
    }

    fn replay(&self) -> Result<Lasso, String> {
        let output = self.tool("spin", &["-t", "-p", "-g", "-w", "-b", "claims.pml"])?;
        lasso(&output.stdout, &self.atoms)
    }

    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        fs::write(self.dir.join(name), text).map_err(|e| format!("cannot write {name}: {e}"))
    }

    /// Runs a tool in the scratch directory and requires it to succeed.
    fn tool(&self, program: &str, args: &[&str]) -> Result<Output, String> {
        let output = self.run(program, args)?;
        if !output.status.success() {
            return Err(format!(
                "{program} {} failed: {}{}",
                args.join(" "),
                output.stdout.trim(),
                output.stderr.trim()
            ));
        }
        Ok(output)
    }

    fn run(&self, program: &str, args: &[&str]) -> Result<Output, String> {
        let mut command = Command::new(program);
        command.args(args).current_dir(&self.dir);
        process::run(&mut command, "", self.limits).map_err(|e| format!("{program}: {e}"))
    }
}

impl Backend for Spin {
    fn check(&mut self, formula: &Formula) -> Verdict {
        let mut verdicts = self.check_all(slice::from_ref(formula));
        verdicts.pop().expect("one verdict per formula")
    }
}

impl Drop for Spin {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn scratch_dir() -> Result<PathBuf, String> {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("gawain-spin-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

fn has_next(formula: &Formula) -> bool {
    match formula.node() {
        Node::True | Node::False | Node::Atom(_) => false,
        Node::Unary(op, child) => *op == UnaryOp::Next || has_next(child),
        Node::Binary(_, lhs, rhs) => has_next(lhs) || has_next(rhs),
    }
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Reads atom `name`'s `#define name (var == value)` from the model.
fn interface_atom(model: &str, name: &str) -> Result<Atom, String> {
    let body = model
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("#define"))
        .filter_map(|rest| {
            let rest = rest.trim_start();
            let after = rest.strip_prefix(name)?;
            after.starts_with([' ', '\t']).then(|| after.trim())
        })
        .next()
        .ok_or_else(|| format!("atom {name} has no #define in the model"))?;
    let unsupported = || {
        format!("atom {name} is defined as {body:?}; only `(variable == constant)` is supported")
    };
    let mut inner = body;
    while let Some(stripped) = inner.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        inner = stripped.trim();
    }
    let (var, value) = inner.split_once("==").ok_or_else(unsupported)?;
    let (var, value) = (var.trim(), value.trim());
    let constant = is_identifier(value) || value.parse::<i64>().is_ok();
    if !is_identifier(var) || !constant {
        return Err(unsupported());
    }
    Ok(Atom {
        name: Arc::from(name),
        var: var.to_owned(),
        value: value.to_owned(),
    })
}

/// The model without its `ltl` blocks, each from a line that starts with
/// `ltl` to the brace that closes it.
fn strip_ltl(model: &str) -> String {
    let mut out = String::with_capacity(model.len());
    // Inside a block: the brace depth, and whether its `{` was seen.
    let mut block: Option<(usize, bool)> = None;
    for line in model.split_inclusive('\n') {
        if block.is_none() {
            let starts = line
                .trim_start()
                .strip_prefix("ltl")
                .is_some_and(|rest| rest.starts_with(|c: char| c.is_whitespace() || c == '{'));
            if !starts {
                out.push_str(line);
                continue;
            }
            block = Some((0, false));
        }
        let (depth, opened) = block.as_mut().expect("inside an ltl block");
        for c in line.chars() {
            match c {
                '{' => {
                    *depth += 1;
                    *opened = true;
                }
                '}' => *depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if *opened && *depth == 0 {
            block = None;
        }
    }
    out
}

/// The model's proctypes and `init`, counted as keyword tokens. A keyword
/// in a comment is counted too, which only makes a build smaller.
fn proctypes(model: &str) -> usize {
    model
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|token| matches!(*token, "proctype" | "init"))
        .count()
}

/// Rewrites SPIN's never claims so a violation is an acceptance cycle.
///
/// SPIN prints each move to `accept_all` as
/// `:: atomic { (c) -> assert(!(c)) }` and ends the claim with
/// `accept_all: skip`, so `pan` stops at the end of a finite prefix. The
/// move becomes `:: (c) -> goto accept_all`, and `accept_all` loops on true.
fn accepting(claims: &str) -> Result<String, String> {
    const MOVE: &str = "\t:: atomic { ";
    const ASSERT: &str = " -> assert(!(";
    let mut out = String::with_capacity(claims.len());
    let mut after_accept_all = false;
    for line in claims.lines() {
        if let Some(rest) = line.strip_prefix(MOVE) {
            let (condition, _) = rest
                .split_once(ASSERT)
                .ok_or_else(|| format!("unrecognised claim line {line:?}"))?;
            let _ = writeln!(out, "\t:: {condition} -> goto accept_all");
        } else if after_accept_all && line == "\tskip" {
            out.push_str("\tdo\n\t:: (1)\n\tod\n");
        } else if line.contains("assert(") {
            return Err(format!("unrecognised claim line {line:?}"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
        after_accept_all = line == "accept_all:";
    }
    Ok(out)
}

/// Reads whether `pan` found a violation. A truncated or out-of-memory
/// search without one is an error, never a proof.
fn search_result(stdout: &str) -> Result<bool, String> {
    let errors = stdout.lines().find_map(|line| {
        let (_, count) = line.split_once("errors: ")?;
        count.trim().parse::<u64>().ok()
    });
    if errors.is_some_and(|n| n > 0) {
        return Ok(true);
    }
    if stdout.contains("max search depth too small") {
        return Err(format!("search depth exhausted at {DEPTH}"));
    }
    if stdout.contains("reached -DMEMLIM bound") || stdout.contains("out of memory") {
        return Err("pan ran out of memory".into());
    }
    match errors {
        Some(_) => Ok(false),
        None => Err(format!("no result from pan: {}", stdout.trim())),
    }
}

/// One step of a replayed trail: does it receive into an atom variable, and
/// the globals printed after it.
#[derive(Default)]
struct Step {
    observable: bool,
    globals: BTreeMap<String, String>,
}

/// Parses a `spin -t -p -g -w` replay into a lasso over the observable
/// positions; see the module documentation.
fn lasso(stdout: &str, atoms: &[Atom]) -> Result<Lasso, String> {
    let mut steps: Vec<Step> = Vec::new();
    let mut cycle_step = None;
    for line in stdout.lines() {
        if line.starts_with("spin: trail ends") {
            break;
        } else if line.contains("<<<<<START OF CYCLE>>>>>") {
            cycle_step = Some(steps.len());
        } else if let Some((name, value)) = global(line) {
            if let Some(step) = steps.last_mut() {
                step.globals.insert(name.to_owned(), value.to_owned());
            }
        } else if let Some(statement) = statement(line) {
            let observable = statement.split_once('?').is_some_and(|(_, target)| {
                let target = target.trim_start_matches('?');
                atoms.iter().any(|a| a.var == target)
            });
            steps.push(Step {
                observable,
                globals: BTreeMap::new(),
            });
        }
    }
    let cycle_step = cycle_step.ok_or("the trail has no cycle")?;

    // The atom variables' values at each position, from the step that
    // opened it.
    let mut positions: Vec<Vec<&str>> = Vec::new();
    let mut cycle_start = None;
    for (i, step) in steps.iter().enumerate() {
        if i == cycle_step {
            cycle_start = Some(positions.len());
        }
        let values = atoms
            .iter()
            .map(|atom| {
                step.globals
                    .get(&atom.var)
                    .map(String::as_str)
                    .ok_or_else(|| {
                        format!(
                            "variable {} of atom {} is not a printed global",
                            atom.var, atom.name
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let last = positions.last();
        // A receive needs an earlier send, so the first step never writes an
        // atom variable and its globals are the initial ones.
        if last.is_none() && step.observable {
            return Err("the trail's first step writes an atom variable".into());
        }
        if last.is_none() || step.observable {
            positions.push(values);
        } else if last != Some(&values) {
            return Err(format!(
                "an atom variable changed outside a receive at step {i}"
            ));
        }
    }
    let start = cycle_start.unwrap_or(positions.len()).max(1);
    let last = positions.last().ok_or("the trail has no steps")?;
    if positions[start - 1] != *last {
        return Err("the trail's cycle does not return to its starting values".into());
    }
    let mut valuations: Vec<Valuation> = positions
        .iter()
        .map(|values| {
            atoms
                .iter()
                .zip(values)
                .filter(|(atom, value)| atom.value == **value)
                .map(|(atom, _)| atom.name.clone())
                .collect()
        })
        .collect();
    // A cycle without a receive stutters on the position it started in.
    let split = if start == valuations.len() {
        start - 1
    } else {
        start
    };
    let cycle = valuations.split_off(split);
    Lasso::new(valuations, cycle).ok_or_else(|| "the trail's cycle is empty".into())
}

/// A printed global, `\t\tname = value`.
fn global(line: &str) -> Option<(&str, &str)> {
    let (name, value) = line.strip_prefix("\t\t")?.split_once(" = ")?;
    Some((name.trim(), value.trim()))
}

/// The statement of a process step, `  12:\tproc  1 (Listener:1) m.pml:38
/// (state 1)\t[act?lastAction]`.
fn statement(line: &str) -> Option<&str> {
    let (number, rest) = line.trim_start().split_once(':')?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = rest.trim_start().strip_prefix("proc ")?;
    let open = rest.find("\t[")?;
    let text = &rest[open + 2..];
    Some(text.strip_suffix(']').unwrap_or(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = "mtype = { a, b, none };
chan act = [0] of { mtype };
mtype last = none;
#define p_a\t (last == a)
#define p_b (last==b)
#define p_both (last == a && last == b)
#define p_less (last < 3)
active proctype Listener() { do :: act ? last od }
active proctype Sender() { do :: act ! a :: act ! b od }
ltl one { [] p_a }
ltl two {
  <> p_b }
";

    fn atom(name: &str) -> Result<Atom, String> {
        interface_atom(MODEL, name)
    }

    fn set(names: &[&str]) -> Valuation {
        names.iter().map(|&n| Arc::from(n)).collect()
    }

    fn atoms(names: &[&str]) -> Vec<Atom> {
        names.iter().map(|n| atom(n).unwrap()).collect()
    }

    #[test]
    fn atoms_are_variable_equals_constant_macros() {
        let p_a = atom("p_a").unwrap();
        assert_eq!((p_a.var.as_str(), p_a.value.as_str()), ("last", "a"));
        assert_eq!(atom("p_b").unwrap().value, "b");
        for bad in ["p_both", "p_less"] {
            let error = atom(bad).unwrap_err();
            assert!(error.contains("only `(variable == constant)`"), "{error}");
        }
        assert!(atom("p_c").unwrap_err().contains("no #define"));
        assert!(atom("p").unwrap_err().contains("no #define"));
    }

    #[test]
    fn ltl_blocks_are_stripped_and_proctypes_counted() {
        let stripped = strip_ltl(MODEL);
        assert!(!stripped.contains("ltl"), "{stripped}");
        assert!(stripped.ends_with("od }\n"), "{stripped}");
        assert_eq!(proctypes(&stripped), 2);
    }

    #[test]
    fn has_next_finds_x_anywhere() {
        let f = |s: &str| s.parse::<Formula>().unwrap();
        assert!(!has_next(&f("G (a -> F b) U c")));
        assert!(has_next(&f("G (a -> F X b)")));
    }

    #[test]
    fn safety_asserts_become_accepting_moves() {
        let claim = "never c0 {    /* !([] (p)) */
T0_init:
\tdo
\t:: atomic { (! ((p))) -> assert(!(! ((p)))) }
\t:: (1) -> goto T0_init
\tod;
accept_all:
\tskip
}
";
        let rewritten = accepting(claim).unwrap();
        assert!(rewritten.contains("\t:: (! ((p))) -> goto accept_all\n"));
        assert!(rewritten.contains("accept_all:\n\tdo\n\t:: (1)\n\tod\n}"));
        assert!(!rewritten.contains("assert"));
        assert!(accepting("\t:: assert(p)\n").is_err());
    }

    #[test]
    fn pan_results() {
        assert_eq!(
            search_result("State-vector 64 byte, depth reached 9, errors: 0"),
            Ok(false)
        );
        assert_eq!(
            search_result("pan:1: acceptance cycle\n... errors: 1\n"),
            Ok(true)
        );
        let deep = "error: max search depth too small\n errors: 0\n";
        assert!(search_result(deep).unwrap_err().contains("depth"));
        let full = "pan: reached -DMEMLIM bound\n";
        assert!(search_result(full).unwrap_err().contains("memory"));
        assert!(search_result("").is_err());
    }

    /// A replay in SPIN's format: the sender's and listener's steps of a
    /// rendezvous, a guard between actions, and a cycle that starts after
    /// the first action.
    const TRAIL: &str = "starting claim 3
Never claim moves to line 5\t[(1)]
  2:\tproc  1 (Sender:1) m.pml:9 (state 1)\t[act!a]
\t\tlast = none
  3:\tproc  0 (Listener:1) m.pml:8 (state 1)\t[act?last]
\t\tlast = a
      <<<<<START OF CYCLE>>>>>
  5:\tproc  1 (Sender:1) m.pml:9 (state 2)\t[act!b]
\t\tlast = a
  6:\tproc  0 (Listener:1) m.pml:8 (state 1)\t[act?last]
\t\tlast = b
  8:\tproc  1 (Sender:1) m.pml:9 (state 3)\t[((x==1))]
\t\tlast = b
  9:\tproc  1 (Sender:1) m.pml:9 (state 1)\t[act!a]
\t\tlast = b
 10:\tproc  0 (Listener:1) m.pml:8 (state 1)\t[act?last]
\t\tlast = a
spin: trail ends after 10 steps
#processes: 2
\t\tlast = a
";

    #[test]
    fn a_trail_becomes_a_lasso_of_receives() {
        let lasso = lasso(TRAIL, &atoms(&["p_a", "p_b"])).unwrap();
        assert_eq!(lasso.prefix(), [set(&[]), set(&["p_a"])]);
        assert_eq!(lasso.cycle(), [set(&["p_b"]), set(&["p_a"])]);
    }

    #[test]
    fn a_cycle_without_receives_stutters() {
        let trail = TRAIL.replace("      <<<<<START OF CYCLE>>>>>\n", "");
        let trail = trail.replace(
            "spin: trail ends",
            "      <<<<<START OF CYCLE>>>>>\nspin: trail ends",
        );
        let lasso = lasso(&trail, &atoms(&["p_a"])).unwrap();
        assert_eq!(lasso.prefix(), [set(&[]), set(&["p_a"]), set(&[])]);
        assert_eq!(lasso.cycle(), [set(&["p_a"])]);
    }

    #[test]
    fn an_unsound_trail_is_an_error() {
        let p_a = atoms(&["p_a"]);
        let no_cycle = TRAIL.replace("      <<<<<START OF CYCLE>>>>>\n", "");
        assert!(lasso(&no_cycle, &p_a).unwrap_err().contains("no cycle"));
        let open = TRAIL.replace("last = a\nspin", "last = none\nspin");
        assert!(lasso(&open, &p_a).unwrap_err().contains("does not return"));
        let hidden = TRAIL.replace("[((x==1))]\n\t\tlast = b", "[((x==1))]\n\t\tlast = a");
        assert!(lasso(&hidden, &p_a)
            .unwrap_err()
            .contains("outside a receive"));
        let other = [Atom {
            name: Arc::from("q"),
            var: "other".into(),
            value: "1".into(),
        }];
        assert!(lasso(TRAIL, &other)
            .unwrap_err()
            .contains("not a printed global"));
    }
}
