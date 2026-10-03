pub mod assets;
pub mod batch;
pub mod cli;
pub mod error;
pub mod hwpx;
pub mod layout;
pub mod manifest;
pub mod model;
pub mod render;
pub mod semantic;

use std::path::Path;

use crate::error::{ConvertError, Result};
use crate::hwpx::header::HeaderStyles;
use crate::hwpx::package::{Limits, Package, MIB};
use crate::hwpx::section::parse_section;
use crate::hwpx::spine::Spine;
use crate::hwpx::DocumentReader;
use crate::model::{AssetRef, Document, PageNumberPolicy};

#[derive(Debug, Clone)]
pub struct HwpxReader {
    pub path: std::path::PathBuf,
    pub limits: Limits,
}

impl HwpxReader {
    pub fn new(path: impl AsRef<Path>, limits: Limits) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            limits,
        }
    }
}

impl DocumentReader for HwpxReader {
    fn read(&self) -> Result<Document> {
        let package = Package::open(&self.path, &self.limits)?;
        let estimated = estimate_memory(&package);
        if estimated > self.limits.memory_budget_bytes {
            return Err(ConvertError::MemoryBudgetExceeded {
                estimated: estimated / MIB,
                budget: self.limits.memory_budget_bytes / MIB,
            });
        }
        let mut header = HeaderStyles::parse(package.required("Contents/header.xml")?)?;
        let spine = Spine::parse(&package)?;
        let section_bytes = spine
            .sections
            .iter()
            .map(|item| Ok((item.path.clone(), package.required(&item.path)?)))
            .collect::<Result<Vec<_>>>()?;
        let redundant_char_styles =
            crate::hwpx::section::collect_redundant_page_number_styles(&section_bytes)?;
        let font_alias_char_styles = std::mem::take(&mut header.char_style_font_aliases);
        header.compact_char_styles(&redundant_char_styles, &font_alias_char_styles);
        let duplicate_para_styles =
            crate::hwpx::section::collect_duplicate_picture_only_para_styles(
                package.required("Contents/header.xml")?,
                &section_bytes,
            )?;
        header.compact_para_styles(&duplicate_para_styles);
        let mut sections = Vec::with_capacity(spine.sections.len());
        let mut warnings = Vec::new();
        let mut page_numbers = PageNumberPolicy {
            start: 1,
            format: "DIGIT".to_owned(),
            side_char: "-".to_owned(),
            ..PageNumberPolicy::default()
        };
        for (index, item) in spine.sections.iter().enumerate() {
            let bytes = package.required(&item.path)?;
            let parsed = parse_section(bytes, index, &item.path, &header)?;
            if parsed.has_page_number_control {
                page_numbers.enabled = true;
            }
            if let Some(format) = parsed.page_number_format.filter(|value| !value.is_empty()) {
                page_numbers.format = format;
            }
            if let Some(side) = parsed.page_number_side.filter(|value| !value.is_empty()) {
                page_numbers.side_char = side;
            }
            let missing_lines = parsed
                .section
                .blocks
                .iter()
                .filter_map(|block| match block {
                    crate::model::Block::Paragraph(paragraph) if paragraph.lines.is_empty() => {
                        Some(paragraph.id.clone())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if !missing_lines.is_empty() {
                warnings.push(format!("{item_path}: {} paragraph(s) have no linesegarray; rendered as one unwrapped line", missing_lines.len(), item_path = item.path));
            }
            sections.push(parsed.section);
        }
        let assets = collect_assets(&package)?;
        let char_styles = header.char_styles.into_values().collect();
        let para_styles = header.para_styles.into_values().collect();
        let numberings = header.numberings;
        Ok(Document {
            title: if spine.title.is_empty() {
                String::new()
            } else {
                spine.title
            },
            input_sha256: package.input_sha256,
            sections,
            char_styles,
            para_styles,
            numberings,
            page_numbers,
            assets,
            warnings,
        })
    }
}

pub fn read_document(path: impl AsRef<Path>, limits: Limits) -> Result<Document> {
    HwpxReader::new(path, limits).read()
}

pub fn open_package(path: impl AsRef<Path>, limits: &Limits) -> Result<Package> {
    Package::open(path, limits)
}

fn collect_assets(package: &Package) -> Result<std::collections::BTreeMap<String, AssetRef>> {
    let bytes = package.required("Contents/content.hpf")?;
    let xml = crate::hwpx::util::parse_xml(bytes, "Contents/content.hpf")?;
    let mut assets = std::collections::BTreeMap::new();
    for item in crate::hwpx::util::descendants(xml.root_element(), "item") {
        let id = crate::hwpx::util::attr_string(item, "id");
        let href = crate::hwpx::util::attr_string(item, "href");
        let mime_type = crate::hwpx::util::attr_string(item, "media-type");
        if !id.is_empty() && !href.is_empty() && !href.ends_with(".xml") && !href.ends_with(".hpf")
        {
            let path = href
                .split('#')
                .next()
                .unwrap_or(&href)
                .trim_start_matches("./")
                .to_owned();
            if package.get(&path).is_some() {
                let data = package.get(&path).unwrap_or_default().to_vec();
                assets.insert(
                    id.clone(),
                    AssetRef {
                        id,
                        path,
                        mime_type,
                        data,
                    },
                );
            }
        }
    }
    Ok(assets)
}

fn estimate_memory(package: &Package) -> u64 {
    let image_bytes = package
        .names()
        .filter(|name| name.starts_with("BinData/"))
        .filter_map(|name| package.get(name).map(|bytes| bytes.len() as u64))
        .sum::<u64>();
    crate::hwpx::package::estimated_memory_bytes(package.unpacked_bytes, image_bytes)
}
