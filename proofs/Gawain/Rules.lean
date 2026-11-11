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

/-! The order-guided rules of `RuleSet::Ordered`, one function per `Rule`
variant of `src/search/operators.rs`, each listing its outcomes where it
applies and nothing elsewhere. -/

def introduce (w : Bool) (a : Fm) : List Fm := [if w then .F a else .G a]

def strip : Bool → Fm → List Fm
  | true, .G x => [x]
  | false, .F x => [x]
  | _, _ => []

def exchangeFG : Bool → Fm → List Fm
  | true, .F (.G x) => [.G (.F x)]
  | false, .G (.F x) => [.F (.G x)]
  | _, _ => []

def splitEventually : Bool → Fm → List Fm
  | true, .F (.and x y) => [.and (.F x) (.F y)]
  | _, _ => []

def splitMeet : Bool → Fm → List Fm
  | false, .F (.and x y) => [.and (.G x) (.F y), .and (.F x) (.G y)]
  | _, _ => []

def eventuallyInward : Bool → Fm → List Fm
  | true, .F (.R x y) => [.R x (.F y)]
  | true, .F (.W x y) => [.W (.F x) (.F y)]
  | _, _ => []

def globallyInward : Bool → Fm → List Fm
  | false, .G (.U x y) => [.U x (.G y)]
  | false, .G (.W x y) => [.W x (.G y)]
  | _, _ => []

def untilToEventually : Bool → Fm → List Fm
  | true, .U _ y => [.F y]
  | _, _ => []

def weakUntilToDisjunction : Bool → Fm → List Fm
  | true, .W x y => [.or x y]
  | _, _ => []

def globallyOutward : Bool → Fm → List Fm
  | true, .U x (.G y) => [.G (.U x y)]
  | true, .W x (.G y) => [.G (.W x y)]
  | _, _ => []

def untilToConsequent : Bool → Fm → List Fm
  | false, .U _ y => [y]
  | false, .W _ y => [y]
  | _, _ => []

def untilToNext : Bool → Fm → List Fm
  | false, .U x y => [.and x (.X y)]
  | _, _ => []

def weakUntilToGlobally : Bool → Fm → List Fm
  | false, .W x _ => [.G x]
  | _, _ => []

def releaseToConjunction : Bool → Fm → List Fm
  | false, .R x y => [.and x y]
  | _, _ => []

def eventuallyOutward : Bool → Fm → List Fm
  | false, .W (.F x) (.F y) => [.F (.W x y)]
  | false, .R x (.F y) => [.F (.R x y)]
  | _, _ => []

def releaseToNext : Bool → Fm → List Fm
  | true, .R x y => [.or x (.X y)]
  | _, _ => []

def nextToUntil : Bool → Fm → List Fm
  | true, .and x (.X y) => [.U x y]
  | _, _ => []

def meetEventually : Bool → Fm → List Fm
  | true, .and (.G x) (.F y) => [.F (.and x y)]
  | true, .and (.F x) (.G y) => [.F (.and x y)]
  | _, _ => []

def joinEventually : Bool → Fm → List Fm
  | false, .and (.F x) (.F y) => [.F (.and x y)]
  | _, _ => []

def nextToRelease : Bool → Fm → List Fm
  | false, .or x (.X y) => [.R x y]
  | _, _ => []

/-- The order-guided rules at `a`, in the order `ordered_rules` pushes them.
At any one node and direction, the rules that apply come in that order. -/
def proposedAt (w : Bool) (a : Fm) : List Fm :=
  introduce w a ++ strip w a ++ exchangeFG w a ++ splitEventually w a ++ splitMeet w a ++
    eventuallyInward w a ++ globallyInward w a ++ untilToEventually w a ++
    weakUntilToDisjunction w a ++ globallyOutward w a ++ untilToConsequent w a ++
    untilToNext w a ++ weakUntilToGlobally w a ++ releaseToConjunction w a ++
    eventuallyOutward w a ++ releaseToNext w a ++ nextToUntil w a ++ meetEventually w a ++
    joinEventually w a ++ nextToRelease w a

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

theorem R_le_orX : (Fm.R x y).Le (.or x (.X y)) := by
  intro ρ t h
  simp only [eval, Gawain.R, Gawain.X] at h ⊢
  rcases h 1 with hy | ⟨i, hi, hx⟩
  · exact .inr hy
  · have : i = 0 := by omega
    subst this; exact .inl hx
theorem andX_le_U : (Fm.and x (.X y)).Le (.U x y) := by
  intro ρ t ⟨hx, hy⟩
  refine ⟨1, hy, fun i hi => ?_⟩
  have : i = 0 := by omega
  subst this; exact hx
theorem F_and_le : (Fm.F (.and x y)).Le (.and (.F x) (.F y)) :=
  fun _ _ ⟨j, hx, hy⟩ => ⟨⟨j, hx⟩, ⟨j, hy⟩⟩
theorem GF_le : (Fm.and (.G x) (.F y)).Le (.F (.and x y)) :=
  fun _ _ ⟨hg, ⟨j, hy⟩⟩ => ⟨j, hg j, hy⟩
theorem FG_le : (Fm.and (.F x) (.G y)).Le (.F (.and x y)) :=
  fun _ _ ⟨⟨j, hx⟩, hg⟩ => ⟨j, hx, hg j⟩
theorem F_R_le : (Fm.F (.R x y)).Le (.R x (.F y)) := by
  intro ρ t h
  simp only [eval, Gawain.R, Gawain.F] at h ⊢
  obtain ⟨j, h⟩ := h
  intro k
  by_cases hk : k ≤ j
  · rcases h 0 with hy | ⟨i, hi, _⟩
    · exact .inl ⟨j - k, by rwa [show t + k + (j - k) = t + j + 0 by omega]⟩
    · omega
  · rcases h (k - j) with hy | ⟨i, hi, hx⟩
    · exact .inl ⟨0, by rwa [show t + k + 0 = t + j + (k - j) by omega]⟩
    · exact .inr ⟨j + i, by omega, by rwa [show t + (j + i) = t + j + i by omega]⟩
theorem F_W_le : (Fm.F (.W x y)).Le (.W (.F x) (.F y)) := by
  intro ρ t h
  simp only [eval, Gawain.W, Gawain.U, Gawain.G, Gawain.F] at h ⊢
  obtain ⟨j, ⟨m, hy, _⟩ | hg⟩ := h
  · exact .inl ⟨0, ⟨j + m, by rwa [show t + 0 + (j + m) = t + j + m by omega]⟩,
      fun i hi => absurd hi (Nat.not_lt_zero _)⟩
  · exact .inr fun k => ⟨j, by have := hg k; rwa [show t + k + j = t + j + k by omega]⟩
theorem U_G_le : (Fm.U x (.G y)).Le (.G (.U x y)) := by
  intro ρ t h
  simp only [eval, Gawain.U, Gawain.G] at h ⊢
  obtain ⟨j, hg, hx⟩ := h
  intro k
  by_cases hk : j ≤ k
  · exact ⟨0, by have := hg (k - j); rwa [show t + j + (k - j) = t + k + 0 by omega] at this,
      fun i hi => absurd hi (Nat.not_lt_zero _)⟩
  · refine ⟨j - k, by have := hg 0; rwa [show t + j + 0 = t + k + (j - k) by omega] at this,
      fun i hi => ?_⟩
    have := hx (k + i) (by omega)
    rwa [show t + (k + i) = t + k + i by omega] at this
theorem W_G_le : (Fm.W x (.G y)).Le (.G (.W x y)) := by
  intro ρ t h
  rcases h with hu | hg
  · exact fun k => .inl (U_G_le (x := x) (y := y) ρ t hu k)
  · simp only [eval, Gawain.W, Gawain.G] at hg ⊢
    exact fun k => .inr fun m => by have := hg (k + m); rwa [show t + (k + m) = t + k + m by omega] at this

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
    | exact Fm.G_le_W | exact Fm.and_le_R | exact Fm.R_le_orX | exact Fm.andX_le_U
    | exact Fm.F_and_le | exact Fm.GF_le | exact Fm.FG_le | exact Fm.F_R_le
    | exact Fm.F_W_le | exact Fm.U_G_le | exact Fm.W_G_le))

theorem rulesAt_sound (w : Bool) (ℓ a : Fm) : ∀ b ∈ rulesAt w ℓ a, Toward w a b := by
  cases w <;> cases a <;> simp [rulesAt, Toward] <;> rule_cases

/-- A rule given as a function sends every node to a list of outcomes that
move the asked way. -/
def RuleSound (r : Bool → Fm → List Fm) : Prop := ∀ w a, ∀ b ∈ r w a, Toward w a b

set_option hygiene false in
macro "rule_sound" r:ident : tactic => `(tactic| (
  intro w a b h
  revert h
  unfold $r
  split <;> intro h <;> simp only [List.mem_cons, List.not_mem_nil, or_false] at h
  all_goals rcases h with rfl | rfl
  all_goals simp only [Toward, ite_true, ite_false, Bool.false_eq_true]
  all_goals rule_cases))

theorem introduce_sound : RuleSound introduce := by
  intro w a b h
  cases w <;> simp [introduce, Toward] at h ⊢ <;> subst h
  · exact Fm.G_le
  · exact Fm.le_F

theorem proposedAt_sound (w : Bool) (a : Fm) : ∀ b ∈ proposedAt w a, Toward w a b := by
  have hs : ∀ r ∈ [strip, exchangeFG, splitEventually, splitMeet, eventuallyInward,
      globallyInward, untilToEventually, weakUntilToDisjunction, globallyOutward,
      untilToConsequent, untilToNext, weakUntilToGlobally, releaseToConjunction,
      eventuallyOutward, releaseToNext, nextToUntil, meetEventually, joinEventually,
      nextToRelease], RuleSound r := by
    simp only [List.mem_cons, List.not_mem_nil, or_false]
    rintro r (rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl |
      rfl | rfl | rfl | rfl | rfl | rfl)
    · rule_sound strip
    · rule_sound exchangeFG
    · rule_sound splitEventually
    · rule_sound splitMeet
    · rule_sound eventuallyInward
    · rule_sound globallyInward
    · rule_sound untilToEventually
    · rule_sound weakUntilToDisjunction
    · rule_sound globallyOutward
    · rule_sound untilToConsequent
    · rule_sound untilToNext
    · rule_sound weakUntilToGlobally
    · rule_sound releaseToConjunction
    · rule_sound eventuallyOutward
    · rule_sound releaseToNext
    · rule_sound nextToUntil
    · rule_sound meetEventually
    · rule_sound joinEventually
    · rule_sound nextToRelease
  intro b h
  simp only [proposedAt, List.mem_append] at h
  have one := fun r (hr : r ∈ _) (hb : b ∈ r w a) => hs r hr w a b hb
  simp only [List.mem_cons, List.not_mem_nil, or_false] at one
  rcases h with ((((((((((((((((((h | h) | h) | h) | h) | h) | h) | h) | h) | h) | h) | h) |
    h) | h) | h) | h) | h) | h) | h) | h
  · exact introduce_sound w a b h
  all_goals first
    | exact one _ (.inl rfl) h
    | exact one _ (.inr (.inl rfl)) h
    | exact one _ (by simp) h

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

/-! ## Every outcome of `monotone_outcomes`

`outs` transcribes `monotone_outcomes` and `outcomes_below` in
`src/search/operators.rs`: the polarity is `some true` (positive), `some
false` (negative) or `none` (under `<->`), and each site contributes the
local outcomes in pre-order. `bench/check_lean_orders.py` compares its lists
with the Rust ones, so this transcription is checked by running both.
`outs_sound` shows that every outcome moves the whole formula the asked way.
-/

/-- The local outcomes at one site: `rules_at` with `RuleSet::Peredur`, then the
order-guided rules under `RuleSet::Ordered`. -/
def localOuts (ordered w : Bool) (ℓ a : Fm) : List Fm :=
  rulesAt w ℓ a ++ (if ordered then proposedAt w a else [])

def flipPol : Option Bool → Option Bool
  | some p => some (!p)
  | none => none

def outs (ordered want : Bool) (ℓ : Fm) : Fm → Option Bool → List Fm
  | a, pol =>
    (match pol with
      | some p => localOuts ordered (p == want) ℓ a
      | none => []) ++
    match a with
    | .tt | .ff | .atom _ => []
    | .not x => (outs ordered want ℓ x (flipPol pol)).map .not
    | .X x => (outs ordered want ℓ x pol).map .X
    | .F x => (outs ordered want ℓ x pol).map .F
    | .G x => (outs ordered want ℓ x pol).map .G
    | .and x y => (outs ordered want ℓ x pol).map (.and · y) ++ (outs ordered want ℓ y pol).map (.and x ·)
    | .or x y => (outs ordered want ℓ x pol).map (.or · y) ++ (outs ordered want ℓ y pol).map (.or x ·)
    | .imp x y =>
      (outs ordered want ℓ x (flipPol pol)).map (.imp · y) ++ (outs ordered want ℓ y pol).map (.imp x ·)
    | .iff x y =>
      (outs ordered want ℓ x none).map (.iff · y) ++ (outs ordered want ℓ y none).map (.iff x ·)
    | .U x y => (outs ordered want ℓ x pol).map (.U · y) ++ (outs ordered want ℓ y pol).map (.U x ·)
    | .W x y => (outs ordered want ℓ x pol).map (.W · y) ++ (outs ordered want ℓ y pol).map (.W x ·)
    | .R x y => (outs ordered want ℓ x pol).map (.R · y) ++ (outs ordered want ℓ y pol).map (.R x ·)

theorem outs_none (o w : Bool) (ℓ a : Fm) : outs o w ℓ a none = [] := by
  induction a <;> simp_all [outs, flipPol]

theorem localOuts_sound {o w : Bool} {ℓ a b : Fm} (h : b ∈ localOuts o w ℓ a) : Toward w a b := by
  simp only [localOuts, List.mem_append] at h
  rcases h with h | h
  · exact rulesAt_sound w ℓ a b h
  · cases o
    · simp at h
    · exact proposedAt_sound w a b (by simpa using h)

theorem Toward.mono {w : Bool} {x x' : Fm} {C : Fm → Fm}
    (hC : ∀ a b : Fm, a.Le b → (C a).Le (C b)) (h : Toward w x x') : Toward w (C x) (C x') := by
  cases w <;> simp only [Toward, ite_true, ite_false, Bool.false_eq_true] at h ⊢ <;> exact hC _ _ h

theorem Toward.anti {w : Bool} {x x' : Fm} {C : Fm → Fm}
    (hC : ∀ a b : Fm, a.Le b → (C b).Le (C a)) (h : Toward w x x') : Toward (!w) (C x) (C x') := by
  cases w <;> simp only [Toward, ite_true, ite_false, Bool.false_eq_true, Bool.not_true,
    Bool.not_false] at h ⊢ <;> exact hC _ _ h

theorem outs_sound (o want : Bool) (ℓ : Fm) :
    ∀ (a : Fm) (p : Bool) (b : Fm), b ∈ outs o want ℓ a (some p) → Toward (p == want) a b := by
  intro a
  induction a with
  | tt | ff | atom _ =>
    intro p b h
    rcases List.mem_append.1 h with h | h
    · exact localOuts_sound h
    · simp at h
  | not x ih =>
    intro p b h
    rcases List.mem_append.1 h with h | h
    · exact localOuts_sound h
    · obtain ⟨b', hb', rfl⟩ := List.mem_map.1 h
      have := Toward.anti (C := Fm.not) (fun a b hab ρ t hn ha => hn (hab ρ t ha)) (ih _ _ hb')
      cases p <;> cases want <;> exact this
  | X x ih | F x ih | G x ih =>
    intro p b h
    rcases List.mem_append.1 h with h | h
    · exact localOuts_sound h
    · obtain ⟨b', hb', rfl⟩ := List.mem_map.1 h
      refine Toward.mono ?_ (ih _ _ hb')
      first
        | exact fun a b hab ρ => X_mono (hab ρ)
        | exact fun a b hab ρ => F_mono (hab ρ)
        | exact fun a b hab ρ => G_mono (hab ρ)
  | iff x y _ _ =>
    intro p b h
    rcases List.mem_append.1 h with h | h
    · exact localOuts_sound h
    · simp [outs_none] at h
  | imp x y ihx ihy =>
    intro p b h
    rcases List.mem_append.1 h with h | h
    · exact localOuts_sound h
    rcases List.mem_append.1 h with h | h
    · obtain ⟨b', hb', rfl⟩ := List.mem_map.1 h
      have := Toward.anti (C := (Fm.imp · y)) (fun a b hab ρ t hi ha => hi (hab ρ t ha))
        (ihx _ _ hb')
      cases p <;> cases want <;> exact this
    · obtain ⟨b', hb', rfl⟩ := List.mem_map.1 h
      exact Toward.mono (C := Fm.imp x) (fun a b hab ρ t hi hx => hab ρ t (hi hx)) (ihy _ _ hb')
  | and x y ihx ihy | or x y ihx ihy | U x y ihx ihy | W x y ihx ihy | R x y ihx ihy =>
    intro p b h
    rcases List.mem_append.1 h with h | h
    · exact localOuts_sound h
    rcases List.mem_append.1 h with h | h
    · obtain ⟨b', hb', rfl⟩ := List.mem_map.1 h
      have ih := ihx _ _ hb'
      first
        | exact Toward.mono (C := (Fm.and · y)) (fun a b hab ρ t ⟨u, v⟩ => ⟨hab ρ t u, v⟩) ih
        | exact Toward.mono (C := (Fm.or · y)) (fun a b hab ρ t h => Or.imp (hab ρ t) id h) ih
        | exact Toward.mono (C := (Fm.U · y)) (fun a b hab ρ => U_mono (hab ρ) (fun _ h => h)) ih
        | exact Toward.mono (C := (Fm.W · y)) (fun a b hab ρ => W_mono (hab ρ) (fun _ h => h)) ih
        | exact Toward.mono (C := (Fm.R · y)) (fun a b hab ρ => R_mono (hab ρ) (fun _ h => h)) ih
    · obtain ⟨b', hb', rfl⟩ := List.mem_map.1 h
      have ih := ihy _ _ hb'
      first
        | exact Toward.mono (C := Fm.and x) (fun a b hab ρ t ⟨u, v⟩ => ⟨u, hab ρ t v⟩) ih
        | exact Toward.mono (C := Fm.or x) (fun a b hab ρ t h => Or.imp id (hab ρ t) h) ih
        | exact Toward.mono (C := Fm.U x) (fun a b hab ρ => U_mono (fun _ h => h) (hab ρ)) ih
        | exact Toward.mono (C := Fm.W x) (fun a b hab ρ => W_mono (fun _ h => h) (hab ρ)) ih
        | exact Toward.mono (C := Fm.R x) (fun a b hab ρ => R_mono (fun _ h => h) (hab ρ)) ih

/-- Every formula that `monotone_outcomes` lists, under either rule set,
moves the whole formula the asked way. -/
theorem monotone_outcomes_sound (o want : Bool) (ℓ a b : Fm)
    (h : b ∈ outs o want ℓ a (some true)) : Toward want a b := by
  have := outs_sound o want ℓ a true b h
  cases want <;> exact this

end Gawain
