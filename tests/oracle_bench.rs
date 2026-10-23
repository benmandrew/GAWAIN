//! End-to-end test of the `oracle_bench` binary: on a small nuXmv model it
//! writes one CSV row per candidate, φ first, the same stream for the same
//! seed, and only verdicts that a direct nuXmv check confirms.
//!
//! The test skips, with a reason on stderr, when nuXmv, `ltlgrind` or
//! `ltlfilt` is not on `PATH`.

mod common;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use common::{have, parse};
use gawain::oracle::nuxmv::Nuxmv;
use gawain::oracle::process::Limits;
use gawain::oracle::{Backend, Verdict};

const MODEL: &str = "MODULE main
VAR req : boolean; grant : boolean; busy : boolean;
ASSIGN
  init(grant) := FALSE; next(grant) := req & !grant & !busy;
  init(busy) := FALSE; next(busy) := grant;
";

fn bench(dir: &Path, seed: &str) -> Vec<Vec<String>> {
    let output = Command::new(env!("CARGO_BIN_EXE_oracle_bench"))
        .args(["--backend", "nuxmv", "--bench", "arbiter", "--seed", seed])
        .args(["--one", "6", "--two", "6", "--timeout", "60"])
        .arg("--model")
        .arg(dir.join("arbiter.smv"))
        .arg("--atoms")
        .arg(dir.join("atoms"))
        .arg("--properties")
        .arg(dir.join("properties"))
        .output()
        .expect("run oracle_bench");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    // No field in this test is quoted, so a plain split is enough.
    String::from_utf8(output.stdout)
        .expect("UTF-8")
        .lines()
        .map(|line| line.split(',').map(String::from).collect())
        .collect()
}

#[test]
fn oracle_bench_writes_one_checked_row_per_candidate() {
    if !have(
        &["nuXmv", "ltlgrind", "ltlfilt"],
        "oracle_bench_writes_one_checked_row_per_candidate",
    ) {
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oracle_bench");
    std::fs::create_dir_all(&dir).expect("make dir");
    std::fs::write(dir.join("arbiter.smv"), MODEL).expect("write model");
    std::fs::write(dir.join("atoms"), "req\ngrant\nbusy\n").expect("write atoms");
    let properties = "busy\tG (busy -> !grant)\nfair\tG (req -> F grant)\n";
    std::fs::write(dir.join("properties"), properties).expect("write properties");

    let rows = bench(&dir, "3");
    let header = &rows[0];
    assert_eq!(header[0], "bench");
    let column = |name: &str| header.iter().position(|h| h == name).unwrap();
    let (property, source, tier) = (column("property"), column("source"), column("tier"));
    let (verdict, backend_s, candidate) =
        (column("verdict"), column("backend_s"), column("candidate"));
    for name in ["busy", "fair"] {
        let mine: Vec<_> = rows[1..].iter().filter(|r| r[property] == name).collect();
        assert!(
            mine.len() > 1 && mine.len() <= 13,
            "{name}: {} rows",
            mine.len()
        );
        assert_eq!(mine[0][source], "phi");
        assert_eq!(mine[0][tier], "backend");
    }
    let limits = Limits {
        timeout: Duration::from_secs(60),
        memory_bytes: 2 << 30,
    };
    let mut direct = Nuxmv::new(dir.join("arbiter.smv"), ["req", "grant", "busy"], limits);
    for row in &rows[1..] {
        assert_eq!(
            row[tier] == "backend",
            !row[backend_s].is_empty(),
            "{row:?}"
        );
        let expected = match direct.check(&parse(&row[candidate])) {
            Verdict::Holds => "holds",
            Verdict::Fails(_) => "fails",
            Verdict::Unknown(reason) => panic!("{reason}"),
        };
        assert_eq!(row[verdict], expected, "{row:?}");
    }
    let again = bench(&dir, "3");
    let candidates = |rows: &[Vec<String>]| -> Vec<String> {
        rows.iter().map(|r| r[candidate].clone()).collect()
    };
    assert_eq!(candidates(&rows), candidates(&again));
}
