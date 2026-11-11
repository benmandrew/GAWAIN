import Gawain.Trace

/-!
# GAWAIN's monotone rewrites are sound

`Fm` mirrors the formula type of `src/ltl/mod.rs`, and `eval` reads it over a
valuation of the atoms. A *context* is a formula with one hole outside any
`<->`, which matches `collect_polarities` in `src/search/operators.rs`: under
`!` and in the antecedent of `->` the polarity flips, and nothing under `<->`
is a site. `ctx_mono` shows that a weaker formula in the hole gives a weaker
whole at a positive hole and a stronger one at a negative hole.

`rulesAt` transcribes `rules_at` and `apply_rule`, with every coin outcome
listed and the drawn literal an arbitrary formula. `proposedAt` holds the
rules that `docs/dev/order-search.md` proposes. `rulesAt_sound` and
`proposedAt_sound` show that every output moves in the stated direction, and
`rewrite_sound` lifts both through a context as `monotone_rewrite` does: the
local direction is the asked one at a positive site and the dual at a
negative one.

The transcription of `rules_at` is checked by reading only.
-/

namespace Gawain

inductive Fm
  | tt
  | ff
  | atom (n : Nat)
  | not (a : Fm)
  | X (a : Fm)
  | F (a : Fm)
  | G (a : Fm)
  | and (a b : Fm)
  | or (a b : Fm)
  | imp (a b : Fm)
  | iff (a b : Fm)
  | U (a b : Fm)
  | W (a b : Fm)
  | R (a b : Fm)
  deriving DecidableEq, Repr

def Fm.eval (ρ : Nat → Pred) : Fm → Pred
  | .tt => fun _ => True
  | .ff => fun _ => False
  | .atom n => ρ n
  | .not a => neg (a.eval ρ)
  | .X a => Gawain.X (a.eval ρ)
  | .F a => Gawain.F (a.eval ρ)
  | .G a => Gawain.G (a.eval ρ)
  | .and a b => fun t => a.eval ρ t ∧ b.eval ρ t
  | .or a b => fun t => a.eval ρ t ∨ b.eval ρ t
  | .imp a b => fun t => a.eval ρ t → b.eval ρ t
  | .iff a b => fun t => (a.eval ρ t ↔ b.eval ρ t)
  | .U a b => Gawain.U (a.eval ρ) (b.eval ρ)
  | .W a b => Gawain.W (a.eval ρ) (b.eval ρ)
  | .R a b => Gawain.R (a.eval ρ) (b.eval ρ)

/-- `a` implies `b` under every valuation, at every position. -/
def Fm.Le (a b : Fm) : Prop := ∀ ρ t, a.eval ρ t → b.eval ρ t

/-! ## Monotone operators -/

section Mono
variable {A A' B B' : Pred}

theorem X_mono (h : Entails A A') : Entails (X A) (X A') := fun t => h (t + 1)

theorem F_mono (h : Entails A A') : Entails (F A) (F A') :=
  fun _ ⟨j, hj⟩ => ⟨j, h _ hj⟩

theorem G_mono (h : Entails A A') : Entails (G A) (G A') := fun _ hg j => h _ (hg j)

theorem U_mono (ha : Entails A A') (hb : Entails B B') : Entails (U A B) (U A' B') :=
  fun _ ⟨j, hq, hp⟩ => ⟨j, hb _ hq, fun i hi => ha _ (hp i hi)⟩

theorem W_mono (ha : Entails A A') (hb : Entails B B') : Entails (W A B) (W A' B')
  | t, .inl hu => .inl (U_mono ha hb t hu)
  | t, .inr hg => .inr (G_mono ha t hg)

theorem R_mono (ha : Entails A A') (hb : Entails B B') : Entails (R A B) (R A' B') := by
  intro t h j
  rcases h j with h | ⟨i, hi, hp⟩
  · exact .inl (hb _ h)
  · exact .inr ⟨i, hi, ha _ hp⟩

end Mono

/-! ## Contexts and polarity -/

/-- A formula with one hole, outside any `<->`. -/
inductive Ctx
  | hole
  | not (c : Ctx)
  | X (c : Ctx)
  | F (c : Ctx)
  | G (c : Ctx)
  | andL (c : Ctx) (b : Fm)
  | andR (a : Fm) (c : Ctx)
  | orL (c : Ctx) (b : Fm)
  | orR (a : Fm) (c : Ctx)
  | impL (c : Ctx) (b : Fm)
  | impR (a : Fm) (c : Ctx)
  | UL (c : Ctx) (b : Fm)
  | UR (a : Fm) (c : Ctx)
  | WL (c : Ctx) (b : Fm)
  | WR (a : Fm) (c : Ctx)
  | RL (c : Ctx) (b : Fm)
  | RR (a : Fm) (c : Ctx)

def Ctx.fill : Ctx → Fm → Fm
  | .hole, x => x
  | .not c, x => .not (c.fill x)
  | .X c, x => .X (c.fill x)
  | .F c, x => .F (c.fill x)
  | .G c, x => .G (c.fill x)
  | .andL c b, x => .and (c.fill x) b
  | .andR a c, x => .and a (c.fill x)
  | .orL c b, x => .or (c.fill x) b
  | .orR a c, x => .or a (c.fill x)
  | .impL c b, x => .imp (c.fill x) b
  | .impR a c, x => .imp a (c.fill x)
  | .UL c b, x => .U (c.fill x) b
  | .UR a c, x => .U a (c.fill x)
  | .WL c b, x => .W (c.fill x) b
  | .WR a c, x => .W a (c.fill x)
  | .RL c b, x => .R (c.fill x) b
  | .RR a c, x => .R a (c.fill x)

/-- The hole is at positive polarity. -/
def Ctx.pos : Ctx → Bool
  | .hole => true
  | .not c => !c.pos
  | .impL c _ => !c.pos
  | .X c | .F c | .G c => c.pos
  | .andL c _ | .andR _ c | .orL c _ | .orR _ c | .impR _ c => c.pos
  | .UL c _ | .UR _ c | .WL c _ | .WR _ c | .RL c _ | .RR _ c => c.pos

/-- The whole moves with the hole at a positive site, against it at a
negative one. -/
def Ctx.Moves (c : Ctx) (a b : Fm) : Prop :=
  if c.pos then (c.fill a).Le (c.fill b) else (c.fill b).Le (c.fill a)

theorem ctx_mono (c : Ctx) {a b : Fm} (h : a.Le b) : c.Moves a b := by
  induction c with
  | hole => simpa [Ctx.Moves, Ctx.pos, Ctx.fill] using h
  | not c ih =>
    unfold Ctx.Moves at *
    cases hc : c.pos <;> simp only [hc, Ctx.pos, Ctx.fill, Bool.not_false, Bool.not_true,
      ite_true] at ih ⊢ <;> intro ρ t hn hx <;> exact hn (ih ρ t hx)
  | impL c b' ih =>
    unfold Ctx.Moves at *
    cases hc : c.pos <;> simp only [hc, Ctx.pos, Ctx.fill, Bool.not_false, Bool.not_true,
      ite_true] at ih ⊢ <;> intro ρ t hi hx <;> exact hi (ih ρ t hx)
  | X c ih | F c ih | G c ih =>
    unfold Ctx.Moves at *
    cases hc : c.pos <;> simp only [hc, Ctx.pos, Ctx.fill, ite_true] at ih ⊢ <;>
      intro ρ <;> first
        | exact X_mono (ih ρ) | exact F_mono (ih ρ) | exact G_mono (ih ρ)
  | andL c b' ih | andR a' c ih | orL c b' ih | orR a' c ih | impR a' c ih =>
    unfold Ctx.Moves at *
    cases hc : c.pos <;> simp only [hc, Ctx.pos, Ctx.fill, ite_true] at ih ⊢ <;>
      intro ρ t <;> simp only [Fm.eval] <;> first
        | exact fun ⟨x, y⟩ => ⟨ih ρ t x, y⟩
        | exact fun ⟨x, y⟩ => ⟨x, ih ρ t y⟩
        | exact fun h => h.imp (ih ρ t) id
        | exact fun h => h.imp id (ih ρ t)
        | exact fun f x => ih ρ t (f x)
  | UL c b' ih | UR a' c ih | WL c b' ih | WR a' c ih | RL c b' ih | RR a' c ih =>
    unfold Ctx.Moves at *
    cases hc : c.pos <;> simp only [hc, Ctx.pos, Ctx.fill, ite_true] at ih ⊢ <;>
      intro ρ <;> first
        | exact U_mono (ih ρ) (fun _ h => h) | exact U_mono (fun _ h => h) (ih ρ)
        | exact W_mono (ih ρ) (fun _ h => h) | exact W_mono (fun _ h => h) (ih ρ)
        | exact R_mono (ih ρ) (fun _ h => h) | exact R_mono (fun _ h => h) (ih ρ)

/-! ## The rules -/

/-- Every output of `rules_at` and `apply_rule` at `a`, for the drawn literal
`ℓ`, weakening when `weaken` holds. -/
def rulesAt (weaken : Bool) (ℓ a : Fm) : List Fm :=
  (if weaken then [.tt, .or a ℓ] else [.ff, .and a ℓ]) ++
  match weaken, a with
  | true, .and x y => [x, y]
  | false, .or x y => [x, y]
  | true, .G x => [.F x, .G (.F x)]
  | false, .F x => [.G x]
  | true, .U x y => [.W x y]
  | false, .W x y => [.U x y]
  | true, .iff x y => [.imp x y, .imp y x]
  | false, .iff x y => [.and x y, .and (.not x) (.not y)]
  | true, .R _ y => [y]
  | false, .R _ y => [.G y]
  | true, .X x => [.F x]
  | false, .X x => [.G x]
  | _, _ => []

/-- The rules proposed in `docs/dev/order-search.md`. -/
def proposedAt (weaken : Bool) (a : Fm) : List Fm :=
  (if weaken then [.F a] else [.G a]) ++
  match weaken, a with
  | true, .G x => [x]
  | false, .F x => [x]
  | true, .U _ y => [.F y]
  | false, .U _ y => [y]
  | true, .W x y => [.or x y]
  | false, .W x y => [y, .G x]
  | false, .R x y => [.and x y]
  | true, .F (.G x) => [.G (.F x)]
  | false, .G (.F x) => [.F (.G x)]
  | _, _ => []

/-- `b` is a weakening of `a` when `weaken` holds, a strengthening otherwise. -/
def Toward (weaken : Bool) (a b : Fm) : Prop := if weaken then a.Le b else b.Le a

namespace Fm
variable {a x y : Fm}

theorem le_tt : a.Le .tt := fun _ _ _ => trivial
theorem ff_le : Fm.ff.Le a := fun _ _ h => h.elim
theorem le_orL : x.Le (.or x y) := fun _ _ h => .inl h
theorem le_orR : y.Le (.or x y) := fun _ _ h => .inr h
theorem andL_le : (Fm.and x y).Le x := fun _ _ h => h.1
theorem andR_le : (Fm.and x y).Le y := fun _ _ h => h.2
theorem G_le_F : (Fm.G x).Le (.F x) := fun _ _ h => ⟨0, h 0⟩
theorem G_le_GF : (Fm.G x).Le (.G (.F x)) := fun _ _ h j => ⟨0, h j⟩
theorem U_le_W : (Fm.U x y).Le (.W x y) := fun _ _ h => .inl h
theorem iff_le_imp : (Fm.iff x y).Le (.imp x y) := fun _ _ h => h.1
theorem iff_le_imp' : (Fm.iff x y).Le (.imp y x) := fun _ _ h => h.2
theorem R_le : (Fm.R x y).Le y := fun _ _ h =>
  (h 0).elim id fun ⟨_, hi, _⟩ => absurd hi (Nat.not_lt_zero _)
theorem X_le_F : (Fm.X x).Le (.F x) := fun _ _ h => ⟨1, h⟩
theorem and_le_iff : (Fm.and x y).Le (.iff x y) := fun _ _ ⟨hx, hy⟩ => ⟨fun _ => hy, fun _ => hx⟩
theorem nand_le_iff : (Fm.and (.not x) (.not y)).Le (.iff x y) := fun _ _ ⟨hx, hy⟩ =>
  ⟨fun h => (hx h).elim, fun h => (hy h).elim⟩
theorem G_le_R : (Fm.G y).Le (.R x y) := fun _ _ h j => .inl (h j)
theorem G_le_X : (Fm.G x).Le (.X x) := fun _ _ h => h 1
theorem le_F : a.Le (.F a) := fun _ _ h => ⟨0, h⟩
theorem G_le : (Fm.G a).Le a := fun _ _ h => h 0
theorem U_le_F : (Fm.U x y).Le (.F y) := fun _ _ ⟨j, hq, _⟩ => ⟨j, hq⟩
theorem W_le_or : (Fm.W x y).Le (.or x y) := fun _ _ h =>
  match h with
  | .inl ⟨0, hq, _⟩ => .inr hq
  | .inl ⟨_ + 1, _, hp⟩ => .inl (hp 0 (Nat.succ_pos _))
  | .inr hg => .inl (hg 0)
theorem FG_le_GF : (Fm.F (.G x)).Le (.G (.F x)) := fun _ t ⟨i, h⟩ k =>
  ⟨i, by have := h k; rwa [show t + i + k = t + k + i by omega] at this⟩
theorem le_U : y.Le (.U x y) := fun _ _ h => ⟨0, h, fun _ hi => absurd hi (Nat.not_lt_zero _)⟩
theorem le_W : y.Le (.W x y) := fun ρ t h => .inl (le_U (x := x) ρ t h)
theorem G_le_W : (Fm.G x).Le (.W x y) := fun _ _ h => .inr h
theorem and_le_R : (Fm.and x y).Le (.R x y) := fun _ _ ⟨hx, hy⟩ j =>
  match j with
  | 0 => .inl hy
  | _ + 1 => .inr ⟨0, Nat.succ_pos _, hx⟩

end Fm

macro "rule_cases" : tactic => `(tactic| (
  repeat' apply And.intro
  all_goals first
    | exact Fm.le_tt | exact Fm.ff_le | exact Fm.le_orL | exact Fm.le_orR
    | exact Fm.andL_le | exact Fm.andR_le | exact Fm.G_le_F | exact Fm.G_le_GF
    | exact Fm.U_le_W | exact Fm.iff_le_imp | exact Fm.iff_le_imp' | exact Fm.R_le
    | exact Fm.X_le_F | exact Fm.and_le_iff | exact Fm.nand_le_iff | exact Fm.G_le_R
    | exact Fm.G_le_X | exact Fm.le_F | exact Fm.G_le | exact Fm.U_le_F
    | exact Fm.W_le_or | exact Fm.FG_le_GF | exact Fm.le_U | exact Fm.le_W
    | exact Fm.G_le_W | exact Fm.and_le_R))

theorem rulesAt_sound (w : Bool) (ℓ a : Fm) : ∀ b ∈ rulesAt w ℓ a, Toward w a b := by
  cases w <;> cases a <;> simp [rulesAt, Toward] <;> rule_cases

theorem proposedAt_sound (w : Bool) (a : Fm) : ∀ b ∈ proposedAt w a, Toward w a b := by
  cases w <;> cases a <;> (try (rename_i x; cases x)) <;> simp [proposedAt, Toward] <;>
    rule_cases

/-- A rewrite at a site moves the whole formula the asked way when the local
rewrite moves the asked way at a positive site and the dual way at a negative
one. This is the contract of `monotone_rewrite`. -/
theorem rewrite_sound (c : Ctx) {w : Bool} {a b : Fm}
    (h : Toward (if c.pos then w else !w) a b) : Toward w (c.fill a) (c.fill b) := by
  cases hp : c.pos <;> cases w <;> simp [Toward, hp] at h ⊢ <;>
    have m := ctx_mono c h <;> simpa [Ctx.Moves, hp] using m

/-- Every rewrite that `monotone_rewrite` can return, with a current rule or a
proposed one, moves the formula the asked way. -/
theorem monotone_rewrite_sound (c : Ctx) (w : Bool) (ℓ a b : Fm)
    (h : b ∈ rulesAt (if c.pos then w else !w) ℓ a ∨ b ∈ proposedAt (if c.pos then w else !w) a) :
    Toward w (c.fill a) (c.fill b) :=
  rewrite_sound c (h.elim (rulesAt_sound _ ℓ a b) (proposedAt_sound _ a b))

end Gawain
