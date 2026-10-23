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
//! A line that does not parse is skipped and so never reads as proved.
//! Start-up dominates a single `ltlfilt` call (about 13 ms), so one exec per
//! batch is the point; see "Implication from known truths" in
//! `docs/dev/oracle-performance.md`.
//!
//! Under [`Semantics::Finite`] each exec adds `--from-ltlf`, which rewrites
//! each line into LTL over a fresh atom ([`ALIVE`]) that holds on a finite
//! prefix and never after, so satisfiability, and with it implication, is
//! decided over finite words. LTL implication is not LTLf implication:
//! `X a -> F a` is valid over infinite words but fails on the one-position
//! word where `a` is false, since `X` is weak there.

use std::process::Command;

use super::process::{self, Limits};
use super::Semantics;
use crate::ltl::Formula;

/// The atom `--from-ltlf` adds to mark the positions of a finite word. A
/// trace set must not use it.
pub const ALIVE: &str = "gawain_alive";

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
    let stdout = ltlfilt(
        pairs,
        &["-v", "--satisfiable", "--format=%L"],
        semantics,
        limits,
    )?;
    parse(&stdout, pairs.len(), str::is_empty)
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

/// Runs `ltlfilt -F - args` with one line `(ψ) & !(φ)` per pair on stdin,
/// under `semantics`, and returns its stdout. An empty batch starts no
/// process.
fn ltlfilt(
    pairs: &[(&Formula, &Formula)],
    args: &[&str],
    semantics: Semantics,
    limits: Limits,
) -> Result<String, String> {
    if pairs.is_empty() {
        return Ok(String::new());
    }
    let mut input = String::new();
    for (psi, phi) in pairs {
        let line = format!("({psi}) & !({phi})\n");
        // A newline inside a quoted atom would split the line and shift
        // every later line number.
        if line.matches('\n').count() != 1 {
            return Err(format!("cannot put {psi} => {phi} on one line"));
        }
        input.push_str(&line);
    }
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
    }
}
