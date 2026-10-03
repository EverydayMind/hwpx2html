use super::{build, Kind, Node};
use crate::hwpx::{header::HeaderStyles, section::parse_section};
use crate::model::Document;

fn document(body: &str, header: &str) -> Document {
    let header = HeaderStyles::parse(
        format!(r#"<head xmlns="http://www.hancom.co.kr/hwpml/2011/head">{header}</head>"#)
            .as_bytes(),
    )
    .unwrap();
    let xml = format!(
        r#"<sec xmlns="http://www.hancom.co.kr/hwpml/2011/section" xmlns:hp="http://www.hancom.co.kr/hwpml/2011/paragraph">{body}</sec>"#
    );
    Document {
        sections: vec![
            parse_section(xml.as_bytes(), 0, "section0.xml", &header)
                .unwrap()
                .section,
        ],
        char_styles: header.char_styles.into_values().collect(),
        numberings: header.numberings,
        ..Default::default()
    }
}

fn p(text: &str) -> String {
    format!("<hp:p><hp:run><hp:t>{text}</hp:t></hp:run></hp:p>")
}

fn find<'a>(node: &'a Node, key: &str) -> &'a Node {
    fn search<'a>(node: &'a Node, key: &str) -> Option<&'a Node> {
        if node.key == key {
            return Some(node);
        }
        node.children.iter().find_map(|child| search(child, key))
    }
    search(node, key).expect("key in tree")
}

#[test]
fn empty_and_note_paragraphs_continue_the_previous_item() {
    let doc = document(
        &[p("□ first"), p(""), p("※ note"), p("* note"), p("□ second")].concat(),
        "",
    );
    let tree = build(&doc, true);
    let body = &tree.children[0].children;
    assert_eq!(body.len(), 1);
    assert!(matches!(body[0].kind, Kind::List { .. }));
    assert_eq!(body[0].children.len(), 2);
    assert_eq!(body[0].children[0].children.len(), 4);
}

#[test]
fn a_note_carrying_a_table_or_picture_ends_the_list() {
    // Corpus evidence: 성과보고서 s27/p[36], a ** note after an inline
    // table, must not carry the preceding list across this paragraph (Q4).
    let table = r#"<hp:tbl rowCnt="1" colCnt="1"><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p/></hp:subList></hp:tc></hp:tr></hp:tbl>"#;
    for anchor in [table, r#"<hp:pic><hp:pos treatAsChar="1"/></hp:pic>"#] {
        let body = format!(
            "{}<hp:p><hp:run>{anchor}<hp:t> ** note</hp:t></hp:run></hp:p>{}",
            p("□ before"),
            p("□ after")
        );
        let tree = build(&document(&body, ""), true);
        let body = &tree.children[0].children;
        assert_eq!(body.len(), 3);
        assert!(matches!(body[0].kind, Kind::List { .. }));
        assert!(matches!(body[1].kind, Kind::Paragraph { .. }));
        assert_eq!(body[1].children.len(), 1);
        assert!(matches!(body[2].kind, Kind::List { .. }));
    }
}

#[test]
fn an_explicit_page_break_starts_a_new_list() {
    let body = format!(
        "{}<hp:p pageBreak=\"1\"><hp:run><hp:t>□ after</hp:t></hp:run></hp:p>",
        p("□ before")
    );
    let tree = build(&document(&body, ""), true);
    assert_eq!(tree.children[0].children.len(), 2);
    assert!(tree.children[0]
        .children
        .iter()
        .all(|n| matches!(n.kind, Kind::List { .. })));
}

#[test]
fn cell_chapters_require_fifteen_point_first_visible_text() {
    let header = r#"<charPr id="0" height="1499"/><charPr id="1" height="1500"/>"#;
    let body = format!(
        r#"{}<hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:pos treatAsChar="1"/><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p><hp:run charPrIDRef="0"><hp:t>제1장 small</hp:t></hp:run></hp:p><hp:p><hp:run charPrIDRef="1"><hp:t>제2장 title</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl></hp:run></hp:p>"#,
        p("제1장 body")
    );
    let tree = build(&document(&body, header), true);
    assert!(matches!(
        find(&tree, "s0/p[0]").kind,
        Kind::Heading { level: 2, .. }
    ));
    assert!(matches!(
        find(&tree, "s0/tbl-anchor[1-0]/r0c0/p0").kind,
        Kind::Paragraph { .. }
    ));
    assert!(matches!(
        find(&tree, "s0/tbl-anchor[1-0]/r0c0/p1").kind,
        Kind::Heading { level: 2, .. }
    ));
}

#[test]
fn disabling_inference_keeps_only_source_headings_and_lists() {
    let header = r#"<paraPr id="1"><heading type="OUTLINE" level="2"/></paraPr><paraPr id="2"><heading type="BULLET" level="0"/></paraPr>"#;
    let body = format!(
        "{}{}<hp:p paraPrIDRef=\"1\"><hp:run><hp:t>source heading</hp:t></hp:run></hp:p><hp:p paraPrIDRef=\"2\"><hp:run><hp:t>source item</hp:t></hp:run></hp:p>",
        p("제1장 inferred"), p("□ inferred")
    );
    let tree = build(&document(&body, header), false);
    assert!(matches!(
        find(&tree, "s0/p[0]").kind,
        Kind::Paragraph { .. }
    ));
    assert!(matches!(
        find(&tree, "s0/p[1]").kind,
        Kind::Paragraph { .. }
    ));
    assert!(matches!(
        find(&tree, "s0/p[2]").kind,
        Kind::Heading {
            level: 3,
            inferred: false,
            ..
        }
    ));
    assert!(matches!(
        find(&tree, "s0/p[3]/list").kind,
        Kind::List {
            inferred: false,
            ..
        }
    ));
}

#[test]
fn provenance_preserves_utf16_utf8_styles_and_control_positions() {
    let doc = document(
        r#"<hp:p paraPrIDRef="5"><hp:run charPrIDRef="7"><hp:t>A😀</hp:t><hp:dutmal><hp:mainText>base</hp:mainText><hp:subText>ruby</hp:subText></hp:dutmal><hp:t>B</hp:t><hp:compose composeText="C"/><hp:t>B</hp:t></hp:run></hp:p>"#,
        "",
    );
    let tree = build(&doc, false);
    let Kind::Paragraph { source, content } = &find(&tree, "s0/p[0]").kind else {
        panic!()
    };
    assert_eq!(source.source_style_id, 5);
    assert_eq!(source.tokens.len(), 5);
    assert_eq!(source.tokens[0].logical.start, Some(0));
    assert_eq!(source.tokens[0].logical.end, Some(3));
    assert_eq!(source.tokens[0].utf8, Some(0..5));
    // A dutmal and a compose are eight positions like the other controls
    // (제어문자.hwpx), so every position after them is known.
    assert_eq!(source.tokens[1].logical.start, Some(3));
    assert_eq!(source.tokens[1].logical.end, Some(11));
    assert_eq!(source.tokens[1].token_index, Some(1));
    assert_eq!(source.tokens[2].logical.start, Some(11));
    assert_eq!(source.tokens[2].token_index, Some(2));
    assert_eq!(source.tokens[3].inline_index, Some(1));
    assert_eq!(source.tokens[3].logical.start, Some(12));
    assert_eq!(source.tokens[3].logical.end, Some(20));
    assert_eq!(source.tokens[4].logical.start, Some(20));
    assert!(source.tokens.iter().all(|t| t.source_char_style_id == 7));
    assert_eq!(
        content.iter().map(|s| s.source_index).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2), Some(3), Some(4)]
    );
}

#[test]
fn provenance_keeps_raw_ids_when_styles_are_compacted() {
    use crate::model::{CharStyle, ParaStyle};
    let mut styles = HeaderStyles::default();
    for id in 0..4 {
        styles.char_styles.insert(
            id,
            CharStyle {
                id,
                ..Default::default()
            },
        );
        styles.para_styles.insert(
            id,
            ParaStyle {
                id,
                ..Default::default()
            },
        );
    }
    let aliases = [(2, 0)].into_iter().collect();
    styles.compact_char_styles(&Default::default(), &aliases);
    styles.compact_para_styles(&aliases);
    let xml = br#"<sec><p paraPrIDRef="3"><run charPrIDRef="2"><t>A</t><dutmal><mainText>B</mainText><subText>C</subText></dutmal></run><run charPrIDRef="3"><t>A</t></run></p></sec>"#;
    let doc = Document {
        sections: vec![
            parse_section(xml, 0, "section0.xml", &styles)
                .unwrap()
                .section,
        ],
        ..Default::default()
    };
    let tree = build(&doc, false);
    let Kind::Paragraph { source, content } = &find(&tree, "s0/p[0]").kind else {
        panic!()
    };
    assert_eq!((source.source_style_id, source.style_id), (3, 2));
    assert_eq!(
        source
            .tokens
            .iter()
            .map(|t| (t.source_char_style_id, t.char_style_id))
            .collect::<Vec<_>>(),
        vec![(2, 0), (2, 0), (3, 2)]
    );
    assert_eq!(
        content.iter().map(|s| s.source_index).collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(2)]
    );
}

#[test]
fn numbering_definition_start_is_distinct_from_level_start() {
    let styles = HeaderStyles::parse(br#"<head><numbering id="7" start="0"><paraHead level="1" start="3" numFormat="DIGIT">^1.</paraHead></numbering><numbering id="8" start="-1"><paraHead level="1" start="5" numFormat="DIGIT">^1)</paraHead></numbering></head>"#).unwrap();
    assert_eq!(styles.numberings[&7].start, 0);
    assert_eq!(styles.numberings[&7].levels[0].start, 3);
    assert_eq!(styles.numberings[&8].start, -1);
    assert_eq!(styles.numberings[&8].levels[0].start, 5);
}

#[test]
fn numbered_lists_continue_across_breaks_contexts_and_sections() {
    let header = r#"<head><numbering id="7" start="0"><paraHead level="1" start="3" numFormat="DIGIT">^1.</paraHead><paraHead level="2" start="5" numFormat="DIGIT">^1.^2)</paraHead></numbering><numbering id="8" start="1"><paraHead level="1" start="9" numFormat="DIGIT">^1.</paraHead></numbering><paraPr id="1"><heading type="NUMBER" idRef="7" level="0"/></paraPr><paraPr id="2"><heading type="NUMBER" idRef="7" level="1"/></paraPr><paraPr id="3"><heading type="NUMBER" idRef="8" level="0"/></paraPr></head>"#;
    let styles = HeaderStyles::parse(header.as_bytes()).unwrap();
    let body = r#"<sec><p paraPrIDRef="1"><run><t>host</t><tbl rowCnt="1" colCnt="1"><tr><tc><cellAddr rowAddr="0" colAddr="0"/><subList><p paraPrIDRef="1"><run><t>cell</t></run></p></subList></tc></tr></tbl></run></p><p/><p paraPrIDRef="1" pageBreak="1"><run><ctrl><newNum numType="PAGE" num="1"/></ctrl><t>next</t></run></p><p paraPrIDRef="2"><run><t>deep</t></run></p><p paraPrIDRef="1"><run><t>shallow</t></run></p><p paraPrIDRef="2"><run><t>deep reset</t></run></p><p paraPrIDRef="3"><run><t>other id</t></run></p></sec>"#;
    let second = r#"<sec><p paraPrIDRef="1"><run><t>second section</t></run></p></sec>"#;
    let doc = Document {
        sections: vec![
            parse_section(body.as_bytes(), 0, "a", &styles)
                .unwrap()
                .section,
            parse_section(second.as_bytes(), 1, "b", &styles)
                .unwrap()
                .section,
        ],
        numberings: styles.numberings,
        ..Default::default()
    };
    let tree = build(&doc, false);
    for (key, value) in [
        ("s0/p[0]", 3),
        ("s0/tbl-anchor[0-0]/r0c0/p0", 4),
        ("s0/p[2]", 5),
        ("s0/p[3]", 5),
        ("s0/p[4]", 6),
        ("s0/p[5]", 5),
        ("s0/p[6]", 9),
        ("s1/p[0]", 7),
    ] {
        let Kind::Paragraph { source, .. } = &find(&tree, key).kind else {
            panic!()
        };
        let numbering = source.numbering.as_ref().unwrap();
        assert_eq!(numbering.value, Some(value), "{key}");
        assert!(numbering.inferred);
        assert!(
            matches!(find(&tree,&format!("{key}/li")).kind, Kind::Item { value: Some(n) } if n == value)
        );
    }
    assert!(matches!(
        find(&tree, "s0/p[2]/list").kind,
        Kind::List { start: Some(5), .. }
    ));
    assert!(matches!(
        find(&tree, "s1/p[0]/list").kind,
        Kind::List { start: Some(7), .. }
    ));
    let Kind::Paragraph { content, .. } = &find(&tree, "s0/p[3]").kind else {
        panic!()
    };
    assert!(matches!(&content[0].value,super::Inline::Generated { text } if text == "5.5)"));
}

#[test]
fn absent_numbering_definition_does_not_invent_a_value() {
    let doc = document(
        r#"<hp:p paraPrIDRef="1"><hp:run><hp:t>A</hp:t></hp:run></hp:p>"#,
        r#"<paraPr id="1"><heading type="NUMBER" idRef="99" level="0"/></paraPr>"#,
    );
    let tree = build(&doc, false);
    assert!(matches!(
        find(&tree, "s0/p[0]/list").kind,
        Kind::List { start: None, .. }
    ));
    assert!(matches!(
        find(&tree, "s0/p[0]/li").kind,
        Kind::Item { value: None }
    ));
}

#[test]
fn numbered_anchors_are_counted_in_interleaved_reading_order() {
    let header = r#"<numbering id="1"><paraHead level="1" start="1" numFormat="DIGIT">^1.</paraHead></numbering><paraPr id="1"><heading type="NUMBER" idRef="1" level="0"/></paraPr>"#;
    let body = r#"<hp:p><hp:run><hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p paraPrIDRef="1"><hp:run><hp:t>table first</hp:t></hp:run></hp:p></hp:subList></hp:tc></hp:tr></hp:tbl><hp:rect><hp:drawText><hp:subList><hp:p paraPrIDRef="1"><hp:run><hp:t>shape second</hp:t></hp:run></hp:p></hp:subList></hp:drawText></hp:rect></hp:run></hp:p>"#;
    let tree = build(&document(body, header), false);
    let values: Vec<_> = find(&tree, "s0/p[0]")
        .children
        .iter()
        .map(|n| {
            let key = if matches!(n.kind, Kind::Table { .. }) {
                "s0/tbl-anchor[0-0]/r0c0/p0"
            } else {
                "s0/p[0]/o0/p0"
            };
            let Kind::Paragraph { source, .. } = &find(n, key).kind else {
                panic!()
            };
            source.numbering.as_ref().unwrap().value
        })
        .collect();
    assert_eq!(values, vec![Some(1), Some(2)]);
}

#[test]
fn marked_paragraph_keeps_its_anchor_in_the_list_item_q8() {
    // Corpus: performance report s17/p[6], a marked paragraph with a table.
    for anchor in [
        r#"<hp:tbl rowCnt="1" colCnt="1"><hp:tr><hp:tc><hp:cellAddr rowAddr="0" colAddr="0"/><hp:subList><hp:p/></hp:subList></hp:tc></hp:tr></hp:tbl>"#,
        "<hp:pic/>",
    ] {
        let body = format!(
            "{}<hp:p><hp:run><hp:t>□ anchored item</hp:t>{anchor}</hp:run></hp:p>{}{}",
            p("□ before"),
            p("※ note"),
            p("□ after")
        );
        let tree = build(&document(&body, ""), true);
        let list = find(&tree, "s0/p[0]/list");
        assert_eq!(list.children.len(), 3);
        let item = find(&tree, "s0/p[1]/li");
        assert_eq!(item.children.len(), 2);
        assert_eq!(item.children[1].key, "s0/p[2]");
        assert_eq!(item.children[0].children.len(), 1);
        assert!(matches!(
            item.children[0].children[0].kind,
            Kind::Table { .. } | Kind::Object { .. }
        ));
        assert_eq!(tree.children[0].children.len(), 1);
    }
}
