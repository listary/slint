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
//! lowered, so no text gets lighter than before. Every text colour gets a similar amount (half
//! coverage maps to about 150-165): light text mostly through the gamma step, dark text mostly
//! through the contrast step, mid-grey text through both.
//!
//! The text colour's alpha is not part of the table: it is still multiplied in afterwards, like
//! before. Only glyphs use the table (see `DrawTextureArgs::coverage_table`); alpha-map images
//! such as single-colour icons are blended as before.

/// The table that maps glyph coverage before blending.
pub type CoverageTable = [u8; 256];

/// Exponent of the blend the correction aims for. At 1.0 only the contrast term would raise
/// coverage; 2.2 would be a fully gamma-correct blend, which makes light text much heavier than
/// WPF or DirectWrite draw it.
#[cfg(feature = "std")]
pub const GAMMA: f32 = 1.6;

/// How much coverage is raised before the gamma step, scaled by the luminance of the assumed
/// background (and capped at 1): strong under dark text, nearly nothing under light text.
#[cfg(feature = "std")]
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
#[cfg(feature = "std")]
fn luminance_index(color: i_slint_core::Color) -> u8 {
    use num_traits::Float;
    let lin = |c: u8| Float::powf(c as f32 / 255.0, GAMMA);
    let lin = 0.2126 * lin(color.red()) + 0.7152 * lin(color.green()) + 0.0722 * lin(color.blue());
    (Float::powf(lin, 1.0 / GAMMA) * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Skia's `build_gamma_table` with the background taken as the opposite luminance, clamped so
/// that no coverage goes down.
#[cfg(feature = "std")]
fn build_table(src: f32, gamma: f32, contrast: f32) -> CoverageTable {
    use num_traits::Float;
    let mut table = [0u8; 256];
    for (i, entry) in table.iter_mut().enumerate() {
        *entry = i as u8;
    }
    let dst = 1.0 - src;
    // Text exactly as light as the assumed background would divide 0 by 0 below. `table_for` never
    // asks for it (luminances are n/255), and the tables on either side of it are smooth.
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
    fn every_text_colour_gets_about_the_same_edge_ink() {
        // Light text through the gamma step, dark text mostly through the contrast step,
        // mid-grey through both: half coverage ends up at about the same value for all of them.
        for (r, g, b) in [
            (0xFF, 0xFF, 0xFF),
            (0xEC, 0xEA, 0xEA),
            (0x80, 0x80, 0x80),
            (0x24, 0x1E, 0x1F),
            (0x00, 0x00, 0x00),
        ] {
            let table = table_for(Color::from_rgb_u8(r, g, b)).unwrap();
            assert!(
                (145..=170).contains(&table[128]),
                "#{r:02X}{g:02X}{b:02X}: 128 -> {}",
                table[128]
            );
        }
    }

    /// One alpha-map pixel at half coverage, light text over a dark pixel: the blend reads the
    /// table when it is set and the raw coverage when it is not, and the text's alpha is applied
    /// to the corrected coverage.
    #[test]
    fn the_blend_reads_coverage_through_the_table_then_applies_alpha() {
        use crate::{Fixed, PhysicalLength, PhysicalRect, RenderingRotation, SceneTexture};
        use i_slint_core::graphics::{Rgb8Pixel, TexturePixelFormat};

        let color = Color::from_rgb_u8(0xEC, 0xEA, 0xEA);
        let draw = |coverage_table, alpha: u8| {
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
                    alpha,
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
        for alpha in [0xFF, 0x92] {
            let plain = draw(None, alpha);
            let corrected = draw(Some(table), alpha);
            // The green channel goes from 0x24 towards 0xEA by coverage x alpha.
            let share =
                |coverage: u16| 0x24 + (0xEA - 0x24) * (coverage * alpha as u16 / 255) / 255;
            assert!(plain.abs_diff(share(128) as u8) <= 2, "alpha {alpha:#x}: plain {plain:#x}");
            assert!(
                corrected.abs_diff(share(table[128] as u16) as u8) <= 2,
                "alpha {alpha:#x}: corrected {corrected:#x}"
            );
            assert!(corrected > plain + 0x08, "alpha {alpha:#x}");
        }
    }

    #[test]
    fn alpha_does_not_pick_the_table() {
        let opaque = table_for(Color::from_argb_u8(0xFF, 0xEC, 0xEA, 0xEA)).unwrap();
        let translucent = table_for(Color::from_argb_u8(0x92, 0xEC, 0xEA, 0xEA)).unwrap();
        assert!(core::ptr::eq(opaque, translucent));
    }
}
