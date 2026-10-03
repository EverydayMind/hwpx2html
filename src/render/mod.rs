pub mod bundle;
pub mod css;
pub mod emit;
pub mod html;
mod logical;
#[cfg(test)]
mod observation;
mod offsets;
mod reading;
mod semantic;
mod units;

pub use bundle::{render_bundle, RenderBundle};

use crate::model::LayoutDocument;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedPolicy {
    Placeholder,
    Skip,
}

#[derive(Debug, Clone)]
pub struct RenderOptions {
    pub unsupported: UnsupportedPolicy,
    pub adjust_letter_spacing: bool,
    /// Show one page at a time with a navigation bar (a head script; every
    /// page shows without it, and printing always prints every page).
    pub page_navigation: bool,
    /// Also tag headings and lists implied by paragraph markers (D25; on
    /// by default since 2026-09-26).
    pub infer_structure: bool,
    /// Write one logical DOM over the pages (D35): a paragraph or table
    /// crossing pages is one element, its parts drawn on their pages.
    pub logical_dom: bool,
    /// Include original/reading views over the same logical DOM (D38).
    /// Opt-in until the reading-view regression gates have passed.
    pub reading_view: bool,
    /// The input file name, used as the document title (D30).
    pub source_name: Option<String>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            unsupported: UnsupportedPolicy::Placeholder,
            adjust_letter_spacing: true,
            page_navigation: true,
            infer_structure: true,
            logical_dom: true,
            reading_view: false,
            source_name: None,
        }
    }
}

pub fn render(document: &LayoutDocument, options: &RenderOptions) -> String {
    render_bundle(document, options, "resources").to_single_html()
}
