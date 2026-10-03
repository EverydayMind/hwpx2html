use serde::Serialize;

pub type HwpUnit = i64;

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct BoxUnits {
    pub x: HwpUnit,
    pub y: HwpUnit,
    pub width: HwpUnit,
    pub height: HwpUnit,
}

#[derive(Debug, Clone, Default)]
pub struct PageSpec {
    pub width: HwpUnit,
    pub height: HwpUnit,
    pub margin_left: HwpUnit,
    pub margin_right: HwpUnit,
    pub margin_top: HwpUnit,
    pub margin_bottom: HwpUnit,
    pub header: HwpUnit,
    pub footer: HwpUnit,
    pub hide_first_page_number: bool,
    pub page_number_enabled: bool,
    pub page_number_char_style_id: u32,
}

#[derive(Debug, Clone, Default)]
pub struct CharStyle {
    pub id: u32,
    pub font_family: String,
    pub latin_font_family: String,
    pub font_size_hwp: HwpUnit,
    pub color: String,
    pub background: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub emboss: bool,
    pub superscript: bool,
    pub subscript: bool,
    pub underline: bool,
    pub underline_color: Option<String>,
    pub strike: bool,
    pub strike_color: Option<String>,
    pub baseline_offset: HwpUnit,
    pub ratio: i64,
    pub spacing: HwpUnit,
    /// `useFontSpace` (글꼴에 어울리는 빈칸): a space keeps the font's own
    /// width. Without it Hancom draws every space half an em wide.
    pub use_font_space: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ParaStyle {
    pub id: u32,
    pub align: String,
    pub hanging_indent: HwpUnit,
    pub margin_left: HwpUnit,
    pub margin_right: HwpUnit,
    pub margin_before: HwpUnit,
    pub margin_after: HwpUnit,
    pub page_break_before: bool,
    /// `condense` (최소 공백): how many percent a line may shrink its spaces
    /// to keep a word on it.
    pub condense: i64,
    pub bullet_marker: Option<String>,
    /// The source's own structure for the paragraph (`hh:heading`).
    pub heading: Option<ParaHeading>,
    /// `hh:heading/@idRef`: the numbering (NUMBER) or bullet (BULLET) id.
    pub heading_ref: u32,
}

/// Raw numbering definition. The definition start and each level's start
/// are distinct source attributes; do not silently substitute one for another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Numbering {
    pub start: i64,
    pub levels: Vec<NumberingLevel>,
}

/// One level of a `hh:numbering` definition (`hh:paraHead`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NumberingLevel {
    /// The source's 1-based `level`.
    pub level: u32,
    pub start: u32,
    /// `numFormat`, e.g. `DIGIT`, `HANGUL_SYLLABLE`.
    pub format: String,
    /// The number's pattern, e.g. `^1.` (`^n` is level n's number).
    pub text: String,
}

/// A paragraph's `hh:heading`: an outline level or a list item. Levels are
/// the source's 0-based `level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParaHeading {
    Outline(u32),
    Number(u32),
    Bullet(u32),
}

#[derive(Debug, Clone)]
pub enum TokenKind {
    Text(String),
    Tab { width: HwpUnit, leader: u32 },
    LineBreak,
    NonBreakingSpace,
    FixedSpace,
    Control { kind: String },
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub logical_len: usize,
    pub char_style_id: u32,
    pub hyperlink: Option<String>,
}

impl Token {
    pub fn visible_text(&self) -> String {
        match &self.kind {
            TokenKind::Text(text) => text.clone(),
            TokenKind::Tab { .. } => "\t".to_owned(),
            TokenKind::LineBreak => "\n".to_owned(),
            TokenKind::NonBreakingSpace => "\u{00a0}".to_owned(),
            TokenKind::FixedSpace => "\u{00a0}".to_owned(),
            TokenKind::Control { .. } => String::new(),
        }
    }

    pub fn manifest_kind(&self) -> &'static str {
        match self.kind {
            TokenKind::Text(_) => "TEXT",
            TokenKind::Tab { .. } => "TAB",
            TokenKind::LineBreak => "LINE_BREAK",
            TokenKind::NonBreakingSpace => "NBSP",
            TokenKind::FixedSpace => "FWSPACE",
            TokenKind::Control { .. } => "CONTROL",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LineSeg {
    pub text_end: Option<usize>,
    pub textpos: usize,
    pub top: HwpUnit,
    pub height: HwpUnit,
    pub text_height: HwpUnit,
    /// `lineseg/@baseline`: the baseline's distance below `top`. A line of
    /// letters of one size has 85% of its `text_height`; a line with an
    /// annotation above its text has the annotation's band more.
    pub baseline: HwpUnit,
    pub spacing: HwpUnit,
    pub left: HwpUnit,
    pub width: HwpUnit,
}

/// Provenance of one sliced token. Logical positions are relative to the
/// parsed paragraph's UTF-16/control stream; byte positions are relative to the original text
/// token. A scalar overlapping a requested boundary is kept whole, so its
/// actual logical range may extend beyond the line's requested range.
/// Unsupported constructs omitted by the parser still need an independent
/// source check; these positions do not restore information the parser lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenSourceRange {
    pub token_index: usize,
    pub logical: std::ops::Range<usize>,
    pub utf8: Option<std::ops::Range<usize>>,
}

/// Original XML token position, distinct from layout's measured stream.
/// An unmeasured construct has a known start but no end; subsequent absolute
/// positions stay unknown. Token/run identity and UTF-8 offsets remain valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLogicalRange {
    pub start: Option<usize>,
    pub end: Option<usize>,
}

/// One XML inline token recorded while parsing, including controls that are
/// intentionally not emitted. Indices refer to the owning paragraph's model
/// tokens/preserved inline payloads, never to text matched after layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceToken {
    pub kind: String,
    pub run: usize,
    pub item: usize,
    pub token_index: Option<usize>,
    pub inline_index: Option<usize>,
    pub source_char_style_id: u32,
    pub char_style_id: u32,
    pub logical: SourceLogicalRange,
    pub utf8: Option<std::ops::Range<usize>>,
}

/// Source content a paragraph carries that the layout does not draw itself,
/// kept where it starts in the logical stream (semantic-first plan §3.1.3).
/// `autoNum`, `dutmal` and `compose` each keep their control token (eight
/// logical positions, DECISIONS "컨트롤 논리 길이"); the token is where the
/// content goes in a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSource {
    pub textpos: usize,
    pub content: InlineContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineContent {
    /// `hp:autoNum`: the number the source records and how it is written
    /// (`autoNumFormat`).
    AutoNumber {
        number_type: String,
        number: u32,
        format: String,
        user_char: String,
        prefix: String,
        suffix: String,
        superscript: bool,
    },
    /// `hp:dutmal`: base text with its annotation above or below.
    Ruby {
        base: String,
        annotation: String,
        /// `posType`: `TOP` or `BOTTOM`.
        position: String,
        /// `szRatio`: the annotation's size in percent of the base's; 0 is
        /// the editor's default (50).
        size_ratio: i64,
        /// `option`, whose meaning the corpus does not tell.
        option: i64,
        /// `styleIDRef`: the document style whose letters the annotation
        /// takes.
        style_ref: u32,
        /// `align`: where the annotation sits across the base text.
        align: String,
        /// The character style of `style_ref`, as the layout numbers them.
        char_style_id: Option<u32>,
    },
    /// `hp:compose`: characters drawn on top of each other in a shape.
    Compose {
        text: String,
        /// `circleType`: the shape around the characters.
        shape: String,
        /// `charSz`: the characters' size, negative for a reduction.
        char_size: i64,
        /// `composeType`: how the characters are arranged (`SPREAD`).
        compose_type: String,
        /// Every `charPr/@prIDRef`, in order (4294967295 is none).
        char_prs: Vec<u32>,
        /// Those as the layout numbers the character styles; `None` for none.
        char_style_ids: Vec<Option<u32>>,
    },
}

/// A table's or object's `hp:caption`.
#[derive(Debug, Clone)]
pub struct Caption {
    /// `LEFT`, `RIGHT`, `TOP` or `BOTTOM` of its table or object.
    pub side: String,
    pub gap: HwpUnit,
    /// `width`. Not the width of a caption above its table: every `TOP`
    /// caption of the corpus says 8504 and is `lastWidth` wide.
    pub width: HwpUnit,
    /// `fullSz`: the caption spans the table's outer margins too.
    pub full_size: bool,
    /// `lastWidth`: the width the editor last laid the caption out in.
    pub last_width: Option<HwpUnit>,
    pub paragraphs: Vec<Paragraph>,
    /// Tables in the caption's paragraphs, which are not parsed.
    pub unparsed_tables: usize,
}

/// An equation's source (`hp:equation`), before any rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquationSource {
    /// Hancom equation script (`hp:script`).
    pub script: String,
    pub base_unit: HwpUnit,
    pub base_line: i64,
    pub font: String,
    pub text_color: String,
}

/// Where a table or object sits in its owning paragraph's text stream
/// (semantic-first plan §3.1.2): the paragraph's [`Paragraph::key`] and the
/// logical position of its eight-unit slot. Recorded at parse time from the
/// XML node itself, never from an id's ordinals. Layout reads it only to find
/// a nested float's host paragraph; the line writers
/// (`presentation::line_items`) read it to set a table in its line where its
/// control stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAnchor {
    pub paragraph_key: String,
    pub textpos: usize,
}

#[derive(Debug, Clone)]
pub struct LineFragment {
    pub source_id: String,
    pub section_index: usize,
    pub paragraph_id: String,
    /// The owning paragraph's [`Paragraph::key`]; unlike `paragraph_id` it
    /// never repeats, so consecutive lines can be regrouped by paragraph.
    pub paragraph_key: String,
    /// Requested source interval, before scalar-boundary expansion.
    pub source_range: std::ops::Range<usize>,
    /// One entry per `tokens` item, recorded at slicing, never text-matched.
    pub token_sources: Vec<TokenSourceRange>,
    pub line_index: usize,
    pub top: HwpUnit,
    /// Source line position before the glyph baseline adjustment. Inline
    /// tables occupy this line box rather than the adjusted text position.
    pub inline_top: HwpUnit,
    pub height: HwpUnit,
    pub left: HwpUnit,
    pub padding_left: HwpUnit,
    pub width: HwpUnit,
    pub line_height: HwpUnit,
    pub para_style_id: u32,
    pub bullet_marker: Option<String>,
    /// The paragraph's own structure, on every one of its lines.
    pub heading: Option<ParaHeading>,
    /// The first 48 characters of the paragraph's text, on every one of its
    /// lines, for the markers that open it (제1장, □, ○).
    pub paragraph_start: String,
    /// The paragraph has no text and nothing inline (no picture, shape,
    /// inline table or bullet): it only holds space. On every one of its
    /// lines.
    pub paragraph_empty: bool,
    /// The paragraph's first line, when the author broke the page before it
    /// (`pageBreak`, or its style's): something new starts here.
    pub page_break: bool,
    pub tokens: Vec<Token>,
    pub inline_objects: Vec<PositionedObject>,
    /// Newspaper column layout in force for this line, as `(count, gap)`.
    pub columns: (u32, HwpUnit),
    /// Which of those columns the line sits in. A source coordinate restart
    /// moves to the next one rather than to a new page.
    pub column_index: u32,
    /// The line's own `lineseg/@spacing`, needed to measure how far a column
    /// of lines advances the flow.
    pub spacing: HwpUnit,
    /// The line's own `lineseg/@textheight` and `@baseline` (see
    /// [`LineSeg::baseline`]); a line with annotations above its text is the
    /// one whose baseline is not 85% of its text height.
    pub text_height: HwpUnit,
    pub baseline: HwpUnit,
    /// How the correction script widens the line to its full width, as its
    /// paragraph's alignment spreads it (양쪽·나눔·배분 정렬), if at all.
    pub fill: Option<LineFill>,
}

/// What a filled line spreads its room over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineFill {
    /// Its spaces (양쪽·나눔 정렬). A line with none spreads its letters.
    Spaces,
    /// The gaps between its letters (배분 정렬).
    Letters,
}

#[derive(Debug, Clone, Default)]
pub struct Anchor {
    pub treat_as_char: bool,
    pub flow_with_text: bool,
    pub vert_rel_to: String,
    pub horz_rel_to: String,
    pub vert_align: String,
    pub horz_align: String,
    pub vert_offset: HwpUnit,
    pub horz_offset: HwpUnit,
    pub z_order: i64,
}

#[derive(Debug, Clone)]
pub struct PositionedObject {
    pub id: String,
    /// Unique like [`Paragraph::key`]: `id` repeats when a document reuses
    /// paragraph ids. Objects under a top-level paragraph are re-keyed
    /// `{paragraph key}/o{n}` (children `/c{n}`); elsewhere it is `id`.
    pub key: String,
    pub source_path: String,
    /// Owning paragraph and slot; `None` for a container's child.
    pub source_anchor: Option<SourceAnchor>,
    pub caption: Option<Box<Caption>>,
    /// Set for `kind == "equation"`.
    pub equation: Option<Box<EquationSource>>,
    /// UTF-16 logical position of this object in its containing paragraph.
    pub textpos: usize,
    pub kind: String,
    pub box_units: BoxUnits,
    pub anchor: Anchor,
    pub alt: String,
    /// A picture's text alternative from the source: its caption, an
    /// author-written description, or a meaningful original file name.
    /// Empty when the source has none. `alt` stays the placeholder label.
    pub description: String,
    /// The key of the paragraph that placed this floating object on its
    /// page, so the output can put it after that paragraph (D28). Empty for
    /// objects nested in shapes or cells and for objects outside paragraphs.
    pub paragraph_key: String,
    pub binary_ref: Option<String>,
    pub mime_type: Option<String>,
    pub crop: Option<BoxUnits>,
    /// `hp:imgDim` — the scaled canvas size that `crop` coordinates are in.
    pub img_dim: BoxUnits,
    pub original_size: BoxUnits,
    pub flip_x: bool,
    pub flip_y: bool,
    pub stacking_order: i64,
    pub shape: Option<Box<ShapeStyle>>,
    pub children: Vec<PositionedObject>,
}

impl PositionedObject {
    /// Whether the renderers draw this object, as opposed to writing a
    /// placeholder for it (or nothing under `UnsupportedPolicy::Skip`). The
    /// one list the report, `--strict` and `Skip` share: shapes (rect,
    /// polygon, line), containers, equations and the pictures
    /// whose image can be written (a raster resource the package holds).
    /// Ellipses, arcs, curves, charts, videos, memos and OLE objects are not.
    pub fn is_drawn(&self, assets: &std::collections::BTreeMap<String, AssetRef>) -> bool {
        match self.kind.as_str() {
            "container" | "equation" | "line" | "rect" | "polygon" => true,
            "pic" => self
                .binary_ref
                .as_deref()
                .and_then(|reference| {
                    assets.get(reference).or_else(|| {
                        assets.values().find(|asset| {
                            asset.path == reference || asset.path.ends_with(reference)
                        })
                    })
                })
                .is_some_and(|asset| crate::assets::raster_mime(asset).is_some()),
            _ => self.shape.is_some(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ShapeStyle {
    /// Shape-local HWPUNIT points for polygons and lines.
    pub points: Vec<(i64, i64)>,
    pub fill: Option<String>,
    /// `hc:fillBrush/hc:imgBrush/hc:img/@binaryItemIDRef` -- a shape filled
    /// with a picture rather than a colour or gradient (e.g. a chart image
    /// dropped in as a `numberingType="PICTURE"` rect). Independent of
    /// `fill`/`gradient`: the source only ever sets one of the three.
    pub image_fill: Option<String>,
    pub gradient: Vec<String>,
    /// `hc:gradation/@step`, the number of colours the reference expands the
    /// gradient into. Zero when the shape has no gradation.
    pub gradient_step: u32,
    /// `hc:gradation/@angle`. Only the horizontal-band case (0) has a
    /// verified band geometry; other angles keep the CSS approximation.
    pub gradient_angle: i64,
    pub line_color: String,
    pub line_width: HwpUnit,
    /// The declared `hp:lineShape/@width`, unlike [`Self::line_width`] never
    /// zeroed for `style="NONE"`. A textbox-carrying shape's own `hsT` inset
    /// and pattern-fill outline still reserve this pen's half-width even
    /// when the line itself is never painted.
    pub declared_line_width: HwpUnit,
    pub corner_ratio: i64,
    pub paragraphs: Vec<Paragraph>,
    /// Tables carried by the text box's paragraphs, in the text box's own
    /// frame (anchored like a cell's nested tables), never hoisted to the
    /// page.
    pub tables: Vec<Table>,
    pub margins: [HwpUnit; 4],
    pub vertical_align: String,
}

#[derive(Debug, Clone)]
pub struct Paragraph {
    pub id: String,
    /// Document-unique structural path. `id` keeps the source `hp:p/@id`,
    /// which almost every top-level paragraph shares (2147483648 or 0).
    pub key: String,
    pub source_path: String,
    pub section_index: usize,
    pub para_style_id: u32,
    /// Original XML id before style compaction/alias resolution.
    pub source_para_style_id: u32,
    pub para_style: ParaStyle,
    pub line_top_offset: HwpUnit,
    pub tokens: Vec<Token>,
    pub source_tokens: Vec<SourceToken>,
    pub lines: Vec<LineSeg>,
    pub objects: Vec<PositionedObject>,
    /// Kept source content not drawn yet, in logical order.
    pub inline_sources: Vec<InlineSource>,
    pub page_break: bool,
    pub has_inline_table: bool,
    pub page_number_restart: Option<i64>,
    /// The paragraph carries a `pageNum` control ("쪽 번호 매기기"), directly or
    /// inside a table cell or text box. Page numbers are drawn from the page
    /// that holds the first such paragraph on.
    pub page_number_control: bool,
    /// The paragraph carries a `pageHiding` control with `hidePageNum="1"`
    /// ("현재 쪽만 감추기"), directly or inside a table cell or text box. It
    /// hides the page number of that one page; the page still counts.
    pub hides_page_number: bool,
    /// The `colPr` this paragraph carries, as `(column count, gap)`. A
    /// paragraph without one continues the previous setting.
    pub columns: Option<(u32, HwpUnit)>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BorderStroke {
    pub kind: String,
    pub color: String,
}

impl Default for BorderStroke {
    fn default() -> Self {
        Self {
            kind: "SOLID".to_owned(),
            color: "#000000".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TableCell {
    /// Source hp:tc/@header; distinct from a generated continuation copy.
    pub is_header: bool,
    /// Left, right, top, bottom border paint, independent of brush zones.
    pub border_strokes: [BorderStroke; 4],
    pub vertical_align: String,
    pub id: String,
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub box_units: BoxUnits,
    pub paragraphs: Vec<Paragraph>,
    pub tables: Vec<Table>,
    pub border_fill_id: Option<u32>,
    pub border_visible: bool,
    pub border_left_width: HwpUnit,
    pub border_right_width: HwpUnit,
    pub border_top_width: HwpUnit,
    pub border_bottom_width: HwpUnit,
    pub fill_color: Option<String>,
    /// This cell's own `borderFill`'s two-colour linear gradation, when it
    /// has one -- independent of `fill_color`, the source only ever sets
    /// one of the two. `gradient_step`/`gradient_angle` mirror
    /// `ShapeStyle`'s fields of the same name; only `gradient_angle == 0`
    /// has a verified band geometry.
    pub gradient: Vec<String>,
    pub gradient_step: u32,
    pub gradient_angle: i64,
    /// A "\" divider from the cell's top-left to its bottom-right corner
    /// (hp:borderFill's `backSlash`), independent of `diagonal_forward`.
    pub diagonal_backward: bool,
    /// A "/" divider from the cell's bottom-left to its top-right corner
    /// (hp:borderFill's `slash`).
    pub diagonal_forward: bool,
    /// Shared stroke style and width for whichever of the two diagonals
    /// above is set; `None` when neither is.
    pub diagonal_stroke: Option<BorderStroke>,
    pub diagonal_width: HwpUnit,
    pub margin_left: HwpUnit,
    pub margin_right: HwpUnit,
    pub margin_top: HwpUnit,
    pub margin_bottom: HwpUnit,
    pub repeated_header: bool,
    pub repeated_from: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub id: String,
    pub source_path: String,
    /// Owning paragraph and slot; `None` for a table outside any paragraph.
    pub source_anchor: Option<SourceAnchor>,
    pub caption: Option<Box<Caption>>,
    pub section_index: usize,
    pub anchor_y: Option<HwpUnit>,
    /// How much `anchor_y` subtracted for the anchor line's glyph box. The
    /// flow needs the adjusted value, but the rendered top does not.
    pub anchor_top_adjustment: HwpUnit,
    /// Left margin of the paragraph this table is anchored to. A
    /// `horzRelTo="PARA"` float measures its horizontal offset from there,
    /// not from the content area's own left edge.
    pub anchor_paragraph_left: HwpUnit,
    /// Right margin of that paragraph: where a `horzAlign="RIGHT"` float's
    /// area ends.
    pub anchor_paragraph_right: HwpUnit,
    pub box_units: BoxUnits,
    pub anchor: Anchor,
    pub out_margin_left: HwpUnit,
    pub out_margin_right: HwpUnit,
    pub out_margin_top: HwpUnit,
    pub out_margin_bottom: HwpUnit,
    pub columns: usize,
    pub rows: usize,
    pub row_heights: Vec<HwpUnit>,
    pub cells: Vec<TableCell>,
    pub page_break: String,
    pub repeat_header: bool,
    pub no_adjust: bool,
    /// Source rows (first, last) shown by this page fragment of a split
    /// table; `None` when the table is not split.
    pub fragment_rows: Option<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub enum Block {
    Paragraph(Paragraph),
    Table(Table),
    Object(PositionedObject),
}

#[derive(Debug, Clone, Default)]
pub struct Section {
    pub index: usize,
    pub source_path: String,
    pub page: PageSpec,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Default)]
pub struct PageNumberPolicy {
    pub enabled: bool,
    pub format: String,
    pub side_char: String,
    pub start: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub title: String,
    pub input_sha256: String,
    pub sections: Vec<Section>,
    pub char_styles: Vec<CharStyle>,
    pub para_styles: Vec<ParaStyle>,
    /// `hh:numbering` definitions by id.
    pub numberings: std::collections::BTreeMap<u32, Numbering>,
    pub page_numbers: PageNumberPolicy,
    pub assets: std::collections::BTreeMap<String, AssetRef>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AssetRef {
    pub id: String,
    pub path: String,
    pub mime_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct PlacedTable {
    pub table: Table,
    pub page_index: usize,
    pub fragment_index: usize,
}

#[derive(Debug, Clone)]
pub struct LayoutPage {
    pub index: usize,
    pub section_index: usize,
    pub spec: PageSpec,
    pub lines: Vec<LineFragment>,
    pub tables: Vec<PlacedTable>,
    pub objects: Vec<PositionedObject>,
    pub page_number: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutDocument {
    pub title: String,
    pub input_sha256: String,
    pub pages: Vec<LayoutPage>,
    pub char_styles: Vec<CharStyle>,
    pub para_styles: Vec<ParaStyle>,
    pub assets: std::collections::BTreeMap<String, AssetRef>,
    pub warnings: Vec<String>,
}
