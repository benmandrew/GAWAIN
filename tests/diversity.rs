//! The `diversity` binary end to end: formulas in, a JSON distance matrix
//! out. It calls no external tool.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str], input: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_diversity"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn prints_the_distance_matrix() {
    let out = run(&["--traces", "1024", "--atoms", "c"], "G a\n!G a\nG a\n");
    assert!(out.starts_with("{\"atoms\": [\"a\", \"c\"], \"traces\": 1024, \"satisfying\": ["));
    assert!(out.contains("\"distance\": [[0.0, 1.0, 0.0],\n[1.0, 0.0, 1.0],\n[0.0, 1.0, 0.0]]}"));
}

#[test]
fn finite_semantics_changes_the_sample() {
    let infinite = run(&["--traces", "256"], "F X false\ntrue\n");
    let finite = run(
        &["--traces", "256", "--semantics", "finite"],
        "F X false\ntrue\n",
    );
    assert!(infinite.contains("[[0.0, 1.0]"));
    assert!(finite.contains("[[0.0, 0.0]"));
}
