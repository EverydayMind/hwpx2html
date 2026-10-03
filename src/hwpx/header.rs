use std::collections::BTreeMap;

use crate::error::Result;
use crate::hwpx::util::{
    attr_bool, attr_i64, attr_string, attr_u32, child, children, descendants, parse_xml,
};
use crate::model::{BorderStroke, CharStyle, ParaStyle};

#[derive(Debug, Clone, Default)]
pub struct BorderFill {
    pub strokes: [BorderStroke; 4],
    pub border_visible: bool,
    pub left_width: i64,
    pub right_width: i64,
    pub top_width: i64,
    pub bottom_width: i64,
    pub fill_color: Option<String>,
    pub gradient: Vec<String>,
    pub gradient_step: u32,
    pub gradient_angle: i64,
    pub diagonal_forward: bool,
    pub diagonal_backward: bool,
    pub diagonal_stroke: Option<BorderStroke>,
    pub diagonal_width: i64,
}

#[derive(Debug, Clone, Default)]
pub struct HeaderStyles {
    pub char_styles: BTreeMap<u32, CharStyle>,
    pub para_styles: BTreeMap<u32, ParaStyle>,
    pub fonts: BTreeMap<(String, u32), String>,
    pub border_fills: BTreeMap<u32, BorderFill>,
    pub page_number_style_char_id: Option<u32>,
    /// `hh:style/@id` -> its raw `@charPrIDRef`. A run child that names a
    /// document style (`dutmal/@styleIDRef`) takes that style's letters.
    pub style_char_refs: BTreeMap<u32, u32>,
    pub numberings: BTreeMap<u32, crate::model::Numbering>,
    /// Raw charPr ids that are byte-identical to an earlier charPr except
    /// for which font id one or more scripts reference, where those font
    /// ids name the exact same face -- see `detect_font_alias_char_styles`.
    /// Consumed once by `compact_char_styles`, then left empty.
    pub char_style_font_aliases: BTreeMap<u32, u32>,
    char_style_renumber: BTreeMap<u32, u32>,
    para_style_renumber: BTreeMap<u32, u32>,
}

impl HeaderStyles {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let xml = parse_xml(bytes, "Contents/header.xml")?;
        let root = xml.root_element();
        let mut styles = Self::default();

        for fontface in descendants(root, "fontface") {
            let language = attr_string(fontface, "lang").to_ascii_lowercase();
            for font in crate::hwpx::util::children(fontface, "font") {
                styles.fonts.insert(
                    (language.clone(), attr_u32(font, "id", 0)),
                    attr_string(font, "face"),
                );
            }
        }

        for char_pr in descendants(root, "charPr") {
            let id = attr_u32(char_pr, "id", styles.char_styles.len() as u32);
            let height = attr_i64(char_pr, "height", 1000);
            let color = normalize_color(&attr_string(char_pr, "textColor"));
            let background = normalize_optional_color(&attr_string(char_pr, "shadeColor"));
            let font_ref = child(char_pr, "fontRef");
            let font_id = font_ref
                .map(|node| attr_u32(node, "hangul", 0))
                .unwrap_or(0);
            let font_family = styles
                .fonts
                .get(&("hangul".to_owned(), font_id))
                .cloned()
                .unwrap_or_else(|| "바탕".to_owned());
            let latin_font_id = font_ref
                .map(|node| attr_u32(node, "latin", font_id))
                .unwrap_or(font_id);
            let latin_font_family = styles
                .fonts
                .get(&("latin".to_owned(), latin_font_id))
                .cloned()
                .unwrap_or_default();
            let ratio = child(char_pr, "ratio")
                .map(|node| attr_i64(node, "hangul", 100))
                .unwrap_or(100);
            let spacing = child(char_pr, "spacing")
                .map(|node| attr_i64(node, "hangul", 0))
                .unwrap_or(0);
            let offset = child(char_pr, "offset")
                .map(|node| attr_i64(node, "hangul", 0))
                .unwrap_or(0);
            let underline = child(char_pr, "underline");
            let strikeout = child(char_pr, "strikeout");
            styles.char_styles.insert(
                id,
                CharStyle {
                    id,
                    font_family,
                    latin_font_family,
                    font_size_hwp: height,
                    color,
                    background,
                    bold: child(char_pr, "bold").is_some(),
                    italic: child(char_pr, "italic").is_some(),
                    emboss: child(char_pr, "emboss").is_some(),
                    superscript: child(char_pr, "supscript").is_some(),
                    subscript: child(char_pr, "subscript").is_some(),
                    underline: underline.is_some_and(is_real_underline),
                    underline_color: underline
                        .filter(|node| is_real_underline(*node))
                        .map(|node| normalize_color(&attr_string(node, "color"))),
                    strike: strikeout.is_some_and(is_real_strikeout),
                    strike_color: strikeout
                        .filter(|node| is_real_strikeout(*node))
                        .map(|node| normalize_color(&attr_string(node, "color"))),
                    baseline_offset: offset,
                    ratio,
                    spacing,
                    use_font_space: attr_bool(char_pr, "useFontSpace"),
                },
            );
        }

        for numbering in descendants(root, "numbering") {
            let levels = crate::hwpx::util::children(numbering, "paraHead")
                .map(|head| crate::model::NumberingLevel {
                    level: attr_u32(head, "level", 1),
                    start: attr_u32(head, "start", 1),
                    format: attr_string(head, "numFormat"),
                    text: crate::hwpx::util::text_content(head),
                })
                .collect();
            styles.numberings.insert(
                attr_u32(numbering, "id", 0),
                crate::model::Numbering {
                    start: attr_i64(numbering, "start", 1),
                    levels,
                },
            );
        }

        let bullets = descendants(root, "bullet")
            .filter_map(|bullet| {
                let id = attr_u32(bullet, "id", 0);
                let marker = attr_string(bullet, "char");
                (!marker.is_empty()).then_some((id, marker))
            })
            .collect::<BTreeMap<_, _>>();

        for para_pr in descendants(root, "paraPr") {
            let id = attr_u32(para_pr, "id", styles.para_styles.len() as u32);
            let align = child(para_pr, "align")
                .map(|node| attr_string(node, "horizontal").to_ascii_lowercase())
                .unwrap_or_else(|| "left".to_owned());
            // HWPX 2016 documents commonly put the effective margin inside
            // hp:switch/hh:paraPr rather than directly under paraPr. The
            // first margin branch is the HwpUnitChar branch used by the
            // reference HTML exporter.
            let direct_margin = child(para_pr, "margin");
            let margin = direct_margin.or_else(|| descendants(para_pr, "margin").next());
            let value = |name: &str| {
                margin
                    .and_then(|node| crate::hwpx::util::children(node, name).next())
                    .map(|node| attr_i64(node, "value", 0))
                    .unwrap_or(0)
            };
            // The legacy direct hh:margin form stores the visual hanging
            // indent and the left/right margins doubled; the HwpUnitChar
            // switch form stores the effective value directly. 산업융합
            // 규제샌드박스 공고문's paraPr left=600 puts its text at 300
            // (every lineseg horzpos) and its PARA-relative table 300 in, so
            // the table sits at 22.06mm in the reference, not 23.11mm.
            let effective = |value: i64| {
                if direct_margin.is_some() {
                    value / 2
                } else {
                    value
                }
            };
            let hanging_indent = margin
                .and_then(|node| child(node, "intent"))
                .map(|node| effective((-attr_i64(node, "value", 0)).max(0)))
                .unwrap_or(0);
            let break_before = child(para_pr, "breakSetting")
                .map(|node| attr_bool(node, "pageBreakBefore"))
                .unwrap_or(false);
            let bullet_marker = child(para_pr, "heading")
                .filter(|heading| attr_string(*heading, "type").eq_ignore_ascii_case("BULLET"))
                .and_then(|heading| bullets.get(&attr_u32(heading, "idRef", 0)).cloned());
            let heading = child(para_pr, "heading").and_then(|heading| {
                let level = attr_u32(heading, "level", 0);
                match attr_string(heading, "type").to_ascii_uppercase().as_str() {
                    "OUTLINE" => Some(crate::model::ParaHeading::Outline(level)),
                    "NUMBER" => Some(crate::model::ParaHeading::Number(level)),
                    "BULLET" => Some(crate::model::ParaHeading::Bullet(level)),
                    _ => None,
                }
            });
            styles.para_styles.insert(
                id,
                ParaStyle {
                    id,
                    align,
                    hanging_indent,
                    margin_left: effective(value("left")),
                    margin_right: effective(value("right")),
                    margin_before: value("prev"),
                    margin_after: value("next"),
                    page_break_before: break_before,
                    condense: attr_i64(para_pr, "condense", 0).clamp(0, 100),
                    bullet_marker,
                    heading,
                    heading_ref: child(para_pr, "heading")
                        .map(|heading| attr_u32(heading, "idRef", 0))
                        .unwrap_or(0),
                },
            );
        }
        for border_fill in descendants(root, "borderFill") {
            let id = attr_u32(border_fill, "id", styles.border_fills.len() as u32 + 1);
            let border_width = |name: &str| {
                child(border_fill, name)
                    .filter(|border| !attr_string(*border, "type").eq_ignore_ascii_case("NONE"))
                    .map(|border| border_width_hwp(&attr_string(border, "width")))
                    .unwrap_or(0)
            };
            let left_width = border_width("leftBorder");
            let right_width = border_width("rightBorder");
            let top_width = border_width("topBorder");
            let bottom_width = border_width("bottomBorder");
            let border_visible =
                left_width > 0 || right_width > 0 || top_width > 0 || bottom_width > 0;
            let fill_color = descendants(border_fill, "winBrush")
                .next()
                .and_then(|brush| normalize_optional_color(&attr_string(brush, "faceColor")));
            // Mirrors the shape `fillBrush/gradation` parsing in section.rs
            // (`ShapeStyle::gradient`/`gradient_step`/`gradient_angle`) --
            // the source only ever sets one of `winBrush`/`gradation` on a
            // given `fillBrush`.
            let gradation = descendants(border_fill, "gradation").next();
            let gradient = gradation
                .into_iter()
                .flat_map(|gradient| children(gradient, "color"))
                .map(|color| attr_string(color, "value"))
                .collect::<Vec<_>>();
            let gradient_step = gradation.map_or(0, |gradient| attr_u32(gradient, "step", 0));
            let gradient_angle = gradation.map_or(0, |gradient| attr_i64(gradient, "angle", 0));
            let has_slant = |name: &str| {
                child(border_fill, name)
                    .is_some_and(|node| !attr_string(node, "type").eq_ignore_ascii_case("NONE"))
            };
            let diagonal_forward = has_slant("slash");
            let diagonal_backward = has_slant("backSlash");
            let diagonal = child(border_fill, "diagonal")
                .filter(|node| !attr_string(*node, "type").eq_ignore_ascii_case("NONE"))
                .map(|node| {
                    let width = border_width_hwp(&attr_string(node, "width"));
                    // HWP's own exporter bumps a 0.1mm diagonal stroke up to
                    // 0.30mm at render time -- verified against 성과보고서's
                    // "직급별 구분" header cell and, independently, two more
                    // 0.1mm-declared cells in 4.(산업부공고 제2026-561호):
                    // all three render at 0.30mm instead of their declared
                    // width. A declared 0.12mm diagonal (also verified, three
                    // more cells across the same two documents) passes
                    // through unchanged. No other declared width is
                    // confirmed yet; only this one exact bump is applied.
                    let width = if width == border_width_hwp("0.1mm") {
                        border_width_hwp("0.3mm")
                    } else {
                        width
                    };
                    let stroke = BorderStroke {
                        kind: attr_string(node, "type"),
                        color: normalize_optional_color(&attr_string(node, "color"))
                            .unwrap_or_else(|| "#000000".to_owned()),
                    };
                    (stroke, width)
                });
            let (diagonal_stroke, diagonal_width) = match diagonal {
                Some((stroke, width)) => (Some(stroke), width),
                None => (None, 0),
            };
            styles.border_fills.insert(
                id,
                BorderFill {
                    strokes: ["leftBorder", "rightBorder", "topBorder", "bottomBorder"].map(
                        |name| {
                            child(border_fill, name).map_or_else(BorderStroke::default, |border| {
                                BorderStroke {
                                    kind: attr_string(border, "type"),
                                    color: normalize_optional_color(&attr_string(border, "color"))
                                        .unwrap_or_else(|| "#000000".to_owned()),
                                }
                            })
                        },
                    ),
                    border_visible,
                    left_width,
                    right_width,
                    top_width,
                    bottom_width,
                    fill_color,
                    gradient,
                    gradient_step,
                    gradient_angle,
                    // Neither diagonal actually shows without a real,
                    // non-NONE `<diagonal>` stroke -- verified against
                    // 4.(산업부공고 제2026-561호)'s own borderFill id 80,
                    // which declares `backSlash type="CENTER"` but no
                    // `<diagonal>` element at all: none of its cells draw a
                    // line in the reference.
                    diagonal_forward: diagonal_forward && diagonal_stroke.is_some(),
                    diagonal_backward: diagonal_backward && diagonal_stroke.is_some(),
                    diagonal_stroke,
                    diagonal_width,
                },
            );
        }

        // HWP records the page-number widget's own character style as a
        // named document style (engName "Page Number", Korean name "쪽
        // 번호") rather than on whatever run happens to embed the pageNum
        // control. That run's charPrIDRef belongs to the surrounding body
        // text, not the number glyph, and never matches the reference's
        // rendered class; see AGENTS.md's page-number style entry. The
        // style's own `type` varies by document -- PARA in some, CHAR in
        // others (2022회계연도 성과보고서 names it as a CHAR style) -- so
        // both are accepted; only `charPrIDRef` is ever read from it.
        styles.page_number_style_char_id = descendants(root, "style")
            .find(|node| {
                let kind = attr_string(*node, "type");
                (kind.eq_ignore_ascii_case("PARA") || kind.eq_ignore_ascii_case("CHAR"))
                    && (attr_string(*node, "engName") == "Page Number"
                        || attr_string(*node, "name") == "쪽 번호")
            })
            .map(|node| attr_u32(node, "charPrIDRef", 0));

        styles.style_char_refs = descendants(root, "style")
            .filter(|node| node.has_attribute("id") && node.has_attribute("charPrIDRef"))
            .map(|node| (attr_u32(node, "id", 0), attr_u32(node, "charPrIDRef", 0)))
            .collect();

        styles.char_style_font_aliases = detect_font_alias_char_styles(root, &styles.fonts);

        Ok(styles)
    }

    /// The character style, as the layout numbers them, of the document
    /// style `style_ref` (`hh:style/@id`); `None` for a style the header
    /// does not define.
    pub fn style_char_style_id(&self, style_ref: u32) -> Option<u32> {
        self.style_char_refs
            .get(&style_ref)
            .map(|&raw| self.resolve_char_style_id(raw))
    }

    pub fn char_style(&self, id: u32) -> CharStyle {
        self.char_styles
            .get(&id)
            .cloned()
            .unwrap_or_else(|| CharStyle {
                id,
                font_family: "바탕".to_owned(),
                font_size_hwp: 1000,
                color: "#000000".to_owned(),
                ..CharStyle::default()
            })
    }

    pub fn para_style(&self, id: u32) -> ParaStyle {
        self.para_styles
            .get(&id)
            .cloned()
            .unwrap_or_else(|| ParaStyle {
                id,
                align: "left".to_owned(),
                hanging_indent: 0,
                ..ParaStyle::default()
            })
    }

    pub fn border_fill(&self, id: u32) -> BorderFill {
        self.border_fills.get(&id).cloned().unwrap_or_default()
    }

    /// Remove the given raw charPr ids from the style pool and compact the
    /// remaining ids into a contiguous sequence, preserving order. Every
    /// later lookup of a raw `charPrIDRef` must go through
    /// [`resolve_char_style_id`](Self::resolve_char_style_id) to land on the
    /// compacted id. A no-op when both arguments are empty, which is the
    /// case for every document without the specific redundant shapes this
    /// exists for.
    ///
    /// `excluded` ids (see `collect_redundant_page_number_styles`) are
    /// dropped outright; callers only ever reach one through a run with no
    /// visible content, so which surviving id its raw id happens to land on
    /// is harmless. `aliases` (raw duplicate id -> raw canonical id, see
    /// `detect_font_alias_char_styles`) are also dropped from the pool, but
    /// resolve to wherever their canonical id ends up -- that duplicate id
    /// can be driving real visible runs, so it must land on the exact same
    /// compacted id as its canonical, not an arbitrary neighboring slot.
    pub fn compact_char_styles(
        &mut self,
        excluded: &std::collections::BTreeSet<u32>,
        aliases: &BTreeMap<u32, u32>,
    ) {
        if excluded.is_empty() && aliases.is_empty() {
            return;
        }
        let mut next_id = 0u32;
        let mut renumber = BTreeMap::new();
        let mut compacted = BTreeMap::new();
        for (raw_id, mut style) in std::mem::take(&mut self.char_styles) {
            if excluded.contains(&raw_id) {
                renumber.insert(raw_id, next_id);
                continue;
            }
            if aliases.contains_key(&raw_id) {
                continue;
            }
            renumber.insert(raw_id, next_id);
            style.id = next_id;
            compacted.insert(next_id, style);
            next_id += 1;
        }
        for (&raw_id, &canonical) in aliases {
            let target = renumber.get(&canonical).copied().unwrap_or(canonical);
            renumber.insert(raw_id, target);
        }
        self.char_styles = compacted;
        self.char_style_renumber = renumber;
        if let Some(page_style_id) = self.page_number_style_char_id {
            self.page_number_style_char_id = Some(self.resolve_char_style_id(page_style_id));
        }
    }

    pub fn resolve_char_style_id(&self, raw: u32) -> u32 {
        self.char_style_renumber.get(&raw).copied().unwrap_or(raw)
    }

    /// Same shape as `compact_char_styles`'s `aliases` handling: an aliased
    /// paraPr is dropped from the numbered pool and resolves to wherever its
    /// canonical id lands, since (unlike a page-number-only exclusion) it
    /// can still be driving a real paragraph's rendered class. See
    /// `collect_duplicate_picture_only_para_styles` for how `aliases` is
    /// derived -- a no-op (empty map) for every document without that exact
    /// shape.
    pub fn compact_para_styles(&mut self, aliases: &BTreeMap<u32, u32>) {
        if aliases.is_empty() {
            return;
        }
        let mut next_id = 0u32;
        let mut renumber = BTreeMap::new();
        let mut compacted = BTreeMap::new();
        for (raw_id, mut style) in std::mem::take(&mut self.para_styles) {
            if aliases.contains_key(&raw_id) {
                continue;
            }
            renumber.insert(raw_id, next_id);
            style.id = next_id;
            compacted.insert(next_id, style);
            next_id += 1;
        }
        for (&raw_id, &canonical) in aliases {
            let target = renumber.get(&canonical).copied().unwrap_or(canonical);
            renumber.insert(raw_id, target);
        }
        self.para_styles = compacted;
        self.para_style_renumber = renumber;
    }

    pub fn resolve_para_style_id(&self, raw: u32) -> u32 {
        self.para_style_renumber.get(&raw).copied().unwrap_or(raw)
    }
}

/// HWPX exporters may emit `<hh:strikeout shape="3D"/>` as a placeholder for
/// a character style without an actual strikeout. The shape is therefore
/// semantic data, not merely an element-presence flag.
fn is_real_strikeout(node: roxmltree::Node<'_, '_>) -> bool {
    matches!(
        attr_string(node, "shape").to_ascii_uppercase().as_str(),
        "SOLID"
            | "DASH"
            | "DOT"
            | "DASH_DOT"
            | "DASH_DOT_DOT"
            | "LONG_DASH"
            | "CIRCLE"
            | "DOUBLE_SLIM"
            | "SLIM_THICK"
            | "THICK_SLIM"
            | "SLIM_THICK_SLIM"
            | "WAVE"
            | "DOUBLE_WAVE"
    )
}

/// `type="NONE"` is commonly emitted for styles without an underline.
/// Only the bottom underline is represented by CSS `text-decoration`.
fn is_real_underline(node: roxmltree::Node<'_, '_>) -> bool {
    attr_string(node, "type").eq_ignore_ascii_case("BOTTOM")
}

fn normalize_color(value: &str) -> String {
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        "#000000".to_owned()
    } else if value.starts_with('#')
        && value.len() == 7
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        value.to_owned()
    } else {
        "#000000".to_owned()
    }
}

fn normalize_optional_color(value: &str) -> Option<String> {
    if value.is_empty() || value.eq_ignore_ascii_case("none") {
        None
    } else if value.starts_with('#')
        && value.len() == 7
        && value[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn border_width_hwp(value: &str) -> i64 {
    let value = value.trim().to_ascii_lowercase();
    let value = value.strip_suffix("mm").unwrap_or(&value).trim();
    let (whole, fraction, scale) = match value.split_once('.') {
        Some((whole, fraction)) => {
            let digits = fraction
                .chars()
                .filter(|character| character.is_ascii_digit())
                .collect::<String>();
            if digits.is_empty() {
                (whole, 0_i128, 1_i128)
            } else {
                let scale = 10_i128.pow(digits.len() as u32);
                (whole, digits.parse::<i128>().unwrap_or(0), scale)
            }
        }
        None => (value, 0_i128, 1_i128),
    };
    let whole = whole.parse::<i128>().unwrap_or(0);
    let sign = if whole < 0 { -1_i128 } else { 1_i128 };
    let magnitude = whole.unsigned_abs() as i128 * scale + fraction;
    // HWPUNIT per millimetre = 7200 / 25.4 = 360000 / 1270.
    let units = crate::layout::round_div(magnitude * 360_000, scale * 1_270);
    sign as i64 * units
}

const CHAR_PR_SCRIPTS: [&str; 7] = [
    "hangul", "latin", "hanja", "japanese", "other", "symbol", "user",
];

/// A charPr's font id for each script, read straight from its `fontRef`
/// child (0 when absent, matching the child's own absence on both sides of
/// any comparison -- this never causes two charPr with a genuinely
/// different font reference to look alike).
fn char_pr_font_ref_ids(node: roxmltree::Node<'_, '_>) -> [u32; CHAR_PR_SCRIPTS.len()] {
    let font_ref = child(node, "fontRef");
    CHAR_PR_SCRIPTS.map(|script| font_ref.map_or(0, |node| attr_u32(node, script, 0)))
}

/// Every other charPr field (all top-level attributes but `id`, and every
/// child but `fontRef`) as an order- and attribute-order-independent
/// string, so two charPr differing only in which font id they reference
/// compare equal here.
fn char_pr_signature(node: roxmltree::Node<'_, '_>) -> String {
    let mut attrs = node
        .attributes()
        .filter(|attribute| attribute.name() != "id")
        .map(|attribute| format!("{}={}", attribute.name(), attribute.value()))
        .collect::<Vec<_>>();
    attrs.sort();
    let mut signature = attrs.join("|");
    for element in node.children().filter(|node| node.is_element()) {
        let tag = element.tag_name().name();
        if tag == "fontRef" {
            continue;
        }
        let mut child_attrs = element
            .attributes()
            .map(|attribute| format!("{}={}", attribute.name(), attribute.value()))
            .collect::<Vec<_>>();
        child_attrs.sort();
        signature.push(';');
        signature.push_str(tag);
        for attribute in child_attrs {
            signature.push(':');
            signature.push_str(&attribute);
        }
    }
    signature
}

/// Finds charPr definitions that are byte-identical to an earlier charPr in
/// every field except which font id one or more scripts reference, where
/// those font ids name the exact same face in this document's font table.
/// HWPX documents can carry two registrations of what is visually the same
/// font (e.g. an HFT substitute alongside the real TTF entry); a charPr
/// built against the redundant registration renders identically to one
/// built against the other, and the reference HTML exporter gives it the
/// earlier charPr's class instead of a class of its own. This differs from
/// merging by fully-resolved CSS content (rejected before -- see AGENTS.md
/// -- because it conflated unrelated styles our own CSS generator happens
/// to render alike): every other charPr field, not just the parts that
/// currently reach CSS, must match exactly, and the only excuse for a font
/// id mismatch is that both ids name one identical face.
///
/// Confirmed via a full 23-sample scan (see AGENTS.md) to occur only in
/// 0103's two pairs (id 45 aliasing 40, id 99 aliasing 30); every other
/// sample's scan is empty, so this is a no-op everywhere else.
fn detect_font_alias_char_styles(
    root: roxmltree::Node<'_, '_>,
    fonts: &BTreeMap<(String, u32), String>,
) -> BTreeMap<u32, u32> {
    let mut entries = descendants(root, "charPr")
        .map(|node| {
            (
                attr_u32(node, "id", 0),
                char_pr_signature(node),
                char_pr_font_ref_ids(node),
            )
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(id, ..)| *id);

    let mut aliases = BTreeMap::new();
    for (index, (id_a, signature_a, fonts_a)) in entries.iter().enumerate() {
        if aliases.contains_key(id_a) {
            continue;
        }
        for (id_b, signature_b, fonts_b) in &entries[index + 1..] {
            if signature_a != signature_b || fonts_a == fonts_b {
                // Identical fontRef alongside identical everything else
                // means an exact duplicate charPr declaration, a different
                // situation this function does not attempt to explain.
                continue;
            }
            let same_face = CHAR_PR_SCRIPTS.iter().zip(fonts_a).zip(fonts_b).all(
                |((script, font_a), font_b)| {
                    font_a == font_b
                        || fonts
                            .get(&(script.to_string(), *font_a))
                            .zip(fonts.get(&(script.to_string(), *font_b)))
                            .is_some_and(|(face_a, face_b)| face_a == face_b)
                },
            );
            if same_face {
                aliases.insert(*id_b, *id_a);
            }
        }
    }
    aliases
}

#[cfg(test)]
mod tests {
    use super::{child, descendants, is_real_strikeout, is_real_underline};

    #[test]
    fn placeholder_strikeout_shape_is_not_rendered() {
        let document = roxmltree::Document::parse(
            r#"<root xmlns:hh="urn:test">
                <hh:charPr id="0"><hh:strikeout shape="3D"/></hh:charPr>
                <hh:charPr id="1"><hh:strikeout shape="SOLID"/></hh:charPr>
                <hh:charPr id="2"><hh:strikeout shape="NONE"/></hh:charPr>
            </root>"#,
        )
        .expect("valid XML");
        let chars = descendants(document.root_element(), "charPr").collect::<Vec<_>>();

        assert!(!is_real_strikeout(
            child(chars[0], "strikeout").expect("strikeout")
        ));
        assert!(is_real_strikeout(
            child(chars[1], "strikeout").expect("strikeout")
        ));
        assert!(!is_real_strikeout(
            child(chars[2], "strikeout").expect("strikeout")
        ));
    }

    #[test]
    fn none_underline_type_is_not_rendered() {
        let document = roxmltree::Document::parse(
            r#"<root xmlns:hh="urn:test">
                <hh:charPr id="0"><hh:underline type="NONE"/></hh:charPr>
                <hh:charPr id="1"><hh:underline type="BOTTOM"/></hh:charPr>
            </root>"#,
        )
        .expect("valid XML");
        let chars = descendants(document.root_element(), "charPr").collect::<Vec<_>>();

        assert!(!is_real_underline(
            child(chars[0], "underline").expect("underline")
        ));
        assert!(is_real_underline(
            child(chars[1], "underline").expect("underline")
        ));
    }

    #[test]
    fn page_number_style_id_comes_from_the_named_style_not_charpr_order() {
        let xml = r#"<hh:head xmlns:hh="urn:test">
            <hh:refList>
                <hh:fontfaces>
                    <hh:fontface lang="HANGUL">
                        <hh:font id="0" face="바탕"/>
                    </hh:fontface>
                </hh:fontfaces>
                <hh:charProperties>
                    <hh:charPr id="0" height="1300"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="79" height="1000"><hh:fontRef hangul="0"/></hh:charPr>
                </hh:charProperties>
                <hh:styles>
                    <hh:style id="9" type="PARA" name="쪽 번호" engName="Page Number" charPrIDRef="79"/>
                </hh:styles>
            </hh:refList>
        </hh:head>"#;
        let styles = super::HeaderStyles::parse(xml.as_bytes()).expect("parse header");
        assert_eq!(styles.page_number_style_char_id, Some(79));
    }

    #[test]
    fn page_number_style_id_is_found_when_named_style_is_char_type() {
        // 2022회계연도 성과보고서 names its page-number style as a CHAR
        // style (id 9, charPrIDRef 83) rather than PARA -- both occur in
        // the wild, so the lookup must accept either.
        let xml = r#"<hh:head xmlns:hh="urn:test">
            <hh:refList>
                <hh:fontfaces>
                    <hh:fontface lang="HANGUL">
                        <hh:font id="0" face="바탕"/>
                    </hh:fontface>
                </hh:fontfaces>
                <hh:charProperties>
                    <hh:charPr id="0" height="1300"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="83" height="1000"><hh:fontRef hangul="0"/></hh:charPr>
                </hh:charProperties>
                <hh:styles>
                    <hh:style id="9" type="CHAR" name="쪽 번호" engName="Page Number" paraPrIDRef="0" charPrIDRef="83"/>
                </hh:styles>
            </hh:refList>
        </hh:head>"#;
        let styles = super::HeaderStyles::parse(xml.as_bytes()).expect("parse header");
        assert_eq!(styles.page_number_style_char_id, Some(83));
    }

    #[test]
    fn compacting_char_styles_shifts_only_ids_past_the_excluded_one() {
        let xml = r#"<hh:head xmlns:hh="urn:test">
            <hh:refList>
                <hh:fontfaces>
                    <hh:fontface lang="HANGUL">
                        <hh:font id="0" face="바탕"/>
                    </hh:fontface>
                </hh:fontfaces>
                <hh:charProperties>
                    <hh:charPr id="0" height="1000"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="1" height="1100"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="2" height="1200"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="3" height="1300"><hh:fontRef hangul="0"/></hh:charPr>
                </hh:charProperties>
                <hh:styles>
                    <hh:style id="9" type="PARA" name="쪽 번호" engName="Page Number" charPrIDRef="3"/>
                </hh:styles>
            </hh:refList>
        </hh:head>"#;
        let mut styles = super::HeaderStyles::parse(xml.as_bytes()).expect("parse header");
        styles.compact_char_styles(&[2].into_iter().collect(), &Default::default());

        assert_eq!(styles.resolve_char_style_id(0), 0);
        assert_eq!(styles.resolve_char_style_id(1), 1);
        assert_eq!(styles.resolve_char_style_id(3), 2);
        assert_eq!(styles.char_styles.len(), 3);
        assert_eq!(styles.char_style(2).font_size_hwp, 1300);
        // The named page-number style pointed at the now-removed id's
        // successor; it must follow the same renumbering.
        assert_eq!(styles.page_number_style_char_id, Some(2));
    }

    #[test]
    fn space_width_and_condense_attributes_are_read() {
        // 0713AI 도입: charPr 23 (useFontSpace="0") and paraPr 67
        // (condense="75"), with 39 standing in for a useFontSpace="1" shape.
        let xml = r#"<hh:head xmlns:hh="urn:test">
            <hh:refList>
                <hh:charProperties>
                    <hh:charPr id="23" height="1500" useFontSpace="0"/>
                    <hh:charPr id="39" height="1500" useFontSpace="1"/>
                </hh:charProperties>
                <hh:paraProperties>
                    <hh:paraPr id="67" condense="75"/>
                    <hh:paraPr id="69" condense="0"/>
                </hh:paraProperties>
            </hh:refList>
        </hh:head>"#;
        let styles = super::HeaderStyles::parse(xml.as_bytes()).expect("parse header");

        assert!(!styles.char_style(23).use_font_space);
        assert!(styles.char_style(39).use_font_space);
        assert_eq!(styles.para_style(67).condense, 75);
        assert_eq!(styles.para_style(69).condense, 0);
    }

    #[test]
    fn compacting_with_no_excluded_ids_is_a_no_op() {
        let mut styles = super::HeaderStyles::default();
        styles.char_styles.insert(
            0,
            crate::model::CharStyle {
                id: 0,
                ..crate::model::CharStyle::default()
            },
        );
        styles.compact_char_styles(&Default::default(), &Default::default());
        assert_eq!(styles.resolve_char_style_id(0), 0);
        assert_eq!(styles.char_styles.len(), 1);
    }

    #[test]
    fn font_alias_char_styles_merge_a_duplicate_font_registration_but_not_a_real_difference() {
        // charPr 1 and 2 differ only by hangul font id (0 vs 1), and fonts 0
        // and 1 name the same face -- a duplicate HFT/TTF registration, like
        // 0103's real case. charPr 3 also differs only by hangul font id (0
        // vs 2), but font 2 names a different face, so it must NOT merge.
        let xml = r#"<hh:head xmlns:hh="urn:test">
            <hh:refList>
                <hh:fontfaces>
                    <hh:fontface lang="HANGUL">
                        <hh:font id="0" face="휴먼명조"/>
                        <hh:font id="1" face="휴먼명조"/>
                        <hh:font id="2" face="다른글꼴"/>
                    </hh:fontface>
                </hh:fontfaces>
                <hh:charProperties>
                    <hh:charPr id="0" height="1000"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="1" height="1500"><hh:fontRef hangul="0"/></hh:charPr>
                    <hh:charPr id="2" height="1500"><hh:fontRef hangul="1"/></hh:charPr>
                    <hh:charPr id="3" height="1500"><hh:fontRef hangul="2"/></hh:charPr>
                </hh:charProperties>
            </hh:refList>
        </hh:head>"#;
        let mut styles = super::HeaderStyles::parse(xml.as_bytes()).expect("parse header");
        let aliases = std::mem::take(&mut styles.char_style_font_aliases);
        assert_eq!(aliases, std::collections::BTreeMap::from([(2, 1)]));

        styles.compact_char_styles(&Default::default(), &aliases);
        assert_eq!(styles.resolve_char_style_id(0), 0);
        assert_eq!(styles.resolve_char_style_id(1), 1);
        assert_eq!(styles.resolve_char_style_id(2), 1);
        assert_eq!(styles.resolve_char_style_id(3), 2);
        assert_eq!(styles.char_styles.len(), 3);
    }

    #[test]
    fn compact_para_styles_aliases_a_duplicate_and_shifts_ids_after_it() {
        let mut styles = super::HeaderStyles::default();
        for id in 0..4u32 {
            styles.para_styles.insert(
                id,
                crate::model::ParaStyle {
                    id,
                    ..crate::model::ParaStyle::default()
                },
            );
        }
        // id 2 aliases to id 0 (like 0831's paraPr 60 -> 3): id 1 keeps its
        // own slot, id 2 disappears, and id 3 shifts down to fill the gap.
        let aliases = std::collections::BTreeMap::from([(2, 0)]);
        styles.compact_para_styles(&aliases);
        assert_eq!(styles.resolve_para_style_id(0), 0);
        assert_eq!(styles.resolve_para_style_id(1), 1);
        assert_eq!(styles.resolve_para_style_id(2), 0);
        assert_eq!(styles.resolve_para_style_id(3), 2);
        assert_eq!(styles.para_styles.len(), 3);
    }

    #[test]
    fn compact_para_styles_with_no_aliases_is_a_no_op() {
        let mut styles = super::HeaderStyles::default();
        styles.para_styles.insert(
            0,
            crate::model::ParaStyle {
                id: 0,
                ..crate::model::ParaStyle::default()
            },
        );
        styles.compact_para_styles(&Default::default());
        assert_eq!(styles.resolve_para_style_id(0), 0);
        assert_eq!(styles.para_styles.len(), 1);
    }

    #[test]
    fn style_char_style_id_resolves_style_ref_to_char_style() {
        let xml = r##"<head xmlns="http://www.hancom.co.kr/hwpml/2011/head" xmlns:hh="http://www.hancom.co.kr/hwpml/2011/head">
            <hh:styles itemCnt="1">
                <hh:style id="17" type="PARA" name="Memo" engName="Memo" paraPrIDRef="0" charPrIDRef="4" nextStyleIDRef="17"/>
            </hh:styles>
            <hh:charProperties itemCnt="1">
                <hh:charPr id="4" height="900" textColor="#000000"/>
            </hh:charProperties>
        </head>"##;
        let header = super::HeaderStyles::parse(xml.as_bytes()).unwrap();
        assert_eq!(header.style_char_style_id(17), Some(4));
        assert_eq!(header.style_char_style_id(999), None);
    }
}
