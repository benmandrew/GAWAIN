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
