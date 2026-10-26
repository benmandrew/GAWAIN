//! End-to-end test of the `score` binary, which counts with `ltl2tgba`.
//! It skips, with a reason on stderr, when `ltl2tgba` is not on `PATH`.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::have;

fn score(input: &str) -> Vec<(f64, f64, f64)> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_score"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("score starts");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "score failed: {out:?}");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let columns: Vec<f64> = line.split('\t').map(|c| c.parse().unwrap()).collect();
            assert_eq!(columns.len(), 3, "{line}");
            (columns[0], columns[1], columns[2])
        })
        .collect()
}

#[test]
fn score_ranks_phi_above_a_weakening_above_true() {
    if !have(
        &["ltl2tgba"],
        "score_ranks_phi_above_a_weakening_above_true",
    ) {
        return;
    }
    let found = score(
        "G (a -> X b)\tG (a -> X b)\nG (a -> X b)\tG (a -> F b)\nG (a -> X b)\ttrue\nF a\ttrue\n",
    );
    assert_eq!(found.len(), 4);
    assert_eq!(found[0], (1.0, 1.0, 1.0));
    let (weak, top) = (found[1], found[2]);
    assert!(1.0 > weak.0 && weak.0 > top.0, "{found:?}");
    assert!(weak.1 > 0.0 && top.1 == 0.0, "{found:?}");
    assert!(1.0 > weak.2 && weak.2 > 0.0 && top.2 == 0.0, "{found:?}");
    // F a misses only the lassos without a, so it scores near ⊤, but ⊤
    // rejects none of them.
    assert!(found[3].0 > top.0 && found[3].2 == 0.0, "{found:?}");
}

#[test]
fn score_rejects_a_line_without_a_tab() {
    let out = Command::new(env!("CARGO_BIN_EXE_score"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child.stdin.take().unwrap().write_all(b"G a\n")?;
            child.wait_with_output()
        })
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
