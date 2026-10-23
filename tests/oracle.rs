//! End-to-end test: the tiered oracle over the real nuXmv back end gives the
//! verdict of a direct nuXmv check on every mutant of a few properties, and
//! the cheap tiers answer some of the queries.
//!
//! The test skips, with a reason on stderr, when nuXmv or `ltlfilt` is not
//! on `PATH`.

mod common;

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use common::{have, mutant, parse, Rng};
use gawain::ltl::Formula;
use gawain::oracle::nuxmv::Nuxmv;
use gawain::oracle::process::Limits;
use gawain::oracle::{Backend, Oracle, Verdict, Words};

/// An arbiter: `req` is free; a grant follows a request when the arbiter is
/// idle, and the step after a grant is busy.
const MODEL: &str = "MODULE main
VAR req : boolean; grant : boolean; busy : boolean;
ASSIGN
  init(grant) := FALSE; next(grant) := req & !grant & !busy;
  init(busy) := FALSE; next(busy) := grant;
";
const ATOMS: &[&str] = &["req", "grant", "busy"];

/// True and false properties of `MODEL`, the seeds of the mutants.
const PROPERTIES: &[&str] = &[
    "G (grant -> X !grant)",
    "G (grant -> X busy)",
    "G (req -> F grant)",
    "G (busy -> !grant)",
    "G (req & !busy & !grant -> X grant)",
];

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 2 << 30,
};

fn same(a: &Verdict, b: &Verdict) -> bool {
    matches!(
        (a, b),
        (Verdict::Holds, Verdict::Holds) | (Verdict::Fails(_), Verdict::Fails(_))
    )
}

#[test]
fn the_tiered_oracle_agrees_with_nuxmv() {
    if !have(&["nuXmv", "ltlfilt"], "the_tiered_oracle_agrees_with_nuxmv") {
        return;
    }
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join("arbiter.smv");
    std::fs::write(&path, MODEL).expect("write model");
    let mut direct = Nuxmv::new(&path, ATOMS, LIMITS);
    let backend = Nuxmv::new(&path, ATOMS, LIMITS);
    let mut oracle = Oracle::new(backend, Words::sample(ATOMS.iter().copied(), 1), LIMITS);

    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    let mut queries: Vec<Formula> = Vec::new();
    for text in PROPERTIES {
        let property = parse(text);
        queries.push(property.clone());
        for _ in 0..8 {
            queries.push(mutant(&mut rng, &property, 1, ATOMS));
        }
    }
    // Each query again, rewritten so only the canonical key matches.
    let repeats: Vec<Formula> = queries.iter().map(|q| parse(&format!("!!({q})"))).collect();
    queries.extend(repeats);

    let mut held = 0;
    for query in &queries {
        let expected = direct.check(query);
        assert!(
            !matches!(expected, Verdict::Unknown(_)),
            "{query}: {expected:?}"
        );
        let got = oracle.check(query);
        assert!(
            same(&got, &expected),
            "{query}: oracle {got:?}, nuXmv {expected:?}"
        );
        if let Verdict::Fails(lasso) = &got {
            assert!(
                !lasso.satisfies(query),
                "{query}: the lasso does not refute it"
            );
        }
        held += usize::from(got == Verdict::Holds);
    }
    let counters = oracle.counters();
    let _ = writeln!(
        std::io::stderr(),
        "tiered oracle: {} queries, {held} hold; {counters:?}",
        queries.len()
    );
    assert_eq!(counters.queries, queries.len() as u64);
    assert_eq!(
        counters.memo_hits
            + counters.lasso_hits
            + counters.implication_hits
            + counters.backend_calls,
        counters.queries
    );
    assert!(counters.memo_hits >= queries.len() as u64 / 2);
    assert!(counters.lasso_hits > 0, "{counters:?}");
    assert!(counters.implication_hits > 0, "{counters:?}");
    assert_eq!(counters.spot_errors, 0);
    assert_eq!(counters.backend_unknowns, 0);
}
