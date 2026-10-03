//! Reading order the source gives only as geometry (semantic-first plan
//! §3.1.1, user decision Q3): a container stores its children in z-order.

/// Put a container's children in reading order -- top to bottom, and left
/// to right within a row -- while keeping the drawing order of every pair
/// that overlaps. A group stores its children in z-order, which in
/// 성과보고서's goal diagram lists "주요내용" before "프로그램 성과목표"
/// before "전략목표", the reverse of how the page reads. Children that do
/// not overlap paint the same pixels in any order, so the DOM can follow the
/// page with no `z-index` (D28's rule for page-level objects). `extent`
/// gives a child's box, its pen, which grows the box so strokes and
/// touching edges count as overlap, and whether it holds text. A textless
/// child is decoration (`aria-hidden`) and is written as soon as it may be,
/// in z-order, so runs of them still share one `svg`.
pub fn reading_order<T>(
    items: Vec<T>,
    extent: impl Fn(&T) -> (crate::model::BoxUnits, i64, bool),
) -> Vec<T> {
    let mut texts = Vec::with_capacity(items.len());
    let boxes = items
        .iter()
        .map(|item| {
            let (area, pen, text) = extent(item);
            texts.push(text);
            let grow = pen.max(0) + 1;
            (
                area.x - grow,
                area.y - grow,
                area.x + area.width.max(0) + grow,
                area.y + area.height.max(0) + grow,
            )
        })
        .collect::<Vec<_>>();
    let overlaps = |a: usize, b: usize| {
        let (a, b) = (boxes[a], boxes[b]);
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    };
    // Same row when the vertical overlap covers half the shorter box.
    let reads_before = |a: usize, b: usize| {
        let (a, b) = (boxes[a], boxes[b]);
        let shared = a.3.min(b.3) - a.1.max(b.1);
        let shorter = (a.3 - a.1).min(b.3 - b.1);
        if shared * 2 >= shorter {
            a.0 < b.0
        } else {
            a.1 < b.1
        }
    };
    let mut placed = vec![false; items.len()];
    let mut order = Vec::with_capacity(items.len());
    while order.len() < items.len() {
        // A child is ready once every earlier child it overlaps is placed.
        let mut best: Option<usize> = None;
        for index in (0..items.len()).filter(|&index| !placed[index]) {
            if (0..index).any(|earlier| !placed[earlier] && overlaps(earlier, index)) {
                continue;
            }
            if !texts[index] {
                best = Some(index);
                break;
            }
            if best.is_none_or(|current| reads_before(index, current)) {
                best = Some(index);
            }
        }
        let next = best.expect("the first unplaced child is always ready");
        placed[next] = true;
        order.push(next);
    }
    let mut items = items.into_iter().map(Some).collect::<Vec<_>>();
    order
        .into_iter()
        .map(|index| items[index].take().expect("each child is placed once"))
        .collect()
}
