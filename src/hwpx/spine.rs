use std::collections::BTreeMap;

use crate::error::{ConvertError, Result};
use crate::hwpx::package::{normalize_entry_name, Package};
use crate::hwpx::util::{attr_string, children, descendants, parse_xml, text_content};

#[derive(Debug, Clone)]
pub struct SpineItem {
    pub id: String,
    pub path: String,
    pub media_type: String,
}

#[derive(Debug, Clone, Default)]
pub struct Spine {
    pub title: String,
    pub sections: Vec<SpineItem>,
}

impl Spine {
    pub fn parse(package: &Package) -> Result<Self> {
        let bytes = package.required("Contents/content.hpf")?;
        let xml = parse_xml(bytes, "Contents/content.hpf")?;
        let root = xml.root_element();
        let title = descendants(root, "title")
            .next()
            .map(text_content)
            .unwrap_or_default();

        let mut manifest = BTreeMap::new();
        for item in descendants(root, "item") {
            let id = attr_string(item, "id");
            let href = attr_string(item, "href");
            if id.is_empty() || href.is_empty() {
                continue;
            }
            let path = resolve_href(&href)?;
            manifest.insert(
                id,
                SpineItem {
                    id: String::new(),
                    path,
                    media_type: attr_string(item, "media-type"),
                },
            );
        }

        let spine = descendants(root, "spine")
            .next()
            .ok_or_else(|| ConvertError::MissingEntry("opf:spine".to_owned()))?;
        let mut sections = Vec::new();
        for itemref in children(spine, "itemref") {
            let idref = attr_string(itemref, "idref");
            let item = manifest
                .get(&idref)
                .ok_or_else(|| ConvertError::InvalidValue {
                    path: "Contents/content.hpf".to_owned(),
                    message: format!("spine references unknown manifest item {idref}"),
                })?;
            if item.path.ends_with("header.xml") || item.media_type.contains("header") {
                continue;
            }
            // The spine can also list the document's scripts
            // (`Scripts/headerScripts`, `application/x-javascript
            // ;charset=utf-16`), which are not sections. rhwp likewise reads
            // only `application/xml` spine items as sections.
            if !item.media_type.is_empty() && !item.media_type.to_ascii_lowercase().contains("xml")
            {
                continue;
            }
            let mut item = item.clone();
            item.id = idref;
            sections.push(item);
        }
        if sections.is_empty() {
            return Err(ConvertError::MissingEntry(
                "sectionN.xml in opf:spine".to_owned(),
            ));
        }
        Ok(Self { title, sections })
    }
}

fn resolve_href(href: &str) -> Result<String> {
    let href = href.split('#').next().unwrap_or(href).trim();
    if href.contains('\\') || href.starts_with('/') || href.contains('\0') {
        return Err(ConvertError::UnsafeZipPath(href.to_owned()));
    }
    normalize_entry_name(href)
}

#[cfg(test)]
mod tests {
    use crate::hwpx::package::Package;

    #[test]
    fn scripts_in_the_spine_are_not_sections() {
        // "(별첨1)산업통상부 하반기 업무보고.hwpx" lists its UTF-16 scripts in
        // the spine after the section; reading them as XML failed the whole
        // conversion ("unknown token at 1:1").
        let content = br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf/"><opf:metadata><opf:title>t</opf:title></opf:metadata><opf:manifest><opf:item id="header" href="Contents/header.xml" media-type="application/xml"/><opf:item id="section0" href="Contents/section0.xml" media-type="application/xml"/><opf:item id="headersc" href="Scripts/headerScripts" media-type="application/x-javascript ;charset=utf-16"/><opf:item id="sourcesc" href="Scripts/sourceScripts" media-type="application/x-javascript ;charset=utf-16"/></opf:manifest><opf:spine><opf:itemref idref="header" linear="yes"/><opf:itemref idref="section0" linear="yes"/><opf:itemref idref="headersc" linear="yes"/><opf:itemref idref="sourcesc" linear="yes"/></opf:spine></opf:package>"#;
        let package = Package::from_entries(&[("Contents/content.hpf", content)]);
        let spine = super::Spine::parse(&package).unwrap();
        let paths = spine
            .sections
            .iter()
            .map(|item| item.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["Contents/section0.xml"]);
    }
}
