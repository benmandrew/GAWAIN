# Oracle performance

A genetic algorithm asks the model-checking oracle "does M ⊨ φ'?" tens of thousands of times per run. This note measures what one such query costs on each back end, tests which cheaper checks can answer most queries before a model checker runs, and recommends a tiered oracle.

## How the numbers were taken

All numbers come from the local development box (20 cores, 30 GB, shared with other users; load average 6 to 16 during the runs). Unless marked otherwise, every tool ran under `systemd-run --user --scope -p MemoryMax=4G -p MemorySwapMax=0 nice -n 10 timeout <s>`, so the figures include a niced, memory-capped scope. The first SPIN breakdowns and the first `gcc` timings on problem101 and problem120 ran before that rule, outside a scope; they are marked "unscoped". Heavy runs were meant to go to `av2` or `av3`, but the SSH call was refused by the permission system, so no number here comes from those hosts. Per-call costs will differ there.

Tool versions: SPIN 6.5.2, nuXmv 2.1.0, Spot 2.15.1 (CLI), gcc 11.4.0. The model is RERS 2016 parallel problems 101 to 106, plus RERS 2017 parallel 101 and 102 for state-space sizes. Candidates were mutants of the RERS properties made by Spot's `ltlgrind` (one and two mutations), which stand in for GA offspring.

The measurement scripts live in the session scratchpad and are not committed. Each was checked against the RERS 2016 solution file before its timings were used: SPIN matched 20 of 20 verdicts on problem101, nuXmv matched 20 of 20 on problem101 and 104, and the explicit-state route below matched 80 of 80 on problems 101 to 104.

## What carries over from PEREDUR

PEREDUR's `docs/dev/performance.md` and the code behind it give five ideas. Four carry over, one with a change.

**Memoised verdicts on a canonical key.** PEREDUR keys every tool cache on `formula_key::canonical()` (commutative operands flattened, ordered and deduplicated, double negation dropped) or `formula_key::renamed()`, which also renames atoms to a canonical sequence. `renamed()` is sound there because satisfiability and model counts do not change under a bijection on atoms. It is **not** sound here: the atoms name observations of M, so `G !p_a` and `G !p_b` have different verdicts on the same M. The oracle cache must use `canonical()` or a stronger *equivalence-preserving* normal form (for example Spot's `ltlfilt --simplify` output). Renaming is sound only by a permutation that is an automorphism of M, which is not worth computing. A model-specific normalisation is also sound: in RERS, at most one action atom holds at any position, so rewriting under that invariant (Spot's `--exclusive-ap`) keeps the verdict. PEREDUR measured that canonical keys remove 13.9% to 22.1% of tool execs depending on the cache; aragon should expect the lower end, since the renaming that gave PEREDUR its extra 2 to 10 points is unavailable.

**Monotone subsumption.** PEREDUR's `RealizabilityChecker` answers a query from an earlier one when realisability is monotone in the change, and removed 26.2% of `ltlsynt` execs with it. The analogue here is the implication order: if M ⊨ ψ and ψ ⇒ φ' then M ⊨ φ', and if M ⊭ ψ and φ' ⇒ ψ then M ⊭ φ'. Both are measured below.

**Lasso-word fingerprints.** `fingerprint::prefilter` evaluates formulas on 256 random lasso words and refutes 95.9% of implication pairs with two machine-word compares. The evaluator (`src/fingerprint/lasso.hpp`) is the piece aragon needs most, because it is also how a counterexample cache checks a candidate against a stored lasso. It caps a lasso at 64 positions (one bit per position). The RERS counterexamples below averaged 7 to 38 positions per property, so the cap fits most of them but needs a fallback path.

**Subprocess hygiene.** Formulas go to tools on stdin through a `memfd`, never as argv (Linux caps one argv string at 131,072 bytes); pipes use `pipe2(O_CLOEXEC)`; one function does every fork and sets `PR_SET_PDEATHSIG`. All of this carries over unchanged.

**Coalescing concurrent misses** was tried in PEREDUR and removed, since the mean batch size was 1.012 and wall time rose 37%. Batching does pay off here for SPIN, where one compile can serve many claims, but only because the GA can hand over a whole generation at once (see below).

## Back end 1: SPIN

A SPIN query has three stages: `spin -a` writes `pan.c`, `gcc` compiles it, and `pan` searches. Times in ms, three runs each, `gcc -O0`, `pan -a -m200000 -w18`:

| Problem | Property | `spin -a` | `gcc -O0` | `pan` | States stored |
|---|---|---|---|---|---|
| 2016/101 | p1 (true) | 15–21 | 358–633 | 10–25 | 14 |
| 2016/101 | p8 (false) | 18–21 | 345–397 | 10–12 | 14 |
| 2016/104 | p1 (false) | 23–26 | 629–766 | 16–19 | 596 |
| 2016/104 | p8 (true) | 18–23 | 605–652 | 573–625 | 113,218 |

The compile dominates every query except a true property on a non-trivial model. Four details matter:

- `gcc -O2` is the wrong default. It took 2.36 s against 0.51 s for `-O0` on problem101 and 15.6 s against 2.1 s on problem120 (hyperfine, 5 runs, unscoped). It speeds the search only on large ones: problem104 p8 took 381–460 ms in `pan` at `-O2` against 573–625 ms at `-O0`.
- `pan`'s default hash table (`-w24`) costs 128 MB and about 100 ms before the search starts: on problem101, `pan` took 0.10 s at 133,440 KB peak by default and under 0.01 s at 4,480 KB with `-w18`. A large `-m` costs the same way: `-m10000000` took 0.57 s at 679,840 KB on a 23-state search. Size `-w` and `-m` from the model's state count.
- One `pan.c` can hold many claims, and `pan -N name` picks one at run time. On problem104, 1 claim compiled in 0.81 s, 20 in 0.79 s, 100 in 1.74 s (`spin -a` plus `gcc`) and 400 in 10.9 s. At 100 claims that is 17 ms of compile per claim, against about 650 ms for a single-claim build.
- Batching has a hard ceiling that fails silently. With 260 or more claims in one file (problem104 has 6 processes), `pan` reported a violation, "end state in claim reached at depth 3", for a candidate that holds and that the 240-claim and 250-claim builds verified. This looks like SPIN's limit of 255 proctypes, since each claim is one. Batches must stay well under 255 minus the process count. The state vector also grows with the batch (176 bytes at 20 claims, 400 at 240), so large batches slow every search a little.

SPIN's state space for RERS is much larger than the model's. The Promela encoding routes each action through a `Listener` process by rendezvous and interleaves the components, so SPIN stores intermediate states that no property can observe. Without a claim and with partial-order reduction off, SPIN stored 34 / 1,537 / 70,933 / 1,870,810 states for 2016 problems 101 to 104 (unscoped), against 11 / 155 / 2,253 / 19,255 states in the action-level Kripke structure below, about 100 times more on problem104. On problem105 the unscoped run reached 5.4 GB in 71 s and was killed, so it **did not complete locally**. A scoped run of p1 with `-DMEMLIM=3500` stopped at 34,000,000 stored states after 311 s with no verdict. Partial-order reduction (on by default) is sound only for stutter-insensitive formulas, so a GA operator that introduces `X` must turn it off.

## Back end 2: nuXmv

nuXmv has no Promela front end, so the RERS models were translated to SMV from the same Promela (one enumerated variable per component plus `last`, the last action; one `TRANS` disjunct per action). nuXmv reports 19,255 reachable states for problem104, the same count as the explicit Kripke structure.

**Batch mode, one process per query** (hyperfine, 10 runs): 30 ms on problem101 and 786 ms on problem104 (property p2). A file with no specification takes 33 ms on problem104, so start-up is cheap and the cost is the check itself. `-dynamic` made it slower (1.37 s).

**Persistent session** (`nuXmv -source` script: `go` once, then repeated `check_ltlspec -p`): on problem101, 40 queries took 0.10 s, or 2.5 ms each against 30 ms in batch mode. On problem104 the session saves almost nothing, because the cost is the per-formula tableau and fair-state fixpoint, not the model build. Per query, in one session:

- 11 true properties: 0.03–0.13 s each.
- 9 false properties: 0.38–13.2 s each, with counterexample generation on. With `set counter_examples 0` the false ones took 0.38–6.18 s, but the oracle then loses the lasso it needs for the cache below.

**BMC** (`check_ltlspec_bmc -k 30`) took 389 ms per query on the 11-state problem101 and cannot prove a property true: it printed "no counterexample found" for all 60 true queries. It is a falsifier only. **IC3** (`check_ltlspec_ic3` after `build_boolean_model`) took 55 ms per query on problem101 and 1.9 s per query on problem104 (37.9 s for 20). Neither beats BDD checking here.

## Back end 3: explicit Kripke structure and an automaton product

RERS parallel models are finite and small at the action level. A Python script parsed each Promela file into component transition systems, composed them (local actions alone, channel actions as send/receive pairs), and explored the reachable states, where a state is the component states plus the last action. Deadlocked states get a self-loop, as SPIN's stutter extension does. This keeps exactly what an `X`-free property can observe.

| Problem | Components | States | Transitions | Python explore time |
|---|---|---|---|---|
| 2016/101 | 1 | 11 | 13 | < 0.01 s |
| 2016/102 | 2 | 155 | 358 | < 0.01 s |
| 2016/103 | 3 | 2,253 | 6,956 | 0.01 s |
| 2016/104 | 4 | 19,255 | 71,764 | 0.11–0.16 s |
| 2016/105 | 5 | 499,041 | 2,295,530 | 6.6 s |
| 2016/106 | 6 | 649,697 | 3,505,673 | 11.1 s |
| 2017/101 | 4 | 20,093 | 69,187 | 0.09 s |
| 2017/102 | 6 | 1,163,558 | 5,780,665 | 9.9 s |

2016 problems 107 to 112 did not complete locally inside 4 GB and 300 s. The Python explorer stores each state as a tuple of Python objects, so a compact explorer in Rust should reach further, though how far is unmeasured. Above about 10^8 states, explicit exploration stops being an option and the symbolic back ends are the only route.

The explored structure was written as HOA (state-labelled, acceptance `t`), and each query ran `ltl2tgba '!φ'` and then `autfilt model.hoa --intersect=neg.hoa -c`. The negation took 11.5–13.1 ms per formula (mean over 20, almost all of it process start-up). The product cost:

- problem101 to 103: 12–22 ms per query.
- problem104: 75–90 ms per query (hyperfine), of which 69 ms is parsing the 5.2 MB HOA file. The product and emptiness check itself costs 6–21 ms.
- problem105, with the HOA projected onto the formula's 4–5 atoms (32–34 MB): parsing took 0.70–1.25 s, and the whole query 1.5–6.8 s while another experiment was running.

So per query, the parse is the main cost, and an oracle that keeps M in memory and builds the product with the ¬φ' automaton in process skips it. On problem104 that leaves the 6–21 ms product and the automaton translation. This is the cheapest complete check measured here, 20 to 30 times cheaper than SPIN's single-claim compile (about 650 ms) and comparable to nuXmv's best case on true properties, while a false verdict also yields the lasso for free (`autfilt --product ... --stats=%w`).

Neither model checker is a good source of that graph. `pan -d` and `-D` print the process automata, not the reachable graph. nuXmv's `print_fair_transitions -v -f csv` does dump it, but on problem104 it took 3.25 s and wrote 65,537 lines against 71,764 transitions, so the output looks truncated at 2^16 rows. For Promela inputs, a front end that explores M itself is the dependable route.

## Counterexample (lasso) cache

Every false verdict yields a lasso u·v^ω of M with u·v^ω ⊭ φ'. The cache stores these lassos, and a new candidate whose evaluation on any stored lasso is false is refuted without a model-checking call. This is sound, because each lasso is a real run of M. It can never prove a candidate true.

**Measured hit rate.** For each false RERS property, the mutants were walked in a random order. Each candidate was checked against the stored lassos first, then given a ground-truth verdict by the explicit back end, and on a cache miss its counterexample was added.

| Problem | Candidates | False | Refuted by cache | Lassos stored | Cache check (Python) |
|---|---|---|---|---|---|
| 2016/101 (8 properties) | 378 | 277 | 265 (95.7%) | 12 | 38–142 µs |
| 2016/104 (9 properties) | 704 | 598 | 569 (95.2%) | 29 | 37–682 µs |

At most 6 lassos per property were enough, and the cache never refuted a true candidate. The rate is high because mutants of one φ fail for the same few reasons: most refutations happen on the run that already refutes φ. A GA population descends from φ in the same way, so the same holds there, but later generations contain more true candidates, which the cache cannot answer. Mean lasso length was 7 to 38 positions. The cache check took microseconds in Python, hundreds of times cheaper than the 150–470 ms model-checking call that produced each ground truth.

The cache also makes the negative half of the implication order redundant. If M ⊭ ψ and φ' ⇒ ψ, then the lasso stored for ψ violates ψ and therefore φ', so the lasso test already refutes φ' with no implication check. Only the positive half (M ⊨ ψ, ψ ⇒ φ') needs its own tier.

## Implication from known truths

On the same problem104 walk, 106 candidates were true. For each one, Spot was asked whether any earlier true candidate of the same property implies it (`ltlfilt -F known --imply=φ'`, one exec per candidate against a mean of 11.5 known truths). **66 of 106 (62%)** were proved true this way, at 10.8 ms per exec. A single implication exec costs 12.9 ms, the same as an `ltlfilt` that does nothing (13.9 ms), so start-up dominates. Batched, 2,400 implication checks ran in one 128 ms exec, about 53 µs each. Some of the 66 are equivalent rewrites that a canonical key would also catch; the experiment did not separate the two.

The known-true set can grow large, so it should be kept as an antichain of the strongest truths found so far, and pairs should go through PEREDUR's lasso fingerprint first (which refuted 95.9% of pairs there). A known-true ψ can imply φ' only if every sampled word that satisfies ψ also satisfies φ', and the fingerprint test finds most counterexamples to that. The aragon port (`src/oracle/implication.rs`) refuted 208 of 212 Spot-refuted pairs (98.1%) on 400 random formula and mutant pairs over three atoms, and ruled out none of the 188 implied ones.

Overall, on problem104 the two cheap tiers answered 635 of 704 candidates (90.2%). Only 69 (9.8%) needed a model-checking call.

## Persistent processes, parallelism and resource limits

Fixed per-process costs: a Spot CLI call costs 13–14 ms before it does any work, nuXmv 30–33 ms, and a SPIN query 350–970 ms of compile. The oracle should therefore (a) do the lasso and fingerprint tiers in process, (b) send implication queries in batches, one `ltlfilt` exec per candidate, or link Spot (C++) through FFI, (c) keep M in memory for the explicit back end, (d) use one long-lived nuXmv session per worker thread, and (e) for SPIN, compile one `pan.c` per generation with at most about 200 claims and run `pan -N` per candidate.

Queries are independent, so they parallelise without coordination. Measured peak RSS per query: `pan` 4.5 MB at `-w18` but 133 MB at the default `-w24`; nuXmv 48 MB on problem104; `autfilt` 24 MB on problem104 and 163 MB on problem105.

These figures are per call, and a SPIN search on problem105 grew to 5.4 GB in 71 s, so the oracle must cap its own children. The recommended order:

1. Put the whole worker pool in one cgroup v2 scope with `MemoryMax`, `MemorySwapMax=0` and a CPU weight, and give each child its own sub-cgroup with a per-query `MemoryMax`. The memory OOM killer then hits only that child, which the oracle reports as "unknown (out of memory)", never as a verdict. Under systemd that means starting aragon under `systemd-run --user --scope -p MemoryMax=...`, or creating sub-cgroups under the delegated user subtree and writing the child's PID into `cgroup.procs` before `exec`.
2. Where cgroup delegation is unavailable (macOS, containers), set `RLIMIT_AS` and `RLIMIT_CPU` with `setrlimit` in the spawn path between fork and exec (`pre_exec` in Rust's `Command`). `RLIMIT_AS` counts virtual memory, so set it with headroom.
3. Pass the tool's own limit too: `pan` takes `-DMEMLIM=<MB>` at compile time and stops cleanly, and nuXmv takes a time limit. A clean stop gives a better message than a kill.
4. Start each child in its own process group (`setsid`), kill the group on timeout, and keep PEREDUR's `PR_SET_PDEATHSIG`, since `pan` and `gcc` spawn their own children.

A timed-out or killed query has no verdict. The oracle must not cache it as false, and must not let the GA treat it as true.

## Recommended oracle

Order the checks cheapest first and stop at the first answer:

| Tier | Check | Answers | Measured cost |
|---|---|---|---|
| 0 | Verdict memo keyed on `canonical()` or Spot-simplified text (never renamed atoms) | repeats, both verdicts | a hash lookup |
| 1 | Evaluate φ' on every stored lasso of M | false | 37–682 µs in Python; refuted 95.2–95.7% of false mutants |
| 2 | Fingerprint prefilter, then Spot implication from the antichain of known truths | true | 10.8 ms per candidate as one exec; 53 µs per pair batched; proved 62% of true mutants |
| 3a | Explicit M in memory, product with the ¬φ' automaton, emptiness check; store the lasso on failure | both | problem104: 12 ms translation + 6–21 ms product (75–90 ms through the CLI) |
| 3b | nuXmv, one persistent session per worker | both | problem104: 0.03–0.13 s true, 0.38–13.2 s false |
| 3c | SPIN, one `pan.c` per generation with ≤ 200 claims | both | problem104: 17 ms compile per claim at 100 claims, plus 16–625 ms search |

Tier 3a should be the default whenever M can be explored, which covers RERS 2016 parallel problems 101 to 106 and 2017 problems 101 and 102. Use nuXmv for models too large to explore. Keep SPIN for parity with CEGIW and for Promela models with data that neither of the others handles, and run it batched. Every complete check that fails feeds tier 1; every one that succeeds feeds tier 2.

With tiers 1 and 2 answering 90.2% of candidates on problem104, a run of 20,000 queries reaches a model checker about 2,000 times. At tier 3a's measured cost (about 30 ms on problem104) those 2,000 calls take about a minute; as single-claim SPIN builds (about 650 ms each) they would take about 22 minutes, and all 20,000 queries without the cheap tiers about 3.6 hours.

Open questions:

- These hit rates come from `ltlgrind` mutants. They should be re-measured on real GA populations, where later generations contain more true candidates.
- Formulas with `X` break both SPIN's partial-order reduction and the action-level Kripke structure. Either keep the GA's operators `X`-free (no RERS property uses `X`) or fix the model semantics before allowing `X`.
- None of these timings come from `av2` or `av3`. Re-run the tier 3 timings there before sizing a campaign.
