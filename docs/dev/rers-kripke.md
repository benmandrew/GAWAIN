# RERS models as explicit Kripke structures

The module `src/rers.rs` reads a RERS parallel-track Promela file and explores its reachable states into a `Kripke` value from `src/kripke.rs`. This is the model side of tier 3a in `docs/dev/oracle-performance.md`. The automaton for ¬φ', the product and the emptiness check live in `src/oracle/explicit.rs`.

## Input shapes

The 2016, 2017 and 2019 files share one shape. An `Environment` process loops over `c ! m` offers on *rendezvous* channels (capacity 0). A `Listener` receives on `act` into `lastAction`. Each component has one local `int state` and a `do` loop of `:: state == k -> atomic { if ... fi }` blocks. A branch is either a receive `c ? m ->` followed by `act ! m;` and `state = j;`, or a send `c ! m ->` followed only by `state = j;`. Receives come from the environment or, on channels named `pI_J`, from another component. Every component's initial state has a single branch, a receive of `nop` that sends nothing on `act`. The atoms are `#define p_X (lastAction == X)`. The parser accepts exactly this, and an `act ! m;` on any branch, and refuses every other line with its line number.

The 2018 files use a different shape. They declare `int state_0 = 16;` and so on as global variables, guard each branch on the states of other components (`(state_2==60 || ...) && state_5==93 -> ...`), and write a send as `{state_0 = 10; p0_2 ! a1_SIGTERM}`. The parser refuses them at the first global declaration. Supporting them needs guards over the global state, which no caller needs yet.

## Semantics

A state is the local state of each component plus the last action. A step is either a component's receive from an environment offer, or a send by one component paired with a matching receive by another. The step sets the last action to the message it sends on `act`, if any, and otherwise leaves it. Atom `p_X` holds exactly where the last action is `X`, so at most one atom holds at each state. A state with no step gets a self-loop, as SPIN's stutter extension does.

Every component starts by receiving `nop`, which records nothing. The explorer takes these steps before it starts, so the initial state is the one after all of them. Each `nop` step is always enabled, is the only branch of its state, and touches nothing another step reads, so it commutes with every other step. Moving it to the front of a run therefore gives a run with the same atoms up to stutter. A component never blocks while it waits for `nop`, so dropping that state also adds no deadlock.

The explorer does not model the Listener, the `atomic` blocks or the hand-over inside a rendezvous. In SPIN these add intermediate states where `lastAction` has not changed yet, and the next observable state follows one or two steps later. An LTL formula without `X` is *stutter-invariant*: it cannot tell a run from one with repeated states removed. The explicit structure therefore satisfies exactly the `X`-free formulas that the SPIN model satisfies, and the RERS properties contain no `X`. A formula with `X` would count the hidden steps, so the structure is unsound for it.

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
