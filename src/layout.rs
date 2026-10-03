use sha2::{Digest, Sha256};

pub mod presentation;

use crate::model::{
    Block, BoxUnits, Document, HwpUnit, InlineContent, LayoutDocument, LayoutPage, LineFill,
    LineFragment, LineSeg, Paragraph, PlacedTable, PositionedObject, Section, Table, Token,
    TokenKind, TokenSourceRange,
};

const TABLE_EDGE_ALLOWANCE: i64 = 661;
const MIN_ROW_SPLIT_HEIGHT: i64 = 2000;
const SPLIT_PARAGRAPH_EXTRA: i64 = 123;
const SPLIT_PAGE_ALLOWANCE: i64 = 101;
pub const LINE_TOP_OFFSET: i64 = 75;

/// The final centi-mm conversion is deliberately the only place where layout
/// values are rounded. All placement decisions stay in HWPUNIT.
/// A value exactly halfway rounds toward zero, not away from it: the
/// reference writes `top:59.05mm` where 59.055 is exact, and `height:3.17mm`
/// for a 900 HWPUNIT glyph box whose exact conversion is 3.175mm.
pub fn round_div(numerator: i128, denominator: i128) -> i64 {
    debug_assert!(denominator > 0);
    let magnitude = numerator.unsigned_abs();
    let step = denominator.unsigned_abs();
    let rounded = (magnitude * 2 + step - 1) / (step * 2);
    if numerator >= 0 {
        rounded as i64
    } else {
        -(rounded as i64)
    }
}

pub fn hwp_to_centi_mm(units: i64) -> i64 {
    round_div(i128::from(units) * 127, 360)
}

pub fn css_mm(units: i64) -> String {
    let centi = hwp_to_centi_mm(units);
    let sign = if centi < 0 { "-" } else { "" };
    let absolute = centi.unsigned_abs();
    let whole = absolute / 100;
    let fraction = absolute % 100;
    if fraction == 0 {
        format!("{sign}{whole}mm")
    } else {
        format!("{sign}{whole}.{fraction:02}mm")
    }
}

/// Same conversion, but keeping the hundredths even when they are zero.
///
/// The reference drops a zero fraction everywhere (`height:24mm`,
/// `left:0mm`, and an inline object box's own `line-height:62mm`) except in
/// one place: the `line-height` of an `hls` line box, which it always writes
/// with two decimals. Across the whole sample set the split is clean, with no
/// counter-example either way -- 37 `hls` line-heights are written `x.00mm`
/// and none plain, while 33 object-box line-heights are written plain and
/// none as `x.00mm`.
pub fn css_mm_hundredths(units: i64) -> String {
    let centi = hwp_to_centi_mm(units);
    let sign = if centi < 0 { "-" } else { "" };
    let absolute = centi.unsigned_abs();
    format!("{sign}{}.{:02}mm", absolute / 100, absolute % 100)
}

pub fn layout_document(document: &Document) -> LayoutDocument {
    let mut pages = Vec::new();
    let mut page_number = document.page_numbers.start;
    let mut page_numbers_active = false;
    for section in &document.sections {
        page_numbers_active |= section.page.page_number_enabled;
        let mut current = new_page(
            pages.len(),
            section,
            page_number_text(
                document,
                page_number,
                section,
                pages.is_empty(),
                page_numbers_active,
            ),
        );
        let mut previous_top = None;
        // Where the flow ended on a page: the last line's advance (and its
        // paragraph's space after), or a floating table's bottom. A paragraph
        // starts there; its own top/bottom floats push its first line lower.
        let mut flow_end: Option<(usize, i64)> = None;
        let mut floating_anchor_restart_pending = false;
        // A PARA-anchored floating table's `anchor_y` is a raw, parse-time
        // coordinate that assumes no page break falls between it and
        // whatever came before. Once a push happens for a reason that
        // coordinate can't see (the checks below), every later table in the
        // same raw coordinate space needs the same correction -- otherwise
        // it renders as if that push never happened. See 샘플/다수 부동표's
        // third and fourth stacked box.
        let mut table_anchor_shift = 0i64;
        // Keep the preceding source line's kind separate from floating table
        // fragment coordinates. A blank line after an inline table can be the
        // first real line on the next page, not a redundant floating anchor.
        let mut previous_line_has_inline_table = false;
        let mut previous_source_line_advance = 0;
        let mut current_columns = (1_u32, 0_i64);
        let mut column_index = 0_u32;
        let mut line_counter = 0usize;
        let mut previous_paragraph_line: Option<(i64, i64, bool)> = None;
        let mut intact_stack_tables = 0usize;
        // Reverse order so `pop()` yields the group's first table first.
        let mut height_stack_positions: Vec<i64> = Vec::new();
        // Bottom of the previous table in an intact `compact_float_stack`
        // group. Top/bottom floats never overlap: 성과보고서 sections 37-42
        // declare offsets up to 4 HWPUNIT inside the previous table, and the
        // reference pushes each one to that bottom (46 of 50 tops exact,
        // against 9 of 50 from the raw offsets).
        let mut stack_bottom: Option<i64> = None;
        // The section's first paragraph can record its line below its own
        // top/bottom floats, which the parser anchors at the page top. Those
        // floats precede the line on the same page; they are not a restart.
        let mut anchor_line_follows_floats = false;
        // A paragraph whose only visible content is a *leftover space*
        // (non-empty text that trims to nothing), anchoring a following
        // floating (non-inline) table, carries no meaning of its own once
        // that table is placed: it exists only to host the table's field
        // markers. When it is also the last thing before a page break --
        // forced or from the table needing to move or split -- its own
        // trailing line never reaches the page at all in the reference --
        // see 성과보고서 s3's single leftover space before "◈ 전략목표Ⅱ"'s
        // own fieldEnd (rhwp's independent layout drops the same line; its
        // plain-text page dump has no trace of it). A genuinely *empty*
        // paragraph (no text token at all) anchoring a floating table is
        // the ordinary, ubiquitous way HWP anchors one at all -- its own
        // line is real, visible content in the reference (confirmed by the
        // corpus regression: requiring non-empty-but-blank text here, not
        // simply trim-empty, was the difference between fixing s3's one
        // line and silently deleting a real line from every other
        // anchor-only paragraph in 55 other samples).
        let mut trailing_trivial_anchor_paragraph: Option<String> = None;
        for (block_index, block) in section.blocks.iter().enumerate() {
            match block {
                Block::Paragraph(source_paragraph) => {
                    // A restart in a newspaper layout moves the flow to the next
                    // column (`current_columns`, below), not to a new page.
                    let carried_paragraph = (current_columns.0 <= 1
                        && source_paragraph
                            .columns
                            .is_none_or(|(columns, _)| columns <= 1))
                    .then(|| paragraph_with_pushed_lines_carried(source_paragraph))
                    .flatten();
                    let paragraph = carried_paragraph.as_ref().unwrap_or(source_paragraph);
                    let pending_trivial_drop = trailing_trivial_anchor_paragraph.take();
                    // Each source paragraph's own `anchor_y` (used by any
                    // table(s) it carries) is a fresh raw coordinate with no
                    // relation to an earlier, unrelated paragraph's tables
                    // needing a mid-run push. Only tables sharing this one
                    // anchor paragraph -- consecutive `Block::Table` entries,
                    // never separated by another paragraph -- should inherit
                    // each other's shift.
                    table_anchor_shift = 0;
                    intact_stack_tables = 0;
                    height_stack_positions.clear();
                    stack_bottom = None;
                    anchor_line_follows_floats = block_index == 0
                        && paragraph.lines.first().is_some_and(|line| line.top > 0);
                    let content_height = section.page.height
                        - section.page.margin_top
                        - section.page.header
                        - section.page.margin_bottom
                        - section.page.footer;
                    if let Some((count, first, bottom)) = compact_float_stack(
                        paragraph,
                        &section.blocks[block_index + 1..],
                        content_height,
                    ) {
                        // A stack measured from the page top can start at
                        // minus the anchor line's glyph adjustment, which the
                        // renderer adds back.
                        let first_adjustment = match section.blocks.get(block_index + 1) {
                            Some(Block::Table(table)) => table.anchor_top_adjustment,
                            _ => 0,
                        };
                        if bottom <= content_height && first + first_adjustment >= 0 {
                            intact_stack_tables = count;
                        } else if let (Some(line), Some((prior_top, prior_advance, true))) =
                            (paragraph.lines.first(), previous_paragraph_line)
                        {
                            let desired = if prior_top == 0 {
                                Some(prior_advance)
                            } else if line.top < prior_top {
                                Some(0)
                            } else {
                                None
                            };
                            if let Some(desired) =
                                desired.filter(|top| top + bottom - first <= content_height)
                            {
                                if !current.lines.is_empty()
                                    || !current.tables.is_empty()
                                    || !current.objects.is_empty()
                                {
                                    drop_pending_trivial_line(
                                        &mut current,
                                        pending_trivial_drop.as_deref(),
                                    );
                                    pages.push(current);
                                    page_number += 1;
                                    current = new_page(
                                        pages.len(),
                                        section,
                                        page_number_text(
                                            document,
                                            page_number,
                                            section,
                                            false,
                                            page_numbers_active,
                                        ),
                                    );
                                }
                                previous_top = None;
                                line_counter = 0;
                                table_anchor_shift = first - desired;
                                intact_stack_tables = count;
                            }
                        }
                    } else if let Some(positions) =
                        compact_float_stack_by_height(paragraph, &section.blocks[block_index + 1..])
                    {
                        intact_stack_tables = positions.len();
                        height_stack_positions = positions.into_iter().rev().collect();
                    }
                    previous_paragraph_line = paragraph.lines.last().map(|line| {
                        (
                            line.top,
                            line.height.saturating_add(line.spacing),
                            paragraph.tokens.iter().all(|t| t.visible_text().is_empty())
                                && paragraph.objects.is_empty()
                                && !paragraph.has_inline_table,
                        )
                    });
                    trailing_trivial_anchor_paragraph = (is_whitespace_only_text(paragraph)
                        && section.blocks[block_index + 1..]
                            .iter()
                            .take_while(|later| matches!(later, Block::Table(_)))
                            .any(
                                |later| matches!(later, Block::Table(t) if !t.anchor.treat_as_char),
                            ))
                    .then(|| paragraph.id.clone());
                    if paragraph.page_break
                        && (!current.lines.is_empty() || !current.tables.is_empty())
                    {
                        drop_pending_trivial_line(&mut current, pending_trivial_drop.as_deref());
                        pages.push(current);
                        page_number += 1;
                        current = new_page(
                            pages.len(),
                            section,
                            page_number_text(
                                document,
                                page_number,
                                section,
                                false,
                                page_numbers_active,
                            ),
                        );
                        line_counter = 0;
                    }
                    if let Some(restart) = paragraph.page_number_restart {
                        page_number = restart;
                        current.page_number = page_number_text(
                            document,
                            page_number,
                            section,
                            pages.is_empty(),
                            page_numbers_active,
                        );
                    }
                    // A paragraph's own colPr replaces the layout from here on
                    // and starts that layout's first column afresh.
                    // Every colPr starts a new column group, even one that
                    // repeats the current settings: 제어정보길이_검증6 has two
                    // consecutive two-column paragraphs and the reference
                    // gives each its own group of containers.
                    let starts_column_group = paragraph.columns.is_some();
                    if let Some(columns) = paragraph.columns {
                        current_columns = columns;
                        column_index = 0;
                    }
                    let fragments = paragraph_fragments(paragraph, line_counter);
                    // A whitespace-only paragraph that directly hosts a
                    // floating table records its line at the table's trailing
                    // page position. Its coordinate restart must not advance
                    // the page before that table gets a chance to lay out.
                    let is_floating_table_anchor = is_whitespace_only_text(paragraph)
                        && section.blocks[block_index + 1..]
                            .iter()
                            .take_while(|later| matches!(later, Block::Table(_)))
                            .any(|later| {
                                matches!(later, Block::Table(table) if !table.anchor.treat_as_char)
                            });
                    // A PARA float measures from the paragraph's top: where the
                    // flow ended on its page (0 on a fresh page), unless its
                    // first line less its space before is higher. 성과보고서
                    // s4 p[8] starts a page with a stack of top/bottom tables
                    // and its line under them sits at 39261; the rule line it
                    // also carries (vertOffset 39260) measures from 0. When the
                    // paragraph started on an earlier page, its objects keep
                    // the last line's flow position (`inline_top`, not the
                    // text's glyph-box pullback).
                    let mut paragraph_top = None;
                    let line_count = paragraph.lines.len();
                    let mut object_anchor_top = None;
                    for (source_line_index, mut fragment) in fragments.into_iter().enumerate() {
                        if intact_stack_tables > 0 {
                            fragment.top -= table_anchor_shift;
                            fragment.inline_top -= table_anchor_shift;
                        }
                        // Tall inline tables have a large glyph-box adjustment.
                        // Their unadjusted line position governs page flow; an
                        // empty text stream still carries the table's anchor.
                        let flow_top = if paragraph.has_inline_table {
                            fragment.inline_top
                        } else {
                            fragment.top
                        };
                        if let Some(top) = previous_top {
                            // Both source lines start at page origin. The blank
                            // line consumed positive flow space, so a following
                            // inline table at the same origin starts a new page.
                            // Comparing CSS tops (-textheight/20 versus 0) hides
                            // this reset. Do not infer it from a table's bottom.
                            // Two consecutive inline-table lines at the page
                            // origin are two pages: 통합관리요령's 심의요청서 table
                            // (63312 tall) and its 목차 table both start at 0.
                            let inline_origin_restart = paragraph.has_inline_table
                                && previous_line_has_inline_table
                                && fragment.inline_top == 0
                                && top == 0
                                && previous_source_line_advance > 0;
                            let origin_restart = inline_origin_restart
                                || (paragraph.has_inline_table
                                    && fragment.inline_top == 0
                                    && !previous_line_has_inline_table
                                    && previous_source_line_advance > 0
                                    && current.tables.is_empty()
                                    && current.objects.is_empty()
                                    && current.lines.last().is_some_and(|line| {
                                        line.inline_top == 0
                                            && line.width > 0
                                            && line
                                                .tokens
                                                .iter()
                                                .all(|t| t.visible_text().is_empty())
                                    }));
                            // In a newspaper layout a coordinate restart means
                            // the flow moved to the next column, and only the
                            // last column's restart reaches a new page.
                            // A restart that only marks the start of the new
                            // layout keeps the flow on this page; the group
                            // simply begins lower down.
                            let group_start_restart = starts_column_group && source_line_index == 0;
                            if flow_top < top
                                && (group_start_restart
                                    || (current_columns.0 > 1
                                        && column_index + 1 < current_columns.0))
                                && (!current.lines.is_empty() || !current.tables.is_empty())
                            {
                                if !group_start_restart {
                                    column_index += 1;
                                }
                                previous_top = Some(flow_top);
                                fragment.columns = current_columns;
                                fragment.column_index = column_index;
                                current.lines.push(fragment);
                                line_counter += 1;
                                continue;
                            }
                            if (flow_top < top || origin_restart)
                                && (!current.lines.is_empty() || !current.tables.is_empty())
                            {
                                if is_floating_table_anchor {
                                    floating_anchor_restart_pending = true;
                                }
                                if !is_floating_table_anchor {
                                    // An embedded-table placeholder token (see
                                    // section.rs's "tbl" case) carries no visible
                                    // text, same as any other Control token; a
                                    // paragraph that pairs one with nothing else
                                    // is still a blank line for this check.
                                    // An origin reset after a page-filling float
                                    // starts real blank lines on the next page.
                                    // Skipping that first line loses the only
                                    // page-break signal before the next float.
                                    let float_fills_page = fragment.inline_top == 0
                                        && current.tables.len() >= 3
                                        && current.tables[0].table.anchor.treat_as_char
                                        && current.tables.last().is_some_and(|placed| {
                                            !placed.table.anchor.treat_as_char
                                                && placed.table.box_units.height * 4
                                                    > content_height * 3
                                                && placed
                                                    .table
                                                    .box_units
                                                    .y
                                                    .saturating_add(placed.table.box_units.height)
                                                    .max(top.saturating_add(
                                                        previous_source_line_advance,
                                                    ))
                                                    .saturating_add(previous_source_line_advance)
                                                    > content_height
                                        });
                                    if paragraph
                                        .tokens
                                        .iter()
                                        .all(|token| token.visible_text().is_empty())
                                        && paragraph.objects.is_empty()
                                        && !paragraph.has_inline_table
                                        && !previous_line_has_inline_table
                                        && !current.tables.is_empty()
                                        && !float_fills_page
                                    {
                                        continue;
                                    }
                                    drop_pending_trivial_line(
                                        &mut current,
                                        pending_trivial_drop.as_deref(),
                                    );
                                    pages.push(current);
                                    page_number += 1;
                                    current = new_page(
                                        pages.len(),
                                        section,
                                        page_number_text(
                                            document,
                                            page_number,
                                            section,
                                            false,
                                            page_numbers_active,
                                        ),
                                    );
                                    line_counter = 0;
                                    column_index = 0;
                                }
                            }
                        }
                        fragment.columns = current_columns;
                        fragment.column_index = column_index;
                        previous_top = Some(flow_top);
                        object_anchor_top = Some(fragment.inline_top);
                        if source_line_index == 0 {
                            let page_flow_end = match flow_end {
                                Some((page, end)) if page == current.index => end,
                                _ => 0,
                            };
                            paragraph_top = Some((
                                current.index,
                                page_flow_end.min(para_float_anchor(
                                    fragment.inline_top,
                                    paragraph.para_style.margin_before,
                                )),
                            ));
                        }
                        previous_line_has_inline_table = paragraph.has_inline_table;
                        previous_source_line_advance = paragraph
                            .lines
                            .get(source_line_index)
                            .map_or(0, |line| line.height.saturating_add(line.spacing));
                        flow_end = Some((
                            current.index,
                            fragment
                                .inline_top
                                .saturating_add(previous_source_line_advance)
                                .saturating_add(if source_line_index + 1 == line_count {
                                    paragraph.para_style.margin_after.max(0)
                                } else {
                                    0
                                }),
                        ));
                        current.lines.push(fragment);
                        line_counter += 1;
                    }
                    for object in &paragraph.objects {
                        if !object.anchor.treat_as_char {
                            let anchor_top = match paragraph_top {
                                Some((page, top)) if page == current.index => top,
                                _ => object_anchor_top.or(previous_top).unwrap_or(0),
                            };
                            let mut placed = position_object(object, anchor_top, current.index);
                            placed.paragraph_key = paragraph.key.clone();
                            current.objects.push(placed);
                        }
                    }
                }
                Block::Table(table) => {
                    let in_offset_stack =
                        intact_stack_tables > 0 && height_stack_positions.is_empty();
                    let fallback_y = if let Some(target_y) = height_stack_positions.pop() {
                        target_y - table.anchor.vert_offset
                    } else {
                        table
                            .anchor_y
                            .map(|anchor_y| anchor_y - table_anchor_shift)
                            .or(previous_top)
                            .unwrap_or(0)
                    };
                    let fallback_y = match stack_bottom {
                        Some(bottom) if in_offset_stack => {
                            let declared_y = table_position(table, fallback_y).y;
                            fallback_y + bottom.saturating_sub(declared_y).max(0)
                        }
                        _ => fallback_y,
                    };
                    if in_offset_stack {
                        stack_bottom = Some(
                            table_position(table, fallback_y)
                                .y
                                .saturating_add(table.box_units.height),
                        );
                    }
                    let mut layout_fallback_y = fallback_y;
                    let mut y = table_position(table, layout_fallback_y).y;
                    let page_content_height = section
                        .page
                        .height
                        .saturating_sub(section.page.margin_top)
                        .saturating_sub(section.page.header)
                        .saturating_sub(section.page.margin_bottom)
                        .saturating_sub(section.page.footer)
                        .max(1);
                    if let Some(top) = previous_top.filter(|_| !anchor_line_follows_floats) {
                        // An inline table belongs to its anchor line. A tall
                        // one's glyph adjustment (textheight/20) can bring its
                        // anchor near 0 without any restart: 산업단지 유공자
                        // 공고's 65185-unit checklist table sits at 4051 - 3273.
                        let anchored_origin_restart = !table.anchor.treat_as_char
                            && table.anchor_y.is_some_and(|anchor_y| {
                                (-1000..=1000).contains(&anchor_y)
                                    && table.anchor.flow_with_text
                                    && table.anchor.vert_rel_to.eq_ignore_ascii_case("PARA")
                            });
                        if y < top
                            && (table.anchor_y.is_none() || anchored_origin_restart)
                            && (!current.lines.is_empty() || !current.tables.is_empty())
                            && (table.anchor_y.is_none() || !current.tables.is_empty())
                        {
                            drop_pending_trivial_line(
                                &mut current,
                                trailing_trivial_anchor_paragraph.as_deref(),
                            );
                            pages.push(current);
                            page_number += 1;
                            current = new_page(
                                pages.len(),
                                section,
                                page_number_text(
                                    document,
                                    page_number,
                                    section,
                                    false,
                                    page_numbers_active,
                                ),
                            );
                            previous_top = None;
                        }
                    }
                    // Floating table offsets are page-local in the HWPX
                    // exporter. When an anchored table crosses the usable
                    // page height, move its coordinate origin together with
                    // the page before calculating row capacity. Otherwise the
                    // splitter sees a negative first capacity and emits a
                    // one-HWPUNIT fragment that rounds to 0.00mm.
                    if !table.anchor.treat_as_char && table.anchor.flow_with_text {
                        while y > page_content_height {
                            if !current.lines.is_empty()
                                || !current.tables.is_empty()
                                || !current.objects.is_empty()
                            {
                                drop_pending_trivial_line(
                                    &mut current,
                                    trailing_trivial_anchor_paragraph.as_deref(),
                                );
                                pages.push(current);
                                page_number += 1;
                                current = new_page(
                                    pages.len(),
                                    section,
                                    page_number_text(
                                        document,
                                        page_number,
                                        section,
                                        false,
                                        page_numbers_active,
                                    ),
                                );
                                previous_top = None;
                            }
                            table_anchor_shift += page_content_height;
                            layout_fallback_y =
                                layout_fallback_y.saturating_sub(page_content_height);
                            y = table_position(table, layout_fallback_y).y;
                        }
                    }
                    let mut laid_out_table = layout_table(table, layout_fallback_y);
                    if intact_stack_tables > 0 && table_anchor_shift != 0 {
                        // `anchor_top_adjustment` exists so the render side can
                        // add back the glyph-box amount that `anchor_y`
                        // subtracted at parse time, recovering the float's raw,
                        // unadjusted line position -- see the render-side
                        // comment at its one call site. That cancellation only
                        // holds when `box_units.y` still descends from the
                        // table's own `anchor_y`. A compacted stack's shift
                        // instead re-derives the position from `desired` (this
                        // paragraph's or the prior one's own top), which never
                        // carried that subtraction, so adding the adjustment
                        // back here would be a bare, uncancelled +50 HWPUNIT
                        // (성과보고서 s4's two stacks: reference has their
                        // first table flush with the prior content, ours sat
                        // 0.17-0.18mm below it until this was zeroed). Scoped to
                        // an actual shift: a stack that already fit natively
                        // (`table_anchor_shift` left at 0, including the
                        // height-based stack below, which derives its position
                        // without ever touching `anchor_y`) still needs this
                        // cancellation to hold.
                        laid_out_table.anchor_top_adjustment = 0;
                    }
                    // A table whose first row doesn't fit what's left of this
                    // page moves whole to a fresh one, landing at its own top
                    // edge (raw y 0), rather than starting a razor-thin split
                    // here -- 샘플/다수 부동표's short info boxes needed
                    // this. Scoped to tables that fall through to the legacy
                    // byte-offset splitter (`rows!=1 && columns!=1`):
                    // `row_restart_pieces` (the `rows==1 || columns==1`
                    // path) already has its own, separately-verified
                    // fragment-boundary logic, and forcing a whole-table
                    // move ahead of it regressed 다중행_표4's single-row
                    // table (`rows==1`, an oversized first row that must
                    // still split starting here, not divert to a fresh
                    // page).
                    // A 1x1 CELL table without coordinate restarts is drawn as
                    // one unsplit fragment however tall it is. When it does not
                    // fit below existing content, the reference starts it on a
                    // fresh page (샘플/다중 문단 셀 콘텐츠 높이's 725.76mm table
                    // after the previous table's last fragment).
                    let unsplit_single_cell =
                        laid_out_table.page_break.eq_ignore_ascii_case("CELL")
                            && split_single_cell_table_by_coordinate_restarts(&laid_out_table)
                                .is_some_and(|fragments| fragments.len() == 1);
                    if !table.anchor.treat_as_char
                        && table.anchor.flow_with_text
                        && ((laid_out_table.rows != 1 && laid_out_table.columns != 1)
                            || unsplit_single_cell)
                    {
                        let first_capacity = page_content_height
                            .saturating_sub(y)
                            .saturating_sub(TABLE_EDGE_ALLOWANCE);
                        let first_row_height =
                            laid_out_table.row_heights.first().copied().unwrap_or(0);
                        if first_row_height > first_capacity
                            && (unsplit_single_cell
                                || first_row_height
                                    <= page_content_height.saturating_sub(TABLE_EDGE_ALLOWANCE))
                            && (!current.lines.is_empty()
                                || !current.tables.is_empty()
                                || !current.objects.is_empty())
                        {
                            drop_pending_trivial_line(
                                &mut current,
                                trailing_trivial_anchor_paragraph.as_deref(),
                            );
                            pages.push(current);
                            page_number += 1;
                            current = new_page(
                                pages.len(),
                                section,
                                page_number_text(
                                    document,
                                    page_number,
                                    section,
                                    false,
                                    page_numbers_active,
                                ),
                            );
                            previous_top = None;
                            table_anchor_shift += y;
                            layout_fallback_y = layout_fallback_y.saturating_sub(y);
                            laid_out_table = layout_table(table, layout_fallback_y);
                            if unsplit_single_cell {
                                // It now starts at the page top, not at its
                                // anchor line, so the line's glyph adjustment
                                // does not follow (reference top 35.99mm).
                                laid_out_table.anchor_top_adjustment = 0;
                            }
                        }
                    }
                    let intact_stack = intact_stack_tables > 0;
                    if intact_stack {
                        intact_stack_tables -= 1;
                    }
                    let fragments_of = |laid_out: Table| {
                        if intact_stack {
                            vec![laid_out]
                        } else {
                            split_table(
                                &laid_out,
                                &section.page,
                                table.box_units.height,
                                stale_no_adjust_header(table),
                                declared_grid_total(table),
                            )
                        }
                    };
                    let mut fragments = fragments_of(laid_out_table);
                    // A table kept whole whose body ends inside the page but
                    // whose bottom outer margin does not goes whole to the
                    // next page, as the reference does: 다수 부동표's third
                    // stacked box ends at 69926 of 70015 and its outMargin 141
                    // makes 70067. A table that does not flow with the text
                    // (성과보고서 p163, flowWithText=0) stays where it is.
                    if !table.anchor.treat_as_char
                        && table.anchor.flow_with_text
                        && y > 0
                        && fragments.len() == 1
                        && fragments[0]
                            .box_units
                            .y
                            .saturating_add(fragments[0].box_units.height)
                            <= page_content_height
                        && fragments[0]
                            .box_units
                            .y
                            .saturating_add(fragments[0].box_units.height)
                            .saturating_add(fragments[0].out_margin_bottom.max(0))
                            > page_content_height
                        && (!current.lines.is_empty()
                            || !current.tables.is_empty()
                            || !current.objects.is_empty())
                    {
                        drop_pending_trivial_line(
                            &mut current,
                            trailing_trivial_anchor_paragraph.as_deref(),
                        );
                        pages.push(current);
                        page_number += 1;
                        current = new_page(
                            pages.len(),
                            section,
                            page_number_text(
                                document,
                                page_number,
                                section,
                                false,
                                page_numbers_active,
                            ),
                        );
                        previous_top = None;
                        table_anchor_shift += y;
                        layout_fallback_y = layout_fallback_y.saturating_sub(y);
                        let moved = layout_table(table, layout_fallback_y);
                        if in_offset_stack {
                            stack_bottom = Some(
                                table_position(table, layout_fallback_y)
                                    .y
                                    .saturating_add(moved.box_units.height),
                            );
                        }
                        fragments = fragments_of(moved);
                    }
                    for (fragment_index, fragment) in fragments.into_iter().enumerate() {
                        if fragment_index > 0 {
                            drop_pending_trivial_line(
                                &mut current,
                                trailing_trivial_anchor_paragraph.as_deref(),
                            );
                            pages.push(current);
                            page_number += 1;
                            current = new_page(
                                pages.len(),
                                section,
                                page_number_text(
                                    document,
                                    page_number,
                                    section,
                                    false,
                                    page_numbers_active,
                                ),
                            );
                        }
                        current.tables.push(PlacedTable {
                            table: fragment.clone(),
                            page_index: current.index,
                            fragment_index,
                        });
                        if !fragment.anchor.treat_as_char {
                            // A float drawn above the page top (negative
                            // offset) leaves the flow at the page top.
                            let bottom = (fragment.box_units.y + fragment.box_units.height).max(0);
                            flow_end = Some(match flow_end {
                                Some((page, end)) if page == current.index => {
                                    (page, end.max(bottom))
                                }
                                _ => (current.index, bottom),
                            });
                        }
                        // Inline objects are already represented by their
                        // source paragraph line. Its origin, not the object's
                        // bottom edge, determines whether a following line
                        // resets onto another page (negative spacing is valid).
                        if fragment_index > 0 {
                            previous_top = Some(fragment.box_units.y);
                            previous_line_has_inline_table = false;
                        } else if floating_anchor_restart_pending && !fragment.anchor.treat_as_char
                        {
                            // A floating table occupies the page through its
                            // bottom edge.  Keep the flow cursor above that
                            // edge so a following coordinate restart opens
                            // the next page instead of being compared with the
                            // empty anchor paragraph's origin.
                            previous_top = Some(fragment.box_units.y + fragment.box_units.height);
                            floating_anchor_restart_pending = false;
                        }
                    }
                }
                Block::Object(object) => {
                    current.objects.push(position_object(
                        object,
                        previous_top.unwrap_or(0),
                        current.index,
                    ));
                }
            }
        }
        if !current.lines.is_empty()
            || !current.tables.is_empty()
            || !current.objects.is_empty()
            || pages.is_empty()
        {
            drop_pending_trivial_line(&mut current, trailing_trivial_anchor_paragraph.as_deref());
            pages.push(current);
            page_number += 1;
        }
    }
    apply_page_number_visibility(document, &mut pages);
    for page in &mut pages {
        page.objects.sort_by_key(|object| object.anchor.z_order);
        for (order, object) in page.objects.iter_mut().enumerate() {
            object.stacking_order = order as i64;
        }
    }
    LayoutDocument {
        title: document.title.clone(),
        input_sha256: document.input_sha256.clone(),
        pages,
        char_styles: document.char_styles.clone(),
        para_styles: document.para_styles.clone(),
        assets: document.assets.clone(),
        warnings: document.warnings.clone(),
    }
}

/// True for a paragraph with at least one non-empty text token whose
/// combined visible text is nonetheless all whitespace -- a leftover space
/// beside a field marker, not the ordinary zero-text paragraph HWP anchors
/// most floating tables to. Deliberately distinct from simple
/// trim-emptiness: that also matches a genuinely empty paragraph, whose own
/// line is real, visible content in the reference.
fn is_whitespace_only_text(paragraph: &Paragraph) -> bool {
    let text: String = paragraph.tokens.iter().map(|t| t.visible_text()).collect();
    !text.is_empty() && text.trim().is_empty()
}

/// Drops `current`'s last line if it is exactly the pending trivial-anchor
/// paragraph's own line -- see `trailing_trivial_anchor_paragraph`'s own
/// comment above. Called at every place a page gets finalized (a forced
/// break, a natural overflow, or a floating table that must move to or
/// split across a fresh page), since which of those actually applies to a
/// given document is exactly the part this rule must stay independent of.
fn drop_pending_trivial_line(current: &mut LayoutPage, pending: Option<&str>) {
    if let Some(trivial_id) = pending {
        if current.lines.last().is_some_and(|line| {
            line.paragraph_id == trivial_id
                && !line
                    .tokens
                    .iter()
                    .map(Token::visible_text)
                    .collect::<String>()
                    .is_empty()
                && line
                    .tokens
                    .iter()
                    .all(|t| t.visible_text().trim().is_empty())
        }) {
            current.lines.pop();
        }
    }
}

/// Clears the page number on the pages where the reference draws none. The
/// counter itself is never touched: a page whose number is not drawn still
/// consumes its number.
///
/// * A page before the one holding the first `pageNum` control. Numbering
///   starts at the control's page, not at its section's first page. The work
///   report's only `pageNum` is in s0 para 25 (page 2), and its cover (page 1)
///   has no number.
/// * The page holding a `pageHiding hidePageNum="1"` control ("현재 쪽만
///   감추기"). Only that page is hidden: 통합관리요령 공고 draws 「- 53 -」,
///   nothing, 「- 55 -」 around the hidden 별지 목차 (s2 para 1139, 1405), and
///   the work report's 목차 (s0 para 25) and 어린이제품 개정령안's 연락처 (s4
///   para 0) are the same.
///
/// This runs once all pages exist rather than when a paragraph starts, since
/// a host paragraph that opens a new page by a coordinate restart (not a
/// `pageBreak`) would otherwise be seen on the page before.
///
/// A control's page is taken to be the first page its top-level host
/// paragraph reaches. Every such paragraph in the corpus lies on one page; a
/// control past the first page of a multi-page paragraph would need its
/// textpos, which no sample exercises. A `pageNum` whose paragraph cannot be
/// located leaves every page numbered, as before.
fn apply_page_number_visibility(document: &Document, pages: &mut [LayoutPage]) {
    let first_page = |key: &str| {
        pages.iter().position(|page| {
            page.lines.iter().any(|line| line.paragraph_key == key)
                || page.tables.iter().any(|placed| {
                    placed
                        .table
                        .source_anchor
                        .as_ref()
                        .is_some_and(|anchor| anchor.paragraph_key == key)
                })
                || page.objects.iter().any(|object| {
                    object
                        .source_anchor
                        .as_ref()
                        .is_some_and(|anchor| anchor.paragraph_key == key)
                })
        })
    };
    let paragraphs = document.sections.iter().flat_map(|section| {
        section.blocks.iter().filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            _ => None,
        })
    });
    let mut numbering_start = None;
    let mut hidden = Vec::new();
    for paragraph in paragraphs {
        if paragraph.page_number_control && numbering_start.is_none() {
            numbering_start = Some(first_page(&paragraph.key).unwrap_or(0));
        }
        if paragraph.hides_page_number {
            hidden.extend(first_page(&paragraph.key));
        }
    }
    for page in pages.iter_mut().take(numbering_start.unwrap_or(0)) {
        page.page_number = None;
    }
    for index in hidden {
        pages[index].page_number = None;
    }
}

fn new_page(index: usize, section: &Section, page_number: Option<String>) -> LayoutPage {
    LayoutPage {
        index,
        section_index: section.index,
        spec: section.page.clone(),
        lines: Vec::new(),
        tables: Vec::new(),
        objects: Vec::new(),
        page_number,
    }
}

fn page_number_text(
    document: &Document,
    value: i64,
    section: &Section,
    first_document_page: bool,
    active: bool,
) -> Option<String> {
    if !document.page_numbers.enabled
        || !active
        || (first_document_page && section.page.hide_first_page_number)
    {
        return None;
    }
    // The same format names as the numbers of the semantic tree (OWPML's
    // `numFormat`): `ROMAN_CAPITAL`/`ROMAN_SMALL`, `LATIN_*`, `CIRCLED_DIGIT`,
    // `HANGUL_SYLLABLE`. A page number in Roman letters is written in ASCII
    // (no sample draws one, so the choice between `iv` and `ⅳ` is open); a
    // format without a text for this number is written in digits.
    let format = document.page_numbers.format.to_ascii_uppercase();
    let number = match format.as_str() {
        "ROMAN_CAPITAL" | "ROMAN_UPPER" => roman(value),
        "ROMAN_SMALL" | "ROMAN_LOWER" => roman(value).to_ascii_lowercase(),
        _ => u32::try_from(value)
            .ok()
            .and_then(|number| crate::semantic::number_text(number, &format))
            .unwrap_or_else(|| value.to_string()),
    };
    Some(format!(
        "{} {} {}",
        document.page_numbers.side_char, number, document.page_numbers.side_char
    ))
}

fn roman(mut value: i64) -> String {
    if value <= 0 {
        return value.to_string();
    }
    let mut result = String::new();
    for (amount, symbol) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while value >= amount {
            result.push_str(symbol);
            value -= amount;
        }
    }
    result
}

pub fn paragraph_fragments_for_render(paragraph: &Paragraph) -> Vec<LineFragment> {
    paragraph_fragments(paragraph, 0)
}

/// The one path a table cell's own paragraphs render through
/// (`render_table_cell`). Unlike a top-level page paragraph -- whose own
/// restart is always resolved earlier, by starting a new page in
/// `layout_document` -- a cell's restart (a later line's `vertpos` resetting
/// below the line before it) can reach here completely unresolved: the
/// table's own splitting decision does not crop every such cell into
/// separate fragments at that exact point, but the source linesegarray still
/// marks where HWP would have started a new page. `resolve_restarts` walks
/// past that raw reset with an accumulated offset instead, so the cell's own
/// later lines render below the ones before them instead of on top --
/// 성과보고서's "바이오나노산업개방형생태계조성촉진사업" cell (row 35 of a
/// 100-row, 10-column table outside `row_restart_pieces`'s
/// `rows==1||columns==1` scope) had its own third line's `vertpos` reset to
/// 0, landing "진사업" directly on top of "바이오나노산업개" instead of below
/// "방형생태계조성촉". Scoped to this one call site: a top-level paragraph's
/// column-group restart is a different, already-correctly-handled shape
/// (`current_columns`/`column_index` in `layout_document`), not this one --
/// resolving it here as well regressed 제어정보길이_검증6's own column tops.
pub fn paragraph_fragments_for_render_with_metric_offset(
    paragraph: &Paragraph,
    use_metric_offset: bool,
) -> Vec<LineFragment> {
    paragraph_fragments_with_offset(paragraph, 0, use_metric_offset, true)
}

fn paragraph_fragments(paragraph: &Paragraph, line_offset: usize) -> Vec<LineFragment> {
    paragraph_fragments_with_offset(paragraph, line_offset, false, false)
}

fn paragraph_fragments_with_offset(
    paragraph: &Paragraph,
    line_offset: usize,
    use_metric_offset: bool,
    resolve_restarts: bool,
) -> Vec<LineFragment> {
    let lines = if paragraph.lines.is_empty() {
        vec![LineSeg {
            text_end: None,
            textpos: 0,
            top: 0,
            height: 1200,
            text_height: 1200,
            baseline: 0,
            spacing: 1200,
            left: 0,
            width: 0,
        }]
    } else {
        paragraph.lines.clone()
    };
    let total_len = paragraph
        .tokens
        .iter()
        .map(|token| token.logical_len)
        .sum::<usize>();
    let has_inline_object = paragraph
        .objects
        .iter()
        .any(|object| object.anchor.treat_as_char);
    // See `paragraph_fragments_for_render_with_metric_offset`'s own comment
    // for what `resolve_restarts` is walking past and why it is scoped to
    // that one call site.
    let mut restart_offset = 0_i64;
    let mut previous_raw_top: Option<i64> = None;
    let mut previous_effective_top = 0_i64;
    let mut previous_extent = 0_i64;
    let mut previous_spacing = 0_i64;
    let effective_tops = lines
        .iter()
        .map(|line| {
            let effective =
                if resolve_restarts && previous_raw_top.is_some_and(|top| line.top <= top) {
                    let continued = previous_effective_top
                        .saturating_add(previous_extent)
                        .saturating_add(previous_spacing);
                    restart_offset = continued.saturating_sub(line.top);
                    continued
                } else {
                    line.top.saturating_add(restart_offset)
                };
            previous_raw_top = Some(line.top);
            previous_effective_top = effective;
            previous_extent = line.height.max(line.text_height).max(0);
            previous_spacing = line.spacing.max(0);
            effective
        })
        .collect::<Vec<_>>();
    let paragraph_start = paragraph
        .tokens
        .iter()
        .map(Token::visible_text)
        .collect::<String>()
        .chars()
        .take(48)
        .collect::<String>();
    let paragraph_empty = paragraph
        .tokens
        .iter()
        .all(|token| token.visible_text().trim().is_empty())
        && !paragraph.has_inline_table
        && paragraph.para_style.bullet_marker.is_none()
        && !paragraph
            .objects
            .iter()
            .any(|object| object.anchor.treat_as_char)
        && paragraph.inline_sources.iter().all(|source| {
            !matches!(
                source.content,
                InlineContent::Ruby { .. } | InlineContent::Compose { .. }
            )
        });
    lines
        .iter()
        .zip(&effective_tops)
        .enumerate()
        .map(|(index, (line, &effective_top))| {
            let start = line.textpos.min(total_len);
            let end = line
                .text_end
                .or_else(|| lines.get(index + 1).map(|next| next.textpos))
                .unwrap_or(total_len)
                .max(start)
                .min(total_len);
            let (tokens, token_sources) = slice_tokens(&paragraph.tokens, start, end);
            let inline_objects = paragraph
                .objects
                .iter()
                .filter(|object| {
                    object.anchor.treat_as_char && start <= object.textpos && object.textpos < end
                })
                .cloned()
                .collect::<Vec<_>>();
            // The script does not measure a line holding an annotated or
            // overlapped part (the reference does not widen one either).
            let composed = tokens.iter().any(|token| {
                matches!(&token.kind, TokenKind::Control { kind } if kind == "dutmal" || kind == "compose")
            });
            let fill = if paragraph.has_inline_table || !inline_objects.is_empty() || composed {
                None
            } else {
                line_fill(&paragraph.para_style.align, end >= total_len, &tokens)
            };
            LineFragment {
                source_id: format!("{}/line[{index}]", paragraph.id),
                section_index: paragraph.section_index,
                paragraph_id: paragraph.id.clone(),
                paragraph_key: paragraph.key.clone(),
                source_range: start..end,
                token_sources,
                line_index: line_offset + index,
                inline_top: effective_top,
                top: if has_inline_object {
                    effective_top
                } else {
                    effective_top
                        - if line.text_height > 0 {
                            (line.text_height / 20).max(1)
                        } else {
                            paragraph.line_top_offset
                        }
                },
                height: if line.height > 0 {
                    line.height
                } else {
                    line.text_height.max(1000)
                },
                left: line.left,
                padding_left: if index > 0 || line.textpos > 0 {
                    paragraph.para_style.hanging_indent
                } else {
                    0
                },
                width: line.width,
                line_height: rendered_line_height(line, use_metric_offset),
                para_style_id: paragraph.para_style_id,
                bullet_marker: if index == 0 && line.textpos == 0 {
                    paragraph.para_style.bullet_marker.clone()
                } else {
                    None
                },
                heading: paragraph.para_style.heading,
                paragraph_start: paragraph_start.clone(),
                paragraph_empty,
                page_break: paragraph.page_break && line_offset + index == 0,
                tokens,
                inline_objects,
                // Filled in by the caller, which knows the column layout in
                // force and how many restarts have been consumed.
                columns: (1, 0),
                column_index: 0,
                spacing: line.spacing,
                text_height: line.text_height,
                baseline: line.baseline,
                fill,
            }
        })
        .collect()
}

/// Whether and how a line is widened to its full width, as Hancom aligns
/// it (rhwp `needs_word_distribution`): a justified line unless it is its
/// paragraph's last or the author broke it (`hp:lineBreak`), a 나눔
/// (`DISTRIBUTE_SPACE`) line unless the author broke it, both over their
/// spaces, and a 배분 (`DISTRIBUTE`) line always, over its letters.
///
/// A line with a tab or a dash leader (three or more `-`) keeps its width:
/// the tab box ends at its stored stop, which widening the spaces before
/// it would move, and rhwp gives a leader's dashes the room instead of the
/// spaces. The caller also keeps lines with inline objects or tables, whose
/// width the script does not measure.
fn line_fill(align: &str, paragraph_end: bool, tokens: &[Token]) -> Option<LineFill> {
    let broken = tokens.iter().any(|token| match &token.kind {
        TokenKind::LineBreak => true,
        TokenKind::Control { kind } => kind == "lineBreak",
        _ => false,
    });
    let fill = match align {
        "justify" if !paragraph_end && !broken => LineFill::Spaces,
        "distribute_space" if !broken => LineFill::Spaces,
        "distribute" | "distributed" => LineFill::Letters,
        _ => return None,
    };
    let mut text = String::new();
    for token in tokens {
        match &token.kind {
            TokenKind::Tab { .. } => return None,
            TokenKind::Text(part) => text.push_str(part),
            _ => {}
        }
    }
    if text.contains("---") || text.trim().chars().count() < 2 {
        return None;
    }
    Some(fill)
}

/// The CSS line height of a line of letters of one size: what a line whose
/// source height is that size is given (`rendered_line_height`).
pub(crate) fn line_height_of_size(size: HwpUnit, use_metric_offset: bool) -> HwpUnit {
    rendered_line_height(
        &LineSeg {
            text_height: size,
            height: size,
            ..LineSeg::default()
        },
        use_metric_offset,
    )
}

fn rendered_line_height(line: &LineSeg, use_metric_offset: bool) -> i64 {
    if use_metric_offset {
        match line.text_height.max(line.height) {
            900 => 700,
            1000 => 790,
            _ => line_height(line),
        }
    } else {
        line_height(line)
    }
}

fn line_height(line: &LineSeg) -> i64 {
    // HWP's spacing is not a CSS line-height. Hancom's HTML export uses the
    // Windows font metric for the common HWP glyph-box sizes below; retaining
    // those values keeps the glyph baseline aligned without enabling browser
    // wrapping. Every entry here was measured from a reference HTML's own
    // `line-height`/`height` pair for that exact text height. The 900-4700
    // range (100-unit steps) is now completely covered by 샘플/글자 크기별
    // 독립 문단, one independent (non-wrapping) paragraph per size: every
    // one of its 39 sizes maps to exactly one line-height with zero
    // inconsistency, which is why this range has no gaps left. Sizes outside
    // that range go to `measured_line_height_curve` below, which reproduces
    // the same measurements; keep taking new table entries from real
    // reference measurements rather than from that curve.
    let height = line.text_height.max(line.height);
    let height = if height > 0 { height } else { 1000 };
    match height {
        100 => 71,
        200 => 142,
        300 => 216,
        400 => 292,
        500 => 371,
        600 => 450,
        700 => 533,
        800 => 615,
        850 => 658,
        900 => 704,
        1000 => 790,
        1048 => 834,
        1050 => 834,
        1100 => 880,
        1200 => 972,
        1300 => 1066,
        1400 => 1162,
        1500 => 1258,
        1550 => 1310,
        1600 => 1360,
        1700 => 1462,
        1800 => 1564,
        1900 => 1672,
        2000 => 1779,
        2100 => 1891,
        2200 => 2001,
        2300 => 2115,
        2350 => 2174,
        2400 => 2231,
        2500 => 2349,
        2600 => 2469,
        2700 => 2591,
        2800 => 2716,
        2900 => 2843,
        3000 => 2971,
        3100 => 3101,
        3199 => 3199,
        3200 => 3200,
        3300 => 3299,
        3400 => 3399,
        3500 => 3500,
        3600 => 3600,
        3700 => 3699,
        3800 => 3801,
        3900 => 3900,
        4000 => 4000,
        4100 => 4099,
        4200 => 4201,
        4300 => 4300,
        4400 => 4399,
        4500 => 4498,
        4600 => 4601,
        4700 => 4700,
        5300 => 5301,
        _ => measured_line_height_curve(height),
    }
}

/// The rule the measured table above follows, for the text heights that table
/// does not list. The reference's `line-height`/`text height` ratio rises by
/// exactly 0.01 for every 100 HWPUNIT of text height -- 0.79 at 1000, 0.89 at
/// 2000, 0.99 at 3000 -- so the ratio is `(height + 6900) / 10000`, and it
/// stops at 1.0: every measured size from 3100 up maps to itself within the
/// table's own rounding. `min` expresses that cap, since the curve only
/// exceeds `height` past that point.
///
/// Verified as a prediction, not a fit: the four sizes below were absent from
/// the table and from the data the curve was read off, and the curve gives
/// the reference's own string for each of them.
///   844  -> 2.31mm  (198 lines, 산업기술개발장비 통합관리요령 개정(안))
///   1054 -> 2.96mm  ( 54 lines, same document)
///   3696 -> 13.04mm (  6 lines, 2026 대한민국 산업단지 발전 유공자 모집 공고)
///   3731 -> 13.16mm (  4 lines, 산업기술혁신사업 공통운영요령 개정(안))
/// The previous `height * 4 / 5` fallback got all four wrong (2.38/2.97/
/// 10.43/10.53mm), and grew worse with size because it has no such cap.
fn measured_line_height_curve(height: i64) -> i64 {
    round_div(i128::from(height) * i128::from(height + 6900), 10_000)
        .min(height)
        .max(1)
}

fn slice_tokens(tokens: &[Token], start: usize, end: usize) -> (Vec<Token>, Vec<TokenSourceRange>) {
    let mut output = Vec::new();
    let mut sources = Vec::new();
    let mut cursor = 0usize;
    for (token_index, token) in tokens.iter().enumerate() {
        let token_start = cursor;
        let token_end = cursor + token.logical_len;
        cursor = token_end;
        if token_end <= start || token_start >= end {
            continue;
        }
        let local_start = start.saturating_sub(token_start);
        let local_end = end.min(token_end).saturating_sub(token_start);
        let mut copy = token.clone();
        let mut source = TokenSourceRange {
            token_index,
            logical: token_start + local_start..token_start + local_end,
            utf8: None,
        };
        if let TokenKind::Text(text) = &token.kind {
            let (part, logical, utf8) = utf16_slice(text, local_start, local_end);
            copy.kind = TokenKind::Text(part);
            copy.logical_len = logical.len();
            source.logical = token_start + logical.start..token_start + logical.end;
            source.utf8 = Some(utf8);
        } else {
            copy.logical_len = token.logical_len.min(local_end.saturating_sub(local_start));
        }
        if copy.logical_len > 0 {
            output.push(copy);
            sources.push(source);
        }
    }
    (output, sources)
}

fn utf16_slice(
    text: &str,
    start: usize,
    end: usize,
) -> (String, std::ops::Range<usize>, std::ops::Range<usize>) {
    let mut result = String::new();
    let mut logical = 0..0;
    let mut utf8 = 0..0;
    let mut cursor = 0usize;
    for (byte, character) in text.char_indices() {
        let len = character.len_utf16();
        let next = cursor + len;
        if next > start && cursor < end {
            if result.is_empty() {
                logical.start = cursor;
                utf8.start = byte;
            }
            logical.end = next;
            utf8.end = byte + character.len_utf8();
            result.push(character);
        }
        cursor = next;
        if cursor >= end {
            break;
        }
    }
    (result, logical, utf8)
}

/// Adjacent floats in one stack can overlap by a few HWPUNIT of editor
/// rounding: 성과보고서 sections 36-42 stack 10mm-multiple tables whose
/// declared offsets overlap the previous table by up to 4 units (0.01mm).
const STACK_OVERLAP_TOLERANCE: i64 = 5;

// A source paragraph can carry a page-sized stack of separate floating
// tables. Their relative offsets describe one group, not independent page
// advances. Only accept ordered, unframed stacks that fit one content page.
fn compact_float_stack(
    paragraph: &Paragraph,
    following: &[Block],
    capacity: i64,
) -> Option<(usize, i64, i64)> {
    // A blank anchor paragraph can still carry a purely decorative floating
    // "line" (a hairline divider drawn above the group, e.g. 성과보고서
    // s4's 18-table group: an `hp:line` sized 45354x1 HWPUNIT, the same
    // width as the tables it separates). That divider positions itself off
    // this same paragraph's flow top, exactly like the tables do once
    // shifted below, so it moves correctly with the compacted group. Any
    // other object kind (pic/rect/...) carries real content whose own
    // placement this function does not verify, so it still disqualifies.
    if paragraph.has_inline_table
        || paragraph.objects.iter().any(|object| object.kind != "line")
        || paragraph
            .tokens
            .iter()
            // A lone space is a harmless artifact of HWP's field-end marker
            // (성과보고서 s4's "재원현황 및 비중" caption+table pair sits in
            // a run with a literal `<hp:t> </hp:t>` right before its
            // `fieldEnd`), not real content.
            .any(|t| !t.visible_text().trim().is_empty())
    {
        return None;
    }
    let tables = following
        .iter()
        .take_while(|b| matches!(b, Block::Table(_)))
        .filter_map(|b| {
            if let Block::Table(t) = b {
                Some(t)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if tables.len() < 2 {
        return None;
    }
    let first = tables[0];
    let mut previous_bottom = None;
    let mut first_top = 0;
    let mut last_bottom = 0;
    for (index, table) in tables.iter().enumerate() {
        if table.anchor.treat_as_char
            || !table.anchor.flow_with_text
            || !table.anchor.vert_rel_to.eq_ignore_ascii_case("PARA")
            || table.anchor_y != first.anchor_y
            || table.anchor.horz_offset != first.anchor.horz_offset
            || !same_stack_width(table, first)
            || table.out_margin_top != 0
            || table.out_margin_bottom != 0
            || table
                .cells
                .iter()
                .any(|cell| !cell.tables.is_empty() || cell_coordinate_runs(cell).len() > 1)
        {
            return None;
        }
        let y = table_position(table, table.anchor_y?).y;
        if previous_bottom.is_some_and(|bottom| y < bottom - STACK_OVERLAP_TOLERANCE) {
            return None;
        }
        if index == 0 {
            first_top = y;
        }
        last_bottom = y.saturating_add(table.box_units.height);
        previous_bottom = Some(last_bottom);
    }
    (last_bottom - first_top <= capacity).then_some((tables.len(), first_top, last_bottom))
}

/// A complementary float-stack pattern `compact_float_stack` above does not
/// cover: several separate PARA-relative floats on one blank paragraph whose
/// declared `vertOffset` values do not encode their true stacking order at
/// all (different widths, one carrying a large, unrelated offset). The one
/// instance found in the 60-sample corpus is 성과보고서 s4's "재원현황 및
/// 비중" caption (width 47624, vertOffset -5219) immediately followed by its
/// data table (width 47894, outMargin 141 each side, vertOffset 43) --
/// `compact_float_stack` rejects the pair on both the width-equality and
/// zero-outMargin checks, and its own vertOffset-based math would place them
/// 0.17mm/50mm+ away from the reference either way.
///
/// The anchor paragraph's own (glyph-box-adjusted) `anchor_y` there turned
/// out to equal the *bottom* edge of the group's last table, including that
/// table's own outMargin frame growth, when the tables are simply stacked
/// back-to-back in declaration order -- verified to within 1 HWPUNIT by
/// solving the reference's exact `top:`/`height:` strings for both tables.
/// So instead of trusting each table's own `vertOffset`, this places the
/// group so its combined rendered height (each table's `box_units.height`
/// plus its own top+bottom outMargin) ends exactly at `anchor_y`.
///
/// Deliberately narrow: only steps in when `compact_float_stack` already
/// declined *and* the widths actually differ, so it cannot reinterpret the
/// uniform-width, correctly-ordered stacks that function already owns.
fn compact_float_stack_by_height(paragraph: &Paragraph, following: &[Block]) -> Option<Vec<i64>> {
    if paragraph.has_inline_table
        || !paragraph.objects.is_empty()
        // A lone space is a harmless artifact of HWP's field-end marker; see
        // the identical check in `compact_float_stack`.
        || paragraph
            .tokens
            .iter()
            .any(|t| !t.visible_text().trim().is_empty())
    {
        return None;
    }
    let tables = following
        .iter()
        .take_while(|b| matches!(b, Block::Table(_)))
        .filter_map(|b| {
            if let Block::Table(t) = b {
                Some(t)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if tables.len() < 2 {
        return None;
    }
    let first = tables[0];
    let anchor_y = first.anchor_y?;
    if tables.iter().all(|table| same_stack_width(table, first)) {
        // compact_float_stack's own domain; let it handle uniform widths.
        return None;
    }
    if tables.iter().any(|table| {
        table.anchor.treat_as_char
            || !table.anchor.flow_with_text
            || !table.anchor.vert_rel_to.eq_ignore_ascii_case("PARA")
            || table.anchor_y != Some(anchor_y)
            || table
                .cells
                .iter()
                .any(|cell| !cell.tables.is_empty() || cell_coordinate_runs(cell).len() > 1)
    }) {
        return None;
    }
    let rendered_heights = tables
        .iter()
        .map(|table| {
            table
                .box_units
                .height
                .saturating_add(table.out_margin_top)
                .saturating_add(table.out_margin_bottom)
        })
        .collect::<Vec<_>>();
    let total = rendered_heights.iter().sum::<i64>();
    let mut y = anchor_y.saturating_sub(total);
    if y < 0 {
        return None;
    }
    let mut positions = Vec::with_capacity(tables.len());
    for height in rendered_heights {
        positions.push(y);
        y = y.saturating_add(height);
    }
    Some(positions)
}

/// Stacked float widths can differ by one HWPUNIT of editor rounding
/// (성과보고서 section 37: 48189 then five 48188-wide tables). A genuinely
/// different width, like s4's 47624/47894 caption+data pair, still differs.
fn same_stack_width(table: &Table, first: &Table) -> bool {
    table.box_units.width.abs_diff(first.box_units.width) <= 1
}

fn table_position(table: &Table, fallback_y: i64) -> BoxUnits {
    let mut result = table.box_units;
    result.x += table.anchor.horz_offset;
    // A PARA-relative float measures from its paragraph's left margin, not
    // from the content area's edge. 성과보고서's 5x3 sidebar sits at
    // horzOffset -1960 under a paragraph whose paraPr margin is 2331, and the
    // reference places it 371 HWPUNIT into the content area rather than 1960
    // out of it.
    if table.anchor.horz_rel_to.eq_ignore_ascii_case("PARA") {
        result.x += table.anchor_paragraph_left;
    }
    // `flowWithText="0"` only tells HWP not to re-flow *later* content
    // around this float; a `vertRelTo="PARA"` float still measures from its
    // anchor paragraph's own flow position regardless. Only a genuinely
    // page-absolute anchor (`vertRelTo="PAPER"`/`"PAGE"`) uses `vertOffset`
    // on its own -- 성과보고서's 25 "flowWithText=0, vertRelTo=PARA" budget
    // tables (one per program section) otherwise land at the bare page
    // content top, burying their own preceding heading underneath them.
    result.y += if table.anchor.treat_as_char
        || table.anchor.flow_with_text
        || table.anchor.vert_rel_to.eq_ignore_ascii_case("PARA")
    {
        fallback_y + table.anchor.vert_offset
    } else {
        table.anchor.vert_offset
    };
    result
}

/// The horizontal area a floating table nested in `cell` aligns in, as
/// `(left, width)` from the cell's content box: the host paragraph's for
/// `horzRelTo="PARA"` (its margins inside the cell), the cell's otherwise.
fn nested_float_area(table: &Table, cell: &crate::model::TableCell) -> Option<(i64, i64)> {
    if table.anchor.treat_as_char {
        return None;
    }
    let inner = cell.box_units.width - cell.margin_left - cell.margin_right;
    let (left, right) = if table.anchor.horz_rel_to.eq_ignore_ascii_case("PARA") {
        let key = &table.source_anchor.as_ref()?.paragraph_key;
        let host = cell
            .paragraphs
            .iter()
            .find(|paragraph| &paragraph.key == key)?;
        (host.para_style.margin_left, host.para_style.margin_right)
    } else {
        (0, 0)
    };
    Some((left, inner - left - right))
}

/// A floating table's margin-box x in an area: `horzAlign` places the whole
/// frame (outMargins included) and `horzOffset` moves it inward from that
/// side. A frame wider than the area overhangs it on both sides when
/// centred -- no clamping. 성과보고서 p420 "직종별 수요·공급차이" (29488
/// wide, CENTER in a 3060-indented paragraph of a 19476 cell) sits at
/// -12.27mm from its cell in the reference, which is
/// `141 + 3060 + (16134 - 29770) / 2 + 141 = -3476`. s28's RIGHT table
/// (44847 in a 46775 cell, margins 509) sits 4.4mm in, which is
/// `509 + 45757 - 45129 + 141`. (rhwp clamps a wider centred table to the
/// left edge; the reference does not.)
fn aligned_table_x(table: &Table, left: i64, width: i64) -> i64 {
    let frame = table.box_units.width + table.out_margin_left + table.out_margin_right;
    let offset = table.anchor.horz_offset;
    left + match table.anchor.horz_align.to_ascii_uppercase().as_str() {
        "CENTER" => (width - frame) / 2 + offset,
        "RIGHT" | "OUTSIDE" => width - frame - offset,
        _ => offset,
    }
}

/// Moves a laid-out table and its cells together. Cell boxes share the
/// table's coordinate space (`cell.y = table.y + rows above`), so moving the
/// frame alone pulls every cell back by the same amount relative to it.
/// Cell paragraphs, objects and nested tables are cell-relative and stay.
/// 성과보고서 s6's process table, centred in its 1x1 frame cell, had its
/// rows drawn 7.22mm above its own box, over the caption above it. rhwp
/// likewise lays a nested table out from one origin (`layout_table` at
/// `nested_y` inside the aligned `inner_area`).
pub(crate) fn translate_table(table: &mut Table, dx: i64, dy: i64) {
    table.box_units.x = table.box_units.x.saturating_add(dx);
    table.box_units.y = table.box_units.y.saturating_add(dy);
    for cell in &mut table.cells {
        cell.box_units.x = cell.box_units.x.saturating_add(dx);
        cell.box_units.y = cell.box_units.y.saturating_add(dy);
    }
}

/// Lay out a table carried by a shape's text box in the text box's own
/// frame, anchored like a cell's nested table.
pub(crate) fn layout_text_box_table(table: &Table) -> Table {
    let anchor_y = if table.anchor.treat_as_char {
        0
    } else {
        table.anchor_y.unwrap_or(0)
    };
    layout_table(table, anchor_y)
}

fn layout_table(table: &Table, fallback_y: i64) -> Table {
    let mut result = table.clone();
    result.box_units = table_position(table, fallback_y);
    for cell in &mut result.cells {
        carry_pushed_lines_into_restarts(cell);
    }
    // Row fitting needs the positioned bottom of nested floats. Source
    // `box_units.y` is only the unplaced declaration, so fitting the source
    // cells first can leave a float below its enclosing cell.
    for cell in &mut result.cells {
        let areas = cell
            .tables
            .iter()
            .map(|nested| nested_float_area(nested, cell))
            .collect::<Vec<_>>();
        for (nested, area) in cell.tables.iter_mut().zip(areas) {
            let anchor_y = if nested.anchor.treat_as_char {
                0
            } else {
                nested.anchor_y.unwrap_or(0)
            };
            *nested = layout_table(nested, anchor_y);
            if let Some((left, width)) = area {
                let x = aligned_table_x(nested, left, width);
                let dx = x - nested.box_units.x;
                translate_table(nested, dx, 0);
            }
        }
    }
    let column_count = table.columns.max(
        table
            .cells
            .iter()
            .map(|cell| cell.column + cell.col_span)
            .max()
            .unwrap_or(0),
    );
    let row_count = table.rows.max(
        table
            .cells
            .iter()
            .map(|cell| cell.row + cell.row_span)
            .max()
            .unwrap_or(0),
    );
    let (column_widths, row_heights) = infer_grid_dimensions(&result, column_count, row_count);
    result.row_heights = row_heights.clone();
    for cell in &mut result.cells {
        cell.box_units.x = column_widths[..cell.column.min(column_widths.len())]
            .iter()
            .sum();
        cell.box_units.y = row_heights[..cell.row.min(row_heights.len())].iter().sum();
        let column_end = (cell.column + cell.col_span).min(column_widths.len());
        cell.box_units.width = column_widths
            .get(cell.column.min(column_end)..column_end)
            .unwrap_or(&[])
            .iter()
            .sum::<i64>()
            .max(cell.box_units.width);
        let row_end = (cell.row + cell.row_span).min(row_heights.len());
        cell.box_units.height = row_heights
            .get(cell.row.min(row_end)..row_end)
            .unwrap_or(&[])
            .iter()
            .sum::<i64>()
            .max(cell.box_units.height);
        cell.box_units.x += result.box_units.x;
        cell.box_units.y += result.box_units.y;
    }
    for cell in &mut result.cells {
        for paragraph in &mut cell.paragraphs {
            let first_line = paragraph_fragments(paragraph, 0).first().cloned();
            let line_top = first_line.as_ref().map_or(0, |line| line.top);
            // A floating object anchors to the line's true flow position
            // (`inline_top`), not `top`'s extra glyph-box pullback that the
            // *text* rendering alone applies -- the same principle already
            // established for floating tables (`anchor_top_adjustment`):
            // that correction belongs to the text flow, not to anything
            // anchored off of it. Using `top` here left a PARA-anchored
            // floating picture's `box_units.y` off by that glyph-box amount,
            // which a table cell's vertical-center math then doubled into a
            // large downward offset (성과보고서's "소재부품기술개발 온라인설명회"
            // cell picture, pushed past the cell's bottom edge and clipped).
            let float_anchor_top = first_line.as_ref().map_or(0, |line| {
                para_float_anchor(line.inline_top, paragraph.para_style.margin_before)
            });
            for object in &mut paragraph.objects {
                let anchor_top = if object.anchor.treat_as_char {
                    line_top
                } else {
                    float_anchor_top
                };
                *object = position_object(object, anchor_top, 0);
            }
        }
    }
    let row_total = row_heights.iter().sum::<i64>();
    // Retain the stored frame when merged declarations disagree with the
    // fitted rows (the 35-row "대분류" sample needs its declared 229.80mm).
    // Matching declarations do NOT override content minima: those must
    // already have been enforced by infer_grid_dimensions. In particular,
    // a CELL continuation can span several pages even when every original
    // cellSz agrees with a much smaller declared grid.
    let rowspan_consistent = !table.no_adjust
        && table.cells.iter().all(|cell| {
            cell.row_span <= 1
                || row_heights
                    .get(cell.row..(cell.row + cell.row_span).min(row_heights.len()))
                    .is_some_and(|spanned| spanned.iter().sum::<i64>() == cell.box_units.height)
        });
    if rowspan_consistent {
        result.box_units.height = row_total.max(1);
    } else if row_total > result.box_units.height {
        result.box_units.height = row_total;
    }
    result
}

fn infer_grid_dimensions(
    table: &Table,
    column_count: usize,
    row_count: usize,
) -> (Vec<i64>, Vec<i64>) {
    let mut column_widths = vec![0_i64; column_count];
    let mut row_heights = vec![0_i64; row_count];
    for cell in &table.cells {
        let required_height = required_cell_height(table, cell);
        if cell.col_span == 1 {
            if let Some(width) = column_widths.get_mut(cell.column) {
                *width = (*width).max(cell.box_units.width.max(0));
            }
        }
        if cell.row_span == 1 {
            if let Some(height) = row_heights.get_mut(cell.row) {
                *height = (*height).max(required_height.max(0));
            }
        }
    }
    for _ in 0..column_count.max(row_count).max(1) {
        for cell in &table.cells {
            distribute_deficit(
                &mut column_widths,
                cell.column,
                cell.col_span,
                cell.box_units.width.max(0),
            );
            let required_height = required_cell_height(table, cell);
            distribute_deficit(
                &mut row_heights,
                cell.row,
                cell.row_span,
                required_height.max(0),
            );
        }
    }
    // Source grid fitting can reduce earlier constraints. Re-apply content
    // minima after fitting so later cells cannot collapse a long cell.
    for cell in &table.cells {
        if cell.row_span == 1 {
            if let Some(height) = row_heights.get_mut(cell.row) {
                let required_height = required_cell_height(table, cell);
                *height = (*height).max(required_height);
            }
        }
    }
    // CELL continuations are cropped in cumulative coordinate-run
    // space (see cell_content_height). Preserve that minimum for merged cells
    // too, after source grid fitting, regardless of noAdjust.
    // Other merged-cell sizing stays unchanged:
    // ordinary adjusted grids can intentionally be shorter than this estimate
    // (finance s20/r4c3 would otherwise enlarge a reference-matching row).
    // A simple stacked text cell can also overflow without a coordinate
    // restart. Its measured content is complete, so preserve that minimum
    // after source grid fitting as well.
    // noAdjust=0 does not mean that subsequent coordinate runs may be lost:
    // s7's measurement cells need 64860/39260 units despite declaring 5544.
    // rhwp's resolve_row_heights_with_common_fit likewise enforces merged
    // content minima after grid fitting, extending the final spanned row.
    for cell in &table.cells {
        let lines = cell
            .paragraphs
            .iter()
            .flat_map(|p| &p.lines)
            .collect::<Vec<_>>();
        let has_continuation = lines.windows(2).any(|pair| pair[1].top <= pair[0].top);
        if cell.row_span > 1
            && table.page_break.eq_ignore_ascii_case("CELL")
            && (has_continuation
                || is_simple_stacked_text_cell(table, cell)
                || (table.no_adjust
                    && table.repeat_header
                    && cell.row_span > 1
                    && is_local_stacked_text_cell(table, cell)
                    && line_flow_height(
                        &lines.iter().map(|line| (*line).clone()).collect::<Vec<_>>(),
                    ) > cell.box_units.height))
        {
            // Adjusted cells paint their text inside their own padding. A
            // coordinate-run extent excludes that padding, so reserving only
            // the extent still clips the final line at the cell's bottom.
            // Keep the existing noAdjust crop contract unchanged.
            let required = if table.no_adjust
                && !has_continuation
                && is_local_stacked_text_cell(table, cell)
            {
                required_cell_height(table, cell)
                    .saturating_add(cell.margin_top.max(0))
                    .saturating_add(cell.margin_bottom.max(0))
            } else if table.no_adjust || !has_continuation {
                required_cell_height(table, cell)
            } else {
                // A stacked cell's measure already carries its margins
                // (`cell_content_height`). It can only reach here through a
                // "restart" that lies at the previous line's own bottom, i.e.
                // a zero-height leading paragraph such as 성과보고서
                // s8/r4c3's (content 29800, reference 29800 + 282, not
                // + 564); the text stays, only the margins are not counted
                // twice.
                let measured = cell_content_height(table, cell);
                let padded = if is_simple_stacked_text_cell(table, cell) {
                    measured
                } else {
                    measured
                        .saturating_add(cell.margin_top.max(0))
                        .saturating_add(cell.margin_bottom.max(0))
                };
                required_cell_height(table, cell).max(padded)
            };
            let end = cell
                .row
                .saturating_add(cell.row_span)
                .min(row_heights.len());
            if cell.row < end
                && required > cell.box_units.height
                && required > row_heights[cell.row..end].iter().sum::<i64>()
                && !stored_pieces_fit_declared_span(table, cell)
            {
                // A merged cell's content overflow extends its final row.
                // Keep preceding sibling rows at their fitted sizes: the
                // reference preserves the short target/actual rows and puts
                // the remaining height in the achievement row (finance s35).
                let current = row_heights[cell.row..end].iter().sum::<i64>();
                row_heights[end - 1] += required - current;
            }
        }
    }
    // An empty, unpadded line is a caret box, not visible content. Its
    // nominal font height must not enlarge a separator beyond the stored
    // table frame. Conversely, cellSz can be stale after a row was enlarged:
    // the outer height retains that extra space (the floating-table sample
    // stores 12055 for a 11055 + 283 grid). Restore only that supported growth,
    // from the tail, without taking space away from real content minima.
    // An inline picture can likewise outgrow a stale cellSz while the table's
    // declared height already includes its size (performance report s26).
    // Give such rows only the supported spare, including the cell margins.
    let mut spare = table
        .box_units
        .height
        .saturating_sub(row_heights.iter().sum());
    for row in (0..row_count).rev() {
        let cells = table
            .cells
            .iter()
            .filter(|cell| cell.row == row)
            .collect::<Vec<_>>();
        let empty_row = !cells.is_empty()
            && cells
                .iter()
                .all(|cell| is_empty_unpadded_local_cell(table, cell));
        let inline_row = !cells.is_empty()
            && cells.iter().all(|cell| cell.row_span == 1)
            && cells.iter().any(|cell| {
                cell.paragraphs.iter().any(|paragraph| {
                    paragraph
                        .objects
                        .iter()
                        .any(|object| object.anchor.treat_as_char)
                })
            });
        if empty_row || inline_row {
            let natural = cells
                .iter()
                .map(|cell| {
                    if empty_row {
                        cell_content_height(table, cell)
                    } else {
                        let object_height = cell
                            .paragraphs
                            .iter()
                            .flat_map(|p| &p.objects)
                            .filter(|object| object.anchor.treat_as_char)
                            .map(|object| object.box_units.height.max(0))
                            .max()
                            .unwrap_or(0);
                        cell_content_height(table, cell).max(
                            object_height
                                .saturating_add(cell.margin_top.max(0))
                                .saturating_add(cell.margin_bottom.max(0)),
                        )
                    }
                })
                .max()
                .unwrap_or(0);
            let growth = natural
                .saturating_sub(row_heights[row])
                .max(0)
                .min(spare.max(0));
            row_heights[row] += growth;
            spare -= growth;
        }
    }
    // A single-row table's own declared height can already be expanded past
    // any individual cell's declared cellSz (e.g. a treatAsChar letterhead
    // table where one cell's real content needs more room than its grid,
    // and the source table height already reflects that). The sole row must
    // occupy that declared height or its cells stop short of the table's own
    // bottom edge.
    if row_count == 1 {
        row_heights[0] = row_heights[0].max(table.box_units.height);
    }
    (column_widths, row_heights)
}

/// A single local text line has no following line to consume its trailing
/// leading, so its own content minimum (glyph box plus padding, computed by
/// `cell_content_height`'s matching branch below) is unambiguous -- unlike a
/// multi-line or coordinate-restarting cell, there is no cumulative-spacing
/// or continuation-cropping judgment call to get wrong.
///
/// The same reasoning extends directly to any number of local lines stacked
/// one after another with no restart between them -- whether they come from
/// several one-line paragraphs or a single paragraph's own word-wrap: a
/// genuine page-continuation fragment always shows up as a restart (a later
/// line's top going back to at or before an earlier one), since that is
/// exactly how `cell_content_height`'s own `no_adjust` branch (and
/// `line_flow_height`) detect one. Without a restart, every line here is
/// the cell's own, complete, un-cropped content.
///
/// Verified against two independent shapes: 성과보고서 s2's "직급별\n구
/// 분" repeat-header cell (two one-line paragraphs, second starting well
/// after the first's own bottom) and s3's "◈ 전략목표Ⅱ : […]" title cell
/// (one paragraph word-wrapped into two lines by HWP itself, same
/// no-restart shape) -- `is_single_local_line_cell`'s original
/// one-paragraph, one-line form left both clipped, hiding the second line
/// behind `.hce`'s `overflow:hidden`.
fn is_simple_stacked_text_cell(table: &Table, cell: &crate::model::TableCell) -> bool {
    // A single-row table's sole row already inherits the table's own
    // (margin-inclusive) declared height via `infer_grid_dimensions`'s own
    // `row_count == 1` rule -- adding a margin again here would double it.
    // 샘플/다중 문단 셀 콘텐츠 높이's single-row, 40-paragraph/84-line cell
    // (no restart, so it would otherwise qualify) is the confirming case:
    // its own `content_height` must stay margin-free, unlike every
    // multi-row case this function was verified against.
    if (table.no_adjust && !(cell.row == 0 && stale_no_adjust_header(table)))
        || !is_local_stacked_text_cell(table, cell)
    {
        return false;
    }
    true
}

fn is_local_stacked_text_cell(table: &Table, cell: &crate::model::TableCell) -> bool {
    if table.rows <= 1 || !cell.tables.is_empty() || cell.paragraphs.is_empty() {
        return false;
    }
    let mut previous_bottom: Option<i64> = None;
    for paragraph in &cell.paragraphs {
        if !paragraph.objects.is_empty() || paragraph.has_inline_table || paragraph.lines.is_empty()
        {
            return false;
        }
        for line in &paragraph.lines {
            match previous_bottom {
                None if line.top != 0 => return false,
                Some(bottom) if line.top < bottom => return false,
                _ => {}
            }
            previous_bottom = Some(
                line.top
                    .saturating_add(line.height.max(line.text_height).max(0)),
            );
        }
    }
    true
}

/// A CELL table may store a short row-0 cellSz even though its stacked text
/// and padding define a taller repeated header. Keep this case separate from
/// other noAdjust tables, whose declared grid can be authoritative.
fn stale_no_adjust_header(table: &Table) -> bool {
    table.no_adjust
        && table.repeat_header
        && table.rows > 1
        && table.page_break.eq_ignore_ascii_case("CELL")
        && table.cells.iter().any(|cell| {
            cell.row == 0
                && cell.paragraphs.len() > 1
                && cell.margin_top + cell.margin_bottom > 0
                && cell.box_units.height <= cell.margin_top + cell.margin_bottom
        })
}

// Empty local paragraphs provide caret metrics, not a visible-content floor.
// Coordinate continuations and nonempty tokens must keep their normal minima.
fn is_empty_unpadded_local_cell(table: &Table, cell: &crate::model::TableCell) -> bool {
    is_simple_stacked_text_cell(table, cell)
        && cell.row_span == 1
        && cell.box_units.height > 0
        && cell.margin_top == 0
        && cell.margin_bottom == 0
        && cell.paragraphs.iter().all(|p| p.tokens.is_empty())
}

/// Keep the row minimum rule shared by all three grid constraint passes.
/// Inline grids and the legacy first-row constraint retain their source size.
/// This sizing fallback is independent of whether cells may be repeated:
/// removing it along with the header-copy fix lost matching reference lines.
///
/// Scoped to cells with no nested table of their own: those two exceptions
/// were verified against simple text cells, whose declared cellSz already
/// reflects HWP's own final layout. A cell that carries a nested table can
/// need its own outer cell to grow well past that declared size --
/// 성과보고서 s3's "<총괄표>" caption cell (in a treatAsChar outer table)
/// declared only 12.86mm while its own nested 5x7 inline table is 27.36mm
/// tall; keeping the declared height clipped everything below the nested
/// table's first couple of rows (`.hce`'s `overflow:hidden`) even though the
/// position fix above already draws it correctly. `cell_content_height()`'s
/// existing `nested_height` term already exists for exactly this case.
///
/// Also scoped away from `is_simple_stacked_text_cell`: that same
/// "<총괄표>" table's own row 0 (header=1, "(단위 : 개)") declares cellSz
/// height 282 HWPUNIT, shorter than its one line's own text height (1000)
/// -- the `repeat_header && row == 0` exception kept it clipped to 282
/// even after the nested-table fix, cascading a 3.53mm shortfall onto
/// every row below it. A simple stack of local lines' content minimum is
/// exactly the unambiguous case `cell_content_height`'s own line-flow
/// branch computes, so it cannot carry the overestimation risk the two
/// size exceptions above guard against; growing to fit it is safe.
fn required_cell_height(table: &Table, cell: &crate::model::TableCell) -> i64 {
    if is_empty_unpadded_local_cell(table, cell)
        || (cell.tables.is_empty()
            && !is_simple_stacked_text_cell(table, cell)
            && ((table.repeat_header && table.rows > 1 && cell.row == 0)
                || (table.anchor.treat_as_char && cell.box_units.height > 0)))
    {
        cell.box_units.height
    } else {
        cell.box_units.height.max(cell_content_height(table, cell))
    }
}

/// Development diagnostic in raw HWPUNIT, using the actual layout functions.
/// The grid-only comparison removes content constraints; it is not a reference
/// layout and its height delta is not a count of additional document pages.
pub fn table_height_diagnostic(table: &Table) -> serde_json::Value {
    let laid_out = layout_table(table, table.anchor_y.unwrap_or(0));
    let mut grid_only = table.clone();
    for cell in &mut grid_only.cells {
        cell.paragraphs.clear();
        cell.tables.clear();
    }
    let grid_only = layout_table(&grid_only, table.anchor_y.unwrap_or(0));
    let cells = table.cells.iter().map(|cell| {
        let lines = cell.paragraphs.iter().flat_map(|p| &p.lines).collect::<Vec<_>>();
        let restarts = lines.windows(2).filter(|pair| pair[1].top <= pair[0].top).count();
        serde_json::json!({
            "source_id": cell.id,
            "row": cell.row, "column": cell.column,
            "row_span": cell.row_span, "col_span": cell.col_span,
            "declared_height": cell.box_units.height,
            "content_height": cell_content_height(table, cell),
            "required_height": required_cell_height(table, cell),
            "grid_exception": (table.repeat_header && table.rows > 1 && cell.row == 0)
                || (table.anchor.treat_as_char && cell.box_units.height > 0),
            "margins": [cell.margin_left, cell.margin_right, cell.margin_top, cell.margin_bottom],
            "coordinate_restarts": restarts,
            "paragraphs": cell.paragraphs.iter().map(|p| serde_json::json!({
                "source_id": p.id, "page_break": p.page_break,
                "lines": p.lines.iter().map(|line| serde_json::json!({
                    "top": line.top, "height": line.height,
                    "text_height": line.text_height, "spacing": line.spacing,
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        })
    }).collect::<Vec<_>>();
    serde_json::json!({
        "source_id": table.id, "source_xml_path": table.source_path,
        "no_adjust": table.no_adjust, "treat_as_char": table.anchor.treat_as_char,
        "repeat_header": table.repeat_header, "page_break": table.page_break,
        "declared_height": table.box_units.height,
        "layout_height": laid_out.box_units.height,
        "row_heights": laid_out.row_heights,
        "grid_only_row_heights": grid_only.row_heights,
        "cells": cells
    })
}

fn cell_content_height(table: &Table, cell: &crate::model::TableCell) -> i64 {
    if let Some(runs) = merged_fragment_runs(table, cell) {
        let margins = cell
            .margin_top
            .max(0)
            .saturating_add(cell.margin_bottom.max(0));
        return runs
            .last()
            .map_or(0, |(offset, extent)| offset.saturating_add(*extent))
            .saturating_add(margins);
    }
    // A single local text line has no following line to consume its trailing
    // leading. Its cell minimum is the glyph box plus the effective padding.
    // Keep coordinate runs, inline objects and nested tables on their existing
    // paths until their continuation/occupancy rules are separately resolved.
    if !table.no_adjust && cell.tables.is_empty() {
        if let [paragraph] = cell.paragraphs.as_slice() {
            if paragraph.objects.is_empty() && !paragraph.has_inline_table {
                if let [line] = paragraph.lines.as_slice() {
                    if line.top == 0 {
                        return line
                            .height
                            .max(line.text_height)
                            .max(0)
                            .saturating_add(cell.margin_top.max(0))
                            .saturating_add(cell.margin_bottom.max(0));
                    }
                }
            }
        }
    }
    // A linesegarray stores local line positions. A page continuation (and
    // some paragraph boundaries) restarts that local position at zero, so a
    // single max() would lose the preceding segment. Conversely, summing
    // every line's height and spacing double-counts the spacing already
    // represented by successive `vertpos` values. Collapse each monotone run
    // to its extent and append only that run's trailing spacing.
    // CELL fragmentation crops in the flattened coordinate-run space, even
    // with noAdjust=1. Its row minimum must use that same space: a maximum
    // local bottom reserves only one page and discards later runs when the
    // row is cropped (finance s109/r2c5 retained only 24 of its 70 lines).
    // Tables that cannot split inside cells retain the noAdjust local extent.
    let paragraph_height = if table.no_adjust && !table.page_break.eq_ignore_ascii_case("CELL") {
        cell.paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.lines.iter())
            .map(|line| {
                line.top
                    .saturating_add(line.height.max(line.text_height).max(0))
            })
            .max()
            .unwrap_or(0)
    } else {
        // A margin-inclusive version of this branch (matching the
        // single-line shortcut above) was tried and reverted: it matched one
        // isolated row's `.hce` height in 샘플/다중행_표3, but made that same
        // document's overall reference match worse (dom_differences 1178 ->
        // 1443) and broke two previously byte-exact page counts elsewhere
        // (산업통상부공고 제2026-542호, 3->9 pages; 산업통상부와 그 소속기관
        // 직제 시행규칙 일부개정령안, 7->3 pages). The real rule for when a
        // multi-line cell's margin joins its content height is still
        // unresolved; leaving it out here is the empirically safer default.
        line_flow_height(
            &cell
                .paragraphs
                .iter()
                .flat_map(|paragraph| paragraph.lines.iter())
                .cloned()
                .collect::<Vec<_>>(),
        )
    };
    let nested_height = cell
        .tables
        .iter()
        .map(|table| {
            table
                .box_units
                .y
                .saturating_add(table.box_units.height.max(0))
        })
        .max()
        .unwrap_or(0);
    let content = paragraph_height.max(nested_height);
    // A cell holding a nested table is unambiguous in a way the general
    // multi-line text case (see the comment above) is not: the nested
    // table's own box already carries its content's exact extent, so this
    // cell's margins cannot be mistaken for spacing already counted inside
    // that extent. 성과보고서 s3's "<총괄표>" caption cell needs its declared
    // 141-unit top/bottom margins added on top of its content_height (9258)
    // to reach the reference's 9540 -- confirmed against the diagnostic
    // dump's raw margins field, which sums to exactly the missing 282 units.
    //
    // `is_simple_stacked_text_cell` is unambiguous for the same reason --
    // no coordinate restart means nothing here is a cropped continuation
    // -- and needs the same margin addition: 성과보고서 s2's "직급별\n구
    // 분" repeat-header cell (two stacked one-line paragraphs) is short by
    // exactly its own 141+141 HWPUNIT top/bottom margin once its content
    // extent (3640) is otherwise correct.
    if cell.tables.is_empty()
        && !is_simple_stacked_text_cell(table, cell)
        && !(is_no_adjust_text_growth(table, cell) && content > cell.box_units.height)
    {
        content
    } else {
        content
            .saturating_add(cell.margin_top.max(0))
            .saturating_add(cell.margin_bottom.max(0))
    }
}

// A plain, unmerged CELL paragraph has a measurable cumulative extent even
// when its stored line coordinates restart. Multi-paragraph cells can have
// independent paragraph origins and need their existing continuation rules.
pub(crate) fn is_no_adjust_text_growth(table: &Table, cell: &crate::model::TableCell) -> bool {
    table.no_adjust
        && table.rows > 1
        && table.columns > 1
        && table.page_break.eq_ignore_ascii_case("CELL")
        && cell.row_span == 1
        && cell.tables.is_empty()
        && cell.paragraphs.len() == 1
        && cell
            .paragraphs
            .iter()
            .all(|p| p.objects.is_empty() && !p.has_inline_table && !p.lines.is_empty())
}

pub(crate) fn line_flow_height(lines: &[LineSeg]) -> i64 {
    let mut total = 0_i64;
    let mut run_height = 0_i64;
    let mut previous_top = None;
    let mut trailing_spacing = 0_i64;
    for line in lines {
        if previous_top.is_some_and(|top| line.top <= top) {
            total = total
                .saturating_add(run_height)
                .saturating_add(trailing_spacing);
            run_height = 0;
        }
        run_height = run_height.max(
            line.top
                .saturating_add(line.height.max(line.text_height).max(0)),
        );
        trailing_spacing = line.spacing.max(0);
        previous_top = Some(line.top);
    }
    // Trailing spacing accounts for the gap before the *next* line; the
    // final run has no following line (in this cell or the next run), so
    // only its own extent belongs in the total.
    total.saturating_add(run_height)
}

/// Hancom keeps a cell line that its page break pushed onto the next page at
/// the old page's `vertpos`, so the restart that follows begins that line's
/// pitch below 0. 성과보고서 s2/tbl-anchor[12-0] restarts c6 and c8 at 1600
/// after a 1000+600 line, and 통합관리요령 공고's 89x2 table restarts r44c1 at
/// 2520 after a 1400+1120 line; both references draw the pushed line at the
/// top of the continuation. Rebase such lines into the restarted run so that
/// every run-based measurement and crop places them on that page. Restarts at
/// 0 (all the others in the samples) carry nothing.
fn carry_pushed_lines_into_restarts(cell: &mut crate::model::TableCell) {
    let mut lines = cell
        .paragraphs
        .iter_mut()
        .flat_map(|paragraph| paragraph.lines.iter_mut())
        .collect::<Vec<_>>();
    let mut run_start = 0_usize;
    for index in 1..lines.len() {
        if lines[index].top > lines[index - 1].top {
            continue;
        }
        let restart_top = lines[index].top;
        let last = &lines[index - 1];
        let carried_top = last
            .top
            .saturating_add(last.height.max(0))
            .saturating_add(last.spacing.max(0))
            .saturating_sub(restart_top);
        let carried = (restart_top > 0)
            .then(|| (run_start + 1..index).find(|&line| lines[line].top == carried_top))
            .flatten();
        if let Some(first) = carried {
            for line in &mut lines[first..index] {
                line.top -= carried_top;
            }
            run_start = first;
        } else {
            run_start = index;
        }
    }
}

/// A top-level paragraph whose first line(s) the page break pushed onto the
/// next page, with those lines rebased to that page (`None` when none were).
/// The same marker as a cell's (`carry_pushed_lines_into_restarts`): the line
/// after the break restarts at the pushed lines' own pitch instead of 0.
/// 통합관리요령 공고's "B.407 전기영동장치" (s2/p[668]) stores its line 0 at
/// 75680 on a 75685 body and its line 1 at 1440 = that line's 900+540; the
/// reference draws the whole paragraph on the next page, where the text of
/// line 0 would otherwise land on the page number. Restarts at 0, which every
/// other restart of the samples is, carry nothing.
fn paragraph_with_pushed_lines_carried(paragraph: &Paragraph) -> Option<Paragraph> {
    let lines = &paragraph.lines;
    if !lines
        .windows(2)
        .any(|pair| pair[1].top <= pair[0].top && pair[1].top > 0)
    {
        return None;
    }
    let mut lines = lines.clone();
    let mut changed = false;
    let mut run_start = 0_usize;
    for index in 1..lines.len() {
        if lines[index].top > lines[index - 1].top {
            continue;
        }
        let restart_top = lines[index].top;
        let last = &lines[index - 1];
        let carried_top = last
            .top
            .saturating_add(last.height.max(0))
            .saturating_add(last.spacing.max(0))
            .saturating_sub(restart_top);
        let carried = (restart_top > 0)
            .then(|| (run_start..index).find(|&line| lines[line].top == carried_top))
            .flatten();
        if let Some(first) = carried {
            for line in &mut lines[first..index] {
                line.top -= carried_top;
            }
            run_start = first;
            changed = true;
        } else {
            run_start = index;
        }
    }
    changed.then(|| Paragraph {
        lines,
        ..paragraph.clone()
    })
}

/// A cell's own coordinate runs: `(raw_offset, extent)` pairs in the same
/// flattened, restart-aware space `crop_cell_content` recomputes from
/// scratch for a given `[start, end)` window. A cell with no internal
/// restart yields exactly one run spanning its whole content.
fn cell_coordinate_runs(cell: &crate::model::TableCell) -> Vec<(i64, i64)> {
    let lines = cell
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.lines.iter())
        .collect::<Vec<_>>();
    let mut offsets = Vec::<i64>::new();
    let mut extents = Vec::<i64>::new();
    let mut raw_offset = 0_i64;
    let mut run_extent = 0_i64;
    let mut previous_top = None;
    let mut trailing_spacing = 0_i64;
    for line in &lines {
        if previous_top.is_some_and(|top| line.top <= top) {
            offsets.push(raw_offset);
            extents.push(run_extent);
            raw_offset = raw_offset
                .saturating_add(run_extent)
                .saturating_add(trailing_spacing);
            run_extent = 0;
        }
        run_extent = run_extent.max(
            line.top
                .saturating_add(line.height.max(line.text_height).max(0)),
        );
        trailing_spacing = line.spacing.max(0);
        previous_top = Some(line.top);
    }
    offsets.push(raw_offset);
    extents.push(run_extent);
    offsets.into_iter().zip(extents).collect()
}

/// Stored page fragments of a merged cell in a noAdjust CELL table. Hancom
/// restarts such a cell's line coordinates on every page it continues onto,
/// and the reference paints each page piece as that run's own extent plus the
/// cell margins, with no inter-run spacing (성과보고서 s35: r13c5 pieces
/// 41020+282 and 59180+282, r16c5 7400+282 and 41000+282). rhwp likewise
/// renders a straddling merged cell from its stored fragment groups (#4698).
///
/// Returns every run's `(offset, extent)` in a cell-local content space where
/// run k starts after the earlier runs' extents plus margins. A zero-extent
/// run (s35 r20c5's zero-height "O 측정방법" line) joins the following run;
/// s35 r23c5 (runs 0 and 41000) is still one 41000+282 piece.
fn merged_fragment_runs(table: &Table, cell: &crate::model::TableCell) -> Option<Vec<(i64, i64)>> {
    if !table.no_adjust
        || !table.page_break.eq_ignore_ascii_case("CELL")
        || cell.row_span <= 1
        || !cell.tables.is_empty()
        || cell
            .paragraphs
            .iter()
            .any(|paragraph| !paragraph.objects.is_empty() || paragraph.has_inline_table)
    {
        return None;
    }
    let runs = cell_coordinate_runs(cell);
    if runs.len() < 2 {
        return None;
    }
    let margins = cell
        .margin_top
        .max(0)
        .saturating_add(cell.margin_bottom.max(0));
    let mut offset = 0_i64;
    Some(
        runs.into_iter()
            .map(|(_, extent)| {
                let run = (offset, extent);
                if extent > 0 {
                    offset = offset.saturating_add(extent).saturating_add(margins);
                }
                run
            })
            .collect(),
    )
}

/// Whether every stored page piece of a merged cell (`merged_fragment_runs`:
/// its run's extent plus the margins) fits the cell's own declared span. Then
/// each page holds its piece in the rows it already has, and the pieces'
/// total is not a height the rows must reach: 성과보고서 s35's r3c5 (3 rows
/// of 1848; pieces 2622 and 3342 on two pages) is 420 over as a stack, and
/// that 420 on its last row put every row below it 1.48mm lower on the
/// second page than the reference. A piece longer than the span (r13c5's
/// 41302 and 59462) still takes the whole total in its last row, which is
/// what the page cut inside that row measures from.
fn stored_pieces_fit_declared_span(table: &Table, cell: &crate::model::TableCell) -> bool {
    let Some(runs) = merged_fragment_runs(table, cell) else {
        return false;
    };
    let margins = cell
        .margin_top
        .max(0)
        .saturating_add(cell.margin_bottom.max(0));
    runs.iter()
        .filter(|(_, extent)| *extent > 0)
        .all(|(_, extent)| extent.saturating_add(margins) <= cell.box_units.height)
}

/// Each line's content-space top under `merged_fragment_runs`, with its run.
fn merged_fragment_line_tops(
    cell: &crate::model::TableCell,
    runs: &[(i64, i64)],
) -> Vec<(i64, usize)> {
    let mut run_index = 0_usize;
    let mut previous_top = None;
    let mut tops = Vec::new();
    for line in cell
        .paragraphs
        .iter()
        .flat_map(|paragraph| &paragraph.lines)
    {
        if previous_top.is_some_and(|top| line.top <= top) {
            run_index = (run_index + 1).min(runs.len().saturating_sub(1));
        }
        previous_top = Some(line.top);
        tops.push((runs[run_index].0.saturating_add(line.top), run_index));
    }
    tops
}

/// A `pageBreak="CELL"` row where some cell has 2+ coordinate runs in its
/// linesegarray splits exactly at those restarts, not at a generic
/// page-capacity offset (see 샘플/다중행_표3, where a 9-line run-vs-run split
/// matched the reference bit for bit while a byte-offset split did not). A
/// single-column row is the degenerate case of this with one cell; a
/// single-*row* table can also restart in only some of its columns
/// (`s2/tbl-anchor[12-0]`, 11 columns, 1 row, restarts confined to columns 6
/// and 8) — every OTHER cell in the row still needs cropping to the same cut
/// points to stay internally consistent, which is why this reports the
/// row's cut points rather than one cell's; the caller crops every cell in
/// the row against them via the existing geometric-intersection path
/// (originally built for row-span cells, but it works for any cell).
/// Multi-row, multi-column tables additionally qualify when their body has
/// an actual repeated header and compatible stored runs. `repeatHeader=1`
/// alone is insufficient: 어린이제품's 63x2 table has header=0 on every
/// cell and keeps its capacity-based continuation. Conflicting cell runs
/// also keep that fallback, rather than creating tiny extra page pieces.
/// Each returned piece is `(raw_offset, raw_length, rendered_height)`, in a
/// raw (margin-free) space local to this row's own restarting cells;
/// `rendered_height` is the tallest participating cell's own content height
/// once cropped to that piece's window, plus that cell's own top+bottom
/// margin (measured directly with `line_flow_height`, not
/// `cell_content_height` -- the latter only adds margin in some of its
/// branches). Returns `None` when no cell in the row has a genuine restart,
/// or when any cell in the row spans multiple rows or holds a nested table
/// (geometry this hasn't been verified against),
/// leaving the row to the existing byte-offset fallback.
fn row_restart_pieces(table: &Table, row: usize) -> Option<Vec<(i64, i64, i64)>> {
    let repeated_body = stored_repeated_header_runs(table) && row > 0;
    if table.columns != 1 && table.rows != 1 && !repeated_body {
        return None;
    }
    if table
        .cells
        .iter()
        .any(|cell| cell.row == row && (cell.row_span != 1 || !cell.tables.is_empty()))
    {
        return None;
    }
    let cells = table
        .cells
        .iter()
        .filter(|cell| cell.row == row)
        .collect::<Vec<_>>();
    if cells.is_empty() {
        return None;
    }
    if repeated_body
        && cells.iter().any(|cell| {
            cell.paragraphs
                .iter()
                .any(|p| !p.objects.is_empty() || p.has_inline_table)
        })
    {
        return None;
    }
    let mut runs_by_cell = Vec::with_capacity(cells.len());
    let mut has_restart = false;
    let mut total_extent = 0_i64;
    for cell in &cells {
        let runs = cell_coordinate_runs(cell);
        if runs.len() >= 2 {
            has_restart = true;
        }
        if let Some(&(last_offset, last_extent)) = runs.last() {
            total_extent = total_extent.max(last_offset.saturating_add(last_extent));
        }
        runs_by_cell.push(runs);
    }
    if !has_restart {
        return None;
    }
    let mut cut_points = std::collections::BTreeSet::<i64>::new();
    cut_points.insert(0);
    cut_points.insert(total_extent);
    // Run k of every cell is the same page, so cut each page at the earliest
    // cell's run k. Cutting at every cell's own offset instead leaves slivers
    // between cells whose earlier runs differ in length (s2/tbl-anchor[12-0]:
    // c8 restarts at 42240 and c6 at 42600, and the sliver both added 1282 to
    // the continuation and moved c6 down by 360). Keep the per-cell offsets
    // when some cell's earlier run would reach past that common cut.
    let run_count = runs_by_cell.iter().map(Vec::len).max().unwrap_or(0);
    let aligned_cuts = (1..run_count)
        .map(|run| {
            runs_by_cell
                .iter()
                .filter_map(|runs| runs.get(run).map(|&(offset, _)| offset))
                .min()
                .unwrap_or(total_extent)
        })
        .collect::<Vec<_>>();
    let aligned = runs_by_cell.iter().all(|runs| {
        runs.iter().enumerate().all(|(run, &(offset, extent))| {
            aligned_cuts
                .get(run)
                .is_none_or(|&cut| offset.saturating_add(extent) <= cut)
        })
    });
    if repeated_body && !aligned {
        return None;
    }
    if aligned {
        cut_points.extend(aligned_cuts);
    } else {
        for runs in &runs_by_cell {
            for &(offset, _) in runs.iter().skip(1) {
                cut_points.insert(offset);
            }
        }
    }
    let boundaries = cut_points.into_iter().collect::<Vec<_>>();
    if boundaries.len() < 2 {
        return None;
    }
    Some(
        boundaries
            .windows(2)
            .map(|window| {
                let (start, end) = (window[0], window[1]);
                let rendered = cells
                    .iter()
                    .map(|cell| {
                        let mut clone = (*cell).clone();
                        crop_cell_content(&mut clone, start, end, total_extent);
                        // Measure the cropped content directly with
                        // `line_flow_height` rather than `cell_content_height`:
                        // the latter's margin inclusion is inconsistent across
                        // its branches (single-line shortcut: yes; the general
                        // multi-line case: no, since that addition was tried
                        // and reverted -- see `cell_content_height`'s own
                        // comment), which would silently drop this cell's own
                        // margin whenever a piece crops down to more than one
                        // line. Every piece gets both margins, not just edge
                        // ones (confirmed against 샘플/다중행_표3).
                        let content = line_flow_height(
                            &clone
                                .paragraphs
                                .iter()
                                .flat_map(|paragraph| paragraph.lines.iter())
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                        content
                            .saturating_add(cell.margin_top.max(0))
                            .saturating_add(cell.margin_bottom.max(0))
                    })
                    .max()
                    .unwrap_or(1)
                    .max(1);
                (start, end.saturating_sub(start), rendered)
            })
            .collect(),
    )
}

/// A row outside `row_restart_pieces`' scope (no cell in it has a genuine
/// coordinate restart) still needs the pre-existing generic byte-offset
/// splitter as a fallback -- pure atomic row-to-row placement matches the
/// fragment count for some documents, but that alone does not verify the
/// continuation text boundaries and produces wrong page counts elsewhere (e.g.
/// 어린이제품 안전 특별법 시행령 일부개정령안, whose reference page count
/// is only matched by allowing a mid-row cut). The byte-offset splitter
/// allows that mid-row cut only when the row has more than one paragraph (a
/// coarse proxy for "this row's content isn't a single atomic flow").
fn row_has_multiple_paragraphs(table: &Table, row: usize) -> bool {
    table
        .cells
        .iter()
        .any(|cell| cell.row == row && cell.paragraphs.len() > 1)
}

/// The single-paragraph counterpart to [`row_has_multiple_paragraphs`]: a
/// cell in this row has exactly one paragraph, but that paragraph's own
/// lines still restart (a later line's `top` not increasing from the one
/// before) -- a manual line break's local coordinate reset, not a second
/// `hp:p`. 샘플/다중행_표와 다중행_표2 (사용자 제작 통제 샘플) showed this
/// exact shape inflating a row exactly like a second paragraph does, but
/// the multi-paragraph check alone missed it, leaving that row's
/// continuation fragment short by the same `SPLIT_PARAGRAPH_EXTRA` this
/// function's sibling adds. See AGENTS.md's "통제된 다중 행 샘플 2개로
/// 일반화 검증" entry.
fn row_has_single_paragraph_coordinate_restart(table: &Table, row: usize) -> bool {
    table.cells.iter().any(|cell| {
        cell.row == row
            && cell.paragraphs.len() == 1
            && cell
                .paragraphs
                .first()
                .is_some_and(|p| p.lines.windows(2).any(|pair| pair[1].top <= pair[0].top))
    })
}

fn distribute_deficit(values: &mut [i64], start: usize, span: usize, required: i64) {
    let end = start.saturating_add(span).min(values.len());
    if start >= end {
        return;
    }
    let current = values[start..end].iter().sum::<i64>();
    let deficit = required.saturating_sub(current);
    if deficit == 0 {
        return;
    }
    let width = end - start;
    let share = deficit / width as i64;
    let remainder = deficit % width as i64;
    for (index, value) in values[start..end].iter_mut().enumerate() {
        *value += share + i64::from(index < remainder as usize);
    }
}

#[derive(Debug, Clone, Copy)]
struct RowPiece {
    row: usize,
    /// Raw (margin-free) crop-window start within the row's own
    /// restart-aware coordinate space, matching `effective_row_heights`
    /// (née `row_heights`) and what `crop_cell_content` recomputes.
    offset: i64,
    /// Raw (margin-free) crop-window length, addressing the same space.
    height: i64,
    /// The size this piece actually occupies once rendered: a stored run's
    /// extent plus cell margins, or the first fragment's frame fill. This
    /// is *not* the source interval `height`; extending the painted frame
    /// must not consume any of the next source run. `None`
    /// means "use `height`", preserving the original single-piece-per-row
    /// behavior exactly.
    rendered_height: Option<i64>,
}

/// A `pageBreak="CELL"` table with exactly one row and one column can only
/// need a *mid-cell* continuation (no sibling row can push it to a new
/// page), so the reference exporter's own split points are directly
/// available: `샘플/다중 문단 셀 콘텐츠 높이` proved the source's
/// coordinate restarts in the cell's `linesegarray` (`top` resetting to a
/// value at or below the previous line's `top`) are exactly where the
/// reference fragments the table, not a computed page capacity. Zero
/// restarts means the reference draws one fragment regardless of how far
/// it overflows a physical page; each restart starts a new fragment sized
/// to that coordinate run's own extent plus the cell's margin (the table's
/// own outMargin is added afterwards by the renderer via
/// `table_frame_extra_y`, so it is intentionally excluded here).
///
/// Returns `None` for any shape this hasn't been verified against (more
/// than one row or column, a missing or spanning sole cell, a nested
/// or no lines at all), so the caller falls back to the
/// page-capacity splitter used for every other table shape.
fn split_single_cell_table_by_coordinate_restarts(table: &Table) -> Option<Vec<Table>> {
    if table.rows != 1 || table.columns != 1 || table.cells.len() != 1 {
        return None;
    }
    let cell = &table.cells[0];
    if cell.row != 0 || cell.column != 0 || cell.row_span != 1 || cell.col_span != 1 {
        return None;
    }
    let lines = cell
        .paragraphs
        .iter()
        .flat_map(|paragraph| paragraph.lines.iter())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    // Collect each coordinate run's (start, extent) in the same flattened,
    // restart-aware coordinate space `crop_cell_content` recomputes from
    // scratch for a given [start, end) window.
    let mut runs = Vec::<(i64, i64)>::new();
    let mut run_start = 0_i64;
    let mut run_extent = 0_i64;
    let mut previous_top = None;
    let mut trailing_spacing = 0_i64;
    for line in &lines {
        if previous_top.is_some_and(|top| line.top <= top) {
            runs.push((run_start, run_extent));
            run_start = run_start
                .saturating_add(run_extent)
                .saturating_add(trailing_spacing);
            run_extent = 0;
        }
        run_extent = run_extent.max(
            line.top
                .saturating_add(line.height.max(line.text_height).max(0)),
        );
        trailing_spacing = line.spacing.max(0);
        previous_top = Some(line.top);
    }
    runs.push((run_start, run_extent));
    if runs.len() <= 1 {
        // A single coordinate run never fragments, however far it overflows
        // a physical page; `layout_table` already gave the whole table the
        // correct (content-driven) box height.
        return Some(vec![table.clone()]);
    }
    let cell_margin = cell
        .margin_top
        .max(0)
        .saturating_add(cell.margin_bottom.max(0));
    let total_extent = run_start.saturating_add(run_extent).max(1);
    Some(
        runs.into_iter()
            .enumerate()
            .map(|(fragment_index, (start, extent))| {
                let mut fragment_cell = cell.clone();
                crop_cell_content(&mut fragment_cell, start, start + extent, total_extent);
                // A float anchored in this coordinate run may extend beyond
                // its final text line. Keep the complete float inside the
                // fragment that owns its anchor.
                let floating_bottom = fragment_cell
                    .tables
                    .iter()
                    .filter(|nested| !nested.anchor.treat_as_char)
                    .map(|nested| {
                        let alignment = match fragment_cell.vertical_align.as_str() {
                            "TOP" => 0,
                            "BOTTOM" => nested.out_margin_bottom.max(0),
                            _ => nested.out_margin_bottom.max(0) / 2,
                        };
                        nested
                            .box_units
                            .y
                            .saturating_add(nested.box_units.height.max(0))
                            .saturating_add(nested.out_margin_top.max(0))
                            .saturating_add(nested.out_margin_bottom.max(0))
                            .saturating_add(alignment)
                    })
                    .max()
                    .unwrap_or(0);
                let cell_height = extent
                    .max(floating_bottom)
                    .saturating_add(cell_margin)
                    .max(1);
                let mut fragment = table.clone();
                fragment.box_units.y = if fragment_index == 0 {
                    table.box_units.y
                } else {
                    // A continuation starts at the page's own top, not at the
                    // anchor, so the anchor line's adjustment does not follow.
                    fragment.anchor_top_adjustment = 0;
                    0
                };
                // The renderer displays a cell at `cell.y - table.y` (its
                // position within the table's own frame); matching the
                // fragment's own absolute y here keeps that difference at 0.
                // Leaving this at a bare 0 while `fragment.box_units.y` sits
                // at a nonzero page position (다수 부동표2's first table, a
                // fresh-page float with `vertOffset=0` but a small nonzero
                // own coordinate) understated the cell's rendered top by
                // that same amount -- enough to miss the boundary paths'
                // exact-0/exact-height match and silently drop a border.
                fragment_cell.box_units.y = fragment.box_units.y;
                fragment_cell.box_units.height = cell_height;
                fragment.box_units.height = cell_height;
                fragment.row_heights = vec![cell_height];
                fragment.cells = vec![fragment_cell];
                fragment
            })
            .collect(),
    )
}

/// The source grid's own height: every row's declared `cellSz` height (the
/// tallest single-row cell in it). `None` when a row has no single-row cell.
fn declared_grid_total(table: &Table) -> Option<i64> {
    (0..table.rows)
        .map(|row| {
            table
                .cells
                .iter()
                .filter(|cell| cell.row == row && cell.row_span == 1)
                .map(|cell| cell.box_units.height)
                .max()
        })
        .sum()
}

/// The repeated-header body whose source page runs can supply both crop
/// ranges and painted heights. Keep noAdjust's separately verified grid
/// and merged-cell rules; a header flag without a designated cell is not
/// evidence for this continuation model.
fn stored_repeated_header_runs(table: &Table) -> bool {
    !table.no_adjust
        && table.rows > 1
        && table.columns > 1
        && table.repeat_header
        && table.page_break.eq_ignore_ascii_case("CELL")
        && table
            .cells
            .iter()
            .any(|cell| cell.row == 0 && cell.is_header)
}

/// The painted height of a continuation fragment's first row when a
/// row-spanning cell resumes there. The row grid cuts such a cell at the
/// piece's top, but the cell's next stored run can begin later than that cut
/// (`crop_cell_content` re-bases its lines to that run). The gap in between is
/// in no line of the source, and Hancom does not paint it: the reference draws
/// the continuation as its run's extent plus the cell margins (성과보고서
/// s8/r1c3: 20052 + 282, not the grid's 21138; s20/r4c3: 5040 + 282, not
/// 5466). Left in the row, the gap becomes spare height that a CENTER cell
/// turns into a shifted first line.
///
/// Only the whole row is refitted, never below what any cell crossing it
/// needs, and never by more than the largest gap; the source range that
/// selects each cell's lines does not change. Rows whose cells continue past
/// them, nested tables and objects keep the grid height: their occupancy is
/// not measured here. noAdjust tables have their own stored-fragment sizing
/// (`merged_fragment_runs`).
fn continuation_fit_height(
    table: &Table,
    row_heights: &[i64],
    effective_row_heights: &[i64],
    piece: &RowPiece,
) -> Option<i64> {
    let row = piece.row;
    let row_height = *row_heights.get(row)?;
    if table.no_adjust
        || !table.page_break.eq_ignore_ascii_case("CELL")
        || piece.rendered_height.is_some()
        || piece.offset != 0
        || piece.height != row_height
        || effective_row_heights.get(row) != Some(&row_height)
    {
        return None;
    }
    let row_top = effective_row_heights.iter().take(row).sum::<i64>();
    // A cell's own share of the row: its lines and margins. A stored cellSz
    // is the grid's height, not a content need -- s8's r3c4..c8 declare the
    // same 21138 and the reference paints them 71.73mm (20334) like r3c3.
    let share = |cell: &crate::model::TableCell| {
        let lines = cell
            .paragraphs
            .iter()
            .flat_map(|paragraph| paragraph.lines.iter())
            .cloned()
            .collect::<Vec<_>>();
        line_flow_height(&lines)
            .saturating_add(cell.margin_top.max(0))
            .saturating_add(cell.margin_bottom.max(0))
    };
    let mut need = 0_i64;
    let mut largest_gap = 0_i64;
    for cell in table
        .cells
        .iter()
        .filter(|cell| cell.row <= row && row < cell.row.saturating_add(cell.row_span))
    {
        if cell.row.saturating_add(cell.row_span) != row + 1
            || !cell.tables.is_empty()
            || cell
                .paragraphs
                .iter()
                .any(|p| !p.objects.is_empty() || p.has_inline_table)
        {
            return None;
        }
        if cell.row == row {
            need = need.max(share(cell));
            continue;
        }
        // A header copy of row 0 is not a cropped continuation.
        if cell.row == 0 {
            return None;
        }
        let cell_top = effective_row_heights.iter().take(cell.row).sum::<i64>();
        let end = row_top.saturating_add(row_height) - cell_top;
        let mut resumed = cell.clone();
        let gap = crop_cell_content(
            &mut resumed,
            row_top - cell_top,
            end,
            end.max(cell.box_units.height.max(1)),
        );
        // A label cell whose text ended before the cut has nothing left to
        // paint here; it only follows the row.
        if resumed.paragraphs.iter().all(|p| p.lines.is_empty()) {
            continue;
        }
        need = need.max(share(&resumed));
        largest_gap = largest_gap.max(gap);
    }
    if largest_gap <= 0 {
        return None;
    }
    let fitted = need.max(row_height.saturating_sub(largest_gap)).max(1);
    (fitted < row_height).then_some(fitted)
}

fn split_table(
    table: &Table,
    page: &crate::model::PageSpec,
    source_declared_height: i64,
    repeat_complete_header: bool,
    declared_grid_total: Option<i64>,
) -> Vec<Table> {
    if table.page_break.eq_ignore_ascii_case("NONE") {
        return vec![table.clone()];
    }
    // `treatAsChar` tables are inline objects owned by their anchor line.
    // Splitting them as page-level tables creates zero-sized continuation
    // fragments when the line's local coordinate is already near the page
    // bottom. The reference exporter keeps the inline table intact; the line
    // box is enlarged by the renderer when necessary.
    if table.anchor.treat_as_char {
        return vec![table.clone()];
    }
    if table.rows == 0 {
        return vec![table.clone()];
    }
    if table.page_break.eq_ignore_ascii_case("CELL") {
        if let Some(fragments) = split_single_cell_table_by_coordinate_restarts(table) {
            return fragments;
        }
    }
    let row_heights = if table.row_heights.len() >= table.rows {
        table.row_heights.clone()
    } else {
        infer_grid_dimensions(table, table.columns, table.rows).1
    };
    // A single-row table has no header row distinct from its own body, so
    // "repeating the header" on a mid-row-split continuation would just
    // duplicate row 0 against itself (see s2/tbl-anchor[12-0], an 11-column,
    // single-row table whose continuation fragment must not reserve its own
    // row as a repeating header).
    let repeat_header_enabled = table.repeat_header
        && table.rows > 1
        && table
            .cells
            .iter()
            .any(|cell| cell.row == 0 && cell.is_header);
    // Preserve the legacy continuation reservation independently of copying.
    // Removing the reservation for header=0 also changes source line cuts;
    // that needs a separate geometry rule, not an inferred header designation.
    let repeated_header_height = if table.repeat_header && table.rows > 1 {
        row_heights.first().copied().unwrap_or(0)
    } else {
        0
    };
    let allow_row_split = table.page_break.eq_ignore_ascii_case("CELL");
    // A noAdjust CELL continuation is cut at Hancom's own frame: the body
    // less the table's outer margins and the repeated header. 성과보고서's
    // budget tables (outMargin 283/283) end every continuation at 254.99mm
    // of content; the old 101-unit allowance left 468 more, which only the
    // 2000-unit split threshold below kept from cutting too late. With the
    // exact frame, the whole-line snap decides instead (as rhwp splits a
    // row only when its first line fits, `pagination/engine.rs`).
    let exact_frame = allow_row_split && table.no_adjust;
    let mut first_capacity = page.height
        - page.margin_top
        - page.header
        - page.margin_bottom
        - page.footer
        - table.box_units.y
        - if repeat_complete_header {
            0
        } else {
            TABLE_EDGE_ALLOWANCE
        };
    // The original table frame describes the first page fragment even when
    // the laid-out row grid spans many pages. For s35/tbl-anchor[10-0] it is
    // 11250 HWPUNIT; adding the table's two 283-unit outer margins yields
    // the reference's 41.68mm first-fragment HTML height.
    if repeat_complete_header && source_declared_height > 0 {
        first_capacity = first_capacity.min(source_declared_height);
    }
    // More generally, a CELL table that overflows its first page and whose
    // stored frame is shorter than its own declared rows (but fits that page)
    // records the first fragment in that frame: 다중행_표5's second 7x1 table
    // stores 27353 for 7x4112 rows and the reference cuts its last row there
    // (98.49mm, with the remaining 1431 after the repeated header). That cut
    // is Hancom's own, so the small-overflow leniency below does not apply.
    // Rows that only grew from our content estimate do not qualify, nor do
    // noAdjust tables (성과보고서 s11/s25 store frames far below their rows
    // and the reference does not cut there).
    let row_total = row_heights.iter().sum::<i64>();
    let frame_first_fragment = !repeat_complete_header
        && !table.no_adjust
        && table.page_break.eq_ignore_ascii_case("CELL")
        && source_declared_height > 0
        && declared_grid_total.is_some_and(|total| source_declared_height < total)
        && row_total > first_capacity
        && source_declared_height <= first_capacity.saturating_add(TABLE_EDGE_ALLOWANCE);
    if frame_first_fragment {
        first_capacity = source_declared_height;
    }
    let frame_fill = repeat_complete_header || frame_first_fragment;
    // Keep the same continuation budget Hancom uses for a table stream even
    // when a later fragment does not visibly repeat the header. The header
    // row still participates in the source table's page geometry; dropping
    // that reservation on continuation fragments lets rows drift into the
    // bottom margin and changes subsequent row boundaries.
    let later_capacity = page.height
        - page.margin_top
        - page.header
        - page.margin_bottom
        - page.footer
        - repeated_header_height
        - if exact_frame {
            table.out_margin_top.max(0) + table.out_margin_bottom.max(0)
        } else if allow_row_split {
            SPLIT_PAGE_ALLOWANCE
        } else {
            0
        };
    let mut chunks = Vec::<(Vec<RowPiece>, i64, i64, bool)>::new();
    let mut current = Vec::<RowPiece>::new();
    let mut current_height = 0i64;
    let mut capacity = first_capacity.max(1);
    let mut current_has_header = false;
    // Mutated only by the legacy byte-offset path below (`SPLIT_PARAGRAPH_EXTRA`);
    // rows handled via `row_restart_pieces` never touch this, so their raw
    // offsets stay valid against the unmutated `row_heights` entry for that row.
    let mut effective_row_heights = row_heights.clone();
    // Every continuation repeats a designated header, including one that
    // starts in the middle of a row: 성과보고서's budget tables, the 현행/개정안
    // tables of 통합관리요령 and 공통운영요령, and 다중행_표5 all do. An earlier
    // suppression after mid-row cuts only matched positional DOM counts while
    // the cuts themselves were still wrong (화학산업 공고's unmatched DOM
    // tokens fall 968 -> 312 without it).
    let continuation_has_header = repeat_complete_header || repeat_header_enabled;
    let stored_header_runs = stored_repeated_header_runs(table);
    for row in 0..table.rows {
        let base_row_height = row_heights.get(row).copied().unwrap_or(0);
        // A row's own coordinate restarts (if any) are the only points the
        // reference ever cuts it at; a byte-offset "it doesn't fit, slice it
        // anyway" split doesn't happen (verified against 샘플/다중행_표3 for
        // single-column rows and s2/tbl-anchor[12-0] for a multi-column one).
        // A row with no detected restart still falls back to the pre-existing
        // byte-offset splitter below: pure atomic placement matches some
        // documents (s6/tbl-anchor[32-0]) but not others (어린이제품 안전
        // 특별법 시행령 일부개정령안 needs the mid-row cut to match the
        // reference's own page count), so it stays the default until that
        // distinction is itself understood.
        let restart_pieces = if allow_row_split {
            row_restart_pieces(table, row)
        } else {
            None
        };
        if let Some(pieces) = restart_pieces {
            for (offset, extent, rendered) in pieces {
                let available = capacity.saturating_sub(current_height);
                // A stored continuation of a repeated-header body opens a
                // page even when our frame estimate could fit both runs.
                // Its own height is measured afresh, not row height - cut.
                if current_height > 0
                    && (available <= 0
                        || rendered > available
                        || (stored_header_runs && row > 0 && offset > 0))
                {
                    // Repeat only explicitly designated header cells when the
                    // table enables repetition; an ordinary first row is
                    // not a header merely because it precedes a restart.
                    chunks.push((current, current_height, capacity, current_has_header));
                    current = Vec::new();
                    current_height = 0;
                    current_has_header = repeat_header_enabled;
                    capacity = later_capacity.max(1);
                }
                current.push(RowPiece {
                    row,
                    offset,
                    height: extent,
                    rendered_height: Some(rendered),
                });
                current_height += rendered;
            }
            continue;
        }

        let available = capacity.saturating_sub(current_height);
        if allow_row_split
            && current_height > 0
            && available >= MIN_ROW_SPLIT_HEIGHT
            && base_row_height > available
            && base_row_height - available >= MIN_ROW_SPLIT_HEIGHT
            && (row_has_multiple_paragraphs(table, row)
                || row_has_single_paragraph_coordinate_restart(table, row))
        {
            if let Some(row_height) = effective_row_heights.get_mut(row) {
                *row_height = row_height.saturating_add(SPLIT_PARAGRAPH_EXTRA);
            }
        }
        let row_height = effective_row_heights
            .get(row)
            .copied()
            .unwrap_or(base_row_height);
        // A full-width merged single cell is structurally a section title,
        // not ordinary body content -- 성과보고서's `tbl id=1151676850`
        // alternates exactly these ("◈ 전략목표N : [...]") against a
        // colSpan=1 body pair. `row_overflow_would_lose_content` calls a
        // single-line title's own overflow harmless (nothing past its one
        // line's own top is ever dropped), so the small-overflow leniency
        // below would gladly shrink such a title to whatever sliver of
        // `available` remains rather than deferring it -- title-Ⅴ needed
        // 3182 raw units with only 1635 available (already below
        // `MIN_ROW_SPLIT_HEIGHT`, so the ordinary `available <
        // MIN_ROW_SPLIT_HEIGHT` deferral just below would have moved it
        // whole), and used to render there anyway (shrunk, at the very
        // bottom of the page) while its entire body moved to the next one
        // regardless. The exclusion below is scoped to exactly that
        // condition (`available < MIN_ROW_SPLIT_HEIGHT` too) rather than
        // every shortfall: a title barely short of fitting by a marginal
        // amount well above that threshold (성과보고서's own floating 5x3
        // sidebar has one, `available` only ~230 raw units short) still
        // takes the leniency path unchanged, since forcing that case
        // through the byte-offset "just take `available`" branch instead
        // changed the surrounding fragment's own overflow bookkeeping
        // enough to move unrelated content (verified as a regression via
        // `finance_contents_shapes_and_page_restart_are_preserved` and
        // `para_relative_float_measures_from_its_paragraph_margin`).
        let row_cells_here = table
            .cells
            .iter()
            .filter(|cell| cell.row == row)
            .collect::<Vec<_>>();
        let is_full_width_single_cell_row = allow_row_split
            && table.columns > 1
            && row_cells_here.len() == 1
            && row_cells_here[0].col_span >= table.columns
            && row_cells_here[0].row_span == 1;
        let mut offset = 0i64;
        while offset < row_height.max(1) {
            let available = capacity.saturating_sub(current_height);
            if available <= 0 {
                chunks.push((current, current_height, capacity, current_has_header));
                current = Vec::new();
                current_height = 0;
                current_has_header = continuation_has_header;
                capacity = later_capacity.max(1);
                continue;
            }
            let remaining = row_height.max(1) - offset;
            let take = if !allow_row_split && current_height == 0 {
                remaining
            } else {
                remaining.min(available)
            };
            let split_remainder = remaining.saturating_sub(available);
            let take = if take < remaining && current_height > 0 {
                if !allow_row_split {
                    chunks.push((current, current_height, capacity, current_has_header));
                    current = Vec::new();
                    current_height = 0;
                    current_has_header = continuation_has_header;
                    capacity = later_capacity.max(1);
                    continue;
                } else if !(frame_first_fragment && chunks.is_empty())
                    && !(is_full_width_single_cell_row && available < MIN_ROW_SPLIT_HEIGHT)
                    && split_remainder < MIN_ROW_SPLIT_HEIGHT
                    && (split_remainder <= SPLIT_PAGE_ALLOWANCE
                        || !row_overflow_would_lose_content(table, row, available))
                {
                    remaining
                } else if frame_first_fragment
                    && chunks.is_empty()
                    && available < MIN_ROW_SPLIT_HEIGHT
                    && split_remainder < MIN_ROW_SPLIT_HEIGHT
                    && row_content_fits(table, row, available)
                {
                    // The stored frame ends inside a row too short to split
                    // on either side. Hancom keeps it in the frame, trimmed
                    // to the frame, when its content fits there; the rest of
                    // the row is not continued. 성과보고서 s2's 29x3 table
                    // (59416) ends 1779 into the 2891-unit "Ⅴ-1" row, whose
                    // one line needs 1382; the reference draws that row
                    // 6.28mm tall on the first page and continues at "Ⅴ-2".
                    // s26 (732 left, 1282 needed) still moves its row.
                    remaining
                } else if available < MIN_ROW_SPLIT_HEIGHT
                    // `snap_no_adjust_text_cut` below keeps whole padded
                    // lines and defers the row when none fits: s7 r97 has
                    // 1114 left for a 1282 first line and moves; s26 r97 has
                    // 1372 and splits after it, as the reference does.
                    && !(exact_frame && row_is_no_adjust_text(table, row))
                {
                    chunks.push((current, current_height, capacity, current_has_header));
                    current = Vec::new();
                    current_height = 0;
                    current_has_header = continuation_has_header;
                    capacity = later_capacity.max(1);
                    continue;
                } else {
                    take
                }
            } else {
                take
            };
            let take = if allow_row_split && take < remaining {
                row_restart_cut_before(table, row, offset, offset.saturating_add(take))
                    .map(|cut| cut - offset)
                    .or_else(|| snap_take_to_whole_line(table, row, offset, take))
                    .or_else(|| {
                        snap_merged_continuation_cut(table, &row_heights, row, offset, take)
                    })
                    .unwrap_or(take)
            } else {
                take
            };
            // Small-overflow leniency can select the whole remaining row,
            // then trim it back to capacity while rendering. Check that
            // eventual trim boundary too, not only explicit partial takes.
            let take = if allow_row_split && (take < remaining || take > available) {
                snap_no_adjust_text_cut(table, row, offset, take.min(available)).unwrap_or(take)
            } else {
                take
            };
            if take == 0 && current_height > 0 {
                chunks.push((current, current_height, capacity, current_has_header));
                current = Vec::new();
                current_height = 0;
                current_has_header = continuation_has_header;
                capacity = later_capacity.max(1);
                continue;
            }
            // An over-page line must still make progress on an empty page.
            let take = if take == 0 {
                remaining.min(available)
            } else {
                take
            };
            current.push(RowPiece {
                row,
                offset,
                height: take,
                rendered_height: None,
            });
            current_height += take;
            offset += take;
            if offset < row_height.max(1) {
                chunks.push((current, current_height, capacity, current_has_header));
                current = Vec::new();
                current_height = 0;
                current_has_header = continuation_has_header;
                capacity = later_capacity.max(1);
            }
        }
    }
    if !current.is_empty() {
        chunks.push((current, current_height, capacity, current_has_header));
    }
    // The source frame can end after the last complete source line but before
    // the next row begins. Preserve its empty tail in the painted box while
    // leaving the source cut at the original line boundary. 성과보고서's
    // noAdjust=0 CELL tables (s2 59416, s8 8686+566, s20, s24, s26) paint
    // their first fragment at exactly that frame too.
    if frame_fill && source_declared_height > 0 {
        if let Some((pieces, height, capacity, _)) = chunks.first_mut() {
            let frame_height = source_declared_height.min(*capacity);
            if *height < frame_height {
                if let Some(last) = pieces.last_mut() {
                    let gap = frame_height - *height;
                    last.rendered_height = Some(last.rendered_height.unwrap_or(last.height) + gap);
                    *height = frame_height;
                }
            }
        }
    }
    if chunks.len() <= 1 {
        return vec![table.clone()];
    }

    chunks
        .into_iter()
        .enumerate()
        .map(
            |(fragment_index, (pieces, height, fragment_capacity, include_repeated_header))| {
                let source_y = table.box_units.y;
                let fragment_y = if fragment_index == 0 { source_y } else { 0 };
                // A continuation's first row is refitted to a resumed
                // row-spanning cell's stored run; the chunk was filled with
                // the grid height, which only the painted box drops.
                let fitted = pieces
                    .iter()
                    .enumerate()
                    .map(|(piece_index, piece)| {
                        if fragment_index == 0 || piece_index > 0 {
                            return None;
                        }
                        continuation_fit_height(table, &row_heights, &effective_row_heights, piece)
                    })
                    .collect::<Vec<_>>();
                let fitted_drop = pieces
                    .iter()
                    .zip(&fitted)
                    .map(|(piece, fit)| fit.map_or(0, |fit| piece.height - fit))
                    .sum::<i64>();
                let raw_overflow = height
                    .saturating_sub(fitted_drop)
                    .saturating_sub(fragment_capacity)
                    .max(0);
                // A restart-based piece's rendered size is the run's own
                // extent plus margin, full stop -- it is never clipped to
                // remaining page capacity (that's the whole point of cutting
                // at restarts instead of a byte offset). Only a legacy
                // byte-offset piece can still be trimmed here.
                let fragment_overflow = if raw_overflow > SPLIT_PAGE_ALLOWANCE
                    && pieces
                        .last()
                        .is_none_or(|piece| piece.rendered_height.is_none())
                {
                    raw_overflow
                } else {
                    0
                };
                let mut cells = Vec::new();
                if include_repeated_header {
                    let headers = table
                        .cells
                        .iter()
                        .filter(|cell| cell.row == 0 && (cell.is_header || repeat_complete_header))
                        .cloned()
                        .map(|mut cell| {
                            let original = cell.id.clone();
                            cell.repeated_header = true;
                            cell.repeated_from = Some(original);
                            cell.box_units.y = fragment_y;
                            cell
                        })
                        .collect::<Vec<_>>();
                    cells.splice(0..0, headers);
                }

                let header_offset = if include_repeated_header {
                    repeated_header_height
                } else {
                    0
                };
                let rendered_height = pieces
                    .iter()
                    .enumerate()
                    .map(|(piece_index, piece)| {
                        let base_height =
                            row_heights.get(piece.row).copied().unwrap_or(piece.height);
                        let effective_height = effective_row_heights
                            .get(piece.row)
                            .copied()
                            .unwrap_or(base_height);
                        let row_height = if let Some(rendered) = piece.rendered_height {
                            rendered
                        } else if let Some(fit) = fitted[piece_index] {
                            fit
                        } else if piece.offset == 0
                            && piece.height == effective_height
                            && effective_height > base_height
                        {
                            base_height
                        } else {
                            piece.height
                        };
                        if piece_index + 1 == pieces.len() {
                            row_height.saturating_sub(fragment_overflow).max(1)
                        } else {
                            row_height
                        }
                    })
                    .sum::<i64>();
                #[derive(Debug, Clone, Copy)]
                struct RenderedPiece {
                    original_top: i64,
                    original_bottom: i64,
                    rendered_top: i64,
                    rendered_bottom: i64,
                    row: usize,
                    height: i64,
                    /// Painted shorter than its source rows (`fitted`), so a
                    /// cell reaching the row's end stops at the painted end.
                    fitted: bool,
                }

                let mut rendered_pieces = Vec::with_capacity(pieces.len());
                let mut piece_top = 0i64;
                let piece_count = pieces.len();
                for (piece_index, piece) in pieces.iter().enumerate() {
                    let original_top = effective_row_heights
                        .iter()
                        .take(piece.row)
                        .sum::<i64>()
                        .saturating_add(piece.offset);
                    let base_height = row_heights.get(piece.row).copied().unwrap_or(piece.height);
                    let effective_height = effective_row_heights
                        .get(piece.row)
                        .copied()
                        .unwrap_or(base_height);
                    let trim_full_row = piece.offset == 0
                        && piece.height == effective_height
                        && effective_height > base_height;
                    let full_rendered_height = if let Some(rendered) = piece.rendered_height {
                        rendered
                    } else if let Some(fit) = fitted[piece_index] {
                        fit
                    } else if trim_full_row {
                        base_height
                    } else {
                        piece.height
                    };
                    let rendered_height = if piece_index + 1 == piece_count {
                        full_rendered_height
                            .saturating_sub(fragment_overflow)
                            .max(1)
                    } else {
                        full_rendered_height
                    };
                    rendered_pieces.push(RenderedPiece {
                        original_top,
                        original_bottom: original_top.saturating_add(piece.height),
                        rendered_top: piece_top,
                        rendered_bottom: piece_top.saturating_add(rendered_height),
                        row: piece.row,
                        height: full_rendered_height,
                        fitted: fitted[piece_index].is_some(),
                    });
                    piece_top = piece_top.saturating_add(rendered_height);
                }

                // A row-spanning source cell can intersect several row pieces
                // in the same page fragment. Emit it once and union those
                // intersections; otherwise the same source id is rendered
                // repeatedly inside one HTML table.
                let repeated_ids = cells.iter().map(|cell| cell.id.clone()).collect::<Vec<_>>();
                for original in &table.cells {
                    if repeated_ids.iter().any(|id| id == &original.id) {
                        continue;
                    }
                    let cell_top = effective_row_heights.iter().take(original.row).sum::<i64>();
                    let row_end = original
                        .row
                        .saturating_add(original.row_span)
                        .min(effective_row_heights.len());
                    let cell_height = effective_row_heights
                        .get(original.row.min(row_end)..row_end)
                        .unwrap_or(&[])
                        .iter()
                        .sum::<i64>()
                        .max(original.box_units.height.max(1));
                    let cell_bottom = cell_top.saturating_add(cell_height);
                    let mut fragment_top = None;
                    let mut fragment_bottom = None;
                    let mut source_start = None;
                    let mut source_end = None;
                    for piece in &rendered_pieces {
                        if original.row.saturating_add(original.row_span) <= piece.row
                            || cell_top >= piece.original_bottom
                            || cell_bottom <= piece.original_top
                        {
                            continue;
                        }
                        let intersection_top = cell_top.max(piece.original_top);
                        let mut intersection_bottom = cell_bottom.min(piece.original_bottom);
                        if piece.rendered_bottom < piece.rendered_top + piece.height
                            && original.row == piece.row
                            && original.row_span == 1
                        {
                            let rendered_source_bottom = piece
                                .original_top
                                .saturating_add(piece.rendered_bottom - piece.rendered_top);
                            intersection_bottom = intersection_bottom.min(rendered_source_bottom);
                        }
                        if intersection_bottom <= intersection_top {
                            continue;
                        }
                        let local_top = piece
                            .rendered_top
                            .saturating_add(intersection_top - piece.original_top);
                        let mut local_bottom = piece
                            .rendered_top
                            .saturating_add(intersection_bottom - piece.original_top);
                        // A source interval ends at the last glyph; its
                        // painted piece still includes both cell margins.
                        // Frame fill likewise belongs only to the painted
                        // box, never to the source range of the next piece.
                        if (frame_fill || stored_header_runs)
                            && piece.rendered_bottom
                                > piece.rendered_top + (piece.original_bottom - piece.original_top)
                            && intersection_bottom == piece.original_bottom
                        {
                            local_bottom = piece.rendered_bottom;
                        }
                        if piece.fitted {
                            local_bottom = local_bottom.min(piece.rendered_bottom);
                        }
                        fragment_top =
                            Some(fragment_top.map_or(local_top, |top: i64| top.min(local_top)));
                        fragment_bottom = Some(
                            fragment_bottom
                                .map_or(local_bottom, |bottom: i64| bottom.max(local_bottom)),
                        );
                        source_start = Some(
                            source_start.map_or(intersection_top - cell_top, |start: i64| {
                                start.min(intersection_top - cell_top)
                            }),
                        );
                        source_end = Some(
                            source_end.map_or(intersection_bottom - cell_top, |end: i64| {
                                end.max(intersection_bottom - cell_top)
                            }),
                        );
                    }
                    let (
                        Some(fragment_top),
                        Some(fragment_bottom),
                        Some(source_start),
                        Some(source_end),
                    ) = (fragment_top, fragment_bottom, source_start, source_end)
                    else {
                        continue;
                    };
                    let mut cell = original.clone();
                    cell.box_units.y = fragment_y
                        .saturating_add(header_offset)
                        .saturating_add(fragment_top);
                    cell.box_units.height = fragment_bottom.saturating_sub(fragment_top).max(1);
                    let preserve_content = source_start <= 0 && source_end >= cell_height;
                    if !preserve_content {
                        if let Some(runs) = merged_fragment_runs(table, original) {
                            crop_merged_fragments(&mut cell, &runs, source_start, source_end);
                        } else {
                            crop_cell_content(&mut cell, source_start, source_end, cell_height);
                        }
                    }
                    cells.push(cell);
                }
                let mut fragment = table.clone();
                fragment.fragment_rows = pieces
                    .first()
                    .zip(pieces.last())
                    .map(|(first, last)| (first.row, last.row));
                if fragment_y == 0 && table.box_units.y != 0 {
                    fragment.anchor_top_adjustment = 0;
                }
                fragment.box_units.y = fragment_y;
                fragment.box_units.height = rendered_height
                    .saturating_add(if include_repeated_header {
                        repeated_header_height
                    } else {
                        0
                    })
                    .max(1);
                fragment.cells = cells;
                fragment
            },
        )
        .collect()
}

/// Whether every one-row cell of `row` is a noAdjust text cell, the rows
/// whose cuts `snap_no_adjust_text_cut` measures (row-spanning cells are cut
/// separately by `snap_merged_continuation_cut`).
fn row_is_no_adjust_text(table: &Table, row: usize) -> bool {
    let cells = table
        .cells
        .iter()
        .filter(|cell| cell.row == row && cell.row_span == 1)
        .collect::<Vec<_>>();
    !cells.is_empty()
        && cells
            .iter()
            .all(|cell| is_no_adjust_text_growth(table, cell))
}

/// Keep a noAdjust text row's capacity cut outside its padded glyph boxes.
/// Grid fitting has already replaced cellSz here, so testing for a stale
/// declared height again would incorrectly disable the safeguard.
fn snap_no_adjust_text_cut(table: &Table, row: usize, start: i64, take: i64) -> Option<i64> {
    // Every one-row cell must be measurable: its lines, and a nested float's
    // frame as one piece from its host line (the host's recorded line sits
    // at the frame's top and the next paragraph at its bottom: 성과보고서
    // s109's "직종별 수요·공급차이" table, host line 10880 + 19132 = 30012).
    // A cut chosen from a short label must not move an adjacent cell whose
    // occupancy this helper cannot measure (paragraph objects, inline tables).
    let measurable = |cell: &crate::model::TableCell| {
        table.no_adjust
            && table.rows > 1
            && table.columns > 1
            && table.page_break.eq_ignore_ascii_case("CELL")
            && !cell.paragraphs.is_empty()
            && cell
                .paragraphs
                .iter()
                .all(|p| p.objects.is_empty() && !p.has_inline_table)
            && cell.paragraphs.iter().any(|p| !p.lines.is_empty())
    };
    let cells = table
        .cells
        .iter()
        .filter(|cell| cell.row == row && cell.row_span == 1)
        .collect::<Vec<_>>();
    if cells.iter().any(|cell| !measurable(cell)) {
        return None;
    }
    let mut intervals = Vec::new();
    let mut cells_run_starts = Vec::new();
    for cell in &cells {
        let runs = cell_coordinate_runs(cell);
        if runs.len() >= 2 {
            cells_run_starts.extend(runs.iter().skip(1).map(|&(offset, _)| offset));
        }
    }
    for cell in cells {
        let margins = cell.margin_top.max(0) + cell.margin_bottom.max(0);
        let mut offset = 0_i64;
        let mut previous = None;
        let mut bottom = 0_i64;
        let mut spacing = 0_i64;
        for paragraph in &cell.paragraphs {
            for (index, line) in paragraph.lines.iter().enumerate() {
                if previous.is_some_and(|top| line.top <= top) {
                    offset += bottom + spacing;
                    bottom = 0;
                }
                let height = line.height.max(line.text_height).max(0);
                bottom = bottom.max(line.top + height);
                spacing = line.spacing.max(0);
                previous = Some(line.top);
                let top = offset + line.top;
                intervals.push((top, top + height, top + height + margins));
                if index > 0 {
                    continue;
                }
                for nested in cell.tables.iter().filter(|nested| {
                    !nested.anchor.treat_as_char
                        && nested
                            .source_anchor
                            .as_ref()
                            .is_some_and(|anchor| anchor.paragraph_key == paragraph.key)
                }) {
                    let frame = nested.box_units.height
                        + nested.out_margin_top.max(0)
                        + nested.out_margin_bottom.max(0);
                    intervals.push((top, top + frame, top + frame + margins));
                }
            }
        }
    }
    // The first line (or nested frame) that does not fit with the cell's
    // margins moves to the next page; the cut is its top. Then no other
    // line may be sliced by that cut. Chaining the padded bottoms instead
    // walks back through every line whose pitch is under height + margins
    // (s109's 1440 pitch against 1200 + 282) to the row's top.
    let end = start.saturating_add(take);
    // A cell's coordinate restarts are the pages Hancom stored; it never
    // cuts inside one. 성과보고서 s109's r2c5 runs start at 2880, 46200,
    // 90360, 133812 and the reference breaks exactly there; the capacity
    // cut kept the empty line at 2880 (then 3960, 48120, ...), one line late
    // on every later page.
    let run_cut = cells_run_starts
        .iter()
        .filter(|&&offset| offset > start && offset <= end)
        .max()
        .copied();
    // No stored page starts inside this page's capacity: then our capacity
    // is the estimate that is off (the rows above it are laid out a little
    // taller than Hancom's), not the stored run. Keep the cut as it was --
    // 통합관리요령 공고 p12's r71 keeps its first run's two lines (next run
    // at 2520, 1282 left), as in the reference.
    if run_cut.is_none() && !cells_run_starts.is_empty() {
        return None;
    }
    let mut cut = intervals
        .iter()
        .filter(|(top, _, padded)| *top >= start && *top < end && *padded > end)
        .map(|(top, _, _)| *top)
        .min()
        .unwrap_or(end);
    if let Some(run_cut) = run_cut {
        cut = cut.min(run_cut);
    }
    loop {
        let next = intervals
            .iter()
            .filter(|(top, bottom, _)| *top >= start && *top < cut && *bottom > cut)
            .map(|(top, _, _)| *top)
            .min();
        match next {
            Some(top) => cut = top,
            None => break,
        }
    }
    (cut < end).then_some(cut - start)
}

/// Guards shared by `row_restart_cut_before` and `snap_take_to_whole_line`:
/// geometry neither is verified against. `no_adjust`/single-row-or-column
/// tables are `row_restart_pieces`' scope, not the byte-offset fallback's;
/// a real repeated header has its own continuation budget (the
/// 73-row/4-column table is a known counterexample); a row-spanning,
/// nested-table, or object-holding cell has geometry this measures wrong.
fn row_restart_cut_scope_blocked(table: &Table, row: usize) -> bool {
    if table.no_adjust || table.rows <= 1 || table.columns <= 1 {
        return true;
    }
    if table.repeat_header
        && table
            .cells
            .iter()
            .any(|cell| cell.row == 0 && cell.is_header)
    {
        return true;
    }
    table.cells.iter().any(|cell| {
        (cell.row < row && cell.row.saturating_add(cell.row_span) > row)
            || !cell.tables.is_empty()
            || cell
                .paragraphs
                .iter()
                .any(|p| !p.objects.is_empty() || p.has_inline_table)
    })
}

/// Refine an existing capacity cut only when it would consume part of a
/// source restart's run (see the loop body below for what "consume" covers).
/// This does not invent a page break at every restart -- a naive cut that
/// lands before a run even starts, or exactly on/after its own end, is left
/// alone.
///
/// Each row is judged purely on its own geometry: whether some *other* row
/// in the table also restarts is irrelevant here, since `capacity` and
/// `current_height` at the call site already reflect however every earlier
/// row was actually placed, whatever method placed it. An early attempt
/// at this generalization did gate on other rows restarting at all (a
/// per-table static check, not per-row), and that broke matches in the
/// 63-row case -- but verified again with this row-scoped version and the
/// whole-run window below, the full 55-sample corpus turned up no
/// regressions from applying it at every genuinely restarting row in a
/// table, including 성과보고서's own 13-row table (전략목표Ⅰ/Ⅲ/Ⅴ, three
/// separate restarting rows, all now cut correctly) and both previously
/// counterexample-cited documents (어린이제품's 63-row table, 화학산업's
/// 73-row table) unaffected either way.
fn row_restart_cut_before(table: &Table, row: usize, start: i64, end: i64) -> Option<i64> {
    if row_restart_cut_scope_blocked(table, row) {
        return None;
    }
    let mut agreed_cut = None;
    for cell in table.cells.iter().filter(|cell| cell.row == row) {
        let runs = cell_coordinate_runs(cell);
        if runs.len() <= 1 {
            continue;
        }
        if cell.row_span != 1 {
            return None;
        }
        let mut previous_top = None;
        let mut run_index = 0;
        let mut candidate = None;
        for line in cell
            .paragraphs
            .iter()
            .flat_map(|paragraph| &paragraph.lines)
        {
            if previous_top.is_some_and(|top| line.top <= top) {
                run_index += 1;
                let (cut, extent) = runs[run_index];
                // The whole run, not just its first line: a run is
                // everything HWP itself placed on one page between two
                // restarts (or a restart and the cell's own end), so a
                // naive capacity cut landing anywhere inside it -- not only
                // in its very first line -- still means our own capacity
                // estimate disagrees with HWP's, and the run belongs
                // whole on the next fragment either way (성과보고서's
                // "ㅇ 글로벌기업 헤드쿼터, ..." two-line wrap: the naive cut
                // fell into its *second* physical line, past the first
                // line's own advance, and used to go uncorrected).
                if line.top == 0 && cut > start && cut < end && end < cut.saturating_add(extent) {
                    candidate = Some(cut);
                }
            }
            previous_top = Some(line.top);
        }
        let cut = candidate?;
        if agreed_cut.is_some_and(|previous| previous != cut) {
            return None;
        }
        agreed_cut = Some(cut);
    }
    agreed_cut
}

/// Every line-start boundary in a row's own cells, in the same
/// restart-aware coordinate space `crop_cell_content` uses for its `top`
/// comparisons (`cell_coordinate_runs`' accumulation, but per line rather
/// than per run). `None` outside `row_restart_cut_before`'s own scope
/// (see `row_restart_cut_scope_blocked`) -- this is only ever a fallback
/// for when that function declines the row for a reason other than
/// several rows restarting, so it honors the same restrictions.
fn row_line_starts(table: &Table, row: usize) -> Option<Vec<i64>> {
    if row_restart_cut_scope_blocked(table, row) {
        return None;
    }
    let mut starts = std::collections::BTreeSet::new();
    starts.insert(0);
    for cell in table.cells.iter().filter(|cell| cell.row == row) {
        let mut run_offset = 0_i64;
        let mut run_height = 0_i64;
        let mut previous_top = None;
        let mut trailing_spacing = 0_i64;
        for line in cell
            .paragraphs
            .iter()
            .flat_map(|paragraph| &paragraph.lines)
        {
            if previous_top.is_some_and(|top| line.top <= top) {
                run_offset = run_offset
                    .saturating_add(run_height)
                    .saturating_add(trailing_spacing);
                run_height = 0;
            }
            let height = line.height.max(line.text_height).max(0);
            run_height = run_height.max(line.top.saturating_add(height));
            previous_top = Some(line.top);
            trailing_spacing = line.spacing.max(0);
            starts.insert(run_offset.saturating_add(line.top));
        }
    }
    Some(starts.into_iter().collect())
}

/// Round a mid-row byte-offset cut down to the nearest line start, when
/// `row_restart_cut_before` cannot itself place it at a source restart.
/// `crop_cell_content` assigns a line to a fragment by its own top alone
/// (`top >= start && top < end`), never trimming a line's rendered extent
/// to the window -- a cut that lands inside a line's own span still
/// renders that line in full, only for the fragment's own (shorter) box
/// to clip it. `row_restart_cut_before` already prevents this by snapping
/// to a genuine restart, but only for a row whose table has exactly one
/// restarting row; 샘플/2022회계연도 성과보고서's `tbl id=1151676850`
/// (13 rows x 2 columns, `pageBreak=CELL`) restarts in three separate
/// rows, putting every one of them outside that scope, so its "◈
/// 전략목표Ⅲ" row's plain capacity cut used to land partway through the
/// second physical line of "ㅇ 글로벌기업 헤드쿼터, ...", clipping it.
/// This is a narrower fix than a genuine multi-row restart solution (it
/// only guarantees no line is bisected -- it does not defer the whole
/// wrapped paragraph the way a single restarting row's cut does), but it
/// is safe as a fallback everywhere: a capacity cut that already lands on
/// or near a line boundary is unaffected, and one that doesn't now moves
/// only the still-partial line to the next fragment instead of clipping it.
fn snap_take_to_whole_line(table: &Table, row: usize, offset: i64, take: i64) -> Option<i64> {
    let target = offset.saturating_add(take);
    let boundary = row_line_starts(table, row)?
        .into_iter()
        .rfind(|&start| start > offset && start <= target)?;
    Some(boundary - offset)
}

/// A row-spanning continuation's line coordinates start at the first row
/// of the cell, not at the row currently being split. The ordinary row-local
/// snap deliberately excludes this case. Translate the accumulated lines
/// into the current row before checking whether a capacity cut bisects one.
fn snap_merged_continuation_cut(
    table: &Table,
    row_heights: &[i64],
    row: usize,
    offset: i64,
    take: i64,
) -> Option<i64> {
    if !table.page_break.eq_ignore_ascii_case("CELL") {
        return None;
    }
    let cells = table
        .cells
        .iter()
        .filter(|c| c.row <= row && row < c.row + c.row_span)
        .collect::<Vec<_>>();
    let has_continuation = cells.iter().any(|c| {
        c.row_span > 1 && {
            let lines = c
                .paragraphs
                .iter()
                .flat_map(|p| &p.lines)
                .collect::<Vec<_>>();
            lines.windows(2).any(|p| p[1].top <= p[0].top)
        }
    });
    if !has_continuation
        || cells.iter().any(|c| {
            !c.tables.is_empty()
                || c.paragraphs
                    .iter()
                    .any(|p| !p.objects.is_empty() || p.has_inline_table)
        })
    {
        return None;
    }
    let mut intervals = Vec::new();
    for cell in cells {
        let row_shift = row_heights[cell.row..row].iter().sum::<i64>();
        // A stored page fragment is never split: cut before the whole run.
        if let Some(runs) = merged_fragment_runs(table, cell) {
            for (offset, extent) in runs.into_iter().filter(|(_, extent)| *extent > 0) {
                let top = offset - row_shift;
                intervals.push((top, top + extent));
            }
            continue;
        }
        let (mut run_offset, mut extent, mut spacing) = (0_i64, 0_i64, 0_i64);
        let mut previous_top = None;
        for line in cell.paragraphs.iter().flat_map(|p| &p.lines) {
            if previous_top.is_some_and(|top| line.top <= top) {
                run_offset += extent + spacing;
                extent = 0;
            }
            let height = line.height.max(line.text_height).max(0);
            let top = run_offset + line.top - row_shift;
            intervals.push((top, top + height));
            extent = extent.max(line.top + height);
            spacing = line.spacing.max(0);
            previous_top = Some(line.top);
        }
    }
    let mut cut = offset + take;
    loop {
        let earlier = intervals
            .iter()
            .filter(|&&(top, bottom)| top < cut && cut < bottom)
            .map(|&(top, _)| top)
            .min();
        match earlier {
            Some(top) if top > offset => cut = top,
            Some(_) => return None,
            None => return (cut < offset + take).then_some(cut - offset),
        }
    }
}

/// Whether letting a row overflow into the next fragment's trim -- rather
/// than cutting it there -- would silently drop a real rendered line. The
/// trim below caps a byte-offset piece's rendered height at `available`;
/// most short overflows (`split_remainder < MIN_ROW_SPLIT_HEIGHT`) are just
/// trailing blank space past the last real line, where absorbing them is
/// fine. But 샘플/다단계 표 샘플's row 3 (a plain two-line wrap with no
/// restart to cut at, "◈ 전략목표Ⅱ ...") has real text starting past that
/// point, and the trim discarded it outright rather than deferring it to a
/// continuation fragment. This mirrors `crop_cell_content`'s own run-based
/// cumulative `top` so the two agree on where the cut actually falls.
fn row_overflow_would_lose_content(table: &Table, row: usize, available: i64) -> bool {
    table.cells.iter().any(|cell| {
        if cell.row != row {
            return false;
        }
        let mut run_offset = 0_i64;
        let mut run_height = 0_i64;
        let mut previous_top = None;
        let mut trailing_spacing = 0_i64;
        for paragraph in &cell.paragraphs {
            for line in &paragraph.lines {
                if previous_top.is_some_and(|top| line.top <= top) {
                    run_offset = run_offset
                        .saturating_add(run_height)
                        .saturating_add(trailing_spacing);
                    run_height = 0;
                }
                let height = line.height.max(line.text_height).max(0);
                let local_bottom = line.top.saturating_add(height);
                run_height = run_height.max(local_bottom);
                previous_top = Some(line.top);
                trailing_spacing = line.spacing.max(0);
                let top = run_offset.saturating_add(line.top);
                if top >= available {
                    return true;
                }
            }
        }
        false
    })
}

/// Whether every cell of `row` fits, glyph boxes and top/bottom margins
/// included, into the first `available` units. Only plain one-row text cells
/// qualify: a row-spanning cell, a nested table or object, or a coordinate
/// restart has occupancy this measure does not model.
fn row_content_fits(table: &Table, row: usize, available: i64) -> bool {
    let cells = table
        .cells
        .iter()
        .filter(|cell| cell.row <= row && row < cell.row + cell.row_span)
        .collect::<Vec<_>>();
    !cells.is_empty()
        && cells.iter().all(|cell| {
            if cell.row != row
                || cell.row_span != 1
                || !cell.tables.is_empty()
                || cell
                    .paragraphs
                    .iter()
                    .any(|p| !p.objects.is_empty() || p.has_inline_table)
            {
                return false;
            }
            let lines = cell
                .paragraphs
                .iter()
                .flat_map(|p| &p.lines)
                .collect::<Vec<_>>();
            if lines.windows(2).any(|pair| pair[1].top <= pair[0].top) {
                return false;
            }
            let extent = lines
                .iter()
                .map(|line| line.top + line.height.max(line.text_height).max(0))
                .max()
                .unwrap_or(0);
            extent + cell.margin_top.max(0) + cell.margin_bottom.max(0) <= available
        })
}

/// Crop a `merged_fragment_runs` cell to one page piece. A piece that begins
/// before a stored run's origin starts at that origin, like the reference's
/// continuation of s35 r3c5, whose second run sits at the cell's top.
fn crop_merged_fragments(
    cell: &mut crate::model::TableCell,
    runs: &[(i64, i64)],
    start: i64,
    end: i64,
) {
    let tops = merged_fragment_line_tops(cell, runs);
    let base = tops
        .iter()
        .find(|(top, _)| *top >= start && *top < end)
        .map_or(start, |(_, run)| runs[*run].0.max(start));
    let mut line_index = 0_usize;
    cell.paragraphs.retain_mut(|paragraph| {
        if paragraph.lines.is_empty() {
            return true;
        }
        let mut lines = Vec::new();
        for (index, source_line) in paragraph.lines.iter().enumerate() {
            let top = tops[line_index].0;
            line_index += 1;
            if top >= start && top < end {
                let mut line = source_line.clone();
                line.text_end = Some(paragraph.lines.get(index + 1).map_or_else(
                    || paragraph.tokens.iter().map(|token| token.logical_len).sum(),
                    |next| next.textpos,
                ));
                line.top = top.saturating_sub(base);
                lines.push(line);
            }
        }
        paragraph.lines = lines;
        if paragraph.lines.is_empty() {
            paragraph.tokens.clear();
        }
        !paragraph.lines.is_empty()
    });
}

/// Crops `cell` to the source window `[start, end)` and re-bases its lines to
/// the piece's top. Returns how far the first kept line's stored run begins
/// past `start` (0 when the window opens inside that run or keeps nothing):
/// the lines move up by that much, but the window itself is not shorter, so a
/// caller sizing the painted piece from the window would leave that gap.
fn crop_cell_content(
    cell: &mut crate::model::TableCell,
    start: i64,
    end: i64,
    source_height: i64,
) -> i64 {
    if start <= 0 && end >= source_height {
        return 0;
    }
    // Cell paragraphs share a coordinate stream. Accumulate only when the
    // source resets that stream, not at every paragraph boundary.
    let mut run_offset = 0_i64;
    let mut run_height = 0_i64;
    let mut previous_top = None;
    let mut trailing_spacing = 0_i64;
    let mut retained_anchors = std::collections::HashMap::new();
    // A window that opens before the first kept line's run starts draws that
    // run in its own page coordinates. The run-aligned cuts of
    // `row_restart_pieces` open c6 of s2/tbl-anchor[12-0] 360 before its run.
    let mut base = None;
    cell.paragraphs.retain_mut(|paragraph| {
        // A source paragraph without line geometry still needs its existing
        // fallback. A paragraph whose source lines were all cropped away is
        // different: retaining it would synthesize a new empty output line.
        if paragraph.lines.is_empty() {
            return true;
        }
        let mut lines = Vec::new();
        let mut first_line_top = None;
        for (index, source_line) in paragraph.lines.iter().enumerate() {
            if previous_top.is_some_and(|top| source_line.top <= top) {
                run_offset = run_offset
                    .saturating_add(run_height)
                    .saturating_add(trailing_spacing);
                run_height = 0;
            }
            let height = source_line.height.max(source_line.text_height).max(0);
            let local_bottom = source_line.top.saturating_add(height);
            run_height = run_height.max(local_bottom);
            previous_top = Some(source_line.top);
            trailing_spacing = source_line.spacing.max(0);
            let top = run_offset.saturating_add(source_line.top);
            if top < start || top >= end {
                continue;
            }
            let base = *base.get_or_insert(start.max(run_offset));
            if index == 0 {
                let adjustment = if source_line.text_height > 0 {
                    (source_line.text_height / 20).max(1)
                } else {
                    paragraph.line_top_offset
                };
                first_line_top = Some((top.saturating_sub(base), adjustment));
            }
            // Assign a line to exactly one fragment, preserving its token offset.
            let mut line = source_line.clone();
            line.text_end = Some(paragraph.lines.get(index + 1).map_or_else(
                || paragraph.tokens.iter().map(|token| token.logical_len).sum(),
                |next| next.textpos,
            ));
            line.top = top.saturating_sub(base);
            lines.push(line);
        }
        paragraph.lines = lines;
        if let Some((top, adjustment)) = first_line_top {
            retained_anchors.insert(paragraph.id.clone(), top.saturating_sub(adjustment));
        }
        if paragraph.lines.is_empty() {
            paragraph.tokens.clear();
        }
        !paragraph.lines.is_empty()
    });
    cell.tables.retain_mut(|table| {
        let Some((prefix, ordinal)) = table.id.rsplit_once("/tbl-anchor[") else {
            return true;
        };
        let Some((ordinal, _)) = ordinal.split_once('-') else {
            return true;
        };
        let host = format!("{prefix}/p{ordinal}");
        let Some(&anchor) = retained_anchors.get(&host) else {
            return false;
        };
        if let Some(original_anchor) = table.anchor_y {
            let delta = anchor.saturating_sub(original_anchor);
            table.anchor_y = Some(anchor);
            if !table.anchor.treat_as_char {
                translate_table(table, 0, delta);
            }
        }
        true
    });
    base.map_or(0, |base| base - start)
}

/// Where a `vertRelTo="PARA"` float measures from: the paragraph's top, its
/// first line's flow position less the space before it. 샘플/2026 대한민국
/// 산업단지 발전 유공자 모집 공고's line under "정부포상 및 장관표창에 대한
/// 동의서" (vertOffset 2815) sits at 1920 + 2815 in the reference: the previous
/// paragraph's end, not the host line at 2920 = 1920 + its space before 1000.
/// rhwp records `para_start_y` before the paragraph's own spacing too. At a
/// page top the space before is not drawn.
pub(crate) fn para_float_anchor(first_line_top: i64, margin_before: i64) -> i64 {
    first_line_top.saturating_sub(margin_before.max(0)).max(0)
}

fn position_object(
    object: &PositionedObject,
    line_top: i64,
    _page_index: usize,
) -> PositionedObject {
    let mut result = object.clone();
    if result.anchor.treat_as_char {
        result.box_units.y += line_top;
    } else {
        // `flowWithText` only keeps the object inside the page area; a
        // `vertRelTo="PARA"` object measures from its paragraph either way
        // (the same rule as floating tables, `table_position`). 샘플/그라데이션
        // 도형's four PARA rects (flowWithText=0) otherwise all sat at the
        // area top instead of their paragraphs' 0/12280/17230/33060. rhwp
        // picks the reference by `vert_rel_to` alone (`VertRelTo::Para =>
        // para_y`).
        result.box_units.x += result.anchor.horz_offset;
        result.box_units.y += result.anchor.vert_offset
            + if result.anchor.flow_with_text
                || result.anchor.vert_rel_to.eq_ignore_ascii_case("PARA")
            {
                line_top
            } else {
                0
            };
    }
    result
}

pub fn token_signature(tokens: &[Token]) -> String {
    let mut hasher = Sha256::new();
    for token in tokens {
        hasher.update(token.manifest_kind().as_bytes());
        hasher.update([0]);
        hasher.update(token.visible_text().as_bytes());
        hasher.update([0xff]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::model::{Anchor, PageSpec};

    use super::*;

    fn fill_tokens(kinds: Vec<TokenKind>) -> Vec<Token> {
        kinds
            .into_iter()
            .map(|kind| Token {
                kind,
                logical_len: 1,
                char_style_id: 0,
                hyperlink: None,
            })
            .collect()
    }

    #[test]
    fn lines_fill_their_width_as_the_alignment_spreads_them() {
        // rhwp `needs_word_distribution`: a justified line unless it ends its
        // paragraph or the author broke it, a 나눔 line unless broken, a 배분
        // line always (성과보고서 ps696 spreads its last line too).
        let text = || fill_tokens(vec![TokenKind::Text("해소하여, 외국인 정주여건".into())]);
        assert_eq!(line_fill("justify", false, &text()), Some(LineFill::Spaces));
        assert_eq!(line_fill("justify", true, &text()), None);
        assert_eq!(
            line_fill("distribute_space", true, &text()),
            Some(LineFill::Spaces)
        );
        assert_eq!(
            line_fill("distribute", true, &text()),
            Some(LineFill::Letters)
        );
        for align in ["left", "center", "right"] {
            assert_eq!(line_fill(align, false, &text()), None);
        }
        let broken = fill_tokens(vec![TokenKind::Text("가 나".into()), TokenKind::LineBreak]);
        assert_eq!(line_fill("justify", false, &broken), None);
        assert_eq!(line_fill("distribute_space", false, &broken), None);
        let inside = fill_tokens(vec![
            TokenKind::Text("가 나".into()),
            TokenKind::Control {
                kind: "lineBreak".into(),
            },
        ]);
        assert_eq!(line_fill("justify", false, &inside), None);
    }

    #[test]
    fn tabs_leaders_and_single_letters_keep_their_width() {
        let tab = fill_tokens(vec![
            TokenKind::Text("목 차".into()),
            TokenKind::Tab {
                width: 4000,
                leader: 3,
            },
            TokenKind::Text("3".into()),
        ]);
        assert_eq!(line_fill("justify", false, &tab), None);
        let leader = fill_tokens(vec![TokenKind::Text("항목 ------ 3".into())]);
        assert_eq!(line_fill("justify", false, &leader), None);
        let single = fill_tokens(vec![TokenKind::Text(" 가 ".into())]);
        assert_eq!(line_fill("distribute", false, &single), None);
    }

    #[test]
    fn centi_mm_uses_integer_rounding() {
        assert_eq!(hwp_to_centi_mm(7200), 2540);
        assert_eq!(hwp_to_centi_mm(-7200), -2540);
        assert_eq!(css_mm(7200), "25.40mm");
        assert_eq!(css_mm(59528), "210mm");
        // An `hls` line box keeps the hundredths that every other length
        // drops; the reference writes `line-height:24.00mm` there but
        // `height:24mm` in the same declaration.
        assert_eq!(css_mm_hundredths(7200), "25.40mm");
        assert_eq!(css_mm_hundredths(59528), "210.00mm");
        assert_eq!(css_mm_hundredths(0), "0.00mm");
    }

    #[test]
    fn an_exact_half_rounds_toward_zero() {
        // 16740 HWPUNIT is exactly 59.055mm and the reference writes
        // top:59.05mm; -180 is exactly -0.635mm and it writes top:-0.63mm
        // (샘플/글자 크기별 독립 문단). Twelve such ties across 27 sample
        // documents all take the bucket nearer zero, and none takes the
        // farther one.
        assert_eq!(hwp_to_centi_mm(16740), 5905);
        assert_eq!(hwp_to_centi_mm(-16740), -5905);
        assert_eq!(hwp_to_centi_mm(-180), -63);
        // Anything off the tie keeps ordinary rounding in both directions.
        assert_eq!(hwp_to_centi_mm(16741), 5906);
        assert_eq!(hwp_to_centi_mm(16739), 5905);
        assert_eq!(hwp_to_centi_mm(-16741), -5906);
    }

    #[test]
    fn merged_cell_deficit_is_distributed_across_its_span() {
        let mut rows = vec![0_i64; 3];
        distribute_deficit(&mut rows, 0, 3, 3000);
        assert_eq!(rows, vec![1000, 1000, 1000]);

        let mut columns = vec![1000_i64, 0, 0];
        distribute_deficit(&mut columns, 0, 3, 3000);
        assert_eq!(columns.iter().sum::<i64>(), 3000);
        assert!(columns[1] > 0 && columns[2] > 0);
    }

    #[test]
    fn line_height_curve_predicts_sizes_the_table_never_measured() {
        // Each expectation is the raw value whose css_mm is the reference's
        // own line-height string for that text height, in a document where
        // the table has no entry: 2.31mm/2.96mm (장비관리요령 개정(안)),
        // 13.04mm (산업단지 공고), 13.16mm (공통운영요령 개정(안)).
        assert_eq!(super::measured_line_height_curve(844), 654);
        assert_eq!(super::measured_line_height_curve(1054), 838);
        assert_eq!(super::measured_line_height_curve(3696), 3696);
        assert_eq!(super::measured_line_height_curve(3731), 3731);
        // The ratio stops at 1.0, so large sizes map to themselves rather
        // than growing past their own glyph box (13320 -> 46.99mm in
        // 샘플/그라디언트 샘플, where the old 4/5 fallback gave 37.59mm).
        assert_eq!(super::measured_line_height_curve(7758), 7758);
        assert_eq!(super::measured_line_height_curve(13320), 13320);
        // It stays within one rounding step of every entry the table does
        // list, which is why it is the same rule rather than a second one.
        for (height, measured) in [
            (1000, 790),
            (1500, 1258),
            (2000, 1779),
            (2500, 2349),
            (3000, 2971),
            (4000, 4000),
        ] {
            let curve = super::measured_line_height_curve(height);
            assert!(
                (curve - measured).abs() <= 2,
                "curve {curve} strays from measured {measured} at {height}"
            );
        }
    }

    #[test]
    fn line_height_uses_measured_metrics_for_extended_sizes() {
        let for_height = |text_height| {
            rendered_line_height(
                &LineSeg {
                    text_height,
                    ..LineSeg::default()
                },
                false,
            )
        };
        // Each pair was measured from a reference HTML's own line-height for
        // that exact text height (see AGENTS.md, line-height table entry).
        assert_eq!(for_height(100), 71);
        assert_eq!(for_height(850), 658);
        assert_eq!(for_height(1048), 834);
        assert_eq!(for_height(1050), 834);
        assert_eq!(for_height(1550), 1310);
        assert_eq!(for_height(2100), 1891);
        assert_eq!(for_height(2200), 2001);
        assert_eq!(for_height(2400), 2231);
        assert_eq!(for_height(2350), 2174);
        assert_eq!(for_height(2600), 2469);
        assert_eq!(for_height(2800), 2716);
        assert_eq!(for_height(3199), 3199);
        assert_eq!(for_height(3200), 3200);
        assert_eq!(for_height(3500), 3500);
        assert_eq!(for_height(4000), 4000);
        assert_eq!(for_height(4300), 4300);
        assert_eq!(for_height(4700), 4700);
        assert_eq!(for_height(5300), 5301);
    }

    #[test]
    fn line_flow_adds_spacing_between_runs_but_not_after_the_last_one() {
        // 샘플/다중 문단 셀 콘텐츠 높이's single-run table (0 coordinate
        // restarts) proved the reference `.htb` height equals the run's own
        // extent plus margins, with no extra trailing spacing tacked on
        // after the last line — there is no following line for that gap to
        // separate. Trailing spacing still belongs *between* two runs (it is
        // the gap the source reserved before the coordinate reset).
        let line = |top| LineSeg {
            top,
            height: 1400,
            text_height: 1400,
            spacing: 1120,
            ..LineSeg::default()
        };
        assert_eq!(line_flow_height(&[line(0), line(2520), line(5040)]), 6440);
        assert_eq!(
            line_flow_height(&[line(0), line(2520), line(0), line(2520)]),
            8960
        );
    }

    #[test]
    fn inline_table_is_not_split_as_a_page_fragment() {
        let table = Table {
            id: "inline".to_owned(),
            source_path: "section.xml".to_owned(),
            source_anchor: None,
            caption: None,
            section_index: 0,
            anchor_y: Some(1000),
            anchor_top_adjustment: 0,
            anchor_paragraph_left: 0,
            anchor_paragraph_right: 0,
            box_units: BoxUnits {
                width: 10000,
                height: 40000,
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
            rows: 2,
            row_heights: vec![20000, 20000],
            cells: Vec::new(),
            page_break: "CELL".to_owned(),
            repeat_header: true,
            no_adjust: false,
            fragment_rows: None,
        };
        let fragments = split_table(
            &table,
            &PageSpec::default(),
            table.box_units.height,
            false,
            None,
        );
        assert_eq!(fragments.len(), 1);
        assert_eq!(fragments[0].id, table.id);
    }

    #[test]
    fn utf16_offsets_do_not_split_surrogate_pairs() {
        let tokens = vec![Token {
            kind: TokenKind::Text("A😀B".to_owned()),
            logical_len: 4,
            char_style_id: 0,
            hyperlink: None,
        }];
        let (result, sources) = slice_tokens(&tokens, 1, 3);
        assert_eq!(result[0].visible_text(), "😀");
        assert_eq!(sources[0].logical, 1..3);
        assert_eq!(sources[0].utf8, Some(1..5));
    }

    #[test]
    fn provenance_records_expanded_scalars_and_combining_mark_bytes() {
        let tokens = vec![Token {
            kind: TokenKind::Text("A😀e\u{301}Z".into()),
            logical_len: 6,
            char_style_id: 7,
            hyperlink: Some("https://example.org/".into()),
        }];
        // Existing slicing keeps a whole scalar if only one surrogate was
        // requested. Record the actual source, not the request 2..3.
        let (part, source) = slice_tokens(&tokens, 2, 3);
        assert_eq!(part[0].visible_text(), "😀");
        assert_eq!(part[0].logical_len, 2);
        assert_eq!(source[0].logical, 1..3);
        assert_eq!(source[0].utf8, Some(1..5));
        assert_eq!(part[0].hyperlink, tokens[0].hyperlink);
        assert_eq!(part[0].char_style_id, 7);
        let (part, source) = slice_tokens(&tokens, 4, 5);
        assert_eq!(part[0].visible_text(), "\u{301}");
        assert_eq!(source[0].logical, 4..5);
        assert_eq!(source[0].utf8, Some(6..8));
    }

    #[test]
    fn provenance_keeps_control_slots_and_original_token_indices() {
        let tokens = vec![
            Token {
                kind: TokenKind::Control {
                    kind: "fieldBegin".into(),
                },
                logical_len: 8,
                char_style_id: 0,
                hyperlink: None,
            },
            Token {
                kind: TokenKind::Text("same".into()),
                logical_len: 4,
                char_style_id: 0,
                hyperlink: None,
            },
            Token {
                kind: TokenKind::Tab {
                    width: 1000,
                    leader: 0,
                },
                logical_len: 8,
                char_style_id: 0,
                hyperlink: None,
            },
            Token {
                kind: TokenKind::Text("same".into()),
                logical_len: 4,
                char_style_id: 0,
                hyperlink: None,
            },
        ];
        let (parts, source) = slice_tokens(&tokens, 10, 22);
        assert_eq!(
            parts.iter().map(Token::visible_text).collect::<String>(),
            "me\tsa"
        );
        assert_eq!(
            source,
            vec![
                TokenSourceRange {
                    token_index: 1,
                    logical: 10..12,
                    utf8: Some(2..4)
                },
                TokenSourceRange {
                    token_index: 2,
                    logical: 12..20,
                    utf8: None
                },
                TokenSourceRange {
                    token_index: 3,
                    logical: 20..22,
                    utf8: Some(0..2)
                },
            ]
        );
        // An empty interval at a proper token boundary does not invent a token.
        let (parts, source) = slice_tokens(&tokens, 12, 12);
        assert!(parts.is_empty());
        assert!(source.is_empty());
        let (parts, source) = slice_tokens(&tokens, 13, 15);
        assert_eq!(parts[0].logical_len, 2);
        assert_eq!(source[0].logical, 13..15);
        assert_eq!(source[0].utf8, None);
    }
}
