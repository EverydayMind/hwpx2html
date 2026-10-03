//! Lengths as the browser lays them out: whole 1/64 px units (Chrome's
//! `LayoutUnit`), so a position can be written exactly.

/// One 1/64 px, the browser's layout unit.
pub(super) const UNIT: i64 = 64;
/// Positions beyond 2^18 px lose 1/64 px as a float: written as a multiple
/// of this plus a remainder.
pub(super) const CHUNK: i64 = 65536 * UNIT;

/// A millimetre length as Chrome lays it out: converted to a float of CSS
/// pixels, snapped down to 1/64 px.
pub(super) fn snap_mm(mm: f64) -> i64 {
    ((mm * 96.0 / 25.4) as f32 as f64 * UNIT as f64).floor() as i64
}

/// A length in 1/64 px, written exactly.
pub(super) fn px(units: i64) -> String {
    let sign = if units < 0 { "-" } else { "" };
    let units = units.abs();
    let whole = units / UNIT;
    let fraction = units % UNIT;
    if fraction == 0 {
        return format!("{sign}{whole}px");
    }
    let digits = format!("{:06}", fraction * 15625);
    format!("{sign}{whole}.{}px", digits.trim_end_matches('0'))
}

pub(super) fn split(units: i64) -> (String, String) {
    let high = units / CHUNK * CHUNK;
    (px(high), px(units - high))
}
