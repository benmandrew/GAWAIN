//! End-to-end tests over the RERS parallel Promela files: every 2016, 2017
//! and 2019 file parses, the 2018 files are refused, and 2016 problems 101
//! to 104 explore to the sizes in `docs/dev/oracle-performance.md`.
//!
//! Each test skips, with a reason on stderr, when the benchmark data is not
//! unpacked under `bench/data/rers`.

use std::io::Write;
use std::path::{Path, PathBuf};

use gawain::rers::Model;

/// The directory of each year's problems, under `bench/data/rers`.
const YEARS: &[(&str, &str)] = &[
    (
        "2016",
        "2016/ParallelRERS2016_version2/ParallelRERS2016_version2",
    ),
    ("2017", "2017/ParallelRERS2017_version2/ParallelRERS2017"),
    (
        "2018",
        "2018/ParallelLtlRers2018WithPml/ParallelLtlRers2018WithPml",
    ),
    ("2019", "2019/ParallelRers2019Models/ParallelRers2019Models"),
];

/// The problem directory of `year`, or `None` after printing why the test
/// skips.
fn year_dir(year: &str, test: &str) -> Option<PathBuf> {
    let (_, rel) = YEARS.iter().find(|(y, _)| *y == year).unwrap();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("bench/data/rers")
        .join(rel);
    if dir.is_dir() {
        return Some(dir);
    }
    let _ = writeln!(
        std::io::stderr(),
        "skipping {test}: {} is absent",
        dir.display()
    );
    None
}

/// Every `problemNNN/problemNNN.pml` under `dir`, in name order.
fn problems(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_string();
            Some(p.join(format!("{name}.pml")))
        })
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

fn parse(path: &Path) -> Result<Model, gawain::rers::Error> {
    Model::parse(&std::fs::read_to_string(path).unwrap())
}

#[test]
fn every_rers_file_parses_except_2018() {
    for (year, _) in YEARS {
        let Some(dir) = year_dir(year, "every_rers_file_parses_except_2018") else {
            continue;
        };
        let files = problems(&dir);
        assert!(!files.is_empty(), "no problems in {}", dir.display());
        for path in files {
            let result = parse(&path);
            if *year == "2018" {
                let e = result.unwrap_err();
                assert!(e.message.contains("unsupported"), "{}: {e}", path.display());
            } else if let Err(e) = result {
                panic!("{}: {e}", path.display());
            }
        }
    }
}

#[test]
fn rers_2016_sizes_match_the_note() {
    let Some(dir) = year_dir("2016", "rers_2016_sizes_match_the_note") else {
        return;
    };
    let expected = [
        (101, 11, 13),
        (102, 155, 358),
        (103, 2_253, 6_956),
        (104, 19_255, 71_764),
    ];
    for (problem, states, transitions) in expected {
        let path = dir.join(format!("problem{problem}/problem{problem}.pml"));
        let k = parse(&path)
            .and_then(|m| m.explore())
            .unwrap_or_else(|e| panic!("{problem}: {e}"));
        assert_eq!(
            (k.state_count(), k.transition_count()),
            (states, transitions),
            "problem{problem}"
        );
    }
}
