use std::collections::BTreeSet;

use crate::error::{ConvertError, Result};
use crate::hwpx::header::HeaderStyles;
use crate::hwpx::util::{
    attr, attr_bool, attr_coord, attr_i64, attr_string, attr_u32, attr_usize, child, children,
    descendants, local_name, parse_xml, text_content,
};
use crate::model::{
    Anchor, Block, BoxUnits, Caption, EquationSource, InlineContent, InlineSource, LineSeg,
    PageSpec, Paragraph, PositionedObject, Section, SourceAnchor, SourceLogicalRange, SourceToken,
    Table, TableCell, Token, TokenKind,
};

const KNOWN_NAMESPACES: &[&str] = &[
    "http://www.hancom.co.kr/hwpml/2011/app",
    "http://www.hancom.co.kr/hwpml/2011/paragraph",
    "http://www.hancom.co.kr/hwpml/2016/paragraph",
    "http://www.hancom.co.kr/hwpml/2011/section",
    "http://www.hancom.co.kr/hwpml/2011/core",
    "http://www.hancom.co.kr/hwpml/2011/head",
    "http://www.hancom.co.kr/hwpml/2011/history",
    "http://www.hancom.co.kr/hwpml/2011/master-page",
    "http://www.hancom.co.kr/schema/2011/hpf",
    "http://www.idpf.org/2007/opf/",
    "http://www.hancom.co.kr/hwpml/2016/ooxmlchart",
    "http://www.idpf.org/2007/ops",
    "urn:oasis:names:tc:opendocument:xmlns:config:1.0",
];

#[derive(Debug, Clone, Default)]
pub struct SectionParseResult {
    pub section: Section,
    pub has_page_number_control: bool,
    pub page_number_start: Option<i64>,
    pub page_number_format: Option<String>,
    pub page_number_side: Option<String>,
}

/// Hancom's HTML export gives no CSS class at all to a character style whose
/// only use in the whole document is a run that repeats the immediately
/// preceding run's `<hp:pageNum>` control byte-for-byte -- a redundant,
/// no-op duplicate insertion, distinct from an ordinary unused or
/// otherwise-textless style (both of which still get a class). Every
/// subsequent style's number then shifts down by one to fill the gap.
/// Detecting exactly this shape (and requiring the candidate id appear
/// nowhere else) keeps the fix from firing on the far more common case of a
/// single, meaningful `<hp:pageNum>` insertion. See AGENTS.md's style-shift
/// entries for the hypotheses this narrowed down from.
pub fn collect_redundant_page_number_styles(sections: &[(String, &[u8])]) -> Result<BTreeSet<u32>> {
    let mut usage_counts: std::collections::BTreeMap<u32, usize> =
        std::collections::BTreeMap::new();
    let mut candidates = Vec::new();
    for (path, bytes) in sections {
        let document = parse_xml(bytes, path)?;
        let root = document.root_element();
        for run in descendants(root, "run") {
            *usage_counts
                .entry(attr_u32(run, "charPrIDRef", 0))
                .or_insert(0) += 1;
        }
        for paragraph in descendants(root, "p") {
            let runs = children(paragraph, "run").collect::<Vec<_>>();
            for pair in runs.windows(2) {
                if let Some(id) = redundant_page_number_run_id(pair[0], pair[1]) {
                    candidates.push(id);
                }
            }
        }
    }
    Ok(candidates
        .into_iter()
        .filter(|id| usage_counts.get(id).copied() == Some(1))
        .collect())
}

fn redundant_page_number_run_id(
    first: roxmltree::Node<'_, '_>,
    second: roxmltree::Node<'_, '_>,
) -> Option<u32> {
    let first_page_num = sole_page_num_control(first)?;
    let second_page_num = sole_page_num_control(second)?;
    let identical = ["pos", "formatType", "sideChar"]
        .iter()
        .all(|name| attr_string(first_page_num, name) == attr_string(second_page_num, name));
    identical.then(|| attr_u32(second, "charPrIDRef", 0))
}

/// A run whose only element child is a `ctrl` whose only element child is a
/// `pageNum` (no text anywhere in the run).
fn sole_page_num_control<'a, 'input>(
    run: roxmltree::Node<'a, 'input>,
) -> Option<roxmltree::Node<'a, 'input>> {
    let mut run_children = run.children().filter(|node| node.is_element());
    let ctrl = run_children.next()?;
    if run_children.next().is_some() || local_name(ctrl) != "ctrl" {
        return None;
    }
    let mut ctrl_children = ctrl.children().filter(|node| node.is_element());
    let page_num = ctrl_children.next()?;
    if ctrl_children.next().is_some() || local_name(page_num) != "pageNum" {
        return None;
    }
    Some(page_num)
}

/// Hancom's HTML export can give a paraPr no CSS class of its own -- reusing
/// an earlier paraPr's class instead -- when it is byte-identical to that
/// earlier paraPr (every attribute and nested element, ignoring only the
/// `id` attribute) AND every paragraph that references it holds nothing but
/// a picture (no text run). A byte-identical paraPr used by ordinary text
/// paragraphs, or one that also has unused/empty-paragraph uses, keeps its
/// own class -- most HWPX documents carry many exact-duplicate paraPr pairs
/// (typically leftover edit history) and the reference exporter gives each
/// its own class regardless, so testing content equality alone would merge
/// far too much (this is the paraPr analogue of the mistake noted in
/// AGENTS.md's char-style-shift entries, which broke the direct gate from
/// 5/22 to 0/22). A full 23-sample scan of this exact combined condition
/// (identical content AND exclusively picture-only usage) matches only one
/// document's one pair; see AGENTS.md for that evidence.
pub fn collect_duplicate_picture_only_para_styles(
    header_bytes: &[u8],
    sections: &[(String, &[u8])],
) -> Result<std::collections::BTreeMap<u32, u32>> {
    let header = parse_xml(header_bytes, "Contents/header.xml")?;
    let mut para_prs = descendants(header.root_element(), "paraPr")
        .map(|node| (attr_u32(node, "id", 0), xml_signature(node, true)))
        .collect::<Vec<_>>();
    para_prs.sort_by_key(|(id, _)| *id);

    let mut aliases = std::collections::BTreeMap::new();
    for (index, (id, signature)) in para_prs.iter().enumerate() {
        if aliases.contains_key(id) {
            continue;
        }
        for (later_id, later_signature) in &para_prs[index + 1..] {
            if signature == later_signature {
                aliases.entry(*later_id).or_insert(*id);
            }
        }
    }

    let mut usage: std::collections::BTreeMap<u32, Vec<(bool, bool)>> =
        std::collections::BTreeMap::new();
    for (path, bytes) in sections {
        let document = parse_xml(bytes, path)?;
        for paragraph in descendants(document.root_element(), "p") {
            let id = attr_u32(paragraph, "paraPrIDRef", 0);
            // An empty trailing run's self-closing <hp:t/> is a structural
            // placeholder, not visible text -- a picture-only paragraph
            // still carries one alongside its <hp:pic> run.
            let has_text = descendants(paragraph, "t").any(|node| !text_content(node).is_empty());
            let has_pic = descendants(paragraph, "pic").next().is_some();
            usage.entry(id).or_default().push((has_text, has_pic));
        }
    }

    Ok(aliases
        .into_iter()
        .filter(|(later_id, _)| {
            usage.get(later_id).is_some_and(|uses| {
                !uses.is_empty() && uses.iter().all(|&(has_text, has_pic)| !has_text && has_pic)
            })
        })
        .collect())
}

/// The full subtree signature of an XML element: its own tag and sorted
/// attributes (minus `id` at the top level only), followed recursively by
/// every child element's signature in document order. Two nodes with the
/// same signature are byte-identical for every field a reader could
/// observe, regardless of nesting depth -- unlike comparing only immediate
/// attributes, this also catches differences buried inside e.g. paraPr's
/// `hp:switch/hp:case/hh:margin` structure.
fn xml_signature(node: roxmltree::Node<'_, '_>, exclude_id: bool) -> String {
    let mut signature = local_name(node).to_owned();
    let mut attrs = node
        .attributes()
        .filter(|attribute| !(exclude_id && attribute.name() == "id"))
        .map(|attribute| format!("{}={}", attribute.name(), attribute.value()))
        .collect::<Vec<_>>();
    attrs.sort();
    for attribute in attrs {
        signature.push(':');
        signature.push_str(&attribute);
    }
    signature.push('[');
    for child in node.children().filter(|node| node.is_element()) {
        signature.push_str(&xml_signature(child, false));
        signature.push(';');
    }
    signature.push(']');
    signature
}

pub fn parse_section(
    bytes: &[u8],
    index: usize,
    source_path: &str,
    styles: &HeaderStyles,
) -> Result<SectionParseResult> {
    let xml = parse_xml(bytes, source_path)?;
    let root = xml.root_element();
    validate_namespace_profile(root)?;
    let page = parse_page_spec(root, styles);
    let has_page_number_control = descendants(root, "pageNum").next().is_some();
    let mut blocks = Vec::new();
    let mut result = SectionParseResult {
        section: Section {
            index,
            source_path: source_path.to_owned(),
            page: PageSpec {
                page_number_enabled: has_page_number_control,
                ..page
            },
            blocks: Vec::new(),
        },
        ..SectionParseResult::default()
    };
    let mut ordinal = 0usize;
    // The nearest preceding top-level paragraph's own last line (its raw
    // `vertpos` plus that line's own `vertsize`+`spacing` advance) -- see
    // its one use below, computing a trivial anchor paragraph's synthetic
    // pre-float line for a `flowWithText="0"` table.
    let mut previous_top_level_line: Option<(i64, i64)> = None;
    // Where the previous top-level paragraph's flow ended: its last line's
    // advance or the bottom of its own top/bottom floats, whichever is lower.
    let mut previous_flow_end: Option<i64> = None;
    for node in root.children().filter(|node| node.is_element()) {
        match local_name(node) {
            "p" => {
                let id = element_id(index, "p", ordinal, node);
                let (mut paragraph, table_slots) =
                    parse_paragraph_with_tables(node, index, id, source_path, styles, &mut result)?;
                assign_paragraph_keys(&mut paragraph, format!("s{index}/p[{ordinal}]"));
                let paragraph_key = paragraph.key.clone();
                let anchor_y = paragraph_anchor_y(&paragraph);
                let paragraph_lines = paragraph.lines.clone();
                let paragraph_line_top_offset = paragraph.line_top_offset;
                let paragraph_has_inline_object = paragraph
                    .objects
                    .iter()
                    .any(|object| object.anchor.treat_as_char);
                // Only a paragraph that carries nothing but the float lets
                // the float ignore the line's glyph-box adjustment; where the
                // paragraph has its own text, the float lines up with it.
                let paragraph_carries_no_visible_text = paragraph
                    .tokens
                    .iter()
                    .all(|token| token.visible_text().trim().is_empty());
                let paragraph_has_leftover_space_only = {
                    let text: String = paragraph.tokens.iter().map(Token::visible_text).collect();
                    !text.is_empty() && text.trim().is_empty()
                };
                let paragraph_has_field_end = paragraph.tokens.iter().any(|token| {
                    matches!(&token.kind, TokenKind::Control { kind } if kind == "fieldEnd")
                });
                let anchor_top_adjustment = if paragraph_carries_no_visible_text {
                    paragraph_top_adjustment(&paragraph)
                } else {
                    0
                };
                // A trivial anchor paragraph's own recorded line -- see the
                // stacking comment just below -- is the *end* of whatever it
                // hosts, not a usable flow position for one more table this
                // same paragraph carries. `prior_top_level_line` is this
                // paragraph's own natural, pre-float continuation instead:
                // the previous real paragraph's line plus its advance.
                let prior_top_level_line = previous_top_level_line;
                let prior_flow_end = previous_flow_end.take();
                previous_top_level_line = paragraph
                    .lines
                    .last()
                    .map(|line| (line.top, line.height.saturating_add(line.spacing)));
                // Tables are commonly carried by the anchor paragraph's run
                // (rather than as a direct child of hs:sec). Preserve them as
                // block objects so their cells are not mistaken for ordinary
                // paragraph content.
                let table_nodes =
                    node.descendants()
                        .filter(|candidate| {
                            candidate.is_element()
                                && local_name(*candidate) == "tbl"
                                && !candidate.ancestors().skip(1).any(|ancestor| {
                                    matches!(local_name(ancestor), "tbl" | "drawText")
                                })
                        })
                        .collect::<Vec<_>>();
                // A TOP_AND_BOTTOM, page-independent float with a zero PARA
                // offset can be recorded in the first line of a paragraph
                // whose visible text actually starts on the next page.  HWP
                // lays out that float in the preceding page's remaining
                // space, then emits the paragraph text on the next page.
                // Keep this exceptional source order in the IR; all other
                // anchored tables retain the normal paragraph-then-table
                // order used by the layout rules below.
                let table_precedes_anchor_paragraph = table_nodes.len() == 1
                    && paragraph_lines.first().is_some_and(|line| line.top == 0)
                    && paragraph
                        .tokens
                        .iter()
                        .any(|token| !token.visible_text().trim().is_empty())
                    && table_nodes[0].attribute("textWrap") == Some("TOP_AND_BOTTOM")
                    && child(table_nodes[0], "pos").is_some_and(|position| {
                        position.attribute("treatAsChar") == Some("0")
                            && position.attribute("flowWithText") != Some("1")
                            && position.attribute("vertRelTo") == Some("PARA")
                            && attr_i64(position, "vertOffset", 0) == 0
                    });
                // Multiple non-overlapping top/bottom floats occupy the space
                // before their shared paragraph line. That line's vertpos is
                // the end of the object stack, not every object's origin.
                // A shared, nonzero `vertOffset` is a different shape though
                // -- 샘플/다수 부동표 has two runs of 4 identical-vertOffset
                // (33247 raw, not 0) TOP_AND_BOTTOM floats that the reference
                // walks apart *forward* from that one shared offset (the
                // first sits exactly where a lone float at that offset
                // would, later ones pushed down by the earlier ones' own
                // heights), not backward from the anchor paragraph's line.
                // Only a *run* of consecutive tables sharing one exact
                // offset stacks; a table with a lone or differing offset
                // (like this same paragraph's other, IN_FRONT_OF_TEXT
                // floats, each at its own distinct vertOffset) keeps using
                // its raw anchor_y placement untouched.
                let stack_offset = |table: roxmltree::Node<'_, '_>| -> Option<i64> {
                    if table.attribute("textWrap") != Some("TOP_AND_BOTTOM") {
                        return None;
                    }
                    let position = child(table, "pos")?;
                    if position.attribute("treatAsChar") != Some("0")
                        || position.attribute("allowOverlap") != Some("0")
                        || position.attribute("vertRelTo") != Some("PARA")
                    {
                        return None;
                    }
                    Some(attr_i64(position, "vertOffset", 0))
                };
                let extent = |table: roxmltree::Node<'_, '_>| {
                    child(table, "sz").map_or(0, |size| attr_i64(size, "height", 0))
                        + child(table, "outMargin").map_or(0, |margin| {
                            attr_i64(margin, "top", 0) + attr_i64(margin, "bottom", 0)
                        })
                };
                // Whether this table qualifies for `stack_offset`'s own
                // "end of the object stack" phenomenon at all -- the exact
                // shape check above, minus reading the offset itself.
                // `flowWithText` plays no part in it: a lone
                // `textWrap="TOP_AND_BOTTOM"` float's hosting paragraph still
                // records the position *after* the float regardless of
                // whether later content re-flows around it, and
                // `flowWithText="1"`'s own lone (unstacked) case is exactly
                // as affected as `flowWithText="0"`'s -- 성과보고서's
                // "성과지표 달성 현황" table (`flowWithText="1"`) sits one
                // whole table-height (82.37mm) too low the same way
                // "프로그램 예산집행 현황" (`flowWithText="0"`) did. See
                // `is_top_and_bottom_para_anchor`'s one use below.
                let is_top_and_bottom_para_anchor =
                    |table: roxmltree::Node<'_, '_>| -> bool { stack_offset(table).is_some() };
                // A paragraph's top/bottom floats measure from its flow start,
                // not from its recorded line: when that line sits lower, the
                // paragraph's own floats pushed it there. A section's first
                // paragraph starts at the fresh page's top (성과보고서 sections
                // 37-42 and 64-65); a paragraph carrying several floats starts
                // where the previous paragraph's flow ended (성과보고서 s96's
                // eight tables, 2548 = 948 + 1600, not its line at 7764).
                // rhwp likewise places such a float at the paragraph's flow
                // start plus its own vertOffset. Single-table paragraphs keep
                // their separately verified rules below.
                // Declared offsets must describe the stack: s4's caption
                // (vertOffset -5219, stored unsigned as 4294962077) and data
                // table are instead stacked by height from their recorded line
                // (`compact_float_stack_by_height`).
                let offsets_describe_stack = table_nodes.iter().all(|table| {
                    stack_offset(*table)
                        .is_none_or(|offset| (0..=i64::from(i32::MAX)).contains(&offset))
                });
                let flow_start = match prior_flow_end {
                    None if ordinal == 0 && prior_top_level_line.is_none() => Some(0),
                    Some(end) if table_nodes.len() > 1 && offsets_describe_stack => Some(end),
                    _ => None,
                };
                let pushed_flow_start = paragraph_carries_no_visible_text
                    .then(|| paragraph_lines.first().zip(flow_start))
                    .flatten()
                    .filter(|(line, start)| line.top > *start);
                let section_opening_origin = pushed_flow_start.map(|(line, start)| {
                    line_anchor_y(
                        paragraph_line_top_offset,
                        &LineSeg {
                            top: start,
                            ..line.clone()
                        },
                    )
                });
                let float_origin = pushed_flow_start
                    .map(|(_, start)| start)
                    .or_else(|| paragraph_lines.first().map(|line| line.top));
                // The paragraph's floating shapes measure from its flow start
                // too; layout finds it (`flow_end` in `layout_document`).
                previous_flow_end = paragraph.lines.last().map(|line| {
                    let line_end = line
                        .top
                        .saturating_add(line.height)
                        .saturating_add(line.spacing);
                    table_nodes
                        .iter()
                        .filter_map(|table| {
                            Some(
                                float_origin?
                                    .saturating_add(stack_offset(*table)?)
                                    .saturating_add(extent(*table)),
                            )
                        })
                        .fold(line_end, i64::max)
                });
                let mut table_anchor_ys = table_nodes
                    .iter()
                    .map(|table_node| {
                        let line_index =
                            table_line_index(node, *table_node, &table_slots, &paragraph_lines);
                        // Line 0's own anchor still goes through the existing
                        // glyph-box subtraction (`line_anchor_y`, matching
                        // `paragraph_anchor_y`) -- render_line's line-fragment
                        // builder applies that same reduction when it derives
                        // a *first* line's own top from `vertpos`. A later
                        // line, reserved for nothing but this table (no glyph
                        // of its own to make room for), keeps that fragment's
                        // top at its raw `vertpos` instead: 성과보고서's "전략
                        // 목표 Ⅰ" title table's own second line renders at
                        // `top:14.63mm`, its plain unadjusted vertpos (4147
                        // HWPUNIT), not the glyph-reduced 3954 the first-line
                        // formula would give it.
                        let computed = if let Some(origin) = section_opening_origin.filter(|_| {
                            line_index == 0 && is_top_and_bottom_para_anchor(*table_node)
                        }) {
                            Some(origin)
                        } else if line_index == 0 {
                            // A textless paragraph carrying a
                            // `textWrap="TOP_AND_BOTTOM"`
                            // float has its *own* recorded line positioned
                            // wherever that float (and, through it, however
                            // many pages the float itself later spans) ends,
                            // not where the float starts -- see the "end of
                            // the object stack" comment above for the same
                            // phenomenon in the run-of-floats case. Unlike
                            // that stacked case, a lone qualifying float has
                            // no earlier sibling table to walk backward from,
                            // so use the nearest preceding real paragraph's
                            // own line plus its advance as this table's
                            // synthetic pre-float line instead -- 성과보고서's
                            // "프로그램 예산집행 현황" table (`flowWithText="0"`,
                            // 25 occurrences, one per program section)
                            // otherwise sits right at the page's content top,
                            // burying its own "□ 주요 사업 및 집행현황"
                            // heading underneath it, and its sibling
                            // "성과지표 달성 현황" table (`flowWithText="1"`)
                            // sits one whole table-height too low, burying
                            // the paragraph that follows it. A genuinely
                            // anchor paragraph whose line has not advanced
                            // past the preceding paragraph keeps using its
                            // own line untouched -- its `vertpos` is not a
                            // computed post-float position. And a paragraph
                            // hosting *more than one* table keeps every one
                            // of them untouched too, lone-table or not:
                            // `compact_float_stack`/`compact_float_stack_by_height`
                            // (layout.rs) already own that shape and need
                            // this same raw, unmodified anchor_y as the
                            // group's shared reference point to stack
                            // back from -- s4's caption+data table pair
                            // (differing vertOffsets, one blank paragraph
                            // with the same kind of leftover space)
                            // regressed when this ran per-table regardless
                            // of how many tables its paragraph carried.
                            if table_nodes.len() == 1
                                && is_top_and_bottom_para_anchor(*table_node)
                                && (paragraph_has_leftover_space_only
                                    || (paragraph_carries_no_visible_text
                                        && paragraph_has_field_end)
                                    || table_precedes_anchor_paragraph)
                            {
                                prior_top_level_line
                                    .map(|(top, advance)| {
                                        let synthetic = LineSeg {
                                            top: top.saturating_add(advance),
                                            ..paragraph_lines.first().cloned().unwrap_or_default()
                                        };
                                        line_anchor_y(paragraph_line_top_offset, &synthetic)
                                    })
                                    .or_else(|| {
                                        paragraph_lines.first().map(|line| {
                                            line_anchor_y(paragraph_line_top_offset, line)
                                        })
                                    })
                            } else {
                                paragraph_lines
                                    .first()
                                    .map(|line| line_anchor_y(paragraph_line_top_offset, line))
                            }
                        } else {
                            paragraph_lines.get(line_index).map(|line| {
                                later_line_anchor_y(
                                    paragraph_has_inline_object,
                                    paragraph_line_top_offset,
                                    line,
                                )
                            })
                        };
                        computed.or(anchor_y)
                    })
                    .collect::<Vec<_>>();
                let mut run_start = 0;
                while run_start < table_nodes.len() {
                    let key = stack_offset(table_nodes[run_start]);
                    let mut run_end = run_start + 1;
                    if key.is_some() {
                        while run_end < table_nodes.len()
                            && stack_offset(table_nodes[run_end]) == key
                        {
                            run_end += 1;
                        }
                    }
                    if let Some(shared_offset) = key {
                        if run_end - run_start > 1 {
                            let run = &table_nodes[run_start..run_end];
                            let mut stack_y = if shared_offset == 0 {
                                anchor_y.unwrap_or(0)
                                    - run.iter().map(|table| extent(*table)).sum::<i64>()
                            } else {
                                anchor_y.unwrap_or(0)
                            };
                            for (offset, table) in run.iter().enumerate() {
                                table_anchor_ys[run_start + offset] = Some(stack_y);
                                stack_y += extent(*table);
                            }
                        }
                    }
                    run_start = run_end;
                }
                if !table_precedes_anchor_paragraph {
                    blocks.push(Block::Paragraph(paragraph.clone()));
                }
                for (table_ordinal, table_node) in table_nodes.into_iter().enumerate() {
                    let table_anchor_y = table_anchor_ys[table_ordinal];
                    let table_id = format!("s{index}/tbl-anchor[{ordinal}-{table_ordinal}]");
                    let mut table = parse_table(
                        table_node,
                        index,
                        table_id,
                        source_path,
                        table_anchor_y,
                        styles,
                        &mut result,
                    )?;
                    table.anchor_top_adjustment = anchor_top_adjustment;
                    table.source_anchor = table_anchor(table_node, &paragraph_key, &table_slots);
                    let anchor_style = styles.para_style(styles.resolve_para_style_id(attr_u32(
                        node,
                        "paraPrIDRef",
                        0,
                    )));
                    table.anchor_paragraph_left = anchor_style.margin_left;
                    table.anchor_paragraph_right = anchor_style.margin_right;
                    blocks.push(Block::Table(table));
                }
                if table_precedes_anchor_paragraph {
                    blocks.push(Block::Paragraph(paragraph));
                }
                ordinal += 1;
            }
            "tbl" => {
                let id = element_id(index, "tbl", ordinal, node);
                blocks.push(Block::Table(parse_table(
                    node,
                    index,
                    id,
                    source_path,
                    None,
                    styles,
                    &mut result,
                )?));
                ordinal += 1;
            }
            name if is_supported_object_name(name) => {
                let id = element_id(index, name, ordinal, node);
                blocks.push(Block::Object(parse_positioned_object(
                    node,
                    index,
                    source_path,
                    id,
                    name,
                    styles,
                    &mut result,
                )?));
                ordinal += 1;
            }
            _ => {}
        }
    }
    result.section.blocks = blocks;
    Ok(result)
}

fn validate_namespace_profile(root: roxmltree::Node<'_, '_>) -> Result<()> {
    if let Some(namespace) = root.tag_name().namespace() {
        if !KNOWN_NAMESPACES.contains(&namespace) {
            return Err(ConvertError::UnsupportedSchema(format!(
                "unknown section root namespace {namespace}"
            )));
        }
    }
    let structural: BTreeSet<&str> = [
        "p",
        "run",
        "t",
        "tbl",
        "tr",
        "tc",
        "subList",
        "linesegarray",
        "lineseg",
        "pic",
        "pos",
        "sz",
    ]
    .into_iter()
    .collect();
    for node in root.descendants().filter(|node| node.is_element()) {
        if structural.contains(local_name(node)) {
            if let Some(namespace) = node.tag_name().namespace() {
                if !KNOWN_NAMESPACES.contains(&namespace) {
                    return Err(ConvertError::UnsupportedSchema(format!(
                        "unknown namespace {namespace} at {}",
                        local_name(node)
                    )));
                }
            }
        }
    }
    Ok(())
}

fn parse_page_spec(root: roxmltree::Node<'_, '_>, styles: &HeaderStyles) -> PageSpec {
    let sec_pr = descendants(root, "secPr").next();
    let page_pr = sec_pr.and_then(|node| child(node, "pagePr"));
    let margin = page_pr.and_then(|node| child(node, "margin"));
    let declared_width = page_pr
        .map(|node| attr_i64(node, "width", 59528))
        .unwrap_or(59528);
    let declared_height = page_pr
        .map(|node| attr_i64(node, "height", 84188))
        .unwrap_or(84188);
    // OWPML stores portrait paper dimensions and marks orientation
    // separately: WIDELY is portrait, NARROWLY is landscape. 성과보고서's 83
    // NARROWLY pages are 297x210mm in the reference, with the margins kept on
    // their declared sides (rhwp's `PageDef::landscape` swaps the same way).
    let landscape = page_pr
        .and_then(|node| node.attribute("landscape"))
        .is_some_and(|value| value.eq_ignore_ascii_case("NARROWLY"));
    let (width, height) = if landscape {
        (declared_height, declared_width)
    } else {
        (declared_width, declared_height)
    };
    PageSpec {
        width,
        height,
        margin_left: margin.map(|node| attr_i64(node, "left", 0)).unwrap_or(0),
        margin_right: margin.map(|node| attr_i64(node, "right", 0)).unwrap_or(0),
        margin_top: margin.map(|node| attr_i64(node, "top", 0)).unwrap_or(0),
        margin_bottom: margin.map(|node| attr_i64(node, "bottom", 0)).unwrap_or(0),
        header: margin.map(|node| attr_i64(node, "header", 0)).unwrap_or(0),
        footer: margin.map(|node| attr_i64(node, "footer", 0)).unwrap_or(0),
        hide_first_page_number: sec_pr
            .and_then(|node| child(node, "visibility"))
            .map(|node| attr_bool(node, "hideFirstPageNum"))
            .unwrap_or(false),
        page_number_enabled: false,
        page_number_char_style_id: page_number_char_style_id(root, styles),
    }
}

fn page_number_char_style_id(root: roxmltree::Node<'_, '_>, styles: &HeaderStyles) -> u32 {
    if let Some(id) = styles.page_number_style_char_id {
        if styles.char_styles.contains_key(&id) {
            return id;
        }
    }
    let source_style_id = descendants(root, "pageNum")
        .next()
        .and_then(|page_num| page_num.ancestors().find(|node| local_name(*node) == "run"))
        .map(|run| styles.resolve_char_style_id(attr_u32(run, "charPrIDRef", 0)));
    let mut candidates = styles
        .char_styles
        .values()
        .filter(|style| {
            style.font_size_hwp == 1000
                && !style.bold
                && !style.italic
                && !style.underline
                && !style.strike
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|style| {
        let is_source = source_style_id == Some(style.id);
        let is_gulim = style.font_family == "굴림";
        // The pageNum run carries the exporter-selected character style.
        // Prefer it over a generic fallback: the same document family uses
        // different page-number widths (and sometimes different faces) for
        // otherwise identical 10pt page numbers.
        (!is_source, !is_gulim, style.id)
    });
    candidates.first().map(|style| style.id).unwrap_or(0)
}

#[cfg(test)]
fn parse_paragraph(
    node: roxmltree::Node<'_, '_>,
    section_index: usize,
    id: String,
    source_path: &str,
    styles: &HeaderStyles,
    result: &mut SectionParseResult,
) -> Result<Paragraph> {
    parse_paragraph_with_tables(node, section_index, id, source_path, styles, result)
        .map(|(paragraph, _)| paragraph)
}

/// A run's `hp:tbl` node and the logical position of its slot.
type TableSlots = Vec<(roxmltree::NodeId, usize)>;

#[derive(Clone, Copy)]
struct SourceRun {
    run: usize,
    item: usize,
    raw_style: u32,
    style: u32,
}

struct SourceTracker {
    position: Option<usize>,
    tokens: Vec<SourceToken>,
}

impl SourceTracker {
    fn new() -> Self {
        Self {
            position: Some(0),
            tokens: Vec::new(),
        }
    }

    fn record(
        &mut self,
        kind: &str,
        token: Option<(usize, &Token)>,
        inline_index: Option<usize>,
        run: SourceRun,
    ) {
        let end = self
            .position
            .zip(token)
            .map(|(start, (_, token))| start + token.logical_len);
        let utf8 = token.and_then(|(_, token)| match &token.kind {
            TokenKind::Text(text) => Some(0..text.len()),
            _ => None,
        });
        self.tokens.push(SourceToken {
            kind: kind.to_owned(),
            run: run.run,
            item: run.item,
            token_index: token.map(|(index, _)| index),
            inline_index,
            source_char_style_id: run.raw_style,
            char_style_id: run.style,
            logical: SourceLogicalRange {
                start: self.position,
                end,
            },
            utf8,
        });
        self.position = end;
    }
}

/// [`parse_paragraph`], also returning where each table of its runs sits in
/// its logical stream, for the caller that parses those tables.
fn parse_paragraph_with_tables(
    node: roxmltree::Node<'_, '_>,
    section_index: usize,
    id: String,
    source_path: &str,
    styles: &HeaderStyles,
    result: &mut SectionParseResult,
) -> Result<(Paragraph, TableSlots)> {
    let mut table_slots = TableSlots::new();
    let mut inline_sources = Vec::new();
    let source_para_style_id = attr_u32(node, "paraPrIDRef", 0);
    let para_style_id = styles.resolve_para_style_id(source_para_style_id);
    let para_style = styles.para_style(para_style_id);
    let primary_char_style_id = children(node, "run")
        .next()
        .map(|run| styles.resolve_char_style_id(attr_u32(run, "charPrIDRef", 0)))
        .unwrap_or(0);
    let mut tokens: Vec<Token> = Vec::new();
    let mut object_textpos: Vec<(roxmltree::NodeId, usize)> = Vec::new();
    let mut active_hyperlink = None;
    let mut source = SourceTracker::new();
    for (run_index, run) in children(node, "run").enumerate() {
        let raw_style = attr_u32(run, "charPrIDRef", 0);
        let char_style_id = styles.resolve_char_style_id(raw_style);
        for (item_index, item) in run.children().filter(|item| item.is_element()).enumerate() {
            let origin = SourceRun {
                run: run_index,
                item: item_index,
                raw_style,
                style: char_style_id,
            };
            let token_start = tokens.len();
            let inline_start = inline_sources.len();
            if is_supported_object_name(local_name(item)) && local_name(item) != "tbl" {
                object_textpos.push((
                    item.id(),
                    tokens.iter().map(|token| token.logical_len).sum(),
                ));
            }
            match local_name(item) {
                "t" => {
                    // Some HWPX exporters put hp:lineBreak between text nodes
                    // inside hp:t. Keep that break in the logical stream so
                    // linesegarray textpos values continue to address the
                    // following text, but do not render it as a second visual
                    // line break; the line segments already provide layout.
                    for part in item.children() {
                        let part_start = tokens.len();
                        if part.is_text() {
                            let text = part.text().unwrap_or_default().to_owned();
                            if !text.is_empty() {
                                tokens.push(Token {
                                    logical_len: text.encode_utf16().count(),
                                    kind: TokenKind::Text(text),
                                    char_style_id,
                                    hyperlink: active_hyperlink.clone(),
                                });
                            }
                        } else {
                            match local_name(part) {
                                "tab" => tokens.push(Token {
                                    kind: TokenKind::Tab {
                                        width: attr_i64(part, "width", 8000),
                                        leader: attr_u32(part, "leader", 0),
                                    },
                                    logical_len: 8,
                                    char_style_id,
                                    hyperlink: None,
                                }),
                                "lineBreak" => tokens.push(Token {
                                    kind: TokenKind::Control {
                                        kind: "lineBreak".to_owned(),
                                    },
                                    logical_len: 1,
                                    char_style_id,
                                    hyperlink: None,
                                }),
                                // The same inline elements may appear either as
                                // a direct child of hp:run or nested inside
                                // hp:t; both spellings are the same character
                                // and must be read identically. Dropping them
                                // here silently shortened the logical stream by
                                // one unit each, which shifted every later
                                // lineseg/@textpos boundary in the paragraph.
                                "nbSpace" => tokens.push(Token {
                                    kind: TokenKind::NonBreakingSpace,
                                    logical_len: 1,
                                    char_style_id,
                                    hyperlink: None,
                                }),
                                "fwSpace" => tokens.push(Token {
                                    kind: TokenKind::FixedSpace,
                                    logical_len: 1,
                                    char_style_id,
                                    hyperlink: None,
                                }),
                                _ => {}
                            }
                        }
                        if part.is_element() || tokens.len() > part_start {
                            source.record(
                                if part.is_text() {
                                    "text"
                                } else {
                                    local_name(part)
                                },
                                tokens.get(part_start).map(|t| (part_start, t)),
                                None,
                                origin,
                            );
                        }
                    }
                }
                "secPr" => tokens.push(Token {
                    // Section and column properties occupy eight UTF-16 logical
                    // positions each, even though they emit no visible text.
                    logical_len: 8,
                    kind: TokenKind::Control {
                        kind: "secPr".to_owned(),
                    },
                    char_style_id,
                    hyperlink: None,
                }),
                "tbl" => tokens.push({
                    table_slots.push((
                        item.id(),
                        tokens.iter().map(|token| token.logical_len).sum(),
                    ));
                    Token {
                        // A table embedded in a run occupies eight logical
                        // positions in its own right, same as every other
                        // object-like run child -- it is parsed into a separate
                        // Block::Table elsewhere, but that extraction must not
                        // silently drop its slot in THIS paragraph's own
                        // logical stream. Confirmed against 2022회계연도
                        // 성과보고서's "무역위원회" paragraph: its first run
                        // opens with a PARA-anchored table before any real
                        // text, and without this token every later
                        // lineseg/@textpos boundary in that paragraph
                        // undercounts by exactly 8, splitting mid-word.
                        logical_len: 8,
                        kind: TokenKind::Control {
                            kind: "tbl".to_owned(),
                        },
                        char_style_id,
                        hyperlink: None,
                    }
                }),
                "tab" => tokens.push(Token {
                    kind: TokenKind::Tab {
                        width: attr_i64(item, "width", 8000),
                        leader: attr_u32(item, "leader", 0),
                    },
                    logical_len: 8,
                    char_style_id,
                    hyperlink: None,
                }),
                "lineBreak" => tokens.push(Token {
                    kind: TokenKind::LineBreak,
                    logical_len: 1,
                    char_style_id,
                    hyperlink: None,
                }),
                "nbSpace" => tokens.push(Token {
                    kind: TokenKind::NonBreakingSpace,
                    logical_len: 1,
                    char_style_id,
                    hyperlink: None,
                }),
                "fwSpace" => tokens.push(Token {
                    kind: TokenKind::FixedSpace,
                    logical_len: 1,
                    char_style_id,
                    hyperlink: None,
                }),
                "ctrl" => {
                    for control in item.children().filter(|child| child.is_element()) {
                        let control_start = tokens.len();
                        let control_inline = inline_sources.len();
                        let kind = local_name(control).to_owned();
                        if kind == "autoNum" {
                            inline_sources.push(InlineSource {
                                textpos: tokens.iter().map(|token| token.logical_len).sum(),
                                content: auto_number(control),
                            });
                        }
                        if kind == "fieldBegin"
                            && attr_string(control, "type").eq_ignore_ascii_case("HYPERLINK")
                        {
                            active_hyperlink = descendants(control, "stringParam")
                                .find(|parameter| {
                                    attr_string(*parameter, "name").eq_ignore_ascii_case("Path")
                                })
                                .map(text_content);
                        }
                        let is_page_new_num = kind == "newNum"
                            && attr_string(control, "numType").eq_ignore_ascii_case("PAGE");
                        if kind == "pageNum" {
                            result.has_page_number_control = true;
                            result.page_number_format = Some(attr_string(control, "formatType"));
                            result.page_number_side = Some(attr_string(control, "sideChar"));
                        } else if is_page_new_num {
                            result.page_number_start = Some(attr_i64(control, "num", 1));
                        }
                        let logical_len = match kind.as_str() {
                            // A `newNum` that restarts the page counter is the
                            // one measured exception: it consumes a single
                            // logical position (unit-tested). Every other
                            // object-like inline control measured so far
                            // (field markers, page/column properties, page
                            // number fields, bookmarks, and `newNum` for
                            // PICTURE/TABLE/EQUATION) occupies eight logical
                            // text positions even though it emits no visible
                            // text of its own. linesegarray::textpos
                            // addresses this stream, so treating them as one
                            // position shifts every later line in a paragraph
                            // that carries one. See
                            // 샘플/제어정보길이_검증/제어문자.hwpx for the
                            // measurements.
                            "newNum" if is_page_new_num => 1,
                            "fieldBegin" | "fieldEnd" | "colPr" | "pageNum" | "bookmark"
                            | "newNum" => 8,
                            // Every hp:ctrl kind we haven't measured directly
                            // (indexMark, titleMark, pageHiding,
                            // pageOddEvenAdjust, footnote/endnote anchors, ...)
                            // used to fall back to 1 with no justification.
                            // The HWP5 extended-control record is [code,
                            // ctrl_id, reserved x4, code] = 8 UTF-16 units
                            // (spec table 58) for every kind except the
                            // page-restart newNum handled above; rhwp
                            // (edwardkim/rhwp, model/control.rs
                            // CTRL_CHAR_CODE_UNITS/occupies_ctrl_char_slot)
                            // encodes the same default from that spec table.
                            _ => 8,
                        };
                        tokens.push(Token {
                            kind: TokenKind::Control { kind },
                            logical_len,
                            char_style_id,
                            hyperlink: None,
                        });
                        if local_name(control) == "fieldEnd" && active_hyperlink.is_some() {
                            active_hyperlink = None;
                        }
                        source.record(
                            local_name(control),
                            tokens.get(control_start).map(|t| (control_start, t)),
                            (inline_sources.len() > control_inline).then_some(control_inline),
                            origin,
                        );
                    }
                }
                // Inline drawing objects (pic, container, rect, ellipse, etc.)
                // that have treat_as_char set occupy eight logical positions in
                // the HWP character stream, identical to tbl and secPr.  Their
                // textpos is recorded above (before this match) so the slot in
                // object_textpos is already correct; we must push the token here
                // so that total_len accounts for the object.  Without it, a
                // paragraph whose only content is an inline shape gets
                // total_len=0, the layout end clamp collapses to 0, and the
                // object is silently dropped from every line's inline_objects.
                name if is_supported_object_name(name) => tokens.push(Token {
                    logical_len: 8,
                    kind: TokenKind::Control {
                        kind: name.to_owned(),
                    },
                    char_style_id,
                    hyperlink: None,
                }),
                // A run child of the same kind as the controls above: eight
                // logical positions (제어문자.hwpx: every `lineseg/@textpos`
                // after a 덧말 or 겹친 글자 is only right counting eight; the
                // three-덧말 paragraph's second line starts at 3 x 8 + 2 =
                // 26). The token marks where the content goes in a line; the
                // content itself is kept in `inline_sources`.
                "dutmal" | "compose" => {
                    let kind = local_name(item);
                    let textpos = tokens.iter().map(|token| token.logical_len).sum();
                    inline_sources.push(InlineSource {
                        textpos,
                        content: if kind == "dutmal" {
                            let style_ref = attr_u32(item, "styleIDRef", 0);
                            InlineContent::Ruby {
                                base: child(item, "mainText")
                                    .map(text_content)
                                    .unwrap_or_default(),
                                annotation: child(item, "subText")
                                    .map(text_content)
                                    .unwrap_or_default(),
                                position: attr_string(item, "posType"),
                                size_ratio: attr_i64(item, "szRatio", 0),
                                option: attr_i64(item, "option", 0),
                                style_ref,
                                align: attr_string(item, "align"),
                                char_style_id: styles.style_char_style_id(style_ref),
                            }
                        } else {
                            let char_prs = children(item, "charPr")
                                .map(|char_pr| attr_u32(char_pr, "prIDRef", u32::MAX))
                                .collect::<Vec<_>>();
                            InlineContent::Compose {
                                text: attr_string(item, "composeText"),
                                shape: attr_string(item, "circleType"),
                                char_size: attr_i64(item, "charSz", 0),
                                compose_type: attr_string(item, "composeType"),
                                char_style_ids: char_prs
                                    .iter()
                                    .map(|&raw| {
                                        (raw != u32::MAX).then(|| styles.resolve_char_style_id(raw))
                                    })
                                    .collect(),
                                char_prs,
                            }
                        },
                    });
                    tokens.push(Token {
                        kind: TokenKind::Control {
                            kind: kind.to_owned(),
                        },
                        logical_len: 8,
                        char_style_id,
                        hyperlink: None,
                    });
                }
                _ => {}
            }
            if !matches!(local_name(item), "t" | "ctrl") {
                source.record(
                    local_name(item),
                    tokens.get(token_start).map(|t| (token_start, t)),
                    (inline_sources.len() > inline_start).then_some(inline_start),
                    origin,
                );
            }
        }
    }
    let lines = child(node, "linesegarray")
        .map(|array| {
            children(array, "lineseg")
                .map(|line| LineSeg {
                    text_end: None,
                    textpos: attr_usize(line, "textpos", 0),
                    top: attr_i64(line, "vertpos", 0),
                    height: attr_i64(line, "vertsize", 0),
                    text_height: attr_i64(line, "textheight", 0),
                    baseline: attr_i64(line, "baseline", 0),
                    spacing: attr_i64(line, "spacing", 0),
                    left: attr_i64(line, "horzpos", 0),
                    width: attr_i64(line, "horzsize", 0),
                })
                .collect()
        })
        .unwrap_or_default();
    let mut objects = Vec::new();
    for (object_ordinal, object) in node
        .descendants()
        .filter(|candidate| {
            candidate.is_element()
                && is_supported_object_name(local_name(*candidate))
                && local_name(*candidate) != "tbl"
                // Only ancestors strictly between `candidate` and `node` can
                // disqualify it (a genuinely nested container inside this
                // paragraph, whose own parse already owns that object). An
                // unbounded walk also reaches `node`'s own ancestors -- for
                // a paragraph inside a rect/container's own drawText, that
                // includes the very shape this paragraph belongs to, which
                // is itself a supported-object name and wrongly excluded
                // every picture in a shape's text (0103's
                // "s0/p#2147483648" caption rect: 3 of 5 hp:pic never
                // reached objects at all).
                && !candidate
                    .ancestors()
                    .skip(1)
                    .take_while(|ancestor| *ancestor != node)
                    .any(|ancestor| {
                        matches!(local_name(ancestor), "tbl" | "subList")
                            || is_supported_object_name(local_name(ancestor))
                    })
        })
        .enumerate()
    {
        let kind = local_name(object);
        let object_id = format!("{id}/object-{object_ordinal}-{kind}");
        let mut positioned = parse_positioned_object(
            object,
            section_index,
            source_path,
            object_id,
            kind,
            styles,
            result,
        )?;
        positioned.textpos = object_textpos
            .iter()
            .find_map(|(node_id, textpos)| (*node_id == object.id()).then_some(*textpos))
            .unwrap_or(0);
        // Only a run's own object has a slot; `object_textpos` holds exactly
        // those. Re-keying the paragraph updates the key.
        positioned.source_anchor = object_textpos
            .iter()
            .find(|(node_id, _)| *node_id == object.id())
            .map(|(_, textpos)| SourceAnchor {
                paragraph_key: id.clone(),
                textpos: *textpos,
            });
        objects.push(positioned);
    }
    let page_break = attr_bool(node, "pageBreak") || para_style.page_break_before;
    Ok((
        Paragraph {
            id: id.clone(),
            // Cell and text-box ids are already ordinal paths; top-level
            // paragraphs are re-keyed by `assign_paragraph_keys`.
            key: id.clone(),
            source_path: source_path.to_owned(),
            section_index,
            para_style_id,
            source_para_style_id,
            para_style,
            line_top_offset: reference_line_top_offset(
                styles.char_style(primary_char_style_id).font_size_hwp,
            ),
            tokens,
            source_tokens: source.tokens,
            lines,
            objects,
            inline_sources,
            page_break,
            // A newNum restarts the page number from this paragraph onward.
            // In the work report (s0 para 27), the newNum sits inside the cell
            // of a title table, so we inspect descendants rather than only
            // direct run/ctrl children.
            page_number_restart: descendants(node, "newNum")
                .find(|number| attr_string(*number, "numType").eq_ignore_ascii_case("PAGE"))
                .map(|number| attr_i64(number, "num", 1)),
            // Like newNum above, these controls can sit inside a cell or a
            // group object's text box rather than directly under a run:
            // the work report's pageHiding is in a text box (s0 para 25), the
            // 통합관리요령 공고's in a table cell (s2 para 1139, 1405), and the
            // 어린이제품 개정령안's directly under the run (s4 para 0). Only
            // hidePageNum matters here; the other pageHiding switches (header,
            // footer, master page, border, fill) leave the number alone
            // (성과보고서 s2 para 0 sets those but keeps its number).
            page_number_control: descendants(node, "pageNum").next().is_some(),
            hides_page_number: descendants(node, "pageHiding")
                .any(|hiding| attr_bool(hiding, "hidePageNum")),
            has_inline_table: children(node, "run").any(|run| {
                children(run, "tbl").any(|table| {
                    child(table, "pos").is_some_and(|pos| attr_bool(pos, "treatAsChar"))
                })
            }),
            // A colPr starts a new column layout from this paragraph onward. It
            // may sit directly under a run's ctrl or inside that run's secPr.
            // Column definitions inside nested containers (table cells, text
            // boxes) belong to their inner layout and must not start a body
            // column group on the host paragraph (work report s0 para 66).
            columns: descendants(node, "colPr")
                .filter(|column| {
                    !column
                        .ancestors()
                        .take_while(|ancestor| *ancestor != node)
                        .any(|ancestor| local_name(ancestor) == "subList")
                })
                .find(|node| attr_string(*node, "type").eq_ignore_ascii_case("NEWSPAPER"))
                .map(|node| {
                    (
                        attr_u32(node, "colCount", 1).max(1),
                        attr_i64(node, "sameGap", 0),
                    )
                }),
        },
        table_slots,
    ))
}

/// An `hp:autoNum` as recorded: its number and `autoNumFormat`.
fn auto_number(control: roxmltree::Node<'_, '_>) -> InlineContent {
    let format = child(control, "autoNumFormat");
    let format_attr = |name| {
        format
            .map(|node| attr_string(node, name))
            .unwrap_or_default()
    };
    InlineContent::AutoNumber {
        number_type: attr_string(control, "numType"),
        number: attr_u32(control, "num", 0),
        format: format_attr("type"),
        user_char: format_attr("userChar"),
        prefix: format_attr("prefixChar"),
        suffix: format_attr("suffixChar"),
        superscript: format.is_some_and(|node| attr_bool(node, "supscript")),
    }
}

/// A table's or object's `hp:caption`, its paragraphs keyed `{id}/caption/p{n}`.
fn parse_caption(
    node: roxmltree::Node<'_, '_>,
    section_index: usize,
    id: &str,
    source_path: &str,
    styles: &HeaderStyles,
    result: &mut SectionParseResult,
) -> Result<Option<Box<Caption>>> {
    let Some(caption) = child(node, "caption") else {
        return Ok(None);
    };
    let mut paragraphs = Vec::new();
    let mut unparsed_tables = 0;
    if let Some(sub_list) = child(caption, "subList") {
        for (ordinal, paragraph) in children(sub_list, "p").enumerate() {
            let (paragraph, tables) = parse_paragraph_with_tables(
                paragraph,
                section_index,
                format!("{id}/caption/p{ordinal}"),
                source_path,
                styles,
                result,
            )?;
            unparsed_tables += tables.len();
            paragraphs.push(paragraph);
        }
    }
    Ok(Some(Box::new(Caption {
        side: attr_string(caption, "side"),
        gap: attr_i64(caption, "gap", 0),
        width: attr_i64(caption, "width", 0),
        full_size: attr_bool(caption, "fullSz"),
        last_width: attr(caption, "lastWidth").and_then(|value| value.trim().parse().ok()),
        paragraphs,
        unparsed_tables,
    })))
}

/// The anchor of `table` among a paragraph's `slots`, or `None` when the
/// table is not one of that paragraph's own run children.
fn table_anchor(
    table: roxmltree::Node<'_, '_>,
    paragraph_key: &str,
    slots: &TableSlots,
) -> Option<SourceAnchor> {
    slots
        .iter()
        .find(|(node_id, _)| *node_id == table.id())
        .map(|(_, textpos)| SourceAnchor {
            paragraph_key: paragraph_key.to_owned(),
            textpos: *textpos,
        })
}

fn reference_line_top_offset(font_size_hwp: i64) -> i64 {
    // The reference exporter reserves a 70-unit minimum glyph-box offset and
    // increases it with larger paragraph-leading fonts (15pt -> 75, 16pt ->
    // 80). This is applied before the final HWPUNIT-to-mm rounding.
    (font_size_hwp / 20).max(crate::layout::LINE_TOP_OFFSET - 5)
}

/// The glyph-box amount `paragraph_anchor_y` takes off the anchor line.
fn paragraph_top_adjustment(paragraph: &Paragraph) -> i64 {
    paragraph.lines.first().map_or(0, |line| {
        if line.text_height > 0 {
            (line.text_height / 20).max(1)
        } else {
            paragraph.line_top_offset
        }
    })
}

fn line_anchor_y(line_top_offset: i64, line: &LineSeg) -> i64 {
    line.top
        - if line.text_height > 0 {
            (line.text_height / 20).max(1)
        } else {
            line_top_offset
        }
}

/// The `top` the layout gives a paragraph's later line (`paragraph_fragments`):
/// the stored `vertpos` as it is when the paragraph holds an inline object --
/// the reference reserves that line for the object alone -- and otherwise
/// reduced by the glyph box, as the first line is.
fn later_line_anchor_y(has_inline_object: bool, line_top_offset: i64, line: &LineSeg) -> i64 {
    if has_inline_object {
        line.top
    } else {
        line_anchor_y(line_top_offset, line)
    }
}

fn paragraph_anchor_y(paragraph: &Paragraph) -> Option<i64> {
    paragraph
        .lines
        .first()
        .map(|line| line_anchor_y(paragraph.line_top_offset, line))
}

/// The `linesegarray` line that holds `table_node`: the last line that starts
/// at or before the table's own logical position (`slots`). A table that
/// shares its run with earlier text or controls is still found -- 기술료
/// 공고's "45 characters, then a table" run puts the table on the paragraph's
/// second line (textpos 45), where counting earlier *runs* only ever said
/// line 0 and the table was drawn beside the text, past the paper's edge.
/// Only an inline (`treatAsChar`) table sits in the text flow that way; a
/// floating table, and a table that is not one of the paragraph's own run
/// children (nested in another object, so without a slot), keep the run count.
fn table_line_index(
    paragraph_node: roxmltree::Node<'_, '_>,
    table_node: roxmltree::Node<'_, '_>,
    slots: &TableSlots,
    lines: &[LineSeg],
) -> usize {
    let inline = child(table_node, "pos")
        .is_some_and(|position| position.attribute("treatAsChar") == Some("1"));
    inline
        .then(|| {
            slots
                .iter()
                .find(|(node_id, _)| *node_id == table_node.id())
        })
        .flatten()
        .map_or_else(
            || preceding_object_like_run_count(paragraph_node, table_node),
            |(_, textpos)| {
                lines
                    .iter()
                    .rposition(|line| line.textpos <= *textpos)
                    .unwrap_or(0)
            },
        )
}

/// How many earlier sibling `hp:run`s in this paragraph carry a table or
/// another object-like child (`hp:container`, `hp:rect`, `hp:pic`, ...) of
/// their own, before the run holding `table_node`. A treatAsChar object
/// that does not share its run with `table_node` almost always claims a
/// whole `linesegarray` line to itself -- confirmed against 성과보고서's
/// "전략 목표 Ⅰ : 융합확산을 통해..." title table, whose paragraph opens
/// with an unrelated `hp:container` (a rounded "제3장" box) on its own
/// line before the table's own. `paragraph_anchor_y` only ever reads
/// `lines.first()`, so every `hp:tbl` in a paragraph inherited that first
/// object's own line regardless of how many came before it; this table's
/// own line (the paragraph's second) never got used, and with no line
/// matching its anchor_y at render time it never appeared at all. Used
/// only to refine the *default* per-table anchor before the existing
/// TOP_AND_BOTTOM stacking pass runs, which still overrides it for the
/// table runs that pass already handles.
fn preceding_object_like_run_count(
    paragraph_node: roxmltree::Node<'_, '_>,
    table_node: roxmltree::Node<'_, '_>,
) -> usize {
    paragraph_node
        .children()
        .filter(|child| child.is_element() && local_name(*child) == "run")
        .take_while(|run| !run.descendants().any(|d| d == table_node))
        .filter(|run| {
            run.children().any(|child| {
                child.is_element()
                    && (local_name(child) == "tbl" || is_supported_object_name(local_name(child)))
            })
        })
        .count()
}

fn parse_table(
    node: roxmltree::Node<'_, '_>,
    section_index: usize,
    id: String,
    source_path: &str,
    anchor_y: Option<i64>,
    styles: &HeaderStyles,
    result: &mut SectionParseResult,
) -> Result<Table> {
    let size = child(node, "sz")
        .map(|size| BoxUnits {
            x: 0,
            y: 0,
            width: attr_i64(size, "width", 0),
            height: attr_i64(size, "height", 0),
        })
        .unwrap_or_default();
    let anchor = parse_anchor(node, attr_i64(node, "zOrder", 0));
    let out_margin = child(node, "outMargin");
    let columns = attr_usize(node, "colCnt", 0);
    let rows = attr_usize(node, "rowCnt", 0);
    let mut cells = Vec::new();
    for (row_index, row) in children(node, "tr").enumerate() {
        for (cell_ordinal, cell) in children(row, "tc").enumerate() {
            let addr = child(cell, "cellAddr");
            let span = child(cell, "cellSpan");
            let cell_size = child(cell, "cellSz");
            let row_addr = addr
                .map(|node| attr_usize(node, "rowAddr", row_index))
                .unwrap_or(row_index);
            let col_addr = addr
                .map(|node| attr_usize(node, "colAddr", cell_ordinal))
                .unwrap_or(cell_ordinal);
            let row_span = span
                .map(|node| attr_usize(node, "rowSpan", 1).max(1))
                .unwrap_or(1);
            let col_span = span
                .map(|node| attr_usize(node, "colSpan", 1).max(1))
                .unwrap_or(1);
            let box_units = BoxUnits {
                x: 0,
                y: 0,
                width: cell_size
                    .map(|node| attr_i64(node, "width", 0))
                    .unwrap_or(0),
                height: cell_size
                    .map(|node| attr_i64(node, "height", 0))
                    .unwrap_or(0),
            };
            let cell_id = format!("{id}/r{row_addr}c{col_addr}");
            let mut paragraphs = Vec::new();
            let mut tables = Vec::new();
            if let Some(sub_list) = child(cell, "subList") {
                for (paragraph_ordinal, paragraph) in children(sub_list, "p").enumerate() {
                    let paragraph_id = format!("{cell_id}/p{paragraph_ordinal}");
                    let (parsed_paragraph, table_slots) = parse_paragraph_with_tables(
                        paragraph,
                        section_index,
                        paragraph_id,
                        source_path,
                        styles,
                        result,
                    )?;
                    let anchor_y = paragraph_anchor_y(&parsed_paragraph);
                    let paragraph_lines = parsed_paragraph.lines.clone();
                    let has_inline_object = parsed_paragraph
                        .objects
                        .iter()
                        .any(|object| object.anchor.treat_as_char);
                    let line_top_offset = parsed_paragraph.line_top_offset;
                    paragraphs.push(parsed_paragraph);
                    for (nested_ordinal, nested_table) in paragraph
                        .descendants()
                        .filter(|candidate| {
                            candidate.is_element()
                                && local_name(*candidate) == "tbl"
                                && candidate
                                    .ancestors()
                                    .skip(1)
                                    .take_while(|ancestor| *ancestor != node)
                                    .all(|ancestor| local_name(ancestor) != "tbl")
                        })
                        .enumerate()
                    {
                        let nested_id =
                            format!("{cell_id}/tbl-anchor[{paragraph_ordinal}-{nested_ordinal}]");
                        // An inline table on a later line of the cell's
                        // paragraph anchors to that line, as in the body.
                        let nested_anchor_y = match table_line_index(
                            paragraph,
                            nested_table,
                            &table_slots,
                            &paragraph_lines,
                        ) {
                            0 => anchor_y,
                            line_index => paragraph_lines
                                .get(line_index)
                                .map(|line| {
                                    later_line_anchor_y(has_inline_object, line_top_offset, line)
                                })
                                .or(anchor_y),
                        };
                        let mut table = parse_table(
                            nested_table,
                            section_index,
                            nested_id,
                            source_path,
                            nested_anchor_y,
                            styles,
                            result,
                        )?;
                        table.source_anchor = table_anchor(
                            nested_table,
                            &paragraphs[paragraph_ordinal].key,
                            &table_slots,
                        );
                        tables.push(table);
                    }
                }
            }
            let border_fill = styles.border_fill(attr_u32(cell, "borderFillIDRef", 0));
            // A cell-zone brush overlays the cell's background while its own
            // borderFill still describes the cell edges. Zone endpoints are
            // inclusive; later matching brushes take precedence.
            let zone_fill = child(node, "cellzoneList")
                .into_iter()
                .flat_map(|list| children(list, "cellzone"))
                .filter(|zone| {
                    row_addr >= attr_usize(*zone, "startRowAddr", 0)
                        && row_addr <= attr_usize(*zone, "endRowAddr", 0)
                        && col_addr >= attr_usize(*zone, "startColAddr", 0)
                        && col_addr <= attr_usize(*zone, "endColAddr", 0)
                })
                .filter_map(|zone| {
                    styles
                        .border_fill(attr_u32(zone, "borderFillIDRef", 0))
                        .fill_color
                })
                .last();
            cells.push(TableCell {
                is_header: attr_bool(cell, "header"),
                border_strokes: border_fill.strokes,
                vertical_align: child(cell, "subList")
                    .and_then(|list| list.attribute("vertAlign"))
                    .unwrap_or("CENTER")
                    .to_owned(),
                id: cell_id,
                row: row_addr,
                column: col_addr,
                row_span,
                col_span,
                box_units,
                paragraphs,
                tables,
                border_fill_id: Some(attr_u32(cell, "borderFillIDRef", 0)),
                border_visible: border_fill.border_visible,
                border_left_width: border_fill.left_width,
                border_right_width: border_fill.right_width,
                border_top_width: border_fill.top_width,
                border_bottom_width: border_fill.bottom_width,
                fill_color: zone_fill.or(border_fill.fill_color),
                gradient: border_fill.gradient,
                gradient_step: border_fill.gradient_step,
                gradient_angle: border_fill.gradient_angle,
                diagonal_forward: border_fill.diagonal_forward,
                diagonal_backward: border_fill.diagonal_backward,
                diagonal_stroke: border_fill.diagonal_stroke,
                diagonal_width: border_fill.diagonal_width,
                margin_left: cell_margin(cell, "left"),
                margin_right: cell_margin(cell, "right"),
                margin_top: cell_margin(cell, "top"),
                margin_bottom: cell_margin(cell, "bottom"),
                repeated_header: false,
                repeated_from: None,
            });
        }
    }
    Ok(Table {
        caption: parse_caption(node, section_index, &id, source_path, styles, result)?,
        id: id.clone(),
        source_path: source_path.to_owned(),
        source_anchor: None,
        section_index,
        anchor_y,
        // Set by the caller, which knows the paragraph this table hangs off.
        anchor_top_adjustment: 0,
        anchor_paragraph_left: 0,
        anchor_paragraph_right: 0,
        box_units: size,
        anchor,
        out_margin_left: out_margin
            .map(|node| attr_i64(node, "left", 0))
            .unwrap_or(0),
        out_margin_right: out_margin
            .map(|node| attr_i64(node, "right", 0))
            .unwrap_or(0),
        out_margin_top: out_margin.map(|node| attr_i64(node, "top", 0)).unwrap_or(0),
        out_margin_bottom: out_margin
            .map(|node| attr_i64(node, "bottom", 0))
            .unwrap_or(0),
        columns,
        rows,
        row_heights: Vec::new(),
        cells,
        page_break: attr_string(node, "pageBreak"),
        repeat_header: attr_bool(node, "repeatHeader"),
        no_adjust: attr_bool(node, "noAdjust"),
        fragment_rows: None,
    })
}

/// Key a top-level paragraph, and the text-box paragraphs of the objects it
/// carries, by structural position: their source ids repeat across the
/// section, so paths derived from them do too.
fn assign_paragraph_keys(paragraph: &mut Paragraph, key: String) {
    for (ordinal, object) in paragraph.objects.iter_mut().enumerate() {
        assign_object_keys(object, &format!("{key}/o{ordinal}"));
        if let Some(anchor) = object.source_anchor.as_mut() {
            anchor.paragraph_key.clone_from(&key);
        }
    }
    paragraph.key = key;
}

fn assign_object_keys(object: &mut PositionedObject, key: &str) {
    object.key = key.to_owned();
    if let Some(caption) = object.caption.as_mut() {
        for (ordinal, paragraph) in caption.paragraphs.iter_mut().enumerate() {
            assign_paragraph_keys(paragraph, format!("{key}/caption/p{ordinal}"));
        }
    }
    if let Some(shape) = object.shape.as_mut() {
        for (ordinal, paragraph) in shape.paragraphs.iter_mut().enumerate() {
            let old = paragraph.key.clone();
            assign_paragraph_keys(paragraph, format!("{key}/p{ordinal}"));
            for table in &mut shape.tables {
                if let Some(anchor) = table.source_anchor.as_mut() {
                    if anchor.paragraph_key == old {
                        anchor.paragraph_key.clone_from(&paragraph.key);
                    }
                }
            }
        }
    }
    for (ordinal, child) in object.children.iter_mut().enumerate() {
        assign_object_keys(child, &format!("{key}/c{ordinal}"));
    }
}

fn cell_margin(cell: roxmltree::Node<'_, '_>, side: &str) -> i64 {
    // Disabled cell margins inherit from the nearest containing table. Stale
    // cellMargin values may remain in the XML after the override is disabled.
    if !attr_bool(cell, "hasMargin") {
        if let Some(value) = cell
            .ancestors()
            .find(|node| node.is_element() && local_name(*node) == "tbl")
            .and_then(|table| child(table, "inMargin"))
            .and_then(|margin| margin.attribute(side))
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value >= 0 && *value != i64::from(u32::MAX))
        {
            return value;
        }
    }
    let explicit = child(cell, "cellMargin")
        .and_then(|node| node.attribute(side))
        .and_then(|value| value.parse::<i64>().ok());
    let value = explicit.unwrap_or(0);
    // Zero is an explicit margin when the cell enables its own margins.
    // Keep the existing inherited/default path for cells without that flag.
    if (value == 0 && !(explicit.is_some() && attr_bool(cell, "hasMargin")))
        || value == i64::from(u32::MAX)
    {
        match side {
            "left" | "right" => 510,
            "top" | "bottom" => 141,
            _ => 0,
        }
    } else {
        value
    }
}

fn parse_positioned_object(
    node: roxmltree::Node<'_, '_>,
    section_index: usize,
    source_path: &str,
    id: String,
    kind: &str,
    styles: &HeaderStyles,
    result: &mut SectionParseResult,
) -> Result<PositionedObject> {
    let anchor = parse_anchor(node, attr_i64(node, "zOrder", 0));
    // A shape's own curSz can be absent or degenerate (a fully collapsed
    // height="0", seen on drawn rects that are only ever displayed through
    // their renderingInfo transform) with no `sz` fallback either, leaving
    // parse_object_size() to return the untransformed orgSz -- the shape's
    // local design size, not what actually gets displayed. Group children
    // already resolve this via transformed_shape_box(); apply the same
    // renderingInfo matrix chain here so a standalone scaled/positioned
    // shape's own box reflects its real displayed size too. This is a
    // no-op whenever the node has no orgSz/renderingInfo pair.
    let mut object = PositionedObject {
        id: id.clone(),
        key: id.clone(),
        source_path: source_path.to_owned(),
        source_anchor: None,
        caption: None,
        equation: None,
        textpos: 0,
        kind: kind.to_owned(),
        box_units: transformed_shape_box(node, parse_object_size(node)),
        anchor,
        alt: String::new(),
        description: String::new(),
        paragraph_key: String::new(),
        binary_ref: None,
        mime_type: None,
        crop: None,
        img_dim: BoxUnits::default(),
        original_size: parse_named_size(node, "orgSz").unwrap_or_default(),
        flip_x: false,
        flip_y: false,
        stacking_order: 0,
        shape: None,
        children: Vec::new(),
    };
    object.caption = parse_caption(node, section_index, &id, source_path, styles, result)?;
    if kind == "equation" {
        object.equation = Some(Box::new(EquationSource {
            script: child(node, "script").map(text_content).unwrap_or_default(),
            base_unit: attr_i64(node, "baseUnit", 0),
            base_line: attr_i64(node, "baseLine", 0),
            font: attr_string(node, "font"),
            text_color: attr_string(node, "textColor"),
        }));
    }
    if kind == "container" {
        for (ordinal, nested) in node
            .descendants()
            .filter(|candidate| {
                candidate.is_element()
                    && is_supported_object_name(local_name(*candidate))
                    && local_name(*candidate) != "container"
                    && candidate
                        .ancestors()
                        .skip(1)
                        .take_while(|parent| *parent != node)
                        .all(|parent| {
                            !is_supported_object_name(local_name(parent))
                                || local_name(parent) == "container"
                        })
            })
            .enumerate()
        {
            let mut child_object = parse_positioned_object(
                nested,
                section_index,
                source_path,
                format!("{id}/child[{ordinal}]"),
                local_name(nested),
                styles,
                result,
            )?;
            child_object.box_units = transformed_shape_box(nested, child_object.box_units);
            object.children.push(child_object);
        }
    } else if matches!(kind, "rect" | "polygon" | "line") {
        let draw_text = child(node, "drawText");
        let sublist = draw_text.and_then(|draw| child(draw, "subList"));
        let margins = draw_text.and_then(|draw| child(draw, "textMargin"));
        let mut paragraphs = Vec::new();
        let mut tables = Vec::new();
        if let Some(sublist) = sublist {
            for (ordinal, paragraph) in children(sublist, "p").enumerate() {
                let (parsed, table_slots) = parse_paragraph_with_tables(
                    paragraph,
                    section_index,
                    format!("{id}/text/p{ordinal}"),
                    source_path,
                    styles,
                    result,
                )?;
                let parsed_key = parsed.key.clone();
                // A table in the text box belongs to it, as rhwp lays it out
                // in the text box's inner area (`shape_layout.rs`), from the
                // paragraph's own line position (`para_start_y`), not the
                // glyph top a cell's nested table uses: 성과보고서 별첨4's
                // title table is 2.07mm into its text box in the reference,
                // the text margin and outer margin with no line offset.
                let anchor_y = parsed.lines.first().map(|line| line.top);
                paragraphs.push(parsed);
                for (nested_ordinal, nested_table) in paragraph
                    .descendants()
                    .filter(|candidate| {
                        candidate.is_element()
                            && local_name(*candidate) == "tbl"
                            && candidate
                                .ancestors()
                                .skip(1)
                                .take_while(|ancestor| *ancestor != paragraph)
                                .all(|ancestor| {
                                    local_name(ancestor) != "tbl"
                                        && local_name(ancestor) != "drawText"
                                })
                    })
                    .enumerate()
                {
                    let mut table = parse_table(
                        nested_table,
                        section_index,
                        format!("{id}/text/tbl-anchor[{ordinal}-{nested_ordinal}]"),
                        source_path,
                        anchor_y,
                        styles,
                        result,
                    )?;
                    table.source_anchor = table_anchor(nested_table, &parsed_key, &table_slots);
                    tables.push(table);
                }
            }
        }
        let brush = child(node, "fillBrush");
        let line = child(node, "lineShape");
        let points = if kind == "polygon" {
            children(node, "pt")
                .map(|pt| (attr_i64(pt, "x", 0), attr_i64(pt, "y", 0)))
                .collect()
        } else if kind == "line" {
            [child(node, "startPt"), child(node, "endPt")]
                .into_iter()
                .flatten()
                .map(|pt| (attr_i64(pt, "x", 0), attr_i64(pt, "y", 0)))
                .collect()
        } else {
            Vec::new()
        };
        let points = transform_shape_points(node, points);
        object.shape = Some(Box::new(crate::model::ShapeStyle {
            points,
            fill: brush
                .and_then(|brush| child(brush, "winBrush"))
                .map(|brush| attr_string(brush, "faceColor"))
                .filter(|color| color.starts_with('#')),
            image_fill: brush
                .and_then(|brush| child(brush, "imgBrush"))
                .and_then(|brush| child(brush, "img"))
                .map(|img| attr_string(img, "binaryItemIDRef"))
                .filter(|reference| !reference.is_empty()),
            gradient: brush
                .and_then(|brush| child(brush, "gradation"))
                .into_iter()
                .flat_map(|gradient| children(gradient, "color"))
                .map(|color| attr_string(color, "value"))
                .collect(),
            gradient_step: brush
                .and_then(|brush| child(brush, "gradation"))
                .map(|gradient| attr_u32(gradient, "step", 0))
                .unwrap_or(0),
            gradient_angle: brush
                .and_then(|brush| child(brush, "gradation"))
                .map(|gradient| attr_i64(gradient, "angle", 0))
                .unwrap_or(0),
            line_color: line
                .map(|line| attr_string(line, "color"))
                .unwrap_or_default(),
            line_width: line
                .filter(|line| !attr_string(*line, "style").eq_ignore_ascii_case("NONE"))
                .map(|line| attr_i64(line, "width", 0))
                .unwrap_or(0),
            declared_line_width: line.map(|line| attr_i64(line, "width", 0)).unwrap_or(0),
            corner_ratio: attr_i64(node, "ratio", 0),
            paragraphs,
            tables,
            margins: ["left", "right", "top", "bottom"]
                .map(|name| margins.map(|m| attr_i64(m, name, 0)).unwrap_or(0)),
            vertical_align: sublist
                .map(|list| attr_string(list, "vertAlign"))
                .unwrap_or_default(),
        }));
    }
    if kind == "pic" {
        if let Some(image) = descendants(node, "img").next() {
            let binary = attr_string(image, "binaryItemIDRef");
            if !binary.is_empty() {
                object.binary_ref = Some(binary);
            }
        }
        object.alt = descendants(node, "caption")
            .next()
            .map(text_content)
            .unwrap_or_else(|| "image".to_owned());
        object.description = descendants(node, "caption")
            .next()
            .map(|caption| collapse_whitespace(&text_content(caption)))
            .filter(|caption| !caption.is_empty())
            .or_else(|| {
                descendants(node, "shapeComment")
                    .next()
                    .and_then(|comment| picture_description(&text_content(comment)))
            })
            .unwrap_or_default();
        if let Some(clip) = descendants(node, "imgClip").next() {
            object.crop = Some(BoxUnits {
                x: attr_i64(clip, "left", 0),
                y: attr_i64(clip, "top", 0),
                width: attr_i64(clip, "right", 0) - attr_i64(clip, "left", 0),
                height: attr_i64(clip, "bottom", 0) - attr_i64(clip, "top", 0),
            });
        }
        if let Some(dim) = descendants(node, "imgDim").next() {
            object.img_dim = BoxUnits {
                x: 0,
                y: 0,
                width: attr_i64(dim, "dimwidth", 0),
                height: attr_i64(dim, "dimheight", 0),
            };
        }
        if let Some(flip) = descendants(node, "flip").next() {
            let value = attr_string(flip, "value").to_ascii_lowercase();
            object.flip_x = value.contains("horizontal") || attr_bool(flip, "horizontal");
            object.flip_y = value.contains("vertical") || attr_bool(flip, "vertical");
        }
    } else if is_unsupported_name(kind) {
        object.alt = kind.to_owned();
    }
    Ok(object)
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text alternative a picture's `hp:shapeComment` offers. Hancom fills
/// the comment itself: "그림입니다." then the original file name, pixel size
/// and photo metadata (115 of the samples' 151 pictures, none edited). Only
/// the file name can describe the picture, and not when a program named it.
/// A comment the author wrote replaces that text and is kept whole.
fn picture_description(comment: &str) -> Option<String> {
    let comment = comment.trim();
    if !comment.starts_with("그림입니다.") {
        let text = collapse_whitespace(comment);
        return (!text.is_empty()).then_some(text);
    }
    let name = comment
        .lines()
        .find_map(|line| line.trim().strip_prefix("원본 그림의 이름:"))?
        .trim();
    let stem = match name.rsplit_once('.') {
        Some((stem, extension))
            if (1..=5).contains(&extension.len())
                && extension.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            stem
        }
        _ => name,
    };
    let stem = collapse_whitespace(&stem.replace('_', " "));
    (!stem.is_empty() && !machine_named(&stem)).then_some(stem)
}

/// A name a program gave a picture: a clipboard paste (`CLP0000420848bb`),
/// a generic word followed only by numbers (`photo 2022-12-06 21-03-02`,
/// `IMG 1234`, `스크린샷 2024-01-01`), bare numbers, or a long hex id.
fn machine_named(name: &str) -> bool {
    // Longer words first, so `picture` is not read as `pic` + `ture`.
    const GENERIC: [&str; 18] = [
        "screenshot",
        "kakaotalk",
        "untitled",
        "picture",
        "capture",
        "image",
        "photo",
        "scan",
        "clip",
        "dcim",
        "dscn",
        "dsc",
        "img",
        "pic",
        "스크린샷",
        "이미지",
        "그림",
        "사진",
    ];
    let lower = name.to_lowercase();
    if let Some(id) = lower.strip_prefix("clp") {
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }
    }
    let rest = GENERIC
        .iter()
        .find_map(|word| lower.strip_prefix(word))
        .unwrap_or(&lower);
    if rest
        .chars()
        .all(|c| c.is_ascii_digit() || c.is_whitespace() || "-.()".contains(c))
    {
        return true;
    }
    let compact = lower
        .chars()
        .filter(|c| !"- ".contains(*c))
        .collect::<String>();
    compact.len() >= 16 && compact.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_object_size(node: roxmltree::Node<'_, '_>) -> BoxUnits {
    let candidates = ["curSz", "sz", "orgSz"];
    for name in candidates {
        if let Some(size) = child(node, name) {
            let width = attr_i64(size, "width", 0);
            let height = attr_i64(size, "height", 0);
            if width > 0 && height > 0 {
                return BoxUnits {
                    x: 0,
                    y: 0,
                    width,
                    height,
                };
            }
        }
    }
    BoxUnits {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    }
}

fn transformed_shape_box(node: roxmltree::Node<'_, '_>, fallback: BoxUnits) -> BoxUnits {
    let Some(original) = child(node, "orgSz") else {
        return fallback;
    };
    let Some(info) = child(node, "renderingInfo") else {
        return fallback;
    };
    let mut points = [
        (0.0, 0.0),
        (attr_i64(original, "width", 1) as f64, 0.0),
        (0.0, attr_i64(original, "height", 1) as f64),
        (
            attr_i64(original, "width", 1) as f64,
            attr_i64(original, "height", 1) as f64,
        ),
    ];
    for matrix in info.children().filter(|node| node.is_element()).rev() {
        let value = |name, fallback| {
            matrix
                .attribute(name)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
        };
        for (x, y) in &mut points {
            (*x, *y) = (
                value("e1", 1.0) * *x + value("e2", 0.0) * *y + value("e3", 0.0),
                value("e4", 0.0) * *x + value("e5", 1.0) * *y + value("e6", 0.0),
            );
        }
    }
    let left = points.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let top = points.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let right = points.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let bottom = points.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    BoxUnits {
        x: left.round() as i64,
        y: top.round() as i64,
        width: (right - left).round() as i64,
        height: (bottom - top).round() as i64,
    }
}

fn transform_shape_points(
    node: roxmltree::Node<'_, '_>,
    mut points: Vec<(i64, i64)>,
) -> Vec<(i64, i64)> {
    let Some(info) = child(node, "renderingInfo") else {
        return points;
    };
    for matrix in info.children().filter(|node| node.is_element()).rev() {
        let value = |name, fallback| {
            matrix
                .attribute(name)
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .unwrap_or(fallback)
        };
        for (x, y) in &mut points {
            let (x0, y0) = (*x as f64, *y as f64);
            *x = (value("e1", 1.0) * x0 + value("e2", 0.0) * y0 + value("e3", 0.0)).round() as i64;
            *y = (value("e4", 0.0) * x0 + value("e5", 1.0) * y0 + value("e6", 0.0)).round() as i64;
        }
    }
    points
}

fn parse_named_size(node: roxmltree::Node<'_, '_>, name: &str) -> Option<BoxUnits> {
    descendants(node, name).next().map(|size| BoxUnits {
        x: 0,
        y: 0,
        width: attr_i64(size, "width", 0),
        height: attr_i64(size, "height", 0),
    })
}

fn parse_anchor(node: roxmltree::Node<'_, '_>, z_order: i64) -> Anchor {
    let pos = child(node, "pos").or_else(|| descendants(node, "pos").next());
    let offset = child(node, "offset").or_else(|| descendants(node, "offset").next());
    Anchor {
        treat_as_char: pos
            .map(|node| attr_bool(node, "treatAsChar"))
            .unwrap_or(offset.is_none()),
        flow_with_text: pos
            .map(|node| attr_bool(node, "flowWithText"))
            .unwrap_or(true),
        vert_rel_to: pos
            .map(|node| attr_string(node, "vertRelTo"))
            .unwrap_or_else(|| "PARA".to_owned()),
        horz_rel_to: pos
            .map(|node| attr_string(node, "horzRelTo"))
            .unwrap_or_else(|| "PARA".to_owned()),
        vert_align: pos
            .map(|node| attr_string(node, "vertAlign"))
            .unwrap_or_default(),
        horz_align: pos
            .map(|node| attr_string(node, "horzAlign"))
            .unwrap_or_default(),
        vert_offset: pos
            .map(|node| attr_coord(node, "vertOffset", 0))
            .or_else(|| offset.map(|node| attr_coord(node, "y", 0)))
            .unwrap_or(0),
        horz_offset: pos
            .map(|node| attr_coord(node, "horzOffset", 0))
            .or_else(|| offset.map(|node| attr_coord(node, "x", 0)))
            .unwrap_or(0),
        z_order,
    }
}

fn element_id(index: usize, kind: &str, ordinal: usize, node: roxmltree::Node<'_, '_>) -> String {
    let original = attr_string(node, "id");
    if original.is_empty() {
        format!("s{index}/{kind}[{ordinal}]")
    } else {
        format!("s{index}/{kind}#{original}")
    }
}

fn is_supported_object_name(name: &str) -> bool {
    matches!(
        name,
        "pic"
            | "container"
            | "line"
            | "rect"
            | "polygon"
            | "ellipse"
            | "arc"
            | "curve"
            | "equation"
            | "chart"
            | "video"
            | "memo"
            | "ole"
    )
}

fn is_unsupported_name(name: &str) -> bool {
    matches!(
        name,
        "equation" | "chart" | "video" | "memo" | "ole" | "ellipse" | "arc" | "curve"
    )
}

#[cfg(test)]
mod tests {
    use crate::model::{Block, Paragraph, Table, TokenKind};

    #[test]
    fn picture_descriptions_keep_only_meaningful_file_names() {
        let hancom = |name: &str| {
            format!("그림입니다.\r\n원본 그림의 이름: {name}\r\n원본 그림의 크기: 가로 602pixel, 세로 237pixel")
        };
        let describe = |name: &str| super::picture_description(&hancom(name));
        assert_eq!(
            describe("슬로건_보도자료_상단.png").as_deref(),
            Some("슬로건 보도자료 상단")
        );
        assert_eq!(
            describe("에너지효율혁신발대식(22.7.4.).PNG").as_deref(),
            Some("에너지효율혁신발대식(22.7.4.)")
        );
        assert_eq!(
            describe("img_opentype01.jpg").as_deref(),
            Some("img opentype01")
        );
        for machine in [
            "CLP0000420848bb.bmp",
            "photo_2022-12-06_21-03-02.jpg",
            "IMG_1234.JPG",
            "KakaoTalk_20230101_123456.png",
            "스크린샷 2024-01-01 120000.png",
            "20230101.png",
            "3f2a9c0d8e7b6a5f4c3d2e1f.png",
        ] {
            assert_eq!(describe(machine), None, "{machine}");
        }
        assert_eq!(super::picture_description("그림입니다."), None);
        assert_eq!(
            super::picture_description(" 2022년 수출 실적\r\n그래프 ").as_deref(),
            Some("2022년 수출 실적 그래프")
        );
    }

    #[test]
    fn section_and_column_properties_preserve_logical_positions_in_run_order() {
        // Include a non-BMP character and a one-unit control to guard against
        // byte indexing and indiscriminately making every control eight units.
        let xml = r#"<p><run><t>A😀</t><secPr/><ctrl><colPr/></ctrl>
            <t>BC</t><ctrl><newNum numType="PAGE" num="1"/></ctrl></run></p>"#;
        let xml = roxmltree::Document::parse(xml).unwrap();
        let paragraph = super::parse_paragraph(
            xml.root_element(),
            0,
            "p0".into(),
            "section0.xml",
            &super::HeaderStyles::default(),
            &mut super::SectionParseResult::default(),
        )
        .unwrap();
        assert_eq!(
            paragraph
                .tokens
                .iter()
                .map(|t| t.logical_len)
                .collect::<Vec<_>>(),
            [3, 8, 8, 2, 1]
        );
        assert_eq!(
            paragraph
                .tokens
                .iter()
                .map(|t| t.visible_text())
                .collect::<String>(),
            "A😀BC"
        );
    }

    #[test]
    fn unmeasured_ctrl_kinds_default_to_eight_logical_positions() {
        // indexMark has no direct measurement in our corpus yet, but it's an
        // ordinary hp:ctrl child like the ones above, so it should fall back
        // to the general 8-unit HWP5 extended-control encoding rather than
        // the old unjustified 1.
        let xml = r#"<p><run><t>A</t><ctrl><indexMark/></ctrl><t>B</t></run></p>"#;
        let xml = roxmltree::Document::parse(xml).unwrap();
        let paragraph = super::parse_paragraph(
            xml.root_element(),
            0,
            "p0".into(),
            "section0.xml",
            &super::HeaderStyles::default(),
            &mut super::SectionParseResult::default(),
        )
        .unwrap();
        assert_eq!(
            paragraph
                .tokens
                .iter()
                .map(|t| t.logical_len)
                .collect::<Vec<_>>(),
            [1, 8, 1]
        );
    }

    #[test]
    fn inline_spaces_count_the_same_nested_in_t_as_directly_under_run() {
        // Every fwSpace/nbSpace in the corpus is spelled nested inside hp:t,
        // never as a direct hp:run child, and each one is a real character:
        // one UTF-16 logical unit that the reference renders as its own
        // `&nbsp;` span. Dropping the nested spelling shortened the logical
        // stream and shifted every later lineseg/@textpos boundary.
        let nested = r#"<p><run charPrIDRef="27"><t>A<fwSpace/>B<nbSpace/>C</t></run></p>"#;
        let direct =
            r#"<p><run charPrIDRef="27"><t>A</t><fwSpace/><t>B</t><nbSpace/><t>C</t></run></p>"#;
        let parse = |xml: &str| {
            let document = roxmltree::Document::parse(xml).unwrap();
            super::parse_paragraph(
                document.root_element(),
                0,
                "p0".into(),
                "section0.xml",
                &super::HeaderStyles::default(),
                &mut super::SectionParseResult::default(),
            )
            .unwrap()
        };
        for paragraph in [parse(nested), parse(direct)] {
            assert_eq!(
                paragraph
                    .tokens
                    .iter()
                    .map(|token| token.logical_len)
                    .collect::<Vec<_>>(),
                [1, 1, 1, 1, 1]
            );
            assert_eq!(
                paragraph
                    .tokens
                    .iter()
                    .map(|token| token.visible_text())
                    .collect::<String>(),
                "A\u{a0}B\u{a0}C"
            );
        }
    }

    #[test]
    fn disabled_cell_margins_inherit_from_the_nearest_table() {
        let xml = r#"<tbl><inMargin top="141" bottom="141"/>
          <tr><tc hasMargin="0"><cellMargin top="800" bottom="566"/>
            <tbl><inMargin top="0" bottom="250"/>
              <tr><tc hasMargin="0"><cellMargin top="700" bottom="900"/></tc>
              <tc hasMargin="1"><cellMargin top="0" bottom="566"/></tc></tr>
            </tbl>
          </tc></tr></tbl>"#;
        let document = roxmltree::Document::parse(xml).unwrap();
        let cells = document
            .descendants()
            .filter(|node| node.has_tag_name("tc"))
            .collect::<Vec<_>>();
        assert_eq!(super::cell_margin(cells[0], "bottom"), 141);
        assert_eq!(super::cell_margin(cells[1], "top"), 0);
        assert_eq!(super::cell_margin(cells[1], "bottom"), 250);
        assert_eq!(super::cell_margin(cells[2], "bottom"), 566);
    }

    #[test]
    fn explicit_zero_margin_is_distinct_from_inherited_or_missing_margin() {
        for (flag, margin, expected) in [
            ("1", "top=\"0\"", 0),
            ("0", "top=\"0\"", 141),
            ("1", "", 141),
            ("1", "top=\"invalid\"", 141),
            ("1", "top=\"4294967295\"", 141),
            ("1", "top=\"566\"", 566),
        ] {
            let xml = format!("<tc hasMargin=\"{flag}\"><cellMargin {margin}/></tc>");
            let document = roxmltree::Document::parse(&xml).unwrap();
            assert_eq!(super::cell_margin(document.root_element(), "top"), expected);
        }
    }

    use super::collect_redundant_page_number_styles;

    #[test]
    fn a_duplicate_page_number_control_is_excluded_but_the_first_copy_is_kept() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p>
                <hp:run charPrIDRef="21"><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="20"><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="18"><hp:t>title text</hp:t></hp:run>
            </hp:p>
        </hs:sec>"#;
        let excluded = collect_redundant_page_number_styles(&[(
            "section0.xml".to_owned(),
            section.as_bytes(),
        )])
        .unwrap();
        assert_eq!(excluded, [20].into_iter().collect());
    }

    #[test]
    fn a_single_page_number_control_is_never_excluded() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p>
                <hp:run charPrIDRef="21"><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="18"><hp:t>title text</hp:t></hp:run>
            </hp:p>
        </hs:sec>"#;
        let excluded = collect_redundant_page_number_styles(&[(
            "section0.xml".to_owned(),
            section.as_bytes(),
        )])
        .unwrap();
        assert!(excluded.is_empty());
    }

    #[test]
    fn a_duplicate_control_whose_style_is_also_used_with_text_elsewhere_is_kept() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p>
                <hp:run charPrIDRef="21"><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="20"><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="18"><hp:t>title text</hp:t></hp:run>
            </hp:p>
            <hp:p>
                <hp:run charPrIDRef="20"><hp:t>this id is not actually a one-off</hp:t></hp:run>
            </hp:p>
        </hs:sec>"#;
        let excluded = collect_redundant_page_number_styles(&[(
            "section0.xml".to_owned(),
            section.as_bytes(),
        )])
        .unwrap();
        assert!(excluded.is_empty());
    }

    #[test]
    fn two_page_number_controls_with_different_attributes_are_not_duplicates() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p>
                <hp:run charPrIDRef="21"><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="20"><hp:ctrl><hp:pageNum pos="TOP_LEFT" formatType="DIGIT" sideChar="-"/></hp:ctrl></hp:run>
                <hp:run charPrIDRef="18"><hp:t>title text</hp:t></hp:run>
            </hp:p>
        </hs:sec>"#;
        let excluded = collect_redundant_page_number_styles(&[(
            "section0.xml".to_owned(),
            section.as_bytes(),
        )])
        .unwrap();
        assert!(excluded.is_empty());
    }

    #[test]
    fn a_standalone_shapes_own_box_reflects_its_rendering_info_transform() {
        // Numbers taken from 샘플/그라디언트 샘플's first gradient rect: its own
        // curSz/sz are absent or degenerate, so before this fix
        // parse_object_size() fell back to the untransformed orgSz -- far
        // smaller than what the shape actually displays at.
        let xml = r#"<hp:rect xmlns:hp="urn:hp" xmlns:hc="urn:hc">
            <hp:orgSz width="18825" height="6900"/>
            <hp:renderingInfo>
                <hc:transMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/>
                <hc:scaMatrix e1="0.918353" e2="0" e3="0" e4="0" e5="0.534783" e6="0"/>
                <hc:rotMatrix e1="1" e2="0" e3="0" e4="0" e5="1" e6="0"/>
            </hp:renderingInfo>
        </hp:rect>"#;
        let document = roxmltree::Document::parse(xml).unwrap();
        let fallback = crate::model::BoxUnits {
            x: 0,
            y: 0,
            width: 18825,
            height: 6900,
        };
        let transformed = super::transformed_shape_box(document.root_element(), fallback);
        assert_eq!(transformed.width, 17288);
        assert_eq!(transformed.height, 3690);
    }

    #[test]
    fn transformed_shape_box_is_a_no_op_without_rendering_info() {
        let xml = r#"<hp:pic xmlns:hp="urn:hp"><hp:orgSz width="100" height="200"/></hp:pic>"#;
        let document = roxmltree::Document::parse(xml).unwrap();
        let fallback = crate::model::BoxUnits {
            x: 1,
            y: 2,
            width: 300,
            height: 400,
        };
        let transformed = super::transformed_shape_box(document.root_element(), fallback);
        assert_eq!(transformed.width, fallback.width);
        assert_eq!(transformed.height, fallback.height);
        assert_eq!(transformed.x, fallback.x);
        assert_eq!(transformed.y, fallback.y);
    }

    const DUPLICATE_PARA_STYLE_HEADER: &str = r#"<hh:head xmlns:hh="urn:hh">
        <hh:refList><hh:paraProperties>
            <hh:paraPr id="3"><hh:align horizontal="CENTER"/></hh:paraPr>
            <hh:paraPr id="60"><hh:align horizontal="CENTER"/></hh:paraPr>
            <hh:paraPr id="61"><hh:align horizontal="LEFT"/></hh:paraPr>
        </hh:paraProperties></hh:refList>
    </hh:head>"#;

    #[test]
    fn a_duplicate_para_style_used_only_by_picture_only_paragraphs_is_aliased() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p paraPrIDRef="3"><hp:run><hp:t>real text</hp:t></hp:run></hp:p>
            <hp:p paraPrIDRef="60"><hp:run><hp:pic/></hp:run><hp:run><hp:t/></hp:run></hp:p>
        </hs:sec>"#;
        let aliases = super::collect_duplicate_picture_only_para_styles(
            DUPLICATE_PARA_STYLE_HEADER.as_bytes(),
            &[("section0.xml".to_owned(), section.as_bytes())],
        )
        .unwrap();
        assert_eq!(aliases, std::collections::BTreeMap::from([(60, 3)]));
    }

    #[test]
    fn a_duplicate_para_style_is_not_aliased_when_any_use_carries_real_text() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p paraPrIDRef="3"><hp:run><hp:t>real text</hp:t></hp:run></hp:p>
            <hp:p paraPrIDRef="60"><hp:run><hp:pic/></hp:run></hp:p>
            <hp:p paraPrIDRef="60"><hp:run><hp:t>caption text</hp:t></hp:run></hp:p>
        </hs:sec>"#;
        let aliases = super::collect_duplicate_picture_only_para_styles(
            DUPLICATE_PARA_STYLE_HEADER.as_bytes(),
            &[("section0.xml".to_owned(), section.as_bytes())],
        )
        .unwrap();
        assert!(aliases.is_empty());
    }

    #[test]
    fn a_non_duplicate_para_style_is_never_aliased() {
        let section = r#"<hs:sec xmlns:hp="urn:hp" xmlns:hs="urn:hs">
            <hp:p paraPrIDRef="61"><hp:run><hp:pic/></hp:run></hp:p>
        </hs:sec>"#;
        let aliases = super::collect_duplicate_picture_only_para_styles(
            DUPLICATE_PARA_STYLE_HEADER.as_bytes(),
            &[("section0.xml".to_owned(), section.as_bytes())],
        )
        .unwrap();
        assert!(aliases.is_empty());
    }

    // Plan §3.1.2 inline-order fixture: `text A -> picture -> table ->
    // text B -> table -> text C`, in a body paragraph, a cell and a text box.

    const HS: &str = "http://www.hancom.co.kr/hwpml/2011/section";
    const HP: &str = "http://www.hancom.co.kr/hwpml/2011/paragraph";

    fn inline_run(tag: &str) -> String {
        let table = |id: &str| {
            format!(
                r#"<hp:tbl id="{tag}{id}" rowCnt="1" colCnt="1"><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p><hp:run><hp:t>cell {tag}{id}</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl>"#
            )
        };
        format!(
            r#"<hp:run><hp:t>A</hp:t><hp:pic id="{tag}9"><hp:pos treatAsChar="1"/></hp:pic>{}<hp:t>B</hp:t>{}<hp:t>C</hp:t></hp:run>"#,
            table("1"),
            table("2")
        )
    }

    fn section(body: &str) -> super::SectionParseResult {
        let xml = format!(r#"<hs:sec xmlns:hs="{HS}" xmlns:hp="{HP}">{body}</hs:sec>"#);
        super::parse_section(
            xml.as_bytes(),
            0,
            "section0.xml",
            &super::HeaderStyles::default(),
        )
        .unwrap()
    }

    /// The paragraph's text and its anchored items merged by logical
    /// position, as the semantic tree reads them. `tables` may hold tables
    /// anchored elsewhere; only this paragraph's own count.
    fn reading_order(paragraph: &Paragraph, tables: &[&Table]) -> Vec<String> {
        let mut items = Vec::new();
        let mut position = 0;
        for token in &paragraph.tokens {
            if let TokenKind::Text(text) = &token.kind {
                items.push((position, 1, text.clone()));
            }
            position += token.logical_len;
        }
        for object in &paragraph.objects {
            let anchor = object.source_anchor.as_ref().expect("object anchor");
            assert_eq!(anchor.paragraph_key, paragraph.key);
            items.push((anchor.textpos, 0, object.kind.clone()));
        }
        for table in tables {
            let anchor = table.source_anchor.as_ref().expect("table anchor");
            if anchor.paragraph_key == paragraph.key {
                items.push((anchor.textpos, 0, "tbl".to_owned()));
            }
        }
        items.sort();
        items.into_iter().map(|(_, _, item)| item).collect()
    }

    const ORDER: [&str; 6] = ["A", "pic", "tbl", "B", "tbl", "C"];

    #[test]
    fn a_body_paragraphs_objects_and_tables_keep_their_text_positions() {
        let parsed = section(&format!(r#"<hp:p id="1">{}</hp:p>"#, inline_run("b")));
        let mut paragraphs = Vec::new();
        let mut tables = Vec::new();
        for block in &parsed.section.blocks {
            match block {
                Block::Paragraph(paragraph) => paragraphs.push(paragraph),
                Block::Table(table) => tables.push(table),
                Block::Object(_) => {}
            }
        }
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(reading_order(paragraphs[0], &tables), ORDER);
        assert_eq!(tables[0].source_anchor.as_ref().unwrap().textpos, 9);
        assert_eq!(tables[1].source_anchor.as_ref().unwrap().textpos, 18);
    }

    #[test]
    fn a_cells_paragraph_keeps_the_same_inline_order() {
        let cell = format!(
            r#"<hp:p id="1"><hp:run><hp:tbl id="outer" rowCnt="1" colCnt="1"><hp:pos treatAsChar="0"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p id="2">{}</hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#,
            inline_run("c")
        );
        let parsed = section(&cell);
        let outer = parsed
            .section
            .blocks
            .iter()
            .find_map(|block| match block {
                Block::Table(table) => Some(table),
                _ => None,
            })
            .unwrap();
        let cell = &outer.cells[0];
        let tables = cell.tables.iter().collect::<Vec<_>>();
        assert_eq!(reading_order(&cell.paragraphs[0], &tables), ORDER);
    }

    #[test]
    fn a_text_boxs_paragraph_keeps_the_same_inline_order_after_re_keying() {
        let body = format!(
            r#"<hp:p id="1"><hp:run><hp:rect id="5"><hp:pos treatAsChar="1"/><hp:drawText><hp:subList><hp:p id="3">{}</hp:p></hp:subList></hp:drawText></hp:rect></hp:run></hp:p>"#,
            inline_run("t")
        );
        let parsed = section(&body);
        let Some(Block::Paragraph(host)) = parsed.section.blocks.first() else {
            panic!("host paragraph")
        };
        let shape = host.objects[0].shape.as_ref().unwrap();
        // Re-keyed with the host paragraph, and the tables followed.
        assert_eq!(shape.paragraphs[0].key, "s0/p[0]/o0/p0");
        let tables = shape.tables.iter().collect::<Vec<_>>();
        assert_eq!(reading_order(&shape.paragraphs[0], &tables), ORDER);
    }

    #[test]
    fn a_table_laid_out_before_its_paragraph_keeps_its_text_position() {
        // `table_precedes_anchor_paragraph`: layout order puts the table
        // first; the anchor still reads it after "A".
        let body = r#"<hp:p id="1"><hp:run><hp:t>A</hp:t><hp:tbl id="f" rowCnt="1" colCnt="1" textWrap="TOP_AND_BOTTOM"><hp:pos treatAsChar="0" flowWithText="0" vertRelTo="PARA" vertOffset="0"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p><hp:run><hp:t>x</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl><hp:t>B</hp:t></hp:run><hp:linesegarray><hp:lineseg textpos="0" vertpos="0" vertsize="1000" textheight="1000" baseline="850" spacing="600" horzpos="0" horzsize="42520" flags="393216"/></hp:linesegarray></hp:p>"#;
        let parsed = section(body);
        let blocks = &parsed.section.blocks;
        let (Block::Table(table), Block::Paragraph(paragraph)) = (&blocks[0], &blocks[1]) else {
            panic!("expected the table before its paragraph in layout order")
        };
        assert_eq!(reading_order(paragraph, &[table]), ["A", "tbl", "B"]);
    }

    #[test]
    fn dutmal_and_compose_advance_logical_position_by_eight() {
        let body = r#"<hp:p id="1"><hp:run><hp:t>A</hp:t><hp:dutmal posType="TOP"><hp:mainText>base</hp:mainText><hp:subText>sub</hp:subText></hp:dutmal><hp:t>B</hp:t><hp:compose composeText="23"/><hp:t>C</hp:t></hp:run></hp:p>"#;
        let parsed = section(body);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected paragraph");
        };
        assert_eq!(p.tokens.len(), 5);
        assert_eq!(p.tokens[0].visible_text(), "A");
        assert_eq!(p.tokens[0].logical_len, 1);

        assert!(
            matches!(&p.tokens[1].kind, crate::model::TokenKind::Control { kind } if kind == "dutmal")
        );
        assert_eq!(p.tokens[1].logical_len, 8);

        assert_eq!(p.tokens[2].visible_text(), "B");
        assert_eq!(p.tokens[2].logical_len, 1);

        assert!(
            matches!(&p.tokens[3].kind, crate::model::TokenKind::Control { kind } if kind == "compose")
        );
        assert_eq!(p.tokens[3].logical_len, 8);

        assert_eq!(p.tokens[4].visible_text(), "C");
        assert_eq!(p.tokens[4].logical_len, 1);

        // Cumulative logical positions: A at 0, dutmal at 1, B at 9, compose at 10, C at 18
        let offsets: Vec<usize> = p
            .tokens
            .iter()
            .scan(0, |pos, tok| {
                let start = *pos;
                *pos += tok.logical_len;
                Some(start)
            })
            .collect();
        assert_eq!(offsets, vec![0, 1, 9, 10, 18]);
        assert_eq!(
            offsets.last().unwrap() + p.tokens.last().unwrap().logical_len,
            19
        );
    }

    #[test]
    fn body_paragraph_ignores_col_pr_inside_sublist_cell() {
        let xml = r#"<hp:p id="1"><hp:run><hp:tbl id="t1" rowCnt="1" colCnt="1"><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p id="2"><hp:run><hp:ctrl><hp:colPr type="NEWSPAPER" colCount="2" sameGap="0"/></hp:ctrl><hp:t>inside cell</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert_eq!(p.columns, None);
    }

    #[test]
    fn body_paragraph_accepts_direct_col_pr() {
        let xml = r#"<hp:p id="1"><hp:run><hp:ctrl><hp:colPr type="NEWSPAPER" colCount="2" sameGap="0"/></hp:ctrl><hp:t>direct column</hp:t></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert_eq!(p.columns, Some((2, 0)));
    }

    #[test]
    fn body_paragraph_accepts_new_num_inside_sublist_cell() {
        let xml = r#"<hp:p id="1"><hp:run><hp:tbl id="t1" rowCnt="1" colCnt="1"><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p id="2"><hp:run><hp:ctrl><hp:newNum numType="PAGE" num="1"/></hp:ctrl><hp:t>title</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert_eq!(p.page_number_restart, Some(1));
    }

    #[test]
    fn body_paragraph_accepts_direct_page_hiding() {
        let xml = r#"<hp:p id="1"><hp:run><hp:ctrl><hp:pageHiding hidePageNum="1"/></hp:ctrl><hp:t>cover</hp:t></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert!(p.hides_page_number);
    }

    #[test]
    fn body_paragraph_accepts_page_hiding_inside_cell() {
        let xml = r#"<hp:p id="1"><hp:run><hp:tbl id="t1" rowCnt="1" colCnt="1"><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p id="2"><hp:run><hp:ctrl><hp:pageHiding hidePageNum="1"/></hp:ctrl><hp:t>table content</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert!(p.hides_page_number);
    }

    #[test]
    fn body_paragraph_accepts_page_hiding_inside_draw_text() {
        let xml = r#"<hp:p id="1"><hp:run><hp:rect id="r1"><hp:drawText><hp:subList><hp:p id="2"><hp:run><hp:ctrl><hp:pageHiding hidePageNum="1"/></hp:ctrl><hp:t>text box</hp:t></hp:run></hp:p></hp:subList></hp:drawText></hp:rect></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert!(p.hides_page_number);
    }

    #[test]
    fn body_paragraph_ignores_page_hiding_without_hide_page_num() {
        let xml = r#"<hp:p id="1"><hp:run><hp:ctrl><hp:pageHiding hideHeader="1" hideFooter="1" hidePageNum="0"/></hp:ctrl><hp:t>other hiding</hp:t></hp:run></hp:p>"#;
        let parsed = section(xml);
        let Some(Block::Paragraph(p)) = parsed.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert!(!p.hides_page_number);
    }

    #[test]
    fn body_paragraph_accepts_page_number_control() {
        let xml_with = r#"<hp:p id="1"><hp:run><hp:ctrl><hp:pageNum pos="BOTTOM_CENTER"/></hp:ctrl><hp:t>starts here</hp:t></hp:run></hp:p>"#;
        let parsed_with = section(xml_with);
        let Some(Block::Paragraph(p_with)) = parsed_with.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert!(p_with.page_number_control);

        let xml_without =
            r#"<hp:p id="1"><hp:run><hp:t>no page num control</hp:t></hp:run></hp:p>"#;
        let parsed_without = section(xml_without);
        let Some(Block::Paragraph(p_without)) = parsed_without.section.blocks.first() else {
            panic!("expected host paragraph");
        };
        assert!(!p_without.page_number_control);
    }
}
