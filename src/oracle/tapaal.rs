//! The TAPAAL back end: one `verifypn` process per query on an MCC P/T net.
//!
//! Each query writes the candidate as a one-property MCC XML file on
//! `verifypn`'s standard input and runs `verifypn -ltl -t -x 1 <model>
//! /dev/stdin`, then other configurations when that one's failure has no
//! usable trace (see `ATTEMPTS`). On a failure, `-t` prints the
//! counterexample to standard error as a list of transition firings with a `<loop/>` marker before
//! the cycle, or else as a finite run that ends either in a dead marking,
//! which the contest's semantics extend by stuttering forever, or in a bad
//! prefix, after which every continuation violates the formula.
//!
//! The trace names transitions only, so the back end replays it on its own
//! copy of the net to get the marking at each position, and evaluates
//! every interface atom there. A trace that does not replay, whose cycle
//! does not return to its start, or whose lasso does not violate the
//! candidate is unusable, and so is a failure with no trace at all. The
//! structural reductions stay at their default, since the replay checks
//! that `verifypn` mapped the trace back to the original net.
//!
//! `extend` closes a finite run into a lasso. `verifypn` marks some bad
//! prefixes `<deadlock/>` although their last marking is live, and some
//! with no marker, so the marker is ignored and the net decides.
//!
//! A `TRUE` answer cannot be checked this way. It stands only when a second
//! configuration without partial-order reduction repeats it (see
//! `CONFIRM`); a lasso from that run overrides it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use roxmltree::Document;

use super::process::{self, Limits};
use super::{Backend, Lasso, Valuation, Verdict};
use crate::ltl::Formula;
use crate::mcc::{query_xml, Atoms, Marking, Net};

/// A [`Backend`] that runs TAPAAL's `verifypn` on a fixed PNML model.
pub struct Tapaal {
    model: PathBuf,
    net: Net,
    atoms: Atoms,
    limits: Limits,
}

impl Tapaal {
    /// A back end for the PNML file at `model`, already parsed as `net`.
    /// `atoms` are the interface: every returned lasso records which of
    /// them hold at each position.
    pub fn new(model: impl Into<PathBuf>, net: Net, atoms: Atoms, limits: Limits) -> Self {
        Tapaal {
            model: model.into(),
            net,
            atoms,
            limits,
        }
    }
}

impl Tapaal {
    /// Runs `verifypn` with `args` on `query`. The outer error is a
    /// process failure, such as a timeout; the inner one is an unusable
    /// answer.
    fn run(
        &self,
        args: &[&str],
        query: &str,
        formula: &Formula,
    ) -> Result<Result<Verdict, String>, String> {
        let mut command = Command::new("verifypn");
        command
            .args(args)
            .args(["-t", "-x", "1"])
            .arg(&self.model)
            .arg("/dev/stdin");
        let output = process::run(&mut command, query, self.limits).map_err(|e| e.to_string())?;
        Ok(parse(
            &output.stdout,
            &output.stderr,
            formula,
            &self.net,
            &self.atoms,
        ))
    }

    /// Reruns a first-run `TRUE` under [`CONFIRM`]. A second `TRUE`
    /// stands, a replayed violating lasso overrides the first run, and
    /// anything else is unknown.
    fn confirm(&self, query: &str, formula: &Formula) -> Verdict {
        let reason = match self.run(CONFIRM, query, formula) {
            Ok(Ok(verdict)) => return verdict,
            Ok(Err(reason)) | Err(reason) => reason,
        };
        Verdict::Unknown(format!(
            "verifypn: {} answered TRUE; {}: {reason}",
            ATTEMPTS[0].join(" "),
            CONFIRM.join(" ")
        ))
    }
}

impl Backend for Tapaal {
    fn check(&mut self, formula: &Formula) -> Verdict {
        let query = match query_xml("q", formula, &self.net, &self.atoms) {
            Ok(query) => query,
            Err(reason) => return Verdict::Unknown(reason),
        };
        let mut reasons = Vec::new();
        for (attempt, args) in ATTEMPTS.iter().enumerate() {
            match self.run(args, &query, formula) {
                Ok(Ok(Verdict::Fails(lasso))) => return Verdict::Fails(lasso),
                Ok(Ok(Verdict::Holds)) if attempt == 0 => return self.confirm(&query, formula),
                Ok(Ok(_)) => reasons.push(format!("{}: contradicts the first run", args.join(" "))),
                // A run that timed out would only time out again.
                Err(error) if attempt == 0 => {
                    return Verdict::Unknown(format!("verifypn: {error}"))
                }
                Ok(Err(reason)) | Err(reason) => {
                    reasons.push(format!("{}: {reason}", args.join(" ")))
                }
            }
        }
        Verdict::Unknown(format!("verifypn: {}", reasons.join("; ")))
    }
}

/// The `verifypn` configurations tried in turn. The first one's verdict
/// stands unless it is a failure without a usable trace; then each later
/// one runs until one gives a failure whose lasso replays and violates the
/// formula. A later run's `TRUE` is never taken: on a 150-formula sample,
/// `-q 0` answered `TRUE` twice where a replayed lasso proves `FALSE`.
/// `ndfs` repairs most traces that the default Tarjan search gets wrong,
/// and `-q 0` (no query simplification) gives a trace where
/// simplification alone proved the failure. See `bench/notes/mcc.md`.
const ATTEMPTS: &[&[&str]] = &[
    &["-ltl"],
    &["-ltl", "ndfs"],
    &["-ltl", "-q", "0"],
    &["-ltl", "ndfs", "-q", "0"],
];

/// The configuration that must repeat a first-run `TRUE`. The default
/// partial-order reduction answered `TRUE` for
/// RobotManipulation-PT-00002-LTLFireability-02, which `-p` (no
/// partial-order reduction) refutes with a lasso. See `bench/notes/mcc.md`.
const CONFIRM: &[&str] = &["-ltl", "-p"];

/// The error for a failing verdict that `verifypn` printed no trace for.
const NO_TRACE: &str = "the failure has no trace";

/// Reads the verdict from standard output and, on a failure, the trace
/// from standard error. A counterexample that does not violate `formula`
/// when replayed is an error, so a misread trace never becomes a verdict.
fn parse(
    stdout: &str,
    stderr: &str,
    formula: &Formula,
    net: &Net,
    atoms: &Atoms,
) -> Result<Verdict, String> {
    let result = stdout
        .lines()
        .find_map(|line| line.strip_prefix("FORMULA q "))
        .ok_or_else(|| format!("no result\n{}", tail(stdout)))?;
    match result.split_whitespace().next() {
        Some("TRUE") => Ok(Verdict::Holds),
        Some("FALSE") => {
            let lasso = lasso(stderr, net, atoms)?;
            if lasso.satisfies(formula) {
                return Err("the counterexample satisfies the formula".into());
            }
            Ok(Verdict::Fails(lasso))
        }
        _ => Err(format!("unrecognised result {result:?}")),
    }
}

fn tail(text: &str) -> &str {
    let at = text.len().saturating_sub(500);
    let at = (at..text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0);
    &text[at..]
}

/// The most steps [`extend`] takes before giving up.
const EXTENSION_LIMIT: usize = 1_000_000;

/// Extends a finite run to a lasso and returns where its cycle starts.
///
/// A finite trace from `verifypn` is a bad prefix, such as the run to a
/// bad marking for `G p`, so every continuation violates the formula. This
/// walk fires the first enabled transition at each step until a marking
/// repeats or a dead marking stutters. The caller still checks that the
/// lasso violates the formula.
fn extend(net: &Net, markings: &mut Vec<Marking>) -> Result<usize, String> {
    // The cycle may close on any earlier position, of the walk or of the
    // trace: firing from an equal marking repeats the same steps.
    let mut seen: HashMap<Marking, usize> = markings
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, m)| (m, i))
        .collect();
    let mut marking = markings.last().expect("the initial marking").clone();
    for _ in 0..EXTENSION_LIMIT {
        let Some(t) = (0..net.transition_count()).find(|&t| net.enabled(&marking, t)) else {
            return Ok(markings.len() - 1);
        };
        net.fire(&mut marking, t);
        if let Some(&start) = seen.get(&marking) {
            return Ok(start);
        }
        seen.insert(marking.clone(), markings.len());
        markings.push(marking.clone());
    }
    Err(format!(
        "no cycle within {EXTENSION_LIMIT} steps after the trace"
    ))
}

/// Replays the trace in `stderr` from the initial marking into a lasso.
fn lasso(stderr: &str, net: &Net, atoms: &Atoms) -> Result<Lasso, String> {
    let start = stderr.find("<trace>").ok_or(NO_TRACE)?;
    let end = stderr[start..]
        .find("</trace>")
        .map(|at| start + at + "</trace>".len())
        .ok_or("the trace is not closed")?;
    let doc = Document::parse(&stderr[start..end]).map_err(|e| format!("trace: {e}"))?;
    let mut marking = net.initial().to_vec();
    let mut markings: Vec<Marking> = vec![marking.clone()];
    let mut loop_start = None;
    for step in doc.root_element().children().filter(|n| n.is_element()) {
        match step.tag_name().name() {
            "transition" => {
                let id = step.attribute("id").ok_or("a trace step has no id")?;
                let t = net
                    .transition(id)
                    .ok_or_else(|| format!("the trace fires unknown transition {id}"))?;
                if !net.fire(&mut marking, t) {
                    return Err(format!("the trace fires {id}, which is not enabled"));
                }
                markings.push(marking.clone());
            }
            "loop" => loop_start = Some(markings.len() - 1),
            "deadlock" => {}
            other => return Err(format!("unexpected trace element <{other}>")),
        }
    }
    let cycle_start = match loop_start {
        Some(start) => {
            let last = markings.pop().expect("the initial marking");
            if markings.len() <= start || markings[start] != last {
                return Err("the trace's cycle does not return to its start".into());
            }
            start
        }
        // A dead last marking stutters; any other is a bad prefix.
        None => extend(net, &mut markings)?,
    };
    let valuation = |m: &Marking| -> Valuation {
        atoms
            .iter()
            .filter(|(_, p)| p.holds(net, m))
            .map(|(name, _)| name.clone())
            .collect()
    };
    let mut prefix: Vec<Valuation> = markings.iter().map(valuation).collect();
    let cycle = prefix.split_off(cycle_start);
    Lasso::new(prefix, cycle).ok_or_else(|| "the trace's cycle is empty".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcc::{Count, Predicate};

    const NET: &str = r#"<pnml><net id="n" type="ptnet"><page id="pg">
  <place id="p"><initialMarking><text>2</text></initialMarking></place>
  <place id="q"/>
  <transition id="move"/>
  <transition id="back"/>
  <arc id="a1" source="p" target="move"/>
  <arc id="a2" source="move" target="q"/>
  <arc id="a3" source="q" target="back"><inscription><text>2</text></inscription></arc>
  <arc id="a4" source="back" target="p"><inscription><text>2</text></inscription></arc>
 </page></net></pnml>"#;

    fn fixture() -> (Net, Atoms) {
        let net = Net::parse(NET).unwrap();
        let atoms = vec![
            (
                "full".into(),
                Predicate::Le(Count::Constant(2), Count::Tokens(vec![1])),
            ),
            (
                "mv".into(),
                Predicate::Fireable(vec![net.transition("move").unwrap()]),
            ),
        ];
        (net, atoms)
    }

    fn set(names: &[&str]) -> Valuation {
        names.iter().map(|&n| n.into()).collect()
    }

    fn f(text: &str) -> Formula {
        text.parse().unwrap()
    }

    fn step(t: &str) -> String {
        format!("<transition id=\"{t}\">\n<token place=\"p\" age=\"0\"/>\n</transition>\n")
    }

    #[test]
    fn a_looping_trace_replays_into_a_lasso() {
        let (net, atoms) = fixture();
        let trace = format!(
            "noise\n<trace>\n{}<loop/>\n{}{}{}</trace>\nSpent 0.1",
            step("move"),
            step("move"),
            step("back"),
            step("move")
        );
        let out = "FORMULA q FALSE TECHNIQUES EXPLICIT\n";
        let Ok(Verdict::Fails(lasso)) = parse(out, &trace, &f("G mv"), &net, &atoms) else {
            panic!("expected a failure");
        };
        assert_eq!(lasso.prefix(), [set(&["mv"])]);
        assert_eq!(lasso.cycle(), [set(&["mv"]), set(&["full"]), set(&["mv"])]);
    }

    #[test]
    fn a_deadlocked_trace_stutters_on_its_last_marking() {
        let net = Net::parse(&NET.replace(r#"<arc id="a4" source="back" target="p"><inscription><text>2</text></inscription></arc>"#, "")).unwrap();
        let atoms = fixture().1;
        let trace = format!(
            "<trace>{}{}{}<deadlock/></trace>",
            step("move"),
            step("move"),
            step("back")
        );
        let Ok(Verdict::Fails(lasso)) =
            parse("FORMULA q FALSE\n", &trace, &f("G F mv"), &net, &atoms)
        else {
            panic!("expected a failure");
        };
        assert_eq!(lasso.prefix().len(), 3);
        assert_eq!(lasso.cycle(), [set(&[])]);
    }

    #[test]
    fn a_finite_trace_is_extended_to_a_cycle() {
        let (net, atoms) = fixture();
        let trace = format!("<trace>{}{}</trace>", step("move"), step("move"));
        let out = "FORMULA q FALSE\n";
        let Ok(Verdict::Fails(lasso)) = parse(out, &trace, &f("G !full"), &net, &atoms) else {
            panic!("expected a failure");
        };
        assert!(lasso.prefix().is_empty());
        assert_eq!(lasso.cycle(), [set(&["mv"]), set(&["mv"]), set(&["full"])]);
        let wrong = parse(out, &trace, &f("G F full"), &net, &atoms).unwrap_err();
        assert!(wrong.contains("satisfies"), "{wrong}");
    }

    #[test]
    fn a_trace_that_does_not_replay_is_an_error() {
        let (net, atoms) = fixture();
        let out = "FORMULA q FALSE\n";
        let g = f("G mv");
        let bad = format!("<trace>{}<loop/>{}</trace>", step("back"), step("move"));
        assert!(parse(out, &bad, &g, &net, &atoms)
            .unwrap_err()
            .contains("not enabled"));
        let open = format!("<trace><loop/>{}</trace>", step("move"));
        assert!(parse(out, &open, &g, &net, &atoms)
            .unwrap_err()
            .contains("return"));
        let live = format!("<trace>{}<deadlock/></trace>", step("move"));
        assert!(matches!(
            parse(out, &live, &g, &net, &atoms),
            Ok(Verdict::Fails(_))
        ));
        assert!(parse(out, "", &g, &net, &atoms)
            .unwrap_err()
            .contains("no trace"));
        assert_eq!(
            parse("FORMULA q TRUE\n", "", &g, &net, &atoms),
            Ok(Verdict::Holds)
        );
        assert!(parse("ERROR\n", "", &g, &net, &atoms).is_err());
    }
}
