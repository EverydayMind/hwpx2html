//! Offsets carried by the boxes they place (D36, after the logical DOM).
//!
//! In the logical DOM every drawn box keeps the chain of positioned
//! ancestors it had on its page (D35). Three links of that chain only move
//! what they hold: the body or cell margin box (`hcD`), the column box
//! (`hcI`) and the table box (`htb`), whose size nothing it holds uses. They
//! have no clip, paint or stacking of their own. This pass removes them and
//! hands their offsets to the boxes they placed, as the lengths they were
//! written with. The page's clone (clip), a float layer (stacking) and a
//! cell or text box (clip) stay.
//!
//! An absolutely positioned box sits at its containing block plus its `left`
//! plus its `margin-left` (likewise `top`), and a browser converts each of
//! those lengths to its layout unit on its own before adding them, exactly
//! as it converted the nested boxes' lengths. So a length moved from a
//! wrapper into a box's free margin leaves the box where it was, at every
//! zoom. A sum computed here would not: Chrome rounds it once instead of per
//! length (a probe of 400 chains at 90-200% zoom moved 70-95% of the boxes
//! written at a computed sum, none written with the carried lengths).
//!
//! A box has two such lengths per axis. A wrapper's offset takes a free one
//! (a margin not written, then a zero inset), or cancels one that is its
//! negation: a float layer undoes the body and column with negative lengths
//! (D28), and a length and its negation convert to opposite units.
//!
//! A cell's lines need three lengths down (the cell's margin, its vertical
//! alignment, their own top). Where lines have no room, the paragraph (or
//! heading, or empty paragraph) holding them takes the wrappers' lengths:
//! positioned where the wrappers were, it places its lines as they did. It
//! is the only case where an element of the document's structure carries
//! layout; a host page styling `p` margins would move those lines (track B
//! must scope them). Where lines sit in the chain itself (a part of a
//! paragraph crossing pages), the wrapper stays, holding its parent
//! wrapper's lengths too, so one box is left where there were two.
//!
//! Only boxes whose place and size cannot depend on the wrapper are moved:
//! boxes with their own size (lines, cells, borders), and wrappers and
//! float layers holding nothing in flow. Between a wrapper and them, only
//! elements that draw nothing may stand (`p`, headings, lists, the empty
//! paragraph's `div`). Anything else leaves the wrapper as it was.
use super::logical::{Node, Tree};

/// Classes whose base rule positions the element absolutely (`css::base_css`).
const ABSOLUTE: &[&str] = &[
    "hce", "hme", "hls", "hfS", "hcD", "hcI", "hcS", "hmB", "hmO", "hmT", "hdS", "hsC", "hsR",
    "hsG", "hsL", "hsT", "hsE", "hsA", "hsP", "hsV", "hsO", "hsU", "hpi", "hch", "hcG", "heq",
    "heG", "htA", "hvi", "htb", "htG", "hfJ", "hfG", "hfB", "hfR", "hfC", "hfO", "hfL", "hfM",
    "hfE", "hpl", "hs", "hcc",
];

/// In-flow elements that draw nothing of their own when written bare: their
/// margins and paddings are zeroed, their list markers off (`semantic_css`).
const FLOW: &[&str] = &[
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "div",
];

/// Rewrite the logical DOM `body` with its move-only boxes folded into the
/// boxes they place, or `None` when it is not markup the pass can read.
pub(super) fn fold(body: &str) -> Option<String> {
    let tree = Tree::parse(body)?;
    let count = tree.nodes.len();
    let mut parent = vec![usize::MAX; count];
    for (node, element) in tree.nodes.iter().enumerate() {
        for &child in &element.children {
            parent[child] = node;
        }
    }
    let styles = tree
        .nodes
        .iter()
        .map(|element| {
            element
                .attr("style")
                .flatten()
                .map(declarations)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    // What an element is does not change as offsets move: a box only gains
    // or loses lengths, never its `left`/`top` or its size.
    let (kinds, absolute) = tree
        .nodes
        .iter()
        .zip(&styles)
        .map(|(element, style)| classify(element, style))
        .unzip();
    let mut fold = Fold {
        tree: &tree,
        parent,
        styles,
        kinds,
        absolute,
        rewritten: vec![false; count],
        removed: vec![false; count],
    };
    fold.visit(0);
    let mut out = String::with_capacity(body.len());
    for &child in &tree.nodes[0].children {
        fold.write(child, &mut out);
    }
    Some(out)
}

type Declarations<'a> = Vec<(&'a str, &'a str)>;

fn declarations(style: &str) -> Declarations<'_> {
    style
        .split(';')
        .filter_map(|declaration| {
            let (name, value) = declaration.split_once(':')?;
            Some((name.trim(), value.trim()))
        })
        .collect()
}

fn get<'a>(declarations: &Declarations<'a>, name: &str) -> Option<&'a str> {
    declarations
        .iter()
        .find(|(key, _)| *key == name)
        .map(|&(_, value)| value)
}

fn set<'a>(declarations: &mut Declarations<'a>, name: &'a str, value: Option<&'a str>) {
    let at = declarations.iter().position(|(key, _)| *key == name);
    match (at, value) {
        (Some(at), Some(value)) => declarations[at].1 = value,
        (None, Some(value)) => declarations.push((name, value)),
        (Some(at), None) => {
            declarations.remove(at);
        }
        (None, None) => {}
    }
}

/// A written offset length: millimetres, or a bare zero.
#[derive(Clone, Copy, Debug)]
struct Length<'a> {
    text: &'a str,
    mm: f64,
}

impl<'a> Length<'a> {
    fn parse(text: &'a str) -> Option<Self> {
        let mm = if text == "0" {
            0.0
        } else {
            text.strip_suffix("mm")?.parse().ok()?
        };
        Some(Self { text, mm })
    }

    fn zero() -> Self {
        Self {
            text: "0mm",
            mm: 0.0,
        }
    }

    fn cancels(&self, other: &Self) -> bool {
        self.mm == -other.mm
    }
}

/// A box's two offset lengths on one axis.
#[derive(Clone, Copy, Debug)]
struct Axis<'a> {
    /// `left`/`top`; `None` is `auto`, the box's static position.
    inset: Option<Length<'a>>,
    /// `margin-left`/`margin-top`; `None` is not written (zero).
    margin: Option<Length<'a>>,
}

const AXES: [(&str, &str); 2] = [("left", "margin-left"), ("top", "margin-top")];

impl<'a> Axis<'a> {
    fn read(declarations: &Declarations<'a>, inset: &str, margin: &str) -> Option<Self> {
        // Not written: `Some(None)`; written but not a length we read: `None`.
        let length = |name: &str| match get(declarations, name) {
            None => Some(None),
            Some(text) => Length::parse(text).map(Some),
        };
        Some(Self {
            inset: length(inset)?,
            margin: length(margin)?,
        })
    }

    fn write(&self, declarations: &mut Declarations<'a>, inset: &'a str, margin: &'a str) {
        set(declarations, inset, self.inset.map(|length| length.text));
        set(
            declarations,
            margin,
            self.margin
                .filter(|length| length.mm != 0.0)
                .map(|length| length.text),
        );
    }

    /// The lengths this axis adds to the containing block's origin; an
    /// `auto` inset adds nothing when the box is at its static origin.
    fn terms(&self) -> Vec<Length<'a>> {
        [self.inset, self.margin]
            .into_iter()
            .flatten()
            .filter(|length| length.mm != 0.0)
            .collect()
    }

    /// Take a removed wrapper's lengths: cancel their negations, then fill
    /// the free lengths. `at_origin`: the box's static position was the
    /// wrapper's origin, so an `auto` inset becomes an explicit zero.
    fn absorb(&mut self, terms: &[Length<'a>], at_origin: bool) -> bool {
        if self.inset.is_none() && !at_origin {
            return false;
        }
        let mut pending = Vec::new();
        for term in terms {
            if self.margin.is_some_and(|m| m.cancels(term)) {
                self.margin = None;
            } else if self.inset.is_some_and(|i| i.cancels(term)) {
                self.inset = Some(Length::zero());
            } else {
                pending.push(*term);
            }
        }
        for term in pending {
            if self.inset.is_none() {
                self.inset = Some(term);
            } else if self.margin.is_none_or(|m| m.mm == 0.0) {
                self.margin = Some(term);
            } else if self.inset.is_some_and(|i| i.mm == 0.0) {
                self.inset = Some(term);
            } else {
                return false;
            }
        }
        if self.inset.is_none() {
            self.inset = Some(Length::zero());
        }
        true
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A move-only box (`hcD`, `hcI`, a table box).
    Wrapper,
    /// A box with its own size (a line, cell, border, shape).
    Sized,
    /// A float layer (D28): a bare positioned box with a `z-index`.
    Layer,
    /// An in-flow element that draws nothing.
    Flow,
    Other,
}

/// What `element` is to the pass, and whether it is out of flow.
fn classify(element: &Node<'_>, style: &Declarations<'_>) -> (Kind, bool) {
    if element.tag.is_empty() {
        return (Kind::Other, false);
    }
    let classes = element
        .attr("class")
        .flatten()
        .map(|value| {
            value
                .split(' ')
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let absolute = classes.iter().any(|class| ABSOLUTE.contains(class))
        || get(style, "position") == Some("absolute");
    let only = |attributes: &[&str]| {
        element
            .attrs
            .iter()
            .all(|(key, _)| attributes.contains(key))
    };
    let offsets_only = |extra: &[&str]| {
        style.iter().all(|&(key, _)| {
            matches!(key, "left" | "top" | "margin-left" | "margin-top") || extra.contains(&key)
        })
    };
    let boxed = matches!(element.tag, "div" | "span");
    let moves_only = match classes[..] {
        ["hcD"] | ["hcI"] => offsets_only(&[]),
        // Nothing the table box may lose uses its size: `placed` admits
        // only boxes sized on their own, and boxes empty in flow.
        ["htb"] => offsets_only(&["width", "height"]),
        _ => false,
    };
    let kind = if boxed && moves_only && only(&["class", "style"]) {
        Kind::Wrapper
    } else if classes.iter().any(|class| ABSOLUTE.contains(class))
        && get(style, "position").is_none_or(|position| position == "absolute")
        && ["left", "top", "width", "height"]
            .iter()
            .all(|name| get(style, name).is_some())
        && ["right", "bottom"]
            .iter()
            .all(|name| get(style, name).is_none())
        && !style.iter().any(|(_, value)| value.contains('%'))
    {
        Kind::Sized
    } else if boxed
        && classes.is_empty()
        && only(&["style"])
        && get(style, "position") == Some("absolute")
        && get(style, "z-index").is_some()
        && offsets_only(&["position", "z-index"])
    {
        Kind::Layer
    } else if FLOW.contains(&element.tag)
        && element.attr("class").is_none()
        && element.attr("style").is_none()
    {
        Kind::Flow
    } else {
        Kind::Other
    };
    (kind, absolute)
}

/// What a wrapper places: a box, or an in-flow element holding only boxes
/// (a paragraph and its lines), each box with whether it is first in flow.
enum Placed {
    Box(usize, bool),
    Holder(usize, Vec<(usize, bool)>),
}

struct Fold<'t, 'a> {
    tree: &'t Tree<'a>,
    parent: Vec<usize>,
    /// Every element's declarations, as written or rewritten.
    styles: Vec<Declarations<'a>>,
    kinds: Vec<Kind>,
    /// Out of flow: absolutely positioned.
    absolute: Vec<bool>,
    rewritten: Vec<bool>,
    /// Removed wrappers: their children take their place.
    removed: Vec<bool>,
}

impl<'a> Fold<'_, 'a> {
    /// Nothing in flow gives the box a size: its width stays zero whatever
    /// its containing block.
    fn empty_flow(&self, node: usize) -> bool {
        self.tree.nodes[node].children.iter().all(|&child| {
            !self.tree.nodes[child].tag.is_empty()
                && (self.absolute[child]
                    || (self.kinds[child] == Kind::Flow && self.empty_flow(child)))
        })
    }

    /// A box `placed` admits: sized on its own, or empty in flow.
    fn is_box(&self, node: usize) -> bool {
        match self.kinds[node] {
            Kind::Sized => true,
            Kind::Wrapper | Kind::Layer => self.empty_flow(node),
            _ => false,
        }
    }

    /// What `wrapper` places, each box with whether its static position is
    /// the wrapper's origin (first in flow). `None` when anything else stands
    /// in the way or a box could change size.
    fn placed(&self, wrapper: usize) -> Option<Vec<Placed>> {
        let mut out = Vec::new();
        self.collect(wrapper, true, &mut out)?;
        Some(out)
    }

    fn collect(&self, node: usize, first: bool, out: &mut Vec<Placed>) -> Option<()> {
        for (index, &child) in self.tree.nodes[node].children.iter().enumerate() {
            let first = first && index == 0;
            if self.is_box(child) {
                out.push(Placed::Box(child, first));
                continue;
            }
            if self.kinds[child] != Kind::Flow {
                return None;
            }
            let held = &self.tree.nodes[child].children;
            if !held.is_empty() && held.iter().all(|&box_| self.is_box(box_)) {
                let boxes = held
                    .iter()
                    .enumerate()
                    .map(|(index, &box_)| (box_, first && index == 0))
                    .collect();
                out.push(Placed::Holder(child, boxes));
            } else {
                self.collect(child, first, out)?;
            }
        }
        Some(())
    }

    fn visit(&mut self, node: usize) {
        if self.kinds[node] == Kind::Wrapper && self.dissolve(node) {
            self.removed[node] = true;
        }
        let tree = self.tree;
        for &child in &tree.nodes[node].children {
            self.visit(child);
        }
    }

    /// Whether a first child's static position is its container's origin:
    /// a block's is; an inline box's follows its line's alignment, which
    /// cannot move it inside a container without width.
    fn starts_at_origin(&self, node: usize, container: usize) -> bool {
        self.tree.nodes[node].tag == "div" || get(&self.styles[container], "width").is_none()
    }

    /// Hand `wrapper`'s offsets to the boxes it places, if all have room.
    fn dissolve(&mut self, wrapper: usize) -> bool {
        let parent = self.parent[wrapper];
        let at_origin = parent != usize::MAX
            && !self.removed[parent]
            && self.tree.nodes[parent].children.first() == Some(&wrapper)
            && self.absolute[parent]
            && self.starts_at_origin(wrapper, parent);
        let mut terms = Vec::new();
        for (inset, margin) in AXES {
            let Some(axis) = Axis::read(&self.styles[wrapper], inset, margin) else {
                return false;
            };
            if axis.inset.is_none() && !at_origin {
                return false;
            }
            terms.push(axis.terms());
        }
        let Some(placed) = self.placed(wrapper) else {
            return false;
        };
        let mut rewritten = Vec::with_capacity(placed.len());
        let mut holders = Vec::new();
        for item in placed {
            match item {
                Placed::Box(node, first) => {
                    let Some(declarations) = self.absorbed(node, first, wrapper, &terms) else {
                        return false;
                    };
                    rewritten.push((node, declarations));
                }
                Placed::Holder(holder, boxes) => {
                    let absorbed = boxes
                        .iter()
                        .map(|&(node, first)| {
                            Some((node, self.absorbed(node, first, wrapper, &terms)?))
                        })
                        .collect::<Option<Vec<_>>>();
                    match absorbed {
                        Some(boxes) => rewritten.extend(boxes),
                        // The lines have no room (a cell's margin, its vertical
                        // alignment and their own top): the paragraph takes
                        // the offsets, where the wrapper was, and places them.
                        None if boxes.iter().all(|&(node, _)| self.explicit(node)) => {
                            holders.push(holder);
                        }
                        None => return false,
                    }
                }
            }
        }
        for (node, declarations) in rewritten {
            self.styles[node] = declarations;
            self.rewritten[node] = true;
        }
        for holder in holders {
            let mut declarations = vec![("position", "absolute")];
            for ((inset, margin), terms) in AXES.into_iter().zip(&terms) {
                let mut axis = Axis {
                    inset: None,
                    margin: None,
                };
                // A wrapper has at most two lengths per axis: they fit.
                let fits = axis.absorb(terms, true);
                debug_assert!(fits);
                axis.write(&mut declarations, inset, margin);
            }
            self.styles[holder] = declarations;
            self.rewritten[holder] = true;
            self.kinds[holder] = Kind::Other;
            self.absolute[holder] = true;
        }
        true
    }

    /// `node`'s declarations with `terms` (a removed wrapper's lengths per
    /// axis) taken, or `None` when it has no room.
    fn absorbed(
        &self,
        node: usize,
        first: bool,
        wrapper: usize,
        terms: &[Vec<Length<'a>>],
    ) -> Option<Declarations<'a>> {
        let first = first && self.starts_at_origin(node, wrapper);
        let mut declarations = self.styles[node].clone();
        for ((inset, margin), terms) in AXES.into_iter().zip(terms) {
            let mut axis = Axis::read(&declarations, inset, margin)?;
            if !axis.absorb(terms, first) {
                return None;
            }
            axis.write(&mut declarations, inset, margin);
        }
        Some(declarations)
    }

    /// Placed by its own `left` and `top`, not its static position.
    fn explicit(&self, node: usize) -> bool {
        let style = &self.styles[node];
        get(style, "left").is_some() && get(style, "top").is_some()
    }

    fn write(&self, node: usize, out: &mut String) {
        let element = &self.tree.nodes[node];
        if !self.removed[node] {
            if self.rewritten[node] {
                start_tag(element.tag, &element.attrs, &self.styles[node], out);
            } else {
                out.push_str(element.raw);
            }
        }
        for &child in &element.children {
            self.write(child, out);
        }
        if !self.removed[node] {
            out.push_str(element.end);
        }
    }
}

/// A start tag with its style replaced, or added last (after `class`, as
/// `styles_to_classes` expects).
fn start_tag(
    tag: &str,
    attrs: &[(&str, Option<&str>)],
    style: &Declarations<'_>,
    out: &mut String,
) {
    let style = style
        .iter()
        .map(|(key, value)| format!("{key}:{value};"))
        .collect::<String>();
    out.push('<');
    out.push_str(tag);
    let mut written = false;
    for &(key, value) in attrs {
        out.push(' ');
        out.push_str(key);
        let value = if key == "style" {
            written = true;
            Some(style.as_str())
        } else {
            value
        };
        if let Some(value) = value {
            out.push_str("=\"");
            out.push_str(value);
            out.push('"');
        }
    }
    if !written {
        out.push_str(" style=\"");
        out.push_str(&style);
        out.push('"');
    }
    out.push('>');
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "<div data-page=\"1\" style=\"position:absolute;left:8.546875px;top:0px;width:210mm;height:297mm;overflow:hidden;\">";

    fn line(top: &str) -> String {
        format!("<span class=\"hls ps0\" style=\"line-height:2.79mm;left:0mm;top:{top};height:3.53mm;width:150mm;\"><span class=\"hrt cs0\">a</span></span>")
    }

    #[test]
    fn body_and_column_move_into_the_lines_margins() {
        let content = format!(
            "<p>{}{}</p><div data-hwpx-empty>{}</div>",
            line("-0.18mm"),
            line("5.47mm"),
            line("11.11mm")
        );
        let body = format!(
            "{ROOT}<div class=\"hcD\" style=\"left:30mm;top:35mm;\"><div class=\"hcI\">{content}</div></div></div>"
        );
        // Only the wrappers go; each line carries their offset as its margin.
        let carried = content.replace(
            "width:150mm;",
            "width:150mm;margin-left:30mm;margin-top:35mm;",
        );
        assert_eq!(fold(&body).unwrap(), format!("{ROOT}{carried}</div>"));
    }

    #[test]
    fn a_float_layer_undoing_the_wrappers_loses_both() {
        let body = format!(
            "{ROOT}<div class=\"hcD\" style=\"left:25mm;top:28.70mm;\"><div class=\"hcI\"><p>{}</p><div style=\"position:absolute;left:0mm;top:0mm;margin-left:-25mm;margin-top:-28.70mm;z-index:1;\"><div class=\"htb\" style=\"left:26mm;top:42.68mm;width:160mm;height:224.96mm;\"><svg class=\"hs\" style=\"left:-2.50mm;top:-2.50mm;width:165mm;height:229.96mm;\"></svg></div></div></div></div></div>",
            line("0mm")
        );
        let folded = fold(&body).unwrap();
        assert!(
            !folded.contains("hcD") && !folded.contains("hcI"),
            "{folded}"
        );
        // The layer is left at the page's origin with its stacking order; the
        // table box's offset is in its border's margins.
        assert!(
            folded.contains("<div style=\"position:absolute;left:0mm;top:0mm;z-index:1;\"><svg class=\"hs\" style=\"left:-2.50mm;top:-2.50mm;width:165mm;height:229.96mm;margin-left:26mm;margin-top:42.68mm;\"></svg></div>"),
            "{folded}"
        );
    }

    #[test]
    fn a_cell_with_three_offsets_moves_them_onto_its_paragraph() {
        // Cell margin, vertical alignment and the line's own top: a line has
        // room for two lengths per axis, so its paragraph takes the wrappers'
        // and places the line where they did.
        let hce = "<div class=\"hce\" style=\"left:0mm;top:0mm;width:79mm;height:8.52mm;\">";
        let body = format!(
            "{ROOT}{hce}<div class=\"hcD\" style=\"left:0.50mm;top:0.50mm;\"><div class=\"hcI\" style=\"top:1.29mm;\"><p>{}</p><div data-hwpx-empty>{}</div></div></div></div></div>",
            line("-0.25mm"),
            line("5mm")
        );
        assert_eq!(
            fold(&body).unwrap(),
            format!(
                "{ROOT}{hce}<p style=\"position:absolute;left:0.50mm;top:1.29mm;margin-top:0.50mm;\">{}</p><div data-hwpx-empty style=\"position:absolute;left:0.50mm;top:1.29mm;margin-top:0.50mm;\">{}</div></div></div>",
                line("-0.25mm"),
                line("5mm")
            )
        );
        // A part of a paragraph crossing pages holds its lines in the chain
        // itself: no paragraph to take them, so one wrapper stays with both.
        let body = format!(
            "{ROOT}{hce}<span class=\"hcD\" style=\"left:0.50mm;top:0.50mm;\"><span class=\"hcI\" style=\"top:1.29mm;\">{}</span></span></div></div>",
            line("-0.25mm")
        );
        assert_eq!(
            fold(&body).unwrap(),
            format!(
                "{ROOT}{hce}<span class=\"hcI\" style=\"top:1.29mm;left:0.50mm;margin-top:0.50mm;\">{}</span></div></div>",
                line("-0.25mm")
            )
        );
    }

    #[test]
    fn a_cells_chain_keeps_its_clips_and_its_layer() {
        let cell = format!(
            "<div class=\"hce\" style=\"left:0mm;top:0mm;width:79mm;height:8.52mm;\"><div class=\"hcD\" style=\"left:2mm;top:0mm;\"><div class=\"hcI\" style=\"top:1.29mm;\"><p>{}</p></div></div></div>",
            line("-0.25mm")
        );
        let body = format!(
            "{ROOT}<div class=\"hcD\" style=\"left:25mm;top:28.70mm;\"><div class=\"hcI\"><div style=\"position:absolute;left:0mm;top:0mm;margin-left:-25mm;margin-top:-28.70mm;z-index:1;\"><div class=\"htb\" style=\"left:26mm;top:42.68mm;width:160mm;height:224.96mm;\">{cell}</div></div></div></div></div>"
        );
        // The page's clone, the layer and the cell box stay; the table box's
        // offset moves into the cell box's margins, the cell's margin box's
        // into its lines (two lengths on each axis fit).
        assert_eq!(
            fold(&body).unwrap(),
            format!(
                "{ROOT}<div style=\"position:absolute;left:0mm;top:0mm;z-index:1;\"><div class=\"hce\" style=\"left:0mm;top:0mm;width:79mm;height:8.52mm;margin-left:26mm;margin-top:42.68mm;\"><p>{}</p></div></div></div>",
                line("-0.25mm").replace(
                    "top:-0.25mm;height:3.53mm;width:150mm;",
                    "top:-0.25mm;height:3.53mm;width:150mm;margin-left:2mm;margin-top:1.29mm;"
                )
            )
        );
        // A table box holding a table (an inline table's) keeps its size.
        let inline = "<div class=\"htb\" style=\"left:1mm;top:1mm;width:10mm;height:5mm;\"><table></table></div>";
        let body = format!("{ROOT}{inline}</div>");
        assert_eq!(fold(&body).unwrap(), body);
    }

    #[test]
    fn a_wrapper_holding_anything_else_stays() {
        for inner in [
            "<p>text outside a line</p>".to_owned(),
            "<img class=\"hpi\" style=\"left:0mm;top:0mm;\">".to_owned(),
            // No width: its size could follow the containing block.
            "<p><span class=\"hls\" style=\"left:0mm;top:0mm;\">a</span></p>".to_owned(),
        ] {
            let body = format!(
                "{ROOT}<div class=\"hcD\" style=\"left:30mm;top:35mm;\">{inner}</div></div>"
            );
            assert_eq!(fold(&body).unwrap(), body);
        }
    }

    #[test]
    fn a_static_position_is_carried_only_from_the_origin() {
        // The second column (`left` auto) follows a sibling: its static
        // position is not the wrapper's origin, so neither moves.
        let body = format!(
            "{ROOT}<div class=\"hcD\" style=\"left:30mm;top:35mm;\"><div class=\"hcI\" style=\"top:0mm;\"><p>{}</p></div><div class=\"hcI\"><p>{}</p></div></div></div>",
            line("0mm"),
            line("0mm")
        );
        let folded = fold(&body).unwrap();
        assert!(
            folded.contains("<div class=\"hcD\" style=\"left:30mm;top:35mm;\">"),
            "{folded}"
        );
        // The first column's own wrapper had no offset: it goes.
        assert_eq!(folded.matches("class=\"hcI\"").count(), 1, "{folded}");
    }
}
