# Case studies and repair shapes

The qualitative part of the evaluation ("Qualitative" in `docs/dev/evaluation.md`) shows what repairs look like to a reader. The first part covers the nine instances of `bench/data/cegiw`. They are the eight case studies of *counterexample-guided interval weakening* (CEGIW) and abp4 `p2false`. Each section sets the repairs of GAWAIN (Genetic Algorithm for Weakening Against Implementations under NSGA-II) beside CEGIW's interval. The instances, CEGIW's method and an earlier comparison at the ablation configuration are in "CEGIW: the common subset" and the two sections after it in `bench/notes/baselines.md`. This note does not repeat them.

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


# Repair shapes on the headline set

The second part classifies by hand the repairs of the headline run on `bench/data/eval` ("Results" in `docs/dev/evaluation.md`). Of the 100 model instances, the `default` arm finds a repair on at least one seed on 89: 55 RERS, 19 MCC, 14 BEEM and abp4 `p2false`. The 11 others are all RERS. The labels are in `bench/results/eval-shapes.csv`, one row per instance.

## Method

Each instance contributes one repair, the one a user reads first. The repairs of the 10 seeds are pooled, and the one with the highest semantic score is kept, with ties going to the shorter formula. The classification reads its `simplified` field, which is Spot's form when that is smaller. φ is also read in Spot's form (`ltlfilt -r`), since many MCC formulas collapse under it. MCC's Diffusion2D φ is equivalent to `F a3`, for example, and SieveSingleMsgMbox φ to `F a0`.

A repair gets one *shape*, the edit that takes φ to it.

- **Local.** One or two proper subformulas of φ are replaced by weaker ones, and the rest stays as it was.
- **Prefix.** An invariant or recurrence is cut down to one point or a few steps near the start.
- **Rewritten.** The repair shares little syntax with φ, and the relation needs Spot to check.
- **Disjunct.** The repair is equivalent to φ ∨ ψ for some ψ.
- **Eventual.** An outer `G` becomes `F G` or `G F`, or the whole of φ goes under an `F`.
- **Conjunct.** A conjunct of φ is dropped.
- **Trigger.** The antecedent of a response or invariant gains a conjunct.

A repair is *readable* when one sentence can say what it allows beyond φ, without running Spot. Where a label rests on an equivalence, `ltlfilt --equivalent-to` confirmed it. Nine equivalences were checked, all true. An example is `X(G F !a1 U !a1)` on TCPcondis, which is `G F !a1 | X !a1`, φ disjoined with a condition on the second state.

The closed share of a repair is (s − ⊤)/(1 − ⊤), with s its semantic score and ⊤ the score of the valid formula from `bench/results/eval-top-default.csv`. On 7 instances ⊤ scores 1.000000, so the share is undefined, and the medians leave them out.

| Shape | Repairs | RERS | MCC | BEEM | abp4 | Readable | Median closed share | Median size |
|---|---|---|---|---|---|---|---|---|
| Local | 38 | 32 | 5 | 1 | | 27 | 0.677 | 13 |
| Prefix | 15 | 2 | | 13 | | 14 | 0.001 | 4 |
| Rewritten | 11 | 6 | 5 | | | 1 | 0.390 | 15 |
| Disjunct | 10 | 4 | 6 | | | 9 | 0.863 | 12.5 |
| Eventual | 7 | 7 | | | | 4 | 0.424 | 7 |
| Conjunct | 6 | 4 | 2 | | | 4 | 0.589 | 9.5 |
| Trigger | 2 | | 1 | | 1 | 2 | 0.452 | 14.5 |
| Total | 89 | 55 | 19 | 14 | 1 | 61 | | |

Size is Spot's node count (`%s`). Readable repairs have a median of 8 nodes and unreadable ones 13.5.

## Local edits

Local edits are the commonest shape, at 38 of 89, and 32 of them are on RERS. They fall into a few patterns.

- **Disjunct in a subformula** (13). On beem-synapse.1-3, `G !p_write10` inside the response becomes `G(!p_write10 | !p_read10)`. On rers2016-103-p11, `F G !p` becomes `F G(!p | X !p)`, which bans only two `p` in a row from some point on.
- **`G` to `G F`** (6). All six are RERS 2016 properties with an inner absence `G !p`, which becomes `G F !p` or `!p & G F !p`. On rers2016-105-p10, the release target ends in `G F !p_c2_t5` in place of `G !p_c2_t5`.
- **Trigger into the until target** (5). φ is `G(!a | !b U c)`, and the repair is `G(!b U (!a | c))`. The trigger's absence can now end the wait, as on rers2016-102-p14, `G(!p_c0_t6 U (!p_c1_t3 | p_c1_t8))`.
- **Release condition widened** (5). In `x R y` the condition x gains a disjunct, so y may stop sooner. rers2016-103-p8 adds `p_c0_t5__c2_t3` to `(p_c0_t6 | p_c0_t8)`.
- **Operator to its first step** (4). An inner `G` or `R` is replaced by what it requires now. On rers2016-106-p12, `!p_c5_t11 | G !p_c3_t9` becomes `!p_c3_t9 | !p_c5_t11` under the outer `G`.
- **Other** (5). There are two `U` to `W`, two weakened invariants and one weakened `W`. rers2018-101-p16 is the plain `U` to `W`, `G(!p_a20_SIGTTOU | !p_a23_SIGTTIN W p_a8_SIGTTIN)`.

These are the edits a person would try first. The 11 unreadable local repairs are on long φ, where two edits interact, or on MCC, where φ's own structure is arbitrary.

## Prefix repairs on BEEM

13 of the 14 BEEM repairs are prefixes. Eleven φ there are `G F p`, and the repair keeps a single occurrence of p: `X X F p` on anderson.1-4, mcs, peterson and rether, and `X(p -> X F p)` on lamport. On anderson.1-2 the response `G(p_wait0 -> F p_cs0)` becomes `X(F p_cs0 | !p_wait0)`, which checks the response at step 1 only. The two RERS prefixes turn `G !p_c0_t2` into `!p_c0_t2 & X !p_c0_t2`.

14 of the 15 prefix repairs are readable, and almost none says anything. Their median closed share is 0.001, and 4 of the 15 score ⊤'s value exactly. That accounts for the BEEM result in the headline run, where 100 of 111 runs with a repair score ⊤'s value. The repairs are correct, but on these models the count barely separates them from ⊤. The rejection strength separates them better. In the `closed_rejection` column of `bench/results/eval-top-default.csv`, the median over the 15 prefix instances is 0.311, against 0.987 over the other 74, and the 10 lowest in the set are all BEEM prefixes.

## Disjunct, eventual, conjunct and trigger

The disjunct repairs keep φ whole. Their median closed share is the highest of any shape, at 0.863, and 9 of 10 read well. On rers2016-106-p11 the repair is φ `| G !p_c3_t4`, and on rers2018-101-p5 it is φ `| X !p_a8_SIGTTIN`. Six of the ten are on MCC.

The eventual repairs move φ to the limit. On rers2016-102-p8 and rers2018-101-p18 the response `G(!a | F b)` becomes the recurrence `G F(!a | b)`. On rers2018-101-p4 it becomes `F G(F p_a5_SIGPROF | !p_a9_SIGLOST)`, so the response must hold only from some point on. On rers2016-104-p6 the whole release goes under an `F`.

The conjunct repairs drop one obligation. On MedleyA φ is `F a0 & X X X X F a1`, and the repair is `F a0`. Two of the four RERS cases also move the trigger of the remaining conjunct into its until target, as in the local pattern above. The two trigger repairs are abp4's, discussed in the first part, and Peterson-PT-2's `X(a0 | a1 -> G !a1)` against φ's `X G !a1`.

## Unreadable repairs

28 of the 89 repairs are unreadable, 11 of them on MCC and 16 on RERS. 10 of the 11 rewritten repairs are among them. rers2018-101-p2 is typical. φ is `G(!p_a20_SIGTTOU | X(p_a8_SIGTTIN W p_a20_SIGTTOU))`, and the repair is `G(!p_a8_SIGTTIN & !p_a20_SIGTTOU | X(!p_a20_SIGTTOU | G(p_a8_SIGTTIN | p_a20_SIGTTOU)))`. It holds and it is weaker than φ, but no one sentence says what it adds. On MCC the cause is often φ itself. The MCC formulas are random, and on CO4 and ShieldPPPt φ is no easier to read than the repair.

Rewritten repairs score lower than local edits, with a median closed share of 0.390 against 0.677. Unreadable repairs as a whole score higher, though. Their median is 0.619 over 25 instances, against 0.493 over the 57 readable ones, or 0.562 with the prefixes left out. Some of the search's strength therefore comes from formulas that a reader cannot check by eye, as on the two-state absence cases of the first part.

## Random mutation

The `mutation` arm's best repair is the same formula on 26 of the 89 instances, and it finds no repair on 9. Its best repairs sit beside the GA's in the `mutation_repair` column of `bench/results/eval-shapes.csv`, but they are not classified.

Over the whole set, 61 of 89 repairs read well and 38 of them are local edits, disjuncts or trigger restrictions that keep most of φ. The BEEM prefixes are the clearest weakness: readable, correct and nearly as weak as ⊤.
