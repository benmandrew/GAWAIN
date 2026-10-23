# BEEM verdicts

The BEnchmarks for Explicit Model checkers (BEEM) suite ships 57 parametrised models in the DiVinE modelling language (DVE), with 300 instances and 76 linear temporal logic (LTL) properties. Some instances set a parameter `ERROR=1` or `ERROR=2`, which plants a bug. This note records the verdicts of every LTL property on those *erroneous* instances and on their `ERROR=0` *correct* twins, as the input for weakening benchmarks. The verdicts are in `bench/results/beem-verdicts.csv`, and the scripts are in `bench/beem/`.

## Corrections to PLAN.md

BEEM does publish verdicts. Every model's `generated_files/results.xml` holds an `<ltl-prob prop-id answer="yes|no|unknown">` per instance and property, where *yes* means the property holds, together with the state count in `<states>`. The plan says there are none.

There are 53 erroneous instances, not 54. Only 38 of them, from 13 models, carry LTL properties, and they give 96 (instance, property) pairs. The 13 models are anderson, bakery, brp, lamport, lamport_nonatomic, leader_election, leader_filters, mcs, peterson, rether, synapse, szymanski and train-gate.

## Route

`bench/beem/fetch.sh` downloads `beem_models_data.tar.gz` from paradise.fi.muni.cz and checks its SHA-256. The archive is not committed.

Each variant goes through a new DVE to Promela translator, `bench/beem/dve.py`, and then SPIN. The shipped Promela (`.pm`) files were rejected for three reasons. First, 15 of the 34 correct twins are not BEEM instances, so they have to be generated from the `.mdve` m4 sources in any case. Second, the shipped `.pm` files wrap DVE rendezvous in `atomic`, which hands control to the receiver and so breaks DVE atomicity. Third, the shipped train-gate `.pm` does not compile (`missing array index for 'e'`), and leader_election and synapse ship none.

The translator writes one `active proctype` whose `do` loop holds one `d_step` per DVE transition and one per sender and receiver pair of a synchronisation. A synchronised step moves the value, then runs the sender's effects, then the receiver's. The m4 macros that the archive lacks (`buffer_channel`, `myloop`, `forloop`, `default`) are rewritten in `bench/beem/beem.m4`. `bench/beem/check_dve.py` expands every instance of the 13 models and compares the parse with the shipped `.dve`, and all 86 match.

Four semantic choices follow DiVinE.

**Range errors.** A step that would write a value outside its type, or index outside an array, is disabled. DiVinE turns such a step into an error state that the LTL check does not explore. Wrapping the byte instead gives anderson.1 352,665 states and turns property 4 true, against BEEM's 347,039 states and *no*.

**Booleans.** `bool` is a byte, as in BEEM's own DVE.

**Bare arrays.** An array name without an index reads element 0, which train-gate needs.

**Deadlocks.** SPIN extends a deadlocked run by stuttering, so a deadlock satisfies `G p` when `p` holds there. A second check per pair uses a Spot never claim for `F dead || φ`, which ignores finite runs. Against BEEM, stuttering disagrees 0 times and ignoring finite runs disagrees 14 times, all where BEEM says *no*. So BEEM stutters as well, and the CSV's `verdict` column uses stuttering. The other semantics is kept in `fin_verdict`.

All initial values are set by a first `d_step`, so every formula φ is checked as `X φ`. No BEEM formula uses `X`, so the shift is exact.

## Checks

`bench/beem/prepare.py` writes one Promela file per variant, with an `ltl` block for φ, for ¬φ and for `G !p` per proposition, and a separate build per Spot never claim. `bench/beem/run.py` compiles `pan` with `-DNOREDUCE -DCOLLAPSE` and runs `pan -a -n` per claim, plus one safety search with `-c0` that counts the states and the deadlocks. `spin -a -o2` keeps write-only variables in the state vector. Without it, SPIN drops them and undercounts leader_filters.2 (22,973 against 29,284), peterson.3, synapse.1 and synapse.5. That cannot change a verdict, because nothing reads them.

Every check ran under `systemd-run` with a memory cap and a timeout. The first pass used a 120 s timeout, 16 jobs of 4 GB on av2 and 8 jobs of 3 GB on av3. It settled 1,058 of the 1,079 checks that ended with a verdict or a state count, counting each check once. A second pass reran the unknowns on av2 with 2 jobs of 16 GB, 3,600 s and a 50 million step depth stack, and a third gave the last three 1 job of 30 GB and 7,200 s on av3. The median check took 0.70 s, the 90th percentile 3.52 s and the longest (peterson.5 property 4) 1,989 s. The finished checks took 2.83 CPU hours in all.

## Results

Of the 96 pairs, the erroneous variant fails φ in 82 and satisfies it in 14. On the correct twins, 64 fail and 32 hold.

**Interesting pairs.** 20 pairs fail on the erroneous variant and hold on the correct one. All 20 survive the degeneracy filter, because ¬φ also fails on the erroneous variant, so neither φ nor ¬φ is a model invariant there. They are anderson.1, .3, .5 and .7 on properties 2 (`G (wait0 -> F cs0)`) and 4 (`GF someoneincs`); lamport.2, .3, .6 and .8, mcs.2, .4 and .6, and peterson.3 and .6 on property 4; rether.4 and .6 on property 5 (`GF nact0`); and synapse.1 on property 3.

**Other pairs.** 62 pairs fail on both variants, 12 hold on both, and 2 (szymanski.2 and .4, property 4) hold only on the erroneous variant. Of the 82 failing pairs, ¬φ is unknown only for anderson.5 property 3, which ran out of 30 GB. That pair fails on both variants, so it does not touch the 20.

**Agreement with BEEM.** 113 rows have both a BEEM answer and a verdict here, and all 113 agree. BEEM answers *unknown* on 40 rows, and all 40 have a verdict here. The state counts agree on 54 of the 56 variants that BEEM counted completely. anderson.1 has 2 states fewer than BEEM and anderson.3 has 12 fewer. Both variants hit range errors, and keeping a partial error state instead gave anderson.1 526 states too many, so the exact DiVinE rule is still unknown. The verdicts on both agree with BEEM.

**Cross-check.** `bench/beem/xcheck.py` runs the shipped `.pm` files with SPIN remote references for the propositions. On 17 instances it agrees on 91 of 92 φ and ¬φ verdicts. The one difference is anderson.1 property 4, where the shipped `.pm` wraps the byte and BEEM agrees with the translator.

## Defects in BEEM

The anderson model's initial list has N+1 values for an array of N; the translator truncates it. In train-gate, the proposition `t1s` is defined as `Train_2@Stop`, and the translator keeps that definition. In synapse.5 (`Lines=1`), the propositions `read10`, `write10` and `write11` name line 1, which does not exist, so properties 2 and 3 there are vacuous; the CSV marks them in `aps_unreachable`.

The 20 surviving pairs have three property shapes. 15 are `GF p`, 4 are the response `G (wait0 -> F cs0)` and 1 is synapse's ordering property, so a benchmark that wants variety must weight them by shape rather than by pair.
