//! End-to-end tests of the explicit back end: on RERS 2016 parallel
//! problems 101 to 104, every property gets its official verdict; on 101
//! and 102, formulas with `X` get nuXmv's verdict on the same structure;
//! and every counterexample violates its formula when replayed.
//!
//! Each test skips, with a reason on stderr, when a tool it needs is not
//! on `PATH` or the benchmark data is not unpacked under `bench/data/rers`.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use gawain::kripke::Kripke;
use gawain::ltl::Formula;
use gawain::oracle::explicit::Explicit;
use gawain::oracle::nuxmv::Nuxmv;
use gawain::oracle::process::Limits;
use gawain::oracle::{Backend, Verdict};
use gawain::rers::Model;

mod common;
use common::{have, mutant, Rng};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(60),
    memory_bytes: 2 << 30,
};

fn skip(test: &str, reason: &str) {
    let _ = writeln!(std::io::stderr(), "skipping {test}: {reason}");
}

/// The `ltl pN { … }` properties of a RERS Promela file, by number.
fn properties(source: &str) -> Vec<(u32, Formula)> {
    source
        .lines()
        .filter_map(|line| line.strip_prefix("ltl p"))
        .map(|rest| {
            let (number, body) = rest.split_once(char::is_whitespace).unwrap();
            let body = body.trim().strip_prefix('{').unwrap().trim_end();
            let body = body.strip_suffix('}').unwrap();
            (number.parse().unwrap(), body.parse().unwrap())
        })
        .collect()
}

#[test]
fn rers_2016_verdicts_match_the_solutions() {
    let test = "rers_2016_verdicts_match_the_solutions";
    if !have(&["ltl2tgba"], test) {
        return;
    }
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/data/rers/2016");
    let problems = data.join("ParallelRERS2016_version2/ParallelRERS2016_version2");
    let Ok(solutions) =
        std::fs::read_to_string(data.join("participantsolutions/Parallel-solution.csv"))
    else {
        return skip(test, &format!("{} is absent", data.display()));
    };
    // `101,1,true`, one per line, with CR line endings.
    let expected: HashMap<(u32, u32), bool> = solutions
        .split(['\r', '\n'])
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split(',').collect();
            let verdict = match fields[2] {
                "true" => true,
                "false" => false,
                other => panic!("verdict {other:?}"),
            };
            (
                (fields[0].parse().unwrap(), fields[1].parse().unwrap()),
                verdict,
            )
        })
        .collect();

    let mut checked = 0;
    for problem in 101..=104 {
        let path = problems.join(format!("problem{problem}/problem{problem}.pml"));
        let source = std::fs::read_to_string(&path).unwrap();
        let kripke = Model::parse(&source).and_then(|m| m.explore()).unwrap();
        let mut backend = Explicit::new(kripke, LIMITS);
        for (number, formula) in properties(&source) {
            let holds = expected[&(problem, number)];
            match backend.check(&formula) {
                Verdict::Holds => assert!(holds, "{problem}/p{number}: proved {formula}"),
                Verdict::Fails(lasso) => {
                    assert!(!holds, "{problem}/p{number}: refuted {formula}");
                    assert!(
                        !lasso.satisfies(&formula),
                        "{problem}/p{number}: {formula} holds on {lasso:?}"
                    );
                }
                Verdict::Unknown(reason) => panic!("{problem}/p{number}: {reason}"),
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 80);
    let _ = writeln!(std::io::stderr(), "{test}: 80 of 80 verdicts match");
}

#[test]
fn a_timeout_is_unknown() {
    let test = "a_timeout_is_unknown";
    if !have(&["ltl2tgba"], test) {
        return;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "bench/data/rers/2016/ParallelRERS2016_version2/ParallelRERS2016_version2/problem101/problem101.pml",
    );
    let Ok(source) = std::fs::read_to_string(&path) else {
        return skip(test, &format!("{} is absent", path.display()));
    };
    let kripke = Model::parse(&source).and_then(|m| m.explore()).unwrap();
    // ltl2tgba takes over 10 ms to start, so 1 ms stops it every time.
    let hurried = Limits {
        timeout: Duration::from_millis(1),
        ..LIMITS
    };
    let mut backend = Explicit::new(kripke, hurried);
    match backend.check(&"G F p_c0_t2".parse().unwrap()) {
        Verdict::Unknown(reason) => assert!(reason.contains("timed out"), "{reason}"),
        other => panic!("expected a timeout, got {other:?}"),
    }
}

/// The structure as an SMV model: a state counter `s`, and each atom a
/// DEFINE that holds on the states it labels.
fn smv(kripke: &Kripke) -> String {
    let n = kripke.state_count() as u32;
    let mut out = format!("MODULE main\nVAR s : 0..{};\nASSIGN init(s) := 0;\n", n - 1);
    out.push_str("next(s) := case\n");
    for s in 0..n {
        let next: Vec<String> = kripke.successors(s).iter().map(u32::to_string).collect();
        out.push_str(&format!("  s = {s} : {{{}}};\n", next.join(", ")));
    }
    out.push_str("esac;\nDEFINE\n");
    for atom in kripke.atoms() {
        let states: Vec<String> = (0..n)
            .filter(|&s| kripke.atom(s) == Some(atom))
            .map(|s| format!("s = {s}"))
            .collect();
        let holds = if states.is_empty() {
            "FALSE".to_string()
        } else {
            states.join(" | ")
        };
        out.push_str(&format!("  {atom} := {holds};\n"));
    }
    out
}

/// SPIN cannot decide formulas with `X` on these models, so this back end
/// is the one that does. nuXmv on the same structure, written as SMV, is
/// the independent answer: every verdict must agree, and every lasso must
/// fail its formula.
#[test]
fn next_formulas_agree_with_nuxmv() {
    let test = "next_formulas_agree_with_nuxmv";
    if !have(&["ltl2tgba", "nuXmv"], test) {
        return;
    }
    let problems = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("bench/data/rers/2016/ParallelRERS2016_version2/ParallelRERS2016_version2");
    if !problems.is_dir() {
        return skip(test, &format!("{} is absent", problems.display()));
    }
    let (mut agreed, mut failed, mut with_next) = (0, 0, 0);
    for problem in [101, 102] {
        let path = problems.join(format!("problem{problem}/problem{problem}.pml"));
        let source = std::fs::read_to_string(&path).unwrap();
        let kripke = Model::parse(&source).and_then(|m| m.explore()).unwrap();
        let model = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("explicit{problem}.smv"));
        std::fs::write(&model, smv(&kripke)).unwrap();
        let atoms: Vec<String> = kripke.atoms().iter().map(|a| a.to_string()).collect();
        let mut nuxmv = Nuxmv::new(&model, &atoms, LIMITS);
        let mut explicit = Explicit::new(kripke, LIMITS);

        // Hand-made X formulas, then random GA-style mutants of the
        // properties, which bring in X and every other operator.
        let properties = properties(&source);
        let mut formulas: Vec<Formula> = (properties.iter())
            .flat_map(|(_, f)| [format!("X ({f})"), format!("X X !({f})")])
            .map(|text| text.parse().unwrap())
            .collect();
        for a in &atoms[..4] {
            for b in &atoms[..4] {
                for text in [
                    format!("G ({a} -> X !{b})"),
                    format!("F ({a} & X X {b})"),
                    format!("!{a} U X {b}"),
                ] {
                    formulas.push(text.parse().unwrap());
                }
            }
        }
        let names: Vec<&str> = atoms.iter().map(String::as_str).collect();
        let mut rng = Rng(problem as u64);
        for i in 0..80 {
            formulas.push(mutant(
                &mut rng,
                &properties[i % properties.len()].1,
                2,
                &names,
            ));
        }
        for formula in formulas {
            let text = formula.to_string();
            // RERS atom names have no capital X, so this finds the operator.
            with_next += usize::from(text.contains('X'));
            let reference = match nuxmv.check(&formula) {
                Verdict::Holds => true,
                Verdict::Fails(_) => false,
                Verdict::Unknown(reason) => panic!("nuXmv on {text}: {reason}"),
            };
            match explicit.check(&formula) {
                Verdict::Holds => assert!(reference, "{problem}: proved {text}"),
                Verdict::Fails(lasso) => {
                    assert!(!reference, "{problem}: refuted {text}");
                    assert!(!lasso.satisfies(&formula), "{text} holds on {lasso:?}");
                    failed += 1;
                }
                Verdict::Unknown(reason) => panic!("{problem} {text}: {reason}"),
            }
            agreed += 1;
        }
    }
    let _ = writeln!(
        std::io::stderr(),
        "{test}: {agreed} verdicts agree with nuXmv, {failed} false, {with_next} with X"
    );
    assert!(with_next >= 100, "only {with_next} formulas with X");
}
