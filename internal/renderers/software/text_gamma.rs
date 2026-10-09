// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH (text-gamma): coverage correction for anti-aliased glyphs.
//!
//! Glyph coverage is blended linearly in sRGB. A half-covered pixel of light text on a dark
//! background then comes out darker than it looks in a gamma-correct blend, so light-on-dark
//! text looks thin and faint; dark text on a light background loses some weight too.
//! DirectWrite and Skia correct the coverage before blending. This does what Skia's mask gamma
//! does (`SkMaskGamma`): from the text colour's luminance it builds a table that maps each
//! coverage value to the one whose sRGB blend gives what a blend with gamma [`GAMMA`] would, over
//! a background of the opposite luminance, after raising the coverage by a contrast term that is
//! strongest for dark text. Coverage 0 and 255 map to themselves, so fully covered pixels and the
//! glyph outlines do not change; only the anti-aliased edges get more ink. Coverage is never
//! lowered, so no text gets lighter than before.
//!
//! The text colour's alpha is not part of the table: it is still multiplied in afterwards, like
//! before. Only glyphs use the table (see `DrawTextureArgs::coverage_table`); alpha-map images
//! such as single-colour icons are blended as before.

/// The table that maps glyph coverage before blending.
pub type CoverageTable = [u8; 256];

/// Exponent of the blend the correction aims for. 1.0 would leave coverage as is; 2.2 would be a
/// fully gamma-correct blend, which makes light text much heavier than WPF or DirectWrite draw it.
pub const GAMMA: f32 = 1.6;

/// How much coverage is raised before the gamma step, scaled by the luminance of the assumed
/// background (and capped at 1): strong under dark text, nearly nothing under light text.
pub const CONTRAST: f32 = 1.25;

/// The correction table for glyphs drawn in `color`, or `None` when this build has no `std`
/// (the tables are built once and kept for the whole process).
#[cfg(feature = "std")]
pub fn table_for(color: i_slint_core::Color) -> Option<&'static CoverageTable> {
    static TABLES: std::sync::OnceLock<alloc::boxed::Box<[CoverageTable; 256]>> =
        std::sync::OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        let mut tables = alloc::boxed::Box::new([[0u8; 256]; 256]);
        for (luminance, table) in tables.iter_mut().enumerate() {
            *table = build_table(luminance as f32 / 255.0, GAMMA, CONTRAST);
        }
        tables
    });
    Some(&tables[luminance_index(color) as usize])
}

#[cfg(not(feature = "std"))]
pub fn table_for(_color: i_slint_core::Color) -> Option<&'static CoverageTable> {
    None
}

/// The text colour's luminance in the blend's gamma space, 0..=255 (alpha ignored).
fn luminance_index(color: i_slint_core::Color) -> u8 {
    use num_traits::Float;
    let lin = |c: u8| Float::powf(c as f32 / 255.0, GAMMA);
    let lin = 0.2126 * lin(color.red()) + 0.7152 * lin(color.green()) + 0.0722 * lin(color.blue());
    (Float::powf(lin, 1.0 / GAMMA) * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Skia's `build_gamma_table` with the background taken as the opposite luminance, clamped so
/// that no coverage goes down.
fn build_table(src: f32, gamma: f32, contrast: f32) -> CoverageTable {
    use num_traits::Float;
    let mut table = [0u8; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        *entry = i as u8;
    }
    let dst = 1.0 - src;
    if (src - dst).abs() < 1.0 / 256.0 {
        return table;
    }
    let lin_src = Float::powf(src, gamma);
    let lin_dst = Float::powf(dst, gamma);
    // Above 1 the raised coverage would overshoot 1 near full coverage and stop rising (black
    // text over the assumed white background); Skia keeps its contrast in 0..=1 for the same reason.
    let contrast = (contrast * lin_dst).min(1.0);
    for (i, entry) in table.iter_mut().enumerate().skip(1).take(254) {
        let coverage = i as f32 / 255.0;
        let raised = coverage + (1.0 - coverage) * contrast * coverage;
        let lin_out = lin_src * raised + (1.0 - raised) * lin_dst;
        let out = Float::powf(lin_out, 1.0 / gamma);
        let corrected = ((out - dst) / (src - dst) * 255.0).round().clamp(0.0, 255.0) as u8;
        *entry = corrected.max(i as u8);
    }
    table
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use i_slint_core::Color;

    #[test]
    fn ends_stay_and_nothing_goes_down() {
        for luminance in 0..=255u8 {
            let table = build_table(luminance as f32 / 255.0, GAMMA, CONTRAST);
            assert_eq!((table[0], table[255]), (0, 255), "luminance {luminance}");
            for i in 0..256 {
                assert!(table[i] as usize >= i, "luminance {luminance}: {i} -> {}", table[i]);
                if i > 0 {
                    assert!(
                        table[i] >= table[i - 1],
                        "luminance {luminance}: not monotonic at {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn light_and_dark_text_get_more_edge_ink_and_grey_none() {
        // Light text through the gamma step, dark text mostly through the contrast step.
        let light = table_for(Color::from_rgb_u8(0xEC, 0xEA, 0xEA)).unwrap();
        let dark = table_for(Color::from_rgb_u8(0x24, 0x1E, 0x1F)).unwrap();
        assert!((150..=175).contains(&light[128]), "light: 128 -> {}", light[128]);
        assert!((150..=175).contains(&dark[128]), "dark: 128 -> {}", dark[128]);
        // A colour as light as the assumed background: nothing to correct.
        let grey = build_table(0.5, GAMMA, CONTRAST);
        assert!(grey.iter().enumerate().all(|(i, &v)| v as usize == i));
    }

    /// One alpha-map pixel at half coverage, light text over a dark pixel: the blend reads the
    /// table when it is set and the raw coverage when it is not.
    #[test]
    fn the_blend_reads_coverage_through_the_table() {
        use crate::{Fixed, PhysicalLength, PhysicalRect, RenderingRotation, SceneTexture};
        use i_slint_core::graphics::{Rgb8Pixel, TexturePixelFormat};

        let color = Color::from_rgb_u8(0xEC, 0xEA, 0xEA);
        let draw = |coverage_table| {
            let data = [128u8];
            let texture = SceneTexture {
                data: &data,
                format: TexturePixelFormat::AlphaMap,
                pixel_stride: 1,
                extra: crate::scene::SceneTextureExtra {
                    dx: Fixed::from_integer(1),
                    dy: Fixed::from_integer(1),
                    off_x: Fixed::from_integer(0),
                    off_y: Fixed::from_integer(0),
                    colorize: color,
                    alpha: 255,
                    rotation: RenderingRotation::NoRotation,
                    coverage_table,
                },
            };
            let mut line = [Rgb8Pixel { r: 0x1E, g: 0x24, b: 0x2D }];
            crate::draw_functions::draw_texture_line(
                &PhysicalRect::new(euclid::point2(0, 0), euclid::size2(1, 1)),
                PhysicalLength::new(0),
                &texture,
                &mut line,
                0,
                0,
            );
            line[0].g
        };
        let table = table_for(color).unwrap();
        let plain = draw(None);
        let corrected = draw(Some(table));
        // Plain: half the way from 0x24 to 0xEA. Corrected: the table's share of the way.
        assert!((0x84..=0x88).contains(&plain), "plain {plain:#x}");
        let expected = 0x24 + (0xEA - 0x24) * table[128] as u16 / 255;
        assert!(
            corrected.abs_diff(expected as u8) <= 2,
            "corrected {corrected:#x}, expected {expected:#x}"
        );
        assert!(corrected > plain + 0x10);
    }

    #[test]
    fn alpha_does_not_pick_the_table() {
        let opaque = table_for(Color::from_argb_u8(0xFF, 0xEC, 0xEA, 0xEA)).unwrap();
        let translucent = table_for(Color::from_argb_u8(0x92, 0xEC, 0xEA, 0xEA)).unwrap();
        assert!(core::ptr::eq(opaque, translucent));
    }
}
