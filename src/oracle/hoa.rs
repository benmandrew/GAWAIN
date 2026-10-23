//! A reader for the subset of the HOA format that Spot's `ltl2tgba -H`
//! writes: one initial state, explicit edge labels, and generalised Büchi
//! acceptance `Inf(0)&…&Inf(n-1)` (or `t`), marked on states or on edges.
//!
//! Labels are boolean expressions over atomic-proposition indices. The
//! explicit back end reads them against a model where at most one atom
//! holds at each position, so a label is stored as the set of *letters*
//! it accepts: letter `i < aps.len()` is "only AP `i` holds", and letter
//! `aps.len()` is "no AP holds". With that alphabet a label is a bit mask.
//! Spot can build the automaton for this alphabet itself (`autfilt
//! --exclusive-ap`), but that costs a second process per query and would
//! still be read the same way here, so the reader does the restriction.

/// A bit mask over acceptance sets.
pub type Marks = u64;

/// One edge: the letters it reads, its target and its acceptance marks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    /// Bit `l` is set when letter `l` satisfies the label.
    pub letters: u64,
    /// The target state.
    pub target: u32,
    /// The acceptance sets of this edge, including those of its source
    /// state when the automaton marks states.
    pub marks: Marks,
}

/// A generalised Büchi automaton over the one-hot alphabet described in the
/// module header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Automaton {
    /// The atomic propositions, in index order.
    pub aps: Vec<String>,
    /// The initial state.
    pub start: u32,
    /// Every acceptance set; a run is accepting when it takes an edge of
    /// each set infinitely often. Zero means every infinite run accepts.
    pub all_marks: Marks,
    /// The outgoing edges of each state.
    pub edges: Vec<Vec<Edge>>,
}

/// Parses `ltl2tgba -H` output.
pub fn parse(text: &str) -> Result<Automaton, String> {
    let (header, body) = text
        .split_once("--BODY--")
        .ok_or("no --BODY-- in the automaton")?;
    let body = body
        .split_once("--END--")
        .ok_or("no --END-- in the automaton")?
        .0;
    let mut states = None;
    let mut start = None;
    let mut aps = None;
    let mut all_marks = None;
    for line in header.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "States" => states = Some(number(value)?),
            "Start" => {
                if start.is_some() {
                    return Err("more than one initial state".into());
                }
                start = Some(number(value)?);
            }
            "AP" => aps = Some(ap_names(value)?),
            "Acceptance" => all_marks = Some(acceptance(value)?),
            _ => {}
        }
    }
    let states = states.ok_or("no States: header")? as usize;
    let start = start.ok_or("no Start: header")?;
    let aps = aps.ok_or("no AP: header")?;
    let all_marks = all_marks.ok_or("no Acceptance: header")?;
    if aps.len() >= 64 {
        return Err(format!("{} atomic propositions, at most 63 fit", aps.len()));
    }
    if start as usize >= states {
        return Err(format!("initial state {start} out of range"));
    }

    let mut edges = vec![Vec::new(); states];
    let mut current: Option<(usize, Marks)> = None;
    for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("State:") {
            let rest = rest.trim();
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let state = number(&rest[..end])? as usize;
            if state >= states {
                return Err(format!("state {state} out of range"));
            }
            current = Some((state, trailing_marks(&rest[end..])?));
        } else if let Some(rest) = line.strip_prefix('[') {
            let (state, state_marks) = current.ok_or("an edge before any State:")?;
            let (label, rest) = rest.split_once(']').ok_or("unclosed label")?;
            let rest = rest.trim();
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let target = number(&rest[..end])?;
            if target as usize >= states {
                return Err(format!("edge target {target} out of range"));
            }
            let letters = Label::new(label, aps.len()).parse()?;
            let marks = state_marks | trailing_marks(&rest[end..])?;
            edges[state].push(Edge {
                letters,
                target,
                marks,
            });
        } else {
            return Err(format!("unsupported body line {line:?}"));
        }
    }
    Ok(Automaton {
        aps,
        start,
        all_marks,
        edges,
    })
}

fn number(text: &str) -> Result<u32, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("expected a number, found {text:?}"))
}

/// `2 "a" "b"`: a count, then that many quoted names.
fn ap_names(value: &str) -> Result<Vec<String>, String> {
    let (count, mut rest) = value.split_once(' ').unwrap_or((value, ""));
    let count = number(count)? as usize;
    let mut names = Vec::with_capacity(count);
    loop {
        rest = rest.trim_start();
        let Some(quoted) = rest.strip_prefix('"') else {
            break;
        };
        let mut name = String::new();
        let mut chars = quoted.char_indices();
        let end = loop {
            match chars.next() {
                Some((_, '\\')) => name.extend(chars.next().map(|(_, c)| c)),
                Some((i, '"')) => break i,
                Some((_, c)) => name.push(c),
                None => return Err("unclosed AP name".into()),
            }
        };
        names.push(name);
        rest = &quoted[end + 1..];
    }
    if names.len() != count || !rest.trim().is_empty() {
        return Err(format!("malformed AP line {value:?}"));
    }
    Ok(names)
}

/// `n Inf(0)&…&Inf(n-1)`, or `0 t`, as the mask of all `n` sets.
fn acceptance(value: &str) -> Result<Marks, String> {
    let (count, condition) = value.split_once(' ').ok_or("malformed Acceptance:")?;
    let count = number(count)?;
    if count >= 64 {
        return Err(format!("{count} acceptance sets, at most 63 fit"));
    }
    let condition: String = condition.split_whitespace().collect();
    let expected = if count == 0 {
        "t".to_string()
    } else {
        (0..count)
            .map(|i| format!("Inf({i})"))
            .collect::<Vec<_>>()
            .join("&")
    };
    if condition != expected {
        return Err(format!("unsupported acceptance {value:?}"));
    }
    Ok((1 << count) - 1)
}

/// An optional `{0 1}` after a state or an edge.
fn trailing_marks(text: &str) -> Result<Marks, String> {
    let text = text.trim();
    // A quoted state name may precede the marks; Spot writes none with -H.
    if text.is_empty() {
        return Ok(0);
    }
    let inner = text
        .strip_prefix('{')
        .and_then(|t| t.strip_suffix('}'))
        .ok_or_else(|| format!("unsupported text {text:?}"))?;
    inner.split_whitespace().try_fold(0, |marks, set| {
        let set = number(set)?;
        if set >= 64 {
            return Err(format!("acceptance set {set} out of range"));
        }
        Ok(marks | 1 << set)
    })
}

/// A recursive-descent reader for one label, `|` looser than `&` looser
/// than `!`. Labels are short, so recursion depth is no concern here.
struct Label<'a> {
    text: &'a [u8],
    at: usize,
    aps: usize,
}

impl<'a> Label<'a> {
    fn new(text: &'a str, aps: usize) -> Self {
        Label {
            text: text.as_bytes(),
            at: 0,
            aps,
        }
    }

    /// Every letter: one per AP, plus "no AP".
    fn full(&self) -> u64 {
        (1 << (self.aps + 1)) - 1
    }

    fn parse(mut self) -> Result<u64, String> {
        let mask = self.or()?;
        self.skip_space();
        if self.at != self.text.len() {
            return Err(format!("trailing text in label at byte {}", self.at));
        }
        Ok(mask)
    }

    fn skip_space(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_space();
        let found = self.text.get(self.at) == Some(&byte);
        if found {
            self.at += 1;
        }
        found
    }

    fn or(&mut self) -> Result<u64, String> {
        let mut mask = self.and()?;
        while self.eat(b'|') {
            mask |= self.and()?;
        }
        Ok(mask)
    }

    fn and(&mut self) -> Result<u64, String> {
        let mut mask = self.not()?;
        while self.eat(b'&') {
            mask &= self.not()?;
        }
        Ok(mask)
    }

    fn not(&mut self) -> Result<u64, String> {
        if self.eat(b'!') {
            return Ok(!self.not()? & self.full());
        }
        if self.eat(b'(') {
            let mask = self.or()?;
            return if self.eat(b')') {
                Ok(mask)
            } else {
                Err("unclosed ( in label".into())
            };
        }
        if self.eat(b't') {
            return Ok(self.full());
        }
        if self.eat(b'f') {
            return Ok(0);
        }
        let start = self.at;
        while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
        let digits = std::str::from_utf8(&self.text[start..self.at]).expect("ASCII digits");
        let ap = number(digits)? as usize;
        if ap >= self.aps {
            return Err(format!("AP {ap} out of range in label"));
        }
        Ok(1 << ap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENERALISED: &str = r#"HOA: v1
name: "G(Fa & Fb)"
States: 1
Start: 0
AP: 2 "a" "b"
acc-name: generalized-Buchi 2
Acceptance: 2 Inf(0)&Inf(1)
properties: trans-labels explicit-labels trans-acc complete
--BODY--
State: 0
[!0&!1] 0
[0&!1] 0 {0}
[!0&1] 0 {1}
[0&1] 0 {0 1}
--END--
"#;

    const STATE_BASED: &str = r#"HOA: v1
name: "\"x y\" U !a"
States: 2
Start: 1
AP: 2 "x y" "a\"b"
acc-name: Buchi
Acceptance: 1 Inf(0)
properties: trans-labels explicit-labels state-acc deterministic
--BODY--
State: 0 {0}
[t] 0
State: 1
[!1] 0
[0&1 | (f)] 1
--END--
"#;

    #[test]
    fn transition_marks_and_one_hot_letters() {
        let a = parse(GENERALISED).unwrap();
        assert_eq!(a.aps, ["a", "b"]);
        assert_eq!((a.start, a.all_marks), (0, 0b11));
        // Letters: bit 0 "only a", bit 1 "only b", bit 2 "neither".
        let read: Vec<_> = a.edges[0].iter().map(|e| (e.letters, e.marks)).collect();
        assert_eq!(read, [(0b100, 0), (0b001, 0b01), (0b010, 0b10), (0, 0b11)]);
    }

    #[test]
    fn state_marks_move_onto_their_edges() {
        let a = parse(STATE_BASED).unwrap();
        assert_eq!(a.aps, ["x y", "a\"b"]);
        assert_eq!((a.start, a.all_marks), (1, 1));
        assert_eq!(
            a.edges[0],
            [Edge {
                letters: 0b111,
                target: 0,
                marks: 1
            }]
        );
        assert_eq!(a.edges[1][0].letters, 0b101);
        assert_eq!(a.edges[1][1].letters, 0);
        assert_eq!(a.edges[1][1].marks, 0);
    }

    #[test]
    fn all_accepting_with_no_edges() {
        let text =
            "HOA: v1\nStates: 1\nStart: 0\nAP: 0\nAcceptance: 0 t\n--BODY--\nState: 0\n--END--\n";
        let a = parse(text).unwrap();
        assert_eq!(a.all_marks, 0);
        assert!(a.edges[0].is_empty());
    }

    #[test]
    fn unsupported_inputs_are_errors() {
        let co_buchi = GENERALISED.replace("Inf(0)&Inf(1)", "Fin(0)&Inf(1)");
        assert!(parse(&co_buchi).unwrap_err().contains("acceptance"));
        let two_starts = GENERALISED.replace("Start: 0", "Start: 0\nStart: 0");
        assert!(parse(&two_starts).is_err());
        let implicit = GENERALISED.replace("[!0&!1] 0", "0");
        assert!(parse(&implicit).is_err());
        let range = GENERALISED.replace("[!0&!1] 0", "[!2] 0");
        assert!(parse(&range).unwrap_err().contains("out of range"));
        assert!(parse("HOA: v1\n").is_err());
    }
}
