//! D38: a second presentation of the direct writer's single logical DOM.
//! These rules are opt-in and do not participate in source page geometry.
use std::collections::BTreeMap;

use crate::layout::hwp_to_centi_mm;
use crate::model::{Block, Document, HwpUnit, Paragraph, PositionedObject, Table, TableCell};

use super::css::safe_color;

/// Shared exclusions: drawings and spatial inline annotations retain their
/// own coordinate systems. Never flatten a drawing merely because its SVG
/// is aria-hidden (the drawing can still carry information).
const FLOW: &str =
    ":not([data-hwpx-island],[data-hwpx-island] *,.heq,.heq *,.hco,.hco *,.hdu,.hdu *)";

pub(super) fn css(document: &Document) -> String {
    let mut weights = BTreeMap::<HwpUnit, usize>::new();
    let sizes: BTreeMap<_, _> = document
        .char_styles
        .iter()
        .map(|style| (style.id, style.font_size_hwp))
        .collect();
    // Body paragraphs, rather than headings' maximum size or the header's
    // first style, determine the reading baseline. Fall back to all content
    // for a document made exclusively of tables/drawings.
    for section in &document.sections {
        for block in &section.blocks {
            if let Block::Paragraph(paragraph) = block {
                weigh(paragraph, &sizes, &mut weights);
            }
        }
    }
    let paragraphs = paragraphs(document);
    if weights.is_empty() {
        for paragraph in &paragraphs {
            weigh(paragraph, &sizes, &mut weights);
        }
    }
    let base = modal_size(&weights).unwrap_or(1000).max(1) as f64;
    let mut output = BASE_CSS.to_owned();
    output.push_str("@media screen {\n");
    output.push_str(&format!(
        "html.hwpx-read main.hwpx-doc *{FLOW} {{position:static !important;inset:auto !important;margin:0 !important;padding:0 !important;width:auto !important;height:auto !important;min-width:0 !important;max-width:none !important;min-height:0 !important;max-height:none !important;clip-path:none !important;overflow:visible !important;transform:none !important;contain:none !important;white-space:normal !important;letter-spacing:normal !important;word-spacing:normal !important;line-height:inherit !important;pointer-events:auto !important;box-sizing:border-box !important;float:none !important;clear:none !important;}}\n"
    ));
    output.push_str(FLOW_CSS);
    for style in &document.char_styles {
        let ratio = (style.font_size_hwp as f64 / base).clamp(0.85, 1.6);
        let ratio = if style.superscript || style.subscript {
            ratio * 0.65
        } else {
            ratio
        };
        output.push_str(&format!(
            "html.hwpx-read main.hwpx-doc .cs{}{FLOW} {{font-family:system-ui,sans-serif !important;font-size:calc(var(--hwpx-read-size,18px) * {ratio:.4}) !important;vertical-align:{} !important;",
            style.id,
            if style.superscript { "super" } else if style.subscript { "sub" } else { "baseline" }
        ));
        if style.underline || style.strike {
            output.push_str("text-decoration-line:");
            if style.underline {
                output.push_str("underline ");
            }
            if style.strike {
                output.push_str("line-through ");
            }
            let color = style
                .underline_color
                .as_deref()
                .filter(|_| style.underline)
                .or(style.strike_color.as_deref())
                .unwrap_or(&style.color);
            output.push_str(&format!(";text-decoration-color:{};", safe_color(color)));
        }
        output.push_str("}\n");
        if style.underline || style.strike {
            output.push_str(&format!(
                "html.hwpx-read main.hwpx-doc .cs{}{FLOW}::after {{display:none !important;}}\n",
                style.id
            ));
        }
    }
    let mut para_weights = BTreeMap::<u32, BTreeMap<HwpUnit, usize>>::new();
    for paragraph in &paragraphs {
        weigh(
            paragraph,
            &sizes,
            para_weights.entry(paragraph.para_style_id).or_default(),
        );
    }
    for style in &document.para_styles {
        let unit = para_weights
            .get(&style.id)
            .and_then(modal_size)
            .unwrap_or(base as i64)
            .max(1) as f64;
        let indent = (style.hanging_indent.max(0) as f64 / unit).min(3.0);
        let first_indent = (style.first_line_indent.max(0) as f64 / unit).min(3.0) - indent;
        let left = (style.margin_left.max(0) as f64 / unit).min(3.0);
        let right = (style.margin_right.max(0) as f64 / unit).min(1.5);
        let before = (style.margin_before.max(0) as f64 / unit).min(1.5);
        let after = (style.margin_after.max(0) as f64 / unit).clamp(0.5, 1.5);
        let align = match style.align.to_ascii_lowercase().as_str() {
            "center" => "center",
            "right" => "right",
            _ => "start",
        };
        output.push_str(&format!(
            "html.hwpx-read main.hwpx-doc .hwpx-rp{}{FLOW} {{padding-left:{:.4}em !important;padding-right:{right:.4}em !important;text-indent:{first_indent:.4}em !important;margin-top:{before:.4}em !important;margin-bottom:{after:.4}em !important;text-align:{align} !important;}}\n",
            style.id, (left + indent).min(4.0)
        ));
    }
    // This is last so blank paragraphs do not inherit the source spacing.
    output.push_str("html.hwpx-read main.hwpx-doc [data-hwpx-empty]:not([data-hwpx-island] *) {min-height:.5em !important;margin:0 !important;padding:0 !important;text-indent:0 !important;line-height:0 !important;}\nhtml.hwpx-read main.hwpx-doc [data-hwpx-empty] + [data-hwpx-empty]:not([data-hwpx-island] *) {min-height:0 !important;}\n}\n");
    output
}

fn modal_size(weights: &BTreeMap<HwpUnit, usize>) -> Option<HwpUnit> {
    weights
        .iter()
        .max_by_key(|(size, count)| (**count, std::cmp::Reverse(**size)))
        .map(|(size, _)| *size)
}

fn weigh(
    paragraph: &Paragraph,
    sizes: &BTreeMap<u32, HwpUnit>,
    weights: &mut BTreeMap<HwpUnit, usize>,
) {
    for token in &paragraph.tokens {
        let count = token
            .visible_text()
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        if let Some(size) = sizes.get(&token.char_style_id).filter(|size| **size > 0) {
            if count > 0 {
                *weights.entry(*size).or_default() += count;
            }
        }
    }
}

fn paragraphs(document: &Document) -> Vec<&Paragraph> {
    fn paragraph<'a>(source: &'a Paragraph, out: &mut Vec<&'a Paragraph>) {
        out.push(source);
        for child in &source.objects {
            object(child, out);
        }
    }
    fn table<'a>(source: &'a Table, out: &mut Vec<&'a Paragraph>) {
        if let Some(caption) = source.caption.as_deref() {
            for source in &caption.paragraphs {
                paragraph(source, out);
            }
        }
        for cell in &source.cells {
            for source in &cell.paragraphs {
                paragraph(source, out);
            }
            for source in &cell.tables {
                table(source, out);
            }
        }
    }
    fn object<'a>(source: &'a PositionedObject, out: &mut Vec<&'a Paragraph>) {
        if let Some(caption) = source.caption.as_deref() {
            for source in &caption.paragraphs {
                paragraph(source, out);
            }
        }
        if let Some(shape) = source.shape.as_deref() {
            for source in &shape.paragraphs {
                paragraph(source, out);
            }
            for source in &shape.tables {
                table(source, out);
            }
        }
        for child in &source.children {
            object(child, out);
        }
    }
    let mut out = Vec::new();
    for source in document.sections.iter().flat_map(|section| &section.blocks) {
        match source {
            Block::Paragraph(source) => paragraph(source, &mut out),
            Block::Table(source) => table(source, &mut out),
            Block::Object(source) => object(source, &mut out),
        }
    }
    out
}

pub(super) fn cell_attributes(cell: &TableCell) -> String {
    // Each cell resets inherited paint (nested cells must not accidentally
    // take their containing cell's fill or diagonal).
    let mut style = "--hwpx-cell-fill:initial;--hwpx-cell-gradient:initial;--hwpx-cell-diag-a:initial;--hwpx-cell-diag-b:initial;".to_owned();
    if let Some(color) = &cell.fill_color {
        style.push_str(&format!("--hwpx-cell-fill:{};", safe_color(color)));
    }
    // A diagonal encodes a divided cell, and gradient colour can encode a
    // category. Preserve those source marks as CSS instead of dropping them
    // with the table's coordinate SVG.
    if !cell.gradient.is_empty() {
        let colors = cell
            .gradient
            .iter()
            .map(|color| safe_color(color))
            .collect::<Vec<_>>();
        if colors.len() > 1 {
            style.push_str(&format!(
                "--hwpx-cell-gradient:linear-gradient({}deg,{});",
                180 - cell.gradient_angle.rem_euclid(360),
                colors.join(",")
            ));
        }
    }
    let color = cell
        .diagonal_stroke
        .as_ref()
        .map(|stroke| safe_color(&stroke.color))
        .unwrap_or("#666666");
    if cell.diagonal_forward {
        style.push_str(&format!("--hwpx-cell-diag-a:linear-gradient(to bottom right,transparent calc(50% - .5px),{color} 50%,transparent calc(50% + .5px));"));
    }
    if cell.diagonal_backward {
        style.push_str(&format!("--hwpx-cell-diag-b:linear-gradient(to top right,transparent calc(50% - .5px),{color} 50%,transparent calc(50% + .5px));"));
    }
    format!(" style=\"{style}\"")
}

pub(super) fn object_attributes(object: &PositionedObject) -> String {
    let canvas = if object.img_dim.width > 0 && object.img_dim.height > 0 {
        object.img_dim
    } else {
        object.original_size
    };
    let cropped = object.crop.is_some_and(|crop| {
        crop.x != 0 || crop.y != 0 || crop.width != canvas.width || crop.height != canvas.height
    });
    if object.kind == "pic" && !cropped && !object.flip_x && !object.flip_y {
        return format!(
            " class=\"hwpx-read-image\" style=\"--hwpx-image-width:{}mm;\"",
            hwp_to_centi_mm(object.box_units.width.max(1)) as f64 / 100.0
        );
    }
    format!(
        " class=\"hwpx-read-island\" data-hwpx-island style=\"--hwpx-island-width:{:.4}px;--hwpx-island-height:{:.4}px;\"",
        hwp_to_centi_mm(object.box_units.width.max(1)) as f64 * 96.0 / 2540.0,
        hwp_to_centi_mm(object.box_units.height.max(1)) as f64 * 96.0 / 2540.0
    )
}

const BASE_CSS: &str = r#".hwpx-read-scroll,.hwpx-read-island,.hwpx-read-image {display:contents;}
.htb > .hwpx-read-scroll > table,.htb > .hwpx-read-scroll > [data-hwpx-table],.htG > .hwpx-read-scroll > table {position:absolute;left:0;top:0;}
.hwpx-read-controls {position:fixed;right:12px;top:12px;z-index:2147483646;display:flex;align-items:center;gap:6px;padding:4px;border:1px solid #bbb;border-radius:6px;background:#f3f3f3;color:#222;font:14px/1.4 system-ui,sans-serif;}
.hwpx-read-controls button {font:inherit;color:inherit;padding:8px 10px;min-height:40px;border:1px solid #aaa;border-radius:4px;background:white;cursor:pointer;}
.hwpx-read-controls button:disabled {opacity:.4;cursor:default;}
.hwpx-read-controls button:focus-visible {outline:3px solid #075a76;outline-offset:2px;}
.hwpx-read-controls[hidden],.hwpx-read-controls [hidden] {display:none !important;}
@media screen {
html.hwpx-read body {padding:0 !important;margin:0 !important;}
html.hwpx-read .hpa[data-page],html.hwpx-read .hwpx-nav {display:none !important;}
html.hwpx-read .hwpx-read-controls {position:sticky;top:0;right:auto;justify-content:center;border-width:0 0 1px;border-radius:0;}
html.hwpx-read main.hwpx-doc {position:static !important;display:block !important;width:100% !important;max-width:42em !important;height:auto !important;margin:0 auto !important;padding:16px !important;box-sizing:border-box !important;pointer-events:auto !important;font:var(--hwpx-read-size,18px)/1.6 system-ui,sans-serif;overflow-wrap:anywhere;}
}
@media print {.hwpx-read-controls {display:none !important;}}
"#;

const FLOW_CSS: &str = r#"html.hwpx-read main.hwpx-doc :is(.hpg,.hpo,.hcD,.hcI,.hcS,.hce,.hme):not([data-hwpx-island] *) {display:contents !important;}
html.hwpx-read main.hwpx-doc :is(span.hls,.hrt,.hhi,.hhe,.hpN,.haN):not([data-hwpx-island] *,.heq *,.hco *,.hdu *) {display:inline !important;}
html.hwpx-read main.hwpx-doc :is(p,h1,h2,h3,h4,h5,h6,div.hls,[data-hwpx-paragraph],[data-hwpx-empty],li,ul,ol):not([data-hwpx-island] *) {display:block !important;}
html.hwpx-read main.hwpx-doc :is([data-hwpx-repeat],[data-hwpx-artifact],svg.hs[aria-hidden],.htC):not([data-hwpx-island] *) {display:none !important;}
html.hwpx-read main.hwpx-doc .htx:not([data-hwpx-island] *) {display:inline !important;white-space:pre-line !important;font-size:inherit !important;line-height:inherit !important;}
html.hwpx-read main.hwpx-doc :is(.hwpx-read-scroll,.htb,.htG):not([data-hwpx-island] *) {display:block !important;overflow-x:auto !important;overflow-y:hidden !important;max-width:100% !important;margin:.5em 0 !important;pointer-events:auto !important;}
html.hwpx-read main.hwpx-doc table:not([data-hwpx-island] *) {display:table !important;border-collapse:collapse !important;width:auto !important;min-width:100% !important;max-width:none !important;table-layout:auto !important;}
html.hwpx-read main.hwpx-doc :is(td,th,[data-hwpx-cell]):not([data-hwpx-island] *) {border:1px solid #aab2ba !important;padding:.4em .6em !important;vertical-align:top !important;text-indent:0 !important;min-width:4em !important;background-color:var(--hwpx-cell-fill,transparent) !important;background-image:var(--hwpx-cell-diag-a,linear-gradient(transparent,transparent)),var(--hwpx-cell-diag-b,linear-gradient(transparent,transparent)),var(--hwpx-cell-gradient,linear-gradient(transparent,transparent)) !important;}
html.hwpx-read main.hwpx-doc :is(th,[data-hwpx-header]):not([data-hwpx-island] *) {background-color:var(--hwpx-cell-fill,#edf2f6) !important;font-weight:600 !important;}
html.hwpx-read main.hwpx-doc [data-hwpx-table]:not([data-hwpx-island] *) {display:block !important;border:1px solid #aab2ba !important;}
html.hwpx-read main.hwpx-doc caption:not([data-hwpx-island] *) {caption-side:top !important;text-align:start !important;}
html.hwpx-read main.hwpx-doc :is(.hwpx-read-image,.hwpx-read-image .hsR):not([data-hwpx-island] *) {display:contents !important;}
html.hwpx-read main.hwpx-doc img.hpi:not([data-hwpx-island] *) {display:block !important;width:var(--hwpx-image-width,auto) !important;max-width:100% !important;height:auto !important;margin:.5em auto !important;}
html.hwpx-read main.hwpx-doc [data-hwpx-island] {display:block !important;position:relative !important;width:var(--hwpx-island-width) !important;height:var(--hwpx-island-height) !important;margin:.5em 0 !important;overflow:visible !important;pointer-events:auto !important;}
html.hwpx-read main.hwpx-doc [data-hwpx-island] > :first-child {position:relative !important;left:0 !important;top:0 !important;margin-left:0 !important;margin-top:0 !important;}
html.hwpx-read main.hwpx-doc > :not([data-hwpx-island],[data-hwpx-empty]) {content-visibility:auto;contain-intrinsic-size:auto 80px;}
html.hwpx-read main.hwpx-doc a[href] {text-decoration:underline;}
"#;

/// Fixed head script: choose the first presentation before the body arrives,
/// and register print restoration before navigation and font correction.
pub const READING_SCRIPT: &str = r#"(() => {
  const root = document.documentElement;
  if (!window.CSS || !CSS.supports('display', 'contents') || !CSS.supports('zoom', '1')) return;
  const get = key => { try { return localStorage.getItem('hwpx2html:' + key); } catch (_) { return null; } };
  const put = (key, value) => { try { localStorage.setItem('hwpx2html:' + key, value); } catch (_) {} };
  const width = Number(document.querySelector('meta[name="hwpx-paper-width"]')?.content) || 794;
  const frame = window !== window.top;
  const narrow = frame ? innerWidth < width : matchMedia('(pointer:coarse)').matches && screen.width < width;
  const saved = get('view');
  let reading = saved === 'read' || (saved !== 'original' && narrow);
  const steps = [90, 100, 115, 130, 150];
  let size = steps.indexOf(Number(get('size')));
  if (size < 0) size = 1;
  root.style.setProperty('--hwpx-read-size', (18 * steps[size] / 100) + 'px');
  let viewport = null;
  const hostViewport = document.querySelector('meta[name="viewport"]');
  const setViewport = () => {
    if (hostViewport) return;
    if (reading || (!frame && matchMedia('(pointer:coarse)').matches)) {
      if (!viewport) { viewport = document.createElement('meta'); viewport.name = 'viewport'; document.head.append(viewport); }
      viewport.content = reading ? 'width=device-width, initial-scale=1' : 'width=' + Math.ceil(Math.max(width + 16, screen.width));
    } else if (viewport) { viewport.remove(); viewport = null; }
  };
  root.classList.toggle('hwpx-read', reading);
  setViewport();
  let printing = false, printState = null, restorePrint = null, preparePrint = null;
  addEventListener('beforeprint', () => {
    if (printing) return;
    printing = true;
    printState = {reading, paged: root.classList.contains('hwpx-paged'), x: scrollX, y: scrollY};
    if (preparePrint) preparePrint();
    root.classList.remove('hwpx-read', 'hwpx-paged');
    dispatchEvent(new CustomEvent('hwpx-viewchange', {detail: {reading: false, printing: true}}));
  });
  addEventListener('afterprint', () => {
    if (!printing) return;
    root.classList.toggle('hwpx-read', printState.reading);
    root.classList.toggle('hwpx-paged', printState.paged && !printState.reading);
    dispatchEvent(new CustomEvent('hwpx-viewchange', {detail: {reading: printState.reading, printing: true}}));
    printing = false;
    if (restorePrint) restorePrint(printState);
  });
  document.addEventListener('DOMContentLoaded', () => {
    const main = document.querySelector('main.hwpx-doc');
    if (!main) { root.classList.remove('hwpx-read'); return; }
    const pages = [...document.querySelectorAll('.hpa[data-page]')];
    const hashPage = () => { const m = /^#page-(\d+)$/.exec(location.hash); return m ? Number(m[1]) : null; };
    const bound = n => Math.max(1, Math.min(pages.length || 1, Number(n) || 1));
    const pageOf = node => Number((node?.nodeType === 1 ? node : node?.parentElement)?.closest('[data-pg]')?.dataset.pg) || null;
    const topContent = () => {
      const y = Math.min(innerHeight - 1, (reading ? bar.getBoundingClientRect().bottom : 0) + 8);
      for (const x of [innerWidth / 2, 20, innerWidth - 20]) {
        const range = document.caretRangeFromPoint?.(x, y);
        const pos = document.caretPositionFromPoint?.(x, y);
        const node = range?.startContainer || pos?.offsetNode;
        if (node && main.contains(node) && pageOf(node)) return {node: node.nodeType === 1 ? node : node.parentElement, page: pageOf(node)};
        const element = document.elementFromPoint(x, y);
        if (element && main.contains(element) && pageOf(element)) return {node: element, page: pageOf(element)};
      }
      return null;
    };
    const currentPage = () => {
      if (!reading && root.classList.contains('hwpx-paged')) return bound(document.querySelector('.hwpx-nav input')?.value || hashPage());
      const found = topContent();
      if (found) return bound(found.page);
      if (!reading) { const page = pages.find(page => page.getBoundingClientRect().bottom > 40); if (page) return bound(page.dataset.page); }
      return bound(hashPage());
    };
    const firstContent = page => {
      for (const element of main.querySelectorAll('[data-pg="' + bound(page) + '"]')) {
        if (element.closest('[data-hwpx-repeat],[data-hwpx-artifact]') || element.matches('svg')) continue;
        const child = element.matches('.hls,img,[data-hwpx-island]') ? element : element.querySelector('.hls,img,[data-hwpx-island]');
        if (child) return child;
      }
      return main;
    };
    const scrollContent = element => {
      element.scrollIntoView({block: 'start'});
      if (reading) scrollBy(0, -bar.getBoundingClientRect().height - 8);
    };
    const make = (tag, props) => Object.assign(document.createElement(tag), props);
    const bar = make('nav', {className: 'hwpx-read-controls'});
    bar.setAttribute('aria-label', '보기와 글자 크기');
    const toggle = make('button', {type: 'button'});
    const minus = make('button', {type: 'button', textContent: '가−', title: '글자 크기 줄이기'});
    const plus = make('button', {type: 'button', textContent: '가+', title: '글자 크기 키우기'});
    minus.setAttribute('aria-label', '글자 크기 줄이기'); plus.setAttribute('aria-label', '글자 크기 키우기');
    const status = make('span', {}); status.setAttribute('aria-live', 'polite');
    bar.append(toggle, minus, status, plus); document.body.prepend(bar);
    const controls = () => {
      toggle.textContent = reading ? '원문 보기' : '읽기 보기'; toggle.setAttribute('aria-pressed', String(reading));
      minus.hidden = plus.hidden = status.hidden = !reading;
      minus.disabled = size === 0; plus.disabled = size === steps.length - 1;
      status.textContent = steps[size] + '%';
      const nav = document.querySelector('.hwpx-nav');
      if (!reading && nav) { nav.append(toggle); bar.hidden = true; }
      else { bar.prepend(toggle); bar.hidden = false; }
    };
    const islands = [...main.querySelectorAll('[data-hwpx-island]')].filter(element => !element.parentElement.closest('[data-hwpx-island]'));
    const fitIslands = () => {
      if (!reading || printing) return;
      const available = main.clientWidth - 32;
      const widths = islands.map(element => parseFloat(getComputedStyle(element).getPropertyValue('--hwpx-island-width')));
      const rooms = islands.map(element => {
        const parentWidth = element.parentElement.getBoundingClientRect().width;
        return parentWidth > 0 ? Math.min(available, parentWidth) : available;
      });
      islands.forEach((element, i) => { element.style.zoom = String(Math.min(1, Math.max(1, rooms[i]) / Math.max(1, widths[i]))); });
    };
    preparePrint = () => islands.forEach(element => { element.style.zoom = ''; });
    const setReading = next => {
      if (printing || next === reading) return;
      const seen = reading ? topContent() : null;
      const page = currentPage();
      reading = next; root.classList.toggle('hwpx-read', reading);
      if (reading) root.classList.remove('hwpx-paged');
      if (!reading) islands.forEach(element => { element.style.zoom = ''; });
      setViewport(); put('view', reading ? 'read' : 'original'); controls();
      dispatchEvent(new CustomEvent('hwpx-viewchange', {detail: {reading, page}}));
      requestAnimationFrame(() => {
        fitIslands();
        if (reading) scrollContent(firstContent(page));
        else if (!root.classList.contains('hwpx-paged')) scrollContent(seen?.node || pages[page - 1] || main);
        toggle.focus({preventScroll: true});
      });
    };
    toggle.addEventListener('click', () => setReading(!reading));
    const changeSize = delta => {
      const seen = topContent(); size = Math.max(0, Math.min(steps.length - 1, size + delta));
      root.style.setProperty('--hwpx-read-size', (18 * steps[size] / 100) + 'px'); put('size', String(steps[size])); controls();
      requestAnimationFrame(() => { fitIslands(); if (seen) scrollContent(seen.node); });
    };
    minus.addEventListener('click', () => changeSize(-1)); plus.addEventListener('click', () => changeSize(1));
    addEventListener('hwpx-navigation-ready', controls);
    let resize = 0;
    addEventListener('resize', () => { cancelAnimationFrame(resize); resize = requestAnimationFrame(fitIslands); });
    addEventListener('hashchange', () => { if (reading && hashPage()) requestAnimationFrame(() => scrollContent(firstContent(hashPage()))); });
    restorePrint = state => { controls(); requestAnimationFrame(() => { fitIslands(); scrollTo(state.x, state.y); }); };
    controls(); root.classList.add('hwpx-reading-ready');
    if (reading) requestAnimationFrame(() => { fitIslands(); if (hashPage()) scrollContent(firstContent(hashPage())); });
  });
})();"#;

/// The direct navigation's reading-aware variant. The original fixed script
/// remains unchanged and is still emitted when the reading option is off.
pub const NAVIGATION_SCRIPT: &str = r#"(() => {
  const root = document.documentElement;
  if (!root.classList.contains('hwpx-read')) root.classList.add('hwpx-paged');
  addEventListener('beforeprint', () => root.classList.remove('hwpx-paged'));
  document.addEventListener('DOMContentLoaded', () => {
    const pages = [...document.querySelectorAll('.hpa')];
    if (pages.length < 2) { root.classList.remove('hwpx-paged'); root.classList.add('hwpx-ready'); return; }
    let current = -1, full = false;
    const rule = document.createElement('style'); document.head.append(rule);
    const make = (tag, props) => Object.assign(document.createElement(tag), props);
    const nav = make('nav', {className: 'hwpx-nav'}); nav.setAttribute('aria-label', '쪽 이동');
    const prev = make('button', {type: 'button', textContent: '‹ 이전', title: '이전 쪽 (←)'});
    const next = make('button', {type: 'button', textContent: '다음 ›', title: '다음 쪽 (→)'});
    const input = make('input', {type: 'number', min: 1, max: pages.length, title: '쪽 번호'}); input.setAttribute('aria-label', '쪽 번호');
    const view = make('button', {type: 'button', textContent: '전체 보기', title: '모든 쪽을 이어서 보여 줍니다. 브라우저의 찾기(Ctrl+F)가 문서 전체를 찾습니다.'}); view.setAttribute('aria-pressed', 'false');
    nav.append(prev, input, make('span', {textContent: '/ ' + pages.length}), next, view);
    const read = () => root.classList.contains('hwpx-read');
    const show = index => {
      if (read()) return;
      if (Number.isNaN(index)) index = current;
      index = Math.max(0, Math.min(pages.length - 1, Math.floor(index)));
      if (full) { current = index; input.value = index + 1; pages[index].scrollIntoView(); return; }
      if (index !== current) {
        const n = pages[index].dataset.page;
        rule.textContent = 'html.hwpx-paged [data-pg="' + n + '"],html.hwpx-paged .hpa[data-page="' + n + '"]{display:block}';
        current = index; scrollTo(0, 0);
      }
      input.value = index + 1; prev.disabled = index === 0; next.disabled = index === pages.length - 1;
      const hash = '#page-' + (index + 1);
      if (location.hash !== hash && (location.hash || index)) try { history.replaceState(null, '', hash); } catch (_) {}
    };
    const fromHash = () => { const m = /^#page-(\d+)$/.exec(location.hash); return m ? m[1] - 1 : NaN; };
    prev.addEventListener('click', () => show(current - 1)); next.addEventListener('click', () => show(current + 1)); input.addEventListener('change', () => show(input.valueAsNumber - 1));
    view.addEventListener('click', () => {
      if (read()) return;
      const seen = full ? Math.max(0, pages.findIndex(page => page.getBoundingClientRect().bottom > 40)) : current;
      full = !full; root.classList.toggle('hwpx-paged', !full);
      view.textContent = full ? '한 쪽 보기' : '전체 보기'; view.setAttribute('aria-pressed', String(full));
      prev.disabled = next.disabled = input.disabled = full;
      if (full) pages[current].scrollIntoView(); else { current = -1; show(seen); }
    });
    addEventListener('hwpx-viewchange', ({detail}) => {
      if (detail.printing || detail.reading) return;
      root.classList.toggle('hwpx-paged', !full); current = -1; show((detail.page || 1) - 1);
    });
    addEventListener('hashchange', () => { const index = fromHash(); if (!read() && !Number.isNaN(index)) show(index); });
    addEventListener('keydown', event => {
      if (read() || full || !root.classList.contains('hwpx-paged')) return;
      const step = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0;
      if (!step || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      if (event.target instanceof Element && event.target.closest('input,textarea,select,[contenteditable]')) return;
      event.preventDefault(); show(current + step);
    });
    let touch = false;
    addEventListener('pointerdown', event => { touch = event.pointerType !== 'mouse'; }, true);
    addEventListener('click', event => {
      if (nav.contains(event.target)) { if (touch) nav.classList.add('hwpx-show'); return; }
      nav.classList.remove('hwpx-show');
      if (read() || event.button || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !root.classList.contains('hwpx-paged')) return;
      if (event.target instanceof Element && event.target.closest('a[href],button,input,textarea,select,label,[contenteditable]')) return;
      const selection = getSelection(); if (selection && !selection.isCollapsed) return;
      const rect = pages[current].getBoundingClientRect(), x = event.clientX - rect.left, width = rect.width;
      if (x < 0 || x >= width || event.clientY < rect.top || event.clientY >= rect.bottom) return;
      const step = x < width / 3 ? -1 : x >= width * 2 / 3 ? 1 : 0; if (step) show(current + step);
    });
    nav.addEventListener('focusin', () => { try { if (nav.querySelector(':focus-visible')) nav.classList.add('hwpx-show'); } catch (_) {} });
    nav.addEventListener('focusout', event => { if (!nav.contains(event.relatedTarget)) nav.classList.remove('hwpx-show'); });
    addEventListener('afterprint', () => { if (!read() && !full) root.classList.add('hwpx-paged'); });
    document.body.prepend(nav);
    if (read()) { current = Math.max(0, Math.min(pages.length - 1, fromHash() || 0)); input.value = current + 1; }
    else show(fromHash());
    root.classList.add('hwpx-ready'); dispatchEvent(new Event('hwpx-navigation-ready'));
  });
})();"#;

/// Reading-aware correction: only this variant records and restores changed
/// text nodes; the disabled option retains SCRIPT_SOURCE byte for byte.
pub const CORRECTION_SCRIPT: &str = r#"(()=>{const reading=()=>document.documentElement.classList.contains('hwpx-read');
const textChanges=new Map(),genChanges=new Map();let spaced=false;
const restore=()=>{textChanges.forEach((v,n)=>{n.data=v.original;});genChanges.forEach((v,e)=>{e.dataset.gen=v.original;});};
const reapply=()=>{textChanges.forEach((v,n)=>{n.data=v.adjusted;});genChanges.forEach((v,e)=>{e.dataset.gen=v.adjusted;});};
const lines=page=>{const n=page.dataset.page;return n?document.querySelectorAll('[data-page="'+n+'"]:not(.hpa) .hls,[data-pg="'+n+'"] .hls,.hls[data-pg="'+n+'"]'):page.querySelectorAll('.hls');};const nominal=new Map();const spaces=()=>{const members=new Map();document.querySelectorAll('.hrt').forEach(e=>e.classList.forEach(c=>{if(/^cs\d+$/.test(c)){if(!members.has(c))members.set(c,[]);members.get(c).push(e);}}));const box=document.createElement('div');box.style.cssText='position:absolute;visibility:hidden';const probes=[...members.keys()].map(c=>{const p=document.createElement('span');p.className='hrt '+c;p.style.cssText='position:absolute;white-space:pre;letter-spacing:0;word-spacing:0';p.textContent=' \u00a0';box.append(p);return p;});document.body.append(box);const advance=(p,i)=>{const r=document.createRange();r.setStart(p.firstChild,i);r.setEnd(p.firstChild,i+1);return r.getBoundingClientRect().width;};const cv=(()=>{try{return document.createElement('canvas').getContext('2d',{willReadFrequently:true});}catch(e){return null;}})();const inked=(s,size)=>{if(!cv)return false;const px=Math.min(size,40),z=Math.ceil(px*2)+8;cv.canvas.width=cv.canvas.height=z;cv.font='10px sans-serif';cv.font=s.fontStyle+' '+s.fontWeight+' '+px+'px '+s.fontFamily;cv.textBaseline='middle';cv.fillText('\u00a0',4,z/2);const d=cv.getImageData(0,0,z,z).data;for(let i=3;i<d.length;i+=4)if(d[i])return true;return false;};let css='.hrt{word-spacing:0}';const mixed=[],boxed=[];probes.forEach(p=>{const c=p.classList[1],s=getComputedStyle(p),size=parseFloat(s.fontSize);if(!(size>0))return;const space=advance(p,0),nbsp=advance(p,1),fontSpace=s.getPropertyValue('--hwpx-font-space').trim()==='1';if(inked(s,size)){boxed.push(c);nominal.set(c,fontSpace?space/size:.5);css+='.'+c+'{white-space:pre'+(fontSpace?'':';word-spacing:'+((size/2-space)/size).toFixed(4)+'em')+'}';return;}if(fontSpace){nominal.set(c,space/size);return;}nominal.set(c,.5);if(Math.abs(space-nbsp)>.01)mixed.push(c);css+='.'+c+'{word-spacing:'+((size/2-nbsp)/size).toFixed(4)+'em}';});box.remove();const convert=(list,from,to)=>list.forEach(c=>members.get(c).forEach(e=>{e.childNodes.forEach(n=>{if(n.nodeType===3&&n.data.includes(from)){if(!textChanges.has(n))textChanges.set(n,{original:n.data});n.data=n.data.split(from).join(to);textChanges.get(n).adjusted=n.data;}});if(e.dataset.gen&&e.dataset.gen.includes(from)){if(!genChanges.has(e))genChanges.set(e,{original:e.dataset.gen});e.dataset.gen=e.dataset.gen.split(from).join(to);genChanges.get(e).adjusted=e.dataset.gen;}}));convert(mixed,' ','\u00a0');convert(boxed,'\u00a0',' ');const sheet=document.createElement('style');sheet.textContent=css;document.head.append(sheet);spaced=true;};
const num=v=>parseFloat(v)||0;
const trail=c=>c>='\udc00'&&c<='\udfff',blank=c=>c===' '||c==='\u00a0';
const measure=line=>{
const own=e=>e.closest('.hls')===line;
const elements=[...line.querySelectorAll('.hrt')].filter(e=>own(e)&&!e.parentElement.closest('.hrt')&&!e.closest('.htC'));
if(!elements.length)return null;
const style=getComputedStyle(line);
const pad=parseFloat(style.paddingLeft)||0,width=line.getBoundingClientRect().width-2*pad-(parseFloat(style.paddingRight)||0);
if(width<=0)return null;
const tabs=[...line.querySelectorAll('.htC')].filter(own).reduce((sum,e)=>{const s=getComputedStyle(e);return sum+e.getBoundingClientRect().width+(parseFloat(s.marginLeft)||0)+(parseFloat(s.marginRight)||0);},0);
const texts=[];
elements.forEach(e=>{const walker=document.createTreeWalker(e,NodeFilter.SHOW_TEXT);while(walker.nextNode())texts.push(walker.currentNode);});
return{line,style,elements,texts,width,tabs,fill:line.dataset.hwpxFill};
};
const hanging=job=>{let sum=0;const texts=job.texts;for(let i=texts.length-1;i>=0;i--){const d=texts[i].data,k=d.length-d.replace(/[ \u00a0]+$/,'').length;if(k){const r=document.createRange();r.setStart(texts[i],d.length-k);r.setEnd(texts[i],d.length);sum+=r.getBoundingClientRect().width;}if(k<d.length)break;}return sum;};
const drawn=job=>job.elements.reduce((sum,e)=>sum+e.getBoundingClientRect().width,job.tabs);
const current=job=>drawn(job)-hanging(job);
const edgeOf=(job,left)=>{const texts=job.texts;for(let i=texts.length-1;i>=0;i--){const d=texts[i].data;for(let j=d.length-1;j>=0;j--)if(!/\s/.test(d[j])){const r=document.createRange();r.setStart(texts[i],j>0&&trail(d[j])?j-1:j);r.setEnd(texts[i],j+1);return r.getBoundingClientRect().right-left;}}return job.width;};
const widenRead=job=>{
const line=job.line,style=getComputedStyle(line),left=line.getBoundingClientRect().left+line.clientLeft+(parseFloat(style.paddingLeft)||0);
const elements=job.elements.filter(e=>!e.closest('.hhe'));
const slack=job.width-edgeOf(job,left);
if(!(slack>.05))return null;
let tail=true,chars=0,gaps=0;
for(let i=job.texts.length-1;i>=0;i--){const d=job.texts[i].data,fixed=job.texts[i].parentElement.closest('[data-hwpx-fixed]');for(let j=d.length-1;j>=0;j--){const c=d[j];if(tail&&(blank(c)||c==='\n'))continue;tail=false;if(trail(c))continue;chars++;if(blank(c)&&!fixed)gaps++;}}
if(job.fill==='space'&&gaps)return{job,left,elements,gaps,word:true,px:slack/gaps};
if(chars>1)return{job,left,elements,word:false,px:slack/(chars-1)};
return null;
};
const spread=(plans,prop)=>{
const writes=[];
plans.forEach(w=>w.elements.forEach(e=>{if(prop==='wordSpacing'&&e.hasAttribute('data-hwpx-fixed'))return;writes.push([e,((parseFloat(getComputedStyle(e)[prop])||0)+w.px)+'px']);}));
writes.forEach(([e,value])=>{e.style[prop]=value;});
};
const widenAll=jobs=>{
const plans=jobs.map(widenRead).filter(Boolean);
const words=plans.filter(w=>w.word);
spread(words,'wordSpacing');
spread(plans.filter(w=>!w.word),'letterSpacing');
const rest=words.map(w=>({elements:w.elements,gaps:w.gaps,room:w.job.width-edgeOf(w.job,w.left)})).filter(w=>Math.abs(w.room)>.05);
spread(rest.map(w=>({elements:w.elements,px:w.room/w.gaps})),'wordSpacing');
};
const squeeze=(job,k)=>job.runs.forEach(([e,base,space])=>{e.style.wordSpacing=(base-k*space)+'px';});
const tighten=jobs=>{
jobs.forEach(j=>{
let n=0,own=0,tail=true;
for(let i=j.texts.length-1;i>=0;i--){const d=j.texts[i].data;let k=0;for(let m=d.length-1;m>=0;m--){const c=d[m];if(trail(c)||tail&&blank(c))continue;tail=false;k++;}if(k){n+=k;own+=k*num(getComputedStyle(j.texts[i].parentElement).letterSpacing);}}
j.n=Math.max(1,n);
j.lo=-4;j.hi=0;j.fit=null;j.over=null;
j.x=Math.min(-1e-4,Math.max(-3.9999,(j.width-current(j)+own)/j.n));
});
let active=jobs;
for(let round=0;round<24&&active.length;round++){
active.forEach(j=>j.elements.forEach(e=>{e.style.letterSpacing=j.x+'px';}));
active=active.filter(j=>{
const now=current(j);
if(now<=j.width){j.lo=j.x;j.fit=now;}else{j.hi=j.x;j.over=now;}
if((j.fit!==null&&j.width-j.fit<=.032)||j.hi-j.lo<=1e-4)return false;
const aim=j.width-.016;
const next=round>5?(j.lo+j.hi)/2:j.fit!==null&&j.over!==null?j.lo+(aim-j.fit)*(j.hi-j.lo)/(j.over-j.fit):j.x+(aim-now)/j.n;
j.x=Math.min(j.hi-5e-5,Math.max(j.lo+5e-5,next));
return true;
});
}
jobs.forEach(j=>j.elements.forEach(e=>{e.style.letterSpacing=j.lo+'px';}));
};
const fitAll=jobs=>{
const tight=jobs.filter(j=>j.condense>0);
tight.forEach(j=>squeeze(j,j.condense));
const least=tight.map(current);
const fitted=new Set();
const second=tight.filter((j,i)=>{if(!(least[i]<=j.width))return false;squeeze(j,j.condense*(j.full-j.width)/(j.full-least[i]));return true;});
second.forEach(j=>{if(current(j)<=j.width)fitted.add(j);});
tighten(jobs.filter(j=>!fitted.has(j)));
};
const adjust=pages=>{
const wide=[],over=[];
pages.forEach(page=>lines(page).forEach(line=>{
const job=measure(line);
if(!job)return;
const total=drawn(job);
if(total<=job.width&&!job.fill)return;
const now=total-hanging(job);
if(now<=job.width){if(job.fill)wide.push(job);return;}
job.full=now;
job.condense=(parseFloat(job.style.getPropertyValue('--hwpx-condense'))||0)/100;
if(job.condense>0)job.runs=job.elements.map(e=>{const s=getComputedStyle(e),c=[...e.classList].find(c=>nominal.has(c));return[e,parseFloat(s.wordSpacing)||0,parseFloat(s.fontSize)*(c?nominal.get(c):.5)];});
over.push(job);
}));
widenAll(wide);
fitAll(over);
};
const equations=pages=>{
if(reading())return;
const fits=[];
pages.forEach(page=>{const n=page.dataset.page;(n?document.querySelectorAll('[data-page="'+n+'"]:not(.hpa) .heq,[data-pg="'+n+'"] .heq'):page.querySelectorAll('.heq')).forEach(heq=>{const q=heq.querySelector('.hmq'),m=q&&q.querySelector('math');if(m){q.style.transform='';fits.push([heq,q,m]);}});});
const reads=fits.map(([heq,q,m])=>[heq.getBoundingClientRect().width,m.getBoundingClientRect().width]);
fits.forEach(([heq,q],i)=>{const[box,ink]=reads[i];if(box>0&&ink>box+.01){q.style.transformOrigin='0 0';q.style.transform='scaleX('+(box/ink)+')';}});
};
const start=()=>{
if(!reading())spaces();
const pages=[...document.querySelectorAll('.hpa')];
const done=new WeakSet();
const run=list=>{
if(reading())return [];
if(!spaced)spaces();
const shown=list.filter(p=>p.getClientRects().length);
const fresh=shown.filter(p=>!done.has(p));
fresh.forEach(p=>done.add(p));
adjust(fresh);
equations(shown);
return shown;
};
if(document.fonts&&document.fonts.addEventListener)document.fonts.addEventListener('loadingdone',()=>equations(pages.filter(p=>p.getClientRects().length)));
if('IntersectionObserver'in window){
const observer=new IntersectionObserver(entries=>{
const seen=entries.filter(e=>e.isIntersecting).map(e=>e.target);
if(seen.length)setTimeout(()=>run(seen).forEach(p=>observer.unobserve(p)),0);
},{rootMargin:'100% 0px'});
pages.forEach(p=>observer.observe(p));
}else{
let i=0;const next=()=>{if(i<pages.length){run([pages[i++]]);setTimeout(next,0);}};
setTimeout(next,0);
}
addEventListener('hwpx-viewchange',({detail})=>{
if(reading()){restore();return;}
if(spaced)reapply();
// Navigation updates the shown page in the same event dispatch. Measure
// after all listeners have run; beforeprint must remain synchronous.
if(detail.printing)run(pages);else requestAnimationFrame(()=>{
const resume=()=>{if(!reading())run(pages);};
if(document.fonts&&document.fonts.ready)document.fonts.ready.then(resume);else resume();
});
});
window.addEventListener('beforeprint',()=>run(pages));
};
if(document.fonts&&document.fonts.ready)document.fonts.ready.then(start);else start();})();"#;
