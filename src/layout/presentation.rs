//! The placement plan (semantic-first plan §3.2, stage 2): where each line of
//! a laid-out page sits on its page, computed from the layout alone. Both
//! writers read it: the page-by-page renderer, which writes the offsets as
//! nested boxes, and the direct emitter, which adds them up.

pub mod caption;
pub mod gradient;
pub mod objects;

use crate::model::{HwpUnit, LayoutPage, LineFragment, PageSpec, Table, TableCell, TokenKind};

/// The space the reference leaves between one column group and the next.
///
/// Read off 제어정보길이_검증6, whose four pages hold eleven groups: each
/// group's `top` is the previous one's plus its tallest column's extent plus
/// this. It reproduces 7.53, 37.63, 56.45, 47.04, 65.86, 31.99, 62.10 and
/// 80.91mm exactly. That sample uses one text size throughout, so a constant
/// cannot yet be told apart from a formula in the line height or spacing.
pub const COLUMN_GROUP_GAP: HwpUnit = 1134;

/// A run of lines the reference gives one column layout.
pub struct ColumnGroup<'a> {
    pub columns: (u32, HwpUnit),
    pub lines: Vec<&'a LineFragment>,
}

/// Split a page's lines into the groups the reference gives their own
/// containers. A group ends where the next line's column layout differs or
/// where its column index steps back to the first column, which is what a new
/// `colPr` produces even when it repeats the current settings.
pub fn column_groups(lines: &[LineFragment]) -> Vec<ColumnGroup<'_>> {
    let mut groups: Vec<ColumnGroup<'_>> = Vec::new();
    for line in lines {
        let starts_group = match groups.last() {
            None => true,
            Some(group) => {
                group.columns != line.columns
                    || (line.column_index == 0
                        && group.lines.last().is_some_and(|last| {
                            last.column_index > 0 || line.inline_top < last.inline_top
                        }))
            }
        };
        if starts_group {
            groups.push(ColumnGroup {
                columns: line.columns,
                lines: Vec::new(),
            });
        }
        if let Some(group) = groups.last_mut() {
            group.lines.push(line);
        }
    }
    groups
}

/// How far a group advances the flow: the tallest column's own extent, since
/// the columns sit side by side. The last line of a column contributes its
/// height but not its trailing spacing, the same way `line_flow_height`
/// measures a cell's content.
pub fn column_flow_height(group: &ColumnGroup<'_>) -> HwpUnit {
    (0..group.columns.0.max(1))
        .map(|column| {
            group
                .lines
                .iter()
                .filter(|line| line.column_index == column)
                .map(|line| line.inline_top.saturating_add(line.height.max(0)))
                .max()
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
}

/// One column of one group, and where it starts in the page's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Container {
    pub group: usize,
    pub column: u32,
    pub left: HwpUnit,
    pub top: HwpUnit,
}

/// Where each column of each group starts: a group's columns stand one
/// stride apart (its widest line plus the gap), and the groups follow each
/// other down the page.
pub fn containers(groups: &[ColumnGroup<'_>]) -> Vec<Container> {
    let mut containers = Vec::new();
    let mut group_top = 0;
    for (group_index, group) in groups.iter().enumerate() {
        let (count, gap) = group.columns;
        let stride = group
            .lines
            .iter()
            .map(|line| line.width)
            .max()
            .unwrap_or(0)
            .saturating_add(gap);
        for column in 0..count.max(1) {
            containers.push(Container {
                group: group_index,
                column,
                left: stride.saturating_mul(HwpUnit::from(column)),
                top: group_top,
            });
        }
        group_top = group_top
            .saturating_add(column_flow_height(group))
            .saturating_add(COLUMN_GROUP_GAP);
    }
    containers
}

/// Where the page's body starts on the page: the text area inside the
/// margins and below the header.
pub fn body_origin(page: &LayoutPage) -> (HwpUnit, HwpUnit) {
    (
        page.spec.margin_left,
        page.spec.margin_top + page.spec.header,
    )
}

/// A line's own left and top on its page, before any inline table lowers it:
/// the body's origin, its column container's, and its own.
pub fn line_origin(
    page: &LayoutPage,
    container: &Container,
    line: &LineFragment,
) -> (HwpUnit, HwpUnit) {
    let (body_left, body_top) = body_origin(page);
    (
        body_left
            .saturating_add(container.left)
            .saturating_add(line.left),
        body_top
            .saturating_add(container.top)
            .saturating_add(line.top),
    )
}

/// The line box a line is drawn in: its top, its CSS line height and its
/// height. A line carrying an inline table sits at its source position
/// (`inline_top`, before the glyph adjustment) and grows to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineBox {
    pub top: HwpUnit,
    pub line_height: HwpUnit,
    pub height: HwpUnit,
}

pub fn line_box(line: &LineFragment, inline_table: Option<&Table>) -> LineBox {
    host_line_box(
        line,
        inline_table.map(|table| {
            (
                table,
                table.box_units.height + inline_table_frame_extra_y(table),
            )
        }),
    )
}

/// How far above the source's `vertpos` a line's box starts: a twentieth of
/// the line's text height, so that the glyphs' baseline lies where the source
/// puts it (`LineFragment::top`).
pub fn glyph_gap(text_height: HwpUnit) -> HwpUnit {
    (text_height / 20).max(1)
}

/// The line box of a line whose letters (of `size`) have annotations above
/// them, taking `band` of the line's height (the source's `baseline` less
/// 85% of `size`; its `textheight` is `size + band`). The box is the one a
/// line of plain letters of that size has, moved down by the band, so the
/// letters' baseline lies at the source's `baseline` and the band above is
/// the annotations'. `line_height` is that size's own CSS line height.
pub fn annotated_line_box(
    line: &LineFragment,
    size: HwpUnit,
    band: HwpUnit,
    line_height: HwpUnit,
) -> LineBox {
    LineBox {
        top: line
            .inline_top
            .saturating_add(band)
            .saturating_sub(glyph_gap(size)),
        line_height,
        height: line.height,
    }
}

/// [`line_box`] for a line carrying an inline table in a box of the given
/// height: the table's own box, or the area it shares with its caption
/// ([`caption::TableCaptionPlan`]).
pub fn host_line_box(line: &LineFragment, host: Option<(&Table, HwpUnit)>) -> LineBox {
    let top = host.map_or(line.top, |_| line.inline_top);
    // A CELL-pageBreak table used to always set the line's CSS line-height to
    // its own box height, regardless of size. 루이지애나 변형2 샘플5's small
    // (4.52mm) single-cell CELL table showed that's wrong when the table is
    // no taller than the paragraph's own text metric would already give: the
    // reference keeps the text-height-derived line-height (5.70mm) there and
    // only grows to the table's own height (this same formula) once the
    // table is large enough to need it -- the same threshold the non-CELL
    // branch already used.
    let line_height = match host {
        Some((_, height)) if height > line.line_height.saturating_add(500) => height,
        _ => line.line_height,
    };
    let height = host
        .filter(|(table, _)| table.page_break.eq_ignore_ascii_case("CELL"))
        .map_or(line.height, |(_, height)| height);
    LineBox {
        top,
        line_height,
        height,
    }
}

/// One thing a line writes, in the order its paragraph has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineItem {
    /// `line.tokens[index]`.
    Token(usize),
    /// The inline table at that index of the tables given to [`line_items`].
    Table(usize),
    /// `line.inline_objects[index]`.
    Object(usize),
}

/// The order a line writes its tokens, the tables set in it (`tables`, those
/// whose anchor is this line) and its inline objects in. A table or object
/// stands where its control stands in the paragraph's text: the parser keeps
/// the control as a token of eight logical positions and records its start
/// (`Table::source_anchor`, `PositionedObject::textpos`), and a box set in the
/// line takes its place in the line's text as a letter does.
///
/// What no control claims -- a table without an anchor, one anchored in
/// another paragraph, an object the parser found outside a run, a line that
/// starts inside a control -- is written after the last token, tables first,
/// as every box was before; the order of what is known is the only change.
pub fn line_items(line: &LineFragment, tables: &[&Table]) -> Vec<LineItem> {
    line_items_with_fallback_counts(line, tables).0
}

/// Same as [`line_items`], but also returns `(unmatched_tables, unmatched_objects)`
/// which fell back to the end of the line.
pub fn line_items_with_fallback_counts(
    line: &LineFragment,
    tables: &[&Table],
) -> (Vec<LineItem>, usize, usize) {
    let mut items =
        Vec::with_capacity(line.tokens.len() + tables.len() + line.inline_objects.len());
    if tables.is_empty() && line.inline_objects.is_empty() {
        items.extend((0..line.tokens.len()).map(LineItem::Token));
        return (items, 0, 0);
    }
    let mut table_used = vec![false; tables.len()];
    let mut object_used = vec![false; line.inline_objects.len()];
    for (index, token) in line.tokens.iter().enumerate() {
        if let (TokenKind::Control { kind }, Some(source)) =
            (&token.kind, line.token_sources.get(index))
        {
            let start = source.logical.start;
            if kind == "tbl" {
                let slot = tables.iter().enumerate().position(|(at, table)| {
                    !table_used[at]
                        && table.source_anchor.as_ref().is_some_and(|anchor| {
                            anchor.paragraph_key == line.paragraph_key && anchor.textpos == start
                        })
                });
                if let Some(at) = slot {
                    table_used[at] = true;
                    items.push(LineItem::Table(at));
                }
            } else {
                let slot = line
                    .inline_objects
                    .iter()
                    .enumerate()
                    .position(|(at, object)| {
                        !object_used[at]
                            && object.source_anchor.is_some()
                            && object.kind == *kind
                            && object.textpos == start
                    });
                if let Some(at) = slot {
                    object_used[at] = true;
                    items.push(LineItem::Object(at));
                }
            }
        }
        items.push(LineItem::Token(index));
    }
    let unmatched_tables = table_used.iter().filter(|used| !**used).count();
    let unmatched_objects = object_used.iter().filter(|used| !**used).count();
    items.extend(
        table_used
            .iter()
            .enumerate()
            .filter(|(_, used)| !**used)
            .map(|(at, _)| LineItem::Table(at)),
    );
    items.extend(
        object_used
            .iter()
            .enumerate()
            .filter(|(_, used)| !**used)
            .map(|(at, _)| LineItem::Object(at)),
    );
    (items, unmatched_tables, unmatched_objects)
}

// Hancom's HTML exporter leaves room around the cell grid of a table whose
// own outMargin is zero, but only where that grid is nested inside another
// box. A page-placed float takes its declared size unchanged.
pub const TABLE_FRAME_EXTRA: HwpUnit = 280;
pub const INLINE_CELL_FRAME_EXTRA_X: HwpUnit = 566;
pub const INLINE_CELL_FRAME_EXTRA_Y: HwpUnit = 565;
/// How far a table's border drawing reaches past its box on every side.
pub const TABLE_SVG_GUTTER: HwpUnit = 709;

/// Where a table is drawn from: nested in a cell, inline in a line, or on
/// the page at a computed place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TablePlacement {
    Nested,
    Inline,
    Page {
        left: HwpUnit,
        top: HwpUnit,
        frame_adjustment: bool,
    },
}

pub fn inline_table_frame_extra_x(table: &Table) -> HwpUnit {
    if table.page_break.eq_ignore_ascii_case("CELL") {
        let extra = table.out_margin_left.saturating_add(table.out_margin_right);
        if extra > 0 {
            extra - 1
        } else {
            INLINE_CELL_FRAME_EXTRA_X - 1
        }
    } else {
        TABLE_FRAME_EXTRA
    }
}

pub fn inline_table_frame_extra_y(table: &Table) -> HwpUnit {
    if table.page_break.eq_ignore_ascii_case("CELL") {
        let extra = table.out_margin_top.saturating_add(table.out_margin_bottom);
        if extra > 0 {
            extra - 1
        } else {
            INLINE_CELL_FRAME_EXTRA_Y
        }
    } else {
        TABLE_FRAME_EXTRA
    }
}

/// A page-placed table starts at its own outMargin, with nothing added when
/// that margin is zero. 성과보고서's 5x3 sidebar is the case that settled
/// this: the reference gives it `left:21.31mm`, exactly its paragraph's
/// margin plus its own -1960 offset, where a 140-unit frame inset would put
/// it at 21.80mm.
pub fn table_frame_offset(out_margin_left: HwpUnit) -> HwpUnit {
    out_margin_left.max(0)
}

/// Likewise for its width and height: 11.23 x 255.86mm in the reference is
/// its declared 3183 x 72528 HWPUNIT verbatim, not those plus a frame.
pub fn table_frame_extra_x(table: &Table) -> HwpUnit {
    table
        .out_margin_left
        .saturating_add(table.out_margin_right)
        .max(0)
}

pub fn table_frame_extra_y(table: &Table) -> HwpUnit {
    table
        .out_margin_top
        .saturating_add(table.out_margin_bottom)
        .max(0)
}

/// A table's box (`htb`): against its cell for [`TablePlacement::Nested`],
/// inside its line for [`TablePlacement::Inline`] (where the browser places
/// it, so `left`/`top` are 0), on the page for [`TablePlacement::Page`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub left: HwpUnit,
    pub top: HwpUnit,
    pub width: HwpUnit,
    pub height: HwpUnit,
}

pub fn table_box(table: &Table, placement: TablePlacement) -> Frame {
    match placement {
        TablePlacement::Nested => {
            // A floating (non-treatAsChar) table nested in a cell still
            // grows by its own declared frame the same way a page-placed
            // float does -- 성과보고서 s6's "<사업운영 관리의 고도화
            // 프로세스>" process table (`outMargin` 141 each side) is
            // 165.71 x 115.46mm in the reference, not the bare 164.71 x
            // 114.47mm its own declared cells sum to.
            let frame_offset = table_frame_offset(table.out_margin_left);
            Frame {
                left: table.box_units.x + frame_offset,
                top: table.box_units.y + frame_offset,
                width: table.box_units.width + table_frame_extra_x(table),
                height: table.box_units.height + table_frame_extra_y(table),
            }
        }
        TablePlacement::Inline => Frame {
            left: 0,
            top: 0,
            width: table.box_units.width + inline_table_frame_extra_x(table),
            height: table.box_units.height + inline_table_frame_extra_y(table),
        },
        TablePlacement::Page {
            left,
            top,
            frame_adjustment,
        } => Frame {
            left,
            top,
            width: table.box_units.width
                + if frame_adjustment {
                    table_frame_extra_x(table)
                } else {
                    0
                },
            height: table.box_units.height
                + if frame_adjustment {
                    table_frame_extra_y(table)
                } else {
                    0
                },
        },
    }
}

/// The border drawing's own box, against the table's box: the table's size
/// with [`TABLE_SVG_GUTTER`] on every side, so strokes on the edge are not
/// cut. Its `viewBox` starts at minus the gutter in the same millimetres.
pub fn table_svg_frame(table: &Table, placement: TablePlacement) -> Frame {
    let frame = table_box(table, placement);
    Frame {
        left: -TABLE_SVG_GUTTER,
        top: -TABLE_SVG_GUTTER,
        width: frame.width + 2 * TABLE_SVG_GUTTER,
        height: frame.height + 2 * TABLE_SVG_GUTTER,
    }
}

/// Where a page-level floating table's box goes on its page: centred in the
/// text area, against the right edge of its anchor paragraph, or at its own
/// offset from the text area (or from the paper for `vertRelTo="PAPER"`).
/// The box includes the table's outMargin frame.
pub fn floating_table_origin(page: &PageSpec, table: &Table) -> (HwpUnit, HwpUnit) {
    let paper_positioned = table.anchor.vert_rel_to.eq_ignore_ascii_case("PAPER");
    // outMargin is the table's own declared padding outside its border,
    // independent of how its anchor position was computed -- a nonzero
    // vertOffset does not mean that offset already bakes the margin in.
    // Confirmed against two independent floating tables with a nonzero
    // PARA-relative vertOffset (성과보고서's cover table, KGS's main
    // table): both need the same outMargin-derived width growth as the
    // vertOffset==0 case already handled.
    let frame_offset = table_frame_offset(table.out_margin_left);
    let rendered_width = table.box_units.width + table_frame_extra_x(table);
    let centered_in_content = table.anchor.horz_align.eq_ignore_ascii_case("CENTER");
    // `horzAlign="RIGHT"` against the anchor paragraph puts the whole
    // frame (outMargins included) against the paragraph's right edge, and
    // `horzOffset` moves it inward. 성과보고서's top-level RIGHT floats
    // (s28 p281-287: 4.0mm to 14mm right of a left-aligned box) end at the
    // content edge plus the left outMargin, which is this frame rule.
    let flush_right = table.anchor.horz_align.eq_ignore_ascii_case("RIGHT")
        && table.anchor.horz_rel_to.eq_ignore_ascii_case("PARA");
    let left = if centered_in_content {
        page.margin_left
            + (page
                .width
                .saturating_sub(page.margin_left)
                .saturating_sub(page.margin_right)
                .saturating_sub(rendered_width))
                / 2
            + frame_offset
    } else if flush_right {
        page.width.saturating_sub(page.margin_right)
            - table.anchor_paragraph_right
            - rendered_width
            - table.anchor.horz_offset
            + frame_offset
    } else {
        page.margin_left + table.box_units.x + frame_offset
    };
    let top = if paper_positioned {
        table.box_units.y + frame_offset
    } else {
        page.margin_top
            + page.header
            + table.box_units.y
            // The anchor line's glyph-box adjustment belongs to the text
            // flow, not to the float hanging off it: 29 of 성과보고서's
            // page-placed tables whose left, width and height already
            // match sit exactly that far above the reference.
            + table.anchor_top_adjustment
            + frame_offset
    };
    (left, top)
}

/// How tall a cell's content is, for its vertical alignment.
pub fn cell_text_height(table: &Table, cell: &TableCell) -> HwpUnit {
    // These cells paint restarted lines consecutively. Measuring only the
    // largest local bottom invents spare space and centers the text down
    // into the bottom border even after the row minimum was corrected.
    let paragraph_height = if crate::layout::is_no_adjust_text_growth(table, cell) {
        crate::layout::line_flow_height(
            &cell
                .paragraphs
                .iter()
                .flat_map(|p| &p.lines)
                .cloned()
                .collect::<Vec<_>>(),
        )
    } else {
        cell.paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.lines.iter())
            .map(|line| {
                line.top
                    .saturating_add(line.height.max(line.text_height).max(0))
            })
            .max()
            .unwrap_or(0)
    };
    let nested_height = cell
        .tables
        .iter()
        .map(|nested| {
            let floating_frame =
                if !nested.anchor.treat_as_char && table.page_break.eq_ignore_ascii_case("CELL") {
                    nested.out_margin_top.max(0)
                        + match cell.vertical_align.as_str() {
                            "TOP" => 0,
                            "BOTTOM" => nested.out_margin_bottom.max(0),
                            _ => nested.out_margin_bottom.max(0) / 2,
                        }
                } else {
                    0
                };
            nested
                .box_units
                .y
                .saturating_add(nested.box_units.height.max(0))
                .saturating_add(floating_frame)
        })
        .max()
        .unwrap_or(0);
    // A floating (non-treatAsChar) object's own bottom edge also needs to
    // enter this max: it is otherwise invisible to the cell's vertical-center
    // math, which then invents spare space below it and pushes the object
    // (and everything after it) down by roughly half the object's own height.
    let float_height = cell
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.objects.iter())
        .filter(|object| !object.anchor.treat_as_char)
        .map(|object| {
            object
                .box_units
                .y
                .saturating_add(object.box_units.height.max(0))
        })
        .max()
        .unwrap_or(0);
    paragraph_height.max(nested_height).max(float_height)
}

/// How far a cell's content moves down inside its margins for the cell's
/// vertical alignment.
pub fn cell_vertical_offset(table: &Table, cell: &TableCell) -> HwpUnit {
    let inner_height = cell
        .box_units
        .height
        .saturating_sub(cell.margin_top)
        .saturating_sub(cell.margin_bottom);
    let spare = inner_height
        .saturating_sub(cell_text_height(table, cell))
        .max(0);
    match cell.vertical_align.as_str() {
        "TOP" => 0,
        "BOTTOM" => spare,
        _ => spare / 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(top: i64, height: i64, column: u32, columns: (u32, i64)) -> LineFragment {
        LineFragment {
            source_id: String::new(),
            section_index: 0,
            paragraph_id: String::new(),
            paragraph_key: String::new(),
            source_range: 0..0,
            token_sources: Vec::new(),
            line_index: 0,
            top,
            inline_top: top,
            height,
            left: 0,
            padding_left: 0,
            width: 1000,
            line_height: height,
            para_style_id: 0,
            bullet_marker: None,
            heading: None,
            paragraph_start: String::new(),
            paragraph_empty: false,
            page_break: false,
            tokens: Vec::new(),
            inline_objects: Vec::new(),
            columns,
            column_index: column,
            spacing: 0,
            text_height: height,
            baseline: 0,
            fill: None,
        }
    }

    #[test]
    fn groups_stack_down_the_page_and_columns_stand_one_stride_apart() {
        let lines = [
            line(0, 500, 0, (2, 200)),
            line(0, 300, 1, (2, 200)),
            // The column layout changes: a new group below the first.
            line(0, 400, 0, (1, 0)),
        ];
        let groups = column_groups(&lines);
        assert_eq!(groups.len(), 2);
        let all = containers(&groups);
        assert_eq!(
            all,
            [
                Container {
                    group: 0,
                    column: 0,
                    left: 0,
                    top: 0
                },
                Container {
                    group: 0,
                    column: 1,
                    left: 1200,
                    top: 0
                },
                Container {
                    group: 1,
                    column: 0,
                    left: 0,
                    top: 500 + COLUMN_GROUP_GAP
                },
            ]
        );
    }

    use crate::model::{
        Anchor, BoxUnits, PositionedObject, SourceAnchor, Token, TokenKind, TokenSourceRange,
    };

    fn test_token(kind: TokenKind, logical_len: usize) -> Token {
        Token {
            kind,
            logical_len,
            char_style_id: 0,
            hyperlink: None,
        }
    }

    fn test_table(anchor: Option<SourceAnchor>) -> Table {
        Table {
            id: "t0".to_owned(),
            source_path: "section.xml".to_owned(),
            source_anchor: anchor,
            caption: None,
            section_index: 0,
            anchor_y: Some(0),
            anchor_top_adjustment: 0,
            anchor_paragraph_left: 0,
            anchor_paragraph_right: 0,
            box_units: BoxUnits {
                width: 1000,
                height: 500,
                ..BoxUnits::default()
            },
            anchor: Anchor {
                treat_as_char: true,
                ..Anchor::default()
            },
            out_margin_left: 0,
            out_margin_right: 0,
            out_margin_top: 0,
            out_margin_bottom: 0,
            columns: 1,
            rows: 1,
            row_heights: vec![500],
            cells: Vec::new(),
            page_break: "CELL".to_owned(),
            repeat_header: false,
            no_adjust: false,
            fragment_rows: None,
        }
    }

    fn test_object(kind: &str, anchor: Option<SourceAnchor>, textpos: usize) -> PositionedObject {
        PositionedObject {
            id: "o0".to_owned(),
            key: "o0".to_owned(),
            source_path: "section.xml".to_owned(),
            source_anchor: anchor,
            caption: None,
            equation: None,
            textpos,
            kind: kind.to_owned(),
            box_units: BoxUnits {
                width: 800,
                height: 400,
                ..BoxUnits::default()
            },
            anchor: Anchor {
                treat_as_char: true,
                ..Anchor::default()
            },
            alt: String::new(),
            description: String::new(),
            paragraph_key: String::new(),
            binary_ref: None,
            mime_type: None,
            crop: None,
            img_dim: BoxUnits::default(),
            original_size: BoxUnits::default(),
            flip_x: false,
            flip_y: false,
            stacking_order: 0,
            shape: None,
            children: Vec::new(),
        }
    }

    #[test]
    fn line_items_places_table_before_tokens_matching_source_anchor() {
        let mut line = line(0, 500, 0, (1, 0));
        line.paragraph_key = "p1".to_string();
        line.tokens = vec![
            test_token(
                TokenKind::Control {
                    kind: "tbl".to_string(),
                },
                8,
            ),
            test_token(TokenKind::FixedSpace, 1),
            test_token(TokenKind::Text("text".to_string()), 4),
        ];
        line.token_sources = vec![
            TokenSourceRange {
                token_index: 0,
                logical: 0..8,
                utf8: None,
            },
            TokenSourceRange {
                token_index: 1,
                logical: 8..9,
                utf8: None,
            },
            TokenSourceRange {
                token_index: 2,
                logical: 9..13,
                utf8: None,
            },
        ];
        let table = test_table(Some(SourceAnchor {
            paragraph_key: "p1".to_string(),
            textpos: 0,
        }));
        let (items, unmatched_tables, unmatched_objects) =
            line_items_with_fallback_counts(&line, &[&table]);
        assert_eq!(
            items,
            vec![
                LineItem::Table(0),
                LineItem::Token(0),
                LineItem::Token(1),
                LineItem::Token(2),
            ]
        );
        assert_eq!(unmatched_tables, 0);
        assert_eq!(unmatched_objects, 0);
    }

    #[test]
    fn line_items_places_inline_object_between_surrounding_tokens() {
        let mut line = line(0, 500, 0, (1, 0));
        line.paragraph_key = "p1".to_string();
        line.tokens = vec![
            test_token(TokenKind::Text("prefix ".to_string()), 7),
            test_token(
                TokenKind::Control {
                    kind: "equation".to_string(),
                },
                8,
            ),
            test_token(TokenKind::Text(" suffix".to_string()), 7),
        ];
        line.token_sources = vec![
            TokenSourceRange {
                token_index: 0,
                logical: 0..7,
                utf8: None,
            },
            TokenSourceRange {
                token_index: 1,
                logical: 7..15,
                utf8: None,
            },
            TokenSourceRange {
                token_index: 2,
                logical: 15..22,
                utf8: None,
            },
        ];
        line.inline_objects = vec![test_object(
            "equation",
            Some(SourceAnchor {
                paragraph_key: "p1".to_string(),
                textpos: 7,
            }),
            7,
        )];
        let (items, unmatched_tables, unmatched_objects) =
            line_items_with_fallback_counts(&line, &[]);
        assert_eq!(
            items,
            vec![
                LineItem::Token(0),
                LineItem::Object(0),
                LineItem::Token(1),
                LineItem::Token(2),
            ]
        );
        assert_eq!(unmatched_tables, 0);
        assert_eq!(unmatched_objects, 0);
    }

    #[test]
    fn line_items_falls_back_unmatched_boxes_to_end_of_line() {
        let mut line = line(0, 500, 0, (1, 0));
        line.paragraph_key = "p1".to_string();
        line.tokens = vec![test_token(TokenKind::Text("only text".to_string()), 9)];
        line.token_sources = vec![TokenSourceRange {
            token_index: 0,
            logical: 0..9,
            utf8: None,
        }];
        let table = test_table(Some(SourceAnchor {
            paragraph_key: "other_para".to_string(),
            textpos: 0,
        }));
        let (items, unmatched_tables, unmatched_objects) =
            line_items_with_fallback_counts(&line, &[&table]);
        assert_eq!(items, vec![LineItem::Token(0), LineItem::Table(0)]);
        assert_eq!(unmatched_tables, 1);
        assert_eq!(unmatched_objects, 0);
    }
}
