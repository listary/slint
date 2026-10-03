// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH: clipping to rounded corners (`clip: true` with a `border-radius`).
//!
//! The rectangular clip still does the straight edges. This only cuts the corners: on the lines
//! that cross a corner, the pixels the curve covers fully are drawn as usual, the ones it does not
//! reach are skipped, and each one on the curve is drawn on its own and mixed with what was there
//! before by how much of it the curve covers.

use crate::draw_functions::TargetPixel;
use i_slint_core::lengths::PhysicalPx;

/// A rounded rectangle that clips what is drawn inside it, in physical pixels of the target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundedClip {
    pub rect: euclid::Box2D<f32, PhysicalPx>,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub radius: [f32; 4],
}

impl RoundedClip {
    /// How much of the pixel at `x`, `y` the clip lets through, from 0 to 255.
    fn coverage(&self, x: i16, y: i16) -> u8 {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let r = &self.rect;
        let [top_left, top_right, bottom_right, bottom_left] = self.radius;
        let left = px < (r.min.x + r.max.x) / 2.;
        let top = py < (r.min.y + r.max.y) / 2.;
        let radius = match (left, top) {
            (true, true) => top_left,
            (false, true) => top_right,
            (false, false) => bottom_right,
            (true, false) => bottom_left,
        };
        let cx = if left { r.min.x + radius } else { r.max.x - radius };
        let cy = if top { r.min.y + radius } else { r.max.y - radius };
        let (dx, dy) = (if left { cx - px } else { px - cx }, if top { cy - py } else { py - cy });
        if dx <= 0. || dy <= 0. {
            return 255;
        }
        let distance = (dx * dx + dy * dy).sqrt();
        ((radius + 0.5 - distance).clamp(0., 1.) * 255.) as u8
    }

    /// The pixels of line `y` that the clip surely covers fully: those between the corners.
    fn full_span(&self, y: i16) -> (i16, i16) {
        let py = y as f32 + 0.5;
        let r = &self.rect;
        let [top_left, top_right, bottom_right, bottom_left] = self.radius;
        let corner = |top: f32, bottom: f32| {
            if py < r.min.y + top {
                top
            } else if py > r.max.y - bottom {
                bottom
            } else {
                0.
            }
        };
        let start = (r.min.x + corner(top_left, bottom_left)).ceil();
        let end = (r.max.x - corner(top_right, bottom_right)).floor();
        (start as i16, end as i16)
    }
}

/// How much of the pixel at `x`, `y` all of `clips` let through together, from 0 to 255.
pub fn coverage(clips: &[RoundedClip], x: i16, y: i16) -> u8 {
    clips.iter().fold(255, |acc, clip| (acc as u16 * clip.coverage(x, y) as u16 / 255) as u8)
}

/// Draws one line of an item through `clips`. `line` holds the pixels from `x` on, and
/// `draw(pixels, extra_left_clip, extra_right_clip)` draws the item into part of it, as for the
/// rectangular clip.
pub fn draw_line<T: TargetPixel>(
    clips: &[RoundedClip],
    y: i16,
    x: i16,
    line: &mut [T],
    extra_left_clip: i16,
    extra_right_clip: i16,
    draw: &mut impl FnMut(&mut [T], i16, i16),
) {
    let len = line.len() as i16;
    let (start, end) = clips.iter().fold((i16::MIN, i16::MAX), |(start, end), clip| {
        let (s, e) = clip.full_span(y);
        (start.max(s), end.min(e))
    });
    let start = start.saturating_sub(x).clamp(0, len);
    let end = end.saturating_sub(x).clamp(start, len);
    if start < end {
        draw(
            &mut line[start as usize..end as usize],
            extra_left_clip + start,
            extra_right_clip + len - end,
        );
    }
    for i in (0..start).chain(end..len) {
        let coverage = coverage(clips, x + i, y);
        if coverage == 0 {
            continue;
        }
        let before = line[i as usize];
        let pixel = &mut line[i as usize..i as usize + 1];
        draw(pixel, extra_left_clip + i, extra_right_clip + len - i - 1);
        if coverage < 255 {
            let drawn = pixel[0];
            pixel[0] = before;
            pixel[0].mix(drawn, coverage);
        }
    }
}

#[test]
fn coverage_cuts_only_the_corners() {
    let clip = RoundedClip {
        rect: euclid::Box2D::new(euclid::point2(10., 10.), euclid::point2(50., 30.)),
        radius: [8., 0., 4., 8.],
    };
    // Outside the curve, on it, and inside it at the top-left corner.
    assert_eq!(clip.coverage(10, 10), 0);
    assert!((1..255).contains(&clip.coverage(12, 12)), "{}", clip.coverage(12, 12));
    assert_eq!(clip.coverage(14, 14), 255);
    // The square top-right corner and the middle of an edge let everything through.
    assert_eq!(clip.coverage(49, 10), 255);
    assert_eq!(clip.coverage(30, 10), 255);
    assert_eq!(clip.coverage(49, 29), 0);
    // A line through the corners is full only between them; one between the corners is full.
    assert_eq!(clip.full_span(10), (18, 50));
    assert_eq!(clip.full_span(29), (18, 46));
    assert_eq!(clip.full_span(20), (10, 50));
}
