use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::Result;
use crate::layout::{hwp_to_centi_mm, layout_document};
use crate::model::{BoxUnits, LayoutDocument, PositionedObject, Table, TokenKind};

#[derive(Debug, Serialize)]
pub struct Manifest {
    pub schema_version: &'static str,
    pub manifest_kind: &'static str,
    pub input_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub css_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_provenance: Option<Provenance>,
    pub pages: Vec<ManifestPage>,
}

#[derive(Debug, Serialize)]
pub struct Provenance {
    pub source: String,
    pub match_status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ManifestPage {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_number: Option<String>,
    pub page_size: ManifestBox,
    pub content_box: ManifestBox,
    pub text_tokens: Vec<ManifestToken>,
    pub text_token_hash: String,
    pub visible_text_hash: String,
    pub table_fragments: Vec<ManifestTable>,
    pub pictures: Vec<ManifestObject>,
    pub shapes: Vec<ManifestObject>,
    pub unsupported_objects: Vec<ManifestObject>,
    pub objects: Vec<ManifestObject>,
}

/// Source-level tokens used to diagnose page alignment. The hashes above are
/// the compact comparison keys; this stream explains a mismatch without
/// losing the distinction between text, tabs, spaces, line breaks, and
/// invisible controls.
#[derive(Debug, Serialize)]
pub struct ManifestToken {
    pub kind: &'static str,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ManifestTable {
    pub source_id: String,
    pub source_xml_path: String,
    pub fragment_index: usize,
    pub stacking_order: i64,
    pub box_units: ManifestBox,
    pub cells: Vec<ManifestCell>,
}

#[derive(Debug, Serialize)]
pub struct ManifestCell {
    pub source_id: String,
    pub source_xml_path: String,
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub box_units: ManifestBox,
    pub repeated_header: bool,
    pub repeated_from: Option<String>,
    pub occurrence: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestObject {
    pub id: String,
    pub source_xml_path: String,
    pub kind: String,
    pub box_units: ManifestBox,
    pub stacking_order: i64,
    pub stacking_context: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManifestBox {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

pub fn layout_manifest(document: &crate::model::Document) -> Manifest {
    let layout = layout_document(document);
    manifest_from_layout(&layout, "layout", None, None)
}

pub fn canonical_manifest(document: &crate::model::Document, source_dir: &Path) -> Manifest {
    let layout = layout_document(document);
    let html_sha256 = std::fs::read(source_dir.join("HMF00000.html"))
        .ok()
        .map(|bytes| sha256_hex(&bytes));
    let css_sha256 = std::fs::read(source_dir.join("HMF00000_style.css"))
        .ok()
        .map(|bytes| sha256_hex(&bytes));
    manifest_from_layout(
        &layout,
        "canonical",
        html_sha256.zip(css_sha256),
        Some(Provenance {
            source: source_dir.display().to_string(),
            match_status: "derived_from_hwpx_layout",
        }),
    )
}

fn manifest_from_layout(
    layout: &LayoutDocument,
    kind: &'static str,
    checksums: Option<(String, String)>,
    provenance: Option<Provenance>,
) -> Manifest {
    Manifest {
        schema_version: "1.1.0",
        manifest_kind: kind,
        input_sha256: layout.input_sha256.clone(),
        html_sha256: checksums.as_ref().map(|pair| pair.0.clone()),
        css_sha256: checksums.map(|pair| pair.1),
        canonical_provenance: provenance,
        pages: layout
            .pages
            .iter()
            .map(|page| {
                let page_size = ManifestBox {
                    x: 0,
                    y: 0,
                    width: hwp_to_centi_mm(page.spec.width),
                    height: hwp_to_centi_mm(page.spec.height),
                };
                let content_box = ManifestBox {
                    x: hwp_to_centi_mm(page.spec.margin_left),
                    y: hwp_to_centi_mm(page.spec.margin_top + page.spec.header),
                    width: hwp_to_centi_mm(
                        page.spec.width - page.spec.margin_left - page.spec.margin_right,
                    ),
                    height: hwp_to_centi_mm(
                        page.spec.height
                            - page.spec.margin_top
                            - page.spec.header
                            - page.spec.margin_bottom
                            - page.spec.footer,
                    ),
                };
                let mut hasher = Sha256::new();
                let mut visible_hasher = Sha256::new();
                let mut text_tokens = Vec::new();
                for line in &page.lines {
                    for token in &line.tokens {
                        text_tokens.push(ManifestToken {
                            kind: token.manifest_kind(),
                            text: token.visible_text(),
                        });
                        hasher.update(token.manifest_kind().as_bytes());
                        hasher.update([0]);
                        hasher.update(token.visible_text().as_bytes());
                        hasher.update([0xff]);
                        if !matches!(token.kind, TokenKind::Control { .. }) {
                            visible_hasher.update(token.manifest_kind().as_bytes());
                            visible_hasher.update([0]);
                            visible_hasher.update(token.visible_text().as_bytes());
                            visible_hasher.update([0xff]);
                        }
                    }
                }
                let mut pictures = Vec::new();
                let mut shapes = Vec::new();
                let mut unsupported_objects = Vec::new();
                let mut objects = Vec::new();
                // Treat-as-character objects are rendered inside their line
                // fragment rather than in `page.objects`. Include them in
                // the manifest as well, otherwise inline pictures silently
                // disappear from the canonical object cardinality check.
                for line in &page.lines {
                    for object in &line.inline_objects {
                        append_object_manifest(
                            object,
                            &mut pictures,
                            &mut shapes,
                            &mut unsupported_objects,
                            &mut objects,
                        );
                    }
                }
                for object in &page.objects {
                    append_object_manifest(
                        object,
                        &mut pictures,
                        &mut shapes,
                        &mut unsupported_objects,
                        &mut objects,
                    );
                }
                for placed in &page.tables {
                    append_table_objects(
                        &placed.table,
                        &mut pictures,
                        &mut shapes,
                        &mut unsupported_objects,
                        &mut objects,
                    );
                }
                ManifestPage {
                    index: page.index,
                    page_number: page.page_number.clone(),
                    page_size,
                    content_box,
                    text_tokens,
                    text_token_hash: hasher
                        .finalize()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect(),
                    visible_text_hash: visible_hasher
                        .finalize()
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect(),
                    table_fragments: page
                        .tables
                        .iter()
                        .enumerate()
                        .flat_map(|(source_index, placed)| {
                            let stacking_order = table_stacking_order(page, source_index);
                            table_manifests(
                                &placed.table,
                                placed.fragment_index,
                                BoxUnits::default(),
                                stacking_order,
                            )
                        })
                        .collect(),
                    pictures,
                    shapes,
                    unsupported_objects,
                    objects,
                }
            })
            .collect(),
    }
}

fn table_manifests(
    table: &Table,
    fragment_index: usize,
    offset: BoxUnits,
    stacking_order: i64,
) -> Vec<ManifestTable> {
    let mut result = vec![table_manifest(
        table,
        fragment_index,
        offset,
        stacking_order,
    )];
    for cell in &table.cells {
        let cell_offset = BoxUnits {
            x: offset.x + cell.box_units.x,
            y: offset.y + cell.box_units.y,
            width: 0,
            height: 0,
        };
        for nested in &cell.tables {
            result.extend(table_manifests(
                nested,
                0,
                cell_offset,
                nested.anchor.z_order,
            ));
        }
    }
    result
}

fn table_manifest(
    table: &Table,
    fragment_index: usize,
    offset: BoxUnits,
    stacking_order: i64,
) -> ManifestTable {
    ManifestTable {
        source_id: table.id.clone(),
        source_xml_path: table.source_path.clone(),
        fragment_index,
        stacking_order,
        box_units: box_manifest(add_offset(table.box_units, offset)),
        cells: table
            .cells
            .iter()
            .enumerate()
            .map(|(occurrence, cell)| ManifestCell {
                source_id: cell.id.clone(),
                source_xml_path: table.source_path.clone(),
                row: cell.row,
                column: cell.column,
                row_span: cell.row_span,
                col_span: cell.col_span,
                box_units: box_manifest(add_offset(cell.box_units, offset)),
                repeated_header: cell.repeated_header,
                repeated_from: cell.repeated_from.clone(),
                occurrence,
            })
            .collect(),
    }
}

fn table_stacking_order(page: &crate::model::LayoutPage, source_index: usize) -> i64 {
    let mut order = page
        .tables
        .iter()
        .enumerate()
        .map(|(index, placed)| (placed.table.anchor.z_order, placed.fragment_index, index))
        .collect::<Vec<_>>();
    order.sort_unstable();
    order
        .iter()
        .position(|(_, _, index)| *index == source_index)
        .map_or(source_index as i64, |index| index as i64)
}

fn add_offset(value: BoxUnits, offset: BoxUnits) -> BoxUnits {
    BoxUnits {
        x: value.x + offset.x,
        y: value.y + offset.y,
        width: value.width,
        height: value.height,
    }
}

fn object_manifest(object: &PositionedObject) -> ManifestObject {
    ManifestObject {
        id: object.id.clone(),
        source_xml_path: object.source_path.clone(),
        kind: object.kind.clone(),
        box_units: box_manifest(object.box_units),
        stacking_order: object.stacking_order,
        stacking_context: None,
    }
}

fn append_object_manifest(
    object: &PositionedObject,
    pictures: &mut Vec<ManifestObject>,
    shapes: &mut Vec<ManifestObject>,
    unsupported_objects: &mut Vec<ManifestObject>,
    objects: &mut Vec<ManifestObject>,
) {
    let entry = object_manifest(object);
    if object.kind == "pic" {
        pictures.push(entry.clone());
    } else if matches!(
        object.kind.as_str(),
        "line" | "rect" | "polygon" | "container"
    ) {
        shapes.push(entry.clone());
    } else {
        unsupported_objects.push(entry.clone());
    }
    objects.push(entry);
    for child in &object.children {
        let mut positioned = child.clone();
        positioned.box_units.x += object.box_units.x;
        positioned.box_units.y += object.box_units.y;
        append_object_manifest(&positioned, pictures, shapes, unsupported_objects, objects);
    }
}

fn append_table_objects(
    table: &Table,
    pictures: &mut Vec<ManifestObject>,
    shapes: &mut Vec<ManifestObject>,
    unsupported_objects: &mut Vec<ManifestObject>,
    objects: &mut Vec<ManifestObject>,
) {
    for cell in &table.cells {
        for paragraph in &cell.paragraphs {
            for object in &paragraph.objects {
                append_object_manifest(object, pictures, shapes, unsupported_objects, objects);
            }
        }
        for nested in &cell.tables {
            append_table_objects(nested, pictures, shapes, unsupported_objects, objects);
        }
    }
}

fn box_manifest(value: BoxUnits) -> ManifestBox {
    ManifestBox {
        x: hwp_to_centi_mm(value.x),
        y: hwp_to_centi_mm(value.y),
        width: hwp_to_centi_mm(value.width),
        height: hwp_to_centi_mm(value.height),
    }
}

pub fn write_manifest(manifest: &Manifest, path: &Path) -> Result<()> {
    let text = serde_json::to_string_pretty(manifest)?;
    std::fs::write(path, format!("{text}\n"))?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
