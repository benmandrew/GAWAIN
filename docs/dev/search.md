# The weakening search

`src/search/` breeds candidate weakenings φ' of one LTL property φ that the model M violates, scores them with the tiered oracle, and selects under NSGA-II. An output gate then keeps only the candidates proved to be weakenings that hold on M and that no other survivor beats. The `weaken` binary (`src/main.rs`) runs it over the nuXmv back end. This file records the loop, the objectives, the gate, and which parts of PEREDUR (`~/projects/peredur`) were left out and why.

## The loop

An individual is one formula. The seed population is P copies of φ, collapsed by the survivor step to one distinct member replicated P times. Each of G generations then runs four steps.

1. **Breed.** Each of the P slots starts as its parent, is crossed with a parent drawn from the whole population at rate 0.1, is mutated at rate 1.0, and is passed through `simplify`. Crossover and mutation interleave per slot, as in PEREDUR's `breed_offspring`, so the draw stream depends on that order.
2. **Filter.** Repeats up to `canonical()` are dropped, then anything larger than twice φ's size (PEREDUR's bloat cap), then anything `simplify` folds to `true`, then anything Spot proves valid modulo the encoding invariant (see below). A dropped candidate costs no oracle call.
3. **Score.** Each survivor is checked by the oracle, in population order.
4. **Select.** Parents and offspring are pooled, as in a (μ+λ) step, and PEREDUR's `nsga2-apportion` survivor step applies. Repeats up to `canonical()` are dropped, the rest are sorted by the crowded-comparison order (front rank, then crowding distance, stable), and the result is truncated to P or replicated up to P. Replication gives each survivor one slot and shares the spare slots by weight `1 / (1 + rank)` under the largest-remainder method.

Mutation draws atoms from φ's atoms only. The model's other atoms, listed in `--atoms`, are out of scope for now, though `--atoms` must still cover φ's, since a lasso reads an unlisted atom as false.

The random source is SplitMix64 with Lemire's bounded reduction (`src/search/rng.rs`). PEREDUR's rule holds: each draw goes into its own local, never two draws as arguments of one call, and a probability of 0 or 1 costs no draw. The stream is this crate's own and does not match PEREDUR's mt19937 stream.

## Operators

Mutation (`src/search/operators.rs`) is `tlsf_mutate` with the slot and section draws removed. With probability 0.25 it is a monotone rewrite, in a direction drawn on a fair coin. Otherwise, with probability 0.2, it is Brizzio et al.'s temporal mutation, which may add, drop or swap temporal operators, with PEREDUR's arm that keeps a binary node's connective. The remaining mutations rewrite only the maximal propositional subtrees, through PEREDUR's `mutate_formula`, so the temporal skeleton is kept exactly.

The monotone rewrite draws a site uniformly over the nodes of determinate *polarity*, and then one rule that moves that node the asked way. A weaker subformula weakens the whole only at a positive occurrence, so `!` and the antecedent of `->` flip the direction, and nothing under `<->` is a site. `tests/search.rs` checks 1,200 rewrites (600 each way, random formulas of depth 4 over three atoms) against Spot in one batch, and every one moves in its stated direction.

One rule differs from PEREDUR. Its `GloballyToInfinitelyOften` builds `G` of the whole node, which gives `G G φ`, a formula equivalent to its input. This port builds the documented `G F φ`.

Crossover is one side of `tlsf_crossover`. On a fair coin, a subformula of the first parent is replaced by one of the second, or joined with one under `&`, `|`, `U` or `W`. The graft sites are the distinct temporal subformulae, or the whole formula when there are none, and only the first occurrence of the chosen site gives way.

## Objectives

`score` returns a list of two objectives, both higher-is-better.

- **Status.** 1 if the oracle answers `Holds`, 0 if it answers `Fails` or `Unknown`. The oracle never memoises `Unknown`, so a candidate that timed out is asked again if it is bred again.
- **Syntactic similarity** to φ (`Formula::syntactic_similarity`).

Model-counting similarity, the measure of strength that PLAN.md ranks repairs by, joins this list as a third entry. The NSGA-II functions take objective vectors of any width, so nothing else changes. No trait was introduced for it, because no second implementation exists yet.

## The output gate

The gate (`search::gate`) runs once, on the final population.

1. Keep the distinct candidates with status 1.
2. In one `ltlfilt` batch, ask for each candidate whether φ implies it and whether the encoding invariant I implies it (see below). Keep the ones proved implied by φ and not proved implied by I. A failed batch proves nothing, so every candidate drops.
3. In a second batch, ask every ordered pair of survivors whether one implies the other. Drop a candidate that a strictly stronger survivor implies. Of two equivalent survivors, keep the one more similar to φ, then the smaller in formula order. A failed batch proves no implication, so every candidate stays.
4. Flag where each surviving repair is vacuous on M (see below). This step asks the oracle, not Spot, and never drops a repair.

So every reported repair φ' has M ⊨ φ' proved by the oracle and φ ⇒ φ' proved by Spot. Spot does not prove I ⇒ φ', and does not prove any other reported repair stronger than it.

## Triviality: validity modulo the encoding

A candidate that holds on M can still hold for a trivial reason. On abp4, `G(s_get & s_wait -> X(…))` holds only because `s_get` and `s_wait` are two values of one enumerated variable, `sender.state`, and never hold together. Spot cannot see this, since it does not know the model, so plain validity lets the candidate through.

The fix is to tell Spot what the encoding guarantees. The atoms fall into groups of mutually exclusive atoms, and the encoding invariant I is `G` of `!(a & b)` over every pair within a group (`search::invariant`). A candidate φ' is trivial when I ⇒ φ' is valid, that is, when `I & !φ'` is unsatisfiable. I takes the place of `true` in the validity batch of each generation's filter and in the gate, so the check adds no process. With no groups, I is `true` and the check is plain validity.

The groups come from three places.

- `nuxmv::exclusive_groups` reads the DEFINEs of `MODULE main`. DEFINEs of the form `name := v = c` with the same `v` and distinct constants `c` form a group, and `name := v` and `name := !v` count as `v = TRUE` and `v = FALSE`. Two names for the same constant are equivalent, not exclusive, so only the first joins. Any other DEFINE is skipped.
- Each `--exclusive a,b,…` flag adds a group by hand.
- On a RERS model, at most one action atom holds per state, so all action atoms would form one group. The `weaken` binary runs only the nuXmv back end, so no code derives this group yet; `--exclusive` can give it.

The binary keeps only the atoms listed in `--atoms` in each group, drops groups of one, and prints each group at start-up as `exclusive: a, b`.

## Vacuity is a flag, not a filter

The standard test of vacuity (Beer, Ben-David, Eisner and Rodeh; Kupferman and Vardi) asks whether M also satisfies φ' with a subformula occurrence ψ replaced by the constant that makes φ' stronger: false where ψ has positive polarity, true where it has negative polarity. That stronger formula is a *witness*. If M satisfies it, φ' is vacuous in ψ.

A first version of the gate dropped every candidate vacuous on M. On abp4 it dropped every repair, including `G(s_get -> X F(s_wait & (…)))`, the `U` of φ weakened to an eventuality. That repair is vacuous in `s_get`, because M satisfies `G X F(s_wait & (…))`. But it is still a useful answer: it is the strongest weakening found that φ implies. Vacuity cannot tell a repair that says nothing from one that is weaker than some other truth of M that φ does not imply. Validity modulo the encoding can tell these apart, since it asks only about the encoding and not about M. So the filter is validity modulo I, and vacuity is a flag on each reported repair.

`operators::vacuity_witnesses` builds one witness per subformula occurrence, simplified, paired with its ψ. It uses the polarity rules of the monotone rewrite, so it skips both children of `<->` and everything under them. It also skips the whole formula and the constants. A witness that simplifies to false is left out, because it cannot hold on a model with a run. A witness that simplifies to true holds with no query. `search::vacuity` asks the same oracle as the search about every witness of each reported repair, so the cheap tiers answer most of them. A witness is stronger than the repair, so a lasso that refutes the repair refutes the witness too.

The `weaken` binary prints the flag as the third column of each repair line: the subformulae in which the repair is vacuous, separated by `; `, or `-` when there are none. A witness whose verdict is unknown adds `?`. A line `vacuity: queries=… unknowns=… backend_calls=…` gives the cost, and the gate's own time is on the `wall` line.

## The validity filter

The brief asked for a syntactic validity check only. That was not enough. On the first runs, with only the `simplify` check in the filter, the gate reported one repair on each of the two end-to-end models, and both were valid: `G(ack -> X(!req <-> ack) | ack)` and a formula ending in `s_wait W (… | !s_wait)`. A valid candidate holds on every model and so wins the status objective outright, and the search crowded towards them. PEREDUR's vacuity filter asks a solver the same question per guarantee, so the filter now sends each generation's offspring to `ltlfilt` in one batch, and the gate asks again. The question is now validity modulo I, as described above. A batch that fails drops nothing in the filter, so the search never depends on machine load.

## Measured runs

Both runs below come from `cargo test --test search -- --nocapture` (debug build, local machine). The test re-checks every reported repair with a direct nuXmv call and a Spot implication check.

| Model | φ | G × P, seed | Repairs | Wall | Queries | Back-end calls | Lasso hits | Implication hits | Memo hits | Spot execs |
|---|---|---|---|---|---|---|---|---|---|---|
| `ack.smv` (in the test) | `G (req -> X ack)` | 30 × 40, 4 | 2 | 2.35 s | 843 | 27 | 429 | 97 | 290 | 102 |
| LMCS-2006 abp4 | `p2false` | 30 × 40, 1 | 2 | 12.5 s | 1121 | 140 | 676 | 199 | 106 | 228 |

The counts include the vacuity flags. On `ack.smv` there are no groups, and the repairs are `req -> F X ack` and `G(req -> F ack)`, the same as before. Neither is vacuous. The flags cost 7 queries and no back-end call.

On abp4 the propositions of `p2false` are DEFINEs appended to `main` (`s_get` for `sender.state = get`, and so on). The binary derives two groups, `{r_ack0, r_ack1}` and `{s_get, s_wait}`. Before the invariant, the gate reported `G(s_get & s_wait -> X(…))`. Now that candidate is dropped as trivial, and the search takes a different path from the same seed. The repairs are as follows.

- `G(s_get -> X F(s_wait & (…)))`, flagged vacuous in `s_get`, in the disjunct `!s_abp & r_ack0` and in the atoms under it. The protocol reaches both acknowledgements infinitely often.
- `G(s_get & r_ack0 -> X((s_abp | !s_get) U (s_wait & (…))))`, flagged vacuous in `s_get` and in every subformula of its consequent. Its antecedent never holds on M, but the encoding does not say so: the sender's state and the channel tag are different variables.

The flags cost 32 queries, no back-end call and 0.23 s. Against the run before the invariant (1097 queries, 132 back-end calls, 12.1 s), the run made 24 more queries and 8 more back-end calls. That difference comes from the changed search path, not from extra processes.

Seed 2 on abp4, with the same budget, ended with no repair after 237 back-end calls and 20.8 s. A run that finds nothing is a normal outcome at this budget.

## Left out of PEREDUR

| PEREDUR piece | Why it is not here |
|---|---|
| Sections, sides, slots, assumptions and guarantees | The individual is one formula, so there is nothing to choose between. |
| `tlsf_add_assumption`, clone-assumption, `tlsf_remove_guarantee` and tombstones | All act on lists of conjuncts. A single property has no list. |
| The FRETISH front end, timings, timing donation, scopes and condition types | FRETISH is a different input language. The input here is plain LTL. |
| Per-section atom pools and initial-condition rules | There are no sections. The pool is φ's atoms. |
| Crossover's target and donor draws | With one slot per parent, each would always draw index 0. |
| `remove_double_negation` after `mutate_formula` | Breeding runs `simplify` on every offspring, which removes it. |
| Elitism (`elitism_rate` 0.1) | The (μ+λ) pool already keeps the best parents. PEREDUR's own notes say both arguments point at 0, and elites would bypass the filters. |
| `selection_rate` (the target size is half the population) | The population stays at P throughout, one offspring per slot. |
| The pad stage and the filter fallback | Under (μ+λ) an empty offspring set leaves the parents, so the run continues without them. |
| `nsga2-truncate` and the weighted-average scheme | Only `nsga2-apportion`, PEREDUR's default, has a caller. |
| Termination by individuals or wall time | Only a generation count is needed now. |
| The repair accumulator and streaming maximality | The gate runs once, on the final population. |
| MUC repair mode | It splits a specification into cores; one formula has none. |
| The atom-renaming cache key | Atoms name model variables, so renaming is unsound (see `CLAUDE.md`). |
| Model-counting similarity | Deferred to the next round; the seam is described above. |
| mt19937 and PEREDUR's draw stream | Each tool must reproduce its own runs, not the other's. |

The rates kept (0.1 crossover, 1.0 mutation, 0.25 monotone, 0.2 temporal, a bloat ratio of 2) are PEREDUR's defaults and are not tuned here. None has been measured on this problem.
