//! Builds the semantic tree from the parsed document (plan §3.1). Order is
//! source order: paragraphs as stored, and each table or object where its
//! [`SourceAnchor`] puts it in its paragraph's text. Headings and lists
//! follow §3.1.1 and the user decisions Q4-Q6 (`docs/DECISIONS.md`).
//!
//! [`SourceAnchor`]: crate::model::SourceAnchor

use std::collections::BTreeMap;

use super::{markers, order, Inline, InlineSpan, Kind, Node, NumberingValue, ParagraphSource};
use crate::model::{
    Block, Caption, Document, InlineContent, ParaHeading, Paragraph, PositionedObject, Table,
    TokenKind,
};

/// A chapter or section marker outside the top-level body is a heading only
/// at this size or larger (user decision Q6), in HWPUNIT (1/100 pt).
const CONTEXT_HEADING_MIN_SIZE: i64 = 1500;

pub fn build(document: &Document, infer: bool) -> Node {
    let mut builder = Builder {
        document,
        infer,
        counters: BTreeMap::new(),
    };
    let children = document
        .sections
        .iter()
        .map(|section| {
            let mut tables = Vec::new();
            let mut items = Vec::new();
            for block in &section.blocks {
                match block {
                    Block::Paragraph(paragraph) => items.push(Item::Paragraph(paragraph)),
                    Block::Table(table) if table.source_anchor.is_some() => tables.push(table),
                    Block::Table(table) => items.push(Item::Table(table)),
                    Block::Object(object) => items.push(Item::Object(object)),
                }
            }
            Node {
                key: format!("s{}", section.index),
                kind: Kind::Section {
                    index: section.index,
                },
                children: builder.context(items, &tables, Context::Body),
            }
        })
        .collect();
    Node {
        key: String::new(),
        kind: Kind::Document,
        children,
    }
}

/// The text of `number` in an `autoNum`/`numFormat` format, or `None` for a
/// format not implemented here (the number is then kept unformatted).
pub fn number_text(number: u32, format: &str) -> Option<String> {
    const HANGUL: [char; 14] = [
        '가', '나', '다', '라', '마', '바', '사', '아', '자', '차', '카', '타', '파', '하',
    ];
    const CIRCLED: [char; 20] = [
        '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱',
        '⑲', '⑳',
    ];
    let index = usize::try_from(number).ok()?.checked_sub(1);
    match format {
        "DIGIT" => Some(number.to_string()),
        "HANGUL_SYLLABLE" => index.and_then(|i| HANGUL.get(i)).map(char::to_string),
        "CIRCLED_DIGIT" => index.and_then(|i| CIRCLED.get(i)).map(char::to_string),
        "LATIN_CAPITAL" | "LATIN_SMALL" => index.filter(|i| *i < 26).map(|i| {
            let base = if format == "LATIN_CAPITAL" {
                b'A'
            } else {
                b'a'
            };
            char::from(base + i as u8).to_string()
        }),
        "ROMAN_CAPITAL" | "ROMAN_SMALL" => index.filter(|i| *i < 12).map(|i| {
            let base = if format == "ROMAN_CAPITAL" {
                0x2160
            } else {
                0x2170
            };
            char::from_u32(base + i as u32)
                .expect("roman numeral block")
                .to_string()
        }),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    /// A section's top-level body.
    Body,
    /// A cell, text box or caption.
    Nested,
}

enum Item<'a> {
    Paragraph(&'a Paragraph),
    Table(&'a Table),
    Object(&'a PositionedObject),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Paragraph,
    Heading {
        level: u32,
        inferred: bool,
    },
    Entry {
        ordered: bool,
        level: u32,
        inferred: bool,
    },
    /// Stays in the open list item: an empty paragraph or a note (Q4).
    Continuation,
}

struct Builder<'a> {
    document: &'a Document,
    infer: bool,
    /// NUMBER paragraph counters: numbering id -> 0-based level -> number.
    counters: BTreeMap<u32, BTreeMap<u32, u32>>,
}

/// An open list and its current item while a context is grouped.
struct Open {
    list: Node,
    item: Option<Node>,
    level: u32,
    ordered: bool,
    inferred: bool,
}

impl<'a> Builder<'a> {
    /// One owning context's content with its lists grouped. `tables` are the
    /// context's anchored tables; each goes into the paragraph its anchor
    /// names, and one whose paragraph is not here stays at the end.
    fn context(
        &mut self,
        items: Vec<Item<'a>>,
        tables: &[&'a Table],
        context: Context,
    ) -> Vec<Node> {
        let mut by_paragraph: BTreeMap<&str, Vec<&'a Table>> = BTreeMap::new();
        for table in tables {
            let anchor = table.source_anchor.as_ref().expect("anchored table");
            by_paragraph
                .entry(anchor.paragraph_key.as_str())
                .or_default()
                .push(table);
        }
        let mut nodes = Vec::new();
        for item in items {
            match item {
                Item::Paragraph(paragraph) => {
                    let anchored = by_paragraph
                        .remove(paragraph.key.as_str())
                        .unwrap_or_default();
                    let role = self.role(paragraph, &anchored, context);
                    let node = self.paragraph(paragraph, &anchored, role);
                    nodes.push((node, role, paragraph.page_break));
                }
                Item::Table(table) => nodes.push((self.table(table), Role::Paragraph, false)),
                Item::Object(object) => nodes.push((self.object(object), Role::Paragraph, false)),
            }
        }
        for tables in by_paragraph.into_values() {
            for table in tables {
                nodes.push((self.table(table), Role::Paragraph, false));
            }
        }
        group_lists(nodes)
    }

    fn role(&self, paragraph: &Paragraph, tables: &[&Table], context: Context) -> Role {
        match paragraph.para_style.heading {
            Some(ParaHeading::Outline(level)) => {
                return Role::Heading {
                    level: (level + 1).min(6),
                    inferred: false,
                }
            }
            Some(ParaHeading::Number(level)) => {
                return Role::Entry {
                    ordered: true,
                    level,
                    inferred: false,
                }
            }
            Some(ParaHeading::Bullet(level)) => {
                return Role::Entry {
                    ordered: false,
                    level,
                    inferred: false,
                }
            }
            None => {}
        }
        let text = paragraph
            .tokens
            .iter()
            .map(|token| token.visible_text())
            .collect::<String>();
        let start = text.trim_start();
        let carries = !tables.is_empty()
            || !paragraph.objects.is_empty()
            || !paragraph.inline_sources.is_empty();
        if start.trim_end().is_empty() {
            return if carries {
                Role::Paragraph
            } else {
                Role::Continuation
            };
        }
        if !self.infer {
            return Role::Paragraph;
        }
        if start.starts_with(['*', '※']) {
            return if carries {
                Role::Paragraph
            } else {
                Role::Continuation
            };
        }
        if let Some(level) = markers::chapter_level(start) {
            if context == Context::Body || self.first_size(paragraph) >= CONTEXT_HEADING_MIN_SIZE {
                return Role::Heading {
                    level,
                    inferred: true,
                };
            }
            return Role::Paragraph;
        }
        match markers::marker_rank(start) {
            Some(level) => Role::Entry {
                ordered: false,
                level,
                inferred: true,
            },
            None => Role::Paragraph,
        }
    }

    /// The size of the paragraph's first visible character.
    fn first_size(&self, paragraph: &Paragraph) -> i64 {
        paragraph
            .tokens
            .iter()
            .find(|token| !token.visible_text().trim().is_empty())
            .and_then(|token| {
                self.document
                    .char_styles
                    .iter()
                    .find(|style| style.id == token.char_style_id)
            })
            .map_or(0, |style| style.font_size_hwp)
    }

    fn paragraph(&mut self, paragraph: &'a Paragraph, tables: &[&'a Table], role: Role) -> Node {
        // Number the host before its anchored children.
        let (numbering, generated) = self.paragraph_number(paragraph);
        // XML token ordinals, not logical lengths, identify the tokens: a
        // content span points at the source token of its control.
        let token_sources: BTreeMap<_, _> = paragraph
            .source_tokens
            .iter()
            .enumerate()
            .filter_map(|(index, source)| source.token_index.map(|t| (t, index)))
            .collect();
        let inline_sources: BTreeMap<_, _> = paragraph
            .source_tokens
            .iter()
            .enumerate()
            .filter_map(|(index, source)| source.inline_index.map(|i| (i, index)))
            .collect();
        let mut anchor_sources = BTreeMap::new();
        let mut content: Vec<(usize, InlineSpan)> = Vec::new();
        let mut position = 0;
        for (token_index, token) in paragraph.tokens.iter().enumerate() {
            let source_index = token_sources.get(&token_index).copied();
            if let TokenKind::Control { kind } = &token.kind {
                anchor_sources.insert((position, kind.as_str()), source_index);
            }
            let inline = match &token.kind {
                TokenKind::Text(text) => Some(Inline::Text {
                    text: text.clone(),
                    href: token.hyperlink.clone(),
                }),
                TokenKind::Tab { .. } => Some(Inline::Tab),
                TokenKind::LineBreak => Some(Inline::LineBreak),
                TokenKind::Control { kind } if kind == "lineBreak" => Some(Inline::LineBreak),
                TokenKind::NonBreakingSpace => Some(Inline::NonBreakingSpace),
                TokenKind::FixedSpace => Some(Inline::FixedSpace),
                // Objects and tables come from their anchors; field and
                // layout controls are deliberately left out (plan §3.1.3).
                TokenKind::Control { .. } => None,
            };
            if let Some(inline) = inline {
                content.push((
                    source_index.unwrap_or(usize::MAX),
                    InlineSpan {
                        value: inline,
                        source_index,
                    },
                ));
            }
            position += token.logical_len;
        }
        for (inline_index, source) in paragraph.inline_sources.iter().enumerate() {
            let inline = match &source.content {
                InlineContent::AutoNumber {
                    number,
                    format,
                    user_char,
                    prefix,
                    suffix,
                    ..
                } => {
                    let digits = if format == "USER_CHAR" {
                        Some(user_char.clone())
                    } else {
                        number_text(*number, format)
                    };
                    match digits {
                        Some(digits) => Inline::Generated {
                            text: format!("{prefix}{digits}{suffix}"),
                        },
                        None => Inline::UnformattedNumber {
                            number: *number,
                            format: format.clone(),
                        },
                    }
                }
                InlineContent::Ruby {
                    base,
                    annotation,
                    position,
                    size_ratio,
                    option,
                    style_ref,
                    align,
                    char_style_id,
                } => Inline::Ruby {
                    base: base.clone(),
                    annotation: annotation.clone(),
                    position: position.clone(),
                    size_ratio: *size_ratio,
                    option: *option,
                    style_ref: *style_ref,
                    align: align.clone(),
                    char_style_id: *char_style_id,
                },
                InlineContent::Compose {
                    text,
                    shape,
                    char_size,
                    compose_type,
                    char_prs,
                    char_style_ids,
                } => Inline::Compose {
                    text: text.clone(),
                    shape: shape.clone(),
                    char_size: *char_size,
                    compose_type: compose_type.clone(),
                    char_prs: char_prs.clone(),
                    char_style_ids: char_style_ids.clone(),
                },
            };
            let source_index = inline_sources.get(&inline_index).copied();
            content.push((
                source_index.unwrap_or(usize::MAX),
                InlineSpan {
                    value: inline,
                    source_index,
                },
            ));
        }
        let mut children: Vec<(usize, Item)> = Vec::new();
        for object in &paragraph.objects {
            let at = object
                .source_anchor
                .as_ref()
                .map_or(0, |anchor| anchor.textpos);
            let source_index = anchor_sources
                .get(&(at, object.kind.as_str()))
                .copied()
                .flatten();
            children.push((source_index.unwrap_or(usize::MAX), Item::Object(object)));
        }
        for table in tables {
            let at = table
                .source_anchor
                .as_ref()
                .map_or(0, |anchor| anchor.textpos);
            let source_index = anchor_sources.get(&(at, "tbl")).copied().flatten();
            children.push((source_index.unwrap_or(usize::MAX), Item::Table(table)));
        }
        children.sort_by_key(|(at, _)| *at);
        let children: Vec<_> = children
            .into_iter()
            .map(|(at, item)| {
                (
                    at,
                    match item {
                        Item::Object(object) => self.object(object),
                        Item::Table(table) => self.table(table),
                        Item::Paragraph(_) => unreachable!("anchored objects and tables only"),
                    },
                )
            })
            .collect();
        for (index, child) in &children {
            content.push((
                *index,
                InlineSpan {
                    value: Inline::Anchor {
                        key: child.key.clone(),
                    },
                    source_index: (*index != usize::MAX).then_some(*index),
                },
            ));
        }
        content.sort_by_key(|(index, _)| *index);
        let mut content = content
            .into_iter()
            .map(|(_, inline)| inline)
            .collect::<Vec<_>>();
        if matches!(
            role,
            Role::Entry {
                ordered: true,
                inferred: false,
                ..
            }
        ) {
            if let Some(number) = generated {
                content.insert(
                    0,
                    InlineSpan {
                        value: number,
                        source_index: None,
                    },
                );
            }
        }
        let source = ParagraphSource {
            style_id: paragraph.para_style_id,
            source_style_id: paragraph.source_para_style_id,
            tokens: paragraph.source_tokens.clone(),
            numbering,
        };
        let kind = match role {
            Role::Heading { level, inferred } => Kind::Heading {
                level,
                inferred,
                content,
                source,
            },
            _ => Kind::Paragraph { content, source },
        };
        Node {
            key: paragraph.key.clone(),
            kind,
            children: children.into_iter().map(|(_, node)| node).collect(),
        }
    }

    /// A NUMBER paragraph's number: per numbering id and level in tree
    /// order, a deeper level restarting when a shallower one advances.
    fn paragraph_number(
        &mut self,
        paragraph: &Paragraph,
    ) -> (Option<NumberingValue>, Option<Inline>) {
        let Some(ParaHeading::Number(level)) = paragraph.para_style.heading else {
            return (None, None);
        };
        let id = paragraph.para_style.heading_ref;
        let mut source = NumberingValue {
            id,
            level,
            value: None,
            inferred: true,
        };
        let Some(numbering) = self.document.numberings.get(&id) else {
            return (Some(source), None);
        };
        let definition = |level: u32| numbering.levels.iter().find(|item| item.level == level + 1);
        let counter = self.counters.entry(id).or_default();
        let next = match counter.get(&level) {
            Some(n) => n.checked_add(1),
            None => Some(definition(level).map_or(1, |item| item.start)),
        };
        let Some(next) = next else {
            return (Some(source), None);
        };
        counter.insert(level, next);
        counter.retain(|deeper, _| *deeper <= level);
        source.value = Some(next);
        let generated = (|| {
            let mut text = definition(level)?.text.clone();
            for source_level in (1..=9).rev() {
                let pattern = format!("^{source_level}");
                if !text.contains(&pattern) {
                    continue;
                }
                let number = *counter.get(&(source_level - 1))?;
                let format = &definition(source_level - 1)?.format;
                text = text.replace(&pattern, &number_text(number, format)?);
            }
            Some(Inline::Generated { text })
        })();
        (Some(source), generated)
    }

    fn table(&mut self, table: &'a Table) -> Node {
        let mut children = Vec::new();
        let caption = table.caption.as_deref();
        if let Some(caption) = caption.filter(|caption| leads(caption)) {
            children.push(self.caption(caption, &table.id));
        }
        let mut cells = table.cells.iter().collect::<Vec<_>>();
        cells.sort_by_key(|cell| (cell.row, cell.column));
        for cell in cells {
            let items = cell.paragraphs.iter().map(Item::Paragraph).collect();
            let tables = cell.tables.iter().collect::<Vec<_>>();
            children.push(Node {
                key: cell.id.clone(),
                kind: Kind::Cell {
                    row: cell.row,
                    column: cell.column,
                    row_span: cell.row_span,
                    col_span: cell.col_span,
                    header: cell.is_header,
                },
                children: self.context(items, &tables, Context::Nested),
            });
        }
        if let Some(caption) = caption.filter(|caption| !leads(caption)) {
            children.push(self.caption(caption, &table.id));
        }
        Node {
            key: table.id.clone(),
            kind: Kind::Table {
                rows: table.rows,
                columns: table.columns,
                inline: table.anchor.treat_as_char,
            },
            children,
        }
    }

    fn object(&mut self, object: &'a PositionedObject) -> Node {
        let mut children = Vec::new();
        let caption = object.caption.as_deref();
        if let Some(caption) = caption.filter(|caption| leads(caption)) {
            children.push(self.caption(caption, &object.key));
        }
        // User decision Q3: a container's children in reading order.
        let ordered = order::reading_order(object.children.iter().collect(), |child| {
            let (pen, text) = child.shape.as_ref().map_or((0, false), |shape| {
                (
                    shape.line_width.max(shape.declared_line_width),
                    !shape.paragraphs.is_empty(),
                )
            });
            (child.box_units, pen, text)
        });
        for child in ordered {
            children.push(self.object(child));
        }
        if let Some(shape) = &object.shape {
            let items = shape.paragraphs.iter().map(Item::Paragraph).collect();
            let tables = shape.tables.iter().collect::<Vec<_>>();
            children.extend(self.context(items, &tables, Context::Nested));
        }
        if let Some(caption) = caption.filter(|caption| !leads(caption)) {
            children.push(self.caption(caption, &object.key));
        }
        Node {
            key: object.key.clone(),
            kind: Kind::Object {
                object: object.kind.clone(),
                inline: object.anchor.treat_as_char,
                description: object.description.clone(),
                equation: object
                    .equation
                    .as_ref()
                    .map(|equation| equation.script.clone()),
            },
            children,
        }
    }

    fn caption(&mut self, caption: &'a Caption, host: &str) -> Node {
        let items = caption.paragraphs.iter().map(Item::Paragraph).collect();
        Node {
            key: format!("{host}/caption"),
            kind: Kind::Caption {
                side: caption.side.clone(),
            },
            children: self.context(items, &[], Context::Nested),
        }
    }
}

/// Whether a caption reads before its table or object.
fn leads(caption: &Caption) -> bool {
    matches!(caption.side.as_str(), "TOP" | "LEFT")
}

/// Groups a context's nodes into lists (plan §3.1.1): consecutive entries of
/// one context, nested by level. A body paragraph, heading, table or object
/// ends every open list, as does a paragraph with a page break before it;
/// a continuation stays in the open item.
fn group_lists(nodes: Vec<(Node, Role, bool)>) -> Vec<Node> {
    let mut output = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    fn close(stack: &mut Vec<Open>, output: &mut Vec<Node>) {
        let mut open = stack.pop().expect("an open list");
        open.list.children.extend(open.item.take());
        match stack.last_mut().and_then(|parent| parent.item.as_mut()) {
            Some(item) => item.children.push(open.list),
            None => output.push(open.list),
        }
    }
    for (node, role, page_break) in nodes {
        if page_break {
            while !stack.is_empty() {
                close(&mut stack, &mut output);
            }
        }
        match role {
            Role::Continuation => match stack.last_mut().and_then(|open| open.item.as_mut()) {
                Some(item) => item.children.push(node),
                None => output.push(node),
            },
            Role::Entry {
                ordered,
                level,
                inferred,
            } => {
                while stack.last().is_some_and(|open| open.level > level) {
                    close(&mut stack, &mut output);
                }
                if stack.last().is_some_and(|open| {
                    open.level == level && (open.ordered != ordered || open.inferred != inferred)
                }) {
                    close(&mut stack, &mut output);
                }
                let value = match &node.kind {
                    Kind::Paragraph { source, .. } => {
                        source.numbering.as_ref().and_then(|n| n.value)
                    }
                    _ => None,
                };
                let item = Node {
                    key: format!("{}/li", node.key),
                    kind: Kind::Item { value },
                    children: vec![],
                };
                match stack.last_mut() {
                    Some(open) if open.level == level => {
                        open.list.children.extend(open.item.take());
                        open.item = Some(item);
                    }
                    _ => stack.push(Open {
                        list: Node {
                            key: format!("{}/list", node.key),
                            kind: Kind::List {
                                ordered,
                                inferred,
                                start: value,
                            },
                            children: vec![],
                        },
                        item: Some(item),
                        level,
                        ordered,
                        inferred,
                    }),
                }
                stack
                    .last_mut()
                    .and_then(|open| open.item.as_mut())
                    .expect("the item just opened")
                    .children
                    .push(node);
            }
            Role::Paragraph | Role::Heading { .. } => {
                while !stack.is_empty() {
                    close(&mut stack, &mut output);
                }
                output.push(node);
            }
        }
    }
    while !stack.is_empty() {
        close(&mut stack, &mut output);
    }
    output
}
