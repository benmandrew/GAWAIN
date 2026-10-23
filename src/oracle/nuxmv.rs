//! The nuXmv back end: one batch nuXmv process per query on an SMV model.
//!
//! Each query starts `nuXmv -int <model>` and sends a command script on
//! stdin that checks the candidate with `check_ltlspec -p`. A persistent
//! session would save little, since on large models the cost is the
//! per-formula tableau, not the model build (see "Back end 2: nuXmv" in
//! `docs/dev/oracle-performance.md`).
//!
//! The script selects trace plugin 1, which prints every variable and
//! DEFINE at every state rather than only the changed ones. A
//! counterexample therefore gives the value of each interface atom at each
//! position, and becomes a [`Lasso`] over those atoms.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use super::process::{self, Limits};
use super::{Backend, Lasso, Valuation, Verdict};
use crate::ltl::Formula;

/// A [`Backend`] that runs nuXmv on a fixed SMV model.
pub struct Nuxmv {
    model: PathBuf,
    atoms: Vec<Arc<str>>,
    limits: Limits,
}

impl Nuxmv {
    /// A back end for the model at `model`. `atoms` are the model's
    /// interface: boolean variables or DEFINEs whose truth values each
    /// returned lasso records at every position.
    pub fn new(
        model: impl Into<PathBuf>,
        atoms: impl IntoIterator<Item = impl AsRef<str>>,
        limits: Limits,
    ) -> Self {
        Nuxmv {
            model: model.into(),
            atoms: atoms.into_iter().map(|a| Arc::from(a.as_ref())).collect(),
            limits,
        }
    }
}

impl Backend for Nuxmv {
    fn check(&mut self, formula: &Formula) -> Verdict {
        let spec = formula.to_nuxmv();
        if spec.contains(['"', '\n']) {
            return Verdict::Unknown(format!("cannot quote {spec:?} for nuXmv"));
        }
        let script = format!("set default_trace_plugin 1\ngo\ncheck_ltlspec -p \"{spec}\"\nquit\n");
        let mut command = Command::new("nuXmv");
        command.arg("-int").arg(&self.model);
        match process::run(&mut command, &script, self.limits) {
            Ok(output) => parse(&output.stdout, &self.atoms).unwrap_or_else(|reason| {
                Verdict::Unknown(format!("nuXmv: {reason}\n{}", output.stderr.trim()))
            }),
            Err(error) => Verdict::Unknown(format!("nuXmv: {error}")),
        }
    }
}

/// Reads the verdict, and on a failure the counterexample, from nuXmv's
/// standard output.
fn parse(stdout: &str, atoms: &[Arc<str>]) -> Result<Verdict, String> {
    let mut lines = stdout.lines();
    // The result follows the interactive prompt on the same line.
    let result = lines
        .by_ref()
        .find_map(|line| line.find("-- specification ").map(|at| &line[at..]))
        .ok_or("no result for the specification")?;
    let result = result.trim_end();
    if result.ends_with(" is true") {
        Ok(Verdict::Holds)
    } else if result.ends_with(" is false") {
        lasso(lines, atoms).map(Verdict::Fails)
    } else {
        Err(format!("unrecognised result {result:?}"))
    }
}

type State = BTreeMap<String, String>;

/// Parses a counterexample trace into a lasso.
///
/// nuXmv marks with "-- Loop starts here" each state that equals the last
/// state of the trace, so the last state closes the loop and is dropped.
/// The last marker gives the shortest cycle.
fn lasso<'a>(lines: impl Iterator<Item = &'a str>, atoms: &[Arc<str>]) -> Result<Lasso, String> {
    let mut states: Vec<State> = Vec::new();
    let mut loop_start = None;
    let mut marked = false;
    // Input sections (IVAR values) are skipped; they are not interface atoms.
    let mut in_state = false;
    for line in lines {
        let line = line.trim();
        if line.starts_with("nuXmv >") {
            break;
        } else if line == "-- Loop starts here" {
            marked = true;
        } else if line.starts_with("-> State:") {
            if marked {
                loop_start = Some(states.len());
                marked = false;
            }
            states.push(State::new());
            in_state = true;
        } else if line.starts_with("-> Input:") {
            in_state = false;
        } else if let Some((name, value)) = line.split_once(" = ") {
            if in_state {
                let state = states.last_mut().expect("in a state");
                state.insert(name.to_owned(), value.to_owned());
            }
        }
    }
    let start = loop_start.ok_or("the counterexample has no loop")?;
    let last = states.pop().ok_or("the counterexample has no states")?;
    if states.get(start) != Some(&last) {
        return Err("the counterexample's last state does not close its loop".into());
    }
    let valuations = states
        .iter()
        .map(|state| valuation(state, atoms))
        .collect::<Result<Vec<_>, _>>()?;
    let mut prefix = valuations;
    let cycle = prefix.split_off(start);
    Lasso::new(prefix, cycle).ok_or_else(|| "the counterexample's loop is empty".into())
}

fn valuation(state: &State, atoms: &[Arc<str>]) -> Result<Valuation, String> {
    let mut holds = Valuation::new();
    for atom in atoms {
        match state.get(&**atom).map(String::as_str) {
            Some("TRUE") => {
                holds.insert(atom.clone());
            }
            Some("FALSE") => {}
            Some(value) => return Err(format!("atom {atom} has non-boolean value {value}")),
            None => return Err(format!("atom {atom} is not in the counterexample")),
        }
    }
    Ok(holds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(names: &[&str]) -> Vec<Arc<str>> {
        names.iter().map(|&n| Arc::from(n)).collect()
    }

    fn set(names: &[&str]) -> Valuation {
        atoms(names).into_iter().collect()
    }

    const TRACE: &str = "nuXmv > nuXmv > -- specification  G !x  is false
-- as demonstrated by the following execution sequence
Trace Description: LTL Counterexample
Trace Type: Counterexample
  -> State: 1.1 <-
    x = FALSE
    n = 0
    z = FALSE
  -- Loop starts here
  -> State: 1.2 <-
    x = TRUE
    n = 1
    z = TRUE
  -> Input: 1.3 <-
    i = TRUE
  -- Loop starts here
  -> State: 1.3 <-
    x = TRUE
    n = 2
    z = FALSE
  -> State: 1.4 <-
    x = TRUE
    n = 1
    z = TRUE
  -> State: 1.5 <-
    x = TRUE
    n = 2
    z = FALSE
nuXmv >
";

    #[test]
    fn a_true_result_holds() {
        let out = "nuXmv > nuXmv > -- specification  F x  is true\nnuXmv > ";
        assert_eq!(parse(out, &atoms(&["x"])), Ok(Verdict::Holds));
    }

    #[test]
    fn a_trace_becomes_a_lasso_at_the_last_loop_marker() {
        let Ok(Verdict::Fails(lasso)) = parse(TRACE, &atoms(&["x", "z"])) else {
            panic!("expected a failure");
        };
        assert_eq!(lasso.prefix(), [set(&[]), set(&["x", "z"])]);
        assert_eq!(lasso.cycle(), [set(&["x"]), set(&["x", "z"])]);
    }

    #[test]
    fn an_unusable_trace_is_an_error() {
        let missing = parse(TRACE, &atoms(&["y"])).unwrap_err();
        assert!(missing.contains("y is not in"), "{missing}");
        let integer = parse(TRACE, &atoms(&["n"])).unwrap_err();
        assert!(integer.contains("non-boolean"), "{integer}");
        let open = TRACE.replace(
            "    n = 2\n    z = FALSE\nnuXmv",
            "    n = 3\n    z = FALSE\nnuXmv",
        );
        assert!(parse(&open, &atoms(&["x"])).is_err());
        assert!(parse("nuXmv > ERROR\n", &atoms(&["x"])).is_err());
    }
}
