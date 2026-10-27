# Case studies: CEGIW and abp4

The qualitative part of the evaluation ("Qualitative" in `docs/dev/evaluation.md`) shows what repairs look like to a reader. This first part covers the nine instances of `bench/data/cegiw`. They are the eight case studies of *counterexample-guided interval weakening* (CEGIW) and abp4 `p2false`. Each section sets the repairs of GAWAIN (Genetic Algorithm for Weakening Against Implementations under NSGA-II) beside CEGIW's interval. The instances, CEGIW's method and an earlier comparison at the ablation configuration are in "CEGIW: the common subset" and the two sections after it in `bench/notes/baselines.md`. This note does not repeat them.

## Method

The rows come from the headline run at commit `ec8a3f9`, 10 seeds per arm. `forced-default` is the evaluation's genetic algorithm (GA) configuration, with the initial-state filter on, and it also searches the three degenerate instances. `forced-mutation` is the random-mutation baseline under the same budget and filters ("Random-mutation baseline" in `docs/dev/search.md`). The run and the analysis are the following commands, the second once per arm.

```
python3 bench/graded_run.py bench/data/cegiw 3 --seeds 1-10 --arms forced-default,forced-mutation --out runs-eval
python3 bench/baselines_cegiw.py bench/data/cegiw bench/data/cegiw/eval-<arm>.csv --runs runs-eval --arm <arm>
```

Each arm's repairs are pooled over the seeds and reduced to the *maximal set*, the repairs that no other pooled repair strictly implies. Spot then places CEGIW's formula against each member, modulo the *encoding invariant* I. Only abp4 has a non-trivial I, `G(!(s_get & s_wait) & !(r_ack0 & r_ack1))`. The full nuXmv check of CEGIW's output is in the `cegiw_holds` column. The rows are in `bench/data/cegiw/eval-forced-default.csv` and `eval-forced-mutation.csv`.

| Case | Class | Found, GA | Found, mutation | Maximal, GA | Maximal, mutation |
|---|---|---|---|---|---|
| forage-response | degenerate | 0/10 | 3/10 | 0 | 3 |
| forage-absence | proper | 10/10 | 7/10 | 1 | 2 |
| forage-limit-response | degenerate | 10/10 | 10/10 | 1 | 1 |
| forage-limit-absence | proper | 10/10 | 7/10 | 1 | 2 |
| minimal-response | proper | 6/10 | 3/10 | 2 | 1 |
| minimal-absence | proper | 5/10 | 2/10 | 9 | 3 |
| minimal-limit-response | proper | 10/10 | 10/10 | 2 | 2 |
| minimal-limit-absence | degenerate | 5/10 | 2/10 | 23 | 3 |
| abp4 `p2false` | proper | 10/10 | 9/10 | 6 | 5 |
| Total | | 66/90 | 53/90 | | |

The GA finds a repair on 13 more runs than random mutation. It finds fewer only on forage-response.

## Foraging absence

φ on both foraging absence cases is `G(resting_p & X !resting_p -> X(!resting_p & …))`, with 15 nested X. It forbids a return to `resting` for 15 steps after the robot leaves it. CEGIW narrows the interval from [1,15] to [1,3] and gives `G((resting_p & X !resting_p) -> X(!resting_p & X(!resting_p & X !resting_p)))`, which holds on both models.

The GA's single maximal repair is equivalent to CEGIW's on both cases. On forage-limit-absence it is the same formula, `G(resting_p & X !resting_p -> X(!resting_p & X(!resting_p & X !resting_p)))`. On forage-absence it is longer: `G((resting_p & X !resting_p -> X(!resting_p & X(!resting_p & X(!resting_p & (τ | !resting_p))))) | !resting_p)`, where τ is a tail of 12 nested X. Both disjunctions collapse. `!resting_p & (τ | !resting_p)` is `!resting_p`, and the outer `| !resting_p` already makes the implication true. What remains is the [1,3] absence again.

Random mutation misses this form on both cases. Its two maximal repairs are `G(resting_p & X !resting_p -> X(!resting_p & X(!resting_p & F(!resting_p & X(!resting_p & X !resting_p)))))`, incomparable with CEGIW's, and `G(resting_p & X !resting_p -> X(!resting_p & X(!resting_p & X !resting_p | resting_p)))`, which CEGIW's implies.

## Foraging response with a limit

φ is `G(resting_p -> X(resting_p | X(resting_p | X resting_p)))`, a return to `resting` within 3 steps. The model violates its negation as well, so the case is degenerate. CEGIW widens the interval to [1,20] and gives `G(resting_p -> F[1,20] resting_p)`.

The GA's single maximal repair is `G((!resting_p -> resting_p) -> X F(resting_p | X resting_p))`. The antecedent `!resting_p -> resting_p` is `resting_p`, and `F(resting_p | X resting_p)` is `F resting_p`. The repair is therefore the eventual response `G(resting_p -> X F resting_p)`, with no bound. CEGIW's bounded response is strictly stronger. Both hold on the model. They also score the same against φ, 0.954634 to six places, so the model count does not separate a bound of 20 from no bound. Random mutation's repair, `!resting_p W G(resting_p W X(resting_p | F X resting_p))`, is also strictly weaker than CEGIW's.

## abp4 `p2false`

Write ψ for `s_wait & (!s_abp & r_ack0 | s_abp & r_ack1)`, a wait with the matching acknowledgement. φ is `G(s_get -> X(!s_get U ψ))`: after a `get`, no further `get` happens until ψ. CEGIW widens the `X` to `F[1,5]` and stops when *bounded model checking* (BMC) finds no counterexample. Its formula fails the full nuXmv check (`cegiw_holds` is false), as "CEGIW: results" in `bench/notes/baselines.md` found with a lasso of 48 states.

All six of the GA's maximal repairs hold, and each changes one part of φ.

- **Trigger.** `G(!s_abp & s_get -> X(!s_get U (s_wait <-> !s_abp & r_ack0 | s_abp & r_ack1)))` restricts the trigger to `get` with bit 0, and also turns the target's conjunction into an equivalence. `G((r_ack0 | r_ack1) & s_get -> X(!s_get U ψ))` restricts it to a `get` with an acknowledgement present.
- **Until.** `G(s_get -> X(!s_get W ψ | !r_ack0))` replaces `U` with the weak until `W`, so ψ need not arrive, and adds the disjunct `!r_ack0` at the next state.
- **Guard.** Three repairs relax the `!s_get` guard to `!(r_ack1 & s_get)`, `!(s_get & s_wait)` and `!s_get | !r_ack0`. The first also repeats `s_wait` inside the target, which is redundant.

CEGIW's formula is incomparable with five of these. It is strictly stronger than one, `G(s_get -> X(!(s_get & s_wait) U ψ))`. Under I the guard `!(s_get & s_wait)` is always true, so modulo I this repair is `G(s_get -> X F ψ)`. That is equivalent to `G(s_get -> X F(!s_get U ψ))`, the unbounded formula that the earlier note found to be the only interval weakening that holds. Without I the two are incomparable.

Random mutation finds 5 maximal repairs on 9 of 10 seeds, of the same shapes. Examples are `G(s_get -> X((r_ack0 -> !s_get) U ψ))`, a relaxed guard, and `G(s_get -> X(!s_get U (s_wait -> !s_abp & r_ack0 | s_abp & r_ack1)))`, a relaxed target. CEGIW's formula is strictly stronger than the second modulo I, and incomparable with the other four.

## Two-state absence

φ on minimal-absence and minimal-limit-absence is the 15-step absence over `a_p`. CEGIW narrows it to [1,1] and gives `G((a_p & X !a_p) -> X !a_p)`. The antecedent already states the consequent, so the formula is valid and equivalent to ⊤ (`cegiw_valid` is true). It scores ⊤'s 0.621732.

Every GAWAIN maximal repair is strictly stronger than CEGIW's, 9 on minimal-absence and 23 on minimal-limit-absence. They are not readable. One of the shorter ones is `G !(a_p & X(a_p & X X(!a_p & X(a_p & X(a_p -> !a_p)))))`, which forbids one fixed pattern of `a_p` over six steps. Most are longer, with nested X chains, `U`, `R` and `W` mixed under `F` and `G`. Many carry unsimplified subterms such as `a_p -> !a_p` and `!(!a_p -> a_p)`, both of which are `!a_p`. The strength they add is small. The best scores are 0.623659 and 0.629640, against ⊤'s 0.621732. Random mutation's 3 maximal repairs on each case are as long, and are the same three formulas on both. On this model GAWAIN beats a valid formula with repairs that a reader cannot check by eye. That is a limitation of the output, and the hand classification in the second part will have to count it.

## Response without a CEGIW weakening

On minimal-response and forage-response CEGIW finds no interval that holds. On minimal-response φ is `G(a_p -> X(a_p | X a_p))`. The GA finds a repair on 6 of 10 seeds, and its two maximal repairs are readable.

- `G(a_p -> X(!a_p | G(a_p | X a_p)))`: after `a_p`, either the next state lacks `a_p`, or from then on `a_p` never misses two steps in a row.
- `G !(!a_p & X !a_p) | X(!a_p | G !a_p)`: `!a_p | G !a_p` is `!a_p`, so this is `G !(!a_p & X !a_p) | X !a_p`. Either `a_p` never misses two steps in a row, or the second state lacks it.

Random mutation finds `X(!a_p | G X(a_p | X a_p))` on 3 of 10 seeds.

On forage-response the GA finds nothing on any seed. The earlier binary run found a repair on 7 of 10 seeds, but the initial-state check marked 7 of its 8 repairs ("CEGIW: results" in `bench/notes/baselines.md`). The headline configuration filters at the initial state, so 0 of 10 is the expected result. Random mutation finds 3 maximal repairs on 3 seeds, such as `G(resting_p -> X(resting_p | X(!resting_p | X !resting_p U X resting_p)))`.

## Two-state response with a limit

On minimal-limit-response CEGIW widens [1,2] to [1,11] and gives `G(a_p -> F[1,11] a_p)`. The GA's two maximal repairs are both short. `G(a_p -> X(a_p | X a_p)) | X !a_p` is φ disjoined with a condition on the second state. `!a_p W G(a_p | F X a_p)` simplifies, since `a_p | F X a_p` is `F a_p`, to `!a_p W G F a_p`. Either `a_p` never holds, or it holds infinitely often.

CEGIW's formula is strictly stronger than `!a_p W G F a_p`, since a bounded return implies infinitely many returns. It is incomparable with φ disjoined with `X !a_p`. CEGIW's score is 0.875320 and the GA's best 0.875233. Random mutation finds `!a_p W G(!F X a_p -> X a_p)` and `X(!a_p | G X(a_p | X a_p))`. The first is again `!a_p W G F a_p`, since `!F X a_p -> X a_p` is `X F a_p`.

## Still to come

The second part is a hand classification of repair shapes on a sample of `bench/data/eval`, once the headline run there finishes. On these nine instances the repairs read well where φ is short, and stop reading well once the search reaches into long X chains.
