# The tiered oracle

`Oracle<B: Backend>` (`src/oracle/tiered.rs`) answers "does M satisfy φ'?" by trying the cheap tiers of "Recommended oracle" in `oracle-performance.md` before a complete back end. This file records the order it implements, how the tiers feed each other, and the counters it keeps for the next campaign.

## Order

`Oracle::check` stops at the first tier that answers.

0. **Memo.** A map from `φ'.canonical()` to an earlier verdict. The key never renames atoms, since atoms name model variables. `Unknown` is never stored, so a timed-out query is asked again.
1. **Lasso cache.** `LassoCache::refutes` evaluates φ' on every stored run of M. A run that violates φ' refutes it. This tier never proves anything.
2. **Known truths.** `KnownTruths::may_imply` keeps only the stored truths whose fingerprint does not rule out ψ ⇒ φ'. One `ltlfilt` exec of Spot's rewriting rules (`syntactic_implications`) then tries every surviving pair. When it proves none, one exec of the complete check (`implications`) tries them again, for at most 250 ms (`COMPLETE_CHECK_BUDGET`). A proved pair proves M ⊨ φ'. This tier never refutes anything.
3. **Back end.** `Backend::check`, for example `Nuxmv`.

Each answer feeds the tiers in front of it. A failure from tier 1 or tier 3 is memoised with its lasso, and a tier 3 lasso also goes into the lasso cache. A tier 3 truth goes into `KnownTruths`. Before it does, one `ltlfilt` exec of the rewriting rules checks it against each stored truth it may imply, and each stored truth the rules prove it implies is removed. Eviction never runs the complete check. It only keeps the store small, and a store that keeps an implied truth still answers correctly, while the complete check on long formulas cost more than the model checker ("Tier 2 eviction" in `oracle-measurements.md`). A truth proved at tier 2 is not stored, since a stored truth already implies it.

## Batched implication

`implications` (`src/oracle/spot.rs`) decides ψ ⇒ φ for a batch of pairs in one exec. ψ ⇒ φ is valid iff `ψ & !φ` is unsatisfiable, so each pair becomes one stdin line `(ψ) & !(φ)`, and `ltlfilt -F - -v --satisfiable --format=%L` prints the line number of each unsatisfiable line. A line that does not parse is skipped, so it never reads as proved. A timeout, a kill or exit status 2 makes the whole batch `Err`, which every caller reads as "nothing proved". The output gate uses the same function to check φ ⇒ φ'.

`syntactic_implications` reads the same lines with `ltlfilt -F - -r2 --ap=0 --format=%L,%f`. Level 2 of Spot's simplifier applies its basic rewritings and its *syntactic implication* rules, and builds no automaton. `--ap=0` keeps only the lines rewritten to a constant, and a line rewritten to `0` is proved. Every pair it proves the complete check also proves (`tests/spot.rs`), and it answered in at most 12 ms on the 13- to 15-deep `U` chains of LMCS production-cell, where the complete check ran past 60 s on 319 of 1,261 pairs.

On 2,000 random formula and mutant pairs over three atoms (`cargo test --test spot -- --nocapture`, Spot 2.15.1), one pair took 8–16 ms and the batch 157–177 ms, so each extra pair cost 74–80 µs. That is close to the 53 µs per pair measured on RERS candidates.

Spot's own `--imply` filter is wrong on one edge case: `ltlfilt -f false --imply=true` rejects the formula. The satisfiability form gives the right answer there, and `tests/spot.rs` checks the batch against `ltlfilt -f '(ψ) -> (φ)' --equivalent-to=true` per pair for that reason.

## Counters

`Oracle::counters` returns a plain `Counters` struct.

| Field | Counts |
|---|---|
| `queries` | calls to `check` |
| `memo_hits` | tier 0 answers |
| `lasso_hits` | tier 1 refutations |
| `implication_hits` | tier 2 proofs |
| `spot_calls` | `ltlfilt` execs, for tier 2 (rules and complete check) and for evictions |
| `spot_errors` | execs that answered "unknown" |
| `backend_calls` | tier 3 calls |
| `backend_unknowns` | tier 3 calls that answered `Unknown` |

Every query is answered by exactly one tier, so `memo_hits + lasso_hits + implication_hits + backend_calls = queries`. On the arbiter model in `tests/oracle.rs` (5 properties, 8 one-subformula mutants each, then all 45 again as `!!(φ')`), the 90 queries split as 48 memo hits, 23 lasso refutations, 11 implication proofs and 8 nuXmv calls, with 11 `ltlfilt` execs, all of them the rewriting rules, which proved every one of the 11. Every verdict matched a direct nuXmv check. These mutants are not a GA population, so the split says only that each tier answers. `oracle-measurements.md` has the split and the per-query times on RERS and LMCS-2006.

## Trace sets

`TraceSet` (`src/oracle/traces.rs`) is a back end with no model checker. M is a set of *positive traces*, and M ⊨ φ' iff every positive trace satisfies φ'. A failing check returns the first positive trace that violates φ' as its counterexample. The *negative traces* play no part in the verdict. `weaken` reports, after each repair, how many of them the repair still rejects. `bench/traces_fetch.py` converts three sources into this format: the SynthesisLab LTLf Learning Benchmarks (15,095 sets), samples2LTL from Flie (1,246) and Scarlet (481).

**Semantics per source.** Every set is either all lassos, read under LTL, or all finite words, read under LTL on finite traces (LTLf). Flie's traces are lassos `p;…;p::k` with the loop starting at k, and its X is the usual LTL next. The LTLf benchmarks and Scarlet are finite. Under LTLf, GAWAIN's `X` is *weak next* (true at the last position), and `X[!]φ` parses as `!X!φ`, *strong next*. This matches Spot's `--from-ltlf`. The LTLf benchmarks write formulas in Spot's syntax, so they convert as they are. Scarlet's evaluator reads X as false past the end of a word, so its X converts to `X[!]`. F and U are strong, and G, W and R hold to the end of the word. `tests/traces.rs` checks the evaluator against `ltlfilt --from-ltlf` on 300 random formula and word pairs, and every pair agreed. `trace_bench check` then checked each source formula against its own labels: all 16,822 converted sets separate, with every positive accepted and every negative rejected. That includes the 2,160 OrderedSequence formulas whose stray trailing `)` the fetch script drops, so none had to be skipped.

**Evaluation.** A finite word is a `Lasso` with an empty cycle, and `Lasso::satisfies` evaluates both kinds. Past the end of a finite word, each fixpoint reads its `init` value, which is false for F and U and true for G, W and R. Every run in the three sources has at most 64 positions, so `TraceSet` packs each run into one `u64` per atom and evaluates a compiled post-order program with bitwise operations (`src/oracle/packed.rs`). A set with a longer run falls back to `Lasso::satisfies`. `packed.rs` checks the two evaluators against each other on 4,000 random cases.

**Which tiers stay sound.** Spot's implication is LTL implication, which is not LTLf implication. For example, `X a -> F a` is LTL-valid but fails on a one-position word, where weak `X a` holds and `F a` does not. So every `implications` call takes the back end's `Semantics`, and under `Finite` it adds `--from-ltlf=gawain_alive`. That covers tier 2, the eviction of implied truths, the validity filter and the output gate. `tests/traces.rs` checks four pairs whose LTL and LTLf answers differ or agree. The memo and the lasso cache stay sound unchanged, since a stored run is evaluated under its own semantics. The fingerprint prefilter samples lasso words, so under LTLf it can only lose completeness: a lasso word can rule out a pair that is LTLf-valid, which loses a proof but never gives a wrong one. The name `gawain_alive` is reserved, and a set that uses it as an atom is rejected.

**Two rewrites assume LTL.** `simplify` folds `X false` to `false`, but under weak X, `X false` holds at the last position. The oracle checks the formula it is given, so this changes only the vacuity flags and which candidates the search proposes. The mutation rule `NextToEventually` weakens `X φ` to `F φ`, which is unsound under LTLf for the same reason. It is used only to propose candidates. The gate proves every reported weakening with Spot under the right semantics, and `trace_bench instances` proves each instance's φ ⇒ ψ the same way.

**Memo only.** A trace-set check costs microseconds, and one `ltlfilt` exec costs about 10 ms, so tier 2 costs far more than the back end it saves. Tier 1 is no cheaper either, as it evaluates every stored run with the unpacked evaluator. `weaken --traces` therefore builds its oracle with `Oracle::memo_only`, which keeps the memo and the back end and never runs or feeds tiers 1 and 2. `trace-sets.md` has the measurements behind both choices.
