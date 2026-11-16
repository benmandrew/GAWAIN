# GAWAIN

GAWAIN (Genetic Algorithm for Weakening Against Implementations under NSGA-II) weakens a temporal specification against a model by genetic search.

The input is a model M and a *linear temporal logic* (LTL) property φ that M violates. The output is a set of *weakenings*: properties φ' that φ implies and that M satisfies, as close to φ as the search can find.

## Quickstart

[`examples/ack.smv`](examples/ack.smv) is a nuXmv model that acknowledges a request one step later, or two steps later when `slow` holds. So `G (req -> X ack)` fails on some runs. Inside `nix develop`:

```console
$ cargo build --release --bin weaken
$ ./target/release/weaken --model examples/ack.smv --atoms req,slow,ack,late \
      --ltl 'G (req -> X ack)' --seed 4 --generations 30 --population 40
input: G(req -> X ack)
instance: proper time=0.03s
initial: !ack & !req | !ack & req
repair	0.4000	0.8993	0.8944	-	G(req -> F ack)
repairs: 1
oracle: queries=840 memo_hits=304 lasso_hits=428 implication_hits=89 spot_calls=102 spot_errors=0 backend_calls=19 backend_unknowns=0
...
wall: 2.70s gate=0.07s
```

`--atoms` lists the model variables that formulas may mention. `--traces t.trace` replaces `--model` and `--atoms` when M is a set of traces. The header of [`src/main.rs`](src/main.rs) documents every flag, column and exit status.

## Building

[`flake.nix`](flake.nix) pins the Rust toolchain, Spot 2.15.1 (built from source), SPIN, nuXmv, TAPAAL and Ganak. The first `nix develop` builds Spot. With [Nix](https://nixos.org/download/) and flakes enabled:

```console
$ nix develop
$ cargo build --release
$ cargo test
```

Tests that need an external tool skip with a printed reason when the tool is not on `PATH`. Tests that read benchmark data skip when `bench/data/` is missing, since it is not in the repository.

## Documentation

| Link | Description |
|---|---|
| [Report](docs/report.md) | write-up of the problem, the benchmark and the results |
| [Plan](PLAN.md) | goal, benchmark sources, decisions and task list |
| [Search](docs/dev/search.md) | the search loop, objectives, operators and output gate |
| [Oracle](docs/dev/oracle.md) | the tiered oracle and its counters |
| [Oracle performance](docs/dev/oracle-performance.md) | cost of one query on each back end, and the tier design |
| [Oracle measurements](docs/dev/oracle-measurements.md) | per-query times and hit rates on RERS and LMCS-2006 |
| [LTL formulas](docs/dev/ltl.md) | the formula type and its canonical form |
| [Trace sets](docs/dev/trace-sets.md) | the trace-set back end and its instances |
| [RERS models](docs/dev/rers-kripke.md) | RERS Promela models as explicit Kripke structures |
| [Order-guided search](docs/dev/order-search.md) | operators that follow the implication order, and MTL |
| [Evaluation](docs/dev/evaluation.md) | research questions, instance sets and commands |
| [Benchmark notes](bench/notes/) | benchmark sources, baselines and case studies |
| [Proofs](proofs/README.md) | Lean 4 proofs of the operator orders and rewrite soundness |
