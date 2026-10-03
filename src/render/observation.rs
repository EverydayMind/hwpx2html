//! Test-only source-key observations. These attributes deliberately do not
//! use data-hwpx-id/part: those attributes drive logical DOM reconstruction.
//! Removing our attributes must recover the ordinary artifact byte for byte.
use super::html::escape_html_attribute;

const ATTRIBUTE: &str = " data-hwpx-observe=\"";
const LINE_ATTRIBUTE: &str = " data-hwpx-observe-line=\"";
const OBJECT_ATTRIBUTE: &str = " data-hwpx-observe-object=\"";

/// The serializer calls this with the preceding attribute's quote open.
/// Provenance describes the layout tokens, before presentation substitutions.
pub(super) fn line_attribute(line: &crate::model::LineFragment) -> String {
    let tokens = line
        .tokens
        .iter()
        .map(|token| {
            let kind = match &token.kind {
                crate::model::TokenKind::Control { kind } => kind.as_str(),
                _ => token.manifest_kind(),
            };
            serde_json::json!([kind, token.visible_text(), token.logical_len])
        })
        .collect::<Vec<_>>();
    let value = serde_json::json!({
        "key": line.paragraph_key,
        "requested": line.source_range,
        "sources": line.token_sources,
        "tokens": tokens,
    });
    format!(
        "\"{LINE_ATTRIBUTE}{}",
        escape_html_attribute(&value.to_string())
    )
}

pub(super) fn annotate_parts(html: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('\u{1}') {
        let end = start + 1 + rest[start + 1..].find('\u{1}').expect("part marker end");
        let (kind, key) = rest[start + 1..end]
            .split_once('\u{2}')
            .expect("part marker key");
        output.push_str(&rest[..start]);
        output.push_str(ATTRIBUTE);
        output.push_str(&escape_html_attribute(&format!("{kind}:{key}")));
        output.push('"');
        // Retain the original marker and therefore the normal merge decisions.
        output.push_str(&rest[start..=end]);
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    output
}

/// Marks the first element an object wrote since `start` with its source id.
/// Only the first element: a picture-filled shape writes its picture and its
/// text box as siblings. An inline object that falls back to the floating
/// path is marked once, by the outer call. Writes nothing for no output.
pub(super) fn mark_object(html: &mut String, start: usize, id: &str) {
    let Some(open) = html[start..].find('<').map(|offset| start + offset) else {
        return;
    };
    let tag_end = open
        + html[open..]
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .expect("tag end");
    let tag_close = open + html[open..].find('>').expect("tag close");
    if html[open..tag_close].contains(OBJECT_ATTRIBUTE) {
        return;
    }
    let attribute = format!("{OBJECT_ATTRIBUTE}{}\"", escape_html_attribute(id));
    html.insert_str(tag_end, &attribute);
}

fn strip_observations(html: &str) -> String {
    strip_attribute(
        &strip_attribute(&strip_attribute(html, LINE_ATTRIBUTE), OBJECT_ATTRIBUTE),
        ATTRIBUTE,
    )
}

#[test]
fn object_marks_go_on_the_first_element_once() {
    let mut html = String::from("<p>");
    let start = html.len();
    html.push_str("<svg class=\"x\"><g/></svg><div>t</div>");
    mark_object(&mut html, start, "s0/p#1/object-0-rect");
    mark_object(&mut html, start, "s0/p#1/object-0-rect");
    assert_eq!(
        html,
        "<p><svg data-hwpx-observe-object=\"s0/p#1/object-0-rect\" class=\"x\"><g/></svg><div>t</div>"
    );
    assert_eq!(
        strip_observations(&html),
        "<p><svg class=\"x\"><g/></svg><div>t</div>"
    );
    let before = html.clone();
    let end = html.len();
    mark_object(&mut html, end, "nothing written");
    assert_eq!(html, before);
}

fn strip_attribute(html: &str, attribute: &str) -> String {
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(attribute) {
        output.push_str(&rest[..start]);
        let value = &rest[start + attribute.len()..];
        rest = &value[value.find('"').expect("observation attribute end") + 1..];
    }
    output.push_str(rest);
    output
}

#[test]
fn observations_keep_singletons_and_do_not_change_part_links() {
    use super::semantic::{finalize_parts, part_marker};
    let source = format!(
        "<p{}>same</p><p{}>same</p><p{}></p><p{}>x</p><p{}>y</p>",
        part_marker('p', "s0/p[0]"),
        part_marker('p', "s0/p[1]"),
        part_marker('p', "s0/p[2]&\""),
        part_marker('p', "s0/p[3]"),
        part_marker('p', "s0/p[3]"),
    );
    let observed = finalize_parts(&annotate_parts(&source));
    assert_eq!(strip_observations(&observed), finalize_parts(&source));
    assert_eq!(observed.matches(ATTRIBUTE).count(), 5);
    assert!(observed.contains("p:s0/p[2]&amp;&quot;"));
    assert_eq!(observed.matches("data-hwpx-part=").count(), 2);
}

#[test]
fn observations_survive_logical_merging_without_changing_markup() {
    use super::semantic::{finalize_parts, part_marker};
    let mut source = String::from("<html><head></head><body>");
    for text in ["first", "second"] {
        source.push_str(&format!(
            "<div class=\"hpa\" style=\"width:210mm;height:297mm;\"><p{}>\
             <span class=\"hls\" style=\"position:absolute;left:1mm;top:1mm;width:10mm;height:3mm;\">{text}</span>\
             </p><p{}></p></div>",
            part_marker('p', "s0/p[0]"),
            part_marker('p', &format!("s0/{text}")),
        ));
    }
    source.push_str("</body></html>");
    let ordinary = super::logical::rebuild(&finalize_parts(&source));
    let observed = super::logical::rebuild(&finalize_parts(&annotate_parts(&source)));
    assert!(
        observed.contains("<main"),
        "must actually rebuild the pages"
    );
    assert_eq!(strip_observations(&observed), ordinary);
    assert_eq!(observed.matches("p:s0/p[0]").count(), 1);
    assert!(observed.contains("p:s0/first"));
    assert!(observed.contains("p:s0/second"));
}

/// Every table's and object's parsed source anchor (plan §3.1.2), keyed like
/// the observation marks, for comparison with the independent XML slots.
#[cfg(test)]
fn source_anchors(document: &crate::model::Document) -> Vec<serde_json::Value> {
    use crate::model::{Block, Paragraph, PositionedObject, SourceAnchor, Table};
    fn anchor(anchor: &Option<SourceAnchor>) -> serde_json::Value {
        anchor.as_ref().map_or(
            serde_json::Value::Null,
            |a| serde_json::json!({"paragraph": a.paragraph_key, "textpos": a.textpos}),
        )
    }
    fn paragraph(out: &mut Vec<serde_json::Value>, p: &Paragraph) {
        p.objects.iter().for_each(|o| object(out, o));
    }
    fn object(out: &mut Vec<serde_json::Value>, o: &PositionedObject) {
        let mut record =
            serde_json::json!({"kind": "o", "key": o.key, "anchor": anchor(&o.source_anchor)});
        if o.kind == "container" {
            // Inputs to Q3, captured before layout or reading-order sorting.
            // The Python gate applies the policy independently; capturing
            // only the already-sorted result would not test the rule.
            record["reading_children"] = serde_json::Value::Array(
                o.children
                    .iter()
                    .map(|child| {
                        let (pen, text) = child.shape.as_ref().map_or((0, false), |s| {
                            (
                                s.line_width.max(s.declared_line_width),
                                !s.paragraphs.is_empty(),
                            )
                        });
                        serde_json::json!({"key": child.key, "box": child.box_units,
                            "pen": pen, "text": text})
                    })
                    .collect(),
            );
        }
        out.push(record);
        o.children.iter().for_each(|c| object(out, c));
        if let Some(shape) = &o.shape {
            shape.paragraphs.iter().for_each(|p| paragraph(out, p));
            shape.tables.iter().for_each(|t| table(out, t));
        }
    }
    fn table(out: &mut Vec<serde_json::Value>, t: &Table) {
        out.push(serde_json::json!({"kind": "t", "key": t.id, "anchor": anchor(&t.source_anchor)}));
        for cell in &t.cells {
            cell.paragraphs.iter().for_each(|p| paragraph(out, p));
            cell.tables.iter().for_each(|n| table(out, n));
        }
    }
    let mut out = Vec::new();
    for section in &document.sections {
        for block in &section.blocks {
            match block {
                Block::Paragraph(p) => paragraph(&mut out, p),
                Block::Table(t) => table(&mut out, t),
                Block::Object(o) => object(&mut out, o),
            }
        }
    }
    out
}

/// Source content kept for the semantic tree but not drawn yet (plan
/// §3.1.3): inline items per paragraph, captions per host, equation
/// scripts. Compared with the independent XML inventory.
fn preserved_sources(document: &crate::model::Document) -> serde_json::Value {
    use crate::model::{Block, Caption, InlineContent, Paragraph, PositionedObject, Table, Token};
    #[derive(Default)]
    struct Out {
        inline: Vec<serde_json::Value>,
        captions: Vec<serde_json::Value>,
        equations: Vec<serde_json::Value>,
        style_refs: Vec<serde_json::Value>,
    }
    fn paragraph(out: &mut Out, p: &Paragraph) {
        out.style_refs.push(
            serde_json::json!({"key": p.key, "style_id": p.para_style_id,
            "tokens": p.tokens.iter().map(|t| t.char_style_id).collect::<Vec<_>>()}),
        );
        for item in &p.inline_sources {
            let content = match &item.content {
                InlineContent::AutoNumber {
                    number_type,
                    number,
                    format,
                    user_char,
                    prefix,
                    suffix,
                    superscript,
                } => serde_json::json!({"kind": "autoNum", "numType": number_type, "num": number,
                    "type": format, "userChar": user_char, "prefixChar": prefix,
                    "suffixChar": suffix, "supscript": superscript}),
                InlineContent::Ruby {
                    base,
                    annotation,
                    position,
                    size_ratio,
                    option,
                    style_ref,
                    align,
                    ..
                } => serde_json::json!({"kind": "dutmal", "mainText": base, "subText": annotation,
                    "posType": position, "szRatio": size_ratio, "option": option,
                    "styleIDRef": style_ref, "align": align}),
                InlineContent::Compose {
                    text,
                    shape,
                    char_size,
                    compose_type,
                    char_prs,
                    ..
                } => serde_json::json!({"kind": "compose", "composeText": text,
                    "circleType": shape, "charSz": char_size, "composeType": compose_type,
                    "charPrIDRefs": char_prs}),
            };
            out.inline.push(
                serde_json::json!({"paragraph": p.key, "textpos": item.textpos,
                "content": content}),
            );
        }
        p.objects.iter().for_each(|o| object(out, o));
    }
    fn caption(out: &mut Out, host: &str, c: &Option<Box<Caption>>) {
        if let Some(c) = c {
            let paragraphs = c
                .paragraphs
                .iter()
                .map(|p| {
                    serde_json::json!({"key": p.key,
                        "text": p.tokens.iter().map(Token::visible_text).collect::<String>()})
                })
                .collect::<Vec<_>>();
            out.captions.push(
                serde_json::json!({"host": host, "side": c.side, "gap": c.gap,
                "width": c.width, "paragraphs": paragraphs}),
            );
            c.paragraphs.iter().for_each(|p| paragraph(out, p));
        }
    }
    fn object(out: &mut Out, o: &PositionedObject) {
        caption(out, &o.key, &o.caption);
        if let Some(e) = &o.equation {
            out.equations
                .push(serde_json::json!({"key": o.key, "script": e.script,
                "baseUnit": e.base_unit, "baseLine": e.base_line, "font": e.font,
                "textColor": e.text_color}));
        }
        o.children.iter().for_each(|c| object(out, c));
        if let Some(shape) = &o.shape {
            shape.paragraphs.iter().for_each(|p| paragraph(out, p));
            shape.tables.iter().for_each(|t| table(out, t));
        }
    }
    fn table(out: &mut Out, t: &Table) {
        caption(out, &t.id, &t.caption);
        for cell in &t.cells {
            cell.paragraphs.iter().for_each(|p| paragraph(out, p));
            cell.tables.iter().for_each(|n| table(out, n));
        }
    }
    let mut out = Out::default();
    for section in &document.sections {
        for block in &section.blocks {
            match block {
                Block::Paragraph(p) => paragraph(&mut out, p),
                Block::Table(t) => table(&mut out, t),
                Block::Object(o) => object(&mut out, o),
            }
        }
    }
    serde_json::json!({"inline": out.inline, "captions": out.captions, "equations": out.equations,
        "style_refs": out.style_refs, "numberings": document.numberings})
}

/// Opt-in corpus capture, kept out of the CLI and normal test run.
/// HWPX_OBSERVE_OUTPUT must name a new directory; samples are only read.
#[test]
#[ignore = "local corpus; set HWPX_OBSERVE_OUTPUT and optionally HWPX_OBSERVE_SAMPLES"]
fn capture_samples() {
    use crate::hwpx::{package::Limits, DocumentReader};
    use crate::render::RenderOptions;
    use sha2::{Digest, Sha256};
    use std::{fs, path::PathBuf};

    let samples =
        PathBuf::from(std::env::var_os("HWPX_OBSERVE_SAMPLES").unwrap_or_else(|| "샘플".into()));
    let output =
        PathBuf::from(std::env::var_os("HWPX_OBSERVE_OUTPUT").expect("HWPX_OBSERVE_OUTPUT"));
    fs::create_dir(&output).expect("new output directory (parent must exist)");
    let mut inputs = walkdir::WalkDir::new(&samples)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("hwpx"))
        })
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    inputs.sort();
    assert!(!inputs.is_empty(), "no HWPX samples");
    let mut records = Vec::new();
    for (index, path) in inputs.iter().enumerate() {
        let bytes = fs::read(path).unwrap();
        let input_hash = format!("{:x}", Sha256::digest(&bytes));
        let sample = path.strip_prefix(&samples).unwrap().to_string_lossy();
        if bytes.starts_with(&[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]) {
            records.push(
                serde_json::json!({"sample":sample,"input_sha256":input_hash,"rejected":"OLE"}),
            );
            continue;
        }
        let document = crate::HwpxReader::new(path, Limits::default())
            .read()
            .unwrap();
        fs::write(
            output.join(format!("{index}-anchors.json")),
            serde_json::to_vec_pretty(&source_anchors(&document)).unwrap(),
        )
        .unwrap();
        fs::write(
            output.join(format!("{index}-preserved.json")),
            serde_json::to_vec_pretty(&preserved_sources(&document)).unwrap(),
        )
        .unwrap();
        fs::write(
            output.join(format!("{index}-tree.json")),
            serde_json::to_vec(&crate::semantic::build(&document, true)).unwrap(),
        )
        .unwrap();
        let layout = crate::layout::layout_document(&document);
        for logical_dom in [false, true] {
            let options = RenderOptions {
                logical_dom,
                adjust_letter_spacing: false,
                page_navigation: false,
                source_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                ..Default::default()
            };
            let normal = super::render_bundle(&layout, &options, "resources");
            let observed = super::bundle::render_bundle_impl(&layout, &options, "resources", true);
            assert_eq!(
                strip_observations(&observed.html),
                normal.html,
                "{sample}, logical={logical_dom}"
            );
            assert_eq!(observed.stylesheet, normal.stylesheet);
            assert_eq!(observed.resources.len(), normal.resources.len());
            for (name, resource) in &normal.resources {
                assert_eq!(
                    observed.resources[name].data, resource.data,
                    "{sample}: {name}"
                );
                assert_eq!(observed.resources[name].mime, resource.mime);
            }
            let filename = format!(
                "{index}-{}.html",
                if logical_dom { "logical" } else { "pages" }
            );
            let embedded = observed.to_single_html();
            assert_eq!(strip_observations(&embedded), normal.to_single_html());
            fs::write(output.join(&filename), embedded).unwrap();
            records.push(serde_json::json!({
                "sample":sample,"input_sha256":input_hash,"logical_dom":logical_dom,
                "html":filename,"normal_html_sha256":format!("{:x}",Sha256::digest(normal.html.as_bytes())),
                "observations":observed.html.matches(ATTRIBUTE).count(),"objects":observed.html.matches(OBJECT_ATTRIBUTE).count(),"pages":layout.pages.len()
            }));
        }
        eprintln!("observed {sample}");
    }
    fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .unwrap();
}
