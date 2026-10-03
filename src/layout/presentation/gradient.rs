//! The bands of a two-colour gradation: the colours and the edges the
//! reference draws a horizontal (`angle="0"`) gradient box with. Whether a
//! box is drawn this way (`objects::gradient_drawn`) depends on them, so they
//! sit with the placement plan and both writers read them.

/// The angles of a two-colour linear gradation whose band geometry is known:
/// 0 (bands across the height), 90 (bands across the width) and 45 (diagonal
/// bands). Every gradation of the samples is one of these, drawn by the
/// reference as mirrored bands about a centre line through the shape's
/// top-left corner (the first colour on it, the second at the far edge).
/// Any other angle keeps the approximation.
pub fn band_angle_supported(angle: i64) -> bool {
    matches!(angle, 0 | 45 | 90)
}

/// The length the bands of a gradation at `angle` divide: the height for 0,
/// the width for 90 and the sum of both for 45 (the shape's extent across
/// its diagonal bands, the same HWPUNIT `gradient_band_edges` divides).
pub fn band_extent(angle: i64, width: i64, height: i64) -> i64 {
    match angle {
        90 => width,
        45 => width.saturating_add(height),
        _ => height,
    }
}

/// The colours the reference expands a two-colour gradation into, in the
/// order its SVG `<pattern>` lists them (each one then appears twice, as a
/// mirrored +y/-y pair).
///
/// Bands run from the gradation's *second* colour to its first. There are
/// `step` colours, spaced by `ceil` rather than `round` -- a 10-step black to
/// white ramp gives 0, 29, 57, 85, 114, ... where rounding would give 28, 57,
/// 85, 113. An odd `step` yields one more band than colours, and the extra
/// one repeats the middle colour, so step 3 paints 0, 128, 128, 255.
///
/// Verified against every gradation in 샘플/그라디언트 샘플2 (steps 2, 3, 10,
/// 100, 200, 254, 255 and per-channel ranges 1, 2, 3, 15, 136), 샘플/그라디언트
/// 샘플 and 2022회계연도 성과보고서 -- 27 patterns, all exact.
///
/// Only gradations whose channels increase have been seen, since every
/// sampled one ends at #FFFFFF; a decreasing channel's rounding direction is
/// still unverified.
pub fn gradient_band_colors(step: u32, first: &str, second: &str) -> Vec<[u8; 3]> {
    let parse = |value: &str| -> Option<[u8; 3]> {
        let digits = value.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some([channel(0)?, channel(2)?, channel(4)?])
    };
    let (Some(start), Some(end)) = (parse(second), parse(first)) else {
        return Vec::new();
    };
    if step == 0 {
        return Vec::new();
    }
    let colors = (0..step)
        .map(|k| {
            let mut color = [0_u8; 3];
            for channel in 0..3 {
                let from = i32::from(start[channel]);
                let span = i32::from(end[channel]) - from;
                let stepped = if step == 1 {
                    span
                } else {
                    let divisor = i32::from(step as u16 - 1);
                    let scaled = span * k as i32;
                    // Round away from the start colour, in whichever
                    // direction the channel is moving.
                    if scaled >= 0 {
                        scaled.div_euclid(divisor) + i32::from(scaled.rem_euclid(divisor) != 0)
                    } else {
                        -((-scaled).div_euclid(divisor)
                            + i32::from((-scaled).rem_euclid(divisor) != 0))
                    }
                };
                color[channel] = (from + stepped).clamp(0, 255) as u8;
            }
            color
        })
        .collect::<Vec<_>>();
    let bands = step + (step & 1);
    if bands == step {
        return colors;
    }
    (0..bands as usize)
        .map(|band| {
            let middle = bands as usize / 2;
            colors[if band < middle { band } else { band - 1 }]
        })
        .collect()
}

/// Band edges of a horizontal (`angle="0"`) gradation, from the shape's far
/// edge back to its centre, in HWPUNIT.
///
/// The reference lays the bands out on the *source* grid, not on the band
/// count: `step` equal divisions of the shape's height, each rounded half up
/// to a whole HWPUNIT before its millimetre conversion. An odd `step` gets
/// one extra edge at the exact half height, which is what splits the middle
/// colour into the duplicated pair that takes the band count to `step + 1`.
/// Verified against every pattern in 샘플/그라디언트 샘플2 -- 7 gradations
/// with steps 2, 3, 10, 100, 200, 254 and 255, 833 edges, all exact.
pub fn gradient_band_edges(height: i64, step: u32) -> Vec<i64> {
    if step == 0 || height <= 0 {
        return Vec::new();
    }
    let step = i64::from(step);
    let divide = |k: i64| (height.saturating_mul(k).saturating_mul(2) + step) / (step * 2);
    let mut edges = Vec::with_capacity(step as usize + 2);
    for k in (0..=step).rev() {
        if step % 2 == 1 && k == step / 2 {
            edges.push((height + 1) / 2);
        }
        edges.push(divide(k));
    }
    edges
}

#[cfg(test)]
mod tests {
    #[test]
    fn gradient_band_edges_land_on_the_source_step_grid() {
        // 샘플/그라디언트 샘플2's rectangles are 4708 HWPUNIT tall. Every
        // expectation is the reference pattern's own band boundary, read back
        // as HWPUNIT: `svg_mm` turns 3139 into 11.07, 2354 into 8.30 and so on.
        assert_eq!(super::gradient_band_edges(4708, 2), vec![4708, 2354, 0]);
        // An odd step keeps its own three divisions and gains the half-height
        // edge, which is what duplicates the middle colour.
        assert_eq!(
            super::gradient_band_edges(4708, 3),
            vec![4708, 3139, 2354, 1569, 0]
        );
        assert_eq!(
            super::gradient_band_edges(4708, 10),
            vec![4708, 4237, 3766, 3296, 2825, 2354, 1883, 1412, 942, 471, 0]
        );
        // Band count is `step + (step & 1)`, so the edge count is one more.
        for step in [2_u32, 3, 10, 100, 200, 254, 255] {
            let bands = step + (step & 1);
            assert_eq!(
                super::gradient_band_edges(4708, step).len(),
                bands as usize + 1,
                "step {step}"
            );
        }
        assert!(super::gradient_band_edges(4708, 0).is_empty());
        assert!(super::gradient_band_edges(0, 10).is_empty());
    }

    #[test]
    fn gradient_bands_follow_the_reference_ramp() {
        let hex = |bands: Vec<[u8; 3]>| {
            bands
                .into_iter()
                .map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
                .collect::<Vec<_>>()
        };
        // Every expectation below is read straight off a reference
        // <pattern>'s own path fills in 샘플/그라디언트 샘플2.
        assert_eq!(
            hex(super::gradient_band_colors(2, "#FFFFFF", "#000000")),
            ["#000000", "#FFFFFF"]
        );
        // An odd step paints one more band than it has colours, repeating
        // the middle one.
        assert_eq!(
            hex(super::gradient_band_colors(3, "#FFFFFF", "#000000")),
            ["#000000", "#808080", "#808080", "#FFFFFF"]
        );
        // ceil, not round: rounding would give 28, 113 and 198 here.
        assert_eq!(
            hex(super::gradient_band_colors(10, "#FFFFFF", "#000000")),
            [
                "#000000", "#1D1D1D", "#393939", "#555555", "#727272", "#8E8E8E", "#AAAAAA",
                "#C7C7C7", "#E3E3E3", "#FFFFFF"
            ]
        );
        let full = super::gradient_band_colors(255, "#FFFFFF", "#000000");
        assert_eq!(full.len(), 256);
        assert_eq!(
            hex(full[..4].to_vec()),
            ["#000000", "#020202", "#030303", "#040404"]
        );
        // A one-channel-wide range reaches the far colour on its second band
        // and stays there.
        let narrow = super::gradient_band_colors(255, "#FFFFFF", "#FEFEFE");
        assert_eq!(narrow.len(), 256);
        assert_eq!(hex(narrow[..3].to_vec()), ["#FEFEFE", "#FFFFFF", "#FFFFFF"]);
        // Channels are independent: this one moves 0, 3 and 136 steps.
        let mixed = super::gradient_band_colors(255, "#FFFFFF", "#FFFC77");
        assert_eq!(hex(mixed[..2].to_vec()), ["#FFFC77", "#FFFD78"]);
    }
}
