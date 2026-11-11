import Gawain

/-!
Prints the order that the proofs decide, for `bench/check_lean_orders.py` to
compare with Spot. Run with `lake env lean --run Tables.lean`.

Each `mtl` row is a rule and case from `bench/results/operator-orders-mtl.csv`
with the Lean verdict. Each `unary` row is a pair of words over `X`, `F` and
`G` of length at most three, with the verdict of `word_exact`. Each `rules`
row is a formula of the grid below, a direction and a rule set, with every
outcome `outs` lists, as `rule_table` prints them.
-/

open Gawain Gawain.Itv

def fmtI (I : Itv) : String :=
  s!"[{I.lo},{match I.hi with | none => "inf" | some b => toString b}]"

def itvs (n : Nat) (unbounded : Bool) : List Itv :=
  (List.range (n + 1)).flatMap fun a =>
    ((List.range (n + 1)).filter (a ≤ ·)).map (fun b => (⟨a, some b⟩ : Itv)) ++
      (if unbounded then [⟨a, none⟩] else [])

def b01 (b : Bool) : String := if b then "1" else "0"

def mtlRows : List String := Id.run do
  let I := itvs 4 true
  let small := itvs 2 false
  let pairs := I.flatMap fun i => I.map fun j => (i, j)
  let mut out : Array String := #[]
  let row (r l : String) (b : Bool) : String := s!"mtl\t{r}\t{l}\t{b01 b}"
  for (i, j) in pairs do
    let l := s!"{fmtI i} {fmtI j}"
    let lr := s!"{fmtI j} {fmtI i}"
    out := out.push (row "F_I p => F_J p iff I sub J" l (subB i j))
    out := out.push (row "G_I p => G_J p iff J sub I" l (subB j i))
    out := out.push (row "G_I p => F_J p iff I meets J" l (meetsB i j))
    out := out.push (row "F_I p => G_J p iff I = J = [k,k]" l (decide (i = j) && pointB i))
    out := out.push (row "p Ui_I q => p Ui_J q iff I sub J" l (subB i j))
    out := out.push (row "p Ri_J q => p Ri_I q iff I sub J" lr (subB i j))
    out := out.push (row "p Uc_I q => p Uc_J q iff a_I = a_J and I sub J" l
      (decide (i.lo = j.lo) && subB i j))
    out := out.push (row "p Ui_I q => F_J q iff I sub J" l (subB i j))
    out := out.push (row "G_J q => p Ri_I q iff I sub J" lr (subB i j))
  for i in I do
    out := out.push (row "p Ui_I q => p Uc_I q, always" (fmtI i) true)
    out := out.push (row "p Uc_I q => p Ui_I q iff a = 0" (fmtI i) (decide (i.lo = 0)))
  for op in ["F", "G"] do
    for i in small do
      for j in small do
        let r := s!"{op}_I {op}_J p = {op}_(I+J) p"
        out := out.push (row r s!"{fmtI i} {fmtI j} ->" true)
        out := out.push (row r s!"{fmtI i} {fmtI j} <-" true)
  for i in small do
    for j in small do
      for k in small do
        for l in small do
          let get (x : Itv) := x.hi.getD 0
          let v := decide (l.lo + get k ≤ i.lo + get j ∧ get i + j.lo ≤ get l + k.lo)
          out := out.push (row "F_I G_J p => G_K F_L p iff I - K sub L - J"
            s!"{fmtI i}{fmtI j} {fmtI k}{fmtI l}" v)
  return out.toList

def words : List (List Op) :=
  let ops := [Op.X, Op.F, Op.G]
  let w1 := ops.map ([·])
  let w2 := ops.flatMap fun a => w1.map (a :: ·)
  let w3 := ops.flatMap fun a => w2.map (a :: ·)
  [[]] ++ w1 ++ w2 ++ w3

def showW (w : List Op) : String :=
  String.join (w.map fun | .X => "X" | .F => "F" | .G => "G") ++ "p"

def unaryRows : List String :=
  words.flatMap fun u => words.map fun v =>
    s!"unary\t{showW u}\t{showW v}\t{b01 (nfLe (norm u) (norm v))}"

def Gawain.Fm.sexp : Fm → String
  | .tt => "tt"
  | .ff => "ff"
  | .atom n => match n with | 0 => "a" | 1 => "b" | 2 => "c" | n => s!"x{n}"
  | .not a => s!"(not {a.sexp})"
  | .X a => s!"(X {a.sexp})"
  | .F a => s!"(F {a.sexp})"
  | .G a => s!"(G {a.sexp})"
  | .and a b => s!"(and {a.sexp} {b.sexp})"
  | .or a b => s!"(or {a.sexp} {b.sexp})"
  | .imp a b => s!"(imp {a.sexp} {b.sexp})"
  | .iff a b => s!"(iff {a.sexp} {b.sexp})"
  | .U a b => s!"(U {a.sexp} {b.sexp})"
  | .W a b => s!"(W {a.sexp} {b.sexp})"
  | .R a b => s!"(R {a.sexp} {b.sexp})"

def unaries : List (Fm → Fm) := [.not, .X, .F, .G]
def binaries : List (Fm → Fm → Fm) := [.and, .or, .imp, .iff, .U, .W, .R]

def apply1 (xs : List Fm) : List Fm :=
  unaries.flatMap (fun u => xs.map u) ++
    binaries.flatMap (fun b => xs.flatMap fun x => xs.map (b x))

/-- The constants, then every formula of depth one or two over `a` and `b`. -/
def grid : List Fm :=
  let leaves := [Fm.atom 0, .atom 1]
  let d1 := leaves ++ apply1 leaves
  [.tt, .ff] ++ d1 ++ apply1 d1

def ruleRows : List String :=
  grid.flatMap fun a =>
    [(true, "w"), (false, "s")].flatMap fun (want, d) =>
      [(false, "peredur"), (true, "ordered")].map fun (o, name) =>
        let os := (outs o want (.atom 2) a (some true)).map Fm.sexp
        s!"rules\t{a.sexp}\t{d}\t{name}\t{" ; ".intercalate os}"

def main : IO Unit := do
  for r in mtlRows ++ unaryRows ++ ruleRows do IO.println r
