//! End-to-end test: the fingerprint prefilter never rules out an implication
//! that Spot proves, and reports how many of Spot's refutations it finds.
//!
//! Each pair is a random formula ψ and a mutant φ' of it, made by replacing
//! one random subformula with a random formula, as a GA offspring would be.
//! Spot decides ψ ⇒ φ' with `ltlfilt -f ψ --imply=φ'`. The test skips, with
//! a reason on stderr, when `ltlfilt` is not on `PATH`. Every tool call runs
//! under `timeout`.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use aragon::ltl::Formula;
use aragon::oracle::Words;
use common::{formula, mutant, on_path, Rng};

const TIMEOUT_SECS: &str = "60";
const PAIRS: usize = 400;
const ATOMS: &[&str] = &["a", "b", "c"];

/// Spot's verdict on `psi -> phi`, valid or not.
fn spot_implies(psi: &Formula, phi: &Formula) -> bool {
    let output = Command::new("timeout")
        .arg(TIMEOUT_SECS)
        .arg("ltlfilt")
        .args(["-q", "-f", &psi.to_string(), &format!("--imply={phi}")])
        .stdin(Stdio::null())
        .output()
        .expect("cannot run ltlfilt");
    let code = output.status.code();
    assert_ne!(code, Some(124), "ltlfilt timed out on {psi} => {phi}");
    // ltlfilt -q exits 0 when the formula passes the filter, 1 otherwise.
    assert!(
        matches!(code, Some(0 | 1)),
        "ltlfilt failed on {psi} => {phi}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    code == Some(0)
}

#[test]
fn fingerprint_is_sound_against_spot() {
    if !on_path("ltlfilt") {
        let _ = writeln!(
            std::io::stderr(),
            "skipping fingerprint_is_sound_against_spot: ltlfilt not on PATH"
        );
        return;
    }
    let words = Words::sample(ATOMS.iter().copied(), 0);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (mut implied, mut refuted, mut caught) = (0, 0, 0);
    for _ in 0..PAIRS {
        let psi = formula(&mut rng, 4, ATOMS);
        let phi = mutant(&mut rng, &psi, 2, ATOMS);
        let may = words.fingerprint(&psi).may_imply(&words.fingerprint(&phi));
        if spot_implies(&psi, &phi) {
            assert!(
                may,
                "the fingerprint ruled out {psi} => {phi}, which Spot proves"
            );
            implied += 1;
        } else {
            refuted += 1;
            caught += usize::from(!may);
        }
    }
    let _ = writeln!(
        std::io::stderr(),
        "fingerprint: {implied} pairs implied; refuted {caught} of {refuted} \
         Spot-refuted pairs ({:.1}%)",
        100.0 * caught as f64 / refuted as f64
    );
    // Guards against a generator that only yields one verdict.
    assert!(
        implied > PAIRS / 20 && refuted > PAIRS / 20,
        "{implied} implied, {refuted} refuted"
    );
}
