//! The semantic tree (semantic-first plan §3.1): the document's structure
//! and content in reading order, built from the parsed [`Document`] alone.
//! It reads neither layout nor HTML, so no rule here depends on where a page
//! or column ends.
//!
//! [`Document`]: crate::model::Document

mod build;
pub mod equation;
pub mod markers;
pub mod order;

#[cfg(test)]
mod tests;

pub use build::{build, number_text};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    /// Stable key: a paragraph's [`Paragraph::key`], a table's id, an
    /// object's [`PositionedObject::key`], a cell's id; lists and items take
    /// their first paragraph's key plus `/list` or `/li`.
    ///
    /// [`Paragraph::key`]: crate::model::Paragraph::key
    /// [`PositionedObject::key`]: crate::model::PositionedObject::key
    pub key: String,
    #[serde(flatten)]
    pub kind: Kind,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Kind {
    Document,
    Section {
        index: usize,
    },
    /// A paragraph; its anchored tables and objects are its children.
    Paragraph {
        content: Vec<InlineSpan>,
        source: ParagraphSource,
    },
    Heading {
        level: u32,
        /// Implied by a marker, not the source's own outline level.
        inferred: bool,
        content: Vec<InlineSpan>,
        source: ParagraphSource,
    },
    List {
        ordered: bool,
        inferred: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        start: Option<u32>,
    },
    Item {
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<u32>,
    },
    Table {
        rows: usize,
        columns: usize,
        /// Placed as a character in its paragraph's line.
        inline: bool,
    },
    Cell {
        row: usize,
        column: usize,
        row_span: usize,
        col_span: usize,
        header: bool,
    },
    Object {
        object: String,
        inline: bool,
        description: String,
        /// Hancom equation script, for `object == "equation"`.
        #[serde(skip_serializing_if = "Option::is_none")]
        equation: Option<String>,
    },
    Caption {
        side: String,
    },
}

/// Original and compacted paragraph style references, and the XML token
/// inventory to which content spans point. Kept apart from presentation.
#[derive(Debug, Clone, Serialize)]
pub struct ParagraphSource {
    pub style_id: u32,
    pub source_style_id: u32,
    pub tokens: Vec<crate::model::SourceToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numbering: Option<NumberingValue>,
}

/// Q7: document-wide id/level counters are an explicit inference, including
/// when structural list boundaries split one numbering sequence.
#[derive(Debug, Clone, Serialize)]
pub struct NumberingValue {
    pub id: u32,
    pub level: u32,
    pub value: Option<u32>,
    pub inferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InlineSpan {
    #[serde(flatten)]
    pub value: Inline,
    /// Index into the paragraph's source inventory. Only a paragraph-head
    /// generated number has no inline XML token (it comes from the style).
    pub source_index: Option<usize>,
}

/// A paragraph's content in logical order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inline {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        href: Option<String>,
    },
    Tab,
    LineBreak,
    NonBreakingSpace,
    FixedSpace,
    /// A number the editor generates (`autoNum`, a NUMBER paragraph's
    /// number), kept as text (user decision Q2).
    Generated {
        text: String,
    },
    /// A generated number whose format this converter does not know.
    UnformattedNumber {
        number: u32,
        format: String,
    },
    /// `hp:dutmal`, with the attributes that place its annotation.
    Ruby {
        base: String,
        annotation: String,
        position: String,
        size_ratio: i64,
        option: i64,
        style_ref: u32,
        align: String,
        /// The annotation's character style (the layout's numbering); the
        /// source names a document style.
        #[serde(skip)]
        char_style_id: Option<u32>,
    },
    /// `hp:compose`: `text` drawn on top of one another in `shape`.
    Compose {
        text: String,
        shape: String,
        char_size: i64,
        compose_type: String,
        char_prs: Vec<u32>,
        /// Those character styles, as the layout numbers them.
        #[serde(skip)]
        char_style_ids: Vec<Option<u32>>,
    },
    /// Where the child with this key (a table or object) sits in the text.
    Anchor {
        key: String,
    },
}
