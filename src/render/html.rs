use std::collections::BTreeMap;

use base64::Engine;
use sha2::{Digest, Sha256};

use super::bundle::RenderContext;
use super::semantic::ParagraphGroup;
pub use crate::layout::presentation::gradient::gradient_band_colors;
use crate::layout::presentation::gradient::{
    band_angle_supported, band_extent, gradient_band_edges,
};
use crate::layout::presentation::{
    self, column_groups, objects as object_plan, LineItem, TablePlacement,
};
use crate::layout::{css_mm, css_mm_hundredths};
use crate::model::{
    BorderStroke, LayoutDocument, LayoutPage, LineFill, LineFragment, Paragraph, PositionedObject,
    Table, TableCell, Token, TokenKind,
};
use crate::render::{RenderOptions, UnsupportedPolicy};

/// Sets the width of spaces, then tightens each line's runs until they fit
/// its box, or widens a justified line to fill it.
///
/// Spaces: Hancom draws a space half an em wide unless the character shape
/// asks for the font's own space (`useFontSpace`, `--hwpx-font-space` in the
/// stylesheet); measured on 0713AI 도입.pdf, where HCR Batang's own space is
/// 0.3em and 맑은 고딕's 0.35em. No CSS unit is a font's space width, so the
/// script measures each run class's U+00A0 in the font the browser picked
/// and sets its `word-spacing` to the difference. Where U+0020 and U+00A0
/// differ (바탕, 굴림, 돋움) it first spells that class's spaces as U+00A0.
/// Some fonts draw U+00A0 as a box (the Hancom bundle's HY fonts map it to a
/// glyph with an outline; their U+0020 is empty, and Hancom draws no glyph for
/// a space). The script draws each class's U+00A0 on a canvas and counts the
/// pixels, because `measureText` reports a box for the blank glyph of the
/// built-in bitmap fonts. A class that draws ink is spelled the other way
/// round: its U+00A0 (text and `data-gen`) become U+0020, `white-space:pre`
/// keeps the runs of spaces and the leading ones, and `word-spacing` is
/// measured from U+0020 so a space is still half an em.
///
/// Fitting: a line's text ends at its box's `width` (the source `horzsize`),
/// which holds its hanging indent (`padding-left`): the indent narrows the
/// line, not moves it (rhwp `available_width`; 성과보고서 p160, where both
/// lines of a hanging paragraph store 48188). Spaces that end a line hang
/// past its edge, as in Hancom, so they are not measured. An overflowing line first shrinks its spaces by up to
/// its paragraph's `condense` percent (최소 공백, `--hwpx-condense`), then its
/// letters. A tab (`htC`) counts as its
/// source width, which is its flow width: the box plus the margins that keep
/// a leader a quarter em inside both ends. Leader dots
/// inside it are clipped filler, neither measured nor tightened: counting
/// their unclipped width squeezed every table-of-contents line to the limit
/// and scattered the right-aligned page numbers (성과보고서 목차). A page
/// that is not laid out (hidden by the one-page view) waits until it shows.
///
/// Widening: a line marked `data-hwpx-fill` (`layout::line_fill`) that is
/// shorter than its box spreads the room over its spaces, the same amount
/// each, leaving out the ones that end it and the marked fixed spaces
/// (`data-hwpx-fixed`), as rhwp's `compute_line_extra_spacing` does; with
/// no such space, or when marked `letter` (배분), over the gaps between its
/// letters. It measures to the right edge of its last visible letter, so a
/// bullet or trailing spaces do not count, and corrects once for rounding.
/// Hancom's 0713AI 도입.pdf widens such lines' spaces to 3.3~3.45mm.
/// In the logical DOM (D35) a page's lines are in the chains marked with its
/// `data-page`, not inside its sheet of paper; in the direct output they are
/// the lines marked with its `data-pg` and the lines inside those (a table
/// set inline in a line).
///
/// Batching: a line's fit depends only on its own runs, so the script does one
/// step for every line of the pages it was given (all read, then all written)
/// before the next step. Fitting a line on its own writes a letter spacing and
/// reads the width back many times, and every read after a write makes the
/// browser lay out again: 433 pages of 성과보고서 took 110 s in `beforeprint`
/// that way. Widening and squeezing spaces give the same values batched. The
/// letter spacing search no longer bisects sixteen times: a line's width is
/// linear in its letter spacing (the slope is its letter count; the browser
/// rounds each run to 1/64 px), so it starts from the spacing that width
/// predicts, then moves by the measured miss, and stops at the first measured
/// spacing that fits with at most two 1/64 px steps to spare (bisection only
/// as a fallback). Most lines take one or two measurements, and the whole
/// document 5 s. The spacing differs from the bisection's by at most about
/// 0.03 px of line width.
///
/// An equation (`.heq > .hmq > math`, the direct writer's) is fitted to the box
/// its source stores for it. The box holds its place in the line and `.hmq`
/// stands out of the flow, so when the browser sets the formula wider than the
/// box (the 11pt 직접비 formulas of 15번 and 16번 by 23px: Hancom drew them
/// squeezed into the stored width) the overflow is only ink. The script reads
/// the box's width and the formula's, with no earlier fit in place, and
/// squeezes `.hmq` about its left edge by their ratio when the formula is
/// wider, never widening one. Only the width changes: the baseline, the
/// height and the reserved box stay. Every shown page is fitted again each time
/// pages are given to the script (also on `beforeprint`, when every page is
/// shown) and when fonts finish loading; the fit starts from no transform, so
/// repeating it gives the same value.
pub const SCRIPT_SOURCE: &str = r#"(()=>{const lines=page=>{const n=page.dataset.page;return n?document.querySelectorAll('[data-page="'+n+'"]:not(.hpa) .hls,[data-pg="'+n+'"] .hls,.hls[data-pg="'+n+'"]'):page.querySelectorAll('.hls');};const nominal=new Map();const spaces=()=>{const members=new Map();document.querySelectorAll('.hrt').forEach(e=>e.classList.forEach(c=>{if(/^cs\d+$/.test(c)){if(!members.has(c))members.set(c,[]);members.get(c).push(e);}}));const box=document.createElement('div');box.style.cssText='position:absolute;visibility:hidden';const probes=[...members.keys()].map(c=>{const p=document.createElement('span');p.className='hrt '+c;p.style.cssText='position:absolute;white-space:pre;letter-spacing:0;word-spacing:0';p.textContent=' \u00a0';box.append(p);return p;});document.body.append(box);const advance=(p,i)=>{const r=document.createRange();r.setStart(p.firstChild,i);r.setEnd(p.firstChild,i+1);return r.getBoundingClientRect().width;};const cv=(()=>{try{return document.createElement('canvas').getContext('2d',{willReadFrequently:true});}catch(e){return null;}})();const inked=(s,size)=>{if(!cv)return false;const px=Math.min(size,40),z=Math.ceil(px*2)+8;cv.canvas.width=cv.canvas.height=z;cv.font='10px sans-serif';cv.font=s.fontStyle+' '+s.fontWeight+' '+px+'px '+s.fontFamily;cv.textBaseline='middle';cv.fillText('\u00a0',4,z/2);const d=cv.getImageData(0,0,z,z).data;for(let i=3;i<d.length;i+=4)if(d[i])return true;return false;};let css='.hrt{word-spacing:0}';const mixed=[],boxed=[];probes.forEach(p=>{const c=p.classList[1],s=getComputedStyle(p),size=parseFloat(s.fontSize);if(!(size>0))return;const space=advance(p,0),nbsp=advance(p,1),fontSpace=s.getPropertyValue('--hwpx-font-space').trim()==='1';if(inked(s,size)){boxed.push(c);nominal.set(c,fontSpace?space/size:.5);css+='.'+c+'{white-space:pre'+(fontSpace?'':';word-spacing:'+((size/2-space)/size).toFixed(4)+'em')+'}';return;}if(fontSpace){nominal.set(c,space/size);return;}nominal.set(c,.5);if(Math.abs(space-nbsp)>.01)mixed.push(c);css+='.'+c+'{word-spacing:'+((size/2-nbsp)/size).toFixed(4)+'em}';});box.remove();const convert=(list,from,to)=>list.forEach(c=>members.get(c).forEach(e=>{e.childNodes.forEach(n=>{if(n.nodeType===3&&n.data.includes(from))n.data=n.data.split(from).join(to);});if(e.dataset.gen&&e.dataset.gen.includes(from))e.dataset.gen=e.dataset.gen.split(from).join(to);}));convert(mixed,' ','\u00a0');convert(boxed,'\u00a0',' ');const sheet=document.createElement('style');sheet.textContent=css;document.head.append(sheet);};
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
const fits=[];
pages.forEach(page=>{const n=page.dataset.page;(n?document.querySelectorAll('[data-page="'+n+'"]:not(.hpa) .heq,[data-pg="'+n+'"] .heq'):page.querySelectorAll('.heq')).forEach(heq=>{const q=heq.querySelector('.hmq'),m=q&&q.querySelector('math');if(m){q.style.transform='';fits.push([heq,q,m]);}});});
const reads=fits.map(([heq,q,m])=>[heq.getBoundingClientRect().width,m.getBoundingClientRect().width]);
fits.forEach(([heq,q],i)=>{const[box,ink]=reads[i];if(box>0&&ink>box+.01){q.style.transformOrigin='0 0';q.style.transform='scaleX('+(box/ink)+')';}});
};
const start=()=>{
spaces();
const pages=[...document.querySelectorAll('.hpa')];
const done=new WeakSet();
const run=list=>{
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
window.addEventListener('beforeprint',()=>run(pages));
};
if(document.fonts&&document.fonts.ready)document.fonts.ready.then(start);else start();})();"#;

/// Shows one page at a time with a navigation bar (`RenderOptions::page_navigation`).
/// It runs in the head, so the stylesheet's `hwpx-paged` rules hide every page
/// but the first while the rest are still arriving, and none of them is laid
/// out. The bar is made here, not written in the markup, and the current page
/// is `#page-N` in the address. The bar takes no room and stays hidden until
/// the pointer reaches the top edge of the window, then appears over the page
/// (a touch there shows it until the next touch elsewhere; a control focused
/// from the keyboard shows it until the focus leaves the bar). A click or touch in the left third of the shown paper
/// goes back a page and one in its right third goes forward; one outside the
/// paper, on the bar, a link or a control, or ending a text selection does
/// not. Without the script every page shows; before printing it shows them
/// all again so the letter-spacing script can measure each one, and the
/// stylesheet prints every page without the bar. That
/// `beforeprint` listener is added when the head script runs, not when the
/// document has loaded: the letter-spacing script's own listener is added
/// sooner than that whenever the fonts are ready, and a listener added first
/// runs first. It would then measure pages that are still hidden, fit none of
/// them, and leave the printout unfitted. In the
/// logical DOM (D35) the current page is its paper and every chain marked
/// with its `data-page`.
pub const NAVIGATION_SCRIPT: &str = r#"(()=>{const root=document.documentElement;root.classList.add('hwpx-paged');addEventListener('beforeprint',()=>root.classList.remove('hwpx-paged'));document.addEventListener('DOMContentLoaded',()=>{const pages=[...document.querySelectorAll('.hpa')];if(pages.length<2){root.classList.remove('hwpx-paged');return;}let current=-1;const group=i=>{const n=pages[i].dataset.page;return n?document.querySelectorAll('[data-page="'+n+'"]'):[pages[i]];};const make=(tag,props)=>Object.assign(document.createElement(tag),props);const nav=make('nav',{className:'hwpx-nav'});nav.setAttribute('aria-label','쪽 이동');const prev=make('button',{type:'button',textContent:'‹ 이전',title:'이전 쪽 (←)'});const next=make('button',{type:'button',textContent:'다음 ›',title:'다음 쪽 (→)'});const input=make('input',{type:'number',min:1,max:pages.length,title:'쪽 번호'});input.setAttribute('aria-label','쪽 번호');nav.append(prev,input,make('span',{textContent:'/ '+pages.length}),next);const show=index=>{if(Number.isNaN(index))index=current;index=Math.max(0,Math.min(pages.length-1,Math.floor(index)));if(index!==current){if(current>=0)group(current).forEach(e=>e.classList.remove('hwpx-current'));group(index).forEach(e=>e.classList.add('hwpx-current'));current=index;scrollTo(0,0);}input.value=index+1;prev.disabled=index===0;next.disabled=index===pages.length-1;const hash='#page-'+(index+1);if(location.hash!==hash&&(location.hash||index))try{history.replaceState(null,'',hash);}catch(e){}};const fromHash=()=>{const m=/^#page-(\d+)$/.exec(location.hash);return m?m[1]-1:NaN;};prev.addEventListener('click',()=>show(current-1));next.addEventListener('click',()=>show(current+1));input.addEventListener('change',()=>show(input.valueAsNumber-1));addEventListener('hashchange',()=>{const i=fromHash();if(!Number.isNaN(i))show(i);});addEventListener('keydown',e=>{const step=e.key==='ArrowLeft'?-1:e.key==='ArrowRight'?1:0;if(!step||e.defaultPrevented||e.altKey||e.ctrlKey||e.metaKey||e.shiftKey)return;if(e.target instanceof Element&&e.target.closest('input,textarea,select,[contenteditable]'))return;e.preventDefault();show(current+step);});let touch=false;addEventListener('pointerdown',e=>{touch=e.pointerType!=='mouse';},true);addEventListener('click',e=>{if(nav.contains(e.target)){if(touch)nav.classList.add('hwpx-show');return;}nav.classList.remove('hwpx-show');if(e.button||e.defaultPrevented||e.altKey||e.ctrlKey||e.metaKey||e.shiftKey||!root.classList.contains('hwpx-paged'))return;if(e.target instanceof Element&&e.target.closest('a[href],button,input,textarea,select,label,[contenteditable]'))return;const s=getSelection();if(s&&!s.isCollapsed)return;const r=pages[current].getBoundingClientRect(),x=e.clientX-r.left,w=r.width;if(x<0||x>=w||e.clientY<r.top||e.clientY>=r.bottom)return;const step=x<w/3?-1:x>=w*2/3?1:0;if(step)show(current+step);});nav.addEventListener('focusin',()=>{try{if(nav.querySelector(':focus-visible'))nav.classList.add('hwpx-show');}catch(e){}});nav.addEventListener('focusout',e=>{if(!nav.contains(e.relatedTarget))nav.classList.remove('hwpx-show');});addEventListener('afterprint',()=>root.classList.add('hwpx-paged'));document.body.prepend(nav);show(fromHash());root.classList.add('hwpx-ready');});})();"#;

const PAGE_NUMBER_HEIGHT: i64 = 1000;
const PAGE_NUMBER_BOTTOM_INSET: i64 = 2;
/// How much wider the reference's `heq` box is than the equation's own `sz`.
/// Pinned to a single value by intersecting eight independent observations
/// (see `render_object`); the height needs no such adjustment.
const EQUATION_WIDTH_EXTRA: i64 = 112;

pub fn render_document(document: &LayoutDocument, options: &RenderOptions) -> String {
    super::render(document, options)
}

pub(super) fn render_external_document(
    document: &RenderContext<'_>,
    options: &RenderOptions,
    stylesheet_link: &str,
) -> String {
    let csp = csp_meta(&scripts(options), true);
    let mut html = String::with_capacity(estimate_output_size(document));
    // The reference exporter misspells its charset meta (`http_quiv`), which
    // leaves the encoding to browser guessing: a mostly-ASCII document is
    // read as windows-1252 and its stylesheet's Hangul font names turn to
    // mojibake. The output declares it.
    html.push_str("<!DOCTYPE html>\n<html lang=\"ko\"><head><meta charset=\"utf-8\"><title>");
    // D30: many content.hpf titles are fragments ("2008", "1"); the file
    // name names the document, the source title stays as meta.
    html.push_str(&escape_html(
        options.source_name.as_deref().unwrap_or(&document.title),
    ));
    html.push_str("</title><meta name=\"generator\" content=\"hwpx2html semantic\">");
    if !document.title.is_empty() {
        html.push_str("<meta name=\"hwpx-title\" content=\"");
        html.push_str(&escape_html_attribute(&document.title));
        html.push_str("\">");
    }
    html.push_str("<meta http-equiv=\"Content-Security-Policy\" content=\"");
    html.push_str(&escape_html_attribute(&csp));
    html.push_str("\">");
    html.push_str(stylesheet_link);
    if options.page_navigation {
        html.push_str("<script>");
        html.push_str(NAVIGATION_SCRIPT);
        html.push_str("</script>");
    }
    html.push_str("</head><body>");
    let mut gradients = GradientIds { next: 0 };
    for page in &document.pages {
        render_page(&mut html, page, document, options, &mut gradients);
    }
    if options.adjust_letter_spacing {
        html.push_str("<script>");
        html.push_str(SCRIPT_SOURCE);
        html.push_str("</script>");
    }
    html.push_str("</body></html>");
    #[cfg(test)]
    let html = if document.observe_keys {
        super::observation::annotate_parts(&html)
    } else {
        html
    };
    let html = super::semantic::finalize_parts(&html);
    if options.logical_dom {
        super::logical::rebuild(&html)
    } else {
        html
    }
}

/// Move every inline `style` into a class of the stylesheet, one class per
/// distinct declaration list in order of first use (`z0`, `z1`, ...), and
/// return the rules. The positions and sizes placing every box are half of
/// the markup; moved out, a reader of the markup (an AI given the raw HTML)
/// sees the structure and the text, and the stylesheet carries the
/// geometry. Appended after every other rule, a class overrides the rules
/// the inline style overrode: none is more specific than one class. Our
/// tags write `class` before `style`, and no attribute value holds a quote
/// or `<`; the scripts, whose code does, are left alone.
pub(super) fn styles_to_classes(html: &str) -> (String, String) {
    let mut classes = std::collections::HashMap::<&str, usize>::new();
    let mut rules = String::new();
    let mut output = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<script>") {
        let end = rest[start..]
            .find("</script>")
            .map_or(rest.len(), |end| start + end + "</script>".len());
        move_styles(&rest[..start], &mut classes, &mut rules, &mut output);
        output.push_str(&rest[start..end]);
        rest = &rest[end..];
    }
    move_styles(rest, &mut classes, &mut rules, &mut output);
    (output, rules)
}

fn move_styles<'a>(
    markup: &'a str,
    classes: &mut std::collections::HashMap<&'a str, usize>,
    rules: &mut String,
    output: &mut String,
) {
    let mut rest = markup;
    while let Some(at) = rest.find(" style=\"") {
        let value_start = at + " style=\"".len();
        let Some(length) = rest[value_start..].find('"') else {
            break;
        };
        let value = &rest[value_start..value_start + length];
        let next = classes.len();
        let index = *classes.entry(value).or_insert_with(|| {
            rules.push_str(&format!(".z{next} {{{value}}}\n"));
            next
        });
        let tag_start = rest[..at].rfind('<').unwrap_or(0);
        match rest[tag_start..at].find(" class=\"") {
            Some(class_at) => {
                let class_end = tag_start
                    + class_at
                    + " class=\"".len()
                    + rest[tag_start + class_at + " class=\"".len()..at]
                        .find('"')
                        .unwrap_or(0);
                output.push_str(&rest[..class_end]);
                output.push_str(&format!(" z{index}"));
                output.push_str(&rest[class_end..at]);
            }
            None => {
                output.push_str(&rest[..at]);
                output.push_str(&format!(" class=\"z{index}\""));
            }
        }
        rest = &rest[value_start + length + 1..];
    }
    output.push_str(rest);
}

/// The inline scripts `options` asks for, in document order.
pub(super) fn scripts(options: &RenderOptions) -> Vec<&'static str> {
    [
        (options.page_navigation, NAVIGATION_SCRIPT),
        (options.adjust_letter_spacing, SCRIPT_SOURCE),
    ]
    .into_iter()
    .filter_map(|(on, script)| on.then_some(script))
    .collect()
}

pub fn script_hash_base64(script: &str) -> String {
    let digest = Sha256::digest(script.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(digest)
}

pub(super) fn csp_meta(scripts: &[&str], external: bool) -> String {
    let script = if scripts.is_empty() {
        "'none'".to_owned()
    } else {
        scripts
            .iter()
            .map(|script| format!("'sha256-{}'", script_hash_base64(script)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let local = if external { " 'self'" } else { "" };
    format!("default-src 'none'; script-src {script}; style-src 'unsafe-inline'{local}; img-src data:{local}; object-src 'none'; base-uri 'none'")
}

/// A table cell's own `borderFill` linear gradation, resolved to band
/// colours and (half-height-relative) edges -- `None` for a solid or
/// unverified-angle fill, which keeps its existing (undrawn) behaviour.
/// Only `gradient_angle == 0` has a verified band geometry, the same
/// restriction `render_gradient_box` applies to a standalone shape's own
/// gradient.
///
/// Unlike a shape (whose own `gradient_band_edges` call uses its full
/// height), a cell's bands converge on its own *half*-height, one copy
/// counting down from the bottom edge and the other up from the top --
/// verified against 성과보고서 s6's "바이오헬스분야 핵심기술개발…" box
/// (`borderFill` id 159, white->#86AFDC, step 255): calling
/// `gradient_band_edges` with the full height reproduces the right colours
/// at the right two edges but at half the reference's resolution (256
/// steps spanning the whole height instead of the half), and the two
/// copies visibly cross past the middle instead of meeting there.
fn cell_gradient_bands(cell: &TableCell) -> Option<(Vec<[u8; 3]>, Vec<i64>)> {
    if cell.gradient_angle != 0 || cell.gradient.len() != 2 || cell.gradient_step == 0 {
        return None;
    }
    let bands = gradient_band_colors(cell.gradient_step, &cell.gradient[0], &cell.gradient[1]);
    let half_height = cell.box_units.height.max(1) / 2;
    let edges = gradient_band_edges(half_height, cell.gradient_step);
    if bands.is_empty() || edges.len() != bands.len() + 1 {
        return None;
    }
    Some((bands, edges))
}

/// Document-wide counter for gradient `<pattern>` ids. The reference numbers
/// them `g_0`, `g_1`, ... in DOM order across the whole export, unlike the
/// per-page `w_` ids a cell fill uses.
pub(super) struct GradientIds {
    next: usize,
}

impl GradientIds {
    pub(super) fn new() -> Self {
        Self { next: 0 }
    }

    fn allocate(&mut self) -> String {
        let id = format!("g_{}", self.next);
        self.next += 1;
        id
    }
}

/// How far past its own box the reference grows a shape's `hsR` container and
/// the `svg` inside it, both in HWPUNIT.
///
/// The container takes the pen's two half-widths (truncated, so an odd pen
/// loses its last unit), and the `svg` adds a further constant on every side.
/// Six box dimensions across two documents and two pen widths (28 and 33)
/// agree exactly on both numbers.
const SHAPE_SVG_MARGIN: i64 = 42;

/// One run of lines sharing a column layout, in flow order.
/// The page's number, drawn as generated content at the foot of the page.
pub(super) fn render_page_number(
    html: &mut String,
    page: &LayoutPage,
    document: &LayoutDocument,
    number: &str,
) {
    let style = document
        .char_styles
        .iter()
        .find(|style| style.id == page.spec.page_number_char_style_id);
    let page_number_width = page_number_width(style, number);
    html.push_str(
        "<div class=\"hpN\" data-hwpx-artifact=\"page-number\" aria-hidden=\"true\" style=\"left:",
    );
    html.push_str(&css_mm(
        (page.spec.width.saturating_sub(page_number_width)) / 2,
    ));
    // The number sits at the bottom margin's top, or at its middle when the
    // footer margin is 0: against the reference, the pages of the 3 documents
    // whose sections have no footer all sit half a bottom margin higher than
    // the rest (676 pages, four margin combinations). A footer between 0 and
    // half the bottom margin has no sample and keeps the full margin.
    let bottom_gap = if page.spec.footer == 0 {
        page.spec.margin_bottom / 2
    } else {
        page.spec.margin_bottom
    };
    html.push_str(";top:");
    html.push_str(&css_mm(
        page.spec
            .height
            .saturating_sub(bottom_gap)
            .saturating_sub(PAGE_NUMBER_BOTTOM_INSET),
    ));
    html.push_str(";width:");
    html.push_str(&css_mm(page_number_width));
    html.push_str(";height:");
    html.push_str(&css_mm(PAGE_NUMBER_HEIGHT));
    // The box hugs the printed string (see `page_number_width`), but a
    // browser's own glyph metrics rarely match Hancom's exactly; centering
    // the text inside that box absorbs the leftover instead of letting it
    // push the glyphs to one side. This departs from the reference HMF's
    // left-aligned box on purpose: HWPX's `pageNum` is `pos="BOTTOM_CENTER"`
    // and Hancom's own screen draws the glyphs filling the box (user
    // decision 2026-10-02; see
    // docs/history/2026-10-02-page-number-width-root-cause.md).
    html.push_str(";text-align:center;\"><span class=\"hrt cs");
    html.push_str(&page.spec.page_number_char_style_id.to_string());
    // Generated, not source text: drawn outside the DOM text (D27).
    html.push_str("\" data-gen=\"");
    html.push_str(&escape_html_attribute(number));
    html.push_str("\"></span></div>");
}

fn render_page(
    html: &mut String,
    page: &LayoutPage,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    gradients: &mut GradientIds,
) {
    let mut patterns = PatternIds {
        page: page.index,
        next: 0,
        colors: Vec::new(),
    };
    html.push_str("<div class=\"hpa\" style=\"width:");
    html.push_str(&css_mm(page.spec.width));
    html.push_str(";height:");
    html.push_str(&css_mm(page.spec.height));
    html.push_str(";\">");
    if let Some(number) = &page.page_number {
        render_page_number(html, page, document, number);
    }
    let groups = column_groups(&page.lines);
    // Each column container (`hcI`) with its group, column and offset in `hcD`.
    let containers = presentation::containers(&groups)
        .into_iter()
        .map(|container| {
            (
                container.group,
                container.column,
                container.left,
                container.top,
            )
        })
        .collect::<Vec<_>>();
    let column_lines = |group: usize, column: u32| {
        groups[group]
            .lines
            .iter()
            .filter(move |line| line.column_index == column)
    };
    // Where each paragraph's lines end on this page: container, line.
    let mut paragraph_ends = std::collections::HashMap::<&str, (usize, usize)>::new();
    for (index, &(group, column, _, _)) in containers.iter().enumerate() {
        for (line_index, line) in column_lines(group, column).enumerate() {
            paragraph_ends.insert(line.paragraph_key.as_str(), (index, line_index));
        }
    }
    // The objects were written after the last group's first column; the
    // floating tables after the body, in the reference HTML's stacking order
    // (anchor z-order), not necessarily the XML spine order.
    let objects_container = containers
        .iter()
        .position(|&(group, column, _, _)| group + 1 == groups.len() && column == 0);
    let mut floating_tables = page
        .tables
        .iter()
        .filter(|placed| !placed.table.anchor.treat_as_char)
        .collect::<Vec<_>>();
    floating_tables.sort_by_key(|placed| (placed.table.anchor.z_order, placed.fragment_index));
    let object_slots = page
        .objects
        .iter()
        .map(
            |object| match paragraph_ends.get(object.paragraph_key.as_str()) {
                Some(&(container, line)) => FloatSlot::After(container, line),
                None => FloatSlot::Unanchored,
            },
        )
        .collect::<Vec<_>>();
    let table_slots = floating_tables
        .iter()
        .map(|placed| match table_anchor_key(&placed.table.id) {
            Some(key) => match paragraph_ends.get(key.as_str()) {
                Some(&(container, line)) => FloatSlot::After(container, line),
                None => FloatSlot::PageStart,
            },
            None => FloatSlot::Unanchored,
        })
        .collect::<Vec<_>>();
    // The inferred outline (D25) is decided in reading order before the page
    // is written: the floats are written after the body, but a title bar
    // floating from a paragraph heads what follows it.
    if options.infer_structure {
        let floats_at = |slot: FloatSlot| {
            floating_tables
                .iter()
                .zip(&table_slots)
                .filter(move |(_, &at)| at == slot)
                .map(|(placed, _)| &placed.table)
        };
        for table in floats_at(FloatSlot::PageStart) {
            super::semantic::outline_title(document, table);
        }
        for (index, &(group, column, _, _)) in containers.iter().enumerate() {
            for (line_index, line) in column_lines(group, column).enumerate() {
                super::semantic::outline_paragraph(document, line);
                for placed in &page.tables {
                    if placed.table.anchor.treat_as_char && placed.table.anchor_y == Some(line.top)
                    {
                        super::semantic::outline_inline_title(document, line, &placed.table);
                    }
                }
                for table in floats_at(FloatSlot::After(index, line_index)) {
                    super::semantic::outline_title(document, table);
                }
            }
        }
        for table in floats_at(FloatSlot::Unanchored) {
            super::semantic::outline_title(document, table);
        }
    }
    // Where each slot lies in `html`, and each float as rendered with the
    // style of its layer, in the old order so pattern and gradient ids are
    // allocated as before.
    let mut slot_positions = Vec::<(FloatSlot, usize)>::new();
    let mut floats = Vec::<(FloatSlot, String, String)>::new();
    let offset_of = |slot: FloatSlot| match slot {
        FloatSlot::After(container, _) => (containers[container].2, containers[container].3),
        FloatSlot::PageStart => containers.first().map_or((0, 0), |c| (c.2, c.3)),
        FloatSlot::Unanchored => {
            objects_container.map_or((0, 0), |c| (containers[c].2, containers[c].3))
        }
        FloatSlot::PageEnd => (0, 0),
    };
    // A float keeps the coordinates of the container it was written in: its
    // layer cancels the offsets of the container it moved to with negative
    // `left`/`top` and re-adds the old ones as `margin`. The browser snaps
    // each offset separately, so equal and opposite ones cancel exactly,
    // where re-based coordinates would round differently.
    let objects_offset = offset_of(FloatSlot::Unanchored);
    let body_offset = (
        page.spec.margin_left,
        page.spec.margin_top + page.spec.header,
    );
    // A negative length is the exact mirror of the positive one printed for
    // the container.
    let signed_mm = |value: i64| {
        if value < 0 {
            format!("-{}", css_mm(-value))
        } else {
            css_mm(value)
        }
    };
    let layer_style = |to: (i64, i64), from: (i64, i64)| {
        let mut style = format!(
            "position:absolute;left:{};top:{};",
            signed_mm(-to.0),
            signed_mm(-to.1)
        );
        if from.0 != 0 {
            style.push_str(&format!("margin-left:{};", signed_mm(from.0)));
        }
        if from.1 != 0 {
            style.push_str(&format!("margin-top:{};", signed_mm(from.1)));
        }
        style
    };
    let render_objects = |floats: &mut Vec<(FloatSlot, String, String)>,
                          patterns: &mut PatternIds,
                          gradients: &mut GradientIds| {
        for (object, &slot) in page.objects.iter().zip(&object_slots) {
            let target = offset_of(slot);
            let style = if target == objects_offset {
                layer_style((0, 0), (0, 0))
            } else {
                layer_style(target, objects_offset)
            };
            let mut rendered = String::new();
            render_object(
                &mut rendered,
                object,
                document,
                options,
                patterns,
                gradients,
            );
            floats.push((slot, style, rendered));
        }
    };
    html.push_str("<div class=\"hcD\" style=\"left:");
    html.push_str(&css_mm(page.spec.margin_left));
    html.push_str(";top:");
    html.push_str(&css_mm(page.spec.margin_top + page.spec.header));
    html.push_str(";\">");
    let inline_tables = page
        .tables
        .iter()
        .filter(|placed| placed.table.anchor.treat_as_char)
        .map(|placed| &placed.table)
        .collect::<Vec<_>>();
    // A list continues into the next column only when no float is written
    // in between: joined, its parts would read before the float.
    if containers.is_empty() || table_slots.contains(&FloatSlot::PageStart) {
        document.open_lists.take();
    }
    let floats_after = |index: usize| {
        (Some(index) == objects_container && object_slots.contains(&FloatSlot::Unanchored))
            || (index + 1 == containers.len() && table_slots.contains(&FloatSlot::Unanchored))
    };
    for (index, &(group, column, left, top)) in containers.iter().enumerate() {
        html.push_str("<div class=\"hcI\"");
        if left != 0 || top != 0 {
            html.push_str(" style=\"");
            if left != 0 {
                html.push_str("left:");
                html.push_str(&css_mm(left));
                html.push(';');
            }
            if top != 0 {
                html.push_str("top:");
                html.push_str(&css_mm(top));
                html.push(';');
            }
            html.push('"');
        }
        html.push('>');
        if index == 0 {
            slot_positions.push((FloatSlot::PageStart, html.len()));
        }
        let mut paragraphs =
            ParagraphGroup::new(document, options.infer_structure).continuing(document);
        for (line_index, line) in column_lines(group, column).enumerate() {
            let phrasing = !carries_block_content(line, &inline_tables);
            paragraphs.line(html, document, line, phrasing, |html| {
                render_line(
                    html,
                    line,
                    document,
                    options,
                    &inline_tables,
                    &mut patterns,
                    gradients,
                )
            });
            let slot = FloatSlot::After(index, line_index);
            if object_slots.contains(&slot) || table_slots.contains(&slot) {
                // The paragraph ends here on this page; its floats follow it.
                paragraphs.close(html);
                slot_positions.push((slot, html.len()));
            }
        }
        paragraphs.end_column(html, document, !floats_after(index));
        if Some(index) == objects_container {
            slot_positions.push((FloatSlot::Unanchored, html.len()));
            render_objects(&mut floats, &mut patterns, gradients);
        }
        html.push_str("</div>");
    }
    if containers.is_empty() {
        html.push_str("<div class=\"hcI\">");
        slot_positions.push((FloatSlot::PageStart, html.len()));
        slot_positions.push((FloatSlot::Unanchored, html.len()));
        render_objects(&mut floats, &mut patterns, gradients);
        html.push_str("</div>");
    }
    html.push_str("</div>");
    let tables_position = html.len();
    for (placed, &slot) in floating_tables.iter().zip(&table_slots) {
        let frame_adjustment = true;
        let (left, top) = presentation::floating_table_origin(&page.spec, &placed.table);
        let mut rendered = String::new();
        render_table(
            &mut rendered,
            &placed.table,
            document,
            options,
            TablePlacement::Page {
                left,
                top,
                frame_adjustment,
            },
            &mut patterns,
            gradients,
        );
        // Page coordinates: a table moved into a column container cancels
        // both that container's and the body's offsets.
        let (slot, style) = if slot == FloatSlot::Unanchored {
            (FloatSlot::PageEnd, layer_style((0, 0), (0, 0)))
        } else {
            let (left, top) = offset_of(slot);
            (
                slot,
                layer_style((left, top), (-body_offset.0, -body_offset.1)),
            )
        };
        floats.push((slot, style, rendered));
    }
    slot_positions.push((FloatSlot::PageEnd, tables_position));
    // Each float in a zero-size layer numbered in the old paint order: every
    // float still paints above the body text and in the same order among
    // themselves wherever they overlap.
    let mut insertions = floats
        .into_iter()
        .enumerate()
        .filter(|(_, (_, _, rendered))| !rendered.is_empty())
        .map(|(layer, (slot, style, rendered))| {
            let position = slot_positions
                .iter()
                .find(|(candidate, _)| *candidate == slot)
                .map(|&(_, position)| position)
                .expect("every float slot is recorded");
            (position, layer + 1, style, rendered)
        })
        .collect::<Vec<_>>();
    insertions.sort_by_key(|insertion| std::cmp::Reverse((insertion.0, insertion.1)));
    for (position, layer, style, rendered) in insertions {
        html.insert_str(
            position,
            &format!("<div style=\"{style}z-index:{layer};\">{rendered}</div>"),
        );
    }
    html.push_str("</div>");
}

/// Where a page-level floating table or object is written (D28).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FloatSlot {
    /// After the anchoring paragraph's last line on the page: the column
    /// container and the line's index in it.
    After(usize, usize),
    /// At the top of the body: a table fragment whose anchoring paragraph
    /// is on an earlier page.
    PageStart,
    /// An object without a paragraph on the page, where objects always went:
    /// after the last group's first column.
    Unanchored,
    /// A table without an anchoring paragraph, after the body as before.
    PageEnd,
}

/// The key of the top-level paragraph a floating table id names:
/// `s2/tbl-anchor[17-0]` is anchored in `s2/p[17]`.
fn table_anchor_key(id: &str) -> Option<String> {
    let (section, rest) = id.split_once("/tbl-anchor[")?;
    let (paragraph, _) = rest.split_once('-')?;
    (!section.contains('/') && paragraph.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{section}/p[{paragraph}]"))
}

/// Per-glyph advance widths (HWPUNIT, at the corpus's `height=1000`/10pt
/// basis) for the five named page-number fonts, as (`-`, digit). All ten
/// digit glyphs share one width per font (every corpus font's digits are
/// monospaced). Each value is the font's own `hmtx` advance at 10pt rounded
/// to the nearest 4 HWPUNIT (1/250 em) -- the grid the reference exporter's
/// own box widths round to. An unmatched font uses the 바탕 row; this is
/// also what the former flat `PAGE_NUMBER_WIDTH` constant (2844 = 2×624 +
/// 1000 + 596, for a one-digit "- N -") was silently computed from. See
/// docs/history/2026-10-02-page-number-width-root-cause.md (measurement 3)
/// for the font files, hmtx values and the grid-search that pinned the
/// rounding rule.
fn page_number_font_metrics(family: &str) -> (i64, i64) {
    match family {
        "한컴바탕" => (832, 584),
        "함초롬돋움" => (468, 552),
        "굴림" => (624, 576),
        "HY견고딕" => (668, 624),
        _ => (624, 596), // 바탕
    }
}

/// One character's advance width in `page_number_font_metrics`'s basis.
fn page_number_glyph_width(family: &str, character: char) -> i64 {
    let (dash, digit) = page_number_font_metrics(family);
    match character {
        '-' => dash,
        '0'..='9' => digit,
        // Half an em: the corpus's "쪽 번호" style is always `useFontSpace`
        // off, which Hancom draws as a half-width space. No corpus example
        // has it on; this value is used for that case too until a
        // counter-example turns up.
        ' ' | '\u{00A0}' => 500,
        // No corpus glyph falls outside `-`/digit/space (e.g. a Roman-numeral
        // page number, see KNOWN_ISSUES); falls back to half an em like an
        // unmatched font's space.
        _ => 500,
    }
}

/// The page-number box's width: the sum of the printed string's own glyph
/// widths, scaled from `page_number_font_metrics`'s `height=1000` basis to
/// the style's actual size. Every corpus "쪽 번호" style is exactly
/// `height=1000`, so scaling to other sizes is unverified, but it is
/// mathematically required to keep the box matching the size the glyphs
/// actually render at (`.cs{id}{font-size:…}`).
///
/// 장평(`ratio`)/자간(`spacing`) are deliberately left out: the renderer
/// never turns `CharStyle::ratio` into a horizontal transform anywhere else
/// (it is parsed but otherwise unused), so scaling this box by it would pull
/// the box away from the glyphs it exists to hug; corpus has no non-default
/// example of either to check against. `style` is `None` when no char style
/// matches `page_number_char_style_id` -- a data anomaly the corpus has no
/// example of -- and falls back to the 바탕 table at `height=1000`.
fn page_number_width(style: Option<&crate::model::CharStyle>, number: &str) -> i64 {
    let (family, font_size_hwp) = style.map_or(("바탕", 1000), |style| {
        (
            super::css::normalize_hangul_font_family(&style.font_family),
            style.font_size_hwp,
        )
    });
    let total: i64 = number
        .chars()
        .map(|character| page_number_glyph_width(family, character))
        .sum();
    total.saturating_mul(font_size_hwp) / 1000
}

/// Whether `render_line` draws block structure inside this line -- an inline
/// table, or an inline text box whose own paragraphs should be `p` -- which
/// keeps the line out of a `p` in semantic mode.
fn carries_block_content(line: &LineFragment, inline_tables: &[&Table]) -> bool {
    fn has_text_box(object: &PositionedObject) -> bool {
        object
            .shape
            .as_ref()
            .is_some_and(|shape| !shape.paragraphs.is_empty())
            || object.children.iter().any(has_text_box)
    }
    inline_tables
        .iter()
        .any(|table| table.anchor_y == Some(line.top))
        || line.inline_objects.iter().any(has_text_box)
}

/// One line box. Its tokens, the tables set in it and its inline objects are
/// written in the order of their controls in the paragraph's text
/// (`presentation::line_items`): a box set in the line is a letter of its
/// text, so where it stands in the DOM is where it stands on the line.
fn render_line(
    html: &mut String,
    line: &LineFragment,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    inline_tables: &[&Table],
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let line_tables = inline_tables
        .iter()
        .copied()
        .filter(|table| table.anchor_y == Some(line.top))
        .collect::<Vec<_>>();
    let inline_table = line_tables.first().copied();
    let presentation::LineBox {
        top: line_top,
        line_height,
        height: line_box_height,
    } = presentation::line_box(line, inline_table);
    html.push_str("<div class=\"hls ps");
    html.push_str(&line.para_style_id.to_string());
    // The correction script widens the line to its width (`line_fill`).
    match line.fill {
        Some(LineFill::Spaces) => html.push_str("\" data-hwpx-fill=\"space"),
        Some(LineFill::Letters) => html.push_str("\" data-hwpx-fill=\"letter"),
        None => {}
    }
    #[cfg(test)]
    if document.observe_keys {
        html.push_str(&super::observation::line_attribute(line));
    }
    html.push_str("\" style=\"");
    if line.padding_left > 0 {
        html.push_str("padding-left:");
        html.push_str(&css_mm(line.padding_left));
        html.push(';');
    }
    html.push_str("line-height:");
    // An `hls` line box is the one place the reference keeps a zero fraction.
    html.push_str(&css_mm_hundredths(line_height));
    html.push_str(";white-space:nowrap;left:");
    html.push_str(&css_mm(line.left));
    html.push_str(";top:");
    html.push_str(&css_mm(line_top));
    html.push_str(";height:");
    html.push_str(&css_mm(line_box_height));
    html.push_str(";width:");
    html.push_str(&css_mm(line.width));
    html.push_str(";\">");
    render_bullet(html, line, document, line_box_height, "div");
    for item in presentation::line_items(line, &line_tables) {
        match item {
            LineItem::Token(index) => render_token(html, &line.tokens[index], document),
            LineItem::Table(index) => render_table(
                html,
                line_tables[index],
                document,
                options,
                TablePlacement::Inline,
                patterns,
                gradients,
            ),
            LineItem::Object(index) => render_inline_object(
                html,
                &line.inline_objects[index],
                document,
                options,
                patterns,
                gradients,
            ),
        }
    }
    html.push_str("</div>");
}

/// The bullet a paragraph style draws before its first line, when the line
/// has one.
pub(super) fn render_bullet(
    html: &mut String,
    line: &LineFragment,
    document: &LayoutDocument,
    line_box_height: i64,
    tag: &str,
) {
    let Some(marker) = &line.bullet_marker else {
        return;
    };
    let style = line.tokens.first().and_then(|token| {
        document
            .char_styles
            .iter()
            .find(|style| style.id == token.char_style_id)
    });
    let char_style_id = style.map_or(0, |value| value.id);
    let font_size = style.map_or(1000, |value| value.font_size_hwp);
    html.push('<');
    html.push_str(tag);
    html.push_str(" class=\"hhe\" style=\"display:inline-block;margin-left:0mm;width:");
    html.push_str(&css_mm(line_box_height));
    html.push_str(";height:");
    html.push_str(&css_mm(line_box_height));
    html.push_str(";\"><span class=\"hrt cs");
    html.push_str(&char_style_id.to_string());
    html.push_str("\" style=\"font-size:");
    html.push_str(&format_pt(font_size));
    // A paraPr bullet is generated, not source text (D27).
    html.push_str("pt;\" data-gen=\"");
    html.push_str(&escape_html_attribute(marker));
    html.push_str("\"></span></");
    html.push_str(tag);
    html.push('>');
}

pub(super) fn render_token(html: &mut String, token: &Token, document: &LayoutDocument) {
    write_token(html, token, document, false);
}

/// A token of a drawn-only copy (a repeated header row): its text is
/// generated content (`data-gen`), so the DOM text holds the row once.
pub(super) fn render_token_generated(html: &mut String, token: &Token, document: &LayoutDocument) {
    write_token(html, token, document, true);
}

/// A text-only span, its (escaped) text either in the DOM or, in a
/// drawn-only copy, generated from `data-gen`. The text escaping also quotes
/// `"`, so the same string serves as the attribute.
fn text_span(html: &mut String, open: &str, escaped: &str, generated: bool) {
    html.push_str(open);
    if generated {
        html.push_str(" data-gen=\"");
        html.push_str(escaped);
        html.push_str("\"></span>");
    } else {
        html.push('>');
        html.push_str(escaped);
        html.push_str("</span>");
    }
}

fn write_token(html: &mut String, token: &Token, document: &LayoutDocument, generated: bool) {
    match &token.kind {
        TokenKind::Control { kind } if kind == "lineBreak" => {
            // A line break inside the text (`hp:t/hp:lineBreak`). The lines
            // on either side are already separate boxes, so the break only
            // has to keep them apart in the text: a zero-size newline, like
            // the tab's (D27). Without it the last word of one line ran into
            // the first of the next ("86.2%매출").
            text_span(html, "<span class=\"htx\"", "\n", generated);
        }
        TokenKind::Control { .. } => {}
        TokenKind::Tab { width, leader } => {
            // `hp:tab/@width` is the whole advance to the next tab stop (in
            // 제어문자 a tab after 62 characters ends at 40000, ten of the
            // section's 4000 stops; in 성과보고서's contents, text + tab +
            // page number fill the 48188 line). The reference exporter draws
            // every tab half an em narrower, shifted a quarter em right, so
            // the text after it starts half an em early and right-aligned
            // page numbers end short of the margin; that is not reproduced
            // (D31). A dotted leader still stops a quarter em short of both
            // neighbours, now as margins inside the full advance.
            let font_size = document
                .char_styles
                .iter()
                .find(|style| style.id == token.char_style_id)
                .map_or(1000, |style| style.font_size_hwp);
            let (inset, marker_width) = if *leader == 3 {
                (font_size / 4, width.saturating_sub(font_size / 2).max(0))
            } else {
                (0, *width)
            };
            // The source tab stays in the DOM text; `htx` collapses it to an
            // invisible zero-size space (D27).
            text_span(html, "<span class=\"htx\"", "\t", generated);
            html.push_str("<span class=\"htC\" aria-hidden=\"true\" style=\"");
            if inset != 0 {
                html.push_str("margin-left:");
                html.push_str(&css_mm(inset));
                html.push(';');
            }
            html.push_str("width:");
            html.push_str(&css_mm(marker_width));
            if inset != 0 {
                html.push_str(";margin-right:");
                html.push_str(&css_mm(width - marker_width - inset));
            }
            html.push_str(";height:100%;\">");
            if *leader == 3 {
                // The dotted leader uses a quarter-em advance. Count only
                // complete dots inside the marker box, excluding its padding.
                let count = marker_width.saturating_mul(4) / font_size.max(1);
                let count = usize::try_from(count.min(10000)).unwrap_or(0);
                html.push_str(&format!(
                    "<span class=\"hrt cs{}\" data-gen=\"{}\"></span>",
                    token.char_style_id,
                    "\u{b7}".repeat(count)
                ));
            }
            html.push_str("</span>");
        }
        TokenKind::LineBreak => html.push_str("<span class=\"hrt cs0\"><br></span>"),
        TokenKind::NonBreakingSpace | TokenKind::FixedSpace => {
            // Its own span carrying the run's own character style, not cs0:
            // the reference emits `<span class="hrt cs84">&nbsp;</span>` for an
            // nbSpace in a charPrIDRef=84 run (샘플/2026 대한민국 산업단지 …),
            // and `cs27` for an fwSpace in a charPrIDRef=27 run (샘플/1.(산업부
            // 공고 제2026-558호) …). No sample in the corpus exercised this
            // path before, so the previous cs0 was never verified.
            // Marked so that widening a justified line leaves it alone, as
            // rhwp does (it widens only U+0020).
            let open = format!(
                "<span class=\"hrt cs{}\" data-hwpx-fixed",
                token.char_style_id
            );
            text_span(html, &open, "\u{a0}", generated);
        }
        TokenKind::Text(text) if generated => {
            // Spelled as `render_text` spells it: a run of spaces keeps its
            // U+00A0s, which generated content does not collapse either.
            let open = format!("<span class=\"hrt cs{}\"", token.char_style_id);
            text_span(html, &open, &escape_html_text(text), true);
        }
        TokenKind::Text(text) => render_text(html, token, text),
    }
}

/// A text run, with a real link when its target is safe (D29).
fn render_text(html: &mut String, token: &Token, text: &str) {
    let link = token
        .hyperlink
        .as_deref()
        .filter(|url| super::semantic::is_safe_link(url));
    let tag = if link.is_some() { "a" } else { "span" };
    html.push('<');
    html.push_str(tag);
    html.push_str(" class=\"hrt cs");
    html.push_str(&token.char_style_id.to_string());
    html.push('"');
    if let Some(url) = link {
        html.push_str(" href=\"");
        html.push_str(&escape_html_attribute(url.trim()));
        html.push_str("\" target=\"_blank\" rel=\"noopener noreferrer\"");
    } else if let Some(url) = &token.hyperlink {
        html.push_str(" data-hwpx-href=\"");
        html.push_str(&escape_html_attribute(url));
        html.push('"');
    }
    // Spaces stay U+00A0 as the reference drew them: several fonts give it a
    // different advance from U+0020, so real spaces would move text (D19
    // over D27; measured on 21 of 55 samples). Their drawn width, half an em
    // as Hancom's, is set by the correction script (`SCRIPT_SOURCE`).
    html.push('>');
    html.push_str(&escape_html_text(text));
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
}

pub(super) struct PatternIds {
    page: usize,
    next: usize,
    /// Solid fill colours already given a pattern id on this page, in
    /// allocation order. A shape's fill pattern reuses an earlier sibling's
    /// id for the same colour instead of redefining it -- verified across
    /// 샘플/여러 색채움 도형을 묶은 컨테이너's three containers, where the
    /// second and third reuse the first's `w_00..w_03` for their identical
    /// colours with no new `<defs>` at all. Table cell fills keep calling
    /// the plain [`Self::allocate`] below and never dedupe, since that
    /// hasn't been verified across separate tables.
    colors: Vec<(String, String)>,
}

impl PatternIds {
    /// Ids for one page's drawings.
    pub(super) fn new(page: usize) -> Self {
        Self {
            page,
            next: 0,
            colors: Vec::new(),
        }
    }

    /// Forget which solid colours already have a pattern on this page, so the
    /// next shape of such a colour defines its own again (the markup that
    /// held the earlier definition was dropped).
    pub(super) fn forget_colors(&mut self) {
        self.colors.clear();
    }

    fn allocate(&mut self) -> String {
        // Reference IDs combine the page number and its paint ordinal. Keep
        // that spelling for single-digit ordinals; separate longer ordinals
        // so page 1/paint 10 cannot collide with page 11/paint 0.
        let id = if self.next < 10 {
            format!("w_{}{}", self.page, self.next)
        } else {
            format!("w_{}_{}", self.page, self.next)
        };
        self.next += 1;
        id
    }

    /// Returns `(id, is_new)` for a solid fill colour, reusing an earlier
    /// id for the same colour on this page rather than allocating fresh.
    fn allocate_for_color(&mut self, color: &str) -> (String, bool) {
        if let Some((_, id)) = self.colors.iter().find(|(existing, _)| existing == color) {
            return (id.clone(), false);
        }
        let id = self.allocate();
        self.colors.push((color.to_owned(), id.clone()));
        (id, true)
    }
}

fn render_table(
    html: &mut String,
    table: &Table,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    placement: TablePlacement,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let frame = presentation::table_box(table, placement);
    html.push_str("<div class=\"htb\"");
    let as_box = box_table(table);
    if as_box {
        html.push_str(&format!(
            " data-hwpx-table=\"{}x{}\"",
            table.rows, table.columns
        ));
        html.push_str(&super::semantic::part_marker('t', &table.id));
    }
    if matches!(placement, TablePlacement::Inline) {
        html.push_str(" style=\"width:");
        html.push_str(&css_mm(frame.width));
        html.push_str(";height:");
        html.push_str(&css_mm(frame.height));
        html.push_str(";display:inline-block;position:relative;vertical-align:-15%;line-height:");
        html.push_str(&css_mm(frame.height));
        html.push_str(";\">");
    } else {
        html.push_str(" style=\"left:");
        html.push_str(&css_mm(frame.left));
        html.push_str(";top:");
        html.push_str(&css_mm(frame.top));
        html.push_str(";width:");
        html.push_str(&css_mm(frame.width));
        html.push_str(";height:");
        html.push_str(&css_mm(frame.height));
        html.push_str(";\">");
    }
    if table_needs_svg(table) {
        html.push_str("<svg class=\"hs\" aria-hidden=\"true\"");
        let svg = presentation::table_svg_frame(table, placement);
        html.push_str(" viewBox=\"");
        html.push_str(&svg_mm(svg.left));
        html.push(' ');
        html.push_str(&svg_mm(svg.top));
        html.push(' ');
        html.push_str(&svg_mm(svg.width));
        html.push(' ');
        html.push_str(&svg_mm(svg.height));
        html.push_str("\" style=\"left:");
        html.push_str(&css_mm(svg.left));
        html.push_str(";top:");
        html.push_str(&css_mm(svg.top));
        html.push_str(";width:");
        html.push_str(&css_mm(svg.width));
        html.push_str(";height:");
        html.push_str(&css_mm(svg.height));
        html.push_str(";\">");
        render_table_svg_body(html, table, patterns, gradients);
        html.push_str("</svg>");
    }
    if as_box {
        render_box_cells(html, table, document, options, patterns, gradients);
    } else {
        render_cells(html, table, document, options, patterns, gradients);
    }
    html.push_str("</div>");
}

/// Whether a table draws anything of its own: a border, a fill, a diagonal
/// or a gradient in some cell.
pub(super) fn table_needs_svg(table: &Table) -> bool {
    table.cells.iter().any(|cell| {
        cell.border_visible
            || cell.fill_color.is_some()
            || cell.diagonal_forward
            || cell.diagonal_backward
            || cell_gradient_bands(cell).is_some()
    })
}

/// The inside of a table's border drawing (`svg.hs`): its fill patterns,
/// fills, diagonals and merged border strokes, in the table's own
/// millimetres from its top left corner.
pub(super) fn render_table_svg_body(
    html: &mut String,
    table: &Table,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let table_width = table.box_units.width.max(1);
    let table_height = table.box_units.height.max(1);
    let coordinate = |value: i64| svg_mm(value);
    let fill_patterns = table
        .cells
        .iter()
        .filter_map(|cell| cell.fill_color.as_deref())
        .fold(Vec::<String>::new(), |mut colors, color| {
            if !colors.iter().any(|existing| existing == color) {
                colors.push(color.to_owned());
            }
            colors
        });
    let pattern_ids = fill_patterns
        .iter()
        .map(|_| patterns.allocate())
        .collect::<Vec<_>>();
    if !fill_patterns.is_empty() {
        html.push_str("<defs>");
        for (index, color) in fill_patterns.iter().enumerate() {
            let (red, green, blue) = rgb_components(color);
            let pattern_id = &pattern_ids[index];
            html.push_str(&format!(
                "<pattern id=\"{pattern_id}\" width=\"10\" height=\"10\" patternUnits=\"userSpaceOnUse\"><rect width=\"10\" height=\"10\" fill=\"rgb({red},{green},{blue})\"/></pattern>"
            ));
        }
        html.push_str("</defs>");
    }
    // A cell's own two-colour linear gradation (`cell_gradient_bands`,
    // verified for `angle == 0` only) gets its own `<pattern>`, one per
    // gradient-filled cell -- unlike a solid fill's `w_` patterns, never
    // deduplicated by colour pair, matching the document-wide, never
    // reused `g_` numbering `GradientIds` already gives a standalone
    // shape's gradient. Band paths reuse `render_gradient_box`'s own
    // colours/edges (`gradient_band_colors`/`gradient_band_edges`) but
    // a plain `[0, cell_width]` span with no shape inset/margin --
    // verified against 성과보고서 s6's "바이오헬스분야 핵심기술개발…"
    // box (`borderFill` id 159, white->#86AFDC, step 255, angle 0).
    let gradient_pattern_ids: Vec<Option<String>> = table
        .cells
        .iter()
        .map(|cell| cell_gradient_bands(cell).map(|_| gradients.allocate()))
        .collect();
    if gradient_pattern_ids.iter().any(Option::is_some) {
        html.push_str("<defs>");
        for (cell, pattern_id) in table.cells.iter().zip(&gradient_pattern_ids) {
            let (Some(pattern_id), Some((bands, edges))) = (pattern_id, cell_gradient_bands(cell))
            else {
                continue;
            };
            let width = cell.box_units.width.max(1);
            let half_height = cell.box_units.height.max(1) / 2;
            html.push_str(&format!(
                "<pattern id=\"{pattern_id}\" width=\"100%\" height=\"100%\" patternUnits=\"userSpaceOnUse\">"
            ));
            for (index, color) in bands.iter().enumerate() {
                let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
                let far = half_height + edges[index];
                let near = half_height + edges[index + 1];
                let mirror_far = half_height - edges[index];
                let mirror_near = half_height - edges[index + 1];
                for (band_far, band_near) in [(far, near), (mirror_far, mirror_near)] {
                    html.push_str(&format!(
                        "<path d=\"M{},{}L{},{}L{},{}L{},{}Z \" style=\"fill:{hex};stroke:{hex};stroke-width:1;\"></path>",
                        coordinate(width),
                        coordinate(band_far),
                        coordinate(0),
                        coordinate(band_far),
                        coordinate(0),
                        coordinate(band_near),
                        coordinate(width),
                        coordinate(band_near),
                    ));
                }
            }
            html.push_str("</pattern>");
        }
        html.push_str("</defs>");
    }
    let mut vertical_segments = BTreeMap::<i64, Vec<(i64, i64, i64, BorderStroke)>>::new();
    let mut horizontal_segments = BTreeMap::<i64, Vec<(i64, i64, i64, BorderStroke)>>::new();
    for (cell, gradient_pattern_id) in table.cells.iter().zip(&gradient_pattern_ids) {
        if let Some(fill_color) = &cell.fill_color {
            let x = cell.box_units.x - table.box_units.x;
            let y = cell.box_units.y - table.box_units.y;
            let right = x.saturating_add(cell.box_units.width.max(1));
            let bottom = y.saturating_add(cell.box_units.height.max(1));
            let pattern_index = fill_patterns
                .iter()
                .position(|color| color == fill_color)
                .unwrap_or(0);
            let pattern_id = &pattern_ids[pattern_index];
            html.push_str(&format!(
                "<path fill=\"url(#{pattern_id})\" d=\"M{},{}L{},{}L{},{}L{},{}L{},{}Z \"></path>",
                coordinate(x),
                coordinate(y),
                coordinate(right),
                coordinate(y),
                coordinate(right),
                coordinate(bottom),
                coordinate(x),
                coordinate(bottom),
                coordinate(x),
                coordinate(y)
            ));
        } else if let Some(pattern_id) = gradient_pattern_id {
            let x = cell.box_units.x - table.box_units.x;
            let y = cell.box_units.y - table.box_units.y;
            let right = x.saturating_add(cell.box_units.width.max(1));
            let bottom = y.saturating_add(cell.box_units.height.max(1));
            html.push_str(&format!(
                "<path fill=\"url(#{pattern_id})\" d=\"M{},{}L{},{}L{},{}L{},{}L{},{}Z \"></path>",
                coordinate(x),
                coordinate(y),
                coordinate(right),
                coordinate(y),
                coordinate(right),
                coordinate(bottom),
                coordinate(x),
                coordinate(bottom),
                coordinate(x),
                coordinate(y)
            ));
        }
        let x = cell.box_units.x - table.box_units.x;
        let y = cell.box_units.y - table.box_units.y;
        let right = x.saturating_add(cell.box_units.width.max(1));
        let bottom = y.saturating_add(cell.box_units.height.max(1));
        if let Some(stroke) = &cell.diagonal_stroke {
            if cell.diagonal_backward {
                append_table_line(html, (x, y), (right, bottom), cell.diagonal_width, stroke);
            }
            if cell.diagonal_forward {
                append_table_line(html, (x, bottom), (right, y), cell.diagonal_width, stroke);
            }
        }
        // vertical_border_extra() extends only the vertical segment's
        // own endpoint -- not the fill rect (drawn above from its own
        // `bottom`) or the horizontal segment keys.
        if cell.border_left_width > 0 {
            let extra = vertical_border_extra(cell.border_left_width, &cell.border_strokes[0]);
            vertical_segments.entry(x).or_default().push((
                y,
                bottom.saturating_add(extra),
                cell.border_left_width,
                cell.border_strokes[0].clone(),
            ));
        }
        if cell.border_right_width > 0 {
            let extra = vertical_border_extra(cell.border_right_width, &cell.border_strokes[1]);
            vertical_segments.entry(right).or_default().push((
                y,
                bottom.saturating_add(extra),
                cell.border_right_width,
                cell.border_strokes[1].clone(),
            ));
        }
        if cell.border_top_width > 0 {
            horizontal_segments.entry(y).or_default().push((
                x,
                right,
                cell.border_top_width,
                cell.border_strokes[2].clone(),
            ));
        }
        if cell.border_bottom_width > 0 {
            horizontal_segments.entry(bottom).or_default().push((
                x,
                right,
                cell.border_bottom_width,
                cell.border_strokes[3].clone(),
            ));
        }
    }
    // The reference extends a horizontal border past its endpoint by half
    // of the *vertical* border it meets there, not by half of its own
    // width. Keep the merged verticals so each horizontal can look up the
    // stroke that actually paints its corner.
    let mut corner_verticals = Vec::<(i64, i64, i64, i64)>::new();
    for (x, mut segments) in vertical_segments {
        for (start, end, width, stroke) in merge_svg_segments(&mut segments) {
            corner_verticals.push((x, start, end, width));
            append_table_line(html, (x, start), (x, end), width, &stroke);
        }
    }
    let mut boundary_segments = BTreeMap::<i64, Vec<(i64, i64, i64, BorderStroke)>>::new();
    for (y, mut segments) in horizontal_segments {
        let merged = merge_svg_segments(&mut segments);
        for (start, end, width, stroke) in &merged {
            let left = corner_width_at(&corner_verticals, *start, y).unwrap_or(*width);
            let right = corner_width_at(&corner_verticals, *end, y).unwrap_or(*width);
            append_table_line(
                html,
                (horizontal_start(*start, *width, left, stroke), y),
                (horizontal_end(*end, *width, right, stroke), y),
                *width,
                stroke,
            );
        }
        if y == 0 || y == table_height {
            boundary_segments.insert(y, merged);
        }
    }
    if let Some(segments) = vertical_segments_for_boundary(table, true) {
        for (start, end, width, stroke) in segments {
            append_table_line(
                html,
                (table_width, start),
                (table_width, end),
                width,
                &stroke,
            );
        }
    }
    if let Some(segments) = vertical_segments_for_boundary(table, false) {
        for (start, end, width, stroke) in segments {
            append_table_line(html, (0, start), (0, end), width, &stroke);
        }
    }
    for y in [table_height, 0] {
        if let Some(segments) = boundary_segments.get(&y) {
            for (start, end, width, stroke) in segments {
                let left = corner_width_at(&corner_verticals, *start, y).unwrap_or(*width);
                let right = corner_width_at(&corner_verticals, *end, y).unwrap_or(*width);
                append_table_line(
                    html,
                    (horizontal_start(*start, *width, left, stroke), y),
                    (horizontal_end(*end, *width, right, stroke), y),
                    *width,
                    stroke,
                );
            }
        }
    }
}

/// A table written as a plain box of cells (`div.hce` in its `htb`), since
/// it carries no grid a reader needs: a single source cell (a frame around
/// a title or a note), or a whole table with nothing in any cell (a drawn
/// grid). Written as `table` either would read as a one-cell or empty
/// table. The source table stays on record: `data-hwpx-table` on the `htb`
/// gives its rows x columns, and each cell of a larger one its place.
pub(super) fn box_table(table: &Table) -> bool {
    let single = table.rows == 1 && table.columns == 1;
    let whole = table
        .fragment_rows
        .is_none_or(|(first, last)| first == 0 && last + 1 >= table.rows);
    single || (whole && !table.cells.iter().any(cell_has_content))
}

pub(super) fn cell_has_content(cell: &TableCell) -> bool {
    !cell.tables.is_empty()
        || cell.paragraphs.iter().any(|paragraph| {
            !paragraph.objects.is_empty()
                || paragraph
                    .tokens
                    .iter()
                    .any(|token| !token.visible_text().trim().is_empty())
        })
}

/// The cells of a [`box_table`], in the order `render_cells` writes them.
fn render_box_cells(
    html: &mut String,
    table: &Table,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let rows = super::semantic::plan_rows(table);
    let single = table.rows == 1 && table.columns == 1;
    for row in rows.head.iter().chain(&rows.body) {
        for slot in &row.slots {
            let super::semantic::Slot::Cell { cell, row_span } = slot else {
                continue;
            };
            let mut attributes = String::new();
            if cell.repeated_header {
                attributes.push_str(" data-hwpx-repeat");
            } else {
                attributes.push_str(&super::semantic::part_marker('c', &cell.id));
            }
            if !single {
                attributes.push_str(&format!(
                    " data-hwpx-cell=\"{},{},{},{}\"",
                    cell.row, cell.column, row_span, cell.col_span
                ));
            }
            if cell.is_header {
                attributes.push_str(" data-hwpx-header");
            }
            render_cell(
                html,
                table,
                cell,
                "div",
                &attributes,
                document,
                options,
                patterns,
                gradients,
            );
        }
    }
}

/// The cells of `table` as `table`/`thead`/`tbody`/`tr`/`th`/`td` (D20):
/// absolutely positioned cell boxes inside the `htb`, carried by table
/// elements whose rows follow the source grid.
fn render_cells(
    html: &mut String,
    table: &Table,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let rows = super::semantic::plan_rows(table);
    html.push_str("<table");
    html.push_str(&super::semantic::part_marker('t', &table.id));
    html.push('>');
    for (group, rows) in [("thead", &rows.head), ("tbody", &rows.body)] {
        if group == "thead" && rows.is_empty() {
            continue;
        }
        html.push('<');
        html.push_str(group);
        html.push('>');
        for row in rows {
            html.push_str(if row.repeated {
                "<tr data-hwpx-repeat>"
            } else {
                "<tr>"
            });
            for slot in &row.slots {
                let super::semantic::Slot::Cell { cell, row_span } = slot else {
                    html.push_str("<td hidden></td>");
                    continue;
                };
                let mut attributes = String::new();
                // A repeated header copy is a second view, not a part.
                if !cell.repeated_header {
                    attributes.push_str(&super::semantic::part_marker('c', &cell.id));
                }
                if *row_span > 1 {
                    attributes.push_str(&format!(" rowspan=\"{row_span}\""));
                }
                if cell.col_span > 1 {
                    attributes.push_str(&format!(" colspan=\"{}\"", cell.col_span));
                }
                let tag = if cell.is_header { "th" } else { "td" };
                render_cell(
                    html,
                    table,
                    cell,
                    tag,
                    &attributes,
                    document,
                    options,
                    patterns,
                    gradients,
                );
            }
            html.push_str("</tr>");
        }
        html.push_str("</");
        html.push_str(group);
        html.push('>');
    }
    html.push_str("</table>");
}

/// One cell box, as `tag` (`td`, `th`, or `div` in a box) with extra
/// `attributes`.
#[allow(clippy::too_many_arguments)]
fn render_cell(
    html: &mut String,
    table: &Table,
    cell: &TableCell,
    tag: &str,
    attributes: &str,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let cell_margin_left = cell.margin_left;
    let cell_margin_top = cell.margin_top;
    html.push('<');
    html.push_str(tag);
    html.push_str(" class=\"hce\"");
    html.push_str(attributes);
    html.push_str(" style=\"left:");
    html.push_str(&css_mm(cell.box_units.x - table.box_units.x));
    html.push_str(";top:");
    html.push_str(&css_mm(cell.box_units.y - table.box_units.y));
    html.push_str(";width:");
    html.push_str(&css_mm(cell.box_units.width));
    html.push_str(";height:");
    html.push_str(&css_mm(cell.box_units.height));
    // A continuation piece whose source lines all belong to another
    // fragment is an empty cell box in the reference, without the
    // `hcD > hcI` wrappers (성과보고서 has 733 of them).
    if cell.paragraphs.is_empty() && cell.tables.is_empty() {
        html.push_str(";\"></");
        html.push_str(tag);
        html.push('>');
        return;
    }
    html.push_str(";\"><div class=\"hcD\" style=\"left:");
    html.push_str(&css_mm(cell_margin_left));
    html.push_str(";top:");
    html.push_str(&css_mm(cell_margin_top));
    html.push_str(";\"><div class=\"hcI\"");
    let vertical_offset = presentation::cell_vertical_offset(table, cell);
    if vertical_offset > 0 {
        html.push_str(" style=\"top:");
        html.push_str(&css_mm(vertical_offset));
        html.push_str(";\"");
    }
    html.push('>');
    let use_metric_offset = table.page_break.eq_ignore_ascii_case("CELL");
    let mut rendered_inline_tables = std::collections::HashSet::new();
    // A floating (non-treatAsChar) object anchored to a cell paragraph is
    // not part of that paragraph's own text flow, so it is rendered after
    // `hcI`/`hcD` close, as a direct `hce` child positioned from the
    // cell's own margin/vertical-alignment origin -- matching the
    // reference exporter's `hce > hsR` structure for every floating
    // object observed inside a table cell. Rendering it inside `hcI`
    // instead double-applies that div's own vertical-centering offset.
    let mut floating_objects = Vec::new();
    let mut floating_tables = Vec::new();
    let mut paragraphs = ParagraphGroup::new(document, false);
    if cell.repeated_header {
        paragraphs = paragraphs.unlinked();
    } else if options.infer_structure && super::semantic::title_paragraph(table).is_some() {
        paragraphs = paragraphs.title();
    }
    for paragraph in &cell.paragraphs {
        // Cell paragraphs are rendered from their source lines using the same
        // line serializer. Their local coordinates are already relative to the cell.
        let inline_tables = cell
            .tables
            .iter()
            .filter(|table| table.anchor.treat_as_char)
            .collect::<Vec<_>>();
        for line in crate::layout::paragraph_fragments_for_render_with_metric_offset(
            paragraph,
            use_metric_offset,
        ) {
            let phrasing = !carries_block_content(&line, &inline_tables);
            paragraphs.line(html, document, &line, phrasing, |html| {
                render_line(
                    html,
                    &line,
                    document,
                    options,
                    &inline_tables,
                    patterns,
                    gradients,
                )
            });
            if let Some(nested) = inline_tables
                .iter()
                .find(|nested| nested.anchor_y == Some(line.top))
            {
                rendered_inline_tables.insert(nested.id.as_str());
            }
        }
        for object in &paragraph.objects {
            if !object.anchor.treat_as_char {
                let mut shifted = object.clone();
                shifted.box_units.x = shifted.box_units.x.saturating_add(cell_margin_left);
                shifted.box_units.y = shifted
                    .box_units
                    .y
                    .saturating_add(cell_margin_top)
                    .saturating_add(vertical_offset);
                floating_objects.push(shifted);
            }
        }
    }
    paragraphs.close(html);
    // Do not emit a second copy of an inline table consumed by its line.
    // Keep the existing fallback when cropping leaves no matching anchor.
    for nested in cell
        .tables
        .iter()
        .filter(|table| !rendered_inline_tables.contains(table.id.as_str()))
    {
        if nested.anchor.treat_as_char {
            render_table(
                html,
                nested,
                document,
                options,
                TablePlacement::Nested,
                patterns,
                gradients,
            );
        } else {
            let mut shifted = (*nested).clone();
            crate::layout::translate_table(
                &mut shifted,
                cell_margin_left,
                cell_margin_top.saturating_add(vertical_offset),
            );
            floating_tables.push(shifted);
        }
    }
    html.push_str("</div></div>");
    for nested in &floating_tables {
        render_table(
            html,
            nested,
            document,
            options,
            TablePlacement::Nested,
            patterns,
            gradients,
        );
    }
    for object in &floating_objects {
        render_object(html, object, document, options, patterns, gradients);
    }
    html.push_str("</");
    html.push_str(tag);
    html.push('>');
}

fn rgb_components(value: &str) -> (u8, u8, u8) {
    if value.len() == 7 && value.starts_with('#') {
        let red = u8::from_str_radix(&value[1..3], 16).ok();
        let green = u8::from_str_radix(&value[3..5], 16).ok();
        let blue = u8::from_str_radix(&value[5..7], 16).ok();
        if let (Some(red), Some(green), Some(blue)) = (red, green, blue) {
            return (red, green, blue);
        }
    }
    (0, 0, 0)
}

fn merge_svg_segments(
    segments: &mut Vec<(i64, i64, i64, BorderStroke)>,
) -> Vec<(i64, i64, i64, BorderStroke)> {
    // Equal-width opposite edges can interleave solid and dashed paints.
    // Mixed-width edges retain their source coordinate splits: the exporter
    // emits separate overlap segments where the stroke thickness changes.
    let uniform_width = segments
        .first()
        .is_none_or(|first| segments.iter().all(|segment| segment.2 == first.2));
    if uniform_width {
        segments.sort_unstable_by(|a, b| (&a.2, &a.3, &a.0, &a.1).cmp(&(&b.2, &b.3, &b.0, &b.1)));
    } else {
        segments.sort_unstable();
    }
    let mut merged = Vec::<(i64, i64, i64, BorderStroke)>::new();
    for (start, end, width, stroke) in segments.drain(..) {
        if let Some((_, current_end, current_width, current_stroke)) = merged.last_mut() {
            if width == *current_width && stroke == *current_stroke && start <= *current_end {
                *current_end = (*current_end).max(end);
                continue;
            }
        }
        merged.push((start, end, width, stroke));
    }
    let mut merged = yield_to_double_borders(merged);
    merged.sort_unstable();
    merged
}

/// One edge shows one border. Where a `DOUBLE_SLIM` border shares an edge
/// with a border no wider than itself, the double wins and the other is
/// cut back to the parts it alone covers; otherwise its stroke fills the
/// double's gap and the pair reads as one heavy line. 성과보고서 별첨6's
/// header rows end in a 0.7mm double over a 0.3mm solid (the next row's
/// top), and Hancom shows the double. rhwp's `merge_border` likewise keeps
/// one border per edge: the wider, and on a tie the double. Other mixes
/// keep both strokes: a wider solid already covers a narrower one.
fn yield_to_double_borders(
    segments: Vec<(i64, i64, i64, BorderStroke)>,
) -> Vec<(i64, i64, i64, BorderStroke)> {
    let doubles = segments
        .iter()
        .filter(|segment| segment.3.kind == "DOUBLE_SLIM")
        .map(|segment| (segment.0, segment.1, segment.2))
        .collect::<Vec<_>>();
    if doubles.is_empty() {
        return segments;
    }
    let mut result = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.3.kind == "DOUBLE_SLIM" {
            result.push(segment);
            continue;
        }
        let mut pieces = vec![(segment.0, segment.1)];
        for &(start, end, width) in &doubles {
            if width < segment.2 {
                continue;
            }
            let mut rest = Vec::with_capacity(pieces.len() + 1);
            for (from, to) in pieces {
                if to <= start || from >= end {
                    rest.push((from, to));
                    continue;
                }
                if from < start {
                    rest.push((from, start));
                }
                if end < to {
                    rest.push((end, to));
                }
            }
            pieces = rest;
        }
        for (start, end) in pieces {
            result.push((start, end, segment.2, segment.3.clone()));
        }
    }
    result
}

fn vertical_segments_for_boundary(
    table: &Table,
    right_edge: bool,
) -> Option<Vec<(i64, i64, i64, BorderStroke)>> {
    let expected = if right_edge {
        table.box_units.width.max(1)
    } else {
        0
    };
    let mut segments = Vec::new();
    for cell in &table.cells {
        let x = cell.box_units.x - table.box_units.x;
        let right = x.saturating_add(cell.box_units.width.max(1));
        if (right_edge && right == expected) || (!right_edge && x == expected) {
            let width = if right_edge {
                cell.border_right_width
            } else {
                cell.border_left_width
            };
            if width > 0 {
                let top = cell.box_units.y - table.box_units.y;
                let stroke = &cell.border_strokes[usize::from(right_edge)];
                // Same one-HWPUNIT vertical extension (skipped for the
                // round-cap CIRCLE pattern) as the main segment loop above;
                // this duplicate boundary path needs it too.
                let extra = vertical_border_extra(width, stroke);
                segments.push((
                    top,
                    top + cell.box_units.height.max(1) + extra,
                    width,
                    stroke.clone(),
                ));
            }
        }
    }
    if segments.is_empty() {
        None
    } else {
        Some(merge_svg_segments(&mut segments))
    }
}

fn has_reference_circle_pattern(width: i64, stroke: &BorderStroke) -> bool {
    // Verified HWPX CIRCLE 0.2 mm profile. Other widths retain the fallback
    // until their exporter metrics have an independent source counterpart.
    stroke.kind == "CIRCLE" && width == 57
}

/// A (butt-cap) vertical table border segment's endpoint lands one HWPUNIT
/// past the cell's own bottom edge in the reference; verified against five
/// independently declared cell heights in 샘플/루이지애나 변형 샘플 (see
/// AGENTS.md). The round-cap CIRCLE dot pattern already overshoots its own
/// endpoint visually and keeps landing exactly on the cell edge instead
/// (verified by the finance report's circle-border regression) -- applying
/// the extra HWPUNIT there as well would double it.
fn vertical_border_extra(width: i64, stroke: &BorderStroke) -> i64 {
    i64::from(!has_reference_circle_pattern(width, stroke))
}

/// Width of the vertical border that paints the corner at `x`, `y`. The
/// merged verticals are `(x, start, end, width)`; a corner counts only when
/// the segment actually reaches that row.
fn corner_width_at(verticals: &[(i64, i64, i64, i64)], x: i64, y: i64) -> Option<i64> {
    verticals
        .iter()
        .find(|(vx, start, end, _)| *vx == x && *start <= y && y <= *end)
        .map(|(_, _, _, width)| *width)
}

fn horizontal_start(start: i64, width: i64, corner_width: i64, stroke: &BorderStroke) -> i64 {
    if has_reference_circle_pattern(width, stroke) {
        return start;
    }
    start - corner_width / 2
}

fn horizontal_end(end: i64, width: i64, corner_width: i64, stroke: &BorderStroke) -> i64 {
    if has_reference_circle_pattern(width, stroke) {
        return end;
    }
    // The exporter keeps the right half of a stroke when converting the
    // positive boundary to its decimal SVG coordinate. Using the ceiling
    // here preserves that last HWPUNIT (e.g. 105.35 rather than 105.34mm),
    // while the left boundary intentionally retains the floor behavior.
    end + (corner_width + 1) / 2
}

/// One straight table border from `from` to `to` (HWPUNIT, table-local).
/// `DOUBLE_SLIM` is two strokes of a quarter of the declared width whose
/// outer edges span the whole width, centred 3/8 of it either side of the
/// line: the reference draws 0.7mm as 0.18mm strokes 0.53mm apart, 0.5mm
/// as 0.12mm 0.37mm apart and 0.12mm as 0.03mm 0.08mm apart (성과보고서,
/// 0103, 산업기술개발장비 공고). rhwp also draws a double border as two
/// parallel strokes inside the declared width.
fn append_table_line(
    html: &mut String,
    from: (i64, i64),
    to: (i64, i64),
    stroke_width: i64,
    stroke: &BorderStroke,
) {
    let path = |from: (i64, i64), to: (i64, i64)| {
        format!(
            "M{},{} L{},{}",
            svg_mm(from.0),
            svg_mm(from.1),
            svg_mm(to.0),
            svg_mm(to.1)
        )
    };
    if stroke.kind == "DOUBLE_SLIM" {
        let (dx, dy) = ((to.0 - from.0) as f64, (to.1 - from.1) as f64);
        let length = dx.hypot(dy);
        if length > 0.0 {
            let offset = stroke_width as f64 * 3.0 / 8.0;
            let (nx, ny) = (
                (-dy / length * offset).round() as i64,
                (dx / length * offset).round() as i64,
            );
            let solid = BorderStroke {
                kind: "SOLID".to_owned(),
                ..stroke.clone()
            };
            let line_width = crate::layout::round_div(i128::from(stroke_width), 4);
            for sign in [-1, 1] {
                append_table_path(
                    html,
                    path(
                        (from.0 + sign * nx, from.1 + sign * ny),
                        (to.0 + sign * nx, to.1 + sign * ny),
                    ),
                    line_width,
                    &solid,
                );
            }
            return;
        }
    }
    append_table_path(html, path(from, to), stroke_width, stroke);
}

fn append_table_path(html: &mut String, path: String, stroke_width: i64, stroke: &BorderStroke) {
    html.push_str("<path d=\"");
    html.push_str(&path);
    html.push_str("\" style=\"stroke:");
    html.push_str(crate::render::css::safe_color(&stroke.color));
    if has_reference_circle_pattern(stroke_width, stroke) {
        // The zero-length dash becomes a circular dot under a round cap.
        // Keep the path endpoints at the cell boundary, not half a stroke out.
        html.push_str(";stroke-linecap:round;stroke-dasharray:0,0.89;stroke-width:0.30;\"></path>");
        return;
    }
    html.push_str(";stroke-linecap:butt;");
    if stroke.kind == "DASH" {
        html.push_str("stroke-dasharray:");
        if let Some(pattern) = reference_dash_pattern(stroke_width) {
            html.push_str(pattern);
        } else {
            html.push_str(&svg_mm(crate::layout::round_div(
                i128::from(stroke_width) * 3,
                2,
            )));
            html.push(',');
            html.push_str(&svg_mm(crate::layout::round_div(
                i128::from(stroke_width) * 11,
                5,
            )));
        }
        html.push(';');
    }
    html.push_str("stroke-width:");
    html.push_str(&svg_mm(stroke_width));
    html.push_str(";\"></path>");
}

// Widths are the parser's HWPUNIT representation of HWPX millimetre labels.
// These six profiles are verified against reference SVG source. Nominal width
// labels do not reproduce dash metrics by a single floating-point multiplier.
// Keep unverified widths on the existing fallback rather than extrapolating.
fn reference_dash_pattern(width: i64) -> Option<&'static str> {
    match width {
        28 => Some("0.15,0.22"),  // 0.1 mm
        34 => Some("0.17,0.26"),  // 0.12 mm
        43 => Some("0.22,0.33"),  // 0.15 mm
        71 => Some("0.37,0.54"),  // 0.25 mm
        113 => Some("0.60,0.88"), // 0.4 mm
        170 => Some("0.89,1.31"), // 0.6 mm
        _ => None,
    }
}

fn svg_mm(units: i64) -> String {
    css_mm(units).trim_end_matches("mm").to_owned()
}

/// Observation attributes are inserted only after a paint fragment is
/// complete. Keep its typed content offsets in step with that insertion.
fn mark_direct_object(html: &mut String, start: usize, key: &str, document: &RenderContext<'_>) {
    let Some(open) = html[start..].find('<').map(|offset| start + offset) else {
        return;
    };
    let name_end = open
        + html[open..]
            .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
            .unwrap_or(1);
    let attribute = format!(" data-hwpx-id=\"{}\"", escape_html_attribute(key));
    html.insert_str(name_end, &attribute);
    for (offset, _) in document.object_slots.borrow_mut().iter_mut() {
        if *offset >= name_end {
            *offset += attribute.len();
        }
    }
}

pub(super) fn render_object(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let start = html.len();
    render_object_markup(html, object, document, options, patterns, gradients);
    if document.direct_observe.get() {
        mark_direct_object(html, start, &object.key, document);
    }
    #[cfg(test)]
    if document.observe_keys {
        super::observation::mark_object(html, start, &object.key);
    }
}

fn render_object_markup(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    if object.shape.is_some() || object.kind == "container" {
        render_graphic(html, object, document, options, false, patterns, gradients);
        return;
    }
    if object.kind == "pic" {
        if let Some(reference) = &object.binary_ref {
            if let Some(asset) = document_asset(document, reference) {
                if let Some(uri) = document.asset_url(asset) {
                    html.push_str("<div class=\"hsR\" style=\"top:");
                    html.push_str(&css_mm(object.box_units.y));
                    html.push_str(";left:");
                    html.push_str(&css_mm(object.box_units.x));
                    html.push_str(";width:");
                    html.push_str(&css_mm(object.box_units.width));
                    html.push_str(";height:");
                    html.push_str(&css_mm(object.box_units.height));
                    // The picture keeps its natural size unless a crop scales
                    // it; either way the box clips it.
                    html.push_str(";overflow:hidden;");
                    let image = picture_style(object_plan::picture_plan(object, false).fit);
                    if object.flip_x || object.flip_y {
                        html.push_str("transform:scale(");
                        html.push_str(if object.flip_x { "-1" } else { "1" });
                        html.push(',');
                        html.push_str(if object.flip_y { "-1" } else { "1" });
                        html.push_str(");");
                    }
                    html.push_str("\">");
                    push_image(html, uri, &object.description, &image);
                    html.push_str("</div>");
                    return;
                }
            }
        }
    }
    if object.kind == "equation" {
        // The reference rasterizes the equation to an external PNG and hangs
        // it off this box's background-image. A self-contained export cannot
        // carry that file, but the box itself is fully derivable and is what
        // the surrounding line flows around, so reproduce it exactly and
        // leave only the background-image out.
        //
        // Its declared `sz` gives the height verbatim (7.94/8.11/9.10mm in
        // the three sampled documents) while the width is that of `sz` plus
        // exactly 112 HWPUNIT. Eight equations across those documents, with
        // widths from 1480 to 36385, each narrow the delta to a 3-unit range;
        // the ranges intersect in {112}, and a constant (not proportional)
        // delta is what a 24x spread of widths agreeing implies.
        html.push_str("<div class=\"heq\" style=\"");
        html.push_str(&equation_box_style(object));
        html.push_str("\"></div>");
        return;
    }
    // What the report and `--strict` count as unsupported
    // (`PositionedObject::is_drawn`) is what `Skip` leaves out; a shapeless
    // line, rectangle or polygon is still drawn as the crude box below.
    if matches!(options.unsupported, UnsupportedPolicy::Skip) && !object.is_drawn(&document.assets)
    {
        return;
    }
    if matches!(object.kind.as_str(), "line" | "rect" | "polygon") {
        html.push_str("<svg class=\"hsR\" aria-hidden=\"true\" style=\"left:");
        html.push_str(&css_mm(object.box_units.x));
        html.push_str(";top:");
        html.push_str(&css_mm(object.box_units.y));
        html.push_str(";width:");
        html.push_str(&css_mm(object.box_units.width.max(1)));
        html.push_str(";height:");
        html.push_str(&css_mm(object.box_units.height.max(1)));
        html.push_str("\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">");
        if object.kind == "line" {
            html.push_str("<line x1=\"0\" y1=\"0\" x2=\"100\" y2=\"100\" stroke=\"#000\"/>");
        } else {
            html.push_str(
                "<rect x=\"1\" y=\"1\" width=\"98\" height=\"98\" fill=\"none\" stroke=\"#000\"/>",
            );
        }
        html.push_str("</svg>");
        return;
    }
    html.push_str("<div class=\"unsupported\" style=\"left:");
    html.push_str(&css_mm(object.box_units.x));
    html.push_str(";top:");
    html.push_str(&css_mm(object.box_units.y));
    html.push_str(";width:");
    html.push_str(&css_mm(object.box_units.width.max(1)));
    html.push_str(";height:");
    html.push_str(&css_mm(object.box_units.height.max(1)));
    html.push_str("\"><span class=\"unsupported-label\">");
    html.push_str(&escape_html(if object.alt.is_empty() {
        &object.kind
    } else {
        &object.alt
    }));
    html.push_str("</span></div>");
}

/// The style of the box an equation takes in its line (`heq`): its size and
/// its place against the line's baseline. Both writers use it, so the box
/// the line is laid out around is the same.
pub(super) fn equation_box_style(object: &PositionedObject) -> String {
    format!(
        "width:{};height:{};background-repeat:no-repeat;display:inline-block;position:relative;vertical-align:-15%;line-height:{};",
        css_mm(object.box_units.width + EQUATION_WIDTH_EXTRA),
        css_mm(object.box_units.height),
        css_mm(object.box_units.height)
    )
}

pub(super) fn render_inline_object(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let start = html.len();
    render_inline_object_markup(html, object, document, options, patterns, gradients);
    if document.direct_observe.get() {
        mark_direct_object(html, start, &object.key, document);
    }
    #[cfg(test)]
    if document.observe_keys {
        super::observation::mark_object(html, start, &object.key);
    }
}

fn render_inline_object_markup(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    if object.shape.is_some() || object.kind == "container" {
        render_graphic(html, object, document, options, true, patterns, gradients);
        return;
    }
    if object.kind != "pic" {
        render_object(html, object, document, options, patterns, gradients);
        return;
    }
    let Some(reference) = &object.binary_ref else {
        render_object(html, object, document, options, patterns, gradients);
        return;
    };
    let Some(asset) = document_asset(document, reference) else {
        render_object(html, object, document, options, patterns, gradients);
        return;
    };
    let Some(uri) = document.asset_url(asset) else {
        render_object(html, object, document, options, patterns, gradients);
        return;
    };
    // An inline picture is emitted inside its hls line as an hsR inline-block
    // in the reference exporter. Its local top/left are zero; the hls owns
    // the paragraph position.
    html.push_str(
        "<div class=\"hsR\" style=\"top:0mm;left:0mm;margin-bottom:0mm;margin-right:0mm;width:",
    );
    html.push_str(&css_mm(object.box_units.width));
    html.push_str(";height:");
    html.push_str(&css_mm(object.box_units.height));
    html.push_str(";display:inline-block;position:relative;vertical-align:-15%;line-height:");
    html.push_str(&css_mm(object.box_units.height));
    // When imgDim data is present and the imgClip region is a strict subset
    // of it (actual cropping), scale the picture so only the clipped region
    // fills the display box, which clips the rest; otherwise the picture
    // fills the box exactly (a picture whose imgDim is 0x0 included).
    let fit = object_plan::picture_plan(object, true).fit;
    html.push(';');
    if matches!(fit, object_plan::PictureFit::Frame(_)) {
        html.push_str("overflow:hidden;");
    }
    // `hp:flip` mirrors the *displayed* picture, independent of resampling
    // above -- 성과보고서 s6's "HMF00000_hd2.png" (`<hp:flip vertical="1">`)
    // rendered right-side up otherwise, matching `render_object`'s own
    // floating-picture case (its one other `transform:scale` use site).
    if object.flip_x || object.flip_y {
        html.push_str("transform:scale(");
        html.push_str(if object.flip_x { "-1" } else { "1" });
        html.push(',');
        html.push_str(if object.flip_y { "-1" } else { "1" });
        html.push_str(");");
    }
    html.push_str("\">");
    push_image(html, uri, &object.description, &picture_style(fit));
    html.push_str("</div>");
}

/// The style a picture's fit gives its `<img>`. The whole-box fit stays 100%
/// and is not rewritten as the box's lengths: the browser would round those
/// on their own.
fn picture_style(fit: object_plan::PictureFit) -> String {
    match fit {
        object_plan::PictureFit::FillBox => "left:0mm;top:0mm;width:100%;height:100%;".to_owned(),
        object_plan::PictureFit::Natural => "left:0mm;top:0mm;".to_owned(),
        object_plan::PictureFit::Frame(frame) => format!(
            "left:{};top:{};width:{};height:{};",
            css_mm(frame.left),
            css_mm(frame.top),
            css_mm(frame.width),
            css_mm(frame.height)
        ),
    }
}

/// A picture as an `<img>` at `style` inside its positioned box, so HTML
/// parsers, search engines and text converters see it (a CSS background is
/// invisible to them). The box keeps the geometry; the image replaces what
/// used to be the box's background. `alt` is the source's description, or
/// "그림" when the source has none (docs/DECISIONS.md).
fn push_image(html: &mut String, uri: &str, description: &str, style: &str) {
    html.push_str("<img class=\"hpi\" src=\"");
    html.push_str(uri);
    html.push_str("\" alt=\"");
    html.push_str(&escape_html_attribute(if description.is_empty() {
        "그림"
    } else {
        description
    }));
    html.push_str("\" style=\"");
    html.push_str(style);
    html.push_str("\">");
}

/// Straight-line points approximating one 90-degree corner of a rounded
/// rectangle: 30 segments (31 points, including both endpoints), landing
/// exactly on the analytic circle at every step. `corner` selects which of
/// the four quadrants this is, matching the reference's own path order
/// (bottom-left, top-left, top-right, bottom-right).
///
/// Verified point-for-point against 샘플/그라데이션 도형's ratio=20 corners
/// (radius 2.60mm): the reference's own polyline lands on this analytic
/// circle at every one of the 31 points in all four corners, not just the
/// ones spot-checked here.
fn quarter_circle_points(center_x: i64, center_y: i64, radius: i64, corner: u8) -> Vec<(i64, i64)> {
    const STEPS: i64 = 30;
    (0..=STEPS)
        .map(|i| {
            let theta = std::f64::consts::FRAC_PI_2 * (i as f64) / (STEPS as f64);
            let (dx, dy) = match corner {
                0 => (-theta.sin(), theta.cos()),  // bottom-left
                1 => (-theta.cos(), -theta.sin()), // top-left
                2 => (theta.sin(), -theta.cos()),  // top-right
                _ => (theta.cos(), theta.sin()),   // bottom-right
            };
            let x = center_x as f64 + radius as f64 * dx;
            let y = center_y as f64 + radius as f64 * dy;
            (x.round() as i64, y.round() as i64)
        })
        .collect()
}

/// The gradation box's own outline, spanning `inset..outline_width` and
/// `inset..outline_height` (the caller has already folded the inset into
/// both, so a plain shape with `inset == 0` spans `0..width`), then shifted
/// by `(offset_x, offset_y)` -- nonzero only for a container child sharing
/// another shape's `<svg>`, where the reference bakes each child's own
/// position directly into its path coordinates rather than wrapping it in
/// its own `<g transform>`. A plain rectangle when `radius` is zero, or
/// four quarter-circle corners (see [`quarter_circle_points`]) joined by
/// straight edges when it isn't. All the moves are `L`; nothing here is an
/// SVG arc command.
fn rounded_rect_outline(
    outline_width: i64,
    outline_height: i64,
    inset: i64,
    radius: i64,
    offset_x: i64,
    offset_y: i64,
) -> String {
    let (left, top) = (inset + offset_x, inset + offset_y);
    let (right, bottom) = (outline_width + offset_x, outline_height + offset_y);
    if radius <= 0 {
        return format!(
            "M{left},{top}L{right},{top}L{right},{bottom}L{left},{bottom}L{left},{top}Z ",
            left = svg_mm(left),
            top = svg_mm(top),
            right = svg_mm(right),
            bottom = svg_mm(bottom),
        );
    }
    let mut points = Vec::with_capacity(125);
    points.extend(quarter_circle_points(
        left + radius,
        bottom - radius,
        radius,
        0,
    ));
    points.extend(quarter_circle_points(
        left + radius,
        top + radius,
        radius,
        1,
    ));
    points.extend(quarter_circle_points(
        right - radius,
        top + radius,
        radius,
        2,
    ));
    points.extend(quarter_circle_points(
        right - radius,
        bottom - radius,
        radius,
        3,
    ));
    points.push(points[0]);
    let mut d = String::new();
    for (index, (x, y)) in points.iter().enumerate() {
        d.push_str(if index == 0 { "M" } else { "L" });
        d.push_str(&svg_mm(*x));
        d.push(',');
        d.push_str(&svg_mm(*y));
    }
    d.push_str("Z ");
    d
}

/// Paint a rectangle whose fill is a horizontal gradation the way the
/// reference does: an `hsR` container grown by the pen, an `svg` grown again
/// by [`SHAPE_SVG_MARGIN`], a `<pattern>` holding one path per colour band
/// mirrored about the shape's centre line, and the outline path filled from
/// that pattern.
///
/// A shape with its own `drawText` -- an `hp:subList`, even one holding no
/// visible text -- is drawn inset by half the pen so the outline's stroke
/// centreline lands on the declared box edge, and gets an `hsT` layer after
/// `</svg>` holding its own paragraph as `hcD > hcI > hls`, unconditionally
/// (never gated on whether that paragraph actually has a visible line). The
/// reference nests the `svg` in that `hsT`. A shape with no `drawText` at all -- 그라디언트
/// 샘플2's 13 shapes, none of which declare `hp:drawText` -- keeps the
/// plain `hsR > svg` layout with no inset, exactly as before. Verified
/// against every combination in 샘플/그라데이션 도형 (inline x floating,
/// textbox x plain, rounded x square corners, 8 shapes): the inline/floating
/// choice only changes `hsR`'s own style, the text overlay's offsets are the
/// same whether the paragraph is empty or not, and a rounded corner redraws
/// via [`rounded_rect_outline`] independently of everything else.
///
/// Returns false for anything outside the verified case -- a rotated
/// gradation or a non-rectangular outline -- leaving the CSS approximation
/// in place for those.
fn render_gradient_box(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    inline: bool,
    gradients: &mut GradientIds,
) -> bool {
    let Some(shape) = &object.shape else {
        return false;
    };
    let width = object.box_units.width.max(1);
    let height = object.box_units.height.max(1);
    if !object_plan::gradient_drawn(object) {
        return false;
    }
    let plan = object_plan::TextBoxPlan::new(object_plan::TextBoxLayout::Gradient, shape, height);
    // An odd pen width loses its last unit here: the reference grows the box
    // by two truncated half-widths, not by the pen itself.
    let pen = plan.pen;
    let grown = |extent: i64| extent + (pen / 2) * 2;
    let has_text_box = !shape.paragraphs.is_empty();
    // A textbox shape's outline and bands are drawn on a grid inset by half
    // the pen from the SVG's own origin (so the stroke centreline lands on
    // the declared box edge). A plain shape (no `drawText` at all, like
    // 그라디언트 샘플2's 13 shapes) keeps inset at 0.
    let inset = if has_text_box { pen / 2 } else { 0 };
    // The viewBox spans exactly the `svg` element's own box, so one unit is
    // one unit and the shape keeps its declared box (D31). The reference
    // grows a textbox's viewBox by a doubled margin but not its box; the
    // default `xMidYMid meet` then shrinks the shape about its centre (a
    // 11mm-tall box by about 2.6%, 성과보고서's chapter title bars).
    let outset = |extent: i64| grown(extent) + SHAPE_SVG_MARGIN * 2;
    let outline_width = width + inset;
    let outline_height = height + inset;
    let bands = gradient_band_colors(shape.gradient_step, &shape.gradient[0], &shape.gradient[1]);
    // The band grid itself is always laid out on the plain (uninset)
    // height -- inset does not rescale the proportional spread of bands,
    // it just slides the whole grid by a constant afterward: `+inset` for
    // the far (`mirrored == false`) half, `-inset` for the near
    // (`mirrored == true`) half, mirrored likewise on the x-axis below. A
    // plain shape keeps inset at 0, collapsing both to the single
    // already-verified grid.
    let angle = shape.gradient_angle;
    let edges = gradient_band_edges(band_extent(angle, width, height), shape.gradient_step);
    let id = gradients.allocate();
    if inline {
        html.push_str(&format!(
            "<div class=\"hsR\" style=\"top:0mm;margin-bottom:0mm;left:0mm;margin-right:0mm;width:{};height:{};display:inline-block;position:relative;vertical-align:-15%;line-height:{};\">",
            css_mm(grown(width)),
            css_mm(grown(height)),
            css_mm(grown(height)),
        ));
    } else {
        html.push_str(&format!(
            "<div class=\"hsR\" style=\"top:{};left:{};width:{};height:{};\">",
            css_mm(object.box_units.y),
            css_mm(object.box_units.x),
            css_mm(grown(width)),
            css_mm(grown(height)),
        ));
    }
    // A textbox's `svg` comes before its `hsT`, where the reference nests
    // it, at the same place (`svg` user 0 is the `hsT` origin). The stroke
    // reaches exactly the `hsT` box, and Chromium snaps that box's
    // `overflow:hidden` clip to device pixels, which shaved sub-pixel pens
    // off 그라데이션 도형's outlines.
    html.push_str(&format!(
        "<svg class=\"hs\" aria-hidden=\"true\" viewBox=\"-{} -{} {} {}\" style=\"left:-{};top:-{};width:{};height:{};\">\
         <defs><pattern id=\"{id}\" width=\"100%\" height=\"100%\" patternUnits=\"userSpaceOnUse\">",
        svg_mm(SHAPE_SVG_MARGIN),
        svg_mm(SHAPE_SVG_MARGIN),
        svg_mm(outset(width)),
        svg_mm(outset(height)),
        css_mm(inset + SHAPE_SVG_MARGIN),
        css_mm(inset + SHAPE_SVG_MARGIN),
        css_mm(outset(width)),
        css_mm(outset(height)),
    ));
    // The far (`mirrored == false`) x is `width + inset`; the near
    // (`mirrored == true`) x is `-(width - inset)`. Both stay fixed across
    // every band and both mirrored copies -- unlike the y edges, the x
    // extent doesn't itself flip between the two copies.
    let side_far = svg_mm(width + inset);
    let side_near = svg_mm(-(width - inset));
    for (index, color) in bands.iter().enumerate() {
        let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
        if angle != 0 {
            for path in angled_band_paths(angle, (width, height), inset, (0, 0), &edges, index) {
                html.push_str(&format!(
                    "<path d=\"{path}\" style=\"fill:{hex};stroke:{hex};stroke-width:1;\"></path>"
                ));
            }
            continue;
        }
        for mirrored in [false, true] {
            // The innermost band ends on the centre line, which the
            // reference writes as a plain `0` -- never `-0`.
            let (far, near) = if mirrored {
                (
                    svg_mm(-(edges[index] - inset)),
                    svg_mm(-(edges[index + 1] - inset)),
                )
            } else {
                (
                    svg_mm(edges[index] + inset),
                    svg_mm(edges[index + 1] + inset),
                )
            };
            html.push_str(&format!(
                "<path d=\"M{side_far},{far}L{side_near},{far}L{side_near},{near}L{side_far},{near}Z \" \
                 style=\"fill:{hex};stroke:{hex};stroke-width:1;\"></path>",
            ));
        }
    }
    let radius = width.min(height) * shape.corner_ratio.min(50) / 100;
    let outline = rounded_rect_outline(outline_width, outline_height, inset, radius, 0, 0);
    html.push_str(&format!(
        "</pattern></defs><path fill=\"url(#{id})\" d=\"{outline}\" \
         style=\"stroke:{stroke};stroke-linecap:butt;stroke-width:{pen_mm};\"></path></svg>",
        stroke = super::css::safe_color(&shape.line_color),
        pen_mm = svg_mm(pen),
    ));
    if has_text_box {
        // Fixed relative to the shape's own margins and pen, never to the
        // actual rendered content -- 샘플/그라데이션 도형's textless variants
        // get exactly the same hcD/hcI offsets as the ones holding "표1" or
        // "목    차".
        html.push_str(&format!(
            "<div class=\"hsT\" style=\"left:-{};top:-{};width:{};height:{};\">",
            css_mm(inset),
            css_mm(inset),
            css_mm(grown(width)),
            css_mm(grown(height)),
        ));
        let mut patterns = PatternIds {
            page: 0,
            next: 0,
            colors: Vec::new(),
        };
        render_text_in_origin(
            html,
            shape,
            &plan,
            document,
            options,
            &mut patterns,
            gradients,
        );
        render_text_box_tables(
            html,
            shape,
            plan.table_offset(),
            document,
            options,
            &mut patterns,
            gradients,
        );
        html.push_str("</div>");
    }
    html.push_str("</div>");
    true
}

/// Emit `<defs><pattern>...</pattern></defs><path fill="url(#...)" d="...">`
/// for one shape's fill and outline at the given local width/height/inset,
/// shifted to `(offset_x, offset_y)` -- the coordinate frame this ends up
/// in (a lone shape's own small `<svg>`, or a shared `<svg>` holding
/// several textless container children) is entirely the caller's concern.
/// A gradient fill draws its own band paths exactly as
/// [`render_gradient_box`] does; a solid fill reuses an existing `w_N`
/// pattern for the same colour on this page rather than redefining it
/// (verified across 샘플/여러 색채움 도형을 묶은 컨테이너's three
/// containers, whose repeated colours share one earlier `<defs>`).
/// Returns false if `shape` has neither an `angle=0` two-stop gradient nor
/// a plain fill colour, leaving nothing drawn.
/// A shape's position and size in whatever local coordinate frame it's
/// being drawn into -- its own small `<svg>` (where `x == y == 0`), or a
/// container's shared `<svg>` (where they're the child's offset from the
/// container's own origin).
#[derive(Clone, Copy)]
struct LocalBox {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

fn render_shape_fill_and_outline(
    html: &mut String,
    shape: &crate::model::ShapeStyle,
    geometry: LocalBox,
    inset: i64,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
    point_origin: (i64, i64),
) -> bool {
    let LocalBox {
        x: offset_x,
        y: offset_y,
        width,
        height,
    } = geometry;
    let pen = shape.declared_line_width.max(0);
    let radius = width.min(height) * shape.corner_ratio.min(50) / 100;
    let outline_width = width + inset;
    let outline_height = height + inset;
    let outline = if shape.points.len() >= 3 {
        polygon_outline(&shape.points, point_origin)
    } else {
        rounded_rect_outline(
            outline_width,
            outline_height,
            inset,
            radius,
            offset_x,
            offset_y,
        )
    };
    let stroke = (shape.line_width > 0).then(|| {
        format!(
            " style=\"stroke:{};stroke-linecap:butt;stroke-width:{};\"",
            super::css::safe_color(&shape.line_color),
            svg_mm(pen),
        )
    });
    if shape.gradient.len() == 2
        && band_angle_supported(shape.gradient_angle)
        && shape.gradient_step > 0
    {
        let angle = shape.gradient_angle;
        let bands =
            gradient_band_colors(shape.gradient_step, &shape.gradient[0], &shape.gradient[1]);
        let edges = gradient_band_edges(band_extent(angle, width, height), shape.gradient_step);
        if bands.is_empty() || edges.len() != bands.len() + 1 {
            return false;
        }
        let id = gradients.allocate();
        html.push_str(&format!(
            "<defs><pattern id=\"{id}\" width=\"100%\" height=\"100%\" patternUnits=\"userSpaceOnUse\">"
        ));
        let side_far = svg_mm(width + inset + offset_x);
        let side_near = svg_mm(offset_x - (width - inset));
        for (index, color) in bands.iter().enumerate() {
            let hex = format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]);
            if angle != 0 {
                for path in angled_band_paths(
                    angle,
                    (width, height),
                    inset,
                    (offset_x, offset_y),
                    &edges,
                    index,
                ) {
                    html.push_str(&format!(
                        "<path d=\"{path}\" style=\"fill:{hex};stroke:{hex};stroke-width:1;\"></path>"
                    ));
                }
                continue;
            }
            for mirrored in [false, true] {
                let (far, near) = if mirrored {
                    (
                        svg_mm(offset_y - (edges[index] - inset)),
                        svg_mm(offset_y - (edges[index + 1] - inset)),
                    )
                } else {
                    (
                        svg_mm(edges[index] + inset + offset_y),
                        svg_mm(edges[index + 1] + inset + offset_y),
                    )
                };
                html.push_str(&format!(
                    "<path d=\"M{side_far},{far}L{side_near},{far}L{side_near},{near}L{side_far},{near}Z \" \
                     style=\"fill:{hex};stroke:{hex};stroke-width:1;\"></path>",
                ));
            }
        }
        html.push_str("</pattern></defs>");
        html.push_str(&format!(
            "<path fill=\"url(#{id})\" d=\"{outline}\"{}></path>",
            stroke.as_deref().unwrap_or(""),
        ));
        true
    } else if let Some(fill) = shape.fill.as_ref().or_else(|| shape.gradient.first()) {
        // A gradation without band geometry (another angle, or an unreadable
        // one) is approximated by its first colour rather than left unpainted.
        let (id, is_new) = patterns.allocate_for_color(fill);
        if is_new {
            let (red, green, blue) = rgb_components(fill);
            html.push_str(&format!(
                "<defs><pattern id=\"{id}\" width=\"10\" height=\"10\" patternUnits=\"userSpaceOnUse\"><rect width=\"10\" height=\"10\" fill=\"rgb({red},{green},{blue})\"/></pattern></defs>"
            ));
        }
        html.push_str(&format!(
            "<path fill=\"url(#{id})\" d=\"{outline}\"{}></path>",
            stroke.as_deref().unwrap_or(""),
        ));
        true
    } else {
        false
    }
}

/// The two paths of band `index` of a gradation at 90 or 45 degrees, the band
/// and its mirror image, in the order the reference writes them. As at 0
/// degrees the bands sit about a centre line through the shape's top-left
/// corner and are mirrored over it, so both copies are written although only
/// one lands in the shape; the `edges` (`gradient_band_edges` over
/// `band_extent`) run from the far edge back to that line, and `inset` and
/// `offset` move the whole grid (half the pen, a container child's place).
///
/// * 90: vertical bands, `x = +-edge` across the width; each path spans the
///   height on either side of the line (`+-height`). The mirror image
///   (negative `x`) comes first.
/// * 45: diagonal bands `y - x = +-d`. Each is a parallelogram whose long
///   sides are `width + height` long along the diagonal, centred on the
///   anti-diagonal through the grid's origin; the mirror image swaps `x` and
///   `y`. Read off 샘플/그라디언트 샘플's three 45-degree boxes: the first and
///   last band of each agree to the hundredth of a millimetre printed.
fn angled_band_paths(
    angle: i64,
    (width, height): (i64, i64),
    inset: i64,
    (offset_x, offset_y): (i64, i64),
    edges: &[i64],
    index: usize,
) -> [String; 2] {
    let (far, near) = (edges[index], edges[index + 1]);
    if angle == 90 {
        let side_far = svg_mm(height + inset + offset_y);
        let side_near = svg_mm(offset_y - (height - inset));
        let band = |far: i64, near: i64| {
            let (far, near) = (svg_mm(far), svg_mm(near));
            format!("M{far},{side_far}L{far},{side_near}L{near},{side_near}L{near},{side_far}Z ")
        };
        return [
            band(offset_x - (far - inset), offset_x - (near - inset)),
            band(far + inset + offset_x, near + inset + offset_x),
        ];
    }
    // A band's end is half the diagonal's length along the diagonal and half
    // its distance across it from the origin; an odd length rounds to a whole
    // unit.
    let half = |value: i64| crate::layout::round_div(i128::from(value), 2);
    let length = width + height;
    let origin = (offset_x + inset, offset_y + inset);
    let vertex = |distance: i64, forward: bool, mirror: bool| {
        let across = half(distance);
        let along = half(if forward { length } else { -length });
        let (x, y) = (along - across, along + across);
        let (x, y) = if mirror { (y, x) } else { (x, y) };
        (svg_mm(origin.0 + x), svg_mm(origin.1 + y))
    };
    let band = |mirror: bool| {
        let first = vertex(far, true, mirror);
        let second = vertex(far, false, mirror);
        let third = vertex(near, false, mirror);
        let fourth = vertex(near, true, mirror);
        format!(
            "M{},{}L{},{}L{},{}L{},{}Z ",
            first.0, first.1, second.0, second.1, third.0, third.1, fourth.0, fourth.1
        )
    };
    [band(false), band(true)]
}

fn polygon_outline(points: &[(i64, i64)], origin: (i64, i64)) -> String {
    let mut d = String::new();
    for (index, (x, y)) in points.iter().enumerate() {
        d.push(if index == 0 { 'M' } else { 'L' });
        d.push_str(&svg_mm(x - origin.0));
        d.push(',');
        d.push_str(&svg_mm(y - origin.1));
    }
    d
}

/// A container child that carries its own `hp:drawText`: the same small
/// `svg` + `hsT > hcD > hcI > hls` shape [`render_gradient_box`] draws for
/// a lone textbox shape, just positioned at `geometry`'s offset within the
/// container instead of implicitly at its own origin. A polygon's points are
/// in the container's frame; `origin` is the child's own box in that frame,
/// so the outline lands in the child's `svg` like a rectangle's does.
#[allow(clippy::too_many_arguments)]
fn render_textbox_fill_child(
    html: &mut String,
    shape: &crate::model::ShapeStyle,
    polygon: bool,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    geometry: LocalBox,
    origin: (i64, i64),
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let LocalBox {
        x: local_x,
        y: local_y,
        width,
        height,
    } = geometry;
    let plan =
        object_plan::TextBoxPlan::new(object_plan::TextBoxLayout::SharedChild, shape, height);
    let inset = plan.inset();
    let grown = |extent: i64| extent + inset * 2;
    // The viewBox spans exactly the `svg` element's own box, and the `svg`
    // comes before the `hsT`/`hsP` rather than inside its clip (see
    // `render_gradient_box`).
    let outset = |extent: i64| grown(extent) + SHAPE_SVG_MARGIN * 2;
    html.push_str(&format!(
        "<svg class=\"hs\" aria-hidden=\"true\" viewBox=\"-{} -{} {} {}\" style=\"left:{};top:{};width:{};height:{};\">",
        svg_mm(SHAPE_SVG_MARGIN),
        svg_mm(SHAPE_SVG_MARGIN),
        svg_mm(outset(width)),
        svg_mm(outset(height)),
        css_mm(local_x - inset - SHAPE_SVG_MARGIN),
        css_mm(local_y - inset - SHAPE_SVG_MARGIN),
        css_mm(outset(width)),
        css_mm(outset(height)),
    ));
    let local_geometry = LocalBox {
        x: 0,
        y: 0,
        width,
        height,
    };
    render_shape_fill_and_outline(
        html,
        shape,
        local_geometry,
        inset,
        patterns,
        gradients,
        (origin.0 - inset, origin.1 - inset),
    );
    html.push_str("</svg>");
    html.push_str(&format!(
        "<div class=\"{}\" style=\"left:{};top:{};width:{};height:{};\">",
        if polygon { "hsP" } else { "hsT" },
        css_mm(local_x - inset),
        css_mm(local_y - inset),
        css_mm(grown(width)),
        css_mm(grown(height)),
    ));
    let mut line_patterns = PatternIds {
        page: 0,
        next: 0,
        colors: Vec::new(),
    };
    render_text_in_origin(
        html,
        shape,
        &plan,
        document,
        options,
        &mut line_patterns,
        gradients,
    );
    render_text_box_tables(
        html,
        shape,
        plan.table_offset(),
        document,
        options,
        patterns,
        gradients,
    );
    html.push_str("</div>");
}

/// Preserve each container child as a semantic owner while the SVG painter
/// keeps its shared drawing groups. These scopes add no coordinate frame.
fn object_child_scope(
    html: &mut String,
    tag: &str,
    object: &PositionedObject,
    document: &RenderContext<'_>,
) {
    if document.direct_objects.get() {
        let start = html.len();
        html.push_str(&format!("<{tag} data-hwpx-object>"));
        if document.direct_observe.get() {
            mark_direct_object(html, start, &object.key, document);
        }
    }
}

fn close_object_child_scope(html: &mut String, tag: &str, document: &RenderContext<'_>) {
    if document.direct_objects.get() {
        html.push_str(&format!("</{tag}>"));
    }
}

/// A container (`hp:container`) whose children are all plain rectangles
/// with a solid or `angle=0` gradient fill: the reference draws every
/// textless child's fill+outline as its own `<pattern>`+`<path>` inside one
/// shared `<svg>` sized to the container's own box, and breaks out into a
/// separate small `<svg>` + `hsT` (see
/// [`render_textbox_fill_child`]) for each child that carries its own
/// `hp:drawText`. A run of textless children shares one `<svg>`; a
/// text-carrying child always gets its own, and a later textless child (if
/// any) reopens a fresh shared `<svg>` rather than reusing the closed one.
///
/// Verified against every container in 샘플/여러 색채움 도형을 묶은
/// 컨테이너 (4 children mixing both kinds, one 2-textless-only container,
/// one 2-text-only container) and against the five *ungrouped* copies of
/// the same rects in that sample, which each get their own `hsR` with no
/// sharing at all -- the sharing is specific to children of one container,
/// not a general "adjacent same-kind shapes merge" rule.
///
/// Returns false (leaving the CSS approximation for the whole container)
/// for anything outside that -- no children, any child with its own
/// children or an unsupported fill.
fn render_container_fill_group(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    inline: bool,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) -> bool {
    if !object_plan::shared_fill_group(object) {
        return false;
    }
    let plans = object
        .children
        .iter()
        .map(|child| {
            let shape = child.shape.as_deref().expect("shared-fill child has shape");
            (child, shape, !shape.paragraphs.is_empty())
        })
        .collect();
    let plans = crate::semantic::order::reading_order(plans, |(child, shape, has_text_box)| {
        (
            child.box_units,
            shape.line_width.max(shape.declared_line_width),
            *has_text_box,
        )
    });

    let width = object.box_units.width.max(1);
    let height = object.box_units.height.max(1);
    if inline {
        html.push_str(&format!(
            "<div class=\"hsR\" style=\"top:0mm;margin-bottom:0mm;left:0mm;margin-right:0mm;width:{};height:{};display:inline-block;position:relative;vertical-align:-15%;line-height:{};\">",
            css_mm(width),
            css_mm(height),
            css_mm(height),
        ));
    } else {
        html.push_str(&format!(
            "<div class=\"hsR\" style=\"top:{};left:{};width:{};height:{};\">",
            css_mm(object.box_units.y),
            css_mm(object.box_units.x),
            css_mm(width),
            css_mm(height),
        ));
    }

    let mut shared_open = false;
    for (child, shape, has_text_box) in plans {
        let child_geometry = LocalBox {
            x: child.box_units.x - object.box_units.x,
            y: child.box_units.y - object.box_units.y,
            width: child.box_units.width.max(1),
            height: child.box_units.height.max(1),
        };
        if child.kind == "line" {
            // A line's points are in the container's frame, so it joins the
            // shared container-sized `svg` as the reference draws it. Its own
            // box can be zero wide or tall (a vertical or horizontal line),
            // and a zero-sized viewBox draws nothing.
            if !shared_open {
                open_shared_svg(html, width, height);
                shared_open = true;
            }
            object_child_scope(html, "g", child, document);
            #[cfg(test)]
            let child_start = html.len();
            let path = if shape.points.len() == 2 {
                format!(
                    "M{},{} L{},{}",
                    svg_mm(shape.points[0].0 - object.box_units.x),
                    svg_mm(shape.points[0].1 - object.box_units.y),
                    svg_mm(shape.points[1].0 - object.box_units.x),
                    svg_mm(shape.points[1].1 - object.box_units.y),
                )
            } else {
                format!(
                    "M{},{} L{},{}",
                    svg_mm(child_geometry.x),
                    svg_mm(child_geometry.y),
                    svg_mm(child_geometry.x + child_geometry.width),
                    svg_mm(child_geometry.y + child_geometry.height)
                )
            };
            html.push_str(&format!(
                "<path d=\"{}\" style=\"stroke:{};stroke-linecap:butt;stroke-width:{};\"></path>",
                path,
                super::css::safe_color(&shape.line_color),
                svg_mm(shape.line_width.max(1)),
            ));
            #[cfg(test)]
            if document.observe_keys {
                super::observation::mark_object(html, child_start, &child.key);
            }
            close_object_child_scope(html, "g", document);
        } else if has_text_box {
            if shared_open {
                html.push_str("</svg>");
                shared_open = false;
            }
            object_child_scope(html, "div", child, document);
            #[cfg(test)]
            let child_start = html.len();
            render_textbox_fill_child(
                html,
                shape,
                child.kind == "polygon",
                document,
                options,
                child_geometry,
                (child.box_units.x, child.box_units.y),
                patterns,
                gradients,
            );
            #[cfg(test)]
            if document.observe_keys {
                super::observation::mark_object(html, child_start, &child.key);
            }
            close_object_child_scope(html, "div", document);
        } else {
            if !shared_open {
                open_shared_svg(html, width, height);
                shared_open = true;
            }
            object_child_scope(html, "g", child, document);
            #[cfg(test)]
            let child_start = html.len();
            render_shape_fill_and_outline(
                html,
                shape,
                child_geometry,
                0,
                patterns,
                gradients,
                (object.box_units.x, object.box_units.y),
            );
            #[cfg(test)]
            if document.observe_keys {
                super::observation::mark_object(html, child_start, &child.key);
            }
            close_object_child_scope(html, "g", document);
        }
    }
    if shared_open {
        html.push_str("</svg>");
    }
    html.push_str("</div>");
    true
}

fn open_shared_svg(html: &mut String, width: i64, height: i64) {
    html.push_str(&format!(
        "<svg class=\"hs\" aria-hidden=\"true\" viewBox=\"-{} -{} {} {}\" style=\"left:-{};top:-{};width:{};height:{};\">",
        svg_mm(SHAPE_SVG_MARGIN),
        svg_mm(SHAPE_SVG_MARGIN),
        svg_mm(width + SHAPE_SVG_MARGIN * 2),
        svg_mm(height + SHAPE_SVG_MARGIN * 2),
        css_mm(SHAPE_SVG_MARGIN),
        css_mm(SHAPE_SVG_MARGIN),
        css_mm(width + SHAPE_SVG_MARGIN * 2),
        css_mm(height + SHAPE_SVG_MARGIN * 2),
    ));
}

/// A lone `hp:rect` filled with a picture (`hc:fillBrush/hc:imgBrush`,
/// commonly `numberingType="PICTURE"` -- a chart or photo dropped in as a
/// shape rather than an `hp:pic`, as in 성과보고서's per-goal pie/bar chart
/// grid). The reference draws it the same way a container's colour-filled
/// child is drawn (one `<pattern>` holding the fill, one `<path>` using it),
/// substituting an `<image>` for the colour; ours is an `<img>` over the
/// same area (see [`push_image`]). The outer box still reserves the pen's
/// space via `declared_line_width` even though these
/// charts' own `lineShape` is `style="NONE" alpha="0"` (invisible) -- the
/// same "reserved but unpainted" rule `ShapeStyle::declared_line_width`
/// already documents for a textbox's own inset, not the plain `line_width`
/// `render_gradient_box` uses for a visible-or-absent outline. Verified
/// against 성과보고서's own 35x30mm (9921x8504 HWPUNIT) chart images with a
/// declared pen of 28: growing by `(28/2)*2` before the usual
/// `SHAPE_SVG_MARGIN` reproduces the reference's `35.10mm`/`30.10mm` outer
/// box exactly, while the picture itself stays the plain, ungrown 35x30. No
/// sample so far combines this with a *visible* outline or caption text, so
/// both stay unhandled until one does.
fn render_image_fill_shape(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    inline: bool,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) -> bool {
    let Some(shape) = &object.shape else {
        return false;
    };
    if object.kind != "rect" || !object.children.is_empty() {
        return false;
    }
    let Some(reference) = &shape.image_fill else {
        return false;
    };
    let Some(asset) = document_asset(document, reference) else {
        return false;
    };
    let Some(uri) = document.asset_url(asset) else {
        return false;
    };
    let width = object.box_units.width.max(1);
    let height = object.box_units.height.max(1);
    let plan = object_plan::TextBoxPlan::new(object_plan::TextBoxLayout::ImageFill, shape, height);
    let pen = plan.pen;
    let grown = |extent: i64| extent + (pen / 2) * 2;
    if inline {
        html.push_str(&format!(
            "<div class=\"hsR\" style=\"top:0mm;margin-bottom:0mm;left:0mm;margin-right:0mm;width:{};height:{};display:inline-block;position:relative;vertical-align:-15%;line-height:{};\">",
            css_mm(grown(width)),
            css_mm(grown(height)),
            css_mm(grown(height)),
        ));
    } else {
        html.push_str(&format!(
            "<div class=\"hsR\" style=\"top:{};left:{};width:{};height:{};\">",
            css_mm(object.box_units.y),
            css_mm(object.box_units.x),
            css_mm(grown(width)),
            css_mm(grown(height)),
        ));
    }
    // The picture spans the plain, ungrown box from the wrapper's origin,
    // where the reference's pattern-filled path drew it. A rect with its own
    // text is a text box on a picture (성과보고서 별첨6's title: the "별첨6"
    // badge and the bar are the picture), so the picture is decoration.
    let has_text_box = !shape.paragraphs.is_empty() || !shape.tables.is_empty();
    let style = format!(
        "left:0mm;top:0mm;width:{};height:{};",
        css_mm(width),
        css_mm(height)
    );
    if has_text_box {
        html.push_str(&format!(
            "<img class=\"hpi\" src=\"{uri}\" alt=\"\" aria-hidden=\"true\" style=\"{style}\">"
        ));
    } else {
        push_image(html, uri, &object.description, &style);
    }
    if has_text_box {
        // Drawn over the picture as a gradient text box is (rhwp likewise
        // lays out the rect's text box after its image-fill node): the text
        // starts at the text margins from the declared box.
        let inset = plan.inset();
        html.push_str(&format!(
            "<div class=\"hsT\" style=\"left:-{};top:-{};width:{};height:{};\">",
            css_mm(inset),
            css_mm(inset),
            css_mm(grown(width)),
            css_mm(grown(height)),
        ));
        if !document.direct_objects.get() {
            // The page-by-page output's own bytes.
            html.push_str("             ");
        }
        render_text_in_origin(html, shape, &plan, document, options, patterns, gradients);
        render_text_box_tables(
            html,
            shape,
            plan.table_offset(),
            document,
            options,
            patterns,
            gradients,
        );
        html.push_str("</div>");
    }
    html.push_str("</div>");
    true
}

fn render_graphic(
    html: &mut String,
    object: &PositionedObject,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    inline: bool,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    let width = object.box_units.width.max(1);
    let height = object.box_units.height.max(1);
    if render_gradient_box(html, object, document, options, inline, gradients) {
        return;
    }
    if render_container_fill_group(html, object, document, options, inline, patterns, gradients) {
        return;
    }
    if render_image_fill_shape(html, object, document, options, inline, patterns, gradients) {
        return;
    }
    html.push_str("<div class=\"hsR\" style=\"box-sizing:border-box;");
    if inline {
        html.push_str(
            "display:inline-block;position:relative;vertical-align:-15%;left:0mm;top:0mm;",
        );
    } else {
        html.push_str(&format!(
            "position:absolute;left:{};top:{};",
            css_mm(object.box_units.x),
            css_mm(object.box_units.y)
        ));
    }
    html.push_str(&format!(
        "width:{};height:{};",
        css_mm(width),
        css_mm(height)
    ));
    if let Some(shape) = &object.shape {
        if let Some(fill) = &shape.fill {
            html.push_str(&format!(
                "background-color:{};",
                super::css::safe_color(fill)
            ));
        }
        if shape.gradient.len() >= 2 {
            let colors = shape
                .gradient
                .iter()
                .map(|color| super::css::safe_color(color))
                .collect::<Vec<_>>();
            html.push_str(&format!(
                "background-image:linear-gradient(to bottom,{});",
                colors.join(",")
            ));
        }
        if shape.line_width > 0 {
            html.push_str(&format!(
                "border:{} solid {};",
                css_mm(shape.line_width),
                super::css::safe_color(&shape.line_color)
            ));
        }
        if shape.corner_ratio > 0 {
            html.push_str(&format!(
                "border-radius:{};",
                css_mm(width.min(height).saturating_mul(shape.corner_ratio.min(50)) / 100)
            ));
        }
    }
    html.push_str("\">");
    let mut children = object.children.iter().collect::<Vec<_>>();
    if document.direct_objects.get() {
        children = crate::semantic::order::reading_order(children, |child| {
            let (pen, text) = child.shape.as_ref().map_or((0, false), |shape| {
                (
                    shape.line_width.max(shape.declared_line_width),
                    !shape.paragraphs.is_empty(),
                )
            });
            (child.box_units, pen, text)
        });
    }
    for child in children {
        render_object(html, child, document, options, patterns, gradients);
    }
    if let Some(shape) = &object.shape {
        let plan = object_plan::TextBoxPlan::new(object_plan::TextBoxLayout::Plain, shape, height);
        let mut patterns = PatternIds {
            page: 0,
            next: 0,
            colors: Vec::new(),
        };
        render_text_in_origin(
            html,
            shape,
            &plan,
            document,
            options,
            &mut patterns,
            gradients,
        );
        render_text_box_tables(
            html,
            shape,
            plan.table_offset(),
            document,
            options,
            &mut patterns,
            gradients,
        );
    }
    html.push_str("</div>");
}

/// A shape's text in the boxes that carry its origin, in the frame the caller
/// has opened: `hcD`, and `hcI` when the alignment is a length of its own.
fn render_text_in_origin(
    html: &mut String,
    shape: &crate::model::ShapeStyle,
    plan: &object_plan::TextBoxPlan,
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    if document.direct_objects.get() {
        // The direct writer puts the origin on each line (`place_text_boxes`):
        // no box between the frame and the lines moves them.
        render_text_box(
            html,
            &shape.paragraphs,
            document,
            options,
            patterns,
            gradients,
        );
        return;
    }
    let origin = plan.origin();
    html.push_str(&format!(
        "<div class=\"hcD\" style=\"left:{};top:{};\">",
        css_mm(origin.left),
        css_mm(origin.top)
    ));
    if let Some(align) = origin.align {
        html.push_str(&format!(
            "<div class=\"hcI\" style=\"top:{};\">",
            css_mm(align)
        ));
    }
    render_text_box(
        html,
        &shape.paragraphs,
        document,
        options,
        patterns,
        gradients,
    );
    if origin.align.is_some() {
        html.push_str("</div>");
    }
    html.push_str("</div>");
}

/// A text box's paragraphs; they carry no inline tables of their own.
fn render_text_box(
    html: &mut String,
    paragraphs: &[Paragraph],
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    if document.direct_objects.get() {
        if let Some(first) = paragraphs.first() {
            document.object_slots.borrow_mut().push((
                html.len(),
                super::bundle::ObjectContent::Paragraphs(first.key.clone()),
            ));
        }
        return;
    }
    let mut group = ParagraphGroup::new(document, false);
    for paragraph in paragraphs {
        for line in crate::layout::paragraph_fragments_for_render(paragraph) {
            group.line(html, document, &line, true, |html| {
                render_line(html, &line, document, options, &[], patterns, gradients)
            });
        }
    }
    group.close(html);
}

/// The tables of a shape's text box, drawn in the text box after its text
/// and moved by the text's own offset (`hcD`), as a cell draws its nested
/// floats. rhwp lays them out in the text box's inner area; the reference
/// nests them in the `hsT` (성과보고서 별첨4's title table at 2.07mm =
/// text margin + half pen + outer margin).
#[allow(clippy::too_many_arguments)]
fn render_text_box_tables(
    html: &mut String,
    shape: &crate::model::ShapeStyle,
    offset: (i64, i64),
    document: &RenderContext<'_>,
    options: &RenderOptions,
    patterns: &mut PatternIds,
    gradients: &mut GradientIds,
) {
    if document.direct_objects.get() {
        for table in &shape.tables {
            document.object_slots.borrow_mut().push((
                html.len(),
                super::bundle::ObjectContent::Table(table.id.clone()),
            ));
        }
        return;
    }
    for placed in object_plan::text_box_tables(shape, offset) {
        render_table(
            html,
            &placed,
            document,
            options,
            TablePlacement::Nested,
            patterns,
            gradients,
        );
    }
}

pub(super) fn document_asset<'a>(
    document: &'a LayoutDocument,
    reference: &str,
) -> Option<&'a crate::model::AssetRef> {
    document.assets.get(reference).or_else(|| {
        document
            .assets
            .values()
            .find(|asset| asset.path == reference || asset.path.ends_with(reference))
    })
}

fn estimate_output_size(document: &LayoutDocument) -> usize {
    4096 + document.pages.len() * 1024
        + document
            .pages
            .iter()
            .map(|page| page.lines.len() * 256 + page.tables.len() * 1024)
            .sum::<usize>()
}

pub(super) fn format_pt(hwp: i64) -> String {
    if hwp % 100 == 0 {
        (hwp / 100).to_string()
    } else {
        format!("{}.{:02}", hwp / 100, hwp.rem_euclid(100))
    }
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Text content. Only markup characters are references; U+00A0, `'` and `·`
/// are written as themselves, the same DOM text as the reference's
/// `&nbsp;`, `&#39;` and `&middot;` at a fraction of the bytes and tokens.
/// `"` stays `&quot;`: the logical DOM copies a text run verbatim into a
/// `data-gen` attribute.
pub(super) fn escape_html_text(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(value.len());
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != ' ' {
            match characters[index] {
                '&' => output.push_str("&amp;"),
                '<' => output.push_str("&lt;"),
                '>' => output.push_str("&gt;"),
                '"' => output.push_str("&quot;"),
                character => output.push(character),
            }
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len() && characters[index] == ' ' {
            index += 1;
        }
        let at_text_start = start == 0;
        let single_trailing_space = index == characters.len() && index - start == 1;
        for space_index in start..index {
            if at_text_start || single_trailing_space || space_index > start {
                output.push('\u{a0}');
            } else {
                output.push(' ');
            }
        }
    }
    output
}

pub fn escape_html_attribute(value: &str) -> String {
    escape_html(value)
}

#[cfg(test)]
mod tests {
    use super::PatternIds;

    #[test]
    fn group_children_read_top_down_but_overlaps_keep_their_drawing_order() {
        let at = |x, y, width, height| crate::model::BoxUnits {
            x,
            y,
            width,
            height,
        };
        // z-order: bottom row, a textless fill under the middle label, the
        // middle label, the top row's right and left boxes.
        let children = vec![
            ("bottom", at(0, 2000, 500, 100)),
            ("fill", at(0, 1000, 500, 100)),
            ("middle", at(100, 1000, 300, 100)),
            ("top right", at(600, 0, 500, 100)),
            ("top left", at(0, 10, 500, 100)),
        ];
        let order = crate::semantic::order::reading_order(children, |(name, area)| {
            (*area, 28, *name != "fill")
        });
        let names = order.iter().map(|(name, _)| *name).collect::<Vec<_>>();
        assert_eq!(names, ["fill", "top left", "top right", "middle", "bottom"]);

        // A box drawn over a lower one stays after it even when it reads first.
        let children = vec![
            ("under", at(0, 100, 500, 100)),
            ("over", at(0, 0, 500, 150)),
        ];
        let order = crate::semantic::order::reading_order(children, |(_, area)| (*area, 0, true));
        assert_eq!(order[0].0, "under");
    }

    #[test]
    fn a_double_slim_border_is_two_quarter_width_strokes_spanning_its_width() {
        let double = crate::model::BorderStroke {
            kind: "DOUBLE_SLIM".to_owned(),
            ..crate::model::BorderStroke::default()
        };
        // 0.7mm (198): the reference's strokes at 20.04 / 20.57mm around a
        // line at 20.30mm. A quarter of the pen is 0.176mm, written 0.17 by
        // `css_mm` (the reference rounds it to 0.18).
        let mut html = String::new();
        super::append_table_line(&mut html, (0, 5755), (1000, 5755), 198, &double);
        let paths = html.matches("<path").count();
        assert_eq!(paths, 2, "{html}");
        assert!(html.contains("M0,20.04 L3.53,20.04"), "{html}");
        assert!(html.contains("M0,20.56 L3.53,20.56"), "{html}");
        assert_eq!(html.matches("stroke-width:0.17;").count(), 2, "{html}");
    }

    #[test]
    fn a_border_on_a_double_borders_edge_yields_to_it() {
        let double = crate::model::BorderStroke {
            kind: "DOUBLE_SLIM".to_owned(),
            ..crate::model::BorderStroke::default()
        };
        let solid = crate::model::BorderStroke {
            kind: "SOLID".to_owned(),
            ..crate::model::BorderStroke::default()
        };
        let mut segments = vec![
            (0, 1000, 85, solid.clone()),
            (100, 600, 198, double.clone()),
            (0, 300, 340, solid.clone()),
        ];
        let merged = super::merge_svg_segments(&mut segments);
        assert_eq!(
            merged,
            vec![
                (0, 100, 85, solid.clone()),
                (0, 300, 340, solid.clone()),
                (100, 600, 198, double),
                (600, 1000, 85, solid),
            ]
        );
    }

    #[test]
    fn vertical_border_extra_adds_one_hwpunit_except_for_the_circle_pattern() {
        let solid = crate::model::BorderStroke::default();
        assert_eq!(super::vertical_border_extra(28, &solid), 1);
        let circle = crate::model::BorderStroke {
            kind: "CIRCLE".to_owned(),
            ..solid
        };
        assert_eq!(super::vertical_border_extra(57, &circle), 0);
        // Only the verified 0.2mm (57 HWPUNIT) CIRCLE profile is exempt;
        // other widths fall back to the ordinary butt-cap extension.
        assert_eq!(super::vertical_border_extra(28, &circle), 1);
    }

    #[test]
    fn glyph_box_heights_on_a_tie_take_the_lower_bucket() {
        // 900/2700/4500 HWPUNIT each convert to an exact .5 centi-mm, and the
        // reference in 샘플/글자 크기별 독립 문단 writes the lower bucket for
        // all of them. These used to need a hand-written correction table;
        // round_div rounding a tie toward zero now covers them, which is the
        // same rule that fixes that sample's `top` values.
        assert_eq!(crate::layout::css_mm(900), "3.17mm");
        assert_eq!(crate::layout::css_mm(2700), "9.52mm");
        assert_eq!(crate::layout::css_mm(4500), "15.87mm");
        // Sizes that are not ties are unaffected.
        assert_eq!(crate::layout::css_mm(1000), "3.53mm");
        assert_eq!(crate::layout::css_mm(2600), "9.17mm");
    }

    #[test]
    fn svg_pattern_ids_are_unique_across_pages_and_many_paints() {
        let mut ids = std::collections::HashSet::new();
        for page in 0..12 {
            let mut patterns = PatternIds {
                page,
                next: 0,
                colors: Vec::new(),
            };
            for _ in 0..25 {
                assert!(ids.insert(patterns.allocate()));
            }
        }
        assert!(ids.contains("w_00"));
        assert!(ids.contains("w_40"));
        assert!(ids.contains("w_1_10"));
        assert!(ids.contains("w_110"));
    }
    #[test]
    fn border_segments_merge_only_when_width_and_paint_match() {
        let solid = crate::model::BorderStroke::default();
        let dash = crate::model::BorderStroke {
            kind: "DASH".to_owned(),
            ..solid.clone()
        };
        let red = crate::model::BorderStroke {
            color: "#FF0000".to_owned(),
            ..dash.clone()
        };
        let mut segments = vec![
            (0, 100, 28, solid.clone()),
            (100, 200, 28, solid),
            (200, 300, 28, dash),
            (300, 400, 28, red),
        ];
        let result = super::merge_svg_segments(&mut segments);
        assert_eq!(result.len(), 3);
        assert_eq!((result[0].0, result[0].1), (0, 200));
    }

    #[test]
    fn interleaved_cell_edges_keep_continuous_paints_and_real_gaps() {
        let solid = crate::model::BorderStroke::default();
        let dash = crate::model::BorderStroke {
            kind: "DASH".to_owned(),
            ..solid.clone()
        };
        let mut segments = vec![
            (0, 100, 34, solid.clone()),
            (100, 200, 34, dash.clone()),
            (100, 200, 34, solid.clone()),
            (200, 300, 34, dash.clone()),
            (200, 300, 34, solid.clone()),
            (400, 500, 34, solid.clone()),
        ];
        assert_eq!(
            super::merge_svg_segments(&mut segments),
            vec![
                (0, 300, 34, solid.clone()),
                (100, 300, 34, dash),
                (400, 500, 34, solid),
            ]
        );
    }
    #[test]
    fn inline_cell_table_frame_extra_uses_the_table_own_out_margin() {
        let mut table = crate::model::Table {
            id: "t".to_owned(),
            source_path: "section.xml".to_owned(),
            source_anchor: None,
            caption: None,
            section_index: 0,
            anchor_y: None,
            anchor_top_adjustment: 0,
            anchor_paragraph_left: 0,
            anchor_paragraph_right: 0,
            box_units: crate::model::BoxUnits::default(),
            anchor: crate::model::Anchor::default(),
            out_margin_left: 285,
            out_margin_right: 285,
            out_margin_top: 285,
            out_margin_bottom: 285,
            columns: 1,
            rows: 1,
            row_heights: vec![3479],
            cells: Vec::new(),
            page_break: "CELL".to_owned(),
            repeat_header: false,
            no_adjust: true,
            fragment_rows: None,
        };
        // 285+285=570, minus 1: confirmed against both the .htb box height
        // (14.28mm) and the SVG viewBox height (19.28mm) independently for
        // the Louisiana letterhead table.
        assert_eq!(
            crate::layout::presentation::inline_table_frame_extra_x(&table),
            569
        );
        assert_eq!(
            crate::layout::presentation::inline_table_frame_extra_y(&table),
            569
        );

        table.out_margin_left = 0;
        table.out_margin_right = 0;
        table.out_margin_top = 0;
        table.out_margin_bottom = 0;
        assert_eq!(
            crate::layout::presentation::inline_table_frame_extra_x(&table),
            crate::layout::presentation::INLINE_CELL_FRAME_EXTRA_X - 1
        );
        assert_eq!(
            crate::layout::presentation::inline_table_frame_extra_y(&table),
            crate::layout::presentation::INLINE_CELL_FRAME_EXTRA_Y
        );
    }

    #[test]
    fn mixed_width_boundaries_keep_their_coordinate_splits() {
        let stroke = crate::model::BorderStroke::default();
        let mut segments = vec![
            (0, 100, 28, stroke.clone()),
            (0, 100, 34, stroke.clone()),
            (100, 200, 28, stroke.clone()),
            (100, 200, 34, stroke),
        ];
        let expected = segments.clone();
        assert_eq!(super::merge_svg_segments(&mut segments), expected);
    }

    #[test]
    fn page_number_box_width_scales_per_glyph_metrics() {
        let style = |family: &str| crate::model::CharStyle {
            font_family: family.to_owned(),
            font_size_hwp: 1000,
            ..Default::default()
        };

        // 함초롬돋움 「- 1 -」/「- 10 -」/「- 100 -」 → 2488/3040/3592(8.78/10.72/12.67mm)
        let dotum = style("함초롬돋움");
        assert_eq!(super::page_number_width(Some(&dotum), "- 1 -"), 2488);
        assert_eq!(crate::layout::css_mm(2488), "8.78mm");
        assert_eq!(super::page_number_width(Some(&dotum), "- 10 -"), 3040);
        assert_eq!(crate::layout::css_mm(3040), "10.72mm");
        assert_eq!(super::page_number_width(Some(&dotum), "- 100 -"), 3592);
        assert_eq!(crate::layout::css_mm(3592), "12.67mm");

        // 굴림 「- 100 -」 → 3976(14.03mm)
        let gulim = style("굴림");
        assert_eq!(super::page_number_width(Some(&gulim), "- 100 -"), 3976);
        assert_eq!(crate::layout::css_mm(3976), "14.03mm");

        // 「한양신명조」 스타일이 바탕 표를 쓴다: 「- 10 -」 → 3440(12.14mm)
        let hy_myungjo = style("한양신명조");
        assert_eq!(super::page_number_width(Some(&hy_myungjo), "- 10 -"), 3440);
        assert_eq!(crate::layout::css_mm(3440), "12.14mm");

        // 모르는 글꼴은 대체 경로(바탕 표)를 쓴다.
        let unknown = style("알수없는글꼴");
        assert_eq!(super::page_number_width(Some(&unknown), "- 10 -"), 3440);
        assert_eq!(super::page_number_width(None, "- 10 -"), 3440);
    }

    #[test]
    fn render_page_number_ends_with_center_alignment() {
        let page = crate::model::LayoutPage {
            index: 0,
            section_index: 0,
            spec: crate::model::PageSpec {
                width: 59528,
                height: 84188,
                margin_bottom: 4252,
                page_number_char_style_id: 1,
                ..Default::default()
            },
            lines: Vec::new(),
            tables: Vec::new(),
            objects: Vec::new(),
            page_number: Some("- 1 -".to_owned()),
        };
        let document = crate::model::LayoutDocument {
            char_styles: vec![crate::model::CharStyle {
                id: 1,
                font_family: "바탕".to_owned(),
                font_size_hwp: 1000,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut html = String::new();
        super::render_page_number(&mut html, &page, &document, "- 1 -");
        assert!(
            html.contains(";height:3.53mm;text-align:center;\"><span class=\"hrt cs1\""),
            "{html}"
        );
    }
}
