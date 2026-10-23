# ARAGON

Specification weakening against a model by genetic search, in Rust. The goal, benchmarks, decisions and task list are in `PLAN.md`. The oracle design is in `docs/dev/oracle-performance.md`, and the search loop and output gate in `docs/dev/search.md`.

## Rust code standards

- **Minimal.** Write only what a current caller or test needs. Do not port PEREDUR code ahead of its first use, and add no option, feature flag or abstraction without a second use.
- **Tested.** Every module has unit tests. Every path that reaches an external tool (SPIN, nuXmv, Spot) has an end-to-end test under `tests/` that runs the real tool. A test skips with a printed reason when the tool is not on `PATH`, and never passes silently.
- **Documented.** `#![deny(missing_docs)]` holds in the crate root. Every module has a `//!` header saying what it is for. Design reasons that span modules go in `docs/dev/`.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings` and `cargo test` are clean before every commit.

## Running tools

- Every tool launch, including cargo, goes through `systemd-run --user --scope -q -p MemoryMax=<n>G -p MemorySwapMax=0 nice -n 10 timeout <s> …`, so a runaway process cannot kill the terminal. Run at most 6 at once locally.
- Expensive runs go to `ssh av2` or `ssh av3` (32 cores and 125 GB each). Never use av1. The home directory is not shared: rsync inputs to `~/aragon/` and copy the results back. spin and nuXmv are in `~/.local/bin` there.
- Agents working in a worktree do not edit `PLAN.md`; the main session keeps the task list, so parallel branches do not conflict on it.
- `bench/data/` is gitignored, so a worktree lacks it and data tests skip there. Link it first: `ln -s ~/projects/aragon/bench/data bench/data`.
- Atoms are tied to the model, so a cache key that renames atoms is unsound here, unlike in PEREDUR.
