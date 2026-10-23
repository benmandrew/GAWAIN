//! Spot's half of oracle tier 2, and the output gate's φ ⇒ φ' check:
//! deciding ψ ⇒ φ for a batch of pairs in one `ltlfilt` exec.
//!
//! ψ ⇒ φ is valid iff `ψ & !φ` is unsatisfiable. So each pair becomes one
//! line `(ψ) & !(φ)` on `ltlfilt`'s stdin, and
//! `ltlfilt -F - -v --satisfiable --format=%L` prints the line number of each
//! unsatisfiable line, that is, of each proved implication. A line that does
//! not parse is skipped and so never reads as proved. Start-up dominates a
//! single `ltlfilt` call (about 13 ms), so one exec per batch is the point;
//! see "Implication from known truths" in `docs/dev/oracle-performance.md`.

use std::process::Command;

use super::process::{self, Limits};
use crate::ltl::Formula;

/// For each pair `(ψ, φ)`, whether Spot proved ψ ⇒ φ, in one `ltlfilt`
/// exec. `false` means only "not proved": the implication may be refuted or
/// the line may have been skipped.
///
/// An `Err` is "unknown" for the whole batch: a timeout, a kill, a parse
/// error or output that cannot be read. Callers must read it as "nothing
/// proved". An empty batch starts no process.
pub fn implications(pairs: &[(&Formula, &Formula)], limits: Limits) -> Result<Vec<bool>, String> {
    if pairs.is_empty() {
        return Ok(Vec::new());
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
    command.args(["-F", "-", "-v", "--satisfiable", "--format=%L"]);
    let output = process::run(&mut command, &input, limits).map_err(|e| format!("ltlfilt: {e}"))?;
    // Exit 0: some line was printed; 1: none was; 2: an error was reported.
    match output.status.code() {
        Some(0 | 1) => {}
        _ => return Err(format!("ltlfilt: {}", output.stderr.trim())),
    }
    parse(&output.stdout, pairs.len())
}

/// Reads the 1-based line numbers of the proved pairs.
fn parse(stdout: &str, n_pairs: usize) -> Result<Vec<bool>, String> {
    let mut proved = vec![false; n_pairs];
    for line in stdout.lines() {
        let number: usize = line
            .trim()
            .parse()
            .map_err(|_| format!("ltlfilt: unexpected output {line:?}"))?;
        let slot = number
            .checked_sub(1)
            .and_then(|i| proved.get_mut(i))
            .ok_or_else(|| format!("ltlfilt: line {number} is out of range"))?;
        *slot = true;
    }
    Ok(proved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbers_mark_proved_pairs() {
        assert_eq!(parse("1\n3\n", 4), Ok(vec![true, false, true, false]));
        assert_eq!(parse("", 2), Ok(vec![false, false]));
    }

    #[test]
    fn unreadable_output_is_an_error() {
        assert!(parse("0\n", 2).is_err());
        assert!(parse("3\n", 2).is_err());
        assert!(parse("G a\n", 2).is_err());
    }

    #[test]
    fn an_empty_batch_needs_no_tool() {
        let limits = Limits {
            timeout: std::time::Duration::from_secs(1),
            memory_bytes: 1 << 20,
        };
        assert_eq!(implications(&[], limits), Ok(Vec::new()));
    }
}
