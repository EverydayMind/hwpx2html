//! A table's caption and the table in the area they share, for the direct
//! emitter. The page-by-page output does not write captions, and nothing
//! here changes what it writes.
//!
//! What is placed is what the corpus shows: a caption with text above its
//! table (`side="TOP"`, `fullSz="0"`) whose lines carry their source
//! positions. Six tables in two documents settle it against their sources
//! and reference HTML (성과보고서 s29 1151677650, 651, 658, 659, 660 and
//! 산업단지 유공자 공고 s0 1944792407; docs/history/
//! 2026-09-30-table-caption-root-cause.md):
//!
//! * The caption is as tall as its lines reach, `vertpos + max(vertsize,
//!   textheight)` at most, the last line's spacing left out. The lines of
//!   all its paragraphs share its origin (1151677660: two paragraphs at 100
//!   and 1700, 2500 tall, where the reference's caption is 8.82mm).
//! * It is `lastWidth` wide, the table's own width in all six; `width` says
//!   8504 in all six.
//! * The area is the table's outer margins around the caption, the gap and
//!   the table: `L + W + R` by `T + C + gap + H + B`. The caption stands at
//!   `(L, T)` and the table at `(L, T + C + gap)`. Nothing is added for zero
//!   margins, unlike a table set in a line without a caption.
//! * A line carrying such a table already reserves the whole area: in the
//!   five inline cases its stored height is the area's height exactly, so
//!   the caption is not added to it again.
//!
//! Anything else with text (a caption beside or below its table, `fullSz`,
//! no `lastWidth`, lines that restart or carry no position, tables or
//! objects in the caption) is refused, not guessed.
use super::{table_svg_frame, Frame, TablePlacement};
use crate::model::{Caption, HwpUnit, Table};

/// Where a table's caption and the table sit in their shared area, against
/// its corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCaptionPlan {
    /// The area's size: what a line carrying the table holds, or what a
    /// floating table covers from its origin.
    pub width: HwpUnit,
    pub height: HwpUnit,
    /// The caption's frame, which also clips its lines.
    pub caption: Frame,
    /// The table's corner, each coordinate one length as the reference
    /// writes it (`left:L`, `top:T+C+gap`).
    pub table_left: HwpUnit,
    pub table_top: HwpUnit,
}

/// Whether a caption holds anything to read: a letter, or content the
/// source generates or keeps apart (a number, ruby, overlapped letters).
pub fn has_text(caption: &Caption) -> bool {
    caption.paragraphs.iter().any(|paragraph| {
        !paragraph.inline_sources.is_empty()
            || paragraph
                .tokens
                .iter()
                .any(|token| !token.visible_text().trim().is_empty())
    })
}

/// The plan of a table's caption: `Ok(None)` for a table without a caption
/// with text (an empty caption stays in the tree and is not drawn), `Err`
/// naming what is not placed yet.
pub fn table_caption_plan(table: &Table) -> Result<Option<TableCaptionPlan>, &'static str> {
    let Some(caption) = table.caption.as_deref().filter(|caption| has_text(caption)) else {
        return Ok(None);
    };
    if !caption.side.eq_ignore_ascii_case("TOP") {
        return Err("a caption beside or below its table");
    }
    if caption.full_size {
        return Err("a caption spanning its table's margins");
    }
    let Some(width) = caption.last_width.filter(|width| *width > 0) else {
        return Err("a caption without its width");
    };
    if caption.unparsed_tables > 0
        || caption
            .paragraphs
            .iter()
            .any(|paragraph| paragraph.has_inline_table || !paragraph.objects.is_empty())
    {
        return Err("a caption holding a table or object");
    }
    let mut height: HwpUnit = 0;
    let mut previous = None;
    for paragraph in &caption.paragraphs {
        if paragraph.lines.is_empty() {
            return Err("a caption paragraph without its line positions");
        }
        for line in &paragraph.lines {
            if previous.is_some_and(|top| line.top <= top) {
                return Err("a caption whose lines restart");
            }
            previous = Some(line.top);
            height = height.max(line.top.saturating_add(line.height.max(line.text_height)));
        }
    }
    let (left, top) = (table.out_margin_left.max(0), table.out_margin_top.max(0));
    let table_top = top + height + caption.gap.max(0);
    Ok(Some(TableCaptionPlan {
        width: left + table.box_units.width + table.out_margin_right.max(0),
        height: table_top + table.box_units.height + table.out_margin_bottom.max(0),
        caption: Frame {
            left,
            top,
            width,
            height,
        },
        table_left: left,
        table_top,
    }))
}

/// The table's border drawing in the area: its own box, with no frame
/// around it (the area's margins are the frame).
pub fn table_svg(table: &Table) -> Frame {
    table_svg_frame(
        table,
        TablePlacement::Page {
            left: 0,
            top: 0,
            frame_adjustment: false,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Anchor, BoxUnits, LineSeg, ParaStyle, Paragraph, Token, TokenKind};

    fn paragraph(text: &str, lines: &[(HwpUnit, HwpUnit, HwpUnit)]) -> Paragraph {
        Paragraph {
            id: String::new(),
            key: String::new(),
            source_path: String::new(),
            section_index: 0,
            para_style_id: 0,
            source_para_style_id: 0,
            para_style: ParaStyle::default(),
            line_top_offset: 0,
            tokens: vec![Token {
                kind: TokenKind::Text(text.to_owned()),
                logical_len: text.chars().count(),
                char_style_id: 0,
                hyperlink: None,
            }],
            source_tokens: Vec::new(),
            lines: lines
                .iter()
                .map(|&(top, height, text_height)| LineSeg {
                    top,
                    height,
                    text_height,
                    spacing: height * 6 / 10,
                    ..LineSeg::default()
                })
                .collect(),
            objects: Vec::new(),
            inline_sources: Vec::new(),
            page_break: false,
            has_inline_table: false,
            page_number_restart: None,
            page_number_control: false,
            hides_page_number: false,
            columns: None,
        }
    }

    fn table(size: (HwpUnit, HwpUnit), margin: HwpUnit, caption: Caption) -> Table {
        Table {
            id: "t".to_owned(),
            source_path: String::new(),
            source_anchor: None,
            caption: Some(Box::new(caption)),
            section_index: 0,
            anchor_y: None,
            anchor_top_adjustment: 0,
            anchor_paragraph_left: 0,
            anchor_paragraph_right: 0,
            box_units: BoxUnits {
                width: size.0,
                height: size.1,
                ..BoxUnits::default()
            },
            anchor: Anchor::default(),
            out_margin_left: margin,
            out_margin_right: margin,
            out_margin_top: margin,
            out_margin_bottom: margin,
            columns: 1,
            rows: 1,
            row_heights: Vec::new(),
            cells: Vec::new(),
            page_break: "CELL".to_owned(),
            repeat_header: false,
            no_adjust: false,
            fragment_rows: None,
        }
    }

    fn caption(gap: HwpUnit, last_width: HwpUnit, paragraphs: Vec<Paragraph>) -> Caption {
        Caption {
            side: "TOP".to_owned(),
            gap,
            width: 8504,
            full_size: false,
            last_width: Some(last_width),
            paragraphs,
            unparsed_tables: 0,
        }
    }

    #[test]
    fn two_paragraphs_share_the_caption_origin_and_the_last_spacing_is_left_out() {
        // 성과보고서 s29 1151677660: the anchor line stores 11908.
        let table = table(
            (45295, 9125),
            0,
            caption(
                283,
                45295,
                vec![
                    paragraph("<최근 3개년 사업화 매출액 성과>", &[(100, 1000, 1000)]),
                    paragraph("(단위 : 건, 억원)", &[(1700, 800, 800)]),
                ],
            ),
        );
        let plan = table_caption_plan(&table).unwrap().unwrap();
        assert_eq!(
            plan,
            TableCaptionPlan {
                width: 45295,
                height: 11908,
                caption: Frame {
                    left: 0,
                    top: 0,
                    width: 45295,
                    height: 2500,
                },
                table_left: 0,
                table_top: 2783,
            }
        );
    }

    #[test]
    fn the_outer_margins_frame_the_caption_and_the_table() {
        // 성과보고서 s29 1151677658 (margins 141, anchor line 8192) and
        // 유공자 공고 s0 1944792407 (margins 283, anchor line 8938).
        let narrow = table(
            (44824, 6060),
            141,
            caption(
                850,
                44824,
                vec![paragraph("< 논문 성과 >", &[(0, 1000, 1000)])],
            ),
        );
        let plan = table_caption_plan(&narrow).unwrap().unwrap();
        assert_eq!((plan.width, plan.height), (45106, 8192));
        assert_eq!((plan.table_left, plan.table_top), (141, 1991));
        assert_eq!(
            plan.caption,
            Frame {
                left: 141,
                top: 141,
                width: 44824,
                height: 1000,
            }
        );
        let wide = table(
            (47901, 6222),
            283,
            caption(
                850,
                47901,
                vec![paragraph("< 심사 주요 절차도 >", &[(0, 1300, 1300)])],
            ),
        );
        let plan = table_caption_plan(&wide).unwrap().unwrap();
        assert_eq!((plan.width, plan.height), (48467, 8938));
        assert_eq!((plan.table_left, plan.table_top), (283, 2433));
    }

    #[test]
    fn an_empty_caption_is_not_placed() {
        let table = table(
            (1000, 1000),
            0,
            caption(850, 1000, vec![paragraph(" ", &[(0, 1000, 1000)])]),
        );
        assert_eq!(table_caption_plan(&table), Ok(None));
    }

    #[test]
    fn what_the_corpus_does_not_show_is_refused() {
        let text = || vec![paragraph("< 표 >", &[(0, 1000, 1000)])];
        let refused = |change: &dyn Fn(&mut Caption)| {
            let mut caption = caption(850, 1000, text());
            change(&mut caption);
            table_caption_plan(&table((1000, 1000), 0, caption)).unwrap_err()
        };
        assert_eq!(
            refused(&|caption| caption.side = "BOTTOM".to_owned()),
            "a caption beside or below its table"
        );
        assert_eq!(
            refused(&|caption| caption.full_size = true),
            "a caption spanning its table's margins"
        );
        assert_eq!(
            refused(&|caption| caption.last_width = None),
            "a caption without its width"
        );
        assert_eq!(
            refused(&|caption| caption.unparsed_tables = 1),
            "a caption holding a table or object"
        );
        assert_eq!(
            refused(&|caption| caption.paragraphs[0].lines.clear()),
            "a caption paragraph without its line positions"
        );
        assert_eq!(
            refused(&|caption| caption
                .paragraphs
                .push(paragraph("둘째", &[(0, 1000, 1000)]))),
            "a caption whose lines restart"
        );
    }
}
