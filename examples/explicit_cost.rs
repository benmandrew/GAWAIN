//! Times the explicit back end on the properties of RERS Promela files.
//!
//! Usage: `cargo run --release --example explicit_cost -- FILE.pml...`.
//! For each file it explores the model once, then for each `ltl` property
//! times the `ltl2tgba -H` call alone and the whole `Explicit::check`. The
//! product and emptiness cost is the difference. Prints one line per
//! property and a per-file summary in milliseconds.

use std::process::Command;
use std::time::{Duration, Instant};

use aragon::ltl::Formula;
use aragon::oracle::explicit::Explicit;
use aragon::oracle::process::{self, Limits};
use aragon::oracle::{Backend, Verdict};

const LIMITS: Limits = Limits {
    timeout: Duration::from_secs(300),
    memory_bytes: 8 << 30,
};

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn main() {
    for path in std::env::args().skip(1) {
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let start = Instant::now();
        let kripke = aragon::rers::Model::parse(&source)
            .and_then(|m| m.explore())
            .unwrap_or_else(|e| panic!("{path}: {e}"));
        println!(
            "{path}: {} states, explored in {:.0} ms",
            kripke.state_count(),
            ms(start.elapsed())
        );
        let mut backend = Explicit::new(kripke, LIMITS);
        let (mut spot_total, mut product_total, mut product_max, mut n) = (0.0, 0.0, 0.0f64, 0);
        for line in source.lines().filter(|l| l.starts_with("ltl p")) {
            let (name, body) = line[4..].split_once('{').expect("ltl { … }");
            let formula: Formula = body.trim_end().trim_end_matches('}').parse().unwrap();
            let start = Instant::now();
            process::run(
                Command::new("ltl2tgba").arg("-H"),
                &format!("!({formula})\n"),
                LIMITS,
            )
            .expect("ltl2tgba");
            let spot = ms(start.elapsed());
            let start = Instant::now();
            let verdict = backend.check(&formula);
            let product = (ms(start.elapsed()) - spot).max(0.0);
            let verdict = match verdict {
                Verdict::Holds => "true",
                Verdict::Fails(_) => "false",
                Verdict::Unknown(reason) => panic!("{name}: {reason}"),
            };
            println!("  {name:4} {verdict:5} spot {spot:6.1} ms  product {product:8.1} ms");
            spot_total += spot;
            product_total += product;
            product_max = product_max.max(product);
            n += 1;
        }
        println!(
            "  mean over {n}: spot {:.1} ms, product {:.1} ms (max {:.1} ms)",
            spot_total / n as f64,
            product_total / n as f64,
            product_max
        );
    }
}
