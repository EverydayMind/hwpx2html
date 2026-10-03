use crate::layout::css_mm;
use crate::model::{CharStyle, ParaStyle};

const UNDERLINE_SVG_DATA_URI: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMDAlIiBoZWlnaHQ9IjEwMCUiPjxsaW5lIHgxPSIwIiB5MT0iOTQlIiB4Mj0iMTAwJSIgeTI9Ijk0JSIgc3R5bGU9InN0cm9rZTpyZ2IoMCwwLDApO3N0cm9rZS13aWR0aDowLjJtbSIvPjwvc3ZnPg==";
const STRIKE_SVG_DATA_URI: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMDAlIiBoZWlnaHQ9IjEwMCUiPjxsaW5lIHgxPSIwIiB5MT0iNTAlIiB4Mj0iMTAwJSIgeTI9IjUwJSIgc3R5bGU9InN0cm9rZTpyZ2IoMjU1LDAsMCk7c3Ryb2tlLXdpZHRoOjAuMm1tIi8+PC9zdmc+";
const BLUE_UNDERLINE_SVG_DATA_URI: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMDAlIiBoZWlnaHQ9IjEwMCUiPjxsaW5lIHgxPSIwIiB5MT0iOTQlIiB4Mj0iMTAwJSIgeTI9Ijk0JSIgc3R5bGU9InN0cm9rZTpyZ2IoMCwwLDI1NSk7c3Ryb2tlLXdpZHRoOjAuMm1tIi8+PC9zdmc+";
const BLACK_STRIKE_SVG_DATA_URI: &str = "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxMDAlIiBoZWlnaHQ9IjEwMCUiPjxsaW5lIHgxPSIwIiB5MT0iNTAlIiB4Mj0iMTAwJSIgeTI9IjUwJSIgc3R5bGU9InN0cm9rZTpyZ2IoMCwwLDApO3N0cm9rZS13aWR0aDowLjJtbSIvPjwvc3ZnPg==";

pub fn base_css() -> &'static str {
    r#"body {margin:0;padding-left:0;padding-right:0;padding-bottom:0;padding-top:2mm;}
.hce {margin:0;padding:0;position:absolute;overflow:hidden;}
.hme {margin:0;padding:0;position:absolute;}
.hhe {margin:0;padding:0;position:relative;}
.hhi {display:inline-block;margin:0;padding:0;position:relative;background-size:contain;}
.hls {margin:0;padding:0;position:absolute;}
.hfS {margin:0;padding:0;position:absolute;}
.hcD {margin:0;padding:0;position:absolute;}
.hcI {margin:0;padding:0;position:absolute;}
.hcS {margin:0;padding:0;position:absolute;}
.hfN {margin:0;padding:0;position:relative;}
.hmB {margin:0;padding:0;position:absolute;}
.hmO {margin:0;padding:0;position:absolute;}
.hmT {margin:0;padding:0;position:absolute;}
.hpN {display:inline-block;margin:0;padding:0;position:relative;white-space:nowrap;}
.htC {display:inline-block;margin:0;padding:0;position:relative;vertical-align:top;overflow:hidden;}
.haN {display:inline-block;margin:0;padding:0;position:relative;}
.hdu {margin:0;padding:0;position:relative;}
.hdS {margin:0;padding:0;position:absolute;}
.hsC {margin:0;padding:0;position:absolute;}
.hsR {margin:0;padding:0;position:absolute;}
.hsG {margin:0;padding:0;position:absolute;}
.hsL {margin:0;padding:0;position:absolute;}
.hsT {margin:0;padding:0;position:absolute;overflow:hidden;}
.hsE {margin:0;padding:0;position:absolute;overflow:hidden;}
.hsA {margin:0;padding:0;position:absolute;overflow:hidden;}
.hsP {margin:0;padding:0;position:absolute;overflow:hidden;}
.hsV {margin:0;padding:0;position:absolute;overflow:hidden;}
.hsO {margin:0;padding:0;position:absolute;}
.hsU {margin:0;padding:0;position:absolute;overflow:hidden;}
.hpi {margin:0;padding:0;position:absolute;}
.hch {margin:0;padding:0;position:absolute;}
.hcG {margin:0;padding:0;position:absolute;}
.heq {margin:0;padding:0;position:absolute;}
.heG {margin:0;padding:0;position:absolute;}
.htA {margin:0;padding:0;position:absolute;}
.hvi {margin:0;padding:0;position:absolute;}
.htb {margin:0;padding:0;position:absolute;}
.htG {margin:0;padding:0;position:absolute;}
.hfJ {margin:0;padding:0;position:absolute;}
.hfG {margin:0;padding:0;position:absolute;}
.hfB {margin:0;padding:0;position:absolute;}
.hfR {margin:0;padding:0;position:absolute;}
.hfC {margin:0;padding:0;position:absolute;}
.hfO {margin:0;padding:0;position:absolute;}
.hfL {margin:0;padding:0;position:absolute;}
.hfM {margin:0;padding:0;position:absolute;}
.hfE {margin:0;padding:0;position:absolute;}
.hpl {margin:0;padding:0;position:absolute;}
.hs {margin:0;padding:0;position:absolute;overflow:visible;}
.hpa {position:relative;padding:0;overflow:hidden;margin-left:2mm;margin-right:0mm;margin-bottom:2mm;margin-top:0mm;border:1px black solid;box-shadow:1mm 1mm 0 #AAAAAA;}
.hpa::after {content:'';position:absolute;margin:0;padding:0;left:0;right:0;top:0;bottom:0;background-color:white;z-index:-2;}
.hrt {display:inline-block;margin:0;padding:0;position:relative;white-space:inherit;line-height:1.1;}
.hco {display:inline-block;margin:0;padding:0;position:relative;white-space:inherit;}
.hcc {margin:0;padding:0;position:absolute;}
.hls {clear:both;}
"#
}

/// Rules for the structural elements. They cancel the user-agent defaults
/// of the elements that carry the positioned boxes (`p` and heading
/// margins and sizes, list indents and markers, `th` bold/centre, `a`
/// underline) so no box moves, and draw marks that are not source text
/// (tab leaders, page numbers, bullets) as generated content outside the
/// DOM text (D27).
pub fn semantic_css() -> &'static str {
    r#"p {margin:0;padding:0;}
h1,h2,h3,h4,h5,h6 {margin:0;padding:0;font-size:inherit;font-weight:inherit;}
ul,ol {margin:0;padding:0;list-style:none;}
table {border-spacing:0;}
th {font-weight:inherit;text-align:inherit;}
a {text-decoration:none;}
.htx {font-size:0;line-height:0;white-space:nowrap;}
[data-gen]::before {content:attr(data-gen);}
"#
}

/// The bar takes no room: it is laid over the top of the window, and the
/// pages sit where they would without it. Hidden, it is transparent and lets
/// the pointer through, but for a strip along the top edge (`::before`, the
/// whole bar on a touch screen). With the pointer on that strip it appears
/// over the page and stays while the pointer is on it. It also shows while
/// the head script sets `hwpx-show` (a touch on the strip, or a control
/// focused from the keyboard).
macro_rules! navigation_bar_css {
    () => {
        r#".hwpx-nav {position:fixed;left:0;right:0;top:0;z-index:2147483647;box-sizing:border-box;height:40px;display:flex;align-items:center;justify-content:center;gap:8px;background:#f3f3f3;border-bottom:1px solid #c8c8c8;color:#222;font:14px/1 system-ui,sans-serif;opacity:0;pointer-events:none;transition:opacity .15s;}
.hwpx-nav::before {content:'';position:absolute;left:0;right:0;top:0;height:12px;pointer-events:auto;}
.hwpx-nav:hover, .hwpx-nav.hwpx-show {opacity:1;pointer-events:auto;}
.hwpx-nav:hover::before, .hwpx-nav.hwpx-show::before {content:none;}
.hwpx-nav button {font:inherit;color:inherit;padding:6px 12px;border:1px solid #b4b4b4;border-radius:4px;background:#fff;cursor:pointer;}
.hwpx-nav button:disabled {opacity:0.4;cursor:default;}
.hwpx-nav input {font:inherit;width:4em;padding:4px 6px;border:1px solid #b4b4b4;border-radius:4px;text-align:right;}
@media (pointer:coarse) {.hwpx-nav::before {height:100%;}}
@media print {.hwpx-nav {display:none;}}
"#
    };
}

pub fn navigation_bar_css() -> &'static str {
    navigation_bar_css!()
}

/// Rules of the one-page view (`RenderOptions::page_navigation`). They
/// hide pages only on screen and only once the head script has marked the
/// root, so without the script, and on paper, every page shows as before.
/// A page not yet made current by the script is the first one. The shown
/// page is centred at the top of the body; the bar is laid over it. In the
/// logical DOM (D35) a page is its sheet of paper and the chains marked with
/// its `data-page`: the shown sheet flows as the page did, and its chains
/// are placed where the flow puts it (from its laid-out width `--w`).
pub fn navigation_css(logical: bool) -> &'static str {
    if logical {
        concat!(
            r#"@media screen {
.hwpx-paged main {height:0 !important;}
.hwpx-paged [data-page] {display:none;}
.hwpx-paged:not(.hwpx-ready) [data-page="1"], .hwpx-paged [data-page].hwpx-current {display:block;}
.hwpx-paged .hpa[data-page] {position:relative !important;left:auto !important;top:auto !important;margin:0 auto 2mm !important;}
.hwpx-paged [data-page]:not(.hpa) {top:calc(2mm + 1px) !important;margin-top:0 !important;left:calc(50% - var(--w) / 2) !important;}
}
"#,
            navigation_bar_css!()
        )
    } else {
        concat!(
            r#"@media screen {
.hwpx-paged .hpa {display:none;margin-left:auto;margin-right:auto;}
.hwpx-paged:not(.hwpx-ready) .hpa:first-of-type, .hwpx-paged .hpa.hwpx-current {display:block;}
}
"#,
            navigation_bar_css!()
        )
    }
}

/// A face for U+00A0 alone, put before the Hancom Hanyang fonts that need it
/// (`NBSP_BOX_FONTS`) in a class's `font-family`. They draw U+00A0 as a filled rectangle (their
/// cmap points it at a box glyph) where Hancom draws the space blank, and the
/// output writes U+00A0 for runs of spaces and a leading or trailing space.
/// Batang, Gulim, Dotum and Gungsuh have a blank U+00A0 of the same half-em
/// advance as the HY box, so the width of a line does not change, and as a
/// face limited to U+00A0 it is never the font the baseline and the line
/// height are taken from. The correction script's own treatment of the box
/// (`spaces()`) finds this face blank and takes its ordinary path, with the
/// same widths. Without any of the four fonts nothing changes.
const NBSP_FACE: &str = "@font-face {font-family:\"hwpx-nbsp\";src:local(\"Batang\"),local(\"Gulim\"),local(\"Dotum\"),local(\"Gungsuh\");unicode-range:U+A0;}
";

/// The fonts whose U+00A0 is a box: the 25 Hanyang fonts of the Hancom Office
/// 2024 bundle (`TTF/All` and `TTF/Hwp`), found by reading the U+00A0 glyph of
/// every font of that install and of Windows. The other `HY` and `한양` fonts
/// (HY헤드라인M, HY중고딕, HY신명조, 한양신명조 ...) have no box, and a name
/// that merely starts with `HY` does not make one.
const NBSP_BOX_FONTS: [&str; 25] = [
    "HY바다L",
    "HY바다M",
    "HY동녘B",
    "HY동녘M",
    "HY강B",
    "HY강M",
    "HY나무B",
    "HY나무L",
    "HY나무M",
    "HY산B",
    "HY수평선B",
    "HY수평선M",
    "HY태백B",
    "HY울릉도B",
    "HY울릉도M",
    "HY백송B",
    "HY크리스탈M",
    "HY그래픽",
    "HY궁서",
    "HY견고딕",
    "HY견명조",
    "HY목판L",
    "HY엽서M",
    "HY센스L",
    "한양해서",
];

fn draws_nbsp_as_a_box(font_family: &str) -> bool {
    NBSP_BOX_FONTS.contains(&font_family)
}

pub fn dynamic_css(char_styles: &[CharStyle], para_styles: &[ParaStyle]) -> String {
    let mut output = String::new();
    if char_styles
        .iter()
        .any(|style| draws_nbsp_as_a_box(&style.font_family))
    {
        output.push_str(NBSP_FACE);
    }
    for style in char_styles {
        let reduced_size = style.superscript || style.subscript;
        let font_size_hwp = if reduced_size {
            style.font_size_hwp / 2
        } else {
            style.font_size_hwp
        };
        output.push_str(&format!(
            ".cs{} {{font-size:{}pt;",
            style.id,
            format_pt(font_size_hwp)
        ));
        if reduced_size {
            output.push_str(&format!("height:{};", css_mm(font_size_hwp)));
        }
        output.push_str(&format!("color:{};", safe_color(&style.color)));
        if let Some(background) = &style.background {
            output.push_str(&format!("background-color:{};", safe_color(background)));
        }
        if draws_nbsp_as_a_box(&style.font_family) {
            output.push_str(&format!(
                "font-family:\"hwpx-nbsp\", {};",
                css_font_family(style)
            ));
        } else {
            output.push_str(&format!("font-family:{};", css_font_family(style)));
        }
        if style.spacing != 0 {
            output.push_str(&format!("letter-spacing:{};", format_em(style.spacing)));
        }
        if style.baseline_offset != 0 {
            output.push_str("top:");
            output.push_str(&format_pt_offset(reference_baseline_offset(style)));
            output.push_str("pt;");
        }
        if style.superscript {
            output.push_str("top:");
            if font_size_hwp > 100 {
                output.push('-');
            }
            output.push_str(&format_pt(font_size_hwp));
            output.push_str("pt;");
        }
        if style.italic {
            output.push_str("font-style:italic;");
        }
        if style.bold {
            output.push_str("font-weight:bold;");
        }
        if style.emboss {
            output.push_str("text-shadow:-0.5pt -0.5pt 0.1pt white, 0.5pt 0.5pt 0.1pt #666;");
        }
        // Read by the correction script, which widens every other class's
        // spaces to half an em (useFontSpace=0).
        if style.use_font_space {
            output.push_str("--hwpx-font-space:1;");
        }
        output.push_str("}\n");
        if style.underline {
            output.push_str(&format!(
                ".cs{}::after {{content:'';position:absolute;left:0;right:0;top:0;bottom:0;z-index:-1;background-image:url('{}');}}\n",
                style.id,
                if style
                    .underline_color
                    .as_deref()
                    .is_some_and(|color| color.eq_ignore_ascii_case("#0000FF"))
                {
                    BLUE_UNDERLINE_SVG_DATA_URI
                } else {
                    UNDERLINE_SVG_DATA_URI
                }
            ));
        }
        if style.strike {
            output.push_str(&format!(
                ".cs{}::after {{content:'';position:absolute;left:0;right:0;top:0;bottom:0;z-index:-1;background-image:url('{}');}}\n",
                style.id,
                if style.strike_color.as_deref() == Some("#FF0000") {
                    STRIKE_SVG_DATA_URI
                } else {
                    BLACK_STRIKE_SVG_DATA_URI
                }
            ));
        }
    }
    for style in para_styles {
        output.push_str(&format!(
            ".ps{} {{text-align:{};",
            style.id,
            safe_align(&style.align)
        ));
        // The correction script shrinks an overflowing line's spaces by up
        // to this many percent before it tightens the letters.
        if style.condense > 0 {
            output.push_str(&format!("--hwpx-condense:{};", style.condense));
        }
        output.push_str("}\n");
    }
    output.push_str("@media print {\n.hpa {margin:0;border:0 black none;box-shadow:none;}\nbody {padding:0;}\n\n}\n");
    output
}

fn format_pt(hwp: i64) -> String {
    let hundredths = hwp.max(0);
    if hundredths % 100 == 0 {
        format!("{}", hundredths / 100)
    } else {
        format!("{}.{:02}", hundredths / 100, hundredths % 100)
    }
}

fn format_pt_offset(hundredths: i64) -> String {
    let sign = if hundredths < 0 { "-" } else { "" };
    let absolute = hundredths.unsigned_abs();
    if absolute.is_multiple_of(100) {
        format!("{sign}{}", absolute / 100)
    } else {
        format!("{sign}{}.{:02}", absolute / 100, absolute % 100)
    }
}

fn reference_baseline_offset(style: &CharStyle) -> i64 {
    // The sample exporter uses its small manual shifts as 0.15pt steps, but
    // treats larger HWP offsets as a percentage of the character height.
    // These are the two encodings present in the reference corpus: -5 ->
    // +0.75pt, +15 at 18pt -> +2.70pt, and -50 at 11pt -> -5.50pt.
    if style.baseline_offset.unsigned_abs() < 10 {
        style.baseline_offset.saturating_mul(-15)
    } else {
        style.baseline_offset.saturating_mul(style.font_size_hwp) / 100
    }
}

pub(crate) fn safe_color(value: &str) -> &str {
    if value.len() == 7
        && value.starts_with('#')
        && value[1..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        value
    } else {
        "#000000"
    }
}

fn safe_align(value: &str) -> &str {
    match value.to_ascii_lowercase().as_str() {
        "center" => "center",
        "right" => "right",
        "justify" => "justify",
        "distribute" | "distributed" => "justify",
        _ => "left",
    }
}

/// The reference exporter's normalization of a primary Hangul face to the
/// corresponding installed Windows family. A face used only as a Latin
/// fallback is handled separately in `css_font_family`, because the
/// reference keeps that fallback name verbatim.
///
/// Shared with `html::page_number_width`, whose per-font glyph-width table
/// is keyed by this same substituted name (a 쪽 번호 style naming 한양신명조
/// must look up the 바탕 row, since that is the font Chromium actually
/// draws).
pub(crate) fn normalize_hangul_font_family(value: &str) -> &str {
    match value {
        "한양신명조" | "한양견명조" | "명조" => "바탕",
        "한양중고딕" | "한양견고딕" => "돋움",
        _ => value,
    }
}

fn safe_font_family(value: &str) -> String {
    if value.is_empty()
        || value.chars().any(|character| {
            character.is_control() || matches!(character, '"' | '\'' | '\\' | ';' | '{' | '}')
        })
    {
        "sans-serif".to_owned()
    } else {
        format!("\"{}\"", normalize_hangul_font_family(value))
    }
}

fn css_font_family(style: &CharStyle) -> String {
    let primary = safe_font_family(&style.font_family);
    if style.latin_font_family.is_empty()
        || style.latin_font_family == style.font_family
        || style.latin_font_family.chars().any(|character| {
            character.is_control() || matches!(character, '"' | '\'' | '\\' | ';' | '{' | '}')
        })
    {
        return primary;
    }
    let latin = match style.latin_font_family.as_str() {
        "#태신명조" | "바탕" => "Times New Roman",
        "HY\u{acac}\u{ace0}\u{b515}" => "Arial Narrow",
        value => value,
    };
    format!("{primary}, {}", safe_font_fallback_family(latin))
}

fn safe_font_fallback_family(value: &str) -> String {
    if value.is_empty()
        || value.chars().any(|character| {
            character.is_control() || matches!(character, '"' | '\'' | '\\' | ';' | '{' | '}')
        })
    {
        "sans-serif".to_owned()
    } else {
        format!("\"{value}\"")
    }
}

fn format_em(spacing: i64) -> String {
    let hundredths = if spacing >= 0 {
        // The reference quantizes the larger positive values one half-step
        // above ordinary integer rounding (32 -> .17, 45 -> .24,
        // 47 -> .25).  Smaller values retain the ordinary mapping.
        if spacing >= 32 {
            spacing / 2 + 1 + i64::from(spacing % 2 != 0)
        } else {
            (spacing + 1) / 2
        }
    } else {
        let magnitude = spacing.unsigned_abs();
        // The reference keeps an extra half-step for strong negative
        // tracking.  At magnitude 27 and above, odd values receive the
        // corresponding additional half-step as well. The two largest
        // values in the corpus (-38 and -39) share the next quantization
        // bucket.
        let rounded = if magnitude >= 38 {
            magnitude / 2 + 2
        } else if magnitude >= 27 {
            magnitude / 2 + 1 + u64::from(!magnitude.is_multiple_of(2))
        } else if magnitude >= 14 && magnitude.is_multiple_of(2) {
            magnitude / 2 + 1
        } else {
            magnitude.div_ceil(2)
        };
        -(rounded as i64)
    };
    let sign = if hundredths < 0 { "-" } else { "" };
    let absolute = hundredths.unsigned_abs();
    format!("{sign}0.{absolute:02}em")
}

#[cfg(test)]
mod tests {
    use super::dynamic_css;
    use crate::model::{CharStyle, ParaStyle};

    #[test]
    fn character_css_keeps_color_and_font_family_in_their_properties() {
        let css = dynamic_css(
            &[CharStyle {
                id: 7,
                font_family: "Test Sans".to_owned(),
                font_size_hwp: 1000,
                color: "#123456".to_owned(),
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains(".cs7 {font-size:10pt;color:#123456;font-family:\"Test Sans\";}"));
    }

    #[test]
    fn character_spacing_uses_em_units() {
        let css = dynamic_css(
            &[CharStyle {
                id: 8,
                font_family: "Test Sans".to_owned(),
                font_size_hwp: 1500,
                spacing: -11,
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains("letter-spacing:-0.06em;"));
    }

    #[test]
    fn only_a_font_space_class_keeps_the_fonts_own_space() {
        let css = dynamic_css(
            &[
                CharStyle {
                    id: 23,
                    font_family: "Test Sans".to_owned(),
                    font_size_hwp: 1000,
                    ..CharStyle::default()
                },
                CharStyle {
                    id: 39,
                    font_family: "Test Sans".to_owned(),
                    font_size_hwp: 1000,
                    use_font_space: true,
                    ..CharStyle::default()
                },
            ],
            &[],
        );

        assert!(!css.contains(".cs23 {font-size:10pt;color:#000000;font-family:\"Test Sans\";--"));
        assert!(css.contains(
            ".cs39 {font-size:10pt;color:#000000;font-family:\"Test Sans\";--hwpx-font-space:1;}"
        ));
    }

    #[test]
    fn a_condensing_paragraph_tells_the_script_its_limit() {
        let css = dynamic_css(
            &[],
            &[
                ParaStyle {
                    id: 67,
                    align: "justify".to_owned(),
                    condense: 75,
                    ..ParaStyle::default()
                },
                ParaStyle {
                    id: 69,
                    align: "justify".to_owned(),
                    ..ParaStyle::default()
                },
            ],
        );

        assert!(css.contains(".ps67 {text-align:justify;--hwpx-condense:75;}"));
        assert!(css.contains(".ps69 {text-align:justify;}"));
    }

    #[test]
    fn distribute_alignment_maps_to_css_justify() {
        // HWPX's hh:align horizontal="DISTRIBUTE" (배분 정렬) has no direct
        // CSS equivalent; the reference exporter renders it as text-align:
        // justify. header.rs lowercases the raw attribute to "distribute"
        // (no trailing "d"), which safe_align must match exactly.
        let css = dynamic_css(
            &[],
            &[ParaStyle {
                id: 695,
                align: "distribute".to_owned(),
                ..ParaStyle::default()
            }],
        );

        assert!(css.contains(".ps695 {text-align:justify;"));
    }

    #[test]
    fn strong_negative_tracking_keeps_reference_half_step() {
        let css = dynamic_css(
            &[CharStyle {
                id: 11,
                font_family: "Test Sans".to_owned(),
                spacing: -14,
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains("letter-spacing:-0.08em;"));
    }

    #[test]
    fn reference_tracking_keeps_large_value_half_steps() {
        let css = dynamic_css(
            &[
                CharStyle {
                    id: 13,
                    font_family: "Test Sans".to_owned(),
                    spacing: -31,
                    ..CharStyle::default()
                },
                CharStyle {
                    id: 14,
                    font_family: "Test Sans".to_owned(),
                    spacing: 47,
                    ..CharStyle::default()
                },
                CharStyle {
                    id: 15,
                    font_family: "Test Sans".to_owned(),
                    spacing: -38,
                    ..CharStyle::default()
                },
            ],
            &[],
        );

        assert!(css.contains("letter-spacing:-0.17em;"));
        assert!(css.contains("letter-spacing:0.25em;"));
        assert!(css.contains("letter-spacing:-0.21em;"));
    }

    #[test]
    fn underline_uses_reference_svg_decoration() {
        let css = dynamic_css(
            &[CharStyle {
                id: 10,
                font_family: "Test Sans".to_owned(),
                underline: true,
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(!css.contains("text-decoration:underline;"));
        assert!(css.contains(".cs10::after {"));
        assert!(css.contains("background-image:url('data:image/svg+xml;base64,"));
    }

    #[test]
    fn reference_baseline_offsets_follow_sample_encodings() {
        let css = dynamic_css(
            &[
                CharStyle {
                    id: 20,
                    font_family: "Test Sans".to_owned(),
                    font_size_hwp: 1800,
                    baseline_offset: 15,
                    ..CharStyle::default()
                },
                CharStyle {
                    id: 21,
                    font_family: "Test Sans".to_owned(),
                    font_size_hwp: 1100,
                    baseline_offset: -50,
                    ..CharStyle::default()
                },
                CharStyle {
                    id: 22,
                    font_family: "Test Sans".to_owned(),
                    font_size_hwp: 1500,
                    baseline_offset: -5,
                    ..CharStyle::default()
                },
            ],
            &[],
        );

        assert!(css.contains(
            ".cs20 {font-size:18pt;color:#000000;font-family:\"Test Sans\";top:2.70pt;}"
        ));
        assert!(css.contains(
            ".cs21 {font-size:11pt;color:#000000;font-family:\"Test Sans\";top:-5.50pt;}"
        ));
        assert!(css.contains(
            ".cs22 {font-size:15pt;color:#000000;font-family:\"Test Sans\";top:0.75pt;}"
        ));
    }

    #[test]
    fn small_superscript_keeps_reference_positive_offset() {
        let css = dynamic_css(
            &[CharStyle {
                id: 23,
                font_family: "Test Sans".to_owned(),
                font_size_hwp: 100,
                superscript: true,
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains(".cs23 {font-size:0.50pt;height:0.18mm;color:#000000;font-family:\"Test Sans\";top:0.50pt;}"));
    }

    #[test]
    fn subscript_keeps_reference_without_a_top_offset() {
        let css = dynamic_css(
            &[CharStyle {
                id: 25,
                font_family: "Test Sans".to_owned(),
                font_size_hwp: 1400,
                subscript: true,
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains(
            ".cs25 {font-size:7pt;height:2.47mm;color:#000000;font-family:\"Test Sans\";}"
        ));
    }

    #[test]
    fn emboss_uses_reference_text_shadow() {
        let css = dynamic_css(
            &[CharStyle {
                id: 24,
                font_family: "Test Sans".to_owned(),
                emboss: true,
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains("text-shadow:-0.5pt -0.5pt 0.1pt white, 0.5pt 0.5pt 0.1pt #666;"));
    }

    #[test]
    fn latin_font_fallback_is_preserved() {
        let css = dynamic_css(
            &[CharStyle {
                id: 9,
                font_family: "휴먼명조".to_owned(),
                latin_font_family: "HCI Poppy".to_owned(),
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains("font-family:\"휴먼명조\", \"HCI Poppy\";"));
    }

    #[test]
    fn primary_hft_faces_use_reference_substitutions() {
        let css = dynamic_css(
            &[
                CharStyle {
                    id: 1,
                    font_family: "한양신명조".to_owned(),
                    ..CharStyle::default()
                },
                CharStyle {
                    id: 2,
                    font_family: "한양중고딕".to_owned(),
                    ..CharStyle::default()
                },
            ],
            &[],
        );

        assert!(css.contains(".cs1 {font-size:0pt;color:#000000;font-family:\"바탕\";}"));
        assert!(css.contains(".cs2 {font-size:0pt;color:#000000;font-family:\"돋움\";}"));
    }

    #[test]
    fn latin_hft_fallback_is_kept_verbatim() {
        let css = dynamic_css(
            &[CharStyle {
                id: 12,
                font_family: "Primary".to_owned(),
                latin_font_family: "한양신명조".to_owned(),
                ..CharStyle::default()
            }],
            &[],
        );

        assert!(css.contains(
            ".cs12 {font-size:0pt;color:#000000;font-family:\"Primary\", \"한양신명조\";}"
        ));
    }
}
