//! End-to-end test: the fingerprint prefilter never rules out an implication
//! that Spot proves, and reports how many of Spot's refutations it finds.
//!
//! Each pair is a random formula ψ and a mutant φ' of it, made by replacing
//! one random subformula with a random formula, as a GA offspring would be.
//! Spot decides every ψ ⇒ φ' in one batch through [`implications`], since
//! `ltlfilt --imply` in Spot 2.15.1 wrongly rejects `false ⇒ true`. The test
//! skips, with a reason on stderr, when `ltlfilt` is not on `PATH`.

mod common;

use std::io::Write;
use std::time::Duration;

use aragon::oracle::process::Limits;
use aragon::oracle::{implications, Words};
use common::{formula, mutant, on_path, Rng};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 2 << 30,
};
const PAIRS: usize = 400;
const ATOMS: &[&str] = &["a", "b", "c"];

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
    let pairs: Vec<_> = (0..PAIRS)
        .map(|_| {
            let psi = formula(&mut rng, 4, ATOMS);
            let phi = mutant(&mut rng, &psi, 2, ATOMS);
            (psi, phi)
        })
        .collect();
    let refs: Vec<_> = pairs.iter().map(|(psi, phi)| (psi, phi)).collect();
    let proved = implications(&refs, LIMITS).expect("ltlfilt failed");
    let (mut implied, mut refuted, mut caught) = (0, 0, 0);
    for ((psi, phi), spot) in pairs.iter().zip(proved) {
        let may = words.fingerprint(psi).may_imply(&words.fingerprint(phi));
        if spot {
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
