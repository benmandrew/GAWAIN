import Gawain.Rules

/-!
# The model-checking facts behind gated search

A model is a set of traces, each a valuation of the atoms, and `M ⊨ φ` holds
when every trace satisfies `φ` at position 0. A *repair* of `φ` is a formula
that `φ` implies and that `M` satisfies, which is the output gate of
`docs/dev/search.md` less maximality.

`fails_down` and `cex_down` say that strengthening a failing candidate keeps it
failing, and that the failing candidate's counterexample refutes the
strengthened one too, so no back-end call is needed. `holds_up` is the dual
for weakening a holder. `and_repair` gives the join crossover: the conjunction
of two repairs is a repair. `or_repair` says that `φ ∨ ψ` repairs `φ` whenever
`M ⊨ ψ`.
-/

namespace Gawain

/-- A model: the traces it allows. -/
abbrev Model := (Nat → Pred) → Prop

def Sat (M : Model) (a : Fm) : Prop := ∀ ρ, M ρ → a.eval ρ 0

def Repair (M : Model) (φ ψ : Fm) : Prop := φ.Le ψ ∧ Sat M ψ

theorem holds_up {M : Model} {a b : Fm} (h : Sat M a) (hab : a.Le b) : Sat M b :=
  fun ρ hρ => hab ρ 0 (h ρ hρ)

theorem cex_down {ρ : Nat → Pred} {a b : Fm} (hc : ¬ a.eval ρ 0) (hba : b.Le a) :
    ¬ b.eval ρ 0 :=
  fun hb => hc (hba ρ 0 hb)

theorem fails_down {M : Model} {a b : Fm} (h : ¬ Sat M a) (hba : b.Le a) : ¬ Sat M b :=
  fun hb => h (holds_up hb hba)

theorem and_repair {M : Model} {φ a b : Fm} (ha : Repair M φ a) (hb : Repair M φ b) :
    Repair M φ (.and a b) :=
  ⟨fun ρ t h => ⟨ha.1 ρ t h, hb.1 ρ t h⟩, fun ρ hρ => ⟨ha.2 ρ hρ, hb.2 ρ hρ⟩⟩

theorem or_repair {M : Model} {φ ψ : Fm} (h : Sat M ψ) : Repair M φ (.or φ ψ) :=
  ⟨Fm.le_orL, fun ρ hρ => .inr (h ρ hρ)⟩

end Gawain
