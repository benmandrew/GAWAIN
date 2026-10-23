# PLAN

## Goal

Adapt the genetic algorithm from PEREDUR to *specification weakening*, the problem that CEGIW (`~/projects/cegiw`) solves for interval bounds only. The input is a reference implementation M and a temporal property φ with M ⊭ φ. The output is a weakening φ' with φ ⇒ φ' and M ⊨ φ', as strong as possible.

This project solves the problem with a genetic algorithm similar to PEREDUR's ([github.com/benmandrew/PEREDUR](https://github.com/benmandrew/PEREDUR), local checkout at `~/projects/peredur`). PEREDUR repairs unrealisable FRETISH and TLSF specifications. It breeds candidate formulas, scores them by syntactic and semantic similarity (bounded model counting with SPOT and Ganak), selects under NSGA-II, and gates the output on realisability. Here the same loop applies, with the realisability check replaced by a model-checking call against M. Read PEREDUR's `CLAUDE.md` and `docs/dev/algorithm.md` and `docs/dev/operators.md` before designing anything here.

No public benchmark exists for this problem. The only paper found that states it directly is CEGIW itself ([arXiv 2604.24540](https://arxiv.org/html/2604.24540)). "Weakening Goals in Logical Specifications" ([arXiv 2510.15718](https://arxiv.org/html/2510.15718)) works in propositional logic and evaluates on a single 5-variable example. Benchmarks therefore come from neighbouring problems.

## Evaluation without a ground truth

A ground-truth weakening is not required. ⊤ always holds, so repairs are ranked by strength instead.

- **Strength.** PEREDUR's model-counting similarity between φ and φ'. Closer to φ is better, and ⊤ scores worst.
- **Baselines.** ⊤, CEGIW's interval-only weakening where φ has intervals, and the implication order between outputs of different tools where they are comparable.

One trap remains. If φ fails on *every* run of M (that is, M ⊨ ¬φ), then on M the formula ¬φ ∨ χ is equivalent to χ. Any weakening that holds is then just some true property of M, and φ contributes nothing to it. Filter every instance with one extra call: check ¬φ on M, and drop the instance if it holds. Constructions that negate a true property produce exactly this case, so they are excluded wholesale.

## Benchmark sources

Every figure below was measured on 2026-10-23 from downloaded files, unless marked unverified. The downloads lived in a session scratchpad and are gone, so re-fetch from the URLs.

### 1. RERS challenge (main third-party set)

[rers-challenge.org](https://rers-challenge.org/), editions 2012–2020 and 2022. The properties are randomly instantiated Dwyer patterns, built by property-preserving construction ([Steffen and Isberner, "Property-Driven Benchmark Generation"](http://rers-challenge.org/material/papers/Property-Driven%20Benchmark%20Generation.pdf)). They read like requirements.

| Year | Track | Problems | Properties/problem | False/total (measured) |
|---|---|---|---|---|
| 2012 | seq | 19 | 100 or 50 | 985/1600 |
| 2013 | white-box | 27 | 100 | 1960/2700 |
| 2014 | white-box | 12 | 100 | 834/1200 |
| 2015 | white-box | 3 | 100 | 165/300 |
| 2016 | seq / par | 9 / 20 | 100 / 20 | 611/900; 175/400 |
| 2017 | seq / par | 9 / 15 | 100 / 20 | 568/900; 168/300 |
| 2018 | seq / par | 9 / 3 | 100 / 20 | 604/900; 44/69 |
| 2019 | seq / par | 9 / 9 | 100 / 20 | 639/900; 90/180 |
| 2020 | seq | 9 | 10 | 57/90 |

- Solutions for 2012–2017 are text files with a lasso counterexample for each false property. Solutions for 2017–2020 are CSV files under `<year>/results/`. No solutions were found for 2019 industrial, 2020 parallel or 2022 (the link returns 404).
- The parallel track ships `.dot`, `.pml` and `.pnml` files (no PNML in 2016). The Promela files already contain `ltl pN {…}` blocks, so SPIN checks them as shipped.
- Sequential problems are C and Java with event-based I/O. Median size is 40k–90k lines for 2016–2019 and 768k lines in 2022, so they need a front end before nuXmv or SPIN.
- In 2017, the Promela solutions disagree with the dot/PNML solutions on 4 parallel problems.
- The pattern "P precedes S after Q until R" is mistranslated as `false R (Q & …)`, which is trivially false. This affects 40 of 611 false properties in 2016 and 65 of 568 in 2017. The correct pattern from the English gloss is strictly weaker than the mistranslation. Whether the correct pattern holds is unchecked.
- No licence or terms of use found.

### 2. Model Checking Contest (scale and stress set)

LTLCardinality and LTLFireability on Petri nets. The oracles are mirrored at [yanntm/pnmcc-models-2026](https://github.com/yanntm/pnmcc-models-2026) ([oracle.tar.gz](https://yanntm.github.io/pnmcc-models-2026/oracle.tar.gz)). Each property XML says "Automatically generated". The formulas are random nested X/U, for example `A ((t1? U !X X t1?) U X !X (t1? & G t2?))`, so a weakening says little about any design.

| Edition | Instances | LTL formulas | FALSE | TRUE | Unknown |
|---|---|---|---|---|---|
| 2021 | 1,411 | 45,152 | 31,195 | 12,272 | — |
| 2026 | 1,953 | 62,496 | 44,964 | 15,352 | 2,180 |

- Verdicts are contest consensus, and 2,132 of the 2026 verdicts rest on a single tool.
- The median state space is 2.9e11 (Tedd's counts, which cover 1,391 instances). Only 501 instances have 1e9 states or fewer, so explicit-state checking is out.
- Input is PNML only. LTSmin, ITS-Tools and TAPAAL `verifypn` read it. Counterexample output from these tools, and any route to nuXmv or SPIN, are unverified.
- The licence is unverified. The mirror's README claims "an open access license".

### 3. BEEM, erroneous variants

Original BEEM ([archive](https://paradise.fi.muni.cz/data/beem_models_data.tar.gz); the portal is discontinued; mirror at [plug-obp/beem-benchmark](https://github.com/plug-obp/beem-benchmark)) has 57 models, 300 instances and 76 LTL properties, for 457 LTL × instance pairs. 54 instances carry deliberate `ERROR=1/2` bugs (Peterson with `pos[k] <= j` in place of `<`), giving 96 LTL pairs. These match CEGIW's degraded-system framing: a real requirement, a buggy model.

- There are no published verdicts, so they must be computed.
- Formats are DVE (DiVinE, LTSmin) and Promela for 235 instances. The Promela files carry no LTL, and the propositions are DVE expressions that need translating.
- The its-ltl derivative ([ltl_bench.md](https://github.com/lip6/ITSTools-web/blob/master/ltl_bench.md)) is unusable. Its "822 formulae" is a line count, and the real count is 807: 406 BEEM formulae plus 401 negations. It has no verdict file.
- No licence found.

### 4. Trace sets in place of M

Here M is replaced by the positive traces. The negative traces give an optional check that φ' still rejects something.

- **LTLf Learning Benchmarks** ([repo](https://github.com/SynthesisLab/LTLf_Learning_Benchmarks), [instances](https://github.com/SynthesisLab/LTLf_Learning_Benchmarks/releases/download/benchmark/benchmarks.zip), from [arXiv 2509.24616](https://arxiv.org/pdf/2509.24616)). 10 families, 15,595 instances, finite traces of length 16, 32 or 64. Only 8,380 instances come from Spot sampling. Hamming (500 instances) stores no formula. All 2,160 OrderedSequence formulas have a stray `)` that Spot rejects. Spot confirmed on 27 sampled instances that the source formula separates all 390 positive traces from all 390 negative traces. No licence.
- **Flie** ([samples2LTL](https://github.com/gergia/samples2LTL)). 1,246 generated files over 18 Dwyer-pattern formulas, lasso traces of length 3–10, with the formula stored in the file. There are also 8 PCTCP protocol trace files with no formula, the only real-system data in any of these sets. No licence.
- **Scarlet** ([repo](https://github.com/rajarshi008/Scarlet)). 579 files of finite traces. The formula is found through `formulas.txt` by file name. MIT licence.

### 5. LMCS-2006 (minor)

From Biere, Heljanko, Junttila, Latvala and Schuppan ([README](https://fmv.jku.at/aiger/README.lmcs2006-aiger-1.9-benchmarks), [original](http://www.tcs.hut.fi/Software/benchmarks/LMCS-2006/)). 14 models and 61 pairs: 21 hold, 39 fail, 1 unknown. 31 of the 39 failures are negated true properties, which the filter above excludes. Only 8 un-negated failures remain: abp4 `p2false` and `ptimo`, bc57 `p3`, and `ptimo` on dme2–dme6. Of these, abp4 `ptimo` looks written to fail.

nuXmv 2.1.0 rejects the files, because they use NuSMV 2.4 syntax where 0/1 stand for booleans. `-old` gives wrong verdicts on `ring`. Replacing `:= 0/1` and `1 :` with FALSE/TRUE fixes this, and the checked verdicts then match the README.

Done 2026-10-23 (`bench/lmcs_fix.py`, `bench/lmcs_check.py`, `bench/results/lmcs2006-filter.csv`). The fix also renames `extend` and `K` and rewrites 0/1 module arguments, `<-> 0` and `{0,1}`. All 60 known verdicts match the README, and production-cell `p4` (unknown there) checks true. 7 of the 8 un-negated failures survive the degeneracy filter; abp4 `ptimo` is degenerate. The slowest checks on av2 were dme6 `ptimo` at 905 s and bc57 `p3` at 659 s.

### 6. Generated from SYNTCOMP (optional)

For each realisable SYNTCOMP TLSF spec, synthesise a controller with `ltlsynt` and use it as M. Then strengthen the guarantees until M violates them. This set is optional now that no ground truth is needed. It is the only source that runs on PEREDUR's existing TLSF/SPOT toolchain without a model-checker front end.

### Excluded

- HWMCC LIVE: properties are compiled into the circuit, so there is no formula left to weaken.
- SV-COMP: fixed properties (termination, overflow), not temporal formulas.
- GR(1) assumption refinement: about unrealisability, which PEREDUR already covers.
- Hanfor and Ultimate ReqAnalyzer: the industrial requirement sets are proprietary.

Not yet investigated, and close in spirit: the Lockheed Martin Cyber-Physical Challenges (LMCPS; [NASA report](https://ntrs.nasa.gov/citations/20205000433)), the ARCH-COMP falsification category ([2019 report](https://www.sosy-lab.org/research/pub/2019-ARCH.Category_Report_Falsification.pdf)), and parametric STL mining ([Jin et al.](https://people.eecs.berkeley.edu/~sseshia/pubs/b2hd-jin-tcad15.html); [Hoxha et al.](https://archive.cps-vo.org/node/50635)). All three use Simulink models with timed requirements, which fits interval weakening.

## Decisions (2026-10-23)

- **Language.** Standalone Rust in this repo. PEREDUR's core (formula representation, operators, model-counting similarity, NSGA-II) is rewritten in Rust, not linked.
- **Back ends.** nuXmv and SPIN behind one oracle interface from the start, as in CEGIW.
- **Weakening.** φ ⇒ φ' is not enforced during the search, so candidates can pass through formulas incomparable with φ. It is enforced at the output gate, one cached implication check per output, so every reported repair is a true weakening.
- **Oracle cost.** The search is only feasible with aggressive caching of tool results, as in PEREDUR. The oracle design is in `docs/dev/oracle-performance.md`.

## Tasks

- [x] Port the LTL formula core to Rust (AST, parser, SPIN and nuXmv printers, simplification, syntactic similarity).
- [x] Oracle interface: `Verdict`, `Lasso`, `Backend` (`src/oracle/mod.rs`).
- [x] Oracle tier 1: evaluate a candidate on stored lassos (`Lasso::satisfies`, `LassoCache`). Agrees with Spot on 300 random cases.
- [x] Oracle tier 2, in-process half: lasso-word fingerprints and the known-truths antichain. On 400 pairs the fingerprint refuted all 208 that Spot refuted and none of the 192 it proved.
- [x] Tier 0 memo, tier 2 Spot implication (one batched `ltlfilt` exec, 74–80 µs per pair) and the tiered `Oracle` over any back end, with per-tier counters (`docs/dev/oracle.md`).
- [x] Process runner with resource limits, and the nuXmv back end (batch mode, lasso from the trace). Every nuXmv lasso replays as a violation under the lasso evaluator.
- [ ] SPIN back end: single and batched checks (at most 200 claims per build), lasso from the trail.
- [x] RERS Promela front end exploring an explicit Kripke structure (model side of tier 3a). Counts match the note for 2016 101–104. On av2: 2016/105 in 0.13 s (78 MB), 2017/102 in 0.39 s (150 MB), 2016/107 at 64M states in 53.6 s (9.8 GB). 2018 models (global `state_N` guards) are rejected. See `docs/dev/rers-kripke.md`.
- [ ] Next wave: tier 3a product of the explicit Kripke structure with the ¬φ' automaton, as a `Backend`.
- [ ] Port operators, model-counting similarity and NSGA-II selection.
- [ ] Add the degeneracy filter (check ¬φ on M) to benchmark loading.
- [ ] RERS parallel track: load the Promela files and solution CSVs, then measure how many false properties survive the filter.
- [ ] RERS 2016/2017: check whether the correct "precedes" patterns hold, and keep the 105 affected cases as a small set with an intended weakening.
- [ ] MCC: find a tool that emits LTL counterexamples on PNML, then pick a subset of instances with 1e9 states or fewer.
- [ ] BEEM: translate the LTL propositions for the 96 ERROR pairs, and compute verdicts on both the correct and the erroneous variants.
- [ ] Trace sets: add a trace-based M (check φ' on finite traces and lassos), and fix or skip the OrderedSequence formulas.
- [ ] Baselines: ⊤ and CEGIW on the subset with intervals.
- [x] Investigate LMCPS, ARCH-COMP falsification and parametric STL mining (`bench/notes/simulink-sources.md`). ARCH-COMP models are continuous hybrid systems, and STL mining produces satisfied formulas, so neither is usable. LMCPS has 74 discrete-time requirements, about 23 of them violated, but its models need MATLAB and it carries no licence, so at most a few small models could be hand-translated to nuXmv.

The benchmarks favour RERS for credibility and MCC for volume. The first concrete step is wiring a model checker into the fitness loop and running the degeneracy filter over the RERS parallel track.
