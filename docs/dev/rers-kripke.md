# RERS models as explicit Kripke structures

The module `src/rers.rs` reads a RERS parallel-track Promela file and explores its reachable states into a `Kripke` value from `src/kripke.rs`. This is the model side of tier 3a in `docs/dev/oracle-performance.md`. The automaton for ¬φ', the product and the emptiness check live in `src/oracle/explicit.rs`.

## Input shapes

The 2016, 2017 and 2019 files share one shape. An `Environment` process loops over `c ! m` offers on *rendezvous* channels (capacity 0). A `Listener` receives on `act` into `lastAction`. Each component has one local `int state` and a `do` loop of `:: state == k -> atomic { if ... fi }` blocks. A branch is either a receive `c ? m ->` followed by `act ! m;` and `state = j;`, or a send `c ! m ->` followed only by `state = j;`. Receives come from the environment or, on channels named `pI_J`, from another component. Every component's initial state has a single branch, a receive of `nop` that sends nothing on `act`. The atoms are `#define p_X (lastAction == X)`. The parser accepts exactly this, and an `act ! m;` on any branch, and refuses every other line with its line number.

The 2018 files use a different shape. They declare `int state_0 = 16;` and so on as *global* variables, one per component in the order of the processes, and each component's blocks test its own variable. A branch is a receive with its body on the same line, `c ? m -> {state_2 = 40; p2_5 ! m}`, or a *guarded send* over one or more lines, `((state_2==60 || ...) && state_5==93) -> {state_0 = 10; p0_2 ! m}`. A body assigns the component's variable and then sends on `act` or on another component's channel, or, for the `nop` receive, only assigns. A guard is a conjunction of disjunctions of `state_I==N`, and the parser refuses any other guard.

## Semantics

A state is the local state of each component plus the last action. A step starts in one component, either with a receive from an environment offer or with a send whose guard holds. A send hands its message to a matching receive in another component, and in 2018 that receive may send in turn, so one step can run along a *chain* of components. In 2018 problem 101, `a1_SIGTERM` passes from component 0 through 2, 5 and 6 before component 7 records it on `act`. The step sets the last action to the message sent on `act`, if any, and otherwise leaves it. Atom `p_X` holds exactly where the last action is `X`, so at most one atom holds at each state. A state with no step gets a self-loop, as SPIN's stutter extension does.

A 2018 send follows an assignment, so in SPIN it would block with the assignment made if no component could receive it. The guard names the states of every component on the chain, which is meant to rule this out, and the explorer fails with the component and message if a chain ever finds no receiver. Problem 101 explores without such a failure. A 2016 send begins its branch, so it is only enabled when some component can receive it.

SPIN can still interleave inside a 2018 chain. A component that has just finished a step sits at the head of its `do` loop until it takes its guard `state_K == k`, and a send that reaches it in that window blocks. Other processes then run while the sender's assignment is visible. The explorer takes the whole chain as one step, as the Petri net of the same problem does, where the synchronisation is one transition. The `.dot` file of each 2018 problem gives the same components as labelled transition systems that synchronise on shared actions. `bench/rers_dot_count.py` explores that composition independently, and on problem 101 it finds the same 527,077 states and 2,341,935 transitions.

Every component starts by receiving `nop`, which records nothing. The explorer takes these steps before it starts, so the initial state is the one after all of them. Each `nop` step is always enabled, is the only branch of its state, and touches nothing another step reads, so it commutes with every other step. Moving it to the front of a run therefore gives a run with the same atoms up to stutter. A component never blocks while it waits for `nop`, so dropping that state also adds no deadlock.

The explorer does not model the Listener, the `atomic` blocks or the hand-over inside a rendezvous. In SPIN these add intermediate states where `lastAction` has not changed yet, and the next observable state follows one or two steps later. An LTL formula without `X` is *stutter-invariant*: it cannot tell a run from one with repeated states removed. The explicit structure therefore satisfies exactly the `X`-free formulas that the SPIN model satisfies. Every step of the structure records one action, so its positions are actions, and `X` reads the next action, as the official solutions read it. SPIN's `X` counts the hidden steps instead.

## Agreement with the official solutions

On 2016 problems 101 to 104 all 80 explicit verdicts match the official ones. On 2018 problem 101, 19 of 20 match the dot/Petri solutions, which number the file's `ltl` lines 1 to 20 (`tests/explicit.rs` checks both). Property 10 fails on the structure and is true officially. Every counterexample to it ends in one of the 12 reachable deadlocks, where the self-loop repeats the last action forever. With an extra atom on the deadlocked states, `F dead | φ10` holds, and the same change makes all 20 verdicts match. The official answers therefore count only infinite runs, while the structure keeps the deadlocking runs, as SPIN does. The degeneracy filter is unaffected on problem 101: each ¬φ fails on an infinite run under both readings.

## Storage

Each component's local states are numbered densely, and a global state packs every local number into its own bit field of a `u128`, with the last action in the top field. A hash map with a multiplicative hash takes a packed state to its number, and a vector of packed states doubles as the breadth-first queue. Successors are stored as one flat `u32` array with one offset per state, sorted and with duplicates removed. Labels are one `u32` atom index per state.

## Measured sizes and times

The counts for 2016 problems 101 to 106 and 2017 problems 101 and 102 match those of the Python explorer in `docs/dev/oracle-performance.md` exactly, and `tests/rers.rs` checks 101 to 104. The times below are for `/usr/bin/time target/release/examples/rers_explore FILE.pml` on av2, under `systemd-run --user --scope -p MemoryMax=100G -p MemorySwapMax=0 nice -n 10 timeout 1800`.

| Problem | States | Transitions | Time | Peak RSS | Python time |
|---|---|---|---|---|---|
| 2016/105 | 499,041 | 2,295,530 | 0.13 s | 78 MB | 6.6 s |
| 2016/106 | 649,697 | 3,505,673 | 0.20 s | 76 MB | 11.1 s |
| 2016/107 | 64,021,489 | 493,427,361 | 53.6 s | 9.8 GB | did not finish in 4 GB and 300 s |
| 2017/102 | 1,163,558 | 5,780,665 | 0.39 s | 150 MB | 9.9 s |

On the local machine, 105, 106 and 2017/102 took 0.26 s, 0.51 s and 1.03 s, and 107 did not finish inside 60 s. Problem 107 costs about 150 bytes per state, about 31 of them in the successor array and most of the rest in the hash map and the queue of packed states. At that rate a 125 GB machine holds about 8 × 10^8 states, so how far past 107 the explicit route reaches is a question for 2016 problems 108 to 112, which are not yet measured.

The 2018 problems are larger per component. Problem 101 has 9 components, 527,077 states and 2,341,935 transitions, and explores locally in 0.39 s with a peak RSS of 75 MB. Problem 102 has 20 components and did not finish on av2. Under `MemoryMax=60G` it was killed at the cap after about 20 minutes, and under `MemoryMax=85G` with `timeout 3000` it was still exploring at 46 GB after 32 minutes when the timeout stopped it. Problem 103 has 34 components and needs about 161 bits, so the 128-bit key refuses it. `bench/results/rers-2018-explicit.csv` holds the explicit check of the 16 false 2018 properties whose ¬φ SPIN could not decide in 120 s, from `target/release/examples/rers_classify FILE.pml 120 N...`. The 4 on problem 101 survive, since each ¬φ fails, and the 12 on 102 and 103 stay unknown.
