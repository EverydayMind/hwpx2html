//! Shared conversion engine, without file writes, threads or browser code.
use std::path::Path;

use serde::Serialize;

use crate::diagnostic::{self, Diagnostic};
use crate::error::{ConvertError, Result};
use crate::hwpx::package::Limits;
use crate::layout::{hwp_to_centi_mm, layout_document};
use crate::model::{Block, Document, Paragraph, PositionedObject, Table};
use crate::render::emit::render_direct_lenient;
use crate::render::{render_bundle, RenderBundle, RenderOptions};

#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    pub render: RenderOptions,
    pub limits: Limits,
    pub strict: bool,
}

#[derive(Debug, Serialize)]
pub struct PaperSize {
    pub page: usize,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, Serialize)]
pub struct Summary {
    pub version: &'static str,
    pub title: String,
    pub input_sha256: String,
    pub page_count: usize,
    pub papers: Vec<PaperSize>,
    /// CSS pixels including the body's top gap, page borders and bottom gap.
    pub document_height_px: f64,
    pub paged_height_px: f64,
    /// Unique raster resources in the output, not object instances.
    pub image_count: usize,
    /// Source objects outside PositionedObject::is_drawn, as in CLI reports.
    pub unsupported_objects: usize,
    /// Emitter omissions/simplifications; distinct from unsupported_objects.
    pub skipped_parts: usize,
}

#[derive(Debug)]
pub struct Conversion {
    pub bundle: RenderBundle,
    pub diagnostics: Vec<Diagnostic>,
    pub summary: Summary,
}

#[derive(Debug)]
pub enum ConvertOutcome {
    Converted(Box<Conversion>),
    Rejected {
        diagnostics: Vec<Diagnostic>,
        unsupported_objects: usize,
    },
}

/// Convert bytes with the default direct writer. `name` is the source file
/// label (and supplies its stem for the title); it is never opened as a path.
/// Diagnostic page-by-page output remains a CLI-only path.
pub fn convert_bytes(
    name: &str,
    bytes: Vec<u8>,
    options: &ConvertOptions,
) -> Result<ConvertOutcome> {
    convert_input(Path::new(name), bytes, options, "resources", false)
}

pub(crate) fn convert_input(
    name: &Path,
    bytes: Vec<u8>,
    options: &ConvertOptions,
    resource_directory: &str,
    page_by_page: bool,
) -> Result<ConvertOutcome> {
    let document = crate::read_document_bytes(name, bytes, options.limits.clone())?;
    let unsupported_objects = unsupported_count(&document);
    let mut diagnostics = diagnostic::reader_diagnostics(&document);
    if options.strict && unsupported_objects > 0 {
        // Preserve CLI's early rejection: no layout or emitter work. The UI
        // can still identify offending source objects, with unknown pages.
        for object in unsupported_objects_in(&document) {
            diagnostics.push(Diagnostic::unsupported_object(object));
        }
        return Ok(ConvertOutcome::Rejected {
            diagnostics,
            unsupported_objects,
        });
    }
    let layout = layout_document(&document);
    let pages = diagnostic::first_pages(&document, &layout);
    for diagnostic in &mut diagnostics {
        diagnostic.page = diagnostic
            .key
            .as_ref()
            .and_then(|key| pages.get(key).copied());
    }
    let render = RenderOptions {
        source_name: name
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned()),
        ..options.render.clone()
    };
    let (bundle, skipped_parts) = if page_by_page {
        (render_bundle(&layout, &render, resource_directory), 0)
    } else {
        let (bundle, report) =
            render_direct_lenient(&document, &layout, &render, resource_directory, false)
                .map_err(|error| ConvertError::UnwritableContent(error.to_string()))?;
        let skipped_parts = report.skipped.len();
        diagnostics.extend(
            report
                .skipped
                .iter()
                .map(|part| Diagnostic::skipped(part, &pages)),
        );
        if options.strict && skipped_parts > 0 {
            return Ok(ConvertOutcome::Rejected {
                diagnostics,
                unsupported_objects,
            });
        }
        (bundle, skipped_parts)
    };
    let (document_height_px, paged_height_px) = crate::render::emit::screen_extents(&layout);
    let summary = Summary {
        version: env!("CARGO_PKG_VERSION"),
        title: render.source_name.unwrap_or_else(|| document.title.clone()),
        input_sha256: document.input_sha256,
        page_count: layout.pages.len(),
        papers: layout
            .pages
            .iter()
            .enumerate()
            .map(|(index, page)| PaperSize {
                page: index + 1,
                width_mm: hwp_to_centi_mm(page.spec.width) as f64 / 100.0,
                height_mm: hwp_to_centi_mm(page.spec.height) as f64 / 100.0,
            })
            .collect(),
        document_height_px,
        paged_height_px,
        image_count: bundle
            .resources
            .values()
            .filter(|resource| {
                matches!(
                    resource.mime.as_str(),
                    "image/png" | "image/jpeg" | "image/gif" | "image/bmp"
                )
            })
            .count(),
        unsupported_objects,
        skipped_parts,
    };
    Ok(ConvertOutcome::Converted(Box::new(Conversion {
        bundle,
        diagnostics,
        summary,
    })))
}

fn collect_paragraph<'a>(
    document: &Document,
    paragraph: &'a Paragraph,
    result: &mut Vec<&'a PositionedObject>,
) {
    for object in &paragraph.objects {
        collect_object(document, object, result);
    }
}

fn collect_table<'a>(
    document: &Document,
    table: &'a Table,
    result: &mut Vec<&'a PositionedObject>,
) {
    if let Some(caption) = table.caption.as_deref() {
        for paragraph in &caption.paragraphs {
            collect_paragraph(document, paragraph, result);
        }
    }
    for cell in &table.cells {
        for paragraph in &cell.paragraphs {
            collect_paragraph(document, paragraph, result);
        }
        for table in &cell.tables {
            collect_table(document, table, result);
        }
    }
}

fn collect_object<'a>(
    document: &Document,
    object: &'a PositionedObject,
    result: &mut Vec<&'a PositionedObject>,
) {
    if !object.is_drawn(&document.assets) {
        result.push(object);
    }
    if let Some(caption) = object.caption.as_deref() {
        for paragraph in &caption.paragraphs {
            collect_paragraph(document, paragraph, result);
        }
    }
    if let Some(shape) = object.shape.as_deref() {
        for paragraph in &shape.paragraphs {
            collect_paragraph(document, paragraph, result);
        }
        for table in &shape.tables {
            collect_table(document, table, result);
        }
    }
    for child in &object.children {
        collect_object(document, child, result);
    }
}

fn unsupported_objects_in(document: &Document) -> Vec<&PositionedObject> {
    let mut result = Vec::new();
    for block in document.sections.iter().flat_map(|section| &section.blocks) {
        match block {
            Block::Paragraph(paragraph) => collect_paragraph(document, paragraph, &mut result),
            Block::Table(table) => collect_table(document, table, &mut result),
            Block::Object(object) => collect_object(document, object, &mut result),
        }
    }
    result
}

/// Same recursive source-object support definition used by convert/inspect.
pub fn unsupported_count(document: &Document) -> usize {
    unsupported_objects_in(document).len()
}
