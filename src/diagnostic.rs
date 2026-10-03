//! Sidecar diagnostics. Their display text never enters the generated HTML.
pub mod reasons;

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::{Block, Document, LayoutDocument, Paragraph, PositionedObject, Table};
use crate::render::emit::Unsupported;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Warning,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub key: Option<String>,
    /// Physical page, from 1 (not the printed page number).
    pub page: Option<usize>,
    pub detail: String,
    pub message: String,
    pub action: String,
    // Several paragraph diagnostics can represent one existing CLI warning.
    #[serde(skip)]
    legacy: Option<String>,
}

impl Diagnostic {
    /// Existing JSONL/stderr text, once per original warning, in original order.
    pub fn legacy_text(&self) -> Option<&str> {
        self.legacy.as_deref()
    }

    pub(crate) fn skipped(part: &Unsupported, pages: &BTreeMap<String, usize>) -> Self {
        let reason = reasons::ALL
            .iter()
            .find(|reason| reason.detail == part.what);
        Self {
            code: reason.map_or("unrecognized_content", |reason| reason.code),
            severity: Severity::Warning,
            key: (!part.key.is_empty()).then(|| part.key.clone()),
            page: pages.get(&part.key).copied(),
            detail: part.what.to_owned(),
            message: reason
                .map_or(
                    "일부 내용을 출력하지 못했거나 단순하게 표시했습니다.",
                    |reason| reason.message,
                )
                .to_owned(),
            action: reason
                .map_or(
                    "원본과 미리보기를 대조해 주세요. 아래 원문 사유를 함께 확인해 주세요.",
                    |reason| reason.action,
                )
                .to_owned(),
            legacy: Some(format!("skipped: {part}")),
        }
    }

    pub(crate) fn unsupported_object(object: &PositionedObject) -> Self {
        Self {
            code: "unsupported_object",
            severity: Severity::Warning,
            key: Some(object.key.clone()),
            page: None,
            detail: object.kind.clone(),
            message: "지원하지 않는 개체 때문에 엄격 모드에서 변환을 중단했습니다.".into(),
            action: "한글에서 해당 개체를 지원하는 그림으로 바꾸거나 엄격 모드를 끄고 결과를 확인해 주세요.".into(),
            legacy: None,
        }
    }
}

pub(crate) fn reader_diagnostics(document: &Document) -> Vec<Diagnostic> {
    let mut result = Vec::new();
    // Keep Document.warnings and inspect's existing public contract intact.
    // These keys come from the same top-level paragraphs the reader counts.
    for warning in &document.warnings {
        let missing = document.sections.iter().find_map(|section| {
            let paragraphs = section
                .blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) if paragraph.lines.is_empty() => Some(paragraph),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let legacy = format!(
                "{}: {} paragraph(s) have no linesegarray; rendered as one unwrapped line",
                section.source_path,
                paragraphs.len()
            );
            (!paragraphs.is_empty() && warning == &legacy).then_some(paragraphs)
        });
        if let Some(paragraphs) = missing {
            for (index, paragraph) in paragraphs.into_iter().enumerate() {
                result.push(Diagnostic {
                    code: "missing_line_positions",
                    severity: Severity::Warning,
                    key: Some(paragraph.key.clone()),
                    page: None,
                    detail: warning.clone(),
                    message: "줄 위치 정보가 없는 문단을 한 줄로 표시했습니다.".into(),
                    action: "줄바꿈과 배치를 원본과 대조해 주세요. 필요하면 한글에서 다시 저장해 주세요.".into(),
                    legacy: (index == 0).then(|| warning.clone()),
                });
            }
        } else {
            result.push(Diagnostic {
                code: "reader_warning",
                severity: Severity::Warning,
                key: None,
                page: None,
                detail: warning.clone(),
                message: "문서를 읽는 중 확인이 필요한 항목이 발견되었습니다.".into(),
                action: "원문 사유와 미리보기를 확인해 주세요.".into(),
                legacy: Some(warning.clone()),
            });
        }
    }
    result
}

fn remember(pages: &mut BTreeMap<String, usize>, key: &str, page: usize) {
    if !key.is_empty() {
        pages.entry(key.to_owned()).or_insert(page);
    }
}

fn paragraph_page(pages: &mut BTreeMap<String, usize>, paragraph: &Paragraph, page: usize) {
    remember(pages, &paragraph.key, page);
    let own = pages.get(&paragraph.key).copied().unwrap_or(page);
    for object in &paragraph.objects {
        object_page(pages, object, own);
    }
}

fn table_page(pages: &mut BTreeMap<String, usize>, table: &Table, page: usize) {
    remember(pages, &table.id, page);
    if let Some(caption) = table.caption.as_deref() {
        remember(pages, &format!("{}/caption", table.id), page);
        for paragraph in &caption.paragraphs {
            paragraph_page(pages, paragraph, page);
        }
    }
    for cell in &table.cells {
        let own = pages.get(&cell.id).copied().unwrap_or(page);
        remember(pages, &cell.id, own);
        for paragraph in &cell.paragraphs {
            paragraph_page(pages, paragraph, own);
        }
        for nested in &cell.tables {
            table_page(pages, nested, own);
        }
    }
}

fn object_page(pages: &mut BTreeMap<String, usize>, object: &PositionedObject, page: usize) {
    remember(pages, &object.key, page);
    let own = pages.get(&object.key).copied().unwrap_or(page);
    if let Some(caption) = object.caption.as_deref() {
        remember(pages, &format!("{}/caption", object.key), own);
        for paragraph in &caption.paragraphs {
            paragraph_page(pages, paragraph, own);
        }
    }
    if let Some(shape) = object.shape.as_deref() {
        for paragraph in &shape.paragraphs {
            paragraph_page(pages, paragraph, own);
        }
        for table in &shape.tables {
            table_page(pages, table, own);
        }
    }
    for child in &object.children {
        object_page(pages, child, own);
    }
}

/// Exact placements first; nested content falls back to its placed owner.
/// No key-prefix parsing or ordinal guessing. Unknown placements stay None.
pub(crate) fn first_pages(document: &Document, layout: &LayoutDocument) -> BTreeMap<String, usize> {
    let mut pages = BTreeMap::new();
    for (index, page) in layout.pages.iter().enumerate() {
        let number = index + 1;
        for line in &page.lines {
            remember(&mut pages, &line.paragraph_key, number);
            for object in &line.inline_objects {
                remember(&mut pages, &object.key, number);
            }
        }
        for object in &page.objects {
            remember(&mut pages, &object.key, number);
        }
        for placed in &page.tables {
            remember(&mut pages, &placed.table.id, number);
            for cell in &placed.table.cells {
                remember(&mut pages, &cell.id, number);
            }
        }
    }
    for (index, page) in layout.pages.iter().enumerate() {
        let number = index + 1;
        for line in &page.lines {
            for object in &line.inline_objects {
                object_page(&mut pages, object, number);
            }
        }
        for object in &page.objects {
            object_page(&mut pages, object, number);
        }
        for placed in &page.tables {
            table_page(&mut pages, &placed.table, number);
        }
    }
    for block in document.sections.iter().flat_map(|section| &section.blocks) {
        match block {
            Block::Paragraph(paragraph) => {
                if let Some(page) = pages.get(&paragraph.key).copied() {
                    paragraph_page(&mut pages, paragraph, page);
                }
            }
            Block::Table(table) => {
                let page = pages.get(&table.id).copied().or_else(|| {
                    table
                        .source_anchor
                        .as_ref()
                        .and_then(|anchor| pages.get(&anchor.paragraph_key).copied())
                });
                if let Some(page) = page {
                    table_page(&mut pages, table, page);
                }
            }
            Block::Object(object) => {
                let page = pages.get(&object.key).copied().or_else(|| {
                    object
                        .source_anchor
                        .as_ref()
                        .and_then(|anchor| pages.get(&anchor.paragraph_key).copied())
                });
                if let Some(page) = page {
                    object_page(&mut pages, object, page);
                }
            }
        }
    }
    pages
}
