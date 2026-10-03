//! Document-structure helpers for the HTML renderer.
//!
//! Every box keeps its absolute page geometry; these helpers decide which
//! elements carry it: a source paragraph's lines are grouped into one `p`, a
//! table's cells into `table`/`tr`/`td`. Pages stay the containers (D20), so
//! a paragraph or table that continues on another page (or column) becomes
//! several elements. Those parts are linked with
//! `data-hwpx-id`/`data-hwpx-part`.
use std::collections::HashMap;

use super::bundle::RenderContext;
use super::html::escape_html_attribute;
use crate::model::{LineFragment, ParaHeading, Paragraph, Table, TableCell, Token};

const MARK: char = '\u{1}';
const SEPARATOR: char = '\u{2}';

/// A placeholder for the part attributes of `key`, resolved by
/// [`finalize_parts`] once every part of the document is known. Neither
/// control character can occur in source text or ids (XML 1.0 forbids both).
pub(super) fn part_marker(kind: char, key: &str) -> String {
    format!("{MARK}{kind}{SEPARATOR}{key}{MARK}")
}

/// Replace every part marker: an element whose key occurs once gets nothing,
/// one of several parts gets `data-hwpx-id` and its `index/count`.
pub(super) fn finalize_parts(html: &str) -> String {
    let mut counts = HashMap::<&str, usize>::new();
    let mut rest = html;
    while let Some(start) = rest.find(MARK) {
        let Some(length) = rest[start + 1..].find(MARK) else {
            break;
        };
        *counts
            .entry(&rest[start + 1..start + 1 + length])
            .or_default() += 1;
        rest = &rest[start + length + 2..];
    }
    if counts.is_empty() {
        return html.to_owned();
    }
    let mut seen = HashMap::<&str, usize>::new();
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(MARK) {
        let Some(length) = rest[start + 1..].find(MARK) else {
            break;
        };
        output.push_str(&rest[..start]);
        let marker = &rest[start + 1..start + 1 + length];
        let count = counts[marker];
        if count > 1 {
            let index = seen.entry(marker).or_default();
            *index += 1;
            let key = marker.split_once(SEPARATOR).map_or(marker, |(_, key)| key);
            output.push_str(&format!(
                " data-hwpx-id=\"{}\" data-hwpx-part=\"{}/{count}\"",
                escape_html_attribute(key),
                index
            ));
        }
        rest = &rest[start + length + 2..];
    }
    output.push_str(rest);
    output
}

/// Which element carries a paragraph (D25): a `p`, a heading, or a list item
/// (a `p` inside `li` inside `ul`/`ol`). The source's own structure
/// (`hh:heading`) always applies. Markers such as 제1장, □ and ○ apply only
/// when inference is on, and the elements they produce say so with
/// `data-inferred`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Paragraph,
    /// A paragraph without text: inside an open list it stays in the item.
    Blank,
    /// A note (`*`, `※`) under a list item: inside an open list it stays in
    /// the item. Inferred only.
    Note,
    Heading {
        level: u32,
        inferred: bool,
    },
    Item {
        ordered: bool,
        level: u32,
        inferred: bool,
    },
}

const HEADINGS: [&str; 6] = ["h1", "h2", "h3", "h4", "h5", "h6"];

fn role(heading: Option<ParaHeading>, start: &str, infer: bool) -> Role {
    match heading {
        Some(ParaHeading::Outline(level)) => Role::Heading {
            level: (level + 1).min(6),
            inferred: false,
        },
        Some(ParaHeading::Number(level)) => Role::Item {
            ordered: true,
            level,
            inferred: false,
        },
        Some(ParaHeading::Bullet(level)) => Role::Item {
            ordered: false,
            level,
            inferred: false,
        },
        None => {
            let text = start.trim_start();
            if text.is_empty() {
                Role::Blank
            } else if infer && text.starts_with(['*', '※', '→']) {
                // A note, or an arrow line drawing a consequence from the
                // item above it (0713AI 도입: "□ 보안 : …" then "→ …").
                Role::Note
            } else if infer {
                inferred_role(text).unwrap_or(Role::Paragraph)
            } else {
                Role::Paragraph
            }
        }
    }
}

/// Structure implied by the marker opening a paragraph in Korean public
/// documents; the markers themselves are `crate::semantic::markers`.
fn inferred_role(text: &str) -> Option<Role> {
    if let Some(level) = crate::semantic::markers::chapter_level(text) {
        return Some(Role::Heading {
            level,
            inferred: true,
        });
    }
    crate::semantic::markers::marker_rank(text).map(|level| Role::Item {
        ordered: false,
        level,
        inferred: true,
    })
}

/// Whether a font family is sans-serif (고딕, 돋움, 굴림, 헤드라인 ...);
/// 바탕, 명조 and 궁서 are serif.
fn sans_serif(family: &str) -> bool {
    const SANS: &[&str] = &[
        "고딕",
        "돋움",
        "굴림",
        "헤드라인",
        "gothic",
        "dotum",
        "gulim",
        "sans",
        "arial",
        "helvetica",
        "verdana",
        "tahoma",
        "segoe",
    ];
    let family = family.to_lowercase();
    SANS.iter().any(|name| family.contains(name))
}

/// How a paragraph's first visible run looks: the signals of the outline
/// rules. `size` is in hundredths of a point.
#[derive(Clone, Copy, Default, Debug)]
struct Look {
    size: i64,
    sans: bool,
    bold: bool,
}

fn look(document: &RenderContext<'_>, line: &LineFragment) -> Look {
    look_of(document, &line.tokens)
}

fn look_of(document: &RenderContext<'_>, tokens: &[Token]) -> Look {
    tokens
        .iter()
        .find(|token| !token.visible_text().trim().is_empty())
        .and_then(|token| {
            document
                .char_styles
                .iter()
                .find(|style| style.id == token.char_style_id)
        })
        .map_or_else(Look::default, |style| Look {
            size: style.font_size_hwp,
            sans: sans_serif(&style.font_family),
            bold: style.bold,
        })
}

/// The only paragraph with text of a one-row table: a title box's text.
pub(super) fn title_paragraph(table: &Table) -> Option<&Paragraph> {
    let has_text = |paragraph: &&Paragraph| {
        paragraph
            .tokens
            .iter()
            .any(|token| !token.visible_text().trim().is_empty())
    };
    if table.rows != 1 {
        return None;
    }
    let mut texts = table
        .cells
        .iter()
        .flat_map(|cell| &cell.paragraphs)
        .filter(has_text);
    let first = texts.next()?;
    texts.next().is_none().then_some(first)
}

/// Decide a top-level paragraph's role in reading order (the outline
/// pre-pass of a page), unless its first line already did.
pub(super) fn outline_paragraph(document: &RenderContext<'_>, line: &LineFragment) {
    if line.heading.is_some() {
        return;
    }
    let mut outline = document.outline.borrow_mut();
    if outline.decided.contains_key(&line.paragraph_key) {
        return;
    }
    let marker_role = role(None, &line.paragraph_start, true);
    let role = outline.role(&line.paragraph_start, look(document, line), marker_role);
    outline.decided.insert(line.paragraph_key.clone(), role);
}

/// Decide a title box's role where the page reads it (the outline
/// pre-pass): after the paragraph it floats from, or within its line.
pub(super) fn outline_title(document: &RenderContext<'_>, table: &Table) {
    let Some(paragraph) = title_paragraph(table) else {
        return;
    };
    let mut outline = document.outline.borrow_mut();
    if outline.decided.contains_key(&paragraph.key) {
        return;
    }
    let role = outline.title_role(look_of(document, &paragraph.tokens));
    outline.decided.insert(paragraph.key.clone(), role);
}

/// A one-row bar inline in a line (the outline pre-pass): decide it as a
/// title box, and let its host paragraph -- blank but for the bar -- end
/// the lists before it instead of staying in their last item. The bar is
/// a table of one row whose only text is one paragraph ([`title_paragraph`],
/// the title box's shape at any size): 산업단지 유공자 공고's "2. 포상
/// 개요" ~ "5. 신청(추천)방법" are 15pt 휴먼명조 bars (so not title boxes)
/// that had ended up inside the list items above them. Across the 57
/// samples no other table moved.
pub(super) fn outline_inline_title(
    document: &RenderContext<'_>,
    line: &LineFragment,
    table: &Table,
) {
    outline_title(document, table);
    if title_paragraph(table).is_none() {
        return;
    }
    let mut outline = document.outline.borrow_mut();
    if outline.decided.get(&line.paragraph_key) == Some(&Role::Blank) {
        outline
            .decided
            .insert(line.paragraph_key.clone(), Role::Paragraph);
    }
}

/// A document title: sans-serif, 16pt or more, alone in a one-row table.
const TITLE_SIZE: i64 = 1600;
/// A section heading (sans-serif, bold, not indented) and the marker
/// headings under it: 15pt or more.
const HEADING_SIZE: i64 = 1500;

/// The top-level outline the inferred headings build across the pages.
#[derive(Default)]
pub(super) struct Outline {
    /// The level of the section heading marker headings nest under.
    base: Option<u32>,
    /// The marker headings open under it, outermost first: marker rank and
    /// heading level.
    chain: Vec<(u32, u32)>,
    /// Roles already decided, by paragraph key: a paragraph continued on
    /// another page or column keeps the role its first line got.
    decided: HashMap<String, Role>,
    /// Whether any heading came before: only a title box before every other
    /// heading is the document's `h1`.
    headed: bool,
}

impl Outline {
    fn role(&mut self, text: &str, look: Look, marker_role: Role) -> Role {
        let role = self.classify(text, look, marker_role);
        self.headed |= matches!(role, Role::Heading { .. });
        role
    }

    /// The role of a title box's text (the only text of a one-row table,
    /// sans-serif at 16pt or more): the document's `h1` when it comes before
    /// every other heading (0713AI 도입's title), otherwise a section
    /// heading like a bold sans-serif one (성과보고서's title bars "1. 임무와
    /// 비전" ...).
    fn title_role(&mut self, look: Look) -> Role {
        if !(look.sans && look.size >= TITLE_SIZE) {
            return Role::Paragraph;
        }
        let level = if self.headed {
            self.base = Some(2);
            self.chain.clear();
            2
        } else {
            1
        };
        self.headed = true;
        Role::Heading {
            level,
            inferred: true,
        }
    }

    /// The role of a top-level paragraph (D25, 2026-09-26 outline rules):
    /// a sans-serif, bold, 15pt or larger paragraph without indent is a
    /// section heading (h2); under it a 15pt or larger □, ○, - and ·
    /// paragraph is a heading one level below the nearest heading of a
    /// higher marker (□ h3, ㅇ h4, - h5). Without a section heading
    /// markers stay list items.
    fn classify(&mut self, text: &str, look: Look, marker_role: Role) -> Role {
        let body = text.trim_start();
        if body.is_empty() {
            return marker_role;
        }
        let indented = body.len() != text.len();
        if !indented && look.sans && look.bold && look.size >= HEADING_SIZE {
            self.base = Some(2);
            self.chain.clear();
            return Role::Heading {
                level: 2,
                inferred: true,
            };
        }
        if let Role::Heading { level, .. } = marker_role {
            self.base = Some(level);
            self.chain.clear();
            return marker_role;
        }
        let (Some(base), Some(rank)) = (self.base, crate::semantic::markers::marker_rank(body))
        else {
            return marker_role;
        };
        if look.size < HEADING_SIZE {
            return marker_role;
        }
        while self.chain.last().is_some_and(|&(open, _)| open >= rank) {
            self.chain.pop();
        }
        let level = (self.chain.last().map_or(base, |&(_, level)| level) + 1).min(6);
        self.chain.push((rank, level));
        Role::Heading {
            level,
            inferred: true,
        }
    }
}

/// A list open around the paragraph being written, with an open `li`.
#[derive(Clone)]
struct OpenList {
    ordered: bool,
    level: u32,
    inferred: bool,
    /// Part ids: the list's (from its first item) and its open item's. A
    /// list or item continued in the next column is written again there
    /// with the same id, a further part of it.
    list: String,
    item: String,
}

/// The lists a column of the page flow ended inside, and the paragraph it
/// ended with: the next column's first paragraph may continue them.
pub(super) struct OpenLists {
    lists: Vec<OpenList>,
    paragraph: String,
}

/// Groups consecutive lines of one source paragraph into its element.
pub(super) struct ParagraphGroup {
    /// False inside an inline text box: its lines are already inside a
    /// `p`, which cannot contain another.
    wrap: bool,
    /// False for a repeated header copy: its paragraphs are not parts.
    link: bool,
    /// Whether markers may imply structure here: top-level text only.
    infer: bool,
    /// The cells of a title box: their roles come from the page's outline
    /// pre-pass.
    title: bool,
    /// The paragraph whose lines are being written, and its role.
    current: Option<(String, Role)>,
    /// The element open for its phrasing lines (`p`, `hN`, or the `div` of
    /// an empty paragraph).
    open: Option<&'static str>,
    /// The lists open around it, innermost last.
    lists: Vec<OpenList>,
    /// The lists the previous column ended in, until the first paragraph.
    resume: Option<OpenLists>,
}

impl ParagraphGroup {
    pub(super) fn new(document: &RenderContext<'_>, infer: bool) -> Self {
        Self {
            wrap: !document.phrasing.get(),
            link: true,
            infer,
            title: false,
            current: None,
            open: None,
            lists: Vec::new(),
            resume: None,
        }
    }

    /// A column of the page flow: it continues the lists the previous
    /// column ended in when its first paragraph belongs in them.
    pub(super) fn continuing(mut self, document: &RenderContext<'_>) -> Self {
        self.resume = document.open_lists.take();
        self
    }

    pub(super) fn unlinked(mut self) -> Self {
        self.link = false;
        self
    }

    /// The group of a title box's cell ([`title_paragraph`]): its text is
    /// the `h1` or a section heading as the outline pre-pass decided.
    pub(super) fn title(mut self) -> Self {
        self.title = true;
        self
    }

    /// A paragraph's role, decided on its first line.
    fn decide(&self, document: &RenderContext<'_>, line: &LineFragment) -> Role {
        let marker_role = role(line.heading, &line.paragraph_start, self.infer);
        if self.title {
            // Decided by the page's outline pre-pass, in reading order.
            return document
                .outline
                .borrow()
                .decided
                .get(&line.paragraph_key)
                .copied()
                .unwrap_or(marker_role);
        }
        if !self.infer || line.heading.is_some() {
            return marker_role;
        }
        let mut outline = document.outline.borrow_mut();
        if let Some(&role) = outline.decided.get(&line.paragraph_key) {
            return role;
        }
        let role = outline.role(&line.paragraph_start, look(document, line), marker_role);
        outline.decided.insert(line.paragraph_key.clone(), role);
        role
    }

    /// Write one line through `render`. A `phrasing` line joins its
    /// paragraph's open element; any other line (one carrying an inline
    /// table) stays a block outside it, still linked to its paragraph.
    pub(super) fn line(
        &mut self,
        html: &mut String,
        document: &RenderContext<'_>,
        line: &LineFragment,
        phrasing: bool,
        render: impl FnOnce(&mut String),
    ) {
        if !self.wrap {
            render(html);
            return;
        }
        let key = line.paragraph_key.as_str();
        let role = match &self.current {
            Some((current, role)) if current == key => *role,
            _ => {
                let role = self.decide(document, line);
                if line.page_break {
                    // After the author's page break the lists start afresh.
                    self.resume = None;
                }
                self.start(html, key, role);
                role
            }
        };
        if phrasing {
            if self.open.is_none() {
                // A paragraph that only holds space is not a paragraph to a
                // reader: a `div` marked as the source's empty paragraph.
                let tag = match role {
                    _ if line.paragraph_empty => "div",
                    Role::Heading { level, .. } => HEADINGS[level as usize - 1],
                    _ => "p",
                };
                html.push('<');
                html.push_str(tag);
                if line.paragraph_empty {
                    html.push_str(" data-hwpx-empty");
                } else if matches!(role, Role::Heading { inferred: true, .. }) {
                    html.push_str(" data-inferred=\"heading\"");
                }
                if self.link {
                    html.push_str(&part_marker('p', key));
                }
                html.push('>');
                self.open = Some(tag);
            }
            let start = html.len();
            document.phrasing.set(true);
            render(html);
            document.phrasing.set(false);
            to_phrasing(html, start);
        } else {
            self.close_element(html);
            let start = html.len();
            render(html);
            if self.link {
                if let Some(end) = html[start..].find('>') {
                    html.insert_str(start + end, &part_marker('p', key));
                }
            }
        }
    }

    /// Begin a paragraph: open, continue or close the lists around it.
    fn start(&mut self, html: &mut String, key: &str, role: Role) {
        self.close_element(html);
        if let Some(open) = self.resume.take() {
            if self.reopen(html, key, role, open) {
                self.current = Some((key.to_owned(), role));
                return;
            }
        }
        match role {
            Role::Item {
                ordered,
                level,
                inferred,
            } => {
                while self.lists.last().is_some_and(|list| list.level > level) {
                    self.close_list(html);
                }
                match self.lists.last() {
                    Some(list) if list.level == level && list.ordered == ordered => {
                        html.push_str("</li>");
                    }
                    Some(list) if list.level == level => {
                        self.close_list(html);
                        self.open_list(html, key, ordered, level, inferred);
                    }
                    // A deeper list opens inside the enclosing item.
                    _ => self.open_list(html, key, ordered, level, inferred),
                }
                self.open_item(html, key);
            }
            Role::Blank | Role::Note if !self.lists.is_empty() => {}
            _ => self.close_lists(html),
        }
        self.current = Some((key.to_owned(), role));
    }

    /// Write again, as further parts, the lists the previous column ended
    /// in that the first paragraph of this one belongs in: all of them for
    /// the rest of the paragraph it ended with, a blank or a note, and those
    /// an item continues (shallower, or its own level and kind) for an item.
    /// A new section starts afresh, as does the author's page break (see
    /// [`Self::line`]). The logical DOM joins the parts (D35).
    /// True when this began the paragraph as well.
    fn reopen(&mut self, html: &mut String, key: &str, role: Role, open: OpenLists) -> bool {
        let continued = open.paragraph == key;
        let section = |key: &str| key.split('/').next().unwrap_or_default().to_owned();
        let keep = match role {
            _ if continued => open.lists.len(),
            _ if section(key) != section(&open.paragraph) => 0,
            Role::Blank | Role::Note => open.lists.len(),
            Role::Item { ordered, level, .. } => open
                .lists
                .iter()
                .take_while(|list| {
                    list.level < level || (list.level == level && list.ordered == ordered)
                })
                .count(),
            _ => 0,
        };
        // A new item beside the innermost list's last one, which is done.
        let sibling = !continued
            && keep > 0
            && matches!(role, Role::Item { level, .. } if open.lists[keep - 1].level == level);
        for (index, list) in open.lists.into_iter().take(keep).enumerate() {
            self.write_list(html, &list);
            if !(sibling && index + 1 == keep) {
                self.write_item(html, &list.item);
            }
            self.lists.push(list);
        }
        if sibling {
            self.open_item(html, key);
        }
        continued || sibling
    }

    fn open_list(
        &mut self,
        html: &mut String,
        key: &str,
        ordered: bool,
        level: u32,
        inferred: bool,
    ) {
        let list = OpenList {
            ordered,
            level,
            inferred,
            list: format!("{key}/list"),
            item: String::new(),
        };
        self.write_list(html, &list);
        self.lists.push(list);
    }

    fn write_list(&self, html: &mut String, list: &OpenList) {
        html.push_str(if list.ordered { "<ol" } else { "<ul" });
        if list.inferred {
            html.push_str(" data-inferred=\"list\"");
        }
        if self.link {
            html.push_str(&part_marker('l', &list.list));
        }
        html.push('>');
    }

    /// The item of the paragraph `key` in the innermost list.
    fn open_item(&mut self, html: &mut String, key: &str) {
        let item = format!("{key}/li");
        self.write_item(html, &item);
        if let Some(list) = self.lists.last_mut() {
            list.item = item;
        }
    }

    fn write_item(&self, html: &mut String, item: &str) {
        html.push_str("<li");
        if self.link {
            html.push_str(&part_marker('i', item));
        }
        html.push('>');
    }

    fn close_list(&mut self, html: &mut String) {
        if let Some(list) = self.lists.pop() {
            html.push_str(if list.ordered {
                "</li></ol>"
            } else {
                "</li></ul>"
            });
        }
    }

    fn close_lists(&mut self, html: &mut String) {
        while !self.lists.is_empty() {
            self.close_list(html);
        }
    }

    fn close_element(&mut self, html: &mut String) {
        if let Some(tag) = self.open.take() {
            html.push_str("</");
            html.push_str(tag);
            html.push('>');
        }
    }

    /// End the group: the open element and every open list.
    pub(super) fn close(&mut self, html: &mut String) {
        self.close_element(html);
        self.close_lists(html);
        self.current = None;
    }

    /// End a column of the page flow. With `carry`, when nothing is written
    /// between this column and the next, the lists it ends inside are left
    /// for the next column to continue.
    pub(super) fn end_column(
        &mut self,
        html: &mut String,
        document: &RenderContext<'_>,
        carry: bool,
    ) {
        let open = match &self.current {
            Some((paragraph, _)) if carry && self.link && self.wrap && !self.lists.is_empty() => {
                Some(OpenLists {
                    lists: self.lists.clone(),
                    paragraph: paragraph.clone(),
                })
            }
            _ => None,
        };
        *document.open_lists.borrow_mut() = open;
        self.close(html);
    }
}

/// Classes whose base rule positions the element absolutely, which makes
/// its box a block whatever the element is.
const ABSOLUTE_CLASSES: &[&str] = &[
    "hls", "hcD", "hcI", "hsT", "hsR", "hsC", "hsG", "hsL", "hsE", "hsA", "hsP", "hsV", "hsO",
    "hsU", "heq", "htb", "hce",
];

/// Rewrite the divs written since `start` as spans, so the line can sit in a
/// `p`. A span that needs a block box keeps it through `display:block`.
pub(super) fn to_phrasing(html: &mut String, start: usize) {
    if !html[start..].contains("<div") {
        return;
    }
    let tail = html.split_off(start);
    let mut rest = tail.as_str();
    while let Some(position) = rest.find('<') {
        html.push_str(&rest[..position]);
        rest = &rest[position..];
        if let Some(after) = rest.strip_prefix("</div>") {
            html.push_str("</span>");
            rest = after;
        } else if rest.starts_with("<div ") || rest.starts_with("<div>") {
            let end = rest.find('>').map_or(rest.len(), |end| end + 1);
            html.push_str(&phrasing_open_tag(&rest[4..end - 1]));
            rest = &rest[end..];
        } else {
            html.push('<');
            rest = &rest[1..];
        }
    }
    html.push_str(rest);
}

fn phrasing_open_tag(attributes: &str) -> String {
    let attribute = |name: &str| {
        let marker = format!("{name}=\"");
        attributes.find(&marker).map(|at| {
            let value = &attributes[at + marker.len()..];
            &value[..value.find('"').unwrap_or(value.len())]
        })
    };
    let style = attribute("style").unwrap_or("");
    let class = attribute("class")
        .and_then(|class| class.split_whitespace().next())
        .unwrap_or("");
    let positioned = ABSOLUTE_CLASSES.contains(&class) && !style.contains("position:");
    if style.contains("display:") || positioned {
        format!("<span{attributes}>")
    } else if attributes.contains("style=\"") {
        format!(
            "<span{}>",
            attributes.replacen("style=\"", "style=\"display:block;", 1)
        )
    } else {
        format!("<span{attributes} style=\"display:block;\">")
    }
}

/// Whether a hyperlink target may become a live `href` (D29).
pub(super) fn is_safe_link(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
}

/// One grid slot of a semantic table row.
pub(super) enum Slot<'a> {
    Cell {
        cell: &'a TableCell,
        row_span: usize,
    },
    /// A position no cell of this fragment covers. An empty hidden cell
    /// keeps the following cells in their source columns.
    Gap,
}

pub(super) struct Row<'a> {
    pub(super) repeated: bool,
    pub(super) slots: Vec<Slot<'a>>,
}

pub(super) struct TableRows<'a> {
    pub(super) head: Vec<Row<'a>>,
    pub(super) body: Vec<Row<'a>>,
}

/// Arrange a (possibly page-fragment) table's cells into rows. A fragment
/// shows only `fragment_rows` of the source, so row spans are clipped to
/// it; repeated header copies form their own leading rows.
pub(super) fn plan_rows(table: &Table) -> TableRows<'_> {
    let body_cells = table
        .cells
        .iter()
        .filter(|cell| !cell.repeated_header)
        .collect::<Vec<_>>();
    let repeated_cells = table
        .cells
        .iter()
        .filter(|cell| cell.repeated_header)
        .collect::<Vec<_>>();
    let covered_last = body_cells
        .iter()
        .map(|cell| cell.row + cell.row_span.max(1) - 1)
        .max()
        .unwrap_or(0);
    let (first, last) = table.fragment_rows.unwrap_or((0, covered_last));
    let last = last.max(first);
    let repeated_end = repeated_cells
        .iter()
        .map(|cell| cell.row + 1)
        .max()
        .unwrap_or(0);

    // (display row, span) per cell, rows numbered in output order.
    let mut placed = Vec::new();
    for cell in &repeated_cells {
        let start = cell.row;
        let end = (cell.row + cell.row_span.max(1)).min(repeated_end);
        placed.push((start, end.max(start + 1) - start, *cell));
    }
    for cell in &body_cells {
        let start = cell.row.clamp(first, last);
        let end = (cell.row + cell.row_span.max(1)).min(last + 1);
        placed.push((
            repeated_end + start - first,
            end.max(start + 1) - start,
            *cell,
        ));
    }
    let row_count = repeated_end + last + 1 - first;
    placed.sort_by_key(|(row, _, cell)| (*row, cell.column));

    let mut covered = vec![Vec::<bool>::new(); row_count];
    let mut rows = (0..row_count)
        .map(|row| Row {
            repeated: row < repeated_end,
            slots: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut cursor = vec![0usize; row_count];
    for (row, span, cell) in placed {
        let span = span.min(row_count - row);
        let slots = &mut rows[row].slots;
        while cursor[row] < cell.column {
            if !covered[row].get(cursor[row]).copied().unwrap_or(false) {
                slots.push(Slot::Gap);
            }
            cursor[row] += 1;
        }
        slots.push(Slot::Cell {
            cell,
            row_span: span,
        });
        for covered_row in &mut covered[row..row + span] {
            let end = cell.column + cell.col_span.max(1);
            if covered_row.len() < end {
                covered_row.resize(end, false);
            }
            covered_row[cell.column..end].fill(true);
        }
        cursor[row] = cell.column + cell.col_span.max(1);
    }

    let head_end = if repeated_end > 0 {
        repeated_end
    } else if first == 0 {
        leading_header_rows(&rows)
    } else {
        0
    };
    let body = rows.split_off(head_end);
    TableRows { head: rows, body }
}

/// Rows `0..n` qualify as `thead` when each holds a source header cell and
/// no cell starting in them spans past them (HTML clips row spans at the
/// end of a row group). At least one body row must remain.
fn leading_header_rows(rows: &[Row<'_>]) -> usize {
    let mut end = 0;
    for row in rows {
        let has_header = row
            .slots
            .iter()
            .any(|slot| matches!(slot, Slot::Cell { cell, .. } if cell.is_header));
        if !has_header {
            break;
        }
        end += 1;
    }
    if end == 0 || end >= rows.len() {
        return 0;
    }
    let crosses = rows[..end].iter().enumerate().any(|(index, row)| {
        row.slots
            .iter()
            .any(|slot| matches!(slot, Slot::Cell { row_span, .. } if index + row_span > end))
    });
    if crosses {
        0
    } else {
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_imply_headings_and_list_levels_only_when_inferring() {
        let inferred = |text: &str| role(None, text, true);
        let heading = |level| Role::Heading {
            level,
            inferred: true,
        };
        let item = |level| Role::Item {
            ordered: false,
            level,
            inferred: true,
        };
        assert_eq!(inferred("제2장 2022회계연도 성과보고"), heading(2));
        assert_eq!(inferred("제1절 목적"), heading(3));
        assert_eq!(inferred("Ⅰ. 도입장비 개요"), heading(2));
        assert_eq!(inferred("□ 중견기업 수는"), item(0));
        assert_eq!(inferred("ㅇ 해당 사항 없음"), item(1));
        assert_eq!(inferred("◦ 경제안보 강화"), item(1));
        assert_eq!(inferred("- 전자우편 : minscho@korea.kr"), item(2));
        assert_eq!(inferred("· (프리랜서의 경우)"), item(3));
        assert_eq!(inferred("※ 제출의견 보내실 곳"), Role::Note);
        assert_eq!(inferred("→ 업무망, 외부망 각각"), Role::Note);
        assert_eq!(inferred("\u{3000}"), Role::Blank);
        // A symbol font's private-use glyph in place of □ (0713AI 도입).
        assert_eq!(inferred("\u{f02b1} (거버넌스) 운영"), item(0));
        // A byline between dashes is not a list item.
        assert_eq!(inferred("- 정보관리담당관(‘26.7.16) -"), Role::Paragraph);
        // Not markers: a section code, an amendment sentence, a contents
        // entry going on over line breaks, a number, a syllable.
        for text in [
            "Ⅰ-1. 신산업진흥",
            "제1조의2를 다음과 같이 신설한다.",
            "제1장 2022회계연도 성과목표관리체계\n 1. 임무와 비전\t1",
            "-1.5%",
            "ㅇ해당",
        ] {
            assert_eq!(inferred(text), Role::Paragraph, "{text}");
        }
        // Without inference only the source's own structure counts.
        assert_eq!(role(None, "□ 중견기업 수는", false), Role::Paragraph);
        assert_eq!(
            role(Some(ParaHeading::Bullet(1)), "text", false),
            Role::Item {
                ordered: false,
                level: 1,
                inferred: false
            }
        );
        assert_eq!(
            role(Some(ParaHeading::Outline(0)), "text", false),
            Role::Heading {
                level: 1,
                inferred: false
            }
        );
    }

    /// The outline rules on 0713AI 도입's paragraphs: a sans-serif, bold,
    /// 15pt, unindented paragraph opens a section (h2); under it □, ㅇ and -
    /// at 15pt are headings one level below the nearest higher marker.
    #[test]
    fn an_outline_nests_marker_headings_under_a_section_heading() {
        let mut outline = Outline::default();
        let section = Look {
            size: 1600,
            sans: true,
            bold: true,
        };
        let body = Look {
            size: 1500,
            sans: false,
            bold: false,
        };
        let small = Look { size: 1200, ..body };
        let heading = |level| Role::Heading {
            level,
            inferred: true,
        };
        let item = |level| Role::Item {
            ordered: false,
            level,
            inferred: true,
        };
        let mut next = |text: &str, look| outline.role(text, look, role(None, text, true));
        // Before any section heading, markers stay list items.
        assert_eq!(next("□ 개요", body), item(0));
        assert_eq!(next("1. AI 도입 시 고려사항", section), heading(2));
        assert_eq!(next(" □ 보안 : 내·외부망 분리 정책", body), heading(3));
        assert_eq!(next("  → 업무망, 외부망 각각", body), Role::Note);
        assert_eq!(next("  ㅇ (공통기반) 저장소 구축", body), heading(4));
        assert_eq!(next("  - (온AI) 기본 서비스", body), heading(5));
        assert_eq!(next("     * KEIT 등 가용 데이터", small), Role::Note);
        // A □-level marker closes the deeper ones.
        assert_eq!(next("\u{f02b1} (거버넌스) 운영", body), heading(3));
        assert_eq!(next(" ㅇ 역할 수행", body), heading(4));
        // Under 15pt a marker stays a list item.
        assert_eq!(next(" ㅇ 작은 글씨", small), item(1));
        // A new section starts the markers over, one level below it.
        assert_eq!(next("2. 내외부 AI 활용 자원", section), heading(2));
        assert_eq!(next("  ㅇ 바로 아래", body), heading(3));
        // Indented, or not bold: not a section heading.
        assert_eq!(next(" 3. 들여쓴 번호", section), Role::Paragraph);
        assert_eq!(
            next(
                "3. 굵지 않은 번호",
                Look {
                    bold: false,
                    ..section
                }
            ),
            Role::Paragraph
        );
        // A title box after other headings heads a section (성과보고서's
        // title bars after 제2장); a new outline's first one is the title.
        let title = Look {
            size: 1900,
            sans: true,
            bold: false,
        };
        assert_eq!(outline.title_role(title), heading(2));
        let text = "  ㅇ 그 아래";
        assert_eq!(outline.role(text, body, role(None, text, true)), heading(3));
        let mut fresh = Outline::default();
        assert_eq!(fresh.title_role(title), heading(1));
        assert_eq!(fresh.title_role(title), heading(2));
        assert_eq!(
            fresh.title_role(Look {
                sans: false,
                ..title
            }),
            Role::Paragraph
        );
        assert!(sans_serif("HY헤드라인M") && sans_serif("맑은 고딕") && sans_serif("함초롬돋움"));
        assert!(!sans_serif("함초롬바탕") && !sans_serif("휴먼명조"));
    }

    /// Lists a column of the page flow ends inside go on in the next column
    /// as further parts of the same lists; the logical DOM joins them into
    /// one nested list (성과보고서: ㅇ items going on after a page break).
    #[test]
    fn a_list_continues_into_the_next_column() {
        let item = |level| Role::Item {
            ordered: false,
            level,
            inferred: true,
        };
        let group = |resume| ParagraphGroup {
            wrap: true,
            link: true,
            infer: true,
            title: false,
            current: None,
            open: None,
            lists: Vec::new(),
            resume,
        };
        let paragraph = |html: &mut String, key: &str, text: &str| {
            html.push_str(&format!(
                "<p{}><span class=\"hls\" style=\"top:0mm;\">{text}</span></p>",
                part_marker('p', key)
            ));
        };
        let page = |content: &str| {
            format!("<div class=\"hpa\" style=\"width:210mm;height:296.99mm;\"><div class=\"hcD\" style=\"left:30mm;top:35mm;\"><div class=\"hcI\">{content}</div></div></div>")
        };
        // □ A, then ㅇ B, which the column ends in.
        let mut first = String::new();
        let mut column = group(None);
        column.start(&mut first, "s0/p[1]", item(0));
        paragraph(&mut first, "s0/p[1]", "A");
        column.start(&mut first, "s0/p[2]", item(1));
        paragraph(&mut first, "s0/p[2]", "B1");
        let open = || OpenLists {
            lists: column.lists.clone(),
            paragraph: "s0/p[2]".to_owned(),
        };
        let (carried, sibling, other_section) = (open(), open(), open());
        column.close(&mut first);
        // The rest of B, ㅇ C, □ D, then a plain paragraph.
        let mut second = String::new();
        let mut column = group(Some(carried));
        column.start(&mut second, "s0/p[2]", item(1));
        paragraph(&mut second, "s0/p[2]", "B2");
        column.start(&mut second, "s0/p[3]", item(1));
        paragraph(&mut second, "s0/p[3]", "C");
        column.start(&mut second, "s0/p[4]", item(0));
        paragraph(&mut second, "s0/p[4]", "D");
        column.start(&mut second, "s0/p[5]", Role::Paragraph);
        paragraph(&mut second, "s0/p[5]", "E");
        column.close(&mut second);
        let html = finalize_parts(&format!(
            "<!DOCTYPE html>\n<html><head></head><body>{}{}</body></html>",
            page(&first),
            page(&second)
        ));
        // Page markup: B's list and item are written again as their parts.
        assert!(html.contains("<div class=\"hcI\"><ul data-inferred=\"list\" data-hwpx-id=\"s0/p[1]/list\" data-hwpx-part=\"2/2\"><li data-hwpx-id=\"s0/p[1]/li\" data-hwpx-part=\"2/2\"><ul data-inferred=\"list\" data-hwpx-id=\"s0/p[2]/list\" data-hwpx-part=\"2/2\"><li data-hwpx-id=\"s0/p[2]/li\" data-hwpx-part=\"2/2\"><p data-hwpx-id=\"s0/p[2]\" data-hwpx-part=\"2/2\">"), "{html}");
        // The logical DOM: one nested list.
        let rebuilt = super::super::logical::rebuild(&html);
        let main = &rebuilt[rebuilt.find("<main").unwrap()..];
        assert!(!main.contains("data-hwpx-part"), "{main}");
        let mut skeleton = String::new();
        for (index, part) in main.split('<').enumerate() {
            let (tag, text) = part.split_once('>').unwrap_or(("", part));
            let name = tag.split(' ').next().unwrap_or_default();
            if index > 0 && matches!(name, "ul" | "/ul" | "li" | "/li") {
                skeleton.push_str(&format!("<{name}>"));
            }
            skeleton.push_str(text);
        }
        assert_eq!(
            skeleton,
            "<ul><li>A<ul><li>B1B2</li><li>C</li></ul></li><li>D</li></ul>E"
        );
        // A new item beside B: its item is done, the list goes on.
        let mut html = String::new();
        group(Some(sibling)).start(&mut html, "s0/p[3]", item(1));
        assert!(html.contains(&part_marker('l', "s0/p[2]/list")), "{html}");
        assert!(!html.contains(&part_marker('i', "s0/p[2]/li")), "{html}");
        assert!(html.ends_with(&format!("<li{}>", part_marker('i', "s0/p[3]/li"))));
        // A new section starts afresh.
        let mut html = String::new();
        group(Some(other_section)).start(&mut html, "s1/p[0]", item(1));
        assert!(!html.contains("s0/p["), "{html}");
    }

    #[test]
    fn single_part_markers_vanish_and_split_parts_are_numbered() {
        let html = format!(
            "<p{}>a</p><p{}>b</p><p{}>c</p>",
            part_marker('p', "s0/p[1]"),
            part_marker('p', "s0/p[2]"),
            part_marker('p', "s0/p[2]")
        );
        assert_eq!(
            finalize_parts(&html),
            "<p>a</p><p data-hwpx-id=\"s0/p[2]\" data-hwpx-part=\"1/2\">b</p>\
             <p data-hwpx-id=\"s0/p[2]\" data-hwpx-part=\"2/2\">c</p>"
        );
    }

    #[test]
    fn phrasing_rewrite_keeps_block_boxes() {
        let mut html = String::from("<p>");
        let start = html.len();
        html.push_str(concat!(
            "<div class=\"hls ps1\" style=\"left:0;\">",
            "<div class=\"hhe\" style=\"display:inline-block;\"></div>",
            "<div class=\"hsR\" style=\"top:0;\"><svg></svg><div class=\"hsT\"></div></div>",
            "<div class=\"unsupported\" style=\"left:0;\"><span>x</span></div>",
            "<div class=\"plain\"></div></div>"
        ));
        to_phrasing(&mut html, start);
        assert_eq!(
            html,
            concat!(
                "<p><span class=\"hls ps1\" style=\"left:0;\">",
                "<span class=\"hhe\" style=\"display:inline-block;\"></span>",
                "<span class=\"hsR\" style=\"top:0;\"><svg></svg><span class=\"hsT\"></span></span>",
                "<span class=\"unsupported\" style=\"display:block;left:0;\"><span>x</span></span>",
                "<span class=\"plain\" style=\"display:block;\"></span></span>"
            )
        );
    }

    #[test]
    fn only_web_and_mail_links_become_live() {
        assert!(is_safe_link(" HTTPS://example.kr"));
        assert!(is_safe_link("mailto:a@b.kr"));
        assert!(!is_safe_link("javascript:alert(1)"));
        assert!(!is_safe_link("C:\\문서\\a.hwp"));
    }
}
