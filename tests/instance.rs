//! End-to-end tests of the degeneracy filter with nuXmv: `classify` on a
//! small model, and the `weaken` binary's exit status, for a proper, a
//! degenerate and an already-true property.
//!
//! Each test skips, with a reason on stderr, when a tool is missing.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use common::{have, parse};
use gawain::instance::{classify, Instance};
use gawain::oracle::nuxmv::Nuxmv;
use gawain::oracle::process::Limits;
use gawain::oracle::{Oracle, Words};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};

/// `a` is free and `b` never holds.
const MODEL: &str = "MODULE main
VAR
  a : boolean;
  b : boolean;
ASSIGN
  init(b) := FALSE; next(b) := b;
";
const ATOMS: &str = "a,b";
/// Fails on the runs where `a` drops, holds on the run where it never does.
const PROPER: &str = "G a";
/// Fails on every run.
const DEGENERATE: &str = "F b";
/// Holds on every run.
const SATISFIED: &str = "G !b";

fn write_model(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, MODEL).expect("write model");
    path
}

#[test]
fn classify_sorts_the_three_outcomes_on_nuxmv() {
    if !have(&["nuXmv"], "classify_sorts_the_three_outcomes_on_nuxmv") {
        return;
    }
    let model = write_model("instance-classify.smv");
    let cases = [
        (PROPER, Instance::Proper { unknown: vec![] }),
        (DEGENERATE, Instance::Degenerate),
        (SATISFIED, Instance::Satisfied),
    ];
    for (phi, expected) in cases {
        let backend = Nuxmv::new(&model, ATOMS.split(','), LIMITS);
        let mut oracle = Oracle::new(backend, Words::sample(ATOMS.split(','), 1), LIMITS);
        assert_eq!(classify(&parse(phi), &mut oracle), expected, "{phi}");
    }
}

#[test]
fn weaken_exits_by_instance_class() {
    if !have(&["nuXmv", "ltlfilt"], "weaken_exits_by_instance_class") {
        return;
    }
    let model = write_model("instance-weaken.smv");
    for (phi, code) in [(PROPER, 0), (SATISFIED, 1), (DEGENERATE, 3)] {
        let output = Command::new(env!("CARGO_BIN_EXE_weaken"))
            .arg("--model")
            .arg(&model)
            .args(["--atoms", ATOMS, "--ltl", phi])
            .args(["--seed", "1", "--generations", "1", "--population", "4"])
            .output()
            .expect("run weaken");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(code),
            "{phi}\n{stdout}\n{stderr}"
        );
        if code == 3 {
            assert!(stderr.contains("degenerate"), "{stderr}");
        }
        // Only a proper instance reaches the search.
        assert_eq!(stdout.contains("instance: proper"), code == 0, "{stdout}");
    }
}
