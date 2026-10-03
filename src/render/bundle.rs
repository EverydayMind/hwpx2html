//! One external render artifact, optionally packaged into a single HTML file.
use std::collections::BTreeMap;
use std::ops::Deref;

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::model::{AssetRef, LayoutDocument};
use crate::render::{html, RenderOptions};

#[derive(Debug, Clone)]
pub struct Resource {
    pub mime: String,
    pub data: Vec<u8>,
}

impl Resource {
    fn data_uri(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.mime,
            base64::engine::general_purpose::STANDARD.encode(&self.data)
        )
    }
}

#[derive(Debug)]
pub struct RenderBundle {
    pub html: String,
    /// Files relative to the caller's resource directory, including the CSS.
    pub resources: BTreeMap<String, Resource>,
    pub stylesheet: String,
    prefix: String,
    scripts: Vec<&'static str>,
}

impl RenderBundle {
    /// Package only this renderer's known resource references. No HTML parsing,
    /// layout, text rewriting, file reads, or network resolution happens here.
    pub fn to_single_html(&self) -> String {
        let mut urls = BTreeMap::new();
        let mut css_urls = BTreeMap::new();
        for (name, resource) in &self.resources {
            if name == &self.stylesheet {
                continue;
            }
            let uri = resource.data_uri();
            urls.insert(format!("{}/{name}", self.prefix), uri.clone());
            css_urls.insert(name.clone(), uri);
        }
        let css = std::str::from_utf8(&self.resources[&self.stylesheet].data)
            .expect("renderer-generated UTF-8 CSS");
        let css = map_urls(css, |url| css_urls.get(url).cloned());
        let result = map_urls(&self.html, |url| urls.get(url).cloned());
        result
            .replacen(&self.stylesheet_link(), &format!("<style>{css}</style>"), 1)
            .replacen(
                &html::escape_html_attribute(&html::csp_meta(&self.scripts, true)),
                &html::escape_html_attribute(&html::csp_meta(&self.scripts, false)),
                1,
            )
    }

    fn stylesheet_link(&self) -> String {
        format!(
            "<link rel=\"stylesheet\" type=\"text/css\" href=\"{}/{}\">",
            self.prefix, self.stylesheet
        )
    }
}

/// A typed insertion point in an object's paint markup. The semantic
/// emitter owns the paragraphs/tables; the painter owns only decoration.
pub(super) enum ObjectContent {
    Paragraphs(String),
    Table(String),
}

pub(super) struct RenderContext<'a> {
    #[cfg(test)]
    pub(super) observe_keys: bool,
    document: &'a LayoutDocument,
    asset_urls: BTreeMap<String, String>,
    /// Set while a semantic-mode line is written inside a `p`, whose
    /// descendants must stay phrasing content.
    pub(super) phrasing: std::cell::Cell<bool>,
    /// The inferred headings open so far, across the pages (D25).
    pub(super) outline: std::cell::RefCell<super::semantic::Outline>,
    /// The lists the last column of the page flow ended inside.
    pub(super) open_lists: std::cell::RefCell<Option<super::semantic::OpenLists>>,
    /// Record content slots instead of writing a text box's paragraphs.
    pub(super) direct_objects: std::cell::Cell<bool>,
    pub(super) object_slots: std::cell::RefCell<Vec<(usize, ObjectContent)>>,
    /// Key observations for every object, including container children.
    pub(super) direct_observe: std::cell::Cell<bool>,
}

impl Deref for RenderContext<'_> {
    type Target = LayoutDocument;
    fn deref(&self) -> &LayoutDocument {
        self.document
    }
}

impl<'a> RenderContext<'a> {
    /// A context for writers other than the page-by-page renderer (the
    /// direct emitter), with the pictures' URLs from [`picture_resources`].
    pub(super) fn new(document: &'a LayoutDocument, asset_urls: BTreeMap<String, String>) -> Self {
        Self {
            #[cfg(test)]
            observe_keys: false,
            document,
            asset_urls,
            phrasing: std::cell::Cell::new(false),
            outline: Default::default(),
            open_lists: Default::default(),
            direct_objects: std::cell::Cell::new(false),
            object_slots: Default::default(),
            direct_observe: std::cell::Cell::new(false),
        }
    }

    pub(super) fn asset_url(&self, asset: &AssetRef) -> Option<&str> {
        self.asset_urls.get(&asset.path).map(String::as_str)
    }
}

fn resource_name(data: &[u8], extension: &str) -> String {
    format!("{:x}.{extension}", Sha256::digest(data))
}

/// Encode one directory-name component, including Unicode, quotes, # and %.
pub(super) fn url_component(name: &str) -> String {
    let mut result = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            result.push(char::from(byte));
        } else {
            use std::fmt::Write;
            write!(result, "%{byte:02X}").unwrap();
        }
    }
    result
}

/// Visit the URL syntax emitted by our CSS/HTML serializers only: CSS
/// `url('…')` and a picture's `<img src="…">`. Fragment links such as
/// url(#w_1), text and unknown URLs are preserved verbatim.
fn map_urls(source: &str, mut replace: impl FnMut(&str) -> Option<String>) -> String {
    // One linear pass per syntax. A replacement is a data URI or a resource
    // name, so it never contains the other syntax's delimiters.
    let source = map_delimited(source, "url('", "')", &mut replace);
    map_delimited(&source, "src=\"", "\"", &mut replace)
}

fn map_delimited(
    source: &str,
    open: &str,
    close: &str,
    replace: &mut impl FnMut(&str) -> Option<String>,
) -> String {
    let mut result = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find(open) {
        let value_start = start + open.len();
        let Some(end) = rest[value_start..].find(close) else {
            break;
        };
        let value_end = value_start + end;
        result.push_str(&rest[..value_start]);
        let url = &rest[value_start..value_end];
        match replace(url) {
            Some(replacement) => result.push_str(&replacement),
            None => result.push_str(url),
        }
        result.push_str(close);
        rest = &rest[value_end + close.len()..];
    }
    result.push_str(rest);
    result
}

pub fn render_bundle(
    document: &LayoutDocument,
    options: &RenderOptions,
    resource_directory: &str,
) -> RenderBundle {
    render_bundle_impl(
        document,
        options,
        resource_directory,
        #[cfg(test)]
        false,
    )
}

pub(super) fn render_bundle_impl(
    document: &LayoutDocument,
    options: &RenderOptions,
    resource_directory: &str,
    #[cfg(test)] observe_keys: bool,
) -> RenderBundle {
    let prefix = url_component(resource_directory);
    let (resources, asset_urls) = picture_resources(document, &prefix);
    let context = RenderContext {
        #[cfg(test)]
        observe_keys,
        document,
        asset_urls,
        phrasing: std::cell::Cell::new(false),
        outline: Default::default(),
        open_lists: Default::default(),
        direct_objects: std::cell::Cell::new(false),
        object_slots: Default::default(),
        direct_observe: std::cell::Cell::new(false),
    };
    let rendered = html::render_external_document(&context, options, PENDING_LINK);
    let (rendered, style_rules) = html::styles_to_classes(&rendered);
    let css = format!(
        "{}{}{}{}{}{}",
        super::css::base_css(),
        super::css::dynamic_css(&document.char_styles, &document.para_styles),
        super::css::semantic_css(),
        if options.logical_dom {
            super::logical::logical_css()
        } else {
            ""
        },
        if options.page_navigation {
            super::css::navigation_css(options.logical_dom)
        } else {
            ""
        },
        style_rules
    );
    assemble(prefix, resources, html::scripts(options), &rendered, &css)
}

/// The link the markup carries until the stylesheet's name is known: the
/// name hashes the rules, which include the classes the markup's styles
/// become.
pub(super) const PENDING_LINK: &str = "\u{3}stylesheet\u{3}";

/// The pictures of the document as resource files, and the URL each one is
/// written under (keyed by its path in the package).
pub(super) fn picture_resources(
    document: &LayoutDocument,
    prefix: &str,
) -> (BTreeMap<String, Resource>, BTreeMap<String, String>) {
    let mut resources = BTreeMap::new();
    let mut asset_urls = BTreeMap::new();
    for asset in document.assets.values() {
        let Some(mime) = crate::assets::raster_mime(asset) else {
            continue;
        };
        let extension = match mime {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/bmp" => "bmp",
            _ => unreachable!(),
        };
        let name = resource_name(&asset.data, extension);
        asset_urls.insert(asset.path.clone(), format!("{prefix}/{name}"));
        resources.entry(name).or_insert_with(|| Resource {
            mime: mime.to_owned(),
            data: asset.data.clone(),
        });
    }
    (resources, asset_urls)
}

/// Package finished markup and stylesheet: the decoration images the CSS
/// names become files, the stylesheet is named by its hash, and its link
/// replaces [`PENDING_LINK`].
pub(super) fn assemble(
    prefix: String,
    mut resources: BTreeMap<String, Resource>,
    scripts: Vec<&'static str>,
    rendered: &str,
    css: &str,
) -> RenderBundle {
    let css = map_urls(css, |url| {
        // The CSS renderer's decoration images are trusted fixed SVGs, not
        // arbitrary SVGs supplied by a document.
        let encoded = url.strip_prefix("data:image/svg+xml;base64,")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("fixed decoration SVG base64");
        let name = resource_name(&data, "svg");
        resources.entry(name.clone()).or_insert(Resource {
            mime: "image/svg+xml".to_owned(),
            data,
        });
        Some(name)
    });
    let stylesheet = format!("style-{}", resource_name(css.as_bytes(), "css"));
    resources.insert(
        stylesheet.clone(),
        Resource {
            mime: "text/css".to_owned(),
            data: css.into_bytes(),
        },
    );
    let mut bundle = RenderBundle {
        html: String::new(),
        resources,
        stylesheet,
        prefix,
        scripts,
    };
    bundle.html = rendered.replacen(PENDING_LINK, &bundle.stylesheet_link(), 1);
    bundle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BoxUnits, CharStyle, LayoutPage, PositionedObject, ShapeStyle};

    fn png_asset() -> AssetRef {
        AssetRef {
            id: "image1".into(),
            path: "BinData/image1.png".into(),
            mime_type: "image/png".into(),
            data: b"\x89PNG\r\n\x1a\nimage payload".to_vec(),
        }
    }

    fn object(kind: &str, binary_ref: Option<&str>, shape: Option<ShapeStyle>) -> PositionedObject {
        PositionedObject {
            id: kind.into(),
            key: kind.into(),
            source_path: String::new(),
            source_anchor: None,
            caption: None,
            equation: None,
            textpos: 0,
            kind: kind.into(),
            box_units: Default::default(),
            anchor: Default::default(),
            alt: String::new(),
            description: String::new(),
            paragraph_key: String::new(),
            binary_ref: binary_ref.map(Into::into),
            mime_type: None,
            crop: None,
            img_dim: Default::default(),
            original_size: Default::default(),
            flip_x: false,
            flip_y: false,
            stacking_order: 0,
            shape: shape.map(Box::new),
            children: Vec::new(),
        }
    }

    fn page_with(objects: Vec<PositionedObject>) -> LayoutPage {
        LayoutPage {
            index: 0,
            section_index: 0,
            spec: Default::default(),
            lines: Vec::new(),
            tables: Vec::new(),
            objects,
            page_number: None,
        }
    }

    #[test]
    fn packaging_preserves_css_and_image_bytes_and_escapes_external_paths() {
        let asset = png_asset();
        let picture = object("pic", Some("image1"), None);
        let document = LayoutDocument {
            title: "Literal url('resources/example.png')".into(),
            assets: BTreeMap::from([("image1".into(), asset.clone())]),
            char_styles: vec![CharStyle {
                underline: true,
                ..Default::default()
            }],
            pages: vec![page_with(vec![picture.clone(), picture])],
            ..Default::default()
        };
        for with_script in [false, true] {
            let options = RenderOptions {
                adjust_letter_spacing: with_script,
                page_navigation: with_script,
                ..Default::default()
            };
            let bundle = render_bundle(&document, &options, "한 글#%\".html.assets");
            assert!(!bundle.html.contains("<style>"));
            assert!(!bundle.html.contains("data:image/"));
            assert!(bundle.html.contains("%20"));
            assert!(bundle.html.contains("%23%25%22.html.assets/"));
            assert_eq!(
                bundle
                    .resources
                    .values()
                    .filter(|r| r.mime == "image/png")
                    .count(),
                1
            );
            assert!(bundle.resources.values().any(|r| r.data == asset.data));
            let css = std::str::from_utf8(&bundle.resources[&bundle.stylesheet].data).unwrap();
            assert!(!css.contains("data:image/"));
            assert!(css.contains(".svg"));
            let embedded = bundle.to_single_html();
            let original_css = format!(
                "{}{}{}{}{}",
                super::super::css::base_css(),
                super::super::css::dynamic_css(&document.char_styles, &document.para_styles),
                super::super::css::semantic_css(),
                super::super::logical::logical_css(),
                if with_script {
                    super::super::css::navigation_css(true)
                } else {
                    ""
                }
            );
            // The styles of the markup follow as classes.
            assert!(embedded.contains(&format!("<style>{original_css}.z0 {{")));
            assert!(!bundle.html.contains(" style=\""));
            assert_eq!(
                embedded
                    .matches(&crate::assets::data_uri(&asset).unwrap())
                    .count(),
                2
            );
            assert!(!embedded.contains("rel=\"stylesheet\""));
            assert!(!embedded.contains(".html.assets/"));
            assert_eq!(embedded.contains("<script>"), with_script);
            assert!(embedded.contains(&html::escape_html(&document.title)));
        }
    }

    /// The one-page view adds a head script, its hash and its rules, and
    /// nothing else: the pages' markup is the same with it or without it.
    #[test]
    fn page_navigation_adds_only_a_head_script_and_its_rules() {
        let document = LayoutDocument {
            pages: vec![page_with(Vec::new()), page_with(Vec::new())],
            ..Default::default()
        };
        let render = |page_navigation| {
            let options = RenderOptions {
                page_navigation,
                ..Default::default()
            };
            render_bundle(&document, &options, "gen.html.assets").to_single_html()
        };
        let (paged, all) = (render(true), render(false));
        assert!(paged.contains(&format!(
            "<script>{}</script></head><body>",
            html::NAVIGATION_SCRIPT
        )));
        for script in [html::NAVIGATION_SCRIPT, html::SCRIPT_SOURCE] {
            assert!(paged.contains(&html::script_hash_base64(script)));
        }
        assert!(paged.contains(super::super::css::navigation_css(true)));
        assert!(!all.contains(html::NAVIGATION_SCRIPT));
        assert!(!all.contains(&html::script_hash_base64(html::NAVIGATION_SCRIPT)));
        assert!(!all.contains("hwpx-paged"));
        let body = |html: &str| html.split_once("<body>").unwrap().1.to_owned();
        assert_eq!(body(&paged), body(&all));
        // The head script does not stop the pages' styles becoming classes.
        assert_eq!(body(&paged).matches("<div class=\"hpa z").count(), 2);
        assert!(!body(&paged).contains(" style=\""));
    }

    /// Printing must find every page shown before the letter-spacing script
    /// measures them. That script's `beforeprint` listener is added as soon as
    /// the fonts are ready, which can be before the document has loaded, so the
    /// navigation script adds the one that shows the pages when it first runs.
    #[test]
    fn navigation_shows_the_pages_for_printing_before_the_document_has_loaded() {
        let script = html::NAVIGATION_SCRIPT;
        let show = script.find("addEventListener('beforeprint'").unwrap();
        assert!(show < script.find("addEventListener('DOMContentLoaded'").unwrap());
        assert_eq!(script.matches("addEventListener('beforeprint'").count(), 1);
    }

    #[test]
    fn navigation_bar_css_hidden_by_default_and_revealed_on_hover_or_touch() {
        let bar_css = super::super::css::navigation_bar_css();
        assert!(bar_css.contains("opacity:0;pointer-events:none;transition:opacity .15s;"));
        assert!(bar_css.contains(
            ".hwpx-nav::before {content:'';position:absolute;left:0;right:0;top:0;height:12px;pointer-events:auto;}"
        ));
        assert!(bar_css
            .contains(".hwpx-nav:hover, .hwpx-nav.hwpx-show {opacity:1;pointer-events:auto;}"));
        assert!(bar_css
            .contains(".hwpx-nav:hover::before, .hwpx-nav.hwpx-show::before {content:none;}"));
        assert!(bar_css.contains("@media (pointer:coarse) {.hwpx-nav::before {height:100%;}}"));
    }

    #[test]
    fn navigation_css_has_no_padding_offset_and_centres_at_top() {
        for logical in [true, false] {
            let css = super::super::css::navigation_css(logical);
            assert!(!css.contains("2mm + 40px"));
        }
        let logical_css = super::super::css::navigation_css(true);
        assert!(logical_css.contains("top:calc(2mm + 1px) !important;"));
    }

    #[test]
    fn navigation_scripts_share_click_and_focus_handlers_before_afterprint() {
        let html_script = html::NAVIGATION_SCRIPT;
        let emit_script = super::super::emit::NAVIGATION_SCRIPT;

        for script in [html_script, emit_script] {
            let pointerdown = script
                .find("addEventListener('pointerdown'")
                .expect("pointerdown listener");
            let click = script
                .find("addEventListener('click',e=>{")
                .expect("click listener");
            let focusin = script
                .find("nav.addEventListener('focusin'")
                .expect("focusin listener");
            let focusout = script
                .find("nav.addEventListener('focusout'")
                .expect("focusout listener");
            let afterprint = script
                .find("addEventListener('afterprint'")
                .expect("afterprint listener");

            assert!(pointerdown < click);
            assert!(click < focusin);
            assert!(focusin < focusout);
            assert!(focusout < afterprint);
        }

        fn extract_handlers(script: &str) -> &str {
            let start = script.find("let touch=false;").expect("handler start");
            let end = script
                .find("addEventListener('afterprint'")
                .expect("afterprint pos");
            &script[start..end]
        }
        assert_eq!(extract_handlers(html_script), extract_handlers(emit_script));
    }

    /// 성과보고서's chart images are rects filled with a picture, written as
    /// an `<img src>` rather than a CSS `url('…')`. The single-file package
    /// must inline them too, or the embedded CSP (`img-src data:`) blocks
    /// every one.
    #[test]
    fn packaging_embeds_the_picture_of_an_image_filled_shape() {
        let asset = png_asset();
        let mut chart = object(
            "rect",
            None,
            Some(ShapeStyle {
                points: Vec::new(),
                fill: None,
                image_fill: Some("image1".into()),
                gradient: Vec::new(),
                gradient_step: 0,
                gradient_angle: 0,
                line_color: String::new(),
                line_width: 0,
                declared_line_width: 28,
                corner_ratio: 0,
                paragraphs: Vec::new(),
                tables: Vec::new(),
                margins: [0; 4],
                vertical_align: String::new(),
            }),
        );
        chart.box_units = BoxUnits {
            x: 0,
            y: 0,
            width: 9921,
            height: 8504,
        };
        let document = LayoutDocument {
            assets: BTreeMap::from([("image1".into(), asset.clone())]),
            pages: vec![page_with(vec![chart])],
            ..Default::default()
        };
        let bundle = render_bundle(&document, &RenderOptions::default(), "gen.html.assets");
        // The picture's own placement is a class of the stylesheet.
        assert!(bundle.html.contains("<img class=\"hpi z"));
        assert!(bundle.html.contains(" src=\"gen.html.assets/"));
        assert!(bundle.html.contains("alt=\"그림\""));
        let embedded = bundle.to_single_html();
        let uri = crate::assets::data_uri(&asset).unwrap();
        assert!(embedded.contains(&format!("src=\"{uri}\"")));
        assert!(!embedded.contains(".html.assets/"));
    }
}
