//! End-to-end tests of the weakening search: the monotone rewrite moves the
//! way it says (checked by Spot), the output gate keeps only maximal proved
//! weakenings, and the `weaken` binary finds repairs that a direct nuXmv
//! check and Spot confirm, on a small model and on LMCS-2006 abp4 `p2false`.
//!
//! Each test skips, with a reason on stderr, when a tool or the benchmark
//! data is missing.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use aragon::ltl::Formula;
use aragon::oracle::nuxmv::{exclusive_groups, Nuxmv};
use aragon::oracle::process::Limits;
use aragon::oracle::{implications, Backend, Oracle, Verdict, Words};
use aragon::search::{
    gate, invariant, monotone_rewrite, vacuity, Direction, Rng, Scored, Vacuity, VacuityCounts,
};
use common::{formula, have, parse};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(120),
    memory_bytes: 4 << 30,
};

fn say(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}

#[test]
fn monotone_rewrites_move_the_way_they_say() {
    if !have(&["ltlfilt"], "monotone_rewrites_move_the_way_they_say") {
        return;
    }
    let names = ["a", "b", "c"];
    let atoms: Vec<std::sync::Arc<str>> = names.iter().map(|&n| n.into()).collect();
    let mut gen = common::Rng(0x005E_ED0F_0DD5);
    let mut rng = Rng::new(17);
    let mut weak = Vec::new();
    let mut strong = Vec::new();
    for i in 0..1200 {
        let subject = formula(&mut gen, 4, &names);
        if i % 2 == 0 {
            let child = monotone_rewrite(&subject, Direction::Weaken, &atoms, &mut rng);
            weak.push((subject, child));
        } else {
            let child = monotone_rewrite(&subject, Direction::Strengthen, &atoms, &mut rng);
            strong.push((child, subject));
        }
    }
    let pairs: Vec<(Formula, Formula)> = weak.into_iter().chain(strong).collect();
    let refs: Vec<_> = pairs.iter().map(|(p, q)| (p, q)).collect();
    let proved = implications(&refs, LIMITS).expect("the batch answers");
    for ((stronger, weaker), proved) in pairs.iter().zip(&proved) {
        assert!(proved, "{stronger} does not imply {weaker}");
    }
    say(&format!(
        "monotone: {} rewrites, all in their direction",
        pairs.len()
    ));
}

fn scored(text: &str, holds: bool, phi: &Formula) -> Scored {
    let formula = parse(text);
    let similarity = formula.syntactic_similarity(phi);
    Scored {
        formula,
        objectives: vec![f64::from(u8::from(holds)), similarity],
    }
}

/// An oracle over the model at `name`, written from `text`.
fn oracle_on(name: &str, text: &str, atoms: &[&str]) -> Oracle<Nuxmv> {
    let backend = Nuxmv::new(write_model(name, text), atoms, LIMITS);
    Oracle::new(backend, Words::sample(atoms.iter().copied(), 1), LIMITS)
}

#[test]
fn the_gate_keeps_maximal_proved_weakenings() {
    if !have(
        &["nuXmv", "ltlfilt"],
        "the_gate_keeps_maximal_proved_weakenings",
    ) {
        return;
    }
    // Both atoms are free, so no candidate below is vacuous on the model.
    let mut oracle = oracle_on(
        "free.smv",
        "MODULE main\nVAR a : boolean; b : boolean;\n",
        &["a", "b"],
    );
    let phi = parse("G (a -> X b)");
    let population = [
        // Implied by the next one, so not maximal.
        scored("G (a -> F b)", true, &phi),
        scored("G (a -> X b | X X b)", true, &phi),
        // Equivalent to the previous one, as similar, and later in order.
        scored("G (a -> X X b | X b)", true, &phi),
        // Holds, but φ does not imply it.
        scored("F b", true, &phi),
        // Holds and φ implies it, but it is valid.
        scored("G (a -> X b | a)", true, &phi),
        // Implied by φ, but M does not satisfy it.
        scored("G (a -> X b | b)", false, &phi),
        scored("G (a -> F b)", true, &phi),
    ];
    let outcome = gate(&phi, &population, &Formula::tt(), &mut oracle, LIMITS);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let kept: Vec<Formula> = outcome.repairs.iter().map(|r| r.formula.clone()).collect();
    assert_eq!(kept, [parse("G (a -> X b | X X b)")]);
    assert_eq!(outcome.repairs[0].vacuity, Vacuity::default());
}

/// A sender cycling through three states. `s_get & s_wait` never holds,
/// as two values of `sender.state` never do in abp4.
const CYCLE: &str = "MODULE main
VAR state : { get, send, wait };
ASSIGN
  init(state) := get;
  next(state) := case state = get : send; state = send : wait; TRUE : get; esac;
DEFINE
  s_get := state = get; s_send := state = send; s_wait := state = wait;
";

#[test]
fn the_gate_drops_validity_modulo_the_encoding_and_flags_vacuity() {
    if !have(
        &["nuXmv", "ltlfilt"],
        "the_gate_drops_validity_modulo_the_encoding_and_flags_vacuity",
    ) {
        return;
    }
    let groups = exclusive_groups(CYCLE);
    let names: Vec<Vec<&str>> = groups
        .iter()
        .map(|g| g.iter().map(|a| &**a).collect())
        .collect();
    assert_eq!(names, [["s_get", "s_send", "s_wait"]]);
    let invariant = invariant(&groups);
    let mut oracle = oracle_on("cycle.smv", CYCLE, &["s_get", "s_send", "s_wait"]);
    let phi = parse("G (s_get -> X (s_send & s_get))");
    assert!(matches!(oracle.check(&phi), Verdict::Fails(_)));
    let population = [
        // Valid modulo the encoding: its antecedent never holds.
        scored("G (s_get & s_wait -> X s_get)", true, &phi),
        // Holds and is kept, but is vacuous in s_get: M satisfies
        // G X F s_send, since the cycle passes through send forever.
        scored("G (s_get -> X F s_send)", true, &phi),
        // The control: holds, and not vacuously.
        scored("G (s_get -> X s_send)", true, &phi),
    ];
    for candidate in &population {
        assert_eq!(oracle.check(&candidate.formula), Verdict::Holds);
    }
    // Without the invariant the first candidate is a repair.
    let plain = gate(&phi, &population, &Formula::tt(), &mut oracle, LIMITS);
    assert!(plain
        .repairs
        .iter()
        .any(|r| r.formula == population[0].formula));
    let outcome = gate(&phi, &population, &invariant, &mut oracle, LIMITS);
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let flags: Vec<(Formula, Vec<Formula>)> = outcome
        .repairs
        .iter()
        .map(|r| (r.formula.clone(), r.vacuity.subformulae.clone()))
        .collect();
    // G (s_get -> X s_send) implies the X F one, so only it is maximal;
    // ask the X F one directly.
    assert_eq!(flags, [(parse("G (s_get -> X s_send)"), vec![])]);
    let mut counts = VacuityCounts::default();
    let eventual = vacuity(&population[1].formula, &mut oracle, &mut counts);
    assert!(
        eventual.subformulae.contains(&parse("s_get")),
        "{eventual:?}"
    );
    assert_eq!(counts.unknowns, 0);
}

/// A request is acknowledged one step later, or two when `slow` holds, so
/// `G (req -> X ack)` fails on some runs and not on others.
const MODEL: &str = "MODULE main
VAR req : boolean; slow : boolean; ack : boolean; late : boolean;
ASSIGN
  init(ack) := FALSE; next(ack) := (req & !slow) | late;
  init(late) := FALSE; next(late) := req & slow;
";
const ATOMS: &str = "req,slow,ack,late";
const PHI: &str = "G (req -> X ack)";

fn weaken(
    model: &Path,
    atoms: &str,
    phi: &str,
    seed: u64,
    generations: u32,
    population: u32,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_weaken"))
        .arg("--model")
        .arg(model)
        .args(["--atoms", atoms, "--ltl", phi])
        .args(["--seed", &seed.to_string()])
        .args(["--generations", &generations.to_string()])
        .args(["--population", &population.to_string()])
        .output()
        .expect("run weaken")
}

/// The repairs the binary printed.
fn repairs(stdout: &str) -> Vec<Formula> {
    flagged(stdout)
        .into_iter()
        .map(|(formula, _)| formula)
        .collect()
}

/// Each repair line as (formula, vacuity column).
fn flagged(stdout: &str) -> Vec<(Formula, String)> {
    stdout
        .lines()
        .filter_map(|line| line.strip_prefix("repair\t"))
        .map(|rest| {
            let fields: Vec<&str> = rest.splitn(3, '\t').collect();
            assert_eq!(fields.len(), 3, "similarity, vacuity, formula: {rest}");
            (parse(fields[2]), fields[1].to_string())
        })
        .collect()
}

/// Checks every repair again: nuXmv on the model, Spot for φ ⇒ φ'.
fn confirm(model: &Path, atoms: &str, phi: &Formula, found: &[Formula]) {
    let mut direct = Nuxmv::new(model, atoms.split(','), LIMITS);
    for repair in found {
        assert_eq!(
            direct.check(repair),
            Verdict::Holds,
            "{repair} fails on the model"
        );
    }
    let pairs: Vec<_> = found.iter().map(|r| (phi, r)).collect();
    let proved = implications(&pairs, LIMITS).expect("the batch answers");
    for (repair, proved) in found.iter().zip(proved) {
        assert!(proved, "{phi} does not imply {repair}");
    }
}

fn write_model(name: &str, text: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, text).expect("write model");
    path
}

#[test]
fn weaken_repairs_a_failing_property_reproducibly() {
    if !have(
        &["nuXmv", "ltlfilt"],
        "weaken_repairs_a_failing_property_reproducibly",
    ) {
        return;
    }
    let model = write_model("ack.smv", MODEL);
    let first = weaken(&model, ATOMS, PHI, 4, 30, 40);
    let stdout = String::from_utf8_lossy(&first.stdout).into_owned();
    assert!(
        first.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let found = repairs(&stdout);
    assert!(!found.is_empty(), "no repair:\n{stdout}");
    confirm(&model, ATOMS, &parse(PHI), &found);
    say(&format!("weaken on ack.smv:\n{stdout}"));

    let second = weaken(&model, ATOMS, PHI, 4, 30, 40);
    let without_wall = |text: &str| -> String {
        text.lines()
            .filter(|l| !l.starts_with("wall:"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(
        without_wall(&stdout),
        without_wall(&String::from_utf8_lossy(&second.stdout))
    );
}

#[test]
fn weaken_refuses_a_property_that_holds() {
    if !have(
        &["nuXmv", "ltlfilt"],
        "weaken_refuses_a_property_that_holds",
    ) {
        return;
    }
    let model = write_model("ack-holds.smv", MODEL);
    let output = weaken(&model, ATOMS, "G (req -> F ack)", 1, 1, 4);
    assert_eq!(output.status.code(), Some(1));
}

/// DEFINEs naming the propositions of abp4 `p2false`, appended to its last
/// module, `main`.
const ABP4_DEFINES: &str = "
DEFINE
  s_get := sender.state = get;
  s_wait := sender.state = wait_for_ack;
  s_abp := sender.abp;
  r_ack0 := r2s_out.tag = ack0;
  r_ack1 := r2s_out.tag = ack1;
";
const ABP4_ATOMS: &str = "s_get,s_wait,s_abp,r_ack0,r_ack1";
/// `p2false.ltl` over those DEFINEs.
const ABP4_PHI: &str =
    "G (s_get -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))";

#[test]
fn weaken_runs_on_lmcs_abp4_p2false() {
    let test = "weaken_runs_on_lmcs_abp4_p2false";
    if !have(&["nuXmv", "ltlfilt"], test) {
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/data/lmcs2006/fixed/abp4");
    let Ok(source) = std::fs::read_to_string(dir.join("abp4.smv")) else {
        say(&format!(
            "skipping {test}: no {} (link bench/data)",
            dir.display()
        ));
        return;
    };
    let original = std::fs::read_to_string(dir.join("p2false.ltl")).expect("read p2false.ltl");
    // The DEFINE spelling must say what the shipped property says.
    let respelled = ABP4_PHI
        .replace("s_get", "(sender.state=get)")
        .replace("s_wait", "(sender.state=wait_for_ack)")
        .replace("s_abp", "(sender.abp)")
        .replace("r_ack0", "(r2s_out.tag=ack0)")
        .replace("r_ack1", "(r2s_out.tag=ack1)");
    let squash = |s: &str| {
        s.chars()
            .filter(|c| !c.is_whitespace() && *c != '(' && *c != ')')
            .collect::<String>()
    };
    assert_eq!(squash(&respelled), squash(&original));

    let model = write_model("abp4-defines.smv", &(source + ABP4_DEFINES));
    let start = Instant::now();
    let output = weaken(&model, ABP4_ATOMS, ABP4_PHI, 1, 30, 40);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let found = repairs(&stdout);
    confirm(&model, ABP4_ATOMS, &parse(ABP4_PHI), &found);
    say(&format!(
        "weaken on abp4 p2false ({:.1}s, {} repairs, all confirmed):\n{stdout}",
        start.elapsed().as_secs_f64(),
        found.len()
    ));
    // s_get and s_wait are two values of sender.state, so the encoding
    // invariant makes the repair with both in its antecedent trivial.
    assert!(stdout.contains("exclusive: s_get, s_wait"), "{stdout}");
    let trivial = parse(
        "G (s_get & s_wait -> X (!s_get U (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1)))))",
    );
    assert!(!found.contains(&trivial), "{stdout}");
    // The U weakened to X F is kept, flagged vacuous in s_get: M satisfies
    // G X F (s_wait & ...), so it says nothing about s_get.
    let eventual = parse("G (s_get -> X F (s_wait & ((!s_abp & r_ack0) | (s_abp & r_ack1))))");
    let flags = flagged(&stdout);
    let (_, flag) = flags
        .iter()
        .find(|(formula, _)| *formula == eventual)
        .unwrap_or_else(|| panic!("no X F repair:\n{stdout}"));
    assert!(flag.split("; ").any(|s| s == "s_get"), "{stdout}");
}
