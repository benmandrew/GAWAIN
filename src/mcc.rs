//! Model Checking Contest inputs: P/T nets in PNML, and the LTLCardinality
//! and LTLFireability properties in the contest's XML format.
//!
//! A [`Net`] holds places, the initial marking and transitions with
//! weighted pre- and post-sets, enough to replay a run that a model checker
//! reports. A [`Property`] is an LTL formula over named atoms, each atom a
//! [`Predicate`] on markings: a comparison of token counts or the
//! enabledness of some transition. Atoms are named `a0`, `a1`, … in order of
//! first appearance, with structurally equal predicates sharing one name.
//!
//! Coloured nets, inhibitor arcs and any XML element not listed below are
//! rejected with an error, never approximated. See `bench/notes/mcc.md`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use roxmltree::{Document, Node as XmlNode};

use crate::ltl::{BinaryOp, Formula, Node, UnaryOp};

/// A marking: the token count of each place, in [`Net`] place order.
pub type Marking = Vec<u64>;

/// One transition, with its input and output arcs as (place, weight).
#[derive(Clone, Debug)]
struct Transition {
    id: String,
    pre: Vec<(usize, u64)>,
    post: Vec<(usize, u64)>,
}

/// A place/transition net with its initial marking.
#[derive(Clone, Debug)]
pub struct Net {
    places: Vec<String>,
    place_index: HashMap<String, usize>,
    initial: Marking,
    transitions: Vec<Transition>,
    transition_index: HashMap<String, usize>,
}

impl Net {
    /// Parses a PNML P/T net. Pages are flattened; reference nodes,
    /// coloured nets and arcs other than normal ones are errors.
    pub fn parse(pnml: &str) -> Result<Net, String> {
        let doc = Document::parse(pnml).map_err(|e| format!("PNML: {e}"))?;
        let net = doc
            .descendants()
            .find(|n| n.has_tag_name("net"))
            .ok_or("PNML: no <net>")?;
        if !net.attribute("type").unwrap_or("").ends_with("ptnet") {
            return Err(format!("PNML: not a P/T net ({:?})", net.attribute("type")));
        }
        let mut places = Vec::new();
        let mut initial = Vec::new();
        let mut transitions = Vec::new();
        let mut arcs = Vec::new();
        for node in net.descendants().filter(XmlNode::is_element) {
            match node.tag_name().name() {
                "place" => {
                    let marking = match child(node, "initialMarking") {
                        Some(m) => number(text(m).unwrap_or("0"))?,
                        None => 0,
                    };
                    places.push(id(node)?.to_owned());
                    initial.push(marking);
                }
                "transition" => transitions.push(Transition {
                    id: id(node)?.to_owned(),
                    pre: Vec::new(),
                    post: Vec::new(),
                }),
                "arc" => arcs.push(node),
                "referencePlace" | "referenceTransition" => {
                    return Err("PNML: reference nodes are not supported".into())
                }
                _ => {}
            }
        }
        let place_index = index(places.iter())?;
        let transition_index = index(transitions.iter().map(|t| &t.id))?;
        for arc in arcs {
            if let Some(kind) = child(arc, "type").and_then(|t| t.attribute("value")) {
                if kind != "normal" {
                    return Err(format!("PNML: {kind} arcs are not supported"));
                }
            }
            let weight = match child(arc, "inscription") {
                Some(i) => number(text(i).unwrap_or("1"))?,
                None => 1,
            };
            let source = arc.attribute("source").ok_or("PNML: arc without source")?;
            let target = arc.attribute("target").ok_or("PNML: arc without target")?;
            match (
                place_index.get(source),
                transition_index.get(target),
                transition_index.get(source),
                place_index.get(target),
            ) {
                (Some(&p), Some(&t), _, _) => transitions[t].pre.push((p, weight)),
                (_, _, Some(&t), Some(&p)) => transitions[t].post.push((p, weight)),
                _ => {
                    return Err(format!(
                        "PNML: arc {source} -> {target} is not place-transition"
                    ))
                }
            }
        }
        Ok(Net {
            places,
            place_index,
            initial,
            transitions,
            transition_index,
        })
    }

    /// The initial marking.
    pub fn initial(&self) -> &[u64] {
        &self.initial
    }

    /// The number of transitions.
    pub fn transition_count(&self) -> usize {
        self.transitions.len()
    }

    /// The index of the transition with PNML id `id`.
    pub fn transition(&self, id: &str) -> Option<usize> {
        self.transition_index.get(id).copied()
    }

    /// Whether transition `t` is enabled in `marking`.
    pub fn enabled(&self, marking: &[u64], t: usize) -> bool {
        self.transitions[t]
            .pre
            .iter()
            .all(|&(p, w)| marking[p] >= w)
    }

    /// Whether no transition is enabled in `marking`.
    pub fn is_deadlock(&self, marking: &[u64]) -> bool {
        (0..self.transitions.len()).all(|t| !self.enabled(marking, t))
    }

    /// Fires transition `t` in `marking`, or returns false and leaves the
    /// marking alone when `t` is not enabled.
    pub fn fire(&self, marking: &mut [u64], t: usize) -> bool {
        if !self.enabled(marking, t) {
            return false;
        }
        let transition = &self.transitions[t];
        for &(p, w) in &transition.pre {
            marking[p] -= w;
        }
        for &(p, w) in &transition.post {
            marking[p] += w;
        }
        true
    }
}

fn index<'a>(names: impl Iterator<Item = &'a String>) -> Result<HashMap<String, usize>, String> {
    let mut map = HashMap::new();
    for (i, name) in names.enumerate() {
        if map.insert(name.clone(), i).is_some() {
            return Err(format!("PNML: duplicate id {name}"));
        }
    }
    Ok(map)
}

fn id<'a>(node: XmlNode<'a, '_>) -> Result<&'a str, String> {
    node.attribute("id")
        .ok_or_else(|| format!("PNML: <{}> without id", node.tag_name().name()))
}

/// The first element child of `node` named `name`.
fn child<'a, 'i>(node: XmlNode<'a, 'i>, name: &str) -> Option<XmlNode<'a, 'i>> {
    node.children().find(|c| c.has_tag_name(name))
}

/// The element children of `node`.
fn elements<'a, 'i>(node: XmlNode<'a, 'i>) -> impl Iterator<Item = XmlNode<'a, 'i>> {
    node.children().filter(XmlNode::is_element)
}

/// The text of the `<text>` child of `node`, trimmed.
fn text<'a>(node: XmlNode<'a, '_>) -> Option<&'a str> {
    child(node, "text").and_then(|t| t.text()).map(str::trim)
}

fn number(text: &str) -> Result<u64, String> {
    text.trim()
        .parse()
        .map_err(|e| format!("not a token count {text:?}: {e}"))
}

/// One side of a cardinality comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Count {
    /// A constant.
    Constant(u64),
    /// The total number of tokens in these places.
    Tokens(Vec<usize>),
}

impl Count {
    fn value(&self, marking: &[u64]) -> u64 {
        match self {
            Count::Constant(c) => *c,
            Count::Tokens(places) => places.iter().map(|&p| marking[p]).sum(),
        }
    }
}

/// An atomic proposition on markings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Predicate {
    /// At least one of these transitions is enabled.
    Fireable(Vec<usize>),
    /// The left count is at most the right one.
    Le(Count, Count),
}

impl Predicate {
    /// Whether the predicate holds in `marking` of `net`.
    pub fn holds(&self, net: &Net, marking: &[u64]) -> bool {
        match self {
            Predicate::Fireable(ts) => ts.iter().any(|&t| net.enabled(marking, t)),
            Predicate::Le(l, r) => l.value(marking) <= r.value(marking),
        }
    }

    fn write_xml(&self, net: &Net, out: &mut String) {
        match self {
            Predicate::Fireable(ts) => {
                out.push_str("<is-fireable>");
                for &t in ts {
                    let _ = write!(out, "<transition>{}</transition>", net.transitions[t].id);
                }
                out.push_str("</is-fireable>");
            }
            Predicate::Le(l, r) => {
                out.push_str("<integer-le>");
                for count in [l, r] {
                    match count {
                        Count::Constant(c) => {
                            let _ = write!(out, "<integer-constant>{c}</integer-constant>");
                        }
                        Count::Tokens(ps) => {
                            out.push_str("<tokens-count>");
                            for &p in ps {
                                let _ = write!(out, "<place>{}</place>", net.places[p]);
                            }
                            out.push_str("</tokens-count>");
                        }
                    }
                }
                out.push_str("</integer-le>");
            }
        }
    }
}

/// The named atoms of one property and the predicate behind each.
pub type Atoms = Vec<(Arc<str>, Predicate)>;

/// One LTL property of an MCC examination.
#[derive(Clone, Debug)]
pub struct Property {
    /// The contest's id, such as `Model-PT-01-LTLCardinality-03`.
    pub id: String,
    /// The formula under the outer `all-paths`, over the names in `atoms`.
    pub formula: Formula,
    /// The atoms of `formula`.
    pub atoms: Atoms,
}

/// Parses an `LTLCardinality.xml` or `LTLFireability.xml` file against the
/// net it belongs to.
pub fn properties(xml: &str, net: &Net) -> Result<Vec<Property>, String> {
    let doc = Document::parse(xml).map_err(|e| format!("properties: {e}"))?;
    let mut out = Vec::new();
    for property in doc.descendants().filter(|n| n.has_tag_name("property")) {
        let id = child(property, "id")
            .and_then(|i| i.text())
            .ok_or("property without <id>")?
            .trim()
            .to_owned();
        let formula = child(property, "formula").ok_or("property without <formula>")?;
        let mut top = elements(formula);
        let paths = match (top.next(), top.next()) {
            (Some(p), None) if p.has_tag_name("all-paths") => p,
            _ => return Err(format!("{id}: formula is not one <all-paths>")),
        };
        let mut atoms = Atoms::new();
        let body = only_child(paths).map_err(|e| format!("{id}: {e}"))?;
        let formula = ltl(body, net, &mut atoms).map_err(|e| format!("{id}: {e}"))?;
        out.push(Property { id, formula, atoms });
    }
    Ok(out)
}

fn only_child<'a, 'i>(node: XmlNode<'a, 'i>) -> Result<XmlNode<'a, 'i>, String> {
    let mut children = elements(node);
    match (children.next(), children.next()) {
        (Some(c), None) => Ok(c),
        _ => Err(format!("<{}> needs one operand", node.tag_name().name())),
    }
}

fn ltl(node: XmlNode, net: &Net, atoms: &mut Atoms) -> Result<Formula, String> {
    let mut unary = |op| -> Result<Formula, String> {
        Ok(Formula::unary(op, ltl(only_child(node)?, net, atoms)?))
    };
    let tag = node.tag_name().name();
    match tag {
        "negation" => unary(UnaryOp::Not),
        "next" => unary(UnaryOp::Next),
        "finally" => unary(UnaryOp::Eventually),
        "globally" => unary(UnaryOp::Globally),
        "conjunction" | "disjunction" => {
            let op = if tag == "conjunction" {
                BinaryOp::And
            } else {
                BinaryOp::Or
            };
            let mut operands = elements(node).map(|c| ltl(c, net, atoms));
            let first = operands.next().ok_or(format!("empty <{tag}>"))??;
            operands.try_fold(first, |acc, f| Ok(Formula::binary(op, acc, f?)))
        }
        "until" => {
            let mut side = |name| -> Result<Formula, String> {
                let s = child(node, name).ok_or(format!("<until> without <{name}>"))?;
                ltl(only_child(s)?, net, atoms)
            };
            Ok(Formula::binary(
                BinaryOp::Until,
                side("before")?,
                side("reach")?,
            ))
        }
        "true" => Ok(Formula::tt()),
        "false" => Ok(Formula::ff()),
        "is-fireable" | "integer-le" => {
            let predicate = predicate(node, net)?;
            let name = match atoms.iter().find(|(_, p)| *p == predicate) {
                Some((name, _)) => name.clone(),
                None => {
                    let name: Arc<str> = Arc::from(format!("a{}", atoms.len()));
                    atoms.push((name.clone(), predicate));
                    name
                }
            };
            Ok(Formula::atom(&*name))
        }
        _ => Err(format!("unsupported element <{tag}>")),
    }
}

fn predicate(node: XmlNode, net: &Net) -> Result<Predicate, String> {
    if node.has_tag_name("is-fireable") {
        let ts = elements(node)
            .map(|t| {
                let name = t.text().unwrap_or("").trim();
                match (t.has_tag_name("transition"), net.transition(name)) {
                    (true, Some(i)) => Ok(i),
                    _ => Err(format!("unknown transition {name:?}")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(Predicate::Fireable(ts));
    }
    let counts = elements(node)
        .map(|c| match c.tag_name().name() {
            "integer-constant" => number(c.text().unwrap_or("")).map(Count::Constant),
            "tokens-count" => elements(c)
                .map(|p| {
                    let name = p.text().unwrap_or("").trim();
                    match (p.has_tag_name("place"), net.place_index.get(name)) {
                        (true, Some(&i)) => Ok(i),
                        _ => Err(format!("unknown place {name:?}")),
                    }
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Count::Tokens),
            other => Err(format!("unsupported integer expression <{other}>")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    match <[Count; 2]>::try_from(counts) {
        Ok([l, r]) => Ok(Predicate::Le(l, r)),
        Err(_) => Err("<integer-le> needs two operands".into()),
    }
}

/// Writes `formula`, over the names in `atoms`, as a one-property MCC XML
/// file with id `id`. `W`, `R`, `->` and `<->` are expanded, since the
/// format has only `U`.
pub fn query_xml(id: &str, formula: &Formula, net: &Net, atoms: &Atoms) -> Result<String, String> {
    let mut out = String::from(
        "<?xml version=\"1.0\"?>\n<property-set xmlns=\"http://mcc.lip6.fr/\"><property>",
    );
    let _ = write!(
        out,
        "<id>{id}</id><description>GAWAIN</description><formula><all-paths>"
    );
    write_formula(formula, net, atoms, &mut out)?;
    out.push_str("</all-paths></formula></property></property-set>\n");
    Ok(out)
}

fn write_formula(f: &Formula, net: &Net, atoms: &Atoms, out: &mut String) -> Result<(), String> {
    let wrap = |tag: &str, parts: &[&Formula], out: &mut String| -> Result<(), String> {
        let _ = write!(out, "<{tag}>");
        for part in parts {
            write_formula(part, net, atoms, out)?;
        }
        let _ = write!(out, "</{tag}>");
        Ok(())
    };
    match f.node() {
        Node::True => out.push_str("<true/>"),
        Node::False => out.push_str("<false/>"),
        Node::Atom(name) => match atoms.iter().find(|(n, _)| n == name) {
            Some((_, p)) => p.write_xml(net, out),
            None => return Err(format!("atom {name} is not in the interface")),
        },
        Node::Unary(op, a) => {
            let tag = match op {
                UnaryOp::Not => "negation",
                UnaryOp::Next => "next",
                UnaryOp::Eventually => "finally",
                UnaryOp::Globally => "globally",
            };
            wrap(tag, &[a], out)?;
        }
        Node::Binary(op, a, b) => {
            let not = |x: &Formula| Formula::unary(UnaryOp::Not, x.clone());
            let and = |x, y| Formula::binary(BinaryOp::And, x, y);
            let or = |x, y| Formula::binary(BinaryOp::Or, x, y);
            let until = |x, y| Formula::binary(BinaryOp::Until, x, y);
            match op {
                BinaryOp::And => wrap("conjunction", &[a, b], out)?,
                BinaryOp::Or => wrap("disjunction", &[a, b], out)?,
                BinaryOp::Until => {
                    out.push_str("<until><before>");
                    write_formula(a, net, atoms, out)?;
                    out.push_str("</before><reach>");
                    write_formula(b, net, atoms, out)?;
                    out.push_str("</reach></until>");
                }
                BinaryOp::Implies => write_formula(&or(not(a), b.clone()), net, atoms, out)?,
                BinaryOp::Iff => {
                    let both = and(a.clone(), b.clone());
                    let neither = and(not(a), not(b));
                    write_formula(&or(both, neither), net, atoms, out)?;
                }
                BinaryOp::WeakUntil => {
                    let always = Formula::unary(UnaryOp::Globally, a.clone());
                    write_formula(&or(until(a.clone(), b.clone()), always), net, atoms, out)?;
                }
                BinaryOp::Release => {
                    write_formula(&not(&until(not(a), not(b))), net, atoms, out)?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two places and a transition that moves a token from p to q, and one
    /// that takes two tokens from q back to p.
    pub(crate) const NET: &str = r#"<?xml version="1.0"?>
<pnml xmlns="http://www.pnml.org/version-2009/grammar/pnml">
 <net id="n" type="http://www.pnml.org/version-2009/grammar/ptnet"><page id="pg">
  <place id="p"><initialMarking><text>2</text></initialMarking></place>
  <place id="q"/>
  <transition id="move"/>
  <transition id="back"/>
  <arc id="a1" source="p" target="move"/>
  <arc id="a2" source="move" target="q"/>
  <arc id="a3" source="q" target="back"><inscription><text>2</text></inscription></arc>
  <arc id="a4" source="back" target="p"><inscription><text>2</text></inscription></arc>
 </page></net></pnml>"#;

    const PROPS: &str = r#"<?xml version="1.0"?>
<property-set xmlns="http://mcc.lip6.fr/">
 <property><id>n-00</id><description>Automatically generated</description>
  <formula><all-paths><globally><until>
   <before><is-fireable><transition>move</transition></is-fireable></before>
   <reach><conjunction>
    <integer-le><integer-constant>2</integer-constant><tokens-count><place>q</place></tokens-count></integer-le>
    <next><is-fireable><transition>move</transition></is-fireable></next>
    <negation><integer-le><tokens-count><place>p</place><place>q</place></tokens-count><integer-constant>1</integer-constant></integer-le></negation>
   </conjunction></reach>
  </until></globally></all-paths></formula>
 </property>
</property-set>"#;

    #[test]
    fn a_net_fires_by_its_arc_weights() {
        let net = Net::parse(NET).unwrap();
        let mut m = net.initial().to_vec();
        assert_eq!(m, [2, 0]);
        let (mv, back) = (
            net.transition("move").unwrap(),
            net.transition("back").unwrap(),
        );
        assert!(!net.fire(&mut m, back));
        assert!(net.fire(&mut m, mv) && net.fire(&mut m, mv));
        assert_eq!(m, [0, 2]);
        assert!(!net.enabled(&m, mv) && net.enabled(&m, back));
        assert!(net.fire(&mut m, back));
        assert_eq!(m, [2, 0]);
        assert!(!net.is_deadlock(&m));
        assert!(net.is_deadlock(&[0, 1]));
    }

    #[test]
    fn coloured_nets_and_inhibitor_arcs_are_refused() {
        let coloured = NET.replace("ptnet", "symmetricnet");
        assert!(Net::parse(&coloured).unwrap_err().contains("not a P/T"));
        let inhibitor = NET.replace(
            r#"<arc id="a1" source="p" target="move"/>"#,
            r#"<arc id="a1" source="p" target="move"><type value="inhibitor"/></arc>"#,
        );
        assert!(Net::parse(&inhibitor).unwrap_err().contains("inhibitor"));
    }

    #[test]
    fn properties_share_atom_names_for_equal_predicates() {
        let net = Net::parse(NET).unwrap();
        let props = properties(PROPS, &net).unwrap();
        assert_eq!(props.len(), 1);
        let p = &props[0];
        assert_eq!(p.id, "n-00");
        let expected: Formula = "G (a0 U (a1 & X a0 & !a2))".parse().unwrap();
        assert_eq!(p.formula, expected);
        assert_eq!(p.atoms.len(), 3);
        let (m0, m1) = ([2, 0], [0, 2]);
        let holds = |i: usize, m: &[u64]| p.atoms[i].1.holds(&net, m);
        assert!(holds(0, &m0) && !holds(0, &m1));
        assert!(!holds(1, &m0) && holds(1, &m1));
        assert!(holds(2, &[1, 0]) && !holds(2, &m0));
    }

    #[test]
    fn a_query_round_trips_through_the_xml_format() {
        let net = Net::parse(NET).unwrap();
        let p = &properties(PROPS, &net).unwrap()[0];
        let xml = query_xml("q", &p.formula, &net, &p.atoms).unwrap();
        let back = &properties(&xml, &net).unwrap()[0];
        assert_eq!(back.formula, p.formula);
        assert_eq!(back.atoms, p.atoms);
        let derived: Formula = "(a0 W a1) | (a0 R !a2) | (a0 -> a1) | (a1 <-> a2)"
            .parse()
            .unwrap();
        let xml = query_xml("q", &derived, &net, &p.atoms).unwrap();
        let expanded: Formula =
            "(((a0 U a1) | G a0) | !(!a0 U !!a2)) | (!a0 | a1) | ((a1 & a2) | (!a1 & !a2))"
                .parse()
                .unwrap();
        assert_eq!(properties(&xml, &net).unwrap()[0].formula, expanded);
        let unknown = Formula::atom("zz");
        assert!(query_xml("q", &unknown, &net, &p.atoms).is_err());
    }
}
