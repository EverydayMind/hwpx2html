//! One logical DOM over the pages (D35, track C of the semantic plan).
//!
//! The renderer writes every page as a box (`div.hpa`) holding what is drawn
//! on it, so a paragraph, table or list crossing a page is written once per
//! page, the parts linked by `data-hwpx-id`/`data-hwpx-part`. This pass
//! rebuilds that markup so a source paragraph is one `p`, a table one
//! `table`, a split cell one `td`, a split list one `ul`/`ol`:
//!
//! * the pages stay as empty sheets of paper (`div.hpa[data-page]`),
//!   absolutely positioned where the flow put them before;
//! * the logical tree follows in `main`;
//! * every drawn box keeps the chain of positioned ancestors it had (page,
//!   body, column, float layer, table box, cell box). The chain is cloned
//!   around the box -- `span`s inside phrasing content, never between
//!   `table`/`tr`/`td` or `ul`/`li` -- and the page's clone is a clipping
//!   box at the page's origin, marked with its `data-page`.
//!
//! A browser positions a box against its positioned ancestor and snaps each
//! offset on its own, so a cloned chain keeps every box exactly where it was.
//! The page origins are Chrome's flow positions of the old pages, computed:
//! each length snapped down to 1/64 px (verified on all 1,043 sample pages).
//! A repeated header row, drawn again on each later page, becomes a
//! drawn-only copy outside the table whose text is generated content.
//!
//! Anything the pass does not expect leaves the markup as it was.
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::units::{px, snap_mm, split, UNIT};

/// Rebuild `html` (a whole document) as one logical DOM, or return it
/// unchanged when its body is not the renderer's page markup.
pub(super) fn rebuild(html: &str) -> String {
    try_rebuild(html).unwrap_or_else(|| html.to_owned())
}

fn try_rebuild(html: &str) -> Option<String> {
    let open = html.find("<body>")? + "<body>".len();
    let close = html.rfind("</body>")?;
    let body = &html[open..close];
    let tree = Tree::parse(body)?;
    let mut builder = Builder {
        tree: &tree,
        items: vec![Item::new(NONE, Rc::from(Vec::new()), Kind::Root, "")],
        order: Vec::new(),
        pages: Vec::new(),
    };
    let content = tree.nodes[0]
        .children
        .iter()
        .copied()
        .filter(|&child| tree.nodes[child].tag != "script")
        .collect::<Vec<_>>();
    let root_path: Rc<[usize]> = Rc::from(Vec::new());
    for child in content {
        builder.child(child, &root_path, ROOT)?;
    }
    if builder.pages.is_empty() {
        return None;
    }
    let Builder {
        mut items,
        order,
        pages,
        ..
    } = builder;
    let merged = merge(&tree, &mut items, &order);
    unwrap_lines(&mut items, ROOT);
    for &item in &merged {
        finish_parts(&mut items[item]);
    }
    let (frames, end) = page_frames(&tree, &pages)?;
    let mut emitter = Emitter {
        tree: &tree,
        items: &items,
        frames: &frames,
        page_of: pages
            .iter()
            .enumerate()
            .map(|(index, &node)| (node, index))
            .collect(),
        anchors: vec![None; items.len()],
        out: String::with_capacity(body.len() + body.len() / 2),
        generated: false,
    };
    for (index, frame) in frames.iter().enumerate() {
        emitter.out.push_str("<div class=\"hpa\" data-page=\"");
        emitter.out.push_str(&(index + 1).to_string());
        emitter.out.push_str("\" style=\"");
        emitter.out.push_str(&frame.paper);
        emitter.out.push_str("\"></div>");
    }
    // The pages left the flow; the logical tree keeps the document as tall as
    // they made it (`main` starts below the body's top padding).
    emitter.out.push_str("<main style=\"height:");
    emitter.out.push_str(&px(end - snap_mm(2.0)));
    emitter.out.push_str(";\">");
    let root_children = items[ROOT].children.clone();
    emitter.children(&root_children, &root_path, true, false)?;
    emitter.out.push_str("</main>");
    for &child in &tree.nodes[0].children {
        if tree.nodes[child].tag == "script" {
            tree.serialize(child, &mut emitter.out);
        }
    }
    // D36: the chains' move-only boxes go, their offsets carried.
    let rebuilt = super::offsets::fold(&emitter.out).unwrap_or(emitter.out);
    let mut result = String::with_capacity(html.len() + rebuilt.len() - body.len());
    result.push_str(&html[..open]);
    result.push_str(&rebuilt);
    result.push_str(&html[close..]);
    Some(result)
}

// ------------------------------------------------------------------- parsing

const VOID: &[&str] = &[
    "meta", "link", "img", "br", "input", "col", "hr", "wbr", "source",
];

/// An element or a text run of the renderer's own markup, kept as written.
pub(super) struct Node<'a> {
    /// Empty for text.
    pub(super) tag: &'a str,
    /// The start tag as written, or the text.
    pub(super) raw: &'a str,
    /// The end tag as written; empty for text and void elements.
    pub(super) end: &'a str,
    pub(super) attrs: Vec<(&'a str, Option<&'a str>)>,
    pub(super) children: Vec<usize>,
}

impl<'a> Node<'a> {
    pub(super) fn attr(&self, name: &str) -> Option<Option<&'a str>> {
        self.attrs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|&(_, value)| value)
    }

    fn has(&self, name: &str) -> bool {
        self.attr(name).is_some()
    }

    fn has_class(&self, class: &str) -> bool {
        self.attr("class")
            .flatten()
            .is_some_and(|value| value.split(' ').any(|c| c == class))
    }
}

pub(super) struct Tree<'a> {
    pub(super) nodes: Vec<Node<'a>>,
}

impl<'a> Tree<'a> {
    /// Parse markup the renderer wrote: every attribute value in double
    /// quotes, every non-void element closed, no comments.
    pub(super) fn parse(source: &'a str) -> Option<Self> {
        let mut nodes = vec![Node {
            tag: "#root",
            raw: "",
            end: "",
            attrs: Vec::new(),
            children: Vec::new(),
        }];
        let mut stack = vec![0_usize];
        let mut pos = 0;
        while pos < source.len() {
            let lt = source[pos..].find('<').map_or(source.len(), |i| pos + i);
            if lt > pos {
                let id = nodes.len();
                nodes.push(Node {
                    tag: "",
                    raw: &source[pos..lt],
                    end: "",
                    attrs: Vec::new(),
                    children: Vec::new(),
                });
                nodes[*stack.last()?].children.push(id);
            }
            if lt >= source.len() {
                break;
            }
            if source[lt..].starts_with("</") {
                let gt = lt + source[lt..].find('>')?;
                let open = stack.pop()?;
                if open == 0 || nodes[open].tag != &source[lt + 2..gt] {
                    return None;
                }
                nodes[open].end = &source[lt..=gt];
                pos = gt + 1;
                continue;
            }
            let (tag, attrs, end, self_closing) = parse_start_tag(source, lt)?;
            let id = nodes.len();
            nodes.push(Node {
                tag,
                raw: &source[lt..end],
                end: "",
                attrs,
                children: Vec::new(),
            });
            nodes[*stack.last()?].children.push(id);
            pos = end;
            if self_closing || VOID.contains(&tag) {
                continue;
            }
            if tag == "script" || tag == "style" {
                let close = format!("</{tag}>");
                let stop = pos + source[pos..].find(&close)?;
                if stop > pos {
                    let text = nodes.len();
                    nodes.push(Node {
                        tag: "",
                        raw: &source[pos..stop],
                        end: "",
                        attrs: Vec::new(),
                        children: Vec::new(),
                    });
                    nodes[id].children.push(text);
                }
                nodes[id].end = &source[stop..stop + close.len()];
                pos = stop + close.len();
                continue;
            }
            stack.push(id);
        }
        (stack.len() == 1).then_some(Self { nodes })
    }

    fn serialize(&self, node: usize, out: &mut String) {
        let node = &self.nodes[node];
        out.push_str(node.raw);
        for &child in &node.children {
            self.serialize(child, out);
        }
        out.push_str(node.end);
    }

    /// Like `serialize`, but an element holding only text draws it as
    /// generated content (`data-gen`): seen, and absent from the DOM text.
    fn serialize_generated(&self, node: usize, out: &mut String) {
        let element = &self.nodes[node];
        let texts = element
            .children
            .iter()
            .filter(|&&child| self.nodes[child].tag.is_empty())
            .count();
        if texts > 0
            && texts == element.children.len()
            && !element.tag.is_empty()
            && !element.has("data-gen")
        {
            let close = if element.raw.ends_with("/>") { 2 } else { 1 };
            out.push_str(&element.raw[..element.raw.len() - close]);
            out.push_str(" data-gen=\"");
            for &child in &element.children {
                out.push_str(self.nodes[child].raw);
            }
            out.push('"');
            out.push_str(&element.raw[element.raw.len() - close..]);
            out.push_str(element.end);
            return;
        }
        out.push_str(element.raw);
        for &child in &element.children {
            self.serialize_generated(child, out);
        }
        out.push_str(element.end);
    }
}

type StartTag<'a> = (&'a str, Vec<(&'a str, Option<&'a str>)>, usize, bool);

/// The name, attributes, end offset and self-closing flag of the start tag
/// at `at`.
fn parse_start_tag(source: &str, at: usize) -> Option<StartTag<'_>> {
    let bytes = source.as_bytes();
    let name_start = at + 1;
    let mut pos = name_start;
    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'-') {
        pos += 1;
    }
    if pos == name_start {
        return None;
    }
    let tag = &source[name_start..pos];
    let mut attrs = Vec::new();
    loop {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        match bytes.get(pos)? {
            b'>' => return Some((tag, attrs, pos + 1, false)),
            b'/' if bytes.get(pos + 1) == Some(&b'>') => return Some((tag, attrs, pos + 2, true)),
            _ => {}
        }
        let key_start = pos;
        while pos < bytes.len() && !matches!(bytes[pos], b'=' | b'>' | b'/' | b' ' | b'\t' | b'\n')
        {
            pos += 1;
        }
        let key = &source[key_start..pos];
        if key.is_empty() {
            return None;
        }
        if bytes.get(pos) == Some(&b'=') {
            if bytes.get(pos + 1) != Some(&b'"') {
                return None;
            }
            let value_start = pos + 2;
            let value_end = value_start + source[value_start..].find('"')?;
            attrs.push((key, Some(&source[value_start..value_end])));
            pos = value_end + 1;
        } else {
            attrs.push((key, None));
        }
    }
}

// ------------------------------------------------------------ logical tree

const NONE: usize = usize::MAX;
const ROOT: usize = 0;

/// What an element is to the logical tree.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    /// A page: the first box of every chain.
    Page,
    /// A positioned box without meaning of its own (body, column, float
    /// layer, table box): transparent, cloned into chains.
    Geometry,
    /// Both a logical element and a positioned box (`td.hce`, a box table
    /// and its cells): split into the element and a cloned box.
    Hybrid,
    /// A line holding an inline table, part of a split paragraph.
    BlockLine,
    Logical,
    /// Copied as written.
    Atomic,
    Text,
}

fn role(node: &Node<'_>) -> Role {
    if node.tag.is_empty() {
        return Role::Text;
    }
    match node.tag {
        "div" if node.has_class("hpa") => Role::Page,
        "div" if node.has_class("hcD") || node.has_class("hcI") => Role::Geometry,
        // A float layer (D28): a bare positioned div with a z-index.
        "div"
            if node.attrs.len() == 1
                && node
                    .attr("style")
                    .flatten()
                    .is_some_and(|style| style.contains("z-index:")) =>
        {
            Role::Geometry
        }
        "div" if node.has_class("htb") => {
            if node.has("data-hwpx-table") {
                Role::Hybrid
            } else {
                Role::Geometry
            }
        }
        "td" | "th" if node.has_class("hce") => Role::Hybrid,
        "div" if node.has_class("hce") => Role::Hybrid,
        "div" if node.has_class("hls") && node.has("data-hwpx-part") => Role::BlockLine,
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "ul" | "ol" | "li" | "table" | "thead"
        | "tbody" | "tr" | "td" | "th" => Role::Logical,
        "div" if node.has("data-hwpx-empty") => Role::Logical,
        _ => Role::Atomic,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Root,
    Text,
    Atomic,
    Logical,
    Hybrid,
    /// A `div` joining the block lines of one paragraph.
    Lines,
    /// The drawn-only copy of a repeated header row.
    Copy,
}

/// Rows of a table being joined: the logical rows in order, and for each
/// split cell its first row and the last row it covers.
#[derive(Default)]
struct TableState<'a> {
    rows: Vec<usize>,
    row_of: HashMap<&'a str, usize>,
    end_of: HashMap<&'a str, usize>,
}

struct Item<'a> {
    node: usize,
    /// The positioned ancestors (nodes) the item sat in.
    path: Rc<[usize]>,
    kind: Kind,
    tag: &'a str,
    /// Attributes of a rewritten start tag.
    attrs: Vec<(&'a str, Option<Cow<'a, str>>)>,
    children: Vec<usize>,
    parent: usize,
    /// Its subtree was changed: written element by element, not copied.
    dirty: bool,
    /// Split element: the kind of element its parts are, and the source id.
    key: Option<(&'static str, &'a str)>,
    /// Its part count ("i/n"), and how many parts it now holds.
    parts: usize,
    frags: usize,
    table: Option<Box<TableState<'a>>>,
}

impl<'a> Item<'a> {
    fn new(node: usize, path: Rc<[usize]>, kind: Kind, tag: &'a str) -> Self {
        Self {
            node,
            path,
            kind,
            tag,
            attrs: Vec::new(),
            children: Vec::new(),
            parent: NONE,
            dirty: false,
            key: None,
            parts: 1,
            frags: 1,
            table: None,
        }
    }
}

struct Builder<'t, 'a> {
    tree: &'t Tree<'a>,
    items: Vec<Item<'a>>,
    order: Vec<usize>,
    pages: Vec<usize>,
}

fn extend(path: &Rc<[usize]>, node: usize) -> Rc<[usize]> {
    let mut next = path.to_vec();
    next.push(node);
    Rc::from(next)
}

fn add(items: &mut [Item<'_>], parent: usize, item: usize, at: Option<usize>) {
    items[item].parent = parent;
    match at {
        Some(index) => items[parent].children.insert(index, item),
        None => items[parent].children.push(item),
    }
}

fn detach(items: &mut [Item<'_>], item: usize) {
    let parent = items[item].parent;
    if parent == NONE {
        return;
    }
    if let Some(index) = items[parent].children.iter().position(|&c| c == item) {
        items[parent].children.remove(index);
    }
    items[item].parent = NONE;
}

fn index_in_parent(items: &[Item<'_>], item: usize) -> Option<usize> {
    let parent = items[item].parent;
    (parent != NONE)
        .then(|| items[parent].children.iter().position(|&c| c == item))
        .flatten()
}

impl<'a> Builder<'_, 'a> {
    fn push(&mut self, item: Item<'a>, parent: usize) -> usize {
        let id = self.items.len();
        self.items.push(item);
        add(&mut self.items, parent, id, None);
        id
    }

    fn child(&mut self, node: usize, path: &Rc<[usize]>, parent: usize) -> Option<()> {
        let tree = self.tree;
        let element = &tree.nodes[node];
        match role(element) {
            Role::Page => {
                self.pages.push(node);
                let path = extend(path, node);
                for &child in &element.children {
                    self.child(child, &path, parent)?;
                }
            }
            Role::Geometry => {
                let path = extend(path, node);
                for &child in &element.children {
                    self.child(child, &path, parent)?;
                }
            }
            role @ (Role::Hybrid | Role::Logical) => {
                let kind = if role == Role::Hybrid {
                    Kind::Hybrid
                } else {
                    Kind::Logical
                };
                let mut item = Item::new(node, path.clone(), kind, element.tag);
                item.attrs = element
                    .attrs
                    .iter()
                    .map(|&(key, value)| (key, value.map(Cow::Borrowed)))
                    .collect();
                if let (Some(Some(id)), Some(Some(part))) =
                    (element.attr("data-hwpx-id"), element.attr("data-hwpx-part"))
                {
                    let name = match element.tag {
                        "table" => "table",
                        "td" | "th" => "cell",
                        "div" if element.has_class("hce") => "cell",
                        "div" if element.has_class("htb") => "box",
                        "div" => "empty",
                        "ul" | "ol" => "list",
                        "li" => "item",
                        _ => "paragraph",
                    };
                    item.key = Some((name, id));
                    item.parts = part.split_once('/')?.1.parse().ok()?;
                }
                let id = self.push(item, parent);
                self.order.push(id);
                let inner = if role == Role::Hybrid {
                    extend(path, node)
                } else {
                    path.clone()
                };
                for &child in &element.children {
                    self.child(child, &inner, id)?;
                }
            }
            Role::BlockLine => {
                let mut lines = Item::new(NONE, path.clone(), Kind::Lines, "div");
                let id = element.attr("data-hwpx-id").flatten()?;
                lines.key = Some(("line", id));
                lines.attrs = vec![("data-hwpx-id", Some(Cow::Borrowed(id)))];
                let lines = self.push(lines, parent);
                self.order.push(lines);
                self.push(
                    Item::new(node, path.clone(), Kind::Atomic, element.tag),
                    lines,
                );
            }
            Role::Atomic => {
                self.push(
                    Item::new(node, path.clone(), Kind::Atomic, element.tag),
                    parent,
                );
            }
            Role::Text => {
                self.push(Item::new(node, path.clone(), Kind::Text, ""), parent);
            }
        }
        Some(())
    }
}

// ------------------------------------------------------------------- joining

fn mark_dirty(items: &mut [Item<'_>], mut item: usize) {
    while item != NONE && !items[item].dirty {
        items[item].dirty = true;
        item = items[item].parent;
    }
}

fn attr<'b>(item: &'b Item<'_>, name: &str) -> Option<Option<&'b str>> {
    item.attrs
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_deref())
}

fn table_rows(items: &[Item<'_>], table: usize) -> Vec<usize> {
    items[table]
        .children
        .iter()
        .filter(|&&group| matches!(items[group].tag, "thead" | "tbody"))
        .flat_map(|&group| items[group].children.iter().copied())
        .filter(|&row| items[row].tag == "tr")
        .collect()
}

fn real_cells(items: &[Item<'_>], row: usize) -> Vec<usize> {
    items[row]
        .children
        .iter()
        .copied()
        .filter(|&cell| {
            matches!(items[cell].tag, "td" | "th") && attr(&items[cell], "hidden").is_none()
        })
        .collect()
}

fn rowspan(item: &Item<'_>) -> usize {
    attr(item, "rowspan")
        .flatten()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1)
}

fn cell_key<'a>(item: &Item<'a>) -> Option<&'a str> {
    item.key.map(|(_, id)| id)
}

fn init_table(items: &mut [Item<'_>], table: usize) {
    let rows = table_rows(items, table);
    let mut state = TableState::default();
    for (index, &row) in rows.iter().enumerate() {
        for cell in real_cells(items, row) {
            if let Some(key) = cell_key(&items[cell]) {
                state.row_of.insert(key, index);
                state.end_of.insert(key, index + rowspan(&items[cell]) - 1);
            }
        }
    }
    state.rows = rows;
    items[table].table = Some(Box::new(state));
}

/// Move `other`'s rows into `first`. A repeated header row becomes a
/// drawn-only copy before the table; a first row made only of continuing
/// cells is the last row of `first` continued. The fills and borders of
/// `other`'s table box paint under its cells: they go before the table.
fn join_table(items: &mut Vec<Item<'_>>, first: usize, other: usize) -> Option<()> {
    if items[first].table.is_none() {
        init_table(items, first);
    }
    let body = *items[first]
        .children
        .iter()
        .rev()
        .find(|&&group| matches!(items[group].tag, "thead" | "tbody"))?;
    let rows = table_rows(items, other);
    let mut before = index_in_parent(items, first)?;
    let target_parent = items[first].parent;
    let parent = items[other].parent;
    let mut at = index_in_parent(items, other)?;
    let mut decorations = Vec::new();
    while at > 0 {
        let sibling = items[parent].children[at - 1];
        if items[sibling].kind == Kind::Atomic && items[sibling].path == items[other].path {
            decorations.insert(0, sibling);
            at -= 1;
        } else {
            break;
        }
    }
    for decoration in decorations {
        detach(items, decoration);
        add(items, target_parent, decoration, Some(before));
        before += 1;
    }
    let mut seen = items[first]
        .table
        .as_ref()?
        .row_of
        .keys()
        .copied()
        .collect::<HashSet<_>>();
    let mut body_rows = 0;
    for row in rows {
        if attr(&items[row], "data-hwpx-repeat").is_some() {
            let copy = items.len();
            let mut item = Item::new(NONE, Rc::from(Vec::new()), Kind::Copy, "div");
            item.dirty = true;
            item.attrs = vec![
                ("aria-hidden", Some(Cow::Borrowed("true"))),
                ("data-hwpx-repeat", None),
            ];
            items.push(item);
            for cell in items[row].children.clone() {
                for child in items[cell].children.clone() {
                    detach(items, child);
                    add(items, copy, child, None);
                }
            }
            add(items, target_parent, copy, Some(before));
            before += 1;
            detach(items, row);
            continue;
        }
        let cells = real_cells(items, row);
        let continued = body_rows == 0
            && !cells.is_empty()
            && cells
                .iter()
                .all(|&cell| cell_key(&items[cell]).is_some_and(|key| seen.contains(key)));
        body_rows += 1;
        detach(items, row);
        let state = items[first].table.as_mut()?;
        let index = if continued {
            state.rows.len().checked_sub(1)?
        } else {
            state.rows.push(row);
            state.rows.len() - 1
        };
        if !continued {
            add(items, body, row, None);
        }
        for cell in cells {
            let end = index + rowspan(&items[cell]) - 1;
            let Some(key) = cell_key(&items[cell]) else {
                continue;
            };
            let state = items[first].table.as_mut()?;
            if seen.contains(key) {
                let last = state.end_of.entry(key).or_insert(end);
                *last = (*last).max(end);
            } else {
                state.row_of.insert(key, index);
                state.end_of.insert(key, end);
                seen.insert(key);
            }
        }
    }
    detach(items, other);
    Some(())
}

/// Join the list items and lists around two parts of one paragraph.
fn lift_parents(items: &mut [Item<'_>], first: usize, other: usize) {
    let mut pairs = Vec::new();
    let (mut a, mut b) = (items[first].parent, items[other].parent);
    while a != NONE
        && b != NONE
        && a != b
        && items[a].kind == Kind::Logical
        && items[b].kind == Kind::Logical
        && items[a].tag == items[b].tag
        && matches!(items[a].tag, "li" | "ul" | "ol")
    {
        pairs.push((a, b));
        a = items[a].parent;
        b = items[b].parent;
    }
    for (keep, drop) in pairs.into_iter().rev() {
        if items[drop].parent == NONE {
            continue;
        }
        for child in items[drop].children.clone() {
            detach(items, child);
            add(items, keep, child, None);
        }
        detach(items, drop);
        mark_dirty(items, keep);
    }
}

fn is_decoration(tree: &Tree<'_>, item: &Item<'_>) -> bool {
    item.kind == Kind::Atomic && tree.nodes[item.node].tag == "svg"
}

/// Join the consecutive parts of every split element written as the same
/// kind of element. A part of another kind (a paragraph's block line next
/// to its `p` parts) starts a new run: joining across it would reorder the
/// text. Returns the elements that now hold several parts.
fn merge(tree: &Tree<'_>, items: &mut Vec<Item<'_>>, order: &[usize]) -> Vec<usize> {
    let mut runs = HashMap::<&str, (&str, usize)>::new();
    let mut merged = Vec::new();
    for &item in order {
        let Some((name, id)) = items[item].key else {
            continue;
        };
        let first = match runs.get(id) {
            Some(&(run, first)) if run == name && items[first].parent != NONE => first,
            _ => {
                runs.insert(id, (name, item));
                continue;
            }
        };
        items[first].frags += 1;
        if items[first].frags == 2 {
            merged.push(first);
        }
        if items[item].tag == "table" {
            if join_table(items, first, item).is_none() {
                continue;
            }
        } else {
            if items[item].parent != NONE && items[item].parent != items[first].parent {
                lift_parents(items, first, item);
            }
            let hybrid = items[first].kind == Kind::Hybrid;
            let mut leading = 0;
            if hybrid {
                while leading < items[first].children.len()
                    && is_decoration(tree, &items[items[first].children[leading]])
                {
                    leading += 1;
                }
            }
            for child in items[item].children.clone() {
                detach(items, child);
                if hybrid && is_decoration(tree, &items[child]) {
                    add(items, first, child, Some(leading));
                    leading += 1;
                } else {
                    add(items, first, child, None);
                }
            }
            detach(items, item);
        }
        mark_dirty(items, first);
    }
    // Cells that now span their continuation rows; rows and groups emptied.
    for &table in &merged {
        let Some(state) = items[table].table.take() else {
            continue;
        };
        for (index, &row) in state.rows.iter().enumerate() {
            for cell in items[row].children.clone() {
                let Some(key) = cell_key(&items[cell]) else {
                    continue;
                };
                if state.row_of.get(key) != Some(&index) {
                    continue;
                }
                let span = state.end_of[key] + 1 - index;
                let attrs = &mut items[cell].attrs;
                attrs.retain(|(name, _)| *name != "rowspan");
                if span > 1 {
                    let at = attrs
                        .iter()
                        .position(|(name, _)| *name == "colspan")
                        .unwrap_or(0);
                    attrs.insert(at, ("rowspan", Some(Cow::Owned(span.to_string()))));
                }
            }
        }
        for group in items[table].children.clone() {
            if matches!(items[group].tag, "thead" | "tbody") {
                for row in items[group].children.clone() {
                    if items[row].tag == "tr" && items[row].children.is_empty() {
                        detach(items, row);
                    }
                }
                if items[group].children.is_empty() {
                    detach(items, group);
                }
            }
        }
    }
    merged
}

/// Whole again: no part attributes. Still missing parts (a block line not
/// joined to its `p`): keep the link.
fn finish_parts(item: &mut Item<'_>) {
    if item.kind == Kind::Lines {
        return;
    }
    if item.frags >= item.parts {
        item.attrs
            .retain(|(name, _)| !matches!(*name, "data-hwpx-id" | "data-hwpx-part"));
    }
}

/// A block line that was not joined needs no wrapper.
fn unwrap_lines(items: &mut [Item<'_>], item: usize) {
    for (index, child) in items[item].children.clone().into_iter().enumerate() {
        if items[child].kind == Kind::Lines && items[child].frags == 1 {
            let inner = items[child].children[0];
            items[inner].parent = item;
            items[item].children[index] = inner;
        } else if !matches!(items[child].kind, Kind::Atomic | Kind::Text) {
            unwrap_lines(items, child);
        }
    }
}

// ------------------------------------------------------------------ geometry

fn style_mm<'s>(style: &'s str, property: &str) -> Option<(&'s str, f64)> {
    // `width:210mm` -> ("210mm", 210.0)
    let start = style
        .split(';')
        .find_map(|declaration| declaration.strip_prefix(property)?.strip_prefix(':'))?;
    Some((start, start.strip_suffix("mm")?.parse().ok()?))
}

/// The styles placing a page: its sheet of paper and the clone that starts
/// every chain drawn on it.
struct Frame {
    paper: String,
    clone: String,
}

/// Where the old flow put the pages: the body's 2mm top padding, then per
/// page a 1px border, the height, a 1px border and a 2mm margin; 2mm from the
/// left. On paper (print) the pages follow each other without gaps. Also
/// returns where the flow ended: the document's height.
fn page_frames(tree: &Tree<'_>, pages: &[usize]) -> Option<(Vec<Frame>, i64)> {
    let gap = snap_mm(2.0);
    let mut screen = gap;
    let mut paper_top = 0;
    let mut frames = Vec::with_capacity(pages.len());
    for &page in pages {
        let style = tree.nodes[page].attr("style").flatten()?;
        let (_, width) = style_mm(style, "width")?;
        let (_, height) = style_mm(style, "height")?;
        let (print_high, print_low) = split(paper_top);
        // The one-page view centres the page as `margin:auto` did: from the
        // width as laid out, not the millimetres.
        let print = format!(
            "--pt:{print_high};--pm:{print_low};--w:{};",
            px(snap_mm(width))
        );
        let (high, low) = split(screen);
        let paper = format!(
            "{style}position:absolute;margin:0;left:{};top:{high};margin-top:{low};{print}",
            px(gap)
        );
        let (high, low) = split(screen + UNIT);
        let clone = format!(
            "position:absolute;left:{};top:{high};margin-top:{low};{style}overflow:hidden;{print}",
            px(gap + UNIT)
        );
        frames.push(Frame { paper, clone });
        let height = snap_mm(height);
        screen += UNIT + height + UNIT + gap;
        paper_top += height;
    }
    Some((frames, screen))
}

// ------------------------------------------------------------------ writing

const PHRASING: &[&str] = &["p", "h1", "h2", "h3", "h4", "h5", "h6"];
/// Elements whose children may not be wrapped.
const NO_WRAP: &[&str] = &["table", "thead", "tbody", "tr", "ul", "ol"];

fn common_prefix(a: &[usize], b: &[usize]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

struct Emitter<'t, 'a> {
    tree: &'t Tree<'a>,
    items: &'t [Item<'a>],
    frames: &'t [Frame],
    page_of: HashMap<usize, usize>,
    anchors: Vec<Option<Rc<[usize]>>>,
    out: String,
    /// Inside a drawn-only copy: text becomes generated content.
    generated: bool,
}

impl Emitter<'_, '_> {
    /// The chain shared by everything drawn inside `item`.
    fn anchor(&mut self, item: usize) -> Rc<[usize]> {
        if let Some(anchor) = &self.anchors[item] {
            return anchor.clone();
        }
        let mut result: Option<Rc<[usize]>> = None;
        for &child in &self.items[item].children {
            let path = match self.items[child].kind {
                Kind::Text => continue,
                Kind::Atomic => self.items[child].path.clone(),
                _ => self.anchor(child),
            };
            result = Some(match result {
                None => path,
                Some(current) => {
                    let n = common_prefix(&current, &path);
                    if n == current.len() {
                        current
                    } else {
                        Rc::from(&current[..n])
                    }
                }
            });
        }
        let anchor = result.unwrap_or_else(|| self.items[item].path.clone());
        self.anchors[item] = Some(anchor.clone());
        anchor
    }

    fn open(&mut self, node: usize, phrasing: bool) -> &'static str {
        let tag = if phrasing { "span" } else { "div" };
        self.out.push('<');
        self.out.push_str(tag);
        if let Some(&page) = self.page_of.get(&node) {
            self.out.push_str(" data-page=\"");
            self.out.push_str(&(page + 1).to_string());
            self.out.push_str("\" style=\"");
            self.out.push_str(&self.frames[page].clone);
            self.out.push('"');
        } else {
            for &(key, value) in &self.tree.nodes[node].attrs {
                if let ("class" | "style", Some(value)) = (key, value) {
                    self.out.push(' ');
                    self.out.push_str(key);
                    self.out.push_str("=\"");
                    self.out.push_str(value);
                    self.out.push('"');
                }
            }
        }
        self.out.push('>');
        tag
    }

    fn children(
        &mut self,
        list: &[usize],
        base: &Rc<[usize]>,
        allow: bool,
        phrasing: bool,
    ) -> Option<()> {
        let items = self.items;
        let tree = self.tree;
        let mut open: Vec<(usize, &'static str)> = Vec::new();
        for &item in list {
            let current = &items[item];
            if current.kind == Kind::Text {
                if self.generated && !current.node_text(tree).trim().is_empty() {
                    return None;
                }
                self.out.push_str(current.node_text(tree));
                continue;
            }
            let mut intact = current.kind == Kind::Atomic || !current.dirty;
            let mut target = if intact {
                current.path.clone()
            } else if allow {
                self.anchor(item)
            } else {
                base.clone()
            };
            if !target.starts_with(base) {
                target = base.clone();
            }
            if !allow && target[..] != base[..] {
                if current.kind == Kind::Atomic {
                    return None;
                }
                intact = false;
                target = base.clone();
            }
            let depth = base.len() + open.len();
            let chain = base.iter().chain(open.iter().map(|(node, _)| node));
            let keep = chain
                .zip(target.iter())
                .take_while(|(a, b)| a == b)
                .count()
                .max(base.len());
            for _ in keep..depth {
                let (_, tag) = open.pop()?;
                self.out.push_str("</");
                self.out.push_str(tag);
                self.out.push('>');
            }
            for &node in &target[keep..] {
                let tag = self.open(node, phrasing);
                open.push((node, tag));
            }
            if intact {
                if self.generated {
                    tree.serialize_generated(current.node, &mut self.out);
                } else {
                    tree.serialize(current.node, &mut self.out);
                }
            } else {
                self.logical(item, &target)?;
            }
        }
        while let Some((_, tag)) = open.pop() {
            self.out.push_str("</");
            self.out.push_str(tag);
            self.out.push('>');
        }
        Some(())
    }

    fn logical(&mut self, item: usize, base: &Rc<[usize]>) -> Option<()> {
        let items = self.items;
        let current = &items[item];
        let tag = current.tag;
        self.out.push('<');
        self.out.push_str(tag);
        let hybrid = current.kind == Kind::Hybrid;
        let mut cell = false;
        for (key, value) in &current.attrs {
            if hybrid && matches!(*key, "class" | "style") {
                continue;
            }
            self.out.push(' ');
            self.out.push_str(key);
            if let Some(value) = value {
                self.out.push_str("=\"");
                self.out.push_str(value);
                self.out.push('"');
            }
            cell |= *key == "data-hwpx-cell";
        }
        // A box table's cell, apart from its box: say which cell it is.
        if hybrid && tag == "div" && self.tree.nodes[current.node].has_class("hce") && !cell {
            self.out.push_str(" data-hwpx-cell=\"0,0,1,1\"");
        }
        self.out.push('>');
        let copy = current.kind == Kind::Copy;
        let was = self.generated;
        self.generated |= copy;
        self.children(
            &current.children,
            base,
            !NO_WRAP.contains(&tag),
            PHRASING.contains(&tag),
        )?;
        self.generated = was;
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
        Some(())
    }
}

impl Item<'_> {
    fn node_text<'t>(&self, tree: &'t Tree<'_>) -> &'t str {
        tree.nodes[self.node].raw
    }
}

/// Style rules of the logical DOM: the logical tree lays out nothing of its
/// own (its boxes are the chains, placed against the page) and only keeps
/// the document as tall as before; on paper each page and its chains move to
/// the page's printed place.
///
/// Every chain on a page starts with a transparent clone as large as the
/// page, and a cell's chains each clone its cell box: the last one written
/// lies over the others and would take the pointer from the text under it,
/// so a drag there selected nothing. Only the lines and pictures take the
/// pointer; every box around them lets it through.
pub fn logical_css() -> &'static str {
    "main {height:0;overflow:hidden;}\nmain td, main th {padding:0;}\nmain {pointer-events:none;}\nmain .hls, main img {pointer-events:auto;}\n@media print {[data-page] {left:0 !important;top:var(--pt) !important;margin-top:var(--pm) !important;}\nmain {height:0 !important;}}\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::units::CHUNK;

    fn page(content: &str) -> String {
        format!("<div class=\"hpa\" style=\"width:210mm;height:296.99mm;\"><div class=\"hcD\" style=\"left:30mm;top:35mm;\"><div class=\"hcI\">{content}</div></div></div>")
    }

    fn document(pages: &[String]) -> String {
        format!(
            "<!DOCTYPE html>\n<html><head></head><body>{}<script>x()</script></body></html>",
            pages.concat()
        )
    }

    #[test]
    fn origins_follow_the_old_flow_in_exact_layout_units() {
        assert_eq!(snap_mm(2.0), 483);
        assert_eq!(px(483), "7.546875px");
        assert_eq!(px(snap_mm(296.99)), "1122.46875px");
        assert_eq!(
            split(CHUNK + 5),
            ("65536px".to_owned(), "0.078125px".to_owned())
        );
    }

    #[test]
    fn a_paragraph_across_pages_is_one_p_with_a_chain_per_page() {
        let line = |top: &str, text: &str| {
            format!("<span class=\"hls\" style=\"left:0mm;top:{top};height:4mm;width:150mm;\">{text}</span>")
        };
        let html = document(&[
            page(&format!(
                "<p data-hwpx-id=\"s0/p[1]\" data-hwpx-part=\"1/2\">{}</p>",
                line("0mm", "a")
            )),
            page(&format!(
                "<p data-hwpx-id=\"s0/p[1]\" data-hwpx-part=\"2/2\">{}</p><p>{}</p>",
                line("0mm", "b"),
                line("5mm", "c")
            )),
        ]);
        let rebuilt = rebuild(&html);
        let body = &rebuilt[rebuilt.find("<main").unwrap()..];
        // The document stays as tall as the pages' flow made it.
        assert!(
            body.starts_with("<main style=\"height:2264.03125px;\">"),
            "{body}"
        );
        assert_eq!(body.matches("<p").count(), 2, "{body}");
        assert!(!body.contains("data-hwpx-part"), "{body}");
        let p = &body[body.find("<p>").unwrap()..body.find("</p>").unwrap()];
        assert_eq!(p.matches("data-page=").count(), 2, "{p}");
        // Each part's chain is its page's clone, then the line, which carries
        // the body's offset in its margins (D36).
        assert!(
            p.contains("\"><span class=\"hls\" style=\"left:0mm;top:0mm;height:4mm;width:150mm;margin-left:30mm;margin-top:35mm;\">b</span></span>"),
            "{p}"
        );
        assert!(!p.contains("hcD") && !p.contains("hcI"), "{p}");
        assert!(!p.contains("<div"), "{p}");
        // The sheets of paper stay, empty, and the scripts still end the body.
        assert_eq!(rebuilt.matches("<div class=\"hpa\" data-page=").count(), 2);
        assert!(rebuilt.ends_with("</main><script>x()</script></body></html>"));
    }

    #[test]
    fn a_table_across_pages_is_one_table_and_its_repeated_header_is_drawn_only() {
        let cell = |attrs: &str, text: &str| {
            format!("<td class=\"hce\" style=\"left:0mm;\"{attrs}><div class=\"hcD\"><div class=\"hcI\"><p><span class=\"hls\"><span class=\"hrt\">{text}</span></span></p></div></div></td>")
        };
        let table = |part: &str, rows: &str| {
            format!("<div style=\"position:absolute;z-index:1;\"><div class=\"htb\" style=\"left:1mm;\"><svg class=\"hs\" aria-hidden=\"true\"><path d=\"M0,0\"></path></svg><table data-hwpx-id=\"s0/t\" data-hwpx-part=\"{part}\">{rows}</table></div></div>")
        };
        let html = document(&[
            page(&table(
                "1/2",
                &format!(
                    "<thead><tr>{}</tr></thead><tbody><tr>{}</tr></tbody>",
                    cell("", "head"),
                    cell(" data-hwpx-id=\"s0/t/r1c0\" data-hwpx-part=\"1/2\"", "one")
                ),
            )),
            page(&table(
                "2/2",
                &format!(
                    "<thead><tr data-hwpx-repeat>{}</tr></thead><tbody><tr>{}</tr><tr>{}</tr></tbody>",
                    cell("", "head"),
                    cell(" data-hwpx-id=\"s0/t/r1c0\" data-hwpx-part=\"2/2\"", "more"),
                    cell("", "two")
                ),
            )),
        ]);
        let rebuilt = rebuild(&html);
        let body = &rebuilt[rebuilt.find("<main").unwrap()..];
        assert_eq!(body.matches("<table").count(), 1, "{body}");
        assert_eq!(body.matches("<tr").count(), 3, "{body}");
        assert_eq!(body.matches("<td").count(), 3, "{body}");
        assert!(!body.contains("data-hwpx-part"), "{body}");
        // The repeated header is drawn from generated content, before the table.
        let copy = body.find("data-hwpx-repeat").unwrap();
        assert!(copy < body.find("<table").unwrap());
        assert!(
            body.contains("<span class=\"hrt\" data-gen=\"head\"></span>"),
            "{body}"
        );
        assert_eq!(body.matches(">head<").count(), 1, "{body}");
        // Both pages' borders paint before the cells.
        assert!(body.rfind("<svg").unwrap() < body.find("<table").unwrap());
        // No wrapper between the table's own elements.
        assert!(body.contains("<tbody><tr><td"), "{body}");
        assert!(body.contains("<td><div data-page=\"1\""), "{body}");
    }

    #[test]
    fn only_lines_and_pictures_take_the_pointer() {
        let css = logical_css();
        // The chains' boxes lie over each other; none of them may take it.
        assert!(css.contains("main {pointer-events:none;}"), "{css}");
        assert!(
            css.contains("main .hls, main img {pointer-events:auto;}"),
            "{css}"
        );
        assert!(!css.contains("[data-page]:not(.hpa) {pointer"), "{css}");
    }

    #[test]
    fn markup_it_does_not_expect_is_left_alone() {
        let html = "<!DOCTYPE html>\n<html><head></head><body><p>a</body></html>";
        assert_eq!(rebuild(html), html);
    }
}
