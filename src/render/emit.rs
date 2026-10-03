//! The direct emitter (semantic-first plan §3, stage 3): writes the semantic
//! tree as the document, and puts each laid-out line where the layout placed
//! it. Nothing here reads a page's HTML back.
//!
//! The tree decides the elements and their order (`p`, headings, lists,
//! tables and cells); the placement plan (`layout::presentation`) says where
//! each line sits. A line is a box of its own, so a paragraph, cell or table
//! on several pages is still one element. There is no chain of boxes around
//! the lines: the page a line belongs to is a class (`pgN`) that carries the
//! page's origin as custom properties, and the line's margins add its place
//! on the page to that origin. A cell's box is not a box either: each of its
//! lines is clipped to it (`clip-path`), and the table's borders and fills
//! are a drawing (`svg.hs`) per page, like the lines.
//!
//! Three kinds of box stay, each for its reason:
//!
//! * A table set inline in a line (`treatAsChar`) is an inline block in that
//!   line, because the browser places it among the line's text (the line's
//!   alignment and the widths of the text before it decide where). It stands
//!   where its control stands in the paragraph's text
//!   (`presentation::line_items`), as an object set in the line does. Its
//!   cells' lines are placed against that box.
//! * A table's caption has its frame (`hcp`), which clips its lines as the
//!   source's caption box does, with `overflow:hidden`: that cuts on whole
//!   pixels, where a `clip-path` at the same edges paints the partly
//!   covered pixel row of a letter's tail fainter.
//! * A box whose layout may reach past its page (a hanging line wider than
//!   the paper, a line whose line height is a whole table, a drawing's
//!   gutter at the edge) stands in a page-size group that clips it
//!   (`hpg`, `data-hwpx-group="overflow"`). `clip-path` hides such paint
//!   but not the layout: the document would scroll past its last page, and
//!   a print would shrink to fit or add blank sheets (1.5 spike, DECISIONS
//!   "쪽 클립은 인쇄 배율 보존까지"). A line standing on its page with
//!   nothing around it (a body line) has the same group for its text
//!   (`data-hwpx-group="text"`): the browser's fonts, not the stored width,
//!   decide how far its letters reach.
//!
//! Positions add the lengths the page-by-page output writes, each snapped
//! to 1/64 px as the browser snaps it, so a box lands on the same layout
//! unit as there and the two outputs can be compared pixel by pixel.
//!
//! A caption with text above a page-level table is written as the table's
//! `caption`, its frame placed in the area it shares with the table
//! (`presentation::caption`) and its lines in the frame; the table moves
//! down in that area.
//!
//! A 덧말 (`dutmal`) is written as `ruby` in its control's place in the line:
//! the base text in flow, the annotation (`rt`) out of flow, across the base
//! text and above or below its em box. The line's box is the one of its
//! letters' size, moved down by the band the annotation takes
//! (`presentation::annotated_line_box`). A 겹친 글자 (`compose`) is written as
//! its characters in a box the width of its shape (`hco`).
//!
//! What the emitter cannot write yet (other captions with text, an annotation
//! the corpus does not show, ...) is refused with [`Unsupported`], never
//! drawn as a guess and never handed to the page-by-page renderer. A lenient
//! run ([`render_direct_lenient`], the converter's default) leaves out just
//! that part, or writes it more plainly (an annotation as its base text, a
//! number in an unknown format as digits, an object the renderer cannot draw
//! not at all), and reports each in `DirectReport::skipped`.
use std::collections::{BTreeMap, HashMap, HashSet};

use super::bundle::{self, RenderBundle, RenderContext};
use super::html::{self, escape_html, escape_html_attribute, GradientIds, PatternIds};
use super::reading;
use super::semantic::{plan_rows, Slot};
use super::units::{px, snap_mm, split, UNIT};
use super::{RenderOptions, UnsupportedPolicy};
use crate::layout::presentation::caption::{self as captions, TableCaptionPlan};
use crate::layout::presentation::{self, LineBox, LineItem, TablePlacement};
use crate::layout::{css_mm, css_mm_hundredths, hwp_to_centi_mm};
use crate::model::{
    Block, CharStyle, Document, HwpUnit, LayoutDocument, LineFill, LineFragment, PositionedObject,
    SourceToken, Table, Token, TokenKind,
};
use crate::semantic::{Inline, InlineSpan, Kind, Node, ParagraphSource};

/// What the emitter does not write yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Unsupported {
    pub what: &'static str,
    pub key: String,
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.what, self.key)
    }
}

impl std::error::Error for Unsupported {}

/// What the emitter did, for checking that each branch ran: a picture that
/// matches says nothing about a branch no document reached. Kept out of the
/// page, which carries no diagnostic metadata.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DirectReport {
    /// Every text box placed, in placing order.
    pub text_boxes: Vec<TextBoxRecord>,
    /// The boxes kept only to clip a layout to its page, by reason.
    pub groups: BTreeMap<String, usize>,
    /// Every equation written, in writing order.
    pub equations: Vec<EquationRecord>,
    /// Every caption placed, in placing order.
    pub captions: Vec<CaptionRecord>,
    /// Every annotated or overlapped part written, in writing order.
    pub annotations: Vec<AnnotationRecord>,
    /// The parts a lenient run left out, or wrote more plainly, each with its
    /// reason and the source key (always empty for a strict run, which refuses
    /// the document at the first one).
    pub skipped: Vec<Unsupported>,
}

/// One 덧말 or 겹친 글자 and how it was written.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AnnotationRecord {
    /// The paragraph's key.
    pub key: String,
    /// `ruby` or `compose`.
    pub kind: &'static str,
    /// The ruby's `posType`, or the compose's `circleType`.
    pub detail: String,
    /// For a ruby, the annotation's size and the offset of its box from the
    /// line's top (HWPUNIT).
    pub size: Option<HwpUnit>,
    pub offset: Option<HwpUnit>,
}

/// One table's caption and where it and its table were placed, in
/// HWPUNIT (`presentation::caption::TableCaptionPlan`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptionRecord {
    /// The caption's key (`{table}/caption`).
    pub key: String,
    /// The page it was placed on (from 1).
    pub page: usize,
    /// `inline` (its table set in a line) or `float`.
    pub host: &'static str,
    /// The area the caption and table share.
    pub width: HwpUnit,
    pub height: HwpUnit,
    /// The caption's frame in the area: left, top, width, height.
    pub caption: [HwpUnit; 4],
    /// The table's corner in the area.
    pub table: [HwpUnit; 2],
    pub paragraphs: usize,
    pub lines: usize,
    /// For an inline table, the height its line stores: the area's height
    /// when the source already reserved the caption there.
    pub line_height: Option<HwpUnit>,
}

/// One equation and how it was written.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EquationRecord {
    pub key: String,
    /// As MathML; otherwise its script's letters (syntax outside the subset).
    pub mathml: bool,
}

/// One text box and how its text was placed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TextBoxRecord {
    /// The object's source key.
    pub key: String,
    /// The page it was placed on (from 1).
    pub page: usize,
    pub layout: &'static str,
    /// The stroke its frame reserves room for, in HWPUNIT.
    pub pen: i64,
    /// Whether its frame clips the text.
    pub clip: bool,
    /// The lengths (HWPUNIT) that add up to the text's origin, each snapped
    /// on its own: across, then down.
    pub origin_x: Vec<i64>,
    pub origin_y: Vec<i64>,
    pub lines: usize,
    pub tables: usize,
}

fn refuse<T>(what: &'static str, key: &str) -> Result<T, Unsupported> {
    Err(Unsupported {
        what,
        key: key.to_owned(),
    })
}

/// Shows one page at a time (`RenderOptions::page_navigation`): the same bar,
/// keys and clicks as the page-by-page output's script, but the current page
/// is one rule naming its `data-pg`, not a class on every element of the page.
///
/// A browser's find reaches only the shown page, since the others are not
/// displayed. The bar's last button switches to every page in a row (and
/// back): the pages show as they do before the script runs and in print, the
/// bar's page controls and the keys and clicks that turn pages rest, and the
/// window goes to the current page. Back to one page, the page the window was
/// on is shown again, and a print in either view leaves the view it found.
pub const NAVIGATION_SCRIPT: &str = r#"(()=>{const root=document.documentElement;root.classList.add('hwpx-paged');addEventListener('beforeprint',()=>root.classList.remove('hwpx-paged'));document.addEventListener('DOMContentLoaded',()=>{const pages=[...document.querySelectorAll('.hpa')];if(pages.length<2){root.classList.remove('hwpx-paged');return;}let current=-1;const rule=document.createElement('style');document.head.append(rule);const make=(tag,props)=>Object.assign(document.createElement(tag),props);const nav=make('nav',{className:'hwpx-nav'});nav.setAttribute('aria-label','쪽 이동');const prev=make('button',{type:'button',textContent:'‹ 이전',title:'이전 쪽 (←)'});const next=make('button',{type:'button',textContent:'다음 ›',title:'다음 쪽 (→)'});const input=make('input',{type:'number',min:1,max:pages.length,title:'쪽 번호'});input.setAttribute('aria-label','쪽 번호');const view=make('button',{type:'button',textContent:'전체 보기',title:'모든 쪽을 이어서 보여 줍니다. 브라우저의 찾기(Ctrl+F)가 문서 전체를 찾습니다.'});view.setAttribute('aria-pressed','false');nav.append(prev,input,make('span',{textContent:'/ '+pages.length}),next,view);let full=false;const show=index=>{if(Number.isNaN(index))index=current;index=Math.max(0,Math.min(pages.length-1,Math.floor(index)));if(full){current=index;input.value=index+1;pages[index].scrollIntoView();return;}if(index!==current){const n=pages[index].dataset.page;rule.textContent='html.hwpx-paged [data-pg="'+n+'"],html.hwpx-paged .hpa[data-page="'+n+'"]{display:block}';current=index;scrollTo(0,0);}input.value=index+1;prev.disabled=index===0;next.disabled=index===pages.length-1;const hash='#page-'+(index+1);if(location.hash!==hash&&(location.hash||index))try{history.replaceState(null,'',hash);}catch(e){}};const fromHash=()=>{const m=/^#page-(\d+)$/.exec(location.hash);return m?m[1]-1:NaN;};prev.addEventListener('click',()=>show(current-1));next.addEventListener('click',()=>show(current+1));input.addEventListener('change',()=>show(input.valueAsNumber-1));view.addEventListener('click',()=>{const seen=full?Math.max(0,pages.findIndex(p=>p.getBoundingClientRect().bottom>40)):current;full=!full;root.classList.toggle('hwpx-paged',!full);view.textContent=full?'한 쪽 보기':'전체 보기';view.setAttribute('aria-pressed',String(full));prev.disabled=next.disabled=input.disabled=full;if(full){pages[current].scrollIntoView();}else{current=-1;show(seen);}});addEventListener('hashchange',()=>{const i=fromHash();if(!Number.isNaN(i))show(i);});addEventListener('keydown',e=>{if(full)return;const step=e.key==='ArrowLeft'?-1:e.key==='ArrowRight'?1:0;if(!step||e.defaultPrevented||e.altKey||e.ctrlKey||e.metaKey||e.shiftKey)return;if(e.target instanceof Element&&e.target.closest('input,textarea,select,[contenteditable]'))return;e.preventDefault();show(current+step);});let touch=false;addEventListener('pointerdown',e=>{touch=e.pointerType!=='mouse';},true);addEventListener('click',e=>{if(nav.contains(e.target)){if(touch)nav.classList.add('hwpx-show');return;}nav.classList.remove('hwpx-show');if(e.button||e.defaultPrevented||e.altKey||e.ctrlKey||e.metaKey||e.shiftKey||!root.classList.contains('hwpx-paged'))return;if(e.target instanceof Element&&e.target.closest('a[href],button,input,textarea,select,label,[contenteditable]'))return;const s=getSelection();if(s&&!s.isCollapsed)return;const r=pages[current].getBoundingClientRect(),x=e.clientX-r.left,w=r.width;if(x<0||x>=w||e.clientY<r.top||e.clientY>=r.bottom)return;const step=x<w/3?-1:x>=w*2/3?1:0;if(step)show(current+step);});nav.addEventListener('focusin',()=>{try{if(nav.querySelector(':focus-visible'))nav.classList.add('hwpx-show');}catch(e){}});nav.addEventListener('focusout',e=>{if(!nav.contains(e.relatedTarget))nav.classList.remove('hwpx-show');});addEventListener('afterprint',()=>{if(!full)root.classList.add('hwpx-paged');});document.body.prepend(nav);show(fromHash());root.classList.add('hwpx-ready');});})();"#;

/// The scripts `options` asks for, in document order.
fn scripts(options: &RenderOptions) -> Vec<&'static str> {
    if options.reading_view && options.logical_dom {
        return [
            (true, reading::READING_SCRIPT),
            (options.page_navigation, reading::NAVIGATION_SCRIPT),
            (options.adjust_letter_spacing, reading::CORRECTION_SCRIPT),
        ]
        .into_iter()
        .filter_map(|(on, script)| on.then_some(script))
        .collect();
    }
    [
        (options.page_navigation, NAVIGATION_SCRIPT),
        (options.adjust_letter_spacing, html::SCRIPT_SOURCE),
    ]
    .into_iter()
    .filter_map(|(on, script)| on.then_some(script))
    .collect()
}

/// A length the page-by-page output writes (`css_mm`) as the browser lays
/// it out: whole 1/64 px, cut toward zero, so that a length and its negative
/// cancel exactly.
fn snap(units: HwpUnit) -> i64 {
    let centi = hwp_to_centi_mm(units);
    let snapped = snap_mm(centi.unsigned_abs() as f64 / 100.0);
    if centi < 0 {
        -snapped
    } else {
        snapped
    }
}

/// A rectangle in 1/64 px.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

impl Rect {
    fn intersect(self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.w).min(other.x + other.w);
        let bottom = (self.y + self.h).min(other.y + other.h);
        Rect {
            x,
            y,
            w: (right - x).max(0),
            h: (bottom - y).max(0),
        }
    }

    fn union(self, other: Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = (self.x + self.w).max(other.x + other.w);
        let bottom = (self.y + self.h).max(other.y + other.h);
        Rect {
            x,
            y,
            w: right - x,
            h: bottom - y,
        }
    }

    fn contains(self, other: Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.w <= self.x + self.w
            && other.y + other.h <= self.y + self.h
    }
}

/// What the tables and objects set in a line place, from the corner of the box
/// that holds each (not of the line: the browser sets them among its text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inner {
    /// Their boxes, the contents of their boxes and their drawings.
    rect: Rect,
    /// How far the contents go past the right edge of the box holding them,
    /// the part of their reach no position in the line removes.
    overhang: i64,
}

impl Inner {
    /// What a box `host` holds, reaching as far as `rect`.
    fn holding(host: Rect, rect: Rect) -> Inner {
        Inner {
            rect,
            overhang: (rect.x + rect.w - (host.x + host.w)).max(0),
        }
    }

    /// Something that holds nothing past its own box.
    fn plain(rect: Rect) -> Inner {
        Inner::holding(rect, rect)
    }

    fn union(self, other: Inner) -> Inner {
        Inner {
            rect: self.rect.union(other.rect),
            overhang: self.overhang.max(other.overhang),
        }
    }
}

/// What a placed box's coordinates are measured from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// The page (index): the box carries the page's class and adds its
    /// place to the page's origin.
    Page(usize),
    /// The inline table it is drawn in: plain `left`/`top` against it.
    Local,
}

/// A line and where it goes.
struct PlacedLine<'a> {
    page: usize,
    frame: Frame,
    line: LineFragment,
    /// The line box's place in its frame.
    x: i64,
    y: i64,
    line_box: LineBox,
    /// How far its layout may reach, in its frame (see [`Emitter::reach`]).
    reach: Rect,
    /// What its paint is clipped to (its cell), in its frame.
    clip: Option<Rect>,
    /// The paint layer of the float it belongs to.
    z: Option<usize>,
    /// The tables set inline in this line (the first sizes its box).
    inline_tables: Vec<&'a Table>,
    /// It carries block content (a table, or an object with a text box), so
    /// it stands outside `p`.
    block: bool,
}

/// What a line with a 덧말 takes (`Emitter::annotation_metrics`).
#[derive(Debug, Clone, Copy)]
struct Annotated {
    /// The size of the line's letters.
    size: HwpUnit,
    /// The height above them the annotations take.
    band: HwpUnit,
}

fn is_ruby_control(token: &Token) -> bool {
    matches!(&token.kind, TokenKind::Control { kind } if kind == "dutmal")
}

fn is_composed_control(token: &Token) -> bool {
    matches!(&token.kind, TokenKind::Control { kind } if kind == "dutmal" || kind == "compose")
}

/// A 덧말's size on letters of `size`: `szRatio` 0 is the editor's 50%.
fn annotation_size(size: HwpUnit) -> HwpUnit {
    size / 2
}

/// Whether two character styles draw the same letters.
fn same_letters(a: &CharStyle, b: &CharStyle) -> bool {
    a.font_family == b.font_family
        && a.latin_font_family == b.latin_font_family
        && a.font_size_hwp == b.font_size_hwp
        && a.color == b.color
        && a.background == b.background
        && (a.bold, a.italic, a.emboss) == (b.bold, b.italic, b.emboss)
        && (a.superscript, a.subscript) == (b.superscript, b.subscript)
        && (a.underline, a.strike) == (b.underline, b.strike)
        && (a.baseline_offset, a.ratio, a.spacing) == (b.baseline_offset, b.ratio, b.spacing)
}

/// A paragraph's generated numbers (user decision Q2): by the paragraph token
/// of the `autoNum` control each replaces, and the number heading its first
/// line.
#[derive(Default)]
struct Numbers {
    by_token: HashMap<usize, String>,
    head: Option<String>,
    /// The 덧말 and 겹친 글자 of the paragraph, by the paragraph token of the
    /// control each is written in the place of.
    composed: HashMap<usize, Inline>,
}

/// A NUMBER paragraph's number at the head of its first line, in the box a
/// bullet takes (`html::render_bullet`), but as DOM text marked generated.
fn number_head(
    html: &mut String,
    line: &LineFragment,
    layout: &LayoutDocument,
    number: &str,
    height: HwpUnit,
) {
    let style = line.tokens.first().and_then(|token| {
        layout
            .char_styles
            .iter()
            .find(|style| style.id == token.char_style_id)
    });
    html.push_str(&format!(
        "<span class=\"hhe\" style=\"display:inline-block;margin-left:0mm;width:{};height:{};\"><span class=\"hrt cs{}\" data-hwpx-generated>{}</span></span>",
        css_mm(height),
        css_mm(height),
        style.map_or(0, |style| style.id),
        escape_html(number)
    ));
}

/// A floating object: the renderer draws it against a layer at `x`/`y` in
/// its frame (the page's objects container, or the cell holding it, the
/// object then moved by the cell's margins and alignment).
struct FloatObject {
    page: usize,
    frame: Frame,
    object: PositionedObject,
    x: i64,
    y: i64,
    /// Its cell's clip, in its frame.
    clip: Option<Rect>,
    z: Option<usize>,
}

/// A table's caption as placed: its plan, and the caption's frame, the box
/// that clips its lines (they are placed against its corner).
#[derive(Debug, Clone, Copy)]
struct PlacedCaption {
    plan: TableCaptionPlan,
    frame: Frame,
    /// The caption's frame in `frame`.
    rect: Rect,
    /// How far the frame and its lines may reach, in `frame`, the lines
    /// counted unclipped: the page's overflow group is decided as it was
    /// before the frame held them.
    reach: Rect,
    /// The paint layer of the floating table it belongs to.
    z: Option<usize>,
}

/// A table fragment's border drawing and where it goes. The table's box
/// (`htb`) is at `x`/`y` in the frame; the drawing reaches past it.
struct Drawing<'a> {
    page: usize,
    frame: Frame,
    table: &'a Table,
    placement: TablePlacement,
    x: i64,
    y: i64,
    reach: Rect,
    clip: Option<Rect>,
    z: Option<usize>,
}

/// Where a page's paper and origin lie on the screen, in 1/64 px.
struct PageFrame {
    /// The paper's left and top (its border included).
    paper: (i64, i64),
    /// Where the page's own coordinates start: inside the paper's border.
    origin: (i64, i64),
    /// The paper's size as the browser lays it out.
    size: (i64, i64),
    /// The paper's size in mm, for `@page`.
    size_mm: (String, String),
}

/// A millimetre length without its unit (`210` or `29.7`).
fn mm_text(hwp: HwpUnit) -> String {
    css_mm(hwp).trim_end_matches("mm").to_owned()
}

/// Where the flow of the old page-by-page markup put the pages: the body's
/// 2mm top padding, then per page a 1px border, the height, a 1px border and
/// a 2mm margin; 2mm from the left (each length snapped down to 1/64 px, as
/// Chrome does). Also the height of the whole flow.
fn page_frames(layout: &LayoutDocument) -> (Vec<PageFrame>, i64) {
    let gap = snap_mm(2.0);
    let mut screen = gap;
    let mut frames = Vec::with_capacity(layout.pages.len());
    for page in &layout.pages {
        let size = (snap(page.spec.width), snap(page.spec.height));
        frames.push(PageFrame {
            paper: (gap, screen),
            origin: (gap + UNIT, screen + UNIT),
            size,
            size_mm: (mm_text(page.spec.width), mm_text(page.spec.height)),
        });
        screen += UNIT + size.1 + UNIT + gap;
    }
    (frames, screen)
}

/// Screen flow height, in CSS px, with the existing border and both gaps.
/// Shares the emitter's snapped lengths; does not change output CSS.
pub(crate) fn screen_extents(layout: &LayoutDocument) -> (f64, f64) {
    let (frames, height) = page_frames(layout);
    let paged = frames
        .iter()
        .map(|frame| frame.size.1)
        .max()
        .map_or(snap_mm(2.0), |height| height + 2 * UNIT + 2 * snap_mm(2.0));
    (height as f64 / UNIT as f64, paged as f64 / UNIT as f64)
}

struct Emitter<'a> {
    layout: &'a LayoutDocument,
    /// Each page's own area, in its coordinates.
    page_rects: Vec<Rect>,
    /// Character style sizes, for how far a line's text may reach.
    font_sizes: HashMap<u32, HwpUnit>,
    /// The source tables, by id, for their full grid.
    sources: HashMap<&'a str, &'a Table>,
    /// The tree's tables, by key.
    table_nodes: HashMap<&'a str, &'a Node>,
    /// Every paragraph's lines, by paragraph key, in page order.
    lines: HashMap<String, Vec<PlacedLine<'a>>>,
    /// Repeated header rows drawn on a table's later pages, by table id.
    repeats: HashMap<String, Vec<PlacedLine<'a>>>,
    /// Each table fragment's border drawing, by table id.
    drawings: HashMap<String, Vec<Drawing<'a>>>,
    /// How many fragments of each table the layout placed.
    fragments: HashMap<String, usize>,
    /// Page-level floating objects, by key.
    floats: HashMap<String, Vec<FloatObject>>,
    /// The tree's objects with a text box, by the text box's first
    /// paragraph (the key in its typed content slot).
    text_owners: HashMap<&'a str, &'a Node>,
    /// The tree's objects, by key.
    object_nodes: HashMap<&'a str, &'a Node>,
    /// Text box tables from the placement plan, by id.
    text_box_tables: HashMap<&'a str, &'a Table>,
    /// Text box tables written at typed content slots in the object's paint, not
    /// after their anchor paragraph.
    deferred_tables: HashSet<String>,
    /// The tables whose caption was placed, by id: the area they share and
    /// the caption's frame.
    captions: HashMap<String, PlacedCaption>,
    patterns: Vec<PatternIds>,
    gradients: GradientIds,
    /// The renderer's context, for what the emitter draws with the
    /// renderer's own writers (objects set in a line).
    context: RenderContext<'a>,
    options: &'a RenderOptions,
    html: String,
    placed: usize,
    written: usize,
    tables_written: HashSet<String>,
    /// Objects a line has drawn, by key.
    objects_written: HashSet<String>,
    observe: bool,
    report: DirectReport,
    /// Leave out what cannot be written instead of refusing the document.
    lenient: bool,
    /// What a lenient run left out so far (see `DirectReport::skipped`).
    skipped: std::cell::RefCell<Vec<Unsupported>>,
    /// The paragraphs whose 덧말 are written as plain base text, not in the
    /// annotated line box (their layout is outside what the emitter knows).
    plain_annotations: std::cell::RefCell<HashSet<String>>,
}

impl<'a> Emitter<'a> {
    /// A part the emitter cannot write. A strict run refuses the document;
    /// a lenient one records the part (for the report) and the caller leaves
    /// it out, or writes it more plainly, and goes on.
    fn unsupported(&self, what: &'static str, key: &str) -> Result<(), Unsupported> {
        if self.lenient {
            let mut skipped = self.skipped.borrow_mut();
            if !skipped
                .iter()
                .any(|part| part.what == what && part.key == key)
            {
                skipped.push(Unsupported {
                    what,
                    key: key.to_owned(),
                });
            }
            Ok(())
        } else {
            refuse(what, key)
        }
    }

    /// [`caption_plan`], leaving out a caption the plan cannot place.
    fn caption_plan_of(&self, table: &Table) -> Result<Option<TableCaptionPlan>, Unsupported> {
        match caption_plan(table) {
            Err(why) if self.lenient => {
                self.unsupported(why.what, &why.key)?;
                Ok(None)
            }
            other => other,
        }
    }

    /// A paragraph whose 덧말 are written as plain base text (`Ok(None)`).
    fn plain_annotation(
        &self,
        what: &'static str,
        key: &str,
    ) -> Result<Option<Annotated>, Unsupported> {
        self.unsupported(what, key)?;
        self.plain_annotations.borrow_mut().insert(key.to_owned());
        Ok(None)
    }

    // ------------------------------------------------------------ placing

    /// Place every line of the layout: the pages' own lines, and the lines of
    /// every table fragment's cells.
    fn place(&mut self) -> Result<(), Unsupported> {
        let layout = self.layout;
        for (index, page) in layout.pages.iter().enumerate() {
            let inline_tables = page
                .tables
                .iter()
                .filter(|placed| placed.table.anchor.treat_as_char)
                .map(|placed| &placed.table)
                .collect::<Vec<_>>();
            let mut consumed = HashSet::new();
            let groups = presentation::column_groups(&page.lines);
            let (body_left, body_top) = presentation::body_origin(page);
            let containers = presentation::containers(&groups);
            for container in &containers {
                for line in groups[container.group]
                    .lines
                    .iter()
                    .filter(|line| line.column_index == container.column)
                {
                    let tables = inline_tables
                        .iter()
                        .filter(|table| table.anchor_y == Some(line.top))
                        .copied()
                        .collect::<Vec<_>>();
                    let mut plans = Vec::with_capacity(tables.len());
                    for (order, table) in tables.iter().enumerate() {
                        let plan = self.caption_plan_of(table)?;
                        // Only the first table sizes the line's box.
                        if order > 0 && plan.is_some() {
                            self.unsupported(
                                crate::diagnostic::reasons::CAPTION_ON_A_TABLE_AFTER_ANOTHER_IN_ITS_LINE,
                                &caption_key(table),
                            )?;
                            plans.push(None);
                            continue;
                        }
                        plans.push(plan);
                    }
                    // The line holds the area a caption shares with its
                    // table; its stored height already includes the caption.
                    let line_box = match (tables.first(), plans.first()) {
                        (Some(table), Some(Some(plan))) => {
                            presentation::host_line_box(line, Some((table, plan.height)))
                        }
                        (table, _) => presentation::line_box(line, table.copied()),
                    };
                    let line_box = self.annotated_box(line, line_box, !tables.is_empty(), false)?;
                    let x = snap(body_left) + snap(container.left) + snap(line.left);
                    let y = snap(body_top) + snap(container.top) + snap(line_box.top);
                    let mut inner: Option<Inner> = None;
                    for (table, plan) in tables.iter().zip(plans) {
                        consumed.insert(table.id.as_str());
                        let placed = match plan {
                            Some(plan) => {
                                self.captioned_inline_table(index, table, plan, line.height)?
                            }
                            None => self.inline_table_placement(index, table)?,
                        };
                        inner = Some(inner.map_or(placed, |inner| inner.union(placed)));
                    }
                    let inner = self.inline_objects_placement(index, line, inner)?;
                    let reach = self.reach(line, line_box, x, y, inner);
                    self.add_line(PlacedLine {
                        page: index,
                        frame: Frame::Page(index),
                        line: (*line).clone(),
                        x,
                        y,
                        line_box,
                        reach,
                        clip: None,
                        z: None,
                        block: !tables.is_empty() || line.inline_objects.iter().any(has_text_box),
                        inline_tables: tables,
                    });
                }
            }
            // The objects the renderer writes after the last column group's
            // first column, against it, and in the page's first paint layers.
            let (objects_left, objects_top) = containers
                .iter()
                .find(|container| container.group + 1 == groups.len() && container.column == 0)
                .map_or((0, 0), |container| (container.left, container.top));
            for (order, object) in page.objects.iter().enumerate() {
                self.check_object(object)?;
                self.place_text_boxes(index, object, false)?;
                self.floats
                    .entry(object.key.clone())
                    .or_default()
                    .push(FloatObject {
                        page: index,
                        frame: Frame::Page(index),
                        object: object.clone(),
                        x: snap(body_left) + snap(objects_left),
                        y: snap(body_top) + snap(objects_top),
                        clip: None,
                        z: Some(order + 1),
                    });
            }
            for table in inline_tables
                .iter()
                .filter(|table| !consumed.contains(table.id.as_str()))
            {
                self.unsupported(
                    crate::diagnostic::reasons::INLINE_TABLE_IN_NO_LINE,
                    &table.id,
                )?;
            }
            // The floats in their paint order, as the page-by-page output
            // layers them: objects first, then tables by z-order.
            let mut floats = page
                .tables
                .iter()
                .filter(|placed| !placed.table.anchor.treat_as_char)
                .collect::<Vec<_>>();
            floats.sort_by_key(|placed| (placed.table.anchor.z_order, placed.fragment_index));
            for (order, placed) in floats.into_iter().enumerate() {
                let layer = page.objects.len() + order + 1;
                let table = &placed.table;
                let (left, top) = presentation::floating_table_origin(&page.spec, table);
                let mut plan = self.caption_plan_of(table)?;
                // With a caption the origin is the area's; the table stands
                // lower in it. Only margins of zero are shown (1151677651),
                // where the origin has no margin in it to take out.
                if plan.is_some()
                    && [
                        table.out_margin_left,
                        table.out_margin_right,
                        table.out_margin_top,
                        table.out_margin_bottom,
                    ]
                    .iter()
                    .any(|margin| *margin != 0)
                {
                    self.unsupported(
                        crate::diagnostic::reasons::CAPTION_ON_A_FLOATING_TABLE_WITH_OUTER_MARGINS,
                        &caption_key(table),
                    )?;
                    plan = None;
                }
                let area = (snap(left), snap(top));
                let (placement, origin) = match plan {
                    Some(plan) => (
                        TablePlacement::Page {
                            left: left + plan.table_left,
                            top: top + plan.table_top,
                            frame_adjustment: true,
                        },
                        (
                            area.0 + snap(plan.table_left),
                            area.1 + snap(plan.table_top),
                        ),
                    ),
                    None => (
                        TablePlacement::Page {
                            left,
                            top,
                            frame_adjustment: true,
                        },
                        area,
                    ),
                };
                if let Some(plan) = plan {
                    self.place_caption(
                        index,
                        table,
                        plan,
                        (Frame::Page(index), area),
                        Some(layer),
                        None,
                    )?;
                }
                self.add_drawing(Drawing {
                    page: index,
                    frame: Frame::Page(index),
                    table,
                    placement,
                    x: origin.0,
                    y: origin.1,
                    reach: svg_reach(table, placement, origin),
                    clip: None,
                    z: Some(layer),
                });
                self.place_table(
                    index,
                    table,
                    Frame::Page(index),
                    origin,
                    None,
                    Some(layer),
                    false,
                )?;
            }
        }
        Ok(())
    }

    /// Place an inline table's cells against its own box, and return how far
    /// they and its drawing reach from that box's corner.
    fn inline_table_placement(
        &mut self,
        page: usize,
        table: &'a Table,
    ) -> Result<Inner, Unsupported> {
        let own = presentation::table_box(table, TablePlacement::Inline);
        let host = Rect {
            x: 0,
            y: 0,
            w: snap(own.width),
            h: snap(own.height),
        };
        let reach = svg_reach(table, TablePlacement::Inline, (0, 0)).union(host);
        let cells = self.place_table(page, table, Frame::Local, (0, 0), None, None, true)?;
        Ok(Inner::holding(host, reach.union(cells)))
    }

    /// [`Self::inline_table_placement`] for a table sharing its box with its
    /// caption: the box is the whole area, the table's cells and drawing
    /// stand at the table's corner in it and the caption's lines at the
    /// caption's. `line_height` is what the carrying line stores.
    fn captioned_inline_table(
        &mut self,
        page: usize,
        table: &'a Table,
        plan: TableCaptionPlan,
        line_height: HwpUnit,
    ) -> Result<Inner, Unsupported> {
        let origin = (snap(plan.table_left), snap(plan.table_top));
        let svg = captions::table_svg(table);
        let host = Rect {
            x: 0,
            y: 0,
            w: snap(plan.width),
            h: snap(plan.height),
        };
        let reach = host.union(Rect {
            x: origin.0 + snap(svg.left),
            y: origin.1 + snap(svg.top),
            w: snap(svg.width),
            h: snap(svg.height),
        });
        let cells = self.place_table(page, table, Frame::Local, origin, None, None, true)?;
        let caption = self.place_caption(
            page,
            table,
            plan,
            (Frame::Local, (0, 0)),
            None,
            Some(line_height),
        )?;
        Ok(Inner::holding(host, reach.union(cells).union(caption)))
    }

    /// Place a table's caption frame in the area it shares with the table,
    /// whose corner is `area` in its frame, and the caption's lines in the
    /// frame; return how far they reach. The frame clips the lines as the
    /// reference's caption box does, with `overflow:hidden`: its edges are
    /// cut on whole pixels, where a line's `clip-path` at the same edges
    /// paints the partly covered pixel row of a letter's tail fainter.
    fn place_caption(
        &mut self,
        page: usize,
        table: &Table,
        plan: TableCaptionPlan,
        area: (Frame, (i64, i64)),
        z: Option<usize>,
        line_height: Option<HwpUnit>,
    ) -> Result<Rect, Unsupported> {
        let (frame, (left, top)) = area;
        let rect = Rect {
            x: left + snap(plan.caption.left),
            y: top + snap(plan.caption.top),
            w: snap(plan.caption.width),
            h: snap(plan.caption.height),
        };
        let mut extent = rect;
        let mut lines = 0;
        let paragraphs = table
            .caption
            .as_deref()
            .map_or(&[][..], |caption| caption.paragraphs.as_slice());
        for paragraph in paragraphs {
            for line in crate::layout::paragraph_fragments_for_render(paragraph) {
                lines += 1;
                let line_box =
                    self.annotated_box(&line, presentation::line_box(&line, None), false, false)?;
                let (x, y) = (snap(line.left), snap(line_box.top));
                let reach = self.reach(&line, line_box, x, y, None);
                extent = extent.union(Rect {
                    x: rect.x + reach.x,
                    y: rect.y + reach.y,
                    ..reach
                });
                self.add_line(PlacedLine {
                    page,
                    frame: Frame::Local,
                    line,
                    x,
                    y,
                    line_box,
                    reach,
                    clip: None,
                    z: None,
                    inline_tables: Vec::new(),
                    block: false,
                });
            }
        }
        self.captions.insert(
            table.id.clone(),
            PlacedCaption {
                plan,
                frame,
                rect,
                reach: extent,
                z,
            },
        );
        self.report.captions.push(CaptionRecord {
            key: caption_key(table),
            page: page + 1,
            host: if line_height.is_some() {
                "inline"
            } else {
                "float"
            },
            width: plan.width,
            height: plan.height,
            caption: [
                plan.caption.left,
                plan.caption.top,
                plan.caption.width,
                plan.caption.height,
            ],
            table: [plan.table_left, plan.table_top],
            paragraphs: paragraphs.len(),
            lines,
            line_height,
        });
        Ok(extent)
    }

    /// How far a line's layout may reach from its box: its box (with its
    /// hanging indent), the text of its largest letters below its line
    /// height, and the table set in it, which the browser puts after the
    /// text, at most a line height lower and, with the box fitting the line,
    /// at most as far right as leaves room for the box: its contents then
    /// reach the line's right edge and their overhang past the box. The
    /// text before it (spaces, alignment) is not measured here, so this does
    /// not guess the table's place. Deliberately generous: a line beyond it
    /// would widen or lengthen the document.
    fn reach(
        &self,
        line: &LineFragment,
        line_box: LineBox,
        x: i64,
        y: i64,
        inner: Option<Inner>,
    ) -> Rect {
        let font = line
            .tokens
            .iter()
            .filter_map(|token| self.font_sizes.get(&token.char_style_id))
            .max()
            .copied()
            .unwrap_or(1000);
        let width = snap(line.padding_left.max(0)) + snap(line.width);
        let below = snap(line_box.height).max(snap(line_box.line_height) + snap(font * 3 / 2));
        let mut reach = Rect {
            x,
            y: y - snap(font),
            w: width,
            h: below + snap(font),
        };
        if let Some(Inner { rect, overhang }) = inner {
            reach = reach.union(Rect {
                x: x + rect.x,
                y: y + rect.y,
                w: rect.w.max(width + overhang - rect.x),
                h: rect.h + snap(line_box.line_height),
            });
        }
        reach
    }

    /// Refuse what the emitter cannot write in an object yet: a caption's
    /// words (not laid out at all).
    fn check_object(&self, object: &PositionedObject) -> Result<(), Unsupported> {
        if object.kind == "equation" {
            if object.equation.is_none() {
                self.unsupported(
                    crate::diagnostic::reasons::EQUATION_WITHOUT_ITS_SCRIPT,
                    &object.key,
                )?;
            } else if !object.anchor.treat_as_char {
                self.unsupported(
                    crate::diagnostic::reasons::EQUATION_NOT_SET_IN_A_LINE,
                    &object.key,
                )?;
            }
        }
        if object.kind == "pic"
            && object
                .binary_ref
                .as_deref()
                .and_then(|reference| {
                    html::document_asset(self.layout, reference)
                        .and_then(|asset| self.context.asset_url(asset))
                })
                .is_none()
        {
            // Lenient, the renderer writes nothing for it (`UnsupportedPolicy::Skip`).
            self.unsupported(
                crate::diagnostic::reasons::PICTURE_WITHOUT_A_SUPPORTED_IMAGE_RESOURCE,
                &object.key,
            )?;
        }
        if object.shape.is_none()
            && !matches!(object.kind.as_str(), "pic" | "container" | "equation")
        {
            self.unsupported(crate::diagnostic::reasons::UNSUPPORTED_OBJECT, &object.key)?;
        }
        if object.caption.as_ref().is_some_and(|caption| {
            caption.paragraphs.iter().any(|paragraph| {
                paragraph
                    .tokens
                    .iter()
                    .any(|token| !token.visible_text().trim().is_empty())
            })
        }) {
            // An object's caption is not written either way.
            self.unsupported(crate::diagnostic::reasons::CAPTION_WITH_TEXT, &object.key)?;
        }
        object
            .children
            .iter()
            .try_for_each(|child| self.check_object(child))
    }

    /// Whether the picture a shape is filled with can be written.
    fn image_available(&self, reference: &str) -> bool {
        html::document_asset(self.layout, reference)
            .and_then(|asset| self.context.asset_url(asset))
            .is_some()
    }

    /// Place the lines of an object's text boxes (its own and its
    /// children's), each against the frame the renderer writes it in. The
    /// text's origin in that frame (margins, half the pen, vertical
    /// alignment) is added to each line, each length snapped as the browser
    /// snaps it: no box between the frame and the lines carries it.
    /// `shared_child` says the object is a child of a shared-`svg` container.
    fn place_text_boxes(
        &mut self,
        page: usize,
        object: &PositionedObject,
        shared_child: bool,
    ) -> Result<(), Unsupported> {
        let plan = presentation::objects::text_box_plan(object, shared_child, &|reference| {
            self.image_available(reference)
        });
        if let (Some(shape), Some(plan)) = (&object.shape, plan) {
            let origin = plan.origin();
            let origin_x: i64 = origin.x_terms().into_iter().map(snap).sum();
            let origin_y: i64 = origin.y_terms().map(snap).sum();
            let mut lines = 0;
            for paragraph in &shape.paragraphs {
                for line in crate::layout::paragraph_fragments_for_render(paragraph) {
                    lines += 1;
                    let line_box = self.annotated_box(
                        &line,
                        presentation::line_box(&line, None),
                        false,
                        false,
                    )?;
                    let (x, y) = (origin_x + snap(line.left), origin_y + snap(line_box.top));
                    let inner = self.inline_objects_placement(page, &line, None)?;
                    let reach = self.reach(&line, line_box, x, y, inner);
                    let block = line.inline_objects.iter().any(has_text_box);
                    self.add_line(PlacedLine {
                        page,
                        frame: Frame::Local,
                        line,
                        x,
                        y,
                        line_box,
                        reach,
                        clip: None,
                        z: None,
                        inline_tables: Vec::new(),
                        block,
                    });
                }
            }
            self.report.text_boxes.push(TextBoxRecord {
                key: object.key.clone(),
                page: page + 1,
                layout: plan.layout.name(),
                pen: plan.pen,
                clip: plan.layout.clips(),
                origin_x: origin.x_terms().to_vec(),
                origin_y: origin.y_terms().collect(),
                lines,
                tables: shape.tables.len(),
            });
        }
        if let Some(shape) = &object.shape {
            for source in &shape.tables {
                let Some(table) = self.text_box_tables.get(source.id.as_str()).copied() else {
                    self.unsupported(
                        crate::diagnostic::reasons::TEXT_BOX_TABLE_NOT_LAID_OUT,
                        &source.id,
                    )?;
                    continue;
                };
                let own = presentation::table_box(table, TablePlacement::Nested);
                let origin = (snap(own.left), snap(own.top));
                self.add_drawing(Drawing {
                    page,
                    frame: Frame::Local,
                    table,
                    placement: TablePlacement::Nested,
                    x: origin.0,
                    y: origin.1,
                    reach: svg_reach(table, TablePlacement::Nested, origin),
                    clip: None,
                    z: None,
                });
                self.place_table(page, table, Frame::Local, origin, None, None, true)?;
                self.deferred_tables.insert(table.id.clone());
            }
        }
        let shared = presentation::objects::shared_fill_group(object);
        for child in &object.children {
            self.place_text_boxes(page, child, shared)?;
        }
        Ok(())
    }

    /// Check and place the objects set in a line, and return how far they
    /// reach from the line's corner, joined to `inner`.
    fn inline_objects_placement(
        &mut self,
        page: usize,
        line: &LineFragment,
        inner: Option<Inner>,
    ) -> Result<Option<Inner>, Unsupported> {
        for object in &line.inline_objects {
            self.check_object(object)?;
            self.place_text_boxes(page, object, false)?;
        }
        Ok(inline_objects_reach(line, inner))
    }

    fn add_line(&mut self, placed: PlacedLine<'a>) {
        self.placed += 1;
        self.lines
            .entry(placed.line.paragraph_key.clone())
            .or_default()
            .push(placed);
    }

    fn add_drawing(&mut self, drawing: Drawing<'a>) {
        self.drawings
            .entry(drawing.table.id.clone())
            .or_default()
            .push(drawing);
    }

    /// Place one table fragment's cells, its box at `origin` in `frame`, and
    /// return how far what it placed reaches.
    #[allow(clippy::too_many_arguments)]
    fn place_table(
        &mut self,
        page: usize,
        table: &'a Table,
        frame: Frame,
        origin: (i64, i64),
        clip: Option<Rect>,
        z: Option<usize>,
        nested: bool,
    ) -> Result<Rect, Unsupported> {
        let count = self.fragments.entry(table.id.clone()).or_default();
        *count += 1;
        // A page-level table is split into fragments by the layout; a table
        // in a cell is drawn once, by the one piece of its cell holding it.
        if *count > 1 && nested {
            self.unsupported(
                crate::diagnostic::reasons::NESTED_TABLE_DRAWN_MORE_THAN_ONCE,
                &table.id,
            )?;
            return Ok(Rect {
                x: origin.0,
                y: origin.1,
                w: 0,
                h: 0,
            });
        }
        let mut extent = Rect {
            x: origin.0,
            y: origin.1,
            w: 0,
            h: 0,
        };
        let use_metric_offset = table.page_break.eq_ignore_ascii_case("CELL");
        for cell in &table.cells {
            let cell_rect = Rect {
                x: origin.0 + snap(cell.box_units.x - table.box_units.x),
                y: origin.1 + snap(cell.box_units.y - table.box_units.y),
                w: snap(cell.box_units.width),
                h: snap(cell.box_units.height),
            };
            extent = extent.union(cell_rect);
            // A continuation piece whose lines all stayed on another page.
            if cell.paragraphs.is_empty() && cell.tables.is_empty() {
                continue;
            }
            let cell_clip = clip.map_or(cell_rect, |outer| outer.intersect(cell_rect));
            let vertical_offset = presentation::cell_vertical_offset(table, cell);
            let content = (
                cell_rect.x + snap(cell.margin_left),
                cell_rect.y + snap(cell.margin_top) + snap(vertical_offset),
            );
            let inline_tables = cell
                .tables
                .iter()
                .filter(|nested| nested.anchor.treat_as_char)
                .collect::<Vec<_>>();
            let mut consumed = HashSet::new();
            for paragraph in &cell.paragraphs {
                for object in paragraph
                    .objects
                    .iter()
                    .filter(|object| !object.anchor.treat_as_char)
                {
                    if cell.repeated_header {
                        self.unsupported(
                            crate::diagnostic::reasons::FLOATING_OBJECT_IN_A_REPEATED_HEADER_ROW,
                            &cell.id,
                        )?;
                        continue;
                    }
                    self.check_object(object)?;
                    self.place_text_boxes(page, object, false)?;
                    let mut moved = object.clone();
                    moved.box_units.x = moved.box_units.x.saturating_add(cell.margin_left);
                    moved.box_units.y = moved
                        .box_units
                        .y
                        .saturating_add(cell.margin_top)
                        .saturating_add(vertical_offset);
                    extent = extent.union(Rect {
                        x: cell_rect.x + snap(moved.box_units.x),
                        y: cell_rect.y + snap(moved.box_units.y),
                        w: snap(moved.box_units.width.max(1)),
                        h: snap(moved.box_units.height.max(1)),
                    });
                    self.floats
                        .entry(object.key.clone())
                        .or_default()
                        .push(FloatObject {
                            page,
                            frame,
                            object: moved,
                            x: cell_rect.x,
                            y: cell_rect.y,
                            clip: Some(cell_clip),
                            z,
                        });
                }
                for line in crate::layout::paragraph_fragments_for_render_with_metric_offset(
                    paragraph,
                    use_metric_offset,
                ) {
                    let tables = inline_tables
                        .iter()
                        .filter(|nested| nested.anchor_y == Some(line.top))
                        .copied()
                        .collect::<Vec<_>>();
                    if cell.repeated_header && !tables.is_empty() {
                        self.unsupported(
                            crate::diagnostic::reasons::TABLE_IN_A_REPEATED_HEADER_ROW,
                            &cell.id,
                        )?;
                        continue;
                    }
                    let line_box = self.annotated_box(
                        &line,
                        presentation::line_box(&line, tables.first().copied()),
                        !tables.is_empty(),
                        use_metric_offset,
                    )?;
                    let x = content.0 + snap(line.left);
                    let y = content.1 + snap(line_box.top);
                    let mut inner: Option<Inner> = None;
                    for nested in &tables {
                        consumed.insert(nested.id.as_str());
                        let placed = self.inline_table_placement(page, nested)?;
                        inner = Some(inner.map_or(placed, |inner| inner.union(placed)));
                    }
                    if cell.repeated_header && line.inline_objects.iter().any(has_text_box) {
                        self.unsupported(
                            crate::diagnostic::reasons::OBJECT_WITH_TEXT_IN_A_REPEATED_HEADER_ROW,
                            &cell.id,
                        )?;
                        continue;
                    }
                    let inner = if cell.repeated_header {
                        inline_objects_reach(&line, inner)
                    } else {
                        self.inline_objects_placement(page, &line, inner)?
                    };
                    let reach = self.reach(&line, line_box, x, y, inner);
                    extent = extent.union(reach);
                    let block = !tables.is_empty() || line.inline_objects.iter().any(has_text_box);
                    let placed = PlacedLine {
                        page,
                        frame,
                        x,
                        y,
                        line,
                        line_box,
                        reach,
                        clip: Some(cell_clip),
                        z,
                        inline_tables: tables,
                        block,
                    };
                    if cell.repeated_header {
                        self.repeats
                            .entry(table.id.clone())
                            .or_default()
                            .push(placed);
                    } else {
                        self.add_line(placed);
                    }
                }
            }
            for nested in cell
                .tables
                .iter()
                .filter(|nested| !consumed.contains(nested.id.as_str()))
            {
                if cell.repeated_header {
                    self.unsupported(
                        crate::diagnostic::reasons::TABLE_IN_A_REPEATED_HEADER_ROW,
                        &cell.id,
                    )?;
                    continue;
                }
                // An inline table no line carries stays in the content
                // area; a floating one moves by the cell's margins and
                // alignment and is placed against the cell.
                let box_origin = if nested.anchor.treat_as_char {
                    let own = presentation::table_box(nested, TablePlacement::Nested);
                    (content.0 + snap(own.left), content.1 + snap(own.top))
                } else {
                    let offset = presentation::table_frame_offset(nested.out_margin_left);
                    (
                        cell_rect.x + snap(nested.box_units.x + cell.margin_left + offset),
                        cell_rect.y
                            + snap(nested.box_units.y + cell.margin_top + vertical_offset + offset),
                    )
                };
                let reach = svg_reach(nested, TablePlacement::Nested, box_origin);
                extent = extent.union(reach);
                self.add_drawing(Drawing {
                    page,
                    frame,
                    table: nested,
                    placement: TablePlacement::Nested,
                    x: box_origin.0,
                    y: box_origin.1,
                    reach,
                    clip: Some(cell_clip),
                    z,
                });
                let placed =
                    self.place_table(page, nested, frame, box_origin, Some(cell_clip), z, true)?;
                extent = extent.union(placed);
            }
        }
        Ok(extent)
    }

    // ------------------------------------------------------------ writing

    /// Write a node. A lenient run that fails inside one (what no finer rule
    /// above leaves out) drops that node's markup, accounts for the lines and
    /// drawings it still held, and goes on with the next.
    fn node(&mut self, node: &'a Node) -> Result<(), Unsupported> {
        if !self.lenient {
            return self.node_inner(node);
        }
        let mark = self.html.len();
        match self.node_inner(node) {
            Err(why) => {
                self.html.truncate(mark);
                self.drop_subtree(node);
                self.skipped.borrow_mut().push(why);
                Ok(())
            }
            written => written,
        }
    }

    /// Forget what the layout placed for `node` and its descendants, as if
    /// it had been written: its lines count as written and its tables and
    /// objects as done, so nothing placed is left over.
    fn drop_subtree(&mut self, node: &'a Node) {
        if let Some(lines) = self.lines.remove(node.key.as_str()) {
            self.written += lines.len();
        }
        self.repeats.remove(node.key.as_str());
        self.drawings.remove(node.key.as_str());
        self.floats.remove(node.key.as_str());
        self.tables_written.insert(node.key.clone());
        self.objects_written.insert(node.key.clone());
        // The markup that defined a solid-fill pattern may be gone: later
        // shapes of the page define theirs again.
        for patterns in &mut self.patterns {
            patterns.forget_colors();
        }
        for child in &node.children {
            self.drop_subtree(child);
        }
    }

    fn node_inner(&mut self, node: &'a Node) -> Result<(), Unsupported> {
        match &node.kind {
            Kind::Document | Kind::Section { .. } => {
                for child in &node.children {
                    self.node(child)?;
                }
                Ok(())
            }
            Kind::Paragraph { content, source } => self.paragraph(node, content, source, None),
            Kind::Heading {
                level,
                inferred,
                content,
                source,
            } => self.paragraph(node, content, source, Some((*level, *inferred))),
            Kind::List {
                ordered,
                inferred,
                start,
            } => {
                let tag = if *ordered { "ol" } else { "ul" };
                self.open(tag, node);
                if let Some(start) = start {
                    self.html.push_str(&format!(" start=\"{start}\""));
                }
                if *inferred {
                    self.html.push_str(" data-inferred=\"list\"");
                }
                self.html.push('>');
                for child in &node.children {
                    self.node(child)?;
                }
                self.close(tag);
                Ok(())
            }
            Kind::Item { value } => {
                self.open("li", node);
                if let Some(value) = value {
                    self.html.push_str(&format!(" value=\"{value}\""));
                }
                self.html.push('>');
                for child in &node.children {
                    self.node(child)?;
                }
                self.close("li");
                Ok(())
            }
            Kind::Table { .. } => self.floating_table(node),
            Kind::Cell { .. } => self.unsupported(
                crate::diagnostic::reasons::CELL_OUTSIDE_ITS_TABLE,
                &node.key,
            ),
            Kind::Object { .. } => self.floating_object(node),
            Kind::Caption { .. } => self.unsupported(
                crate::diagnostic::reasons::CAPTION_OUTSIDE_ITS_TABLE,
                &node.key,
            ),
        }
    }

    /// `<tag` and, when observing, the source key; the caller closes the tag.
    fn open(&mut self, tag: &str, node: &Node) {
        self.html.push('<');
        self.html.push_str(tag);
        if self.observe {
            self.html.push_str(" data-hwpx-id=\"");
            self.html.push_str(&escape_html_attribute(&node.key));
            self.html.push('"');
        }
    }

    fn close(&mut self, tag: &str) {
        self.html.push_str("</");
        self.html.push_str(tag);
        self.html.push('>');
    }

    fn paragraph(
        &mut self,
        node: &'a Node,
        content: &[InlineSpan],
        source: &ParagraphSource,
        heading: Option<(u32, bool)>,
    ) -> Result<(), Unsupported> {
        let mut blank = true;
        let mut numbers = Numbers::default();
        for (index, span) in content.iter().enumerate() {
            match &span.value {
                Inline::Text { text, .. } => blank &= text.trim().is_empty(),
                Inline::Tab
                | Inline::LineBreak
                | Inline::NonBreakingSpace
                | Inline::FixedSpace
                | Inline::Anchor { .. } => {}
                // User decision Q2: a generated number is text in the DOM,
                // marked as generated. An `autoNum` goes where its control
                // is in the line; a NUMBER paragraph's number (no source
                // token) heads its first line, as a bullet does.
                Inline::Generated { text } => match span.source_index {
                    Some(at) => match source.tokens.get(at).and_then(|token| token.token_index) {
                        Some(token) => {
                            numbers.by_token.insert(token, text.clone());
                        }
                        None => self.unsupported(
                            crate::diagnostic::reasons::GENERATED_NUMBER_WITHOUT_ITS_CONTROL,
                            &node.key,
                        )?,
                    },
                    None if index == 0 => numbers.head = Some(text.clone()),
                    None => self.unsupported(
                        crate::diagnostic::reasons::GENERATED_NUMBER_WITHOUT_ITS_CONTROL,
                        &node.key,
                    )?,
                },
                // Lenient, the number stays as digits.
                Inline::UnformattedNumber { number, .. } if self.lenient => {
                    self.unsupported(
                        crate::diagnostic::reasons::NUMBER_IN_AN_UNKNOWN_FORMAT,
                        &node.key,
                    )?;
                    let text = number.to_string();
                    match span.source_index {
                        Some(at) => {
                            if let Some(token) =
                                source.tokens.get(at).and_then(|token| token.token_index)
                            {
                                numbers.by_token.insert(token, text);
                            }
                        }
                        None if index == 0 => numbers.head = Some(text),
                        None => {}
                    }
                }
                Inline::UnformattedNumber { .. } => {
                    return refuse(
                        crate::diagnostic::reasons::NUMBER_IN_AN_UNKNOWN_FORMAT,
                        &node.key,
                    )
                }
                // A 덧말 or 겹친 글자 goes where its control is in the line.
                Inline::Ruby { .. } | Inline::Compose { .. } => {
                    blank = false;
                    match span.source_index.and_then(|at| source.tokens.get(at)) {
                        Some(SourceToken {
                            token_index: Some(token),
                            ..
                        }) => {
                            numbers.composed.insert(*token, span.value.clone());
                        }
                        _ => self.unsupported(
                            crate::diagnostic::reasons::ANNOTATION_WITHOUT_ITS_CONTROL,
                            &node.key,
                        )?,
                    }
                }
            }
        }
        let lines = self.lines.remove(node.key.as_str()).unwrap_or_default();
        if !blank && lines.is_empty() {
            self.unsupported(
                crate::diagnostic::reasons::PARAGRAPH_THE_LAYOUT_DREW_NO_LINE_FOR,
                &node.key,
            )?;
        }
        if lines.iter().any(|line| line.block) {
            // A line carrying a table is no phrasing content: the paragraph
            // is a container of its lines, the others grouped in `p`.
            if heading.is_some() {
                self.unsupported(
                    crate::diagnostic::reasons::HEADING_CARRYING_A_TABLE,
                    &node.key,
                )?;
            }
            self.open("div", node);
            if self.options.reading_view {
                self.html
                    .push_str(&format!(" class=\"hwpx-rp{}\"", source.style_id));
            }
            self.html.push_str(" data-hwpx-paragraph>");
            let mut in_p = false;
            for placed in &lines {
                let block = placed.block;
                if block && in_p {
                    self.html.push_str("</p>");
                    in_p = false;
                } else if !block && !in_p {
                    self.html.push_str("<p>");
                    in_p = true;
                }
                self.line(placed, false, &numbers)?;
                numbers.head = None;
            }
            if in_p {
                self.html.push_str("</p>");
            }
            self.close("div");
        } else {
            let tag = match heading {
                Some((level, _)) => {
                    ["h1", "h2", "h3", "h4", "h5", "h6"][(level.clamp(1, 6) - 1) as usize]
                }
                None if lines
                    .first()
                    .map_or(blank, |line| line.line.paragraph_empty) =>
                {
                    "div"
                }
                None => "p",
            };
            self.open(tag, node);
            if self.options.reading_view {
                self.html
                    .push_str(&format!(" class=\"hwpx-rp{}\"", source.style_id));
            }
            match heading {
                Some((_, true)) => self.html.push_str(" data-inferred=\"heading\""),
                Some(_) => {}
                None if tag == "div" => self.html.push_str(" data-hwpx-empty"),
                None => {}
            }
            self.html.push('>');
            for placed in &lines {
                self.line(placed, false, &numbers)?;
                numbers.head = None;
            }
            if lines.is_empty() && blank {
                // The layout drew no line for a paragraph of spaces: one
                // that only hosts a floating table drops its line with the
                // page break (`drop_pending_trivial_line`; 30 of 성과보고서's
                // 466 space paragraphs). Its text stays in the document as the
                // source has it, once, in its own element: plain spaces,
                // which a block does not draw, so no line, height or page
                // follows from them.
                for span in content {
                    if let Inline::Text { text, .. } = &span.value {
                        self.html.push_str(&escape_html(text));
                    }
                }
            }
            self.close(tag);
        }
        // Its anchored tables and objects follow it (D28), unless a line
        // already carried them.
        for child in &node.children {
            if self.tables_written.contains(child.key.as_str())
                || self.objects_written.contains(child.key.as_str())
                || self.deferred_tables.contains(child.key.as_str())
            {
                continue;
            }
            match &child.kind {
                Kind::Table { .. } => self.floating_table(child)?,
                Kind::Object { .. } => self.floating_object(child)?,
                _ => self.unsupported(crate::diagnostic::reasons::PARAGRAPH_CHILD, &child.key)?,
            }
        }
        Ok(())
    }

    /// The page whose overflow group a box at `reach` in `frame` needs: a
    /// page-framed box whose layout may leave its page.
    fn group_for(&self, frame: Frame, reach: Rect) -> Option<usize> {
        match frame {
            Frame::Page(page) if !self.page_rects[page].contains(reach) => Some(page),
            _ => None,
        }
    }

    /// The page whose group a line needs, and why: its reach leaving the
    /// page (`overflow`), or else its text when the line stands on the page
    /// with nothing around it (`text`: a body line, no cell or frame). The
    /// browser sets the text in its own fonts and the line does not wrap, so
    /// the stored width bounds nothing: without the correction script a
    /// line inside the paper by that width still reached past it (성과보고서
    /// `section6` `p[110]`: the space after its last word, 5.6px past the
    /// paper, shrank the whole print to 99.3%).
    fn line_group(&self, placed: &PlacedLine) -> Option<(usize, &'static str)> {
        if let Some(page) = self.group_for(placed.frame, placed.reach) {
            return Some((page, "overflow"));
        }
        match (placed.frame, placed.clip) {
            (Frame::Page(page), None) => Some((page, "text")),
            _ => None,
        }
    }

    /// Open a page-size group that clips its box's layout to the page. The
    /// group carries the page (origin, one-page view, print anchor) and the
    /// paint layer; the box inside is placed in page coordinates.
    fn open_group(&mut self, tag: &str, page: usize, z: Option<usize>, reason: &str) {
        *self.report.groups.entry(reason.to_owned()).or_default() += 1;
        let rect = self.page_rects[page];
        self.html.push_str(&format!(
            "<{tag} class=\"hpg pg{n}\" data-pg=\"{n}\" data-hwpx-group=\"{reason}\" style=\"width:{};height:{};margin-left:var(--ox);margin-top:var(--oy);",
            px(rect.w),
            px(rect.h),
            n = page + 1
        ));
        if let Some(z) = z {
            self.html.push_str(&format!("z-index:{z};"));
        }
        self.html.push_str("\">");
    }

    /// One line box, its runs and the table set inline in it.
    fn line(
        &mut self,
        placed: &PlacedLine<'a>,
        generated: bool,
        numbers: &Numbers,
    ) -> Result<(), Unsupported> {
        let line = &placed.line;
        let tag = if placed.block { "div" } else { "span" };
        let group = self.line_group(placed);
        if let Some((page, reason)) = group {
            self.open_group(tag, page, placed.z, reason);
        }
        let frame = if group.is_some() {
            Frame::Local
        } else {
            placed.frame
        };
        self.html.push('<');
        self.html.push_str(tag);
        self.html.push_str(" class=\"hls ps");
        self.html.push_str(&line.para_style_id.to_string());
        if let Frame::Page(page) = frame {
            self.html
                .push_str(&format!(" pg{}\" data-pg=\"{}", page + 1, page + 1));
        }
        self.html.push('"');
        match line.fill {
            Some(LineFill::Spaces) => self.html.push_str(" data-hwpx-fill=\"space\""),
            Some(LineFill::Letters) => self.html.push_str(" data-hwpx-fill=\"letter\""),
            None => {}
        }
        self.html.push_str(" style=\"");
        if line.padding_left > 0 {
            self.html
                .push_str(&format!("padding-left:{};", css_mm(line.padding_left)));
        }
        self.html.push_str(&format!(
            "line-height:{};white-space:nowrap;",
            css_mm_hundredths(placed.line_box.line_height)
        ));
        self.place_style(frame, placed.x, placed.y);
        self.html.push_str(&format!(
            "height:{};width:{};",
            css_mm(placed.line_box.height),
            css_mm(line.width)
        ));
        if let (Some(z), None) = (placed.z, group) {
            self.html.push_str(&format!("z-index:{z};"));
        }
        if let Some(clip) = placed.clip {
            self.clip_style(clip, placed.x, placed.y);
        }
        self.html.push_str("\">");
        // Always a span: a line is phrasing content of its paragraph.
        html::render_bullet(
            &mut self.html,
            line,
            self.layout,
            placed.line_box.height,
            "span",
        );
        if let Some(number) = &numbers.head {
            number_head(
                &mut self.html,
                line,
                self.layout,
                number,
                placed.line_box.height,
            );
        }
        let annotated = self.check_annotations(line, numbers)?;
        // The tokens, tables and objects of the line in the order of their
        // controls in the paragraph (`presentation::line_items`). In a `p` an
        // object's boxes become spans (the renderer's own rewrite).
        for item in presentation::line_items(line, &placed.inline_tables) {
            match item {
                LineItem::Token(index) => {
                    let token = &line.tokens[index];
                    let source = line.token_sources.get(index);
                    let number =
                        source.and_then(|source| numbers.by_token.get(&source.token_index));
                    let composed =
                        source.and_then(|source| numbers.composed.get(&source.token_index));
                    if let (Some(composed), true) = (composed, is_composed_control(token)) {
                        if generated {
                            // A drawn-only copy has no text of its own to fall back to.
                            self.unsupported(
                                crate::diagnostic::reasons::ANNOTATION_IN_A_DRAWN_ONLY_COPY,
                                &line.paragraph_key,
                            )?;
                        } else {
                            match self.composed(&line.paragraph_key, token, composed, annotated) {
                                Ok(()) => {}
                                // Nothing is written before a refusal: the
                                // base text stands without its annotation.
                                Err(why) if self.lenient => {
                                    self.unsupported(why.what, &why.key)?;
                                    self.composed_plain(token, composed);
                                }
                                Err(why) => return Err(why),
                            }
                        }
                    } else if let Some(number) = number {
                        self.html.push_str(&format!(
                            "<span class=\"hrt cs{}\" data-hwpx-generated>{}</span>",
                            token.char_style_id,
                            escape_html(number)
                        ));
                    } else if generated {
                        html::render_token_generated(&mut self.html, token, self.layout);
                    } else {
                        html::render_token(&mut self.html, token, self.layout);
                    }
                }
                LineItem::Table(index) => {
                    self.inline_table(placed.page, placed.inline_tables[index])?
                }
                LineItem::Object(index) => {
                    let object = &line.inline_objects[index];
                    if object.kind == "equation" {
                        match self.equation(object, !generated) {
                            Ok(()) => {}
                            // Nothing is written before a refusal.
                            Err(why) if self.lenient => self.unsupported(why.what, &why.key)?,
                            Err(why) => return Err(why),
                        }
                    } else {
                        self.object_markup(placed.page, object, true, tag == "span", !generated)?;
                    }
                }
            }
        }
        self.close(tag);
        if group.is_some() {
            self.close(tag);
        }
        if !generated {
            self.written += 1;
        }
        Ok(())
    }

    /// What a line with a 덧말 takes: the size of its letters and the band
    /// above them, from the source line (`baseline` is the band plus 85% of
    /// the letters' size). `None` for a line with no 덧말.
    fn annotation_metrics(&self, line: &LineFragment) -> Option<Annotated> {
        (line.tokens.iter().any(is_ruby_control)
            && !self
                .plain_annotations
                .borrow()
                .contains(&line.paragraph_key))
        .then(|| {
            let size = line
                .tokens
                .iter()
                .filter_map(|token| self.font_sizes.get(&token.char_style_id))
                .max()
                .copied()
                .unwrap_or(1000);
            Annotated {
                size,
                band: line.baseline - size * 85 / 100,
            }
        })
    }

    /// The box of a line, as `annotated_line_box` moves it when the line has
    /// a 덧말. A line holding a table or an object is refused: it would need
    /// the box of each, which no document shows.
    fn annotated_box(
        &self,
        line: &LineFragment,
        standard: LineBox,
        holds_tables: bool,
        use_metric_offset: bool,
    ) -> Result<LineBox, Unsupported> {
        let Some(annotated) = self.annotation_metrics(line) else {
            return Ok(standard);
        };
        if holds_tables || !line.inline_objects.is_empty() {
            self.plain_annotation(
                "an annotation in a line holding an object",
                &line.paragraph_key,
            )?;
            return Ok(standard);
        }
        if annotated.band < 0 {
            self.plain_annotation(
                "an annotated line without the baseline of its letters",
                &line.paragraph_key,
            )?;
            return Ok(standard);
        }
        Ok(presentation::annotated_line_box(
            line,
            annotated.size,
            annotated.band,
            crate::layout::line_height_of_size(annotated.size, use_metric_offset),
        ))
    }

    /// Check that a line with 덧말 is the line `annotated_box` is made for:
    /// every annotation is on letters of the line's size, and the band above
    /// the letters is the largest annotation set above (50% of the letters'
    /// size), with the source line as high as letters and band together.
    /// Anything else is a line whose layout the corpus does not show.
    fn check_annotations(
        &self,
        line: &LineFragment,
        numbers: &Numbers,
    ) -> Result<Option<Annotated>, Unsupported> {
        let Some(annotated) = self.annotation_metrics(line) else {
            return Ok(None);
        };
        let key = &line.paragraph_key;
        let mut above = 0;
        for (index, token) in line.tokens.iter().enumerate() {
            if !is_ruby_control(token) {
                continue;
            }
            let Some(Inline::Ruby { position, .. }) = line
                .token_sources
                .get(index)
                .and_then(|source| numbers.composed.get(&source.token_index))
            else {
                return self.plain_annotation("an annotation without its content", key);
            };
            let size = self
                .font_sizes
                .get(&token.char_style_id)
                .copied()
                .unwrap_or(1000);
            if size != annotated.size {
                return self
                    .plain_annotation("an annotation smaller than the letters of its line", key);
            }
            if position == "TOP" {
                above = above.max(annotation_size(size));
            }
        }
        if (annotated.band - above).abs() > 1
            || (line.text_height - annotated.size - above).abs() > 1
        {
            return self
                .plain_annotation("an annotated line not as high as its letters and band", key);
        }
        Ok(Some(annotated))
    }

    /// The text of a 덧말 or 겹친 글자 that could not be written as one (a
    /// lenient run): the base text, or the overlapped letters side by side.
    fn composed_plain(&mut self, token: &Token, inline: &Inline) {
        let text = match inline {
            Inline::Ruby { base, .. } => base,
            Inline::Compose { text, .. } => text,
            _ => return,
        };
        self.html.push_str(&format!(
            "<span class=\"hrt cs{}\">{}</span>",
            token.char_style_id,
            html::escape_html_text(text)
        ));
    }

    /// A 덧말 or 겹친 글자 in the place of its control.
    ///
    /// A 덧말 is a `ruby` the size of a line's top-aligned inline block: its
    /// top is the line's top, so the annotation (`rt`, out of the flow) is
    /// placed against the line's own box, not against the font's metrics.
    /// The annotation's em box touches the base text's: above, it ends where
    /// the base text's begins (a glyph gap below the line's top); below, it
    /// begins where the base text's ends. Its baseline is a fixed height
    /// into the box (an empty inline block, as the equation's).
    fn composed(
        &mut self,
        key: &str,
        token: &Token,
        inline: &Inline,
        annotated: Option<Annotated>,
    ) -> Result<(), Unsupported> {
        let size = self
            .font_sizes
            .get(&token.char_style_id)
            .copied()
            .unwrap_or(1000);
        match inline {
            Inline::Ruby {
                base,
                annotation,
                position,
                size_ratio,
                option,
                align,
                char_style_id,
                ..
            } => {
                // The corpus shows these values; any other is a meaning the
                // converter has no source for.
                if !matches!(position.as_str(), "TOP" | "BOTTOM") {
                    return refuse(
                        crate::diagnostic::reasons::ANNOTATION_POSITION_THE_CORPUS_DOES_NOT_SHOW,
                        key,
                    );
                }
                if *size_ratio != 0 || align != "CENTER" || !matches!(option, 0 | 4) {
                    return refuse(
                        crate::diagnostic::reasons::ANNOTATION_SIZE_ALIGNMENT_OR_OPTION_NOT_SHOWN,
                        key,
                    );
                }
                let (Some(style), Some(_)) = (char_style_id, annotated) else {
                    return refuse(
                        crate::diagnostic::reasons::ANNOTATION_WITHOUT_ITS_LETTERS_OR_ITS_LINE,
                        key,
                    );
                };
                let small = annotation_size(size);
                let gap = presentation::glyph_gap(size);
                let offset = if position == "TOP" {
                    gap - small
                } else {
                    size + gap
                };
                self.html.push_str(&format!(
                    "<ruby class=\"hdu\" data-hwpx-ruby=\"{position}\"><span class=\"hrt cs{}\">{}</span><rt class=\"hdS cs{style}\" style=\"top:{};font-size:{}pt;\"><span class=\"hdb\" style=\"height:{};\"></span>{}</rt></ruby>",
                    token.char_style_id,
                    html::escape_html_text(base),
                    css_mm(offset),
                    html::format_pt(small),
                    css_mm(small * 85 / 100),
                    html::escape_html_text(annotation)
                ));
                self.report.annotations.push(AnnotationRecord {
                    key: key.to_owned(),
                    kind: "ruby",
                    detail: position.clone(),
                    size: Some(small),
                    offset: Some(offset),
                });
                Ok(())
            }
            Inline::Compose {
                text,
                shape,
                char_size,
                compose_type,
                char_style_ids,
                ..
            } => {
                let glyph = match shape.as_str() {
                    "SHAPE_RECTANGLE" => '\u{25a1}',
                    "SHAPE_THIN_CIRCULATE_TRIANGLE" => '\u{267a}',
                    _ => {
                        return refuse(crate::diagnostic::reasons::OVERLAPPED_LETTERS_SHAPE_THE_CORPUS_DOES_NOT_SHOW, key)
                    }
                };
                // `charSz` -3 is 7pt on 10pt letters whether it counts points
                // off or percent of the size; only there are the two readings
                // one. Two letters fit side by side in the shape's width.
                if compose_type != "SPREAD"
                    || *char_size != -3
                    || size != 1000
                    || text.chars().count() != 2
                {
                    return refuse(
                        crate::diagnostic::reasons::OVERLAPPED_LETTERS_THE_CORPUS_DOES_NOT_SHOW,
                        key,
                    );
                }
                let base = self
                    .layout
                    .char_styles
                    .iter()
                    .find(|style| style.id == token.char_style_id);
                for id in char_style_ids.iter().flatten() {
                    let own = self.layout.char_styles.iter().find(|style| style.id == *id);
                    if !base
                        .zip(own)
                        .is_some_and(|(base, own)| same_letters(base, own))
                    {
                        return refuse(
                            crate::diagnostic::reasons::OVERLAPPED_LETTERS_IN_LETTERS_OF_THEIR_OWN,
                            key,
                        );
                    }
                }
                self.html.push_str(&format!(
                    "<span class=\"hco cs{}\" data-hwpx-compose=\"{shape}\" data-hwpx-shape=\"{glyph}\"><span class=\"hcc\" style=\"font-size:{}pt;\">{}</span></span>",
                    token.char_style_id,
                    html::format_pt(size * 7 / 10),
                    html::escape_html_text(text)
                ));
                self.report.annotations.push(AnnotationRecord {
                    key: key.to_owned(),
                    kind: "compose",
                    detail: shape.clone(),
                    size: None,
                    offset: None,
                });
                Ok(())
            }
            _ => refuse(crate::diagnostic::reasons::ANNOTATION_OF_NO_KNOWN_KIND, key),
        }
    }

    /// An object drawn by the renderer's own writers (set in a line when
    /// `inline`, else against its layer), with the tree's paragraphs in its
    /// text boxes. In a `p` its boxes become spans (the renderer's rewrite).
    fn object_markup(
        &mut self,
        page: usize,
        object: &PositionedObject,
        inline: bool,
        phrasing: bool,
        source: bool,
    ) -> Result<(), Unsupported> {
        let mut markup = String::new();
        self.context.direct_objects.set(true);
        self.context.direct_observe.set(source && self.observe);
        if inline {
            html::render_inline_object(
                &mut markup,
                object,
                &self.context,
                self.options,
                &mut self.patterns[page],
                &mut self.gradients,
            );
        } else {
            html::render_object(
                &mut markup,
                object,
                &self.context,
                self.options,
                &mut self.patterns[page],
                &mut self.gradients,
            );
        }
        self.context.direct_objects.set(false);
        self.context.direct_observe.set(false);
        let slots = self.context.object_slots.take();
        if phrasing {
            if !slots.is_empty() {
                self.unsupported(
                    crate::diagnostic::reasons::BLOCK_CONTENT_IN_A_PHRASING_OBJECT,
                    &object.key,
                )?;
                return Ok(());
            }
            super::semantic::to_phrasing(&mut markup, 0);
        }
        if !source && !slots.is_empty() {
            self.unsupported(
                crate::diagnostic::reasons::TEXT_BOX_IN_A_DRAWN_ONLY_COPY,
                &object.key,
            )?;
            return Ok(());
        }
        let reading_tag = if phrasing { "span" } else { "div" };
        if self.options.reading_view {
            self.html.push('<');
            self.html.push_str(reading_tag);
            self.html.push_str(&reading::object_attributes(object));
            self.html.push('>');
        }
        let mut cursor = 0;
        for (offset, content) in slots {
            self.html.push_str(&markup[cursor..offset]);
            match content {
                bundle::ObjectContent::Table(table) => self.text_box_table(&table)?,
                bundle::ObjectContent::Paragraphs(first) => self.text_box_content(&first)?,
            }
            cursor = offset;
        }
        self.html.push_str(&markup[cursor..]);
        if self.options.reading_view {
            self.close(reading_tag);
        }
        if source {
            self.objects_written.insert(object.key.clone());
        }
        Ok(())
    }

    /// An equation set in its line (user decision Q1): the box the layout
    /// reserved for it (`heq`, the same box as the page-by-page output's),
    /// holding the equation as MathML, or its script's letters when the
    /// script uses syntax the converter does not know. The content stands
    /// out of the box's flow, so the box keeps its size and its baseline and
    /// the line is laid out as before. The equation's baseline lies at its
    /// source `baseLine` (percent of its height from the top): an empty
    /// inline block as tall as that (and the box's height more, so a tall
    /// equation cannot push it) ends on the baseline of the content's line,
    /// which starts the box's height above the box.
    fn equation(&mut self, object: &PositionedObject, source: bool) -> Result<(), Unsupported> {
        if !source {
            return refuse(
                crate::diagnostic::reasons::EQUATION_IN_A_DRAWN_ONLY_COPY,
                &object.key,
            );
        }
        let Some(node) = self.object_nodes.get(object.key.as_str()).copied() else {
            return refuse(
                crate::diagnostic::reasons::EQUATION_NOT_IN_THE_TREE,
                &object.key,
            );
        };
        let (
            Kind::Object {
                equation: Some(script),
                ..
            },
            Some(style),
        ) = (&node.kind, object.equation.as_deref())
        else {
            return refuse(
                crate::diagnostic::reasons::EQUATION_WITHOUT_ITS_SCRIPT,
                &object.key,
            );
        };
        self.html.push_str("<span");
        if self.observe {
            self.html.push_str(" data-hwpx-id=\"");
            self.html.push_str(&escape_html_attribute(&node.key));
            self.html.push('"');
        }
        self.html.push_str(" class=\"heq\" style=\"");
        self.html.push_str(&html::equation_box_style(object));
        let height = object.box_units.height.max(0);
        let baseline = height.saturating_mul(style.base_line.clamp(0, 100)) / 100;
        self.html.push_str(&format!(
            "\"><span class=\"hmq\" style=\"top:{};font-size:{}pt;color:{};\"><span class=\"hmb\" style=\"height:{};\"></span>",
            css_mm(-height),
            html::format_pt(style.base_unit.max(0)),
            super::css::safe_color(&style.text_color),
            css_mm(baseline + height)
        ));
        let mathml = crate::semantic::equation::to_mathml(script);
        match &mathml {
            Some(mathml) => {
                self.html.push_str("<math data-hwpx-equation=\"mathml\">");
                self.html.push_str(mathml);
                self.html.push_str("</math>");
            }
            None => {
                self.html.push_str("<span data-hwpx-equation=\"script\">");
                self.html.push_str(&escape_html(script));
                self.html.push_str("</span>");
            }
        }
        self.html.push_str("</span></span>");
        self.objects_written.insert(object.key.clone());
        self.report.equations.push(EquationRecord {
            key: object.key.clone(),
            mathml: mathml.is_some(),
        });
        Ok(())
    }

    /// A text box's table where the object's drawing puts it: its drawing
    /// and the table, placed against the box the drawing writes it in.
    fn text_box_table(&mut self, id: &str) -> Result<(), Unsupported> {
        let Some(node) = self.table_nodes.get(id).copied() else {
            self.unsupported(
                crate::diagnostic::reasons::TEXT_BOX_TABLE_NOT_IN_THE_TREE,
                id,
            )?;
            return Ok(());
        };
        self.deferred_tables.remove(id);
        for drawing in self.drawings.remove(id).unwrap_or_default() {
            self.drawing(&drawing);
        }
        self.table_structure(node)
    }

    /// The tree's content of the text box whose first paragraph is `first`:
    /// its paragraphs (and lists), not its captions or drawn children.
    fn text_box_content(&mut self, first: &str) -> Result<(), Unsupported> {
        let Some(owner) = self.text_owners.get(first).copied() else {
            self.unsupported(crate::diagnostic::reasons::TEXT_BOX_NOT_IN_THE_TREE, first)?;
            return Ok(());
        };
        for child in &owner.children {
            if !matches!(child.kind, Kind::Object { .. } | Kind::Caption { .. }) {
                self.node(child)?;
            }
        }
        Ok(())
    }

    /// A page-level floating object: a page-size group (the page's clip, and
    /// its paint layer) holding the renderer's objects layer and drawing.
    fn floating_object(&mut self, node: &'a Node) -> Result<(), Unsupported> {
        if self.objects_written.contains(node.key.as_str()) {
            return Ok(());
        }
        let Some(pieces) = self.floats.remove(node.key.as_str()) else {
            self.unsupported(
                crate::diagnostic::reasons::OBJECT_THE_LAYOUT_DID_NOT_PLACE,
                &node.key,
            )?;
            return Ok(());
        };
        for piece in pieces {
            // On a page the group gives the page's clip and paint layer; in
            // an inline table the layer itself carries them.
            let grouped = matches!(piece.frame, Frame::Page(_));
            if grouped {
                self.open_group("div", piece.page, piece.z, "object");
            }
            self.html.push_str(&format!(
                "<div class=\"hpo\" style=\"left:{};top:{};",
                px(piece.x),
                px(piece.y)
            ));
            if let (Some(z), false) = (piece.z, grouped) {
                self.html.push_str(&format!("z-index:{z};"));
            }
            if let Some(clip) = piece.clip {
                self.clip_style(clip, piece.x, piece.y);
            }
            self.html.push_str("\">");
            self.object_markup(piece.page, &piece.object, false, false, true)?;
            self.html.push_str("</div>");
            if grouped {
                self.html.push_str("</div>");
            }
        }
        Ok(())
    }

    /// Where a box goes in its frame.
    fn place_style(&mut self, frame: Frame, x: i64, y: i64) {
        match frame {
            Frame::Page(_) => self.html.push_str(&format!(
                "margin-left:calc(var(--ox) + {});margin-top:calc(var(--oy) + {});",
                px(x),
                px(y)
            )),
            Frame::Local => self
                .html
                .push_str(&format!("left:{};top:{};", px(x), px(y))),
        }
    }

    /// `clip` (in the frame) as the box at `x`/`y` clips itself.
    fn clip_style(&mut self, clip: Rect, x: i64, y: i64) {
        self.html.push_str(&format!(
            "clip-path:rect({} {} {} {});",
            px(clip.y - y),
            px(clip.x + clip.w - x),
            px(clip.y + clip.h - y),
            px(clip.x - x)
        ));
    }

    /// A table set in its line: an inline block holding its drawing and its
    /// cells, which are placed against it.
    fn inline_table(&mut self, page: usize, table: &'a Table) -> Result<(), Unsupported> {
        let Some(node) = self.table_nodes.get(table.id.as_str()).copied() else {
            self.unsupported(
                crate::diagnostic::reasons::INLINE_TABLE_NOT_IN_THE_TREE,
                &table.id,
            )?;
            return Ok(());
        };
        // A table sharing its box with its caption: the box is the whole
        // area (`htG`, as the reference names it), the drawing at the
        // table's corner in it.
        let plan = self
            .captions
            .get(table.id.as_str())
            .map(|placed| placed.plan);
        let (class, width, height) = match plan {
            Some(plan) => ("htG", plan.width, plan.height),
            None => {
                let frame = presentation::table_box(table, TablePlacement::Inline);
                ("htb", frame.width, frame.height)
            }
        };
        self.html.push_str(&format!(
            "<div class=\"{class}\" style=\"width:{};height:{};display:inline-block;position:relative;vertical-align:-15%;line-height:{};\">",
            css_mm(width),
            css_mm(height),
            css_mm(height)
        ));
        if html::table_needs_svg(table) {
            let (svg, left, top) = match plan {
                Some(plan) => {
                    let svg = captions::table_svg(table);
                    let left = px(snap(plan.table_left) + snap(svg.left));
                    let top = px(snap(plan.table_top) + snap(svg.top));
                    (svg, left, top)
                }
                None => {
                    let svg = presentation::table_svg_frame(table, TablePlacement::Inline);
                    (svg, css_mm(svg.left), css_mm(svg.top))
                }
            };
            self.open_svg(&svg, None);
            self.html.push_str(&format!(
                "left:{left};top:{top};width:{};height:{};\">",
                css_mm(svg.width),
                css_mm(svg.height)
            ));
            html::render_table_svg_body(
                &mut self.html,
                table,
                &mut self.patterns[page],
                &mut self.gradients,
            );
            self.html.push_str("</svg>");
        }
        self.table_structure(node)?;
        self.html.push_str("</div>");
        Ok(())
    }

    /// `<svg class="hs …" aria-hidden viewBox=… style="` for a drawing,
    /// left open for its placement.
    fn open_svg(&mut self, svg: &presentation::Frame, page: Option<usize>) {
        self.html.push_str("<svg class=\"hs");
        if let Some(page) = page {
            self.html
                .push_str(&format!(" pg{}\" data-pg=\"{}", page + 1, page + 1));
        }
        self.html.push_str(&format!(
            "\" aria-hidden=\"true\" viewBox=\"{} {} {} {}\" style=\"",
            mm_text(svg.left),
            mm_text(svg.top),
            mm_text(svg.width),
            mm_text(svg.height)
        ));
    }

    /// A floating table: its drawings on each page, the copies of its
    /// repeated header rows, then the table.
    fn floating_table(&mut self, node: &'a Node) -> Result<(), Unsupported> {
        if self.tables_written.contains(node.key.as_str()) {
            return Ok(());
        }
        for drawing in self.drawings.remove(node.key.as_str()).unwrap_or_default() {
            self.drawing(&drawing);
        }
        if let Some(copies) = self.repeats.remove(node.key.as_str()) {
            // Drawn only: the header row is in the table once (D35).
            self.html
                .push_str("<div data-hwpx-repeat aria-hidden=\"true\">");
            for placed in &copies {
                self.line(placed, true, &Numbers::default())?;
            }
            self.html.push_str("</div>");
        }
        self.table_structure(node)
    }

    fn drawing(&mut self, drawing: &Drawing<'a>) {
        if !html::table_needs_svg(drawing.table) {
            return;
        }
        let svg = presentation::table_svg_frame(drawing.table, drawing.placement);
        let x = drawing.x + snap(svg.left);
        let y = drawing.y + snap(svg.top);
        let group = self.group_for(drawing.frame, drawing.reach);
        if let Some(page) = group {
            self.open_group("div", page, drawing.z, "overflow");
        }
        let (frame, page) = match (group, drawing.frame) {
            (None, Frame::Page(page)) => (drawing.frame, Some(page)),
            _ => (Frame::Local, None),
        };
        self.open_svg(&svg, page);
        self.place_style(frame, x, y);
        self.html.push_str(&format!(
            "width:{};height:{};",
            css_mm(svg.width),
            css_mm(svg.height)
        ));
        if let (Some(z), None) = (drawing.z, group) {
            self.html.push_str(&format!("z-index:{z};"));
        }
        if let Some(clip) = drawing.clip {
            self.clip_style(clip, x, y);
        }
        self.html.push_str("\">");
        html::render_table_svg_body(
            &mut self.html,
            drawing.table,
            &mut self.patterns[drawing.page],
            &mut self.gradients,
        );
        self.html.push_str("</svg>");
        if group.is_some() {
            self.html.push_str("</div>");
        }
    }

    /// The table itself: one `table` (or, for a table without a grid worth
    /// reading, a box of cells, D33) over all its pages, its rows in source
    /// order and each cell once.
    fn table_structure(&mut self, node: &'a Node) -> Result<(), Unsupported> {
        let key = node.key.as_str();
        let Some(source) = self.sources.get(key).copied() else {
            self.unsupported(crate::diagnostic::reasons::TABLE_NOT_IN_THE_SOURCE, key)?;
            return Ok(());
        };
        let pieces = self.fragments.get(key).copied().unwrap_or(0);
        if pieces == 0 {
            self.unsupported(
                crate::diagnostic::reasons::TABLE_THE_LAYOUT_DID_NOT_PLACE,
                key,
            )?;
            return Ok(());
        }
        self.tables_written.insert(key.to_owned());
        let mut cells = HashMap::new();
        let mut caption = None;
        let placed_caption = self.captions.get(key).copied();
        for child in &node.children {
            match &child.kind {
                Kind::Cell { .. } => {
                    cells.insert(child.key.as_str(), child);
                }
                // Written only where its lines were placed with the table.
                Kind::Caption { .. } if has_text(child) && placed_caption.is_some() => {
                    caption = Some(child);
                }
                Kind::Caption { .. } if has_text(child) => {
                    if !self
                        .skipped
                        .borrow()
                        .iter()
                        .any(|part| part.key == child.key)
                    {
                        self.unsupported(crate::diagnostic::reasons::CAPTION_WITH_TEXT, &child.key)?
                    }
                }
                Kind::Caption { .. } => {}
                _ => self.unsupported(crate::diagnostic::reasons::TABLE_CHILD, &child.key)?,
            }
        }
        let single = source.rows == 1 && source.columns == 1;
        let boxed = single || (pieces == 1 && !source.cells.iter().any(html::cell_has_content));
        let rows = plan_rows(source);
        if self.options.reading_view {
            self.html.push_str("<div class=\"hwpx-read-scroll\">");
        }
        if boxed {
            // A box of cells (D33) has no place for a caption; no sample
            // shows which structure should hold one.
            if let Some(caption) = caption {
                self.unsupported(
                    crate::diagnostic::reasons::CAPTION_ON_A_TABLE_WITHOUT_A_GRID,
                    &caption.key,
                )?;
            }
            self.open("div", node);
            self.html.push_str(&format!(
                " data-hwpx-table=\"{}x{}\">",
                source.rows, source.columns
            ));
        } else {
            self.open("table", node);
            self.html.push('>');
            if let (Some(caption), Some(placed)) = (caption, placed_caption) {
                self.caption(caption, placed)?;
            }
        }
        let groups = [("thead", &rows.head), ("tbody", &rows.body)];
        for (group, rows) in groups {
            if rows.is_empty() && (boxed || group == "thead") {
                continue;
            }
            if !boxed {
                self.html.push_str(&format!("<{group}>"));
            }
            for row in rows.iter() {
                if !boxed {
                    self.html.push_str("<tr>");
                }
                for slot in &row.slots {
                    let Slot::Cell { cell, row_span } = slot else {
                        if !boxed {
                            self.html.push_str("<td hidden></td>");
                        }
                        continue;
                    };
                    let Some(cell_node) = cells.get(cell.id.as_str()).copied() else {
                        self.unsupported(
                            crate::diagnostic::reasons::CELL_NOT_IN_THE_TREE,
                            &cell.id,
                        )?;
                        self.html.push_str(if boxed {
                            "<div></div>"
                        } else if cell.is_header {
                            "<th></th>"
                        } else {
                            "<td></td>"
                        });
                        continue;
                    };
                    let tag = if boxed {
                        "div"
                    } else if cell.is_header {
                        "th"
                    } else {
                        "td"
                    };
                    self.open(tag, cell_node);
                    if self.options.reading_view {
                        self.html.push_str(&reading::cell_attributes(cell));
                    }
                    if boxed {
                        self.html.push_str(&format!(
                            " data-hwpx-cell=\"{},{},{},{}\"",
                            cell.row, cell.column, row_span, cell.col_span
                        ));
                        if cell.is_header {
                            self.html.push_str(" data-hwpx-header");
                        }
                    } else {
                        if *row_span > 1 {
                            self.html.push_str(&format!(" rowspan=\"{row_span}\""));
                        }
                        if cell.col_span > 1 {
                            self.html
                                .push_str(&format!(" colspan=\"{}\"", cell.col_span));
                        }
                    }
                    self.html.push('>');
                    for child in &cell_node.children {
                        self.node(child)?;
                    }
                    self.close(tag);
                }
                if !boxed {
                    self.html.push_str("</tr>");
                }
            }
            if !boxed {
                self.html.push_str(&format!("</{group}>"));
            }
        }
        self.close(if boxed { "div" } else { "table" });
        if self.options.reading_view {
            self.html.push_str("</div>");
        }
        Ok(())
    }

    /// A table's caption: the table's first child, holding its frame (`hcp`,
    /// the box that clips its lines, placed in the area it shares with the
    /// table) and in it its paragraphs. The frame carries the page (and
    /// paint layer) of a floating table; its lines are placed against it.
    fn caption(&mut self, node: &'a Node, placed: PlacedCaption) -> Result<(), Unsupported> {
        self.open("caption", node);
        self.html.push('>');
        let group = self.group_for(placed.frame, placed.reach);
        if let Some(page) = group {
            self.open_group("div", page, placed.z, "overflow");
        }
        let frame = if group.is_some() {
            Frame::Local
        } else {
            placed.frame
        };
        self.html.push_str("<div class=\"hcp");
        if let Frame::Page(page) = frame {
            self.html
                .push_str(&format!(" pg{}\" data-pg=\"{}", page + 1, page + 1));
        }
        self.html.push_str("\" style=\"");
        self.place_style(frame, placed.rect.x, placed.rect.y);
        self.html.push_str(&format!(
            "width:{};height:{};",
            px(placed.rect.w),
            px(placed.rect.h)
        ));
        if let (Some(z), None) = (placed.z, group) {
            self.html.push_str(&format!("z-index:{z};"));
        }
        self.html.push_str("\">");
        for child in &node.children {
            match child.kind {
                Kind::Paragraph { .. } | Kind::Heading { .. } | Kind::List { .. } => {
                    self.node(child)?
                }
                _ => self.unsupported(crate::diagnostic::reasons::CAPTION_CHILD, &child.key)?,
            }
        }
        self.html.push_str("</div>");
        if group.is_some() {
            self.html.push_str("</div>");
        }
        self.close("caption");
        Ok(())
    }
}

/// Whether an object (or a child it draws) has a text box, whose paragraphs
/// the tree writes: a line carrying it holds block content.
fn has_text_box(object: &PositionedObject) -> bool {
    object
        .shape
        .as_ref()
        .is_some_and(|shape| !shape.paragraphs.is_empty() || !shape.tables.is_empty())
        || object.children.iter().any(has_text_box)
}

/// How far the objects set in a line reach from the line's corner (they
/// follow its text, and a tall one lowers the line box), joined to `inner`.
fn inline_objects_reach(line: &LineFragment, inner: Option<Inner>) -> Option<Inner> {
    let mut reach = inner;
    for object in &line.inline_objects {
        let own = Inner::plain(Rect {
            x: 0,
            y: 0,
            w: snap(object.box_units.width.max(1)),
            h: snap(object.box_units.height.max(1)),
        });
        reach = Some(reach.map_or(own, |reach| reach.union(own)));
    }
    reach
}

/// The key of a table's caption in the tree.
fn caption_key(table: &Table) -> String {
    format!("{}/caption", table.id)
}

/// Where a page-level table's caption with text goes, if it has one, or
/// what is not placed yet: besides the plan's own limits, a table split
/// across pages (whose caption no fragment may repeat).
fn caption_plan(table: &Table) -> Result<Option<TableCaptionPlan>, Unsupported> {
    let plan = captions::table_caption_plan(table).map_err(|what| Unsupported {
        what,
        key: caption_key(table),
    })?;
    if plan.is_some() && table.fragment_rows.is_some() {
        return refuse(
            crate::diagnostic::reasons::CAPTION_ON_A_TABLE_SPLIT_ACROSS_PAGES,
            &caption_key(table),
        );
    }
    Ok(plan)
}

/// Where a table's drawing reaches, its box at `origin`.
fn svg_reach(table: &Table, placement: TablePlacement, origin: (i64, i64)) -> Rect {
    let svg = presentation::table_svg_frame(table, placement);
    Rect {
        x: origin.0 + snap(svg.left),
        y: origin.1 + snap(svg.top),
        w: snap(svg.width),
        h: snap(svg.height),
    }
}

/// Whether a node holds any text of its own or in its descendants.
fn has_text(node: &Node) -> bool {
    let own = match &node.kind {
        Kind::Paragraph { content, .. } | Kind::Heading { content, .. } => {
            content.iter().any(|span| match &span.value {
                Inline::Text { text, .. } => !text.trim().is_empty(),
                Inline::Generated { .. }
                | Inline::UnformattedNumber { .. }
                | Inline::Ruby { .. }
                | Inline::Compose { .. } => true,
                _ => false,
            })
        }
        _ => false,
    };
    own || node.children.iter().any(has_text)
}

/// The source tables by id: the sections' tables, their cells' tables and
/// the tables of text boxes, at any depth.
fn source_tables(document: &Document) -> HashMap<&str, &Table> {
    fn add_table<'d>(table: &'d Table, out: &mut HashMap<&'d str, &'d Table>) {
        out.insert(table.id.as_str(), table);
        for cell in &table.cells {
            for nested in &cell.tables {
                add_table(nested, out);
            }
        }
    }
    fn add_object<'d>(object: &'d PositionedObject, out: &mut HashMap<&'d str, &'d Table>) {
        if let Some(shape) = &object.shape {
            for nested in &shape.tables {
                add_table(nested, out);
            }
        }
        for child in &object.children {
            add_object(child, out);
        }
    }
    let mut out = HashMap::new();
    for section in &document.sections {
        for block in &section.blocks {
            match block {
                Block::Table(source) => add_table(source, &mut out),
                Block::Object(source) => add_object(source, &mut out),
                Block::Paragraph(paragraph) => {
                    for source in &paragraph.objects {
                        add_object(source, &mut out);
                    }
                }
            }
        }
    }
    out
}

/// The tree's objects with a text box, by the text box's first paragraph.
fn text_owners<'n>(node: &'n Node, out: &mut HashMap<&'n str, &'n Node>) {
    fn first_paragraph(node: &Node) -> Option<&str> {
        match node.kind {
            Kind::Paragraph { .. } | Kind::Heading { .. } => Some(&node.key),
            Kind::Object { .. } | Kind::Caption { .. } | Kind::Table { .. } => None,
            _ => node.children.iter().find_map(first_paragraph),
        }
    }
    if let Kind::Object { .. } = node.kind {
        if let Some(first) = node.children.iter().find_map(first_paragraph) {
            out.insert(first, node);
        }
    }
    for child in &node.children {
        text_owners(child, out);
    }
}

/// The tree's objects by key.
fn object_nodes<'n>(node: &'n Node, out: &mut HashMap<&'n str, &'n Node>) {
    if let Kind::Object { .. } = node.kind {
        out.insert(node.key.as_str(), node);
    }
    for child in &node.children {
        object_nodes(child, out);
    }
}

/// The tree's tables by key.
fn table_nodes<'n>(node: &'n Node, out: &mut HashMap<&'n str, &'n Node>) {
    if let Kind::Table { .. } = node.kind {
        out.insert(node.key.as_str(), node);
    }
    for child in &node.children {
        table_nodes(child, out);
    }
}

/// An equation's content stands out of its box's flow (`Emitter::equation`):
/// a line of no height starting the box's height above the box, holding the
/// empty block that ends on its baseline and the equation.
const EQUATION_CSS: &str = ".hmq {position:absolute;left:0;margin:0;padding:0;line-height:0;white-space:nowrap;}\n.hmb {display:inline-block;width:0;}\n";

/// A 덧말: the `ruby` is an inline block aligned to the top of its line, so
/// its annotation is placed against the line's own box (`Emitter::composed`);
/// the annotation is a block out of the flow, across the base text, whose
/// first line is an empty inline block of the height of its baseline.
const RUBY_CSS: &str = ".hdu {display:inline-block;vertical-align:top;}\n.hdu > rt {display:block;left:0;right:0;text-align:center;line-height:0;white-space:nowrap;}\n.hdb {display:inline-block;width:0;}\n";

/// A 겹친 글자: the box is as wide as its shape's glyph (generated, so it is
/// no DOM text); its letters stand out of the flow, side by side, at the
/// box's centre.
const COMPOSE_CSS: &str = ".hco::before {content:attr(data-hwpx-shape);}\n.hco > .hcc {left:50%;top:50%;transform:translate(-50%,-50%);line-height:1;white-space:nowrap;}\n";

/// The structure of a table sharing its inline box with its caption lays
/// out nothing either: it stands at the box's corner, like `.htb > table`.
/// A caption's frame is a placed box that clips its lines.
const CAPTION_CSS: &str = ".htG > table {position:absolute;left:0;top:0;}\n.hcp {position:absolute;margin:0;padding:0;border:0;overflow:hidden;}\n";

/// One paper as `hp210x297`, the name of the `@page` that sizes it.
fn paper_name(size: &(String, String)) -> String {
    format!("hp{}x{}", size.0, size.1).replace('.', "_")
}

/// The rules of the pages: each page's origin, the height that keeps the
/// document as tall as its pages, the one-page view and the printed sheets.
fn page_css(frames: &[PageFrame], height: i64) -> String {
    let mut css = String::new();
    for (index, frame) in frames.iter().enumerate() {
        let number = index + 1;
        let (high, low) = split(frame.origin.1);
        let (paper_high, paper_low) = split(frame.paper.1);
        css.push_str(&format!(
            ".pg{number} {{left:0;top:{high};--ox:{};--oy:{low};--pw:{};}}\n",
            px(frame.origin.0),
            px(frame.size.0)
        ));
        css.push_str(&format!(
            ".hpa[data-page=\"{number}\"] {{position:absolute;margin:0;left:{};top:{paper_high};margin-top:{paper_low};}}\n",
            px(frame.paper.0)
        ));
    }
    // The body's own top padding is the flow's first 2mm.
    css.push_str(&format!("main {{height:{};}}\n", px(height - snap_mm(2.0))));
    // The table structure lays out nothing: its lines are placed boxes. An
    // inline table's grid is taken out of its box's flow, so the box keeps
    // its bottom edge as its baseline as before. An overflow group clips
    // its box to the page. Only the lines and pictures take the pointer.
    css.push_str("main td, main th {padding:0;}\n.htb > table, .htb > [data-hwpx-table] {position:absolute;left:0;top:0;}\n.hpg {position:absolute;margin:0;padding:0;overflow:hidden;}\n.hpo {position:absolute;margin:0;padding:0;}\nmain {pointer-events:none;}\nmain .hls, main img {pointer-events:auto;}\n");
    // The one-page view: everything on a page hides but the current page's
    // paper and boxes, which the head script names in one more rule and the
    // stylesheet centres at the top of the body (the bar is laid over it and
    // takes no room). The origin is the paper's content edge (the body's 2mm
    // and the paper's 1px border) in whole layout units, as the flow puts the
    // paper.
    css.push_str(&format!(
        "@media screen {{\n\
.hwpx-paged main {{height:0 !important;}}\n\
.hwpx-paged [data-pg], .hwpx-paged .hpa[data-page] {{display:none;}}\n\
.hwpx-paged [data-pg] {{top:0 !important;--ox:calc(50% - var(--pw) / 2);--oy:{};}}\n\
.hwpx-paged:not(.hwpx-ready) [data-pg=\"1\"], .hwpx-paged:not(.hwpx-ready) .hpa[data-page=\"1\"] {{display:block;}}\n\
.hwpx-paged .hpa[data-page] {{position:relative !important;left:auto !important;top:auto !important;margin:0 auto 2mm !important;}}\n\
}}\n",
        px(snap_mm(2.0) + UNIT)
    ));
    // Printing: every paper is a sheet of its own size in the flow, ending
    // in a page break, and a page's boxes are pinned to their sheet. Nothing
    // is stacked from a common origin, so nothing drifts with the sheet
    // pitch the browser really uses, and pages of different sizes print on
    // their own paper.
    css.push_str("@page {margin:0;}\n");
    let mut papers = frames
        .iter()
        .map(|frame| frame.size_mm.clone())
        .collect::<Vec<_>>();
    papers.sort();
    papers.dedup();
    for paper in &papers {
        css.push_str(&format!(
            "@page {} {{size:{}mm {}mm;margin:0;}}\n",
            paper_name(paper),
            paper.0,
            paper.1
        ));
    }
    css.push_str("@media print {\n");
    css.push_str(".hpa[data-page] {position:relative !important;left:auto !important;top:auto !important;margin:0 !important;border:0 !important;box-shadow:none !important;break-after:page;}\n");
    for (index, frame) in frames.iter().enumerate() {
        let number = index + 1;
        css.push_str(&format!(
            ".hpa[data-page=\"{number}\"] {{anchor-name:--hp{number};page:{};}}\n.pg{number} {{position-anchor:--hp{number};top:anchor(top) !important;left:anchor(left) !important;--ox:0px;--oy:0px;}}\n",
            paper_name(&frame.size_mm)
        ));
    }
    if let Some(last) = frames.last() {
        // A change of page name forces a break: the main after the last
        // sheet must not open a blank one.
        css.push_str(&format!(
            ".hpa[data-page=\"{}\"] {{break-after:auto;}}\nmain {{page:{};height:0 !important;}}\n",
            frames.len(),
            paper_name(&last.size_mm)
        ));
    }
    // The pages carry their own copies of repeated header rows. A print
    // still makes (blank) sheets for what an anchored overflow group clips
    // unless its layout is contained (Chromium 151, 다중 문단 셀 콘텐츠 높이:
    // 7 sheets for 5); on screen containment would snap the group's paint
    // to whole pixels, so it applies to print only.
    css.push_str("main thead {display:table-row-group;}\n.hpg {contain:size layout paint;}\n}\n");
    css
}

/// Write the document from its semantic tree and the layout's placement, or
/// say what this emitter does not write yet.
pub fn render_direct(
    document: &Document,
    layout: &LayoutDocument,
    options: &RenderOptions,
    resource_directory: &str,
    observe: bool,
) -> Result<RenderBundle, Unsupported> {
    render_direct_reported(document, layout, options, resource_directory, observe)
        .map(|(bundle, _)| bundle)
}

/// [`render_direct`], and a report of which branches it took.
pub fn render_direct_reported(
    document: &Document,
    layout: &LayoutDocument,
    options: &RenderOptions,
    resource_directory: &str,
    observe: bool,
) -> Result<(RenderBundle, DirectReport), Unsupported> {
    render_direct_with(
        document,
        layout,
        options,
        resource_directory,
        observe,
        false,
    )
}

/// [`render_direct_reported`] for a document with parts the emitter cannot
/// write: each such part (an object it cannot draw, a caption it cannot
/// place, an annotation outside what the corpus shows, ...) is left out, or
/// written more plainly, and the rest of the document is written. What was
/// left out is in `DirectReport::skipped`. A strict run refuses the document
/// at the first such part instead.
pub fn render_direct_lenient(
    document: &Document,
    layout: &LayoutDocument,
    options: &RenderOptions,
    resource_directory: &str,
    observe: bool,
) -> Result<(RenderBundle, DirectReport), Unsupported> {
    render_direct_with(document, layout, options, resource_directory, observe, true)
}

fn render_direct_with(
    document: &Document,
    layout: &LayoutDocument,
    options: &RenderOptions,
    resource_directory: &str,
    observe: bool,
    lenient: bool,
) -> Result<(RenderBundle, DirectReport), Unsupported> {
    // A lenient run leaves out an object the renderer cannot draw: its
    // placeholder is a bare text label (nothing styles it), not a stand-in.
    let leaving_out;
    let without_reading;
    let options = if !options.logical_dom && options.reading_view {
        without_reading = RenderOptions {
            reading_view: false,
            ..options.clone()
        };
        &without_reading
    } else {
        options
    };
    let options = if lenient {
        leaving_out = RenderOptions {
            unsupported: UnsupportedPolicy::Skip,
            ..options.clone()
        };
        &leaving_out
    } else {
        options
    };
    let tree = crate::semantic::build(document, options.infer_structure);
    let (frames, height) = page_frames(layout);
    let mut nodes = HashMap::new();
    table_nodes(&tree, &mut nodes);
    let mut owners = HashMap::new();
    text_owners(&tree, &mut owners);
    let mut objects = HashMap::new();
    object_nodes(&tree, &mut objects);
    let prefix = bundle::url_component(resource_directory);
    let (resources, asset_urls) = bundle::picture_resources(layout, &prefix);
    let context = RenderContext::new(layout, asset_urls);
    let laid_out = presentation::objects::collect_text_box_tables(layout, |reference| {
        html::document_asset(layout, reference)
            .and_then(|asset| context.asset_url(asset))
            .is_some()
    });
    let mut emitter = Emitter {
        layout,
        page_rects: frames
            .iter()
            .map(|frame| Rect {
                x: 0,
                y: 0,
                w: frame.size.0,
                h: frame.size.1,
            })
            .collect(),
        font_sizes: layout
            .char_styles
            .iter()
            .map(|style| (style.id, style.font_size_hwp))
            .collect(),
        sources: source_tables(document),
        table_nodes: nodes,
        lines: HashMap::new(),
        repeats: HashMap::new(),
        drawings: HashMap::new(),
        fragments: HashMap::new(),
        floats: HashMap::new(),
        text_owners: owners,
        object_nodes: objects,
        text_box_tables: laid_out
            .iter()
            .map(|table| (table.id.as_str(), table))
            .collect(),
        deferred_tables: HashSet::new(),
        captions: HashMap::new(),
        patterns: layout
            .pages
            .iter()
            .map(|page| PatternIds::new(page.index))
            .collect(),
        gradients: GradientIds::new(),
        context,
        options,
        html: String::new(),
        placed: 0,
        written: 0,
        tables_written: HashSet::new(),
        objects_written: HashSet::new(),
        observe,
        report: DirectReport::default(),
        lenient,
        skipped: std::cell::RefCell::new(Vec::new()),
        plain_annotations: std::cell::RefCell::new(HashSet::new()),
    };
    emitter.place()?;
    emitter.node(&tree)?;
    // Everything placed is written exactly once. A lenient run reports what
    // is left over instead of refusing the document.
    if let Some(key) = emitter.lines.keys().min() {
        emitter.unsupported(
            crate::diagnostic::reasons::LINE_OF_NO_PARAGRAPH_IN_THE_TREE,
            key,
        )?;
    }
    if let Some(key) = emitter.repeats.keys().chain(emitter.drawings.keys()).min() {
        emitter.unsupported(
            crate::diagnostic::reasons::TABLE_DRAWING_OF_NO_TABLE_IN_THE_TREE,
            key,
        )?;
    }
    if emitter.written != emitter.placed && !lenient {
        return refuse(crate::diagnostic::reasons::LINE_WRITTEN_TWICE, "");
    }
    if let Some(key) = emitter
        .table_nodes
        .keys()
        .filter(|key| !emitter.tables_written.contains(**key))
        .min()
    {
        emitter.unsupported(
            crate::diagnostic::reasons::TABLE_OF_THE_TREE_NOT_WRITTEN,
            key,
        )?;
    }
    emitter.report.skipped = emitter.skipped.take();
    let scripts = scripts(options);
    let csp = html::csp_meta(&scripts, true);
    let mut out = String::with_capacity(emitter.html.len() + 4096);
    out.push_str("<!DOCTYPE html>\n<html lang=\"ko\"><head><meta charset=\"utf-8\"><title>");
    out.push_str(&escape_html(
        options.source_name.as_deref().unwrap_or(&layout.title),
    ));
    out.push_str("</title><meta name=\"generator\" content=\"hwpx2html semantic\">");
    if !layout.title.is_empty() {
        out.push_str("<meta name=\"hwpx-title\" content=\"");
        out.push_str(&escape_html_attribute(&layout.title));
        out.push_str("\">");
    }
    out.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    out.push_str(&escape_html_attribute(&csp));
    out.push_str("\">");
    out.push_str(bundle::PENDING_LINK);
    if options.reading_view {
        let width = layout.pages.first().map_or(210.0, |page| {
            hwp_to_centi_mm(page.spec.width) as f64 / 100.0
        });
        out.push_str(&format!(
            "<meta name=\"hwpx-paper-width\" content=\"{:.4}\"><script>",
            width * 96.0 / 25.4
        ));
        out.push_str(reading::READING_SCRIPT);
        out.push_str("</script>");
    }
    if options.page_navigation {
        out.push_str("<script>");
        out.push_str(if options.reading_view {
            reading::NAVIGATION_SCRIPT
        } else {
            NAVIGATION_SCRIPT
        });
        out.push_str("</script>");
    }
    out.push_str("</head><body>");
    for (index, page) in layout.pages.iter().enumerate() {
        out.push_str(&format!(
            "<div class=\"hpa\" data-page=\"{}\" style=\"width:{};height:{};\">",
            index + 1,
            css_mm(page.spec.width),
            css_mm(page.spec.height)
        ));
        if let Some(number) = &page.page_number {
            html::render_page_number(&mut out, page, layout, number);
        }
        out.push_str("</div>");
    }
    out.push_str(if options.reading_view {
        "<main class=\"hwpx-doc\">"
    } else {
        "<main>"
    });
    out.push_str(&emitter.html);
    out.push_str("</main>");
    if options.adjust_letter_spacing {
        out.push_str("<script>");
        out.push_str(if options.reading_view {
            reading::CORRECTION_SCRIPT
        } else {
            html::SCRIPT_SOURCE
        });
        out.push_str("</script>");
    }
    out.push_str("</body></html>");
    let (out, style_rules) = html::styles_to_classes(&out);
    let annotated = |kind| {
        emitter
            .report
            .annotations
            .iter()
            .any(|record| record.kind == kind)
    };
    let mut css = format!(
        "{}{}{}{}{}{}{}{}{}{}",
        super::css::base_css(),
        super::css::dynamic_css(&layout.char_styles, &layout.para_styles),
        super::css::semantic_css(),
        page_css(&frames, height),
        if emitter.report.equations.is_empty() {
            ""
        } else {
            EQUATION_CSS
        },
        if annotated("ruby") { RUBY_CSS } else { "" },
        if annotated("compose") {
            COMPOSE_CSS
        } else {
            ""
        },
        if emitter.report.captions.is_empty() {
            ""
        } else {
            CAPTION_CSS
        },
        if options.page_navigation {
            super::css::navigation_bar_css()
        } else {
            ""
        },
        style_rules
    );
    if options.reading_view {
        css.push_str(&reading::css(document));
    }
    Ok((
        bundle::assemble(prefix, resources, scripts, &out, &css),
        emitter.report,
    ))
}
