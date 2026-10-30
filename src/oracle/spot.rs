//! Spot's half of oracle tier 2, and the output gate's φ ⇒ φ' check:
//! deciding ψ ⇒ φ for a batch of pairs in one `ltlfilt` exec.
//!
//! ψ ⇒ φ is valid iff `ψ & !φ` is unsatisfiable. So each pair becomes one
//! line `(ψ) & !(φ)` on `ltlfilt`'s stdin. Two checks read those lines.
//!
//! - [`implications`] runs `ltlfilt -F - -v --satisfiable --format=%L`,
//!   which prints the line number of each unsatisfiable line. It is complete,
//!   but it translates each line to an automaton, which on long formulas can
//!   take minutes.
//! - [`syntactic_implications`] runs `ltlfilt -F - -r2 --ap=0
//!   --format=%L,%f`, which only rewrites each line with Spot's basic and
//!   syntactic-implication rules and prints the lines that become a
//!   constant. A line rewritten to `0` is proved. It builds no automaton, so
//!   it answers in milliseconds where the complete check times out, and it
//!   proves less.
//!
//! [`simplifications`] runs `ltlfilt -F - -r`, Spot's simplifier at its
//! highest level, over one formula per line, for the gate to report a
//! shorter equivalent form of each repair.
//!
//! A line that does not parse is skipped and so never reads as proved.
//! Start-up dominates a single `ltlfilt` call (about 13 ms), so one exec per
//! batch is the point; see "Implication from known truths" in
//! `docs/dev/oracle-performance.md`.
//!
//! One hard pair can hold a whole batch past its time limit.
//! [`implications_each`] retries a failed batch one pair per exec, so the
//! hard pair reads as not proved and the rest are still decided; see "Slow
//! implication batches" in `docs/dev/trace-sets.md`. The oracle's tier 2
//! calls [`implications`] directly, since a failed batch there only falls
//! through to the back end.
//!
//! Under [`Semantics::Finite`], [`implications`] runs `ltlf2dfa -Hl -F -`
//! instead, which builds the minimal DFA of each line, one HOA automaton per
//! output line, and a pair is proved when its DFA marks no transition
//! accepting. The other `ltlfilt` execs add `--from-ltlf`, which rewrites
//! each line into LTL over a fresh atom ([`ALIVE`]) that holds on a finite
//! prefix and never after. That route also decides implication, but on long
//! LTLf formulas its ω-automata time out or need more than Spot's 32
//! acceptance sets, which a DFA never does; see "Undecided pairs" in
//! `docs/dev/trace-sets.md`. LTL implication is not LTLf implication:
//! `X a -> F a` is valid over infinite words but fails on the one-position
//! word where `a` is false, since `X` is weak there. `ltlf2dfa` reads `X` as
//! `--from-ltlf` does, weak, with `X[!]` the strong next.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::process::{self, Limits};
use super::Semantics;
use crate::ltl::Formula;

/// The atom `--from-ltlf` adds to mark the positions of a finite word. A
/// trace set must not use it.
pub const ALIVE: &str = "gawain_alive";

/// The `ltlfilt` execs that run at once when [`implications_each`] retries
/// a failed batch one pair per exec.
const RETRY_THREADS: usize = 4;

/// For each pair `(ψ, φ)`, whether Spot proved ψ ⇒ φ under `semantics`, in
/// one `ltlfilt` exec. `false` means only "not proved": the implication may
/// be refuted or the line may have been skipped.
///
/// An `Err` is "unknown" for the whole batch: a timeout, a kill, a parse
/// error or output that cannot be read. Callers must read it as "nothing
/// proved". An empty batch starts no process.
pub fn implications(
    pairs: &[(&Formula, &Formula)],
    semantics: Semantics,
    limits: Limits,
) -> Result<Vec<bool>, String> {
    if semantics == Semantics::Finite {
        return dfa_implications(pairs, limits);
    }
    let stdout = ltlfilt(
        pairs,
        &["-v", "--satisfiable", "--format=%L"],
        semantics,
        limits,
    )?;
    parse(&stdout, pairs.len(), str::is_empty)
}

/// [`implications`] under [`Semantics::Finite`]: one `ltlf2dfa` exec, with
/// pair i proved when the i-th automaton has no accepting transition. A line
/// that does not parse prints no automaton and makes `ltlf2dfa` exit 2, so
/// it fails the whole batch.
fn dfa_implications(pairs: &[(&Formula, &Formula)], limits: Limits) -> Result<Vec<bool>, String> {
    let lines = pair_lines(pairs);
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let mut command = Command::new("ltlf2dfa");
    command.args(["-Hl", "-F", "-"]);
    let output = process::run(&mut command, &stdin(&lines)?, limits)
        .map_err(|e| format!("ltlf2dfa: {e}"))?;
    if output.status.code() != Some(0) {
        return Err(format!("ltlf2dfa: {}", output.stderr.trim()));
    }
    parse_dfas(&output.stdout, pairs.len())
}

/// Reads one single-line HOA automaton per pair, and marks a pair proved
/// when its body has no acceptance mark `{…}`. The header holds the
/// formula's name, which may contain braces, so only the body is read.
fn parse_dfas(stdout: &str, n_pairs: usize) -> Result<Vec<bool>, String> {
    let proved: Vec<bool> = (stdout.lines())
        .map(|line| match line.split_once("--BODY--") {
            Some((_, body)) => Ok(!body.contains('{')),
            None => Err(format!("ltlf2dfa: unexpected output {line:?}")),
        })
        .collect::<Result<_, _>>()?;
    if proved.len() != n_pairs {
        return Err(format!(
            "ltlf2dfa: {} automata for {n_pairs} pairs",
            proved.len()
        ));
    }
    Ok(proved)
}

/// [`implications`], but a batch that fails is retried one pair per exec,
/// four at a time, each under the same `limits`, and a pair
/// whose own exec fails reads as not proved. One hard pair then costs one
/// more limit and leaves the rest decided. The error of the failed batch,
/// if any, comes back with the answers.
pub fn implications_each(
    pairs: &[(&Formula, &Formula)],
    semantics: Semantics,
    limits: Limits,
) -> (Vec<bool>, Option<String>) {
    let error = match implications(pairs, semantics, limits) {
        Ok(proved) => return (proved, None),
        Err(error) => error,
    };
    let next = AtomicUsize::new(0);
    let mut proved = vec![false; pairs.len()];
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..RETRY_THREADS.min(pairs.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut found = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&pair) = pairs.get(i) else {
                            return found;
                        };
                        if implications(&[pair], semantics, limits).is_ok_and(|p| p[0]) {
                            found.push(i);
                        }
                    }
                })
            })
            .collect();
        for worker in workers {
            for i in worker.join().expect("a retry thread") {
                proved[i] = true;
            }
        }
    });
    (proved, Some(error))
}

/// For each pair `(ψ, φ)`, whether Spot's rewriting rules alone reduce
/// `ψ & !φ` to false, which proves ψ ⇒ φ, in one `ltlfilt` exec. It proves
/// a subset of what [`implications`] proves, and an `Err` means "nothing
/// proved" as there.
pub fn syntactic_implications(
    pairs: &[(&Formula, &Formula)],
    semantics: Semantics,
    limits: Limits,
) -> Result<Vec<bool>, String> {
    let stdout = ltlfilt(
        pairs,
        &["-r2", "--ap=0", "--format=%L,%f"],
        semantics,
        limits,
    )?;
    // Each printed line is `N,0` or `N,1`: line N became a constant.
    parse(&stdout, pairs.len(), |rest| rest == ",0")
}

/// Each formula rewritten by Spot's simplifier at its highest level
/// (`ltlfilt -r`), in Spot's syntax, in one exec. The rewrite keeps each
/// formula equivalent over infinite words, not finite ones. An `Err` covers
/// the whole batch, as in [`implications`].
pub fn simplifications(formulas: &[&Formula], limits: Limits) -> Result<Vec<String>, String> {
    let lines: Vec<String> = formulas.iter().map(|f| f.to_string()).collect();
    let stdout = exec(&lines, &["-r"], Semantics::Infinite, limits)?;
    let simplified: Vec<String> = stdout.lines().map(str::to_owned).collect();
    if simplified.len() != formulas.len() {
        return Err(format!(
            "ltlfilt: {} lines for {} formulas",
            simplified.len(),
            formulas.len()
        ));
    }
    Ok(simplified)
}

/// Runs `ltlfilt -F - args` with one line `(ψ) & !(φ)` per pair on stdin,
/// under `semantics`, and returns its stdout. An empty batch starts no
/// process.
fn ltlfilt(
    pairs: &[(&Formula, &Formula)],
    args: &[&str],
    semantics: Semantics,
    limits: Limits,
) -> Result<String, String> {
    exec(&pair_lines(pairs), args, semantics, limits)
}

/// One line `(ψ) & !(φ)` per pair.
fn pair_lines(pairs: &[(&Formula, &Formula)]) -> Vec<String> {
    (pairs.iter())
        .map(|(psi, phi)| format!("({psi}) & !({phi})"))
        .collect()
}

/// The lines as stdin text, one per line.
fn stdin(lines: &[String]) -> Result<String, String> {
    let mut input = String::new();
    for line in lines {
        // A newline inside a quoted atom would split the line and shift
        // every later line number.
        if line.contains('\n') {
            return Err(format!("cannot put {line:?} on one line"));
        }
        input.push_str(line);
        input.push('\n');
    }
    Ok(input)
}

/// Runs `ltlfilt -F - args` with `lines` on stdin, under `semantics`, and
/// returns its stdout. An empty batch starts no process.
fn exec(
    lines: &[String],
    args: &[&str],
    semantics: Semantics,
    limits: Limits,
) -> Result<String, String> {
    if lines.is_empty() {
        return Ok(String::new());
    }
    let input = stdin(lines)?;
    let mut command = Command::new("ltlfilt");
    if semantics == Semantics::Finite {
        command.arg(format!("--from-ltlf={ALIVE}"));
    }
    command.args(["-F", "-"]).args(args);
    let output = process::run(&mut command, &input, limits).map_err(|e| format!("ltlfilt: {e}"))?;
    // Exit 0: some line was printed; 1: none was; 2: an error was reported.
    match output.status.code() {
        Some(0 | 1) => Ok(output.stdout),
        _ => Err(format!("ltlfilt: {}", output.stderr.trim())),
    }
}

/// Reads the 1-based line number that starts each output line, and marks
/// that pair proved when `proves` accepts the rest of the line.
fn parse(stdout: &str, n_pairs: usize, proves: impl Fn(&str) -> bool) -> Result<Vec<bool>, String> {
    let mut proved = vec![false; n_pairs];
    for line in stdout.lines() {
        let line = line.trim();
        let digits = line
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(line.len());
        let (number, rest) = line.split_at(digits);
        let number: usize = number
            .parse()
            .map_err(|_| format!("ltlfilt: unexpected output {line:?}"))?;
        let slot = number
            .checked_sub(1)
            .and_then(|i| proved.get_mut(i))
            .ok_or_else(|| format!("ltlfilt: line {number} is out of range"))?;
        *slot = proves(rest);
    }
    Ok(proved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_mark_proved_pairs() {
        assert_eq!(
            parse("1\n3\n", 4, str::is_empty),
            Ok(vec![true, false, true, false])
        );
        assert_eq!(parse("", 2, str::is_empty), Ok(vec![false, false]));
    }

    #[test]
    fn only_a_line_rewritten_to_false_is_proved_syntactically() {
        let falsity = |rest: &str| rest == ",0";
        assert_eq!(
            parse("1,0\n2,1\n", 3, falsity),
            Ok(vec![true, false, false])
        );
    }

    #[test]
    fn unreadable_output_is_an_error() {
        assert!(parse("0\n", 2, str::is_empty).is_err());
        assert!(parse("3\n", 2, str::is_empty).is_err());
        assert!(parse("G a\n", 2, str::is_empty).is_err());
        assert!(parse(",0\n", 2, |rest| rest == ",0").is_err());
    }

    #[test]
    fn a_dfa_without_acceptance_marks_proves_its_pair() {
        let empty = "HOA: v1 name: \"a & !a\" States: 1 --BODY-- State: 0 --END--";
        let some = "HOA: v1 name: \"Fa\" --BODY-- State: 0 [!0] 0 [0] 1 {0} --END--";
        assert_eq!(
            parse_dfas(&format!("{empty}\n{some}\n"), 2),
            Ok(vec![true, false])
        );
        // Braces in the name are not acceptance marks.
        let named = "HOA: v1 name: \"{a}\" --BODY-- State: 0 --END--";
        assert_eq!(parse_dfas(named, 1), Ok(vec![true]));
    }

    #[test]
    fn missing_or_unreadable_automata_are_an_error() {
        assert!(parse_dfas("", 1).is_err());
        assert!(parse_dfas("HOA: v1 --BODY-- --END--\n", 2).is_err());
        assert!(parse_dfas("G a\n", 1).is_err());
    }

    #[test]
    fn an_empty_batch_needs_no_tool() {
        let limits = Limits {
            timeout: std::time::Duration::from_secs(1),
            memory_bytes: 1 << 20,
        };
        let none = |semantics| {
            assert_eq!(implications(&[], semantics, limits), Ok(Vec::new()));
            assert_eq!(
                syntactic_implications(&[], semantics, limits),
                Ok(Vec::new())
            );
        };
        none(Semantics::Infinite);
        none(Semantics::Finite);
        assert_eq!(simplifications(&[], limits), Ok(Vec::new()));
    }
}
