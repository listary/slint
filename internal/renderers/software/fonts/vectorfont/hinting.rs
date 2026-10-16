// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// LISTARY PATCH (glyph-hinting): TrueType hinting picked per font and size from the `gasp`
// table, the way DirectWrite picks its rendering mode. See LISTARY-PATCHES.md.

use alloc::rc::Rc;
use alloc::vec::Vec;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{
    DrawSettings, Engine, HintingInstance, HintingOptions, OutlineGlyphFormat, OutlinePen,
    SmoothMode, Target,
};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gasp::{Gasp, GaspRangeBehavior};
use skrifa::{GlyphId, MetadataProvider};
use swash::zeno::{Command, Format, Mask, Origin, Placement, Vector};

/// How a font draws at one size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rasterization {
    /// The font's TrueType instructions fit the outline to the pixel grid vertically.
    pub hinted: bool,
    /// Each pixel row takes the coverage of the whole row. Without it, a row takes the coverage
    /// along its center line, so a horizontal stroke on whole rows fills them.
    pub vertical_smoothing: bool,
}

/// The outline as designed, smoothed in both directions.
const UNHINTED_SMOOTHED: Rasterization = Rasterization { hinted: false, vertical_smoothing: true };

/// Without `gasp` flags DirectWrite smooths vertically above this many pixels per em.
const NO_GASP_SMOOTHING_ABOVE_PPEM: u16 = 20;

/// The rendering DirectWrite's grayscale mode recommends for a font at `ppem` pixels per em,
/// given the `gasp` flags for that size (`None` without a version 1 `gasp` table).
///
/// DirectWrite reads only the symmetric flags, so it ignores version 0 tables, which lack them.
/// Plain grid-fitting and grayscale don't matter.
pub fn rasterization_for(gasp: Option<GaspRangeBehavior>, ppem: u16) -> Rasterization {
    match gasp {
        None => {
            Rasterization { hinted: true, vertical_smoothing: ppem > NO_GASP_SMOOTHING_ABOVE_PPEM }
        }
        Some(flags) => Rasterization {
            hinted: flags.contains(GaspRangeBehavior::GASP_SYMMETRIC_GRIDFIT),
            vertical_smoothing: flags.contains(GaspRangeBehavior::GASP_SYMMETRIC_SMOOTHING),
        },
    }
}

fn gasp_flags(gasp: Option<Gasp>, ppem: u16) -> Option<GaspRangeBehavior> {
    let gasp = gasp.filter(|gasp| gasp.version() >= 1)?;
    let range = gasp.gasp_ranges().iter().find(|range| ppem <= range.range_max_ppem());
    Some(range.map_or(GaspRangeBehavior::empty(), |range| range.range_gasp_behavior()))
}

/// CFF outlines draw unhinted and smoothed, as before.
fn rasterization(font: &skrifa::FontRef, size: f32) -> Rasterization {
    if font.outline_glyphs().format() != Some(OutlineGlyphFormat::Glyf) {
        return UNHINTED_SMOOTHED;
    }
    let ppem = size.round().clamp(0.0, u16::MAX as f32) as u16;
    rasterization_for(gasp_flags(font.gasp().ok(), ppem), ppem)
}

/// Glyph sizes in 1/64 pixels. Glyphs are cached and drawn at the size this rounds to.
pub fn size_64(size: f32) -> u32 {
    (size * 64.0).round() as u32
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct InstanceKey {
    font_blob_id: u64,
    font_index: u32,
    /// The size in 1/64 pixels.
    size: u32,
    coords_hash: u64,
}

struct Hinting {
    rasterization: Rasterization,
    instance: Option<HintingInstance>,
}

/// Hinting instances kept for reuse. One instance holds the font's executed font program,
/// control values and storage, about 4 KiB for Microsoft YaHei UI.
const MAX_INSTANCES: usize = 48;

i_slint_core::thread_local!(static INSTANCES: core::cell::RefCell<Vec<(InstanceKey, Rc<Hinting>)>> =
    Default::default());

fn hinting(
    font: &skrifa::FontRef,
    key: InstanceKey,
    size: f32,
    coords: &[NormalizedCoord],
) -> Rc<Hinting> {
    INSTANCES.with(|instances| {
        let mut instances = instances.borrow_mut();
        if let Some(at) = instances.iter().position(|(k, _)| *k == key) {
            // Most recently used last.
            let entry = instances.remove(at);
            let hinting = entry.1.clone();
            instances.push(entry);
            return hinting;
        }
        let rasterization = rasterization(font, size);
        let instance = rasterization
            .hinted
            .then(|| {
                HintingInstance::new(
                    &font.outline_glyphs(),
                    Size::new(size),
                    LocationRef::new(coords),
                    HintingOptions {
                        engine: Engine::Interpreter,
                        target: Target::Smooth {
                            mode: SmoothMode::Normal,
                            symmetric_rendering: rasterization.vertical_smoothing,
                            preserve_linear_metrics: true,
                        },
                    },
                )
                .ok()
            })
            .flatten();
        let hinting = Rc::new(Hinting {
            // A font whose program fails to run draws as designed.
            rasterization: if rasterization.hinted && instance.is_none() {
                UNHINTED_SMOOTHED
            } else {
                rasterization
            },
            instance,
        });
        if instances.len() >= MAX_INSTANCES {
            instances.remove(0);
        }
        instances.push((key, hinting.clone()));
        hinting
    })
}

/// The glyph's coverage at `size` rounded to 1/64 pixels, see `size_64`.
/// Returns `Some(None)` when the font draws as designed and smoothed at this size,
/// and `None` when the glyph can't be drawn here; the caller renders the plain outline then.
pub fn render_hinted(
    data: &[u8],
    font_index: u32,
    font_blob_id: u64,
    size: f32,
    normalized_coords: &[i16],
    coords_hash: u64,
    glyph_id: u16,
    offset_x: f32,
) -> Option<Option<(Vec<u8>, Placement)>> {
    let font = skrifa::FontRef::from_index(data, font_index).ok()?;
    let coords: Vec<NormalizedCoord> =
        normalized_coords.iter().map(|&c| NormalizedCoord::from_bits(c)).collect();
    let key = InstanceKey { font_blob_id, font_index, size: size_64(size), coords_hash };
    let size = key.size as f32 / 64.0;
    let hinting = hinting(&font, key, size, &coords);
    if hinting.rasterization == UNHINTED_SMOOTHED {
        return Some(None);
    }
    let settings: DrawSettings = match &hinting.instance {
        Some(instance) => instance.into(),
        None => DrawSettings::unhinted(Size::new(size), LocationRef::new(&coords)),
    };
    let glyph = font.outline_glyphs().get(GlyphId::new(glyph_id as u32))?;
    if hinting.rasterization.vertical_smoothing {
        let mut pen = ZenoPen(Vec::new());
        glyph.draw(settings, &mut pen).ok()?;
        let offset = Vector::new(offset_x, 0.0);
        return Some(Some(
            Mask::new(&pen.0[..])
                .format(Format::Alpha)
                .origin(Origin::BottomLeft)
                .offset(offset)
                .render_offset(offset)
                .inspect(|_, _, _| {})
                .render(),
        ));
    }
    let mut pen = FlatteningPen::default();
    glyph.draw(settings, &mut pen).ok()?;
    if hinting.instance.is_none() {
        // Unhinted, DirectWrite places rows as at the whole pixels per em.
        let y_scale = size.round() / size;
        pen.edges.iter_mut().for_each(|e| (e[1], e[3]) = (e[1] * y_scale, e[3] * y_scale));
    }
    Some(Some(rasterize_row_centers(&pen.edges, offset_x)?))
}

struct ZenoPen(Vec<Command>);

impl OutlinePen for ZenoPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(Command::MoveTo(Vector::new(x, y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(Command::LineTo(Vector::new(x, y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.push(Command::QuadTo(Vector::new(cx0, cy0), Vector::new(x, y)));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.push(Command::CurveTo(
            Vector::new(cx0, cy0),
            Vector::new(cx1, cy1),
            Vector::new(x, y),
        ));
    }
    fn close(&mut self) {
        self.0.push(Command::Close);
    }
}

/// How far a flattened curve may stray from the real one, in pixels.
const FLATTEN_TOLERANCE: f32 = 0.05;

/// Collects an outline as straight edges, curves split into lines.
#[derive(Default)]
struct FlatteningPen {
    edges: Vec<[f32; 4]>,
    start: (f32, f32),
    at: (f32, f32),
}

impl FlatteningPen {
    fn line(&mut self, x: f32, y: f32) {
        if (x, y) != self.at {
            self.edges.push([self.at.0, self.at.1, x, y]);
        }
        self.at = (x, y);
    }

    fn segments(deviation: f32) -> usize {
        ((deviation / FLATTEN_TOLERANCE).sqrt().ceil() as usize).clamp(1, 32)
    }
}

impl OutlinePen for FlatteningPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = (x, y);
        self.at = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.line(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        let deviation = ((x0 - 2.0 * cx + x).hypot(y0 - 2.0 * cy + y)) / 4.0;
        let n = Self::segments(deviation);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.line(
                u * u * x0 + 2.0 * u * t * cx + t * t * x,
                u * u * y0 + 2.0 * u * t * cy + t * t * y,
            );
        }
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        let deviation = (x0 - 2.0 * cx0 + cx1)
            .hypot(y0 - 2.0 * cy0 + cy1)
            .max((cx0 - 2.0 * cx1 + x).hypot(cy0 - 2.0 * cy1 + y))
            * 0.75;
        let n = Self::segments(deviation);
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.line(a * x0 + b * cx0 + c * cx1 + d * x, a * y0 + b * cy0 + c * cy1 + d * y);
        }
    }
    fn close(&mut self) {
        let (x, y) = self.start;
        self.line(x, y);
    }
}

/// The largest bitmap `rasterize_row_centers` makes, in pixels.
const MAX_ROW_CENTER_PIXELS: usize = 1 << 22;

/// Fills the outline with the nonzero rule, one sample per pixel row at the row's center
/// line and exact coverage across. Rows are counted up from the baseline, as in `Placement`.
/// Returns `None` for an outline with points that aren't finite or a bitmap over
/// `MAX_ROW_CENTER_PIXELS`.
fn rasterize_row_centers(edges: &[[f32; 4]], offset_x: f32) -> Option<(Vec<u8>, Placement)> {
    let empty = (Vec::new(), Placement { left: 0, top: 0, width: 0, height: 0 });
    if edges.is_empty() {
        return Some(empty);
    }
    if !offset_x.is_finite() || edges.iter().flatten().any(|v| !v.is_finite()) {
        return None;
    }
    let (mut x_min, mut x_max, mut y_min, mut y_max) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for e in edges {
        x_min = x_min.min(e[0]).min(e[2]);
        x_max = x_max.max(e[0]).max(e[2]);
        y_min = y_min.min(e[1]).min(e[3]);
        y_max = y_max.max(e[1]).max(e[3]);
    }
    // Row k spans [k, k + 1] and samples at k + 0.5.
    let first_row = (y_min - 0.5).ceil() as i64;
    let last_row = (y_max - 0.5).floor() as i64;
    if last_row < first_row {
        return Some(empty);
    }
    let left = (x_min + offset_x).floor() as i64;
    let width = usize::try_from(((x_max + offset_x).ceil() as i64).checked_sub(left)?).ok()?;
    if width == 0 {
        return Some(empty);
    }
    let height = usize::try_from(last_row.checked_sub(first_row)?.checked_add(1)?).ok()?;
    if width.checked_mul(height)? > MAX_ROW_CENTER_PIXELS {
        return None;
    }
    let (left, top) = (i32::try_from(left).ok()?, i32::try_from(last_row.checked_add(1)?).ok()?);
    let mut out = alloc::vec![0u8; width * height];
    let mut coverage = alloc::vec![0f32; width];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for (r, row) in (first_row..=last_row).rev().enumerate() {
        let y = row as f32 + 0.5;
        crossings.clear();
        for &[x0, y0, x1, y1] in edges {
            let (lo, hi, winding) = if y0 < y1 { (y0, y1, 1) } else { (y1, y0, -1) };
            if lo <= y && y < hi {
                crossings.push((x0 + (y - y0) * (x1 - x0) / (y1 - y0) + offset_x, winding));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        coverage.iter_mut().for_each(|c| *c = 0.0);
        let mut winding = 0;
        let mut span_start = 0.0;
        for &(x, w) in &crossings {
            let was_inside = winding != 0;
            winding += w;
            if !was_inside && winding != 0 {
                span_start = x;
            } else if was_inside && winding == 0 {
                add_span(&mut coverage, span_start - left as f32, x - left as f32);
            }
        }
        for (dst, c) in out[r * width..(r + 1) * width].iter_mut().zip(&coverage) {
            *dst = (c.min(1.0) * 255.0).round() as u8;
        }
    }
    Some((out, Placement { left, top, width: width as u32, height: height as u32 }))
}

fn add_span(coverage: &mut [f32], a: f32, b: f32) {
    let a = a.max(0.0);
    let b = b.min(coverage.len() as f32);
    if b <= a {
        return;
    }
    let first = a.floor() as usize;
    let last = (b.ceil() as usize).min(coverage.len());
    for (i, c) in coverage.iter_mut().enumerate().take(last).skip(first) {
        *c += (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrifa::raw::FontRead;

    const GRIDFIT: GaspRangeBehavior = GaspRangeBehavior::GASP_GRIDFIT;
    const DOGRAY: GaspRangeBehavior = GaspRangeBehavior::GASP_DOGRAY;
    const SYM_GRIDFIT: GaspRangeBehavior = GaspRangeBehavior::GASP_SYMMETRIC_GRIDFIT;
    const SYM_SMOOTHING: GaspRangeBehavior = GaspRangeBehavior::GASP_SYMMETRIC_SMOOTHING;

    fn rasterization_of(hinted: bool, vertical_smoothing: bool) -> Rasterization {
        Rasterization { hinted, vertical_smoothing }
    }

    /// The flags of each case come from Windows fonts; DirectWrite's recommended mode for them
    /// was read with `IDWriteFontFace3::GetRecommendedRenderingMode`.
    #[test]
    fn rasterization_follows_the_gasp_flags_like_directwrite() {
        let empty = GaspRangeBehavior::empty();
        let cases = [
            // Microsoft YaHei UI up to 21 ppem, and above.
            (Some(GRIDFIT | DOGRAY | SYM_GRIDFIT), 15, (true, false)),
            (Some(GRIDFIT | DOGRAY | SYM_SMOOTHING), 23, (false, true)),
            // Segoe UI above 19 ppem.
            (Some(GRIDFIT | DOGRAY | SYM_GRIDFIT | SYM_SMOOTHING), 20, (true, true)),
            // Candara and Cascadia Code at small sizes.
            (Some(DOGRAY | SYM_SMOOTHING), 9, (false, true)),
            // Arial and Javanese Text from 9 to 17 ppem.
            (Some(GRIDFIT | SYM_GRIDFIT), 12, (true, false)),
            // Roboto up to 8 ppem: grayscale alone neither fits nor smooths vertically.
            (Some(DOGRAY), 8, (false, false)),
            (Some(empty), 8, (false, false)),
            // MV Boli at every size: plain grid-fitting doesn't count.
            (Some(GRIDFIT), 12, (false, false)),
            (Some(GRIDFIT | DOGRAY), 30, (false, false)),
            // No flags: fitted, smoothed vertically above 20 ppem.
            (None, 20, (true, false)),
            (None, 21, (true, true)),
        ];
        for (flags, ppem, (hinted, smoothed)) in cases {
            assert_eq!(
                rasterization_for(flags, ppem),
                rasterization_of(hinted, smoothed),
                "{flags:?} at {ppem} ppem"
            );
        }
    }

    #[test]
    fn version_0_gasp_tables_count_as_none() {
        // One range up to 65535 ppem with the grayscale flag.
        let version_0 = [0, 0, 0, 1, 0xff, 0xff, 0, 2];
        let gasp = Gasp::read(skrifa::raw::FontData::new(&version_0)).unwrap();
        assert_eq!(gasp_flags(Some(gasp), 12), None);
        let version_1 = [0, 1, 0, 1, 0xff, 0xff, 0, 2];
        let gasp = Gasp::read(skrifa::raw::FontData::new(&version_1)).unwrap();
        assert_eq!(gasp_flags(Some(gasp), 12), Some(DOGRAY));
    }

    /// Roboto's `gasp`: up to 8 ppem neither fitted nor smoothed vertically, fitted and
    /// smoothed above.
    const ROBOTO: &[u8] =
        include_bytes!("../../../../../examples/carousel/ui/fonts/Roboto-Regular.ttf");

    fn render_roboto(size: f32) -> (Vec<u8>, (i32, i32, u32, u32)) {
        render_roboto_char('E', size)
    }

    fn render_roboto_char(c: char, size: f32) -> (Vec<u8>, (i32, i32, u32, u32)) {
        let font = skrifa::FontRef::new(ROBOTO).unwrap();
        let gid = font.charmap().map(c).unwrap().to_u32() as u16;
        let (data, placement) = render_hinted(ROBOTO, 0, 1, size, &[], 0, gid, 0.0)
            .expect("drawn here")
            .expect("not drawn as designed");
        (data, placed(placement))
    }

    #[test]
    fn sizes_that_share_a_cache_entry_draw_the_same_whichever_comes_first() {
        // 8.49 rounds to 8 ppem; 8.496 and 8.5 share 1/64 px 544, at 9 ppem.
        let sizes = [8.49, 8.496, 8.5];
        let cold: Vec<_> = sizes
            .iter()
            .map(|&size| {
                INSTANCES.with(|instances| instances.borrow_mut().clear());
                render_roboto(size)
            })
            .collect();
        assert_ne!(cold[0], cold[2], "the sizes fall in different gasp ranges");
        assert_eq!(cold[1], cold[2]);
        for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2]] {
            INSTANCES.with(|instances| instances.borrow_mut().clear());
            for i in order {
                assert_eq!(render_roboto(sizes[i]), cold[i], "{} after {order:?}", sizes[i]);
            }
        }
    }

    #[test]
    fn unhinted_sizes_without_vertical_smoothing_draw_stems_without_faded_ends() {
        // Roboto's H at 8 ppem is drawn as designed but sampled once per row: the stems' top
        // and bottom rows are as dark as their middle. Smoothed, they fade.
        let ends_match_middle = |data: &[u8], width: u32, height: u32| {
            let rows: Vec<&[u8]> = data.chunks(width as usize).collect();
            let middle = rows[height as usize / 4];
            rows[0] == middle && rows[rows.len() - 1] == middle
        };
        let (data, (_, _, width, height)) = render_roboto_char('H', 8.0);
        assert!(ends_match_middle(&data, width, height));
        let font = swash::FontRef::from_index(ROBOTO, 0).unwrap();
        let mut context = swash::scale::ScaleContext::new();
        let gid = font.charmap().map('H');
        let image = swash::scale::Render::new(&[swash::scale::Source::Outline])
            .format(Format::Alpha)
            .render(&mut context.builder(font).size(8.0).build(), gid)
            .unwrap();
        assert!(!ends_match_middle(&image.data, image.placement.width, image.placement.height));
    }

    fn placed(p: Placement) -> (i32, i32, u32, u32) {
        (p.left, p.top, p.width, p.height)
    }

    #[test]
    fn outlines_too_large_or_not_finite_are_left_to_the_plain_rasterizer() {
        assert!(rasterize_row_centers(&rectangle(0.0, 0.0, f32::NAN, 1.0), 0.0).is_none());
        assert!(rasterize_row_centers(&rectangle(0.0, 0.0, f32::INFINITY, 1.0), 0.0).is_none());
        assert!(rasterize_row_centers(&rectangle(0.0, 0.0, 1.0, 1.0), f32::NAN).is_none());
        assert!(rasterize_row_centers(&rectangle(0.0, 0.0, 4096.0, 4096.0), 0.0).is_none());
        assert!(rasterize_row_centers(&rectangle(-3e38, -3e38, 3e38, 3e38), 0.0).is_none());
        assert!(rasterize_row_centers(&rectangle(0.0, 0.0, 2048.0, 2048.0), 0.0).is_some());
        // No width: nothing to draw, however tall, and no row is scanned.
        let (data, placement) =
            rasterize_row_centers(&rectangle(0.0, -2e9, 0.0, 0.0), 0.0).unwrap();
        assert!(data.is_empty() && placement.height == 0);
        assert!(rasterize_row_centers(&rectangle(0.0, -2e9, 0.0, 0.0), 0.5).is_none());
    }

    fn rectangle(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 4]> {
        let mut pen = FlatteningPen::default();
        pen.move_to(x0, y0);
        pen.line_to(x1, y0);
        pen.line_to(x1, y1);
        pen.line_to(x0, y1);
        pen.close();
        pen.edges
    }

    #[test]
    fn a_fitted_horizontal_stroke_fills_whole_rows_with_exact_coverage_across() {
        // A stroke 1 pixel tall from y = 3 to 4, x from 0.25 to 2.5.
        let (data, placement) =
            rasterize_row_centers(&rectangle(0.25, 3.0, 2.5, 4.0), 0.0).unwrap();
        assert_eq!(placed(placement), (0, 4, 3, 1));
        assert_eq!(data, [191, 255, 128]);
        // The subpixel offset moves it across, not down.
        let (data, placement) =
            rasterize_row_centers(&rectangle(0.25, 3.0, 2.5, 4.0), 0.75).unwrap();
        assert_eq!(placed(placement), (1, 4, 3, 1));
        assert_eq!(data, [255, 255, 64]);
    }

    #[test]
    fn a_stroke_that_misses_every_row_center_leaves_no_row() {
        // 0.4 pixels tall, between two centers.
        let (data, placement) =
            rasterize_row_centers(&rectangle(0.0, 3.55, 2.0, 3.95), 0.0).unwrap();
        assert_eq!(placement.height, 0);
        assert!(data.is_empty());
    }

    #[test]
    fn overlapping_contours_count_once_and_holes_stay_empty() {
        let mut edges = rectangle(0.0, 0.0, 4.0, 3.0);
        edges.extend(rectangle(1.0, 0.0, 3.0, 3.0)); // same winding: overlaps
        let (data, _) = rasterize_row_centers(&edges, 0.0).unwrap();
        assert!(data.iter().all(|&a| a == 255));
        let mut edges = rectangle(0.0, 0.0, 4.0, 3.0);
        // Reversed winding: a hole in the middle row's middle.
        let mut hole = rectangle(1.0, 1.0, 3.0, 2.0);
        hole.iter_mut().for_each(|e| *e = [e[2], e[3], e[0], e[1]]);
        edges.extend(hole);
        let (data, placement) = rasterize_row_centers(&edges, 0.0).unwrap();
        assert_eq!(placement.height, 3);
        assert_eq!(&data[4..8], &[255, 0, 0, 255]);
    }

    /// The rows each glyph inks match DirectWrite's (`directwrite_rows.txt`, recorded from
    /// the fonts of Windows 11 26200). Pixel values differ: DirectWrite's grayscale has 8
    /// coverage levels and blurs across more, so only the rows are compared.
    #[cfg(target_os = "windows")]
    #[test]
    fn hinted_rows_match_directwrite_for_the_windows_ui_fonts() {
        let fonts = [
            ("yahei", "msyh.ttc", 1),
            ("jhenghei", "msjh.ttc", 1),
            ("yugothic", "YuGothM.ttc", 1),
            ("malgun", "malgun.ttf", 0),
            ("segoe", "segoeui.ttf", 0),
            ("nirmala", "Nirmala.ttc", 0),
            ("arlrdbd", "ARLRDBD.TTF", 0),
            ("segvar", "SegUIVar.ttf", 0),
            ("roboto", "Roboto-Regular.ttf", 0),
            ("mvboli", "mvboli.ttf", 0),
            ("agency", "AGENCYR.TTF", 0),
            ("javatext", "javatext.ttf", 0),
            ("arial", "arial.ttf", 0),
        ];
        let reference = include_str!("directwrite_rows.txt");
        let mut scale_context = swash::scale::ScaleContext::new();
        let mut checked = 0;
        for (blob_id, (name, file, index)) in fonts.into_iter().enumerate() {
            let Ok(data) = std::fs::read(std::format!("C:/Windows/Fonts/{file}")) else { continue };
            let face = skrifa::FontRef::from_index(&data, index).unwrap();
            // [without vertical smoothing below 9 ppem, from 9 ppem, with vertical smoothing]:
            // (glyphs, glyphs inking the same rows)
            let mut counts = [(0, 0); 3];
            let mut first_miss = None;
            for line in reference.lines().filter(|l| l.split(' ').next() == Some(name)) {
                let mut it = line.split(' ').skip(1);
                let size: f32 = it.next().unwrap().parse().unwrap();
                let gid: u16 = it.next().unwrap().parse().unwrap();
                let top: i32 = it.next().unwrap().parse().unwrap();
                let rows = it.next().unwrap();
                let (bitmap, placement) =
                    match render_hinted(&data, index, blob_id as u64, size, &[], 0, gid, 0.0)
                        .unwrap()
                    {
                        Some(hinted) => hinted,
                        None => {
                            let font = swash::FontRef::from_index(&data, index as usize).unwrap();
                            let mut scaler = scale_context.builder(font).size(size).build();
                            let image = swash::scale::Render::new(&[swash::scale::Source::Outline])
                                .format(Format::Alpha)
                                .render(&mut scaler, gid)
                                .unwrap();
                            (image.data, image.placement)
                        }
                    };
                let w = placement.width as usize;
                let inked: Vec<bool> = (0..placement.height as usize)
                    .map(|r| bitmap[r * w..(r + 1) * w].iter().any(|&a| a >= 77))
                    .collect();
                let ours = inked.iter().position(|&i| i).map(|first| {
                    let last = inked.iter().rposition(|&i| i).unwrap();
                    let pattern: alloc::string::String =
                        inked[first..=last].iter().map(|&i| if i { '1' } else { '0' }).collect();
                    (placement.top - first as i32, pattern)
                });
                let smoothed = rasterization(&face, size).vertical_smoothing;
                let count = &mut counts[if smoothed { 2 } else { (size.round() >= 9.0) as usize }];
                count.0 += 1;
                if ours.as_ref().is_some_and(|(t, p)| *t == top && p == rows) {
                    count.1 += 1;
                } else if !smoothed && size.round() >= 9.0 && first_miss.is_none() {
                    first_miss = Some((size, gid, top, alloc::string::String::from(rows), ours));
                }
            }
            std::println!(
                "{name}: same rows {:?} without vertical smoothing below 9 ppem, {:?} from 9 ppem, {:?} with",
                counts[0],
                counts[1],
                counts[2]
            );
            // Where rows are sampled once, the rows must match. Below 9 ppem DirectWrite also
            // keeps strokes that fall between two row centers (dropout control), which this
            // doesn't. With vertical smoothing a faint edge row can fall on either side of the
            // ink threshold.
            let [(tiny, tiny_same), (sharp, sharp_same), (smooth, smooth_same)] = counts;
            assert!(
                tiny_same * 100 >= tiny * 80,
                "{name}: {tiny_same} of {tiny} glyphs below 9 ppem"
            );
            assert!(
                sharp_same * 100 >= sharp * 97,
                "{name}: {sharp_same} of {sharp} glyphs ink the rows DirectWrite inks; first miss {first_miss:?}"
            );
            assert!(
                smooth_same * 100 >= smooth * 70,
                "{name}: {smooth_same} of {smooth} smoothed glyphs"
            );
            checked += tiny + sharp + smooth;
        }
        std::println!("checked {checked} glyphs");
        assert!(checked > 0, "none of the fonts is installed");
    }
}
