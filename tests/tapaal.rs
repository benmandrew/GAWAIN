//! End-to-end tests of the TAPAAL back end: `verifypn` decides small nets
//! with and without a deadlock, every counterexample replays as a
//! violation, and on two MCC instances every LTL formula gets the contest's
//! consensus verdict.
//!
//! Each test skips, with a reason on stderr, when `verifypn` is not on
//! `PATH`; the MCC tests also skip when `bench/data/mcc` lacks the instance
//! (fetch both with `bench/mcc_fetch.sh ResAllocation-PT-R003C002
//! RobotManipulation-PT-00002`).

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{have, parse};
use gawain::mcc::{properties, Count, Net, Predicate};
use gawain::oracle::process::Limits;
use gawain::oracle::tapaal::Tapaal;
use gawain::oracle::{Backend, Verdict};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 2 << 30,
};

/// `move` takes a token from p to q; `back` takes two from q and puts two
/// on p. From p = 2 the only run is move, move, back, forever.
const NET: &str = r#"<?xml version="1.0"?>
<pnml xmlns="http://www.pnml.org/version-2009/grammar/pnml">
 <net id="n" type="http://www.pnml.org/version-2009/grammar/ptnet"><page id="pg">
  <place id="p"><initialMarking><text>2</text></initialMarking></place>
  <place id="q"/>
  <transition id="move"/>
  <transition id="back"/>
  <arc id="a1" source="p" target="move"/>
  <arc id="a2" source="move" target="q"/>
  <arc id="a3" source="q" target="back"><inscription><text>2</text></inscription></arc>
  <arc id="a4" source="back" target="p"><inscription><text>2</text></inscription></arc>
 </page></net></pnml>"#;

/// Writes `source` to a fresh file under the target directory.
fn model(name: &str, source: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tapaal");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, source).unwrap();
    path
}

fn backend(name: &str, source: &str) -> Tapaal {
    let net = Net::parse(source).unwrap();
    let atoms = vec![
        (
            "full".into(),
            Predicate::Le(Count::Constant(2), Count::Tokens(vec![1])),
        ),
        (
            "mv".into(),
            Predicate::Fireable(vec![net.transition("move").unwrap()]),
        ),
    ];
    Tapaal::new(model(name, source), net, atoms, LIMITS)
}

/// Checks `formula`, and on a failure that the lasso violates it.
fn holds(backend: &mut Tapaal, formula: &str) -> bool {
    let f = parse(formula);
    match backend.check(&f) {
        Verdict::Holds => true,
        Verdict::Fails(lasso) => {
            assert!(!lasso.satisfies(&f), "{formula}: lasso satisfies it");
            false
        }
        Verdict::Unknown(reason) => panic!("{formula}: {reason}"),
    }
}

#[test]
fn verifypn_decides_a_cycling_net() {
    if !have(&["verifypn"], "verifypn_decides_a_cycling_net") {
        return;
    }
    let mut b = backend("cycle.pnml", NET);
    assert!(holds(&mut b, "G F full"));
    assert!(holds(&mut b, "X X full"));
    assert!(holds(&mut b, "G (full -> X X mv)"));
    assert!(!holds(&mut b, "G !full"));
    assert!(!holds(&mut b, "F G mv"));
    assert!(!holds(&mut b, "F (full & mv)"));
    assert!(!holds(&mut b, "(mv W full) -> X full"));
}

#[test]
fn verifypn_extends_a_deadlock_by_stuttering() {
    if !have(&["verifypn"], "verifypn_extends_a_deadlock_by_stuttering") {
        return;
    }
    let source = NET.replace(
        r#"<arc id="a4" source="back" target="p"><inscription><text>2</text></inscription></arc>"#,
        "",
    );
    let mut b = backend("deadlock.pnml", &source);
    assert!(holds(&mut b, "F G (!mv & !full)"));
    assert!(holds(&mut b, "X X X G !mv"));
    assert!(!holds(&mut b, "G F mv"));
    assert!(!holds(&mut b, "G F full"));
}

#[test]
fn verifypn_matches_the_mcc_oracle_on_resallocation() {
    matches_the_oracle(
        "verifypn_matches_the_mcc_oracle_on_resallocation",
        "ResAllocation-PT-R003C002",
    );
}

/// The default configuration answers `TRUE` for LTLFireability-02 here,
/// against the oracle's `FALSE`, so this instance tests the confirmation
/// of `TRUE` answers.
#[test]
fn verifypn_matches_the_mcc_oracle_on_robotmanipulation() {
    matches_the_oracle(
        "verifypn_matches_the_mcc_oracle_on_robotmanipulation",
        "RobotManipulation-PT-00002",
    );
}

/// Checks all 32 LTL formulas of `instance` against the MCC oracle, and
/// every lasso against its formula.
fn matches_the_oracle(test: &str, instance: &str) {
    if !have(&["verifypn"], test) {
        return;
    }
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/data/mcc");
    let dir = data.join("inputs").join(instance);
    let pnml = dir.join("model.pnml");
    if !pnml.is_file() || !data.join("oracle").is_dir() {
        let _ = writeln!(
            std::io::stderr(),
            "skipping {test}: no MCC data for {instance}"
        );
        return;
    }
    let net = Net::parse(&fs::read_to_string(&pnml).unwrap()).unwrap();
    let mut checked = 0;
    for (exam, short) in [("LTLCardinality", "LTLC"), ("LTLFireability", "LTLF")] {
        let xml = fs::read_to_string(dir.join(format!("{exam}.xml"))).unwrap();
        let oracle =
            fs::read_to_string(data.join(format!("oracle/{instance}-{short}.out"))).unwrap();
        for property in properties(&xml, &net).unwrap() {
            let expected = oracle
                .lines()
                .find_map(|l| l.strip_prefix(&format!("FORMULA {} ", property.id)))
                .and_then(|rest| rest.split_whitespace().next())
                .unwrap();
            let mut b = Tapaal::new(&pnml, net.clone(), property.atoms.clone(), LIMITS);
            let got = match b.check(&property.formula) {
                Verdict::Holds => "TRUE",
                Verdict::Fails(lasso) => {
                    assert!(!lasso.satisfies(&property.formula), "{}", property.id);
                    "FALSE"
                }
                Verdict::Unknown(reason) => panic!("{}: {reason}", property.id),
            };
            assert_eq!(got, expected, "{}", property.id);
            checked += 1;
        }
    }
    assert_eq!(checked, 32);
}
