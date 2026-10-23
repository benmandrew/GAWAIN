//! Times the exploration of RERS parallel Promela files.
//!
//! Usage: `cargo run --release --example rers_explore -- FILE.pml...`.
//! Prints one line per file: states, transitions and seconds.

use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let start = Instant::now();
        match gawain::rers::Model::parse(&source).and_then(|m| m.explore()) {
            Ok(k) => println!(
                "{path}: {} states, {} transitions, {:.2} s",
                k.state_count(),
                k.transition_count(),
                start.elapsed().as_secs_f64()
            ),
            Err(e) => println!("{path}: error: {e}"),
        }
    }
}
