// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// LISTARY PATCH (glyph-hinting): TrueType hinting picked per font and size from the `gasp`
// table, the way DirectWrite picks its rendering mode. See LISTARY-PATCHES.md.

use alloc::rc::Rc;
use alloc::vec::Vec;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::outline::{
    Engine, HintingInstance, HintingOptions, OutlineGlyphFormat, OutlinePen, SmoothMode, Target,
};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::gasp::GaspRangeBehavior;
use skrifa::{GlyphId, MetadataProvider};
use swash::zeno::{Command, Format, Mask, Origin, Placement, Vector};

/// How a font draws at one size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rasterization {
    /// The outline as designed, smoothed in both directions.
    Unhinted,
    /// The outline grid-fitted by the font's TrueType instructions in the vertical direction.
    /// Without `vertical_smoothing`, each pixel row takes the coverage along its center line,
    /// so a fitted horizontal stroke fills whole rows.
    Hinted { vertical_smoothing: bool },
}

/// No `gasp` table: DirectWrite smooths vertically from this size up.
const NO_GASP_SMOOTHING_ABOVE: f32 = 20.0;

/// The rendering DirectWrite recommends for a font at `size` pixels, given the behavior flags
/// its `gasp` table lists for that size (`None` when the font has no `gasp`).
///
/// The size is rounded to whole pixels per em before the lookup. With vertical smoothing
/// the font is grid-fitted when the flags ask for symmetric grid-fitting, without it when they
/// ask for plain grid-fitting.
pub fn rasterization_for(gasp: Option<GaspRangeBehavior>, size: f32) -> Rasterization {
    let Some(flags) = gasp else {
        return Rasterization::Hinted { vertical_smoothing: size > NO_GASP_SMOOTHING_ABOVE };
    };
    let vertical_smoothing = flags.contains(GaspRangeBehavior::GASP_SYMMETRIC_SMOOTHING);
    let grid_fit = if vertical_smoothing {
        flags.contains(GaspRangeBehavior::GASP_SYMMETRIC_GRIDFIT)
    } else {
        flags.contains(GaspRangeBehavior::GASP_GRIDFIT)
    };
    if grid_fit { Rasterization::Hinted { vertical_smoothing } } else { Rasterization::Unhinted }
}

fn gasp_flags(font: &skrifa::FontRef, ppem: u16) -> Option<GaspRangeBehavior> {
    let gasp = font.gasp().ok()?;
    let range = gasp.gasp_ranges().iter().find(|range| ppem <= range.range_max_ppem());
    Some(range.map_or(GaspRangeBehavior::empty(), |range| range.range_gasp_behavior()))
}

/// Only TrueType outlines that carry instructions are hinted; CFF outlines and fonts without
/// a font or control-value program draw unhinted, as before.
fn rasterization(font: &skrifa::FontRef, size: f32) -> Rasterization {
    let outlines = font.outline_glyphs();
    if outlines.format() != Some(OutlineGlyphFormat::Glyf) {
        return Rasterization::Unhinted;
    }
    let gasp = gasp_flags(font, size.round().clamp(0.0, u16::MAX as f32) as u16);
    let has_program =
        |tag: &[u8; 4]| font.data_for_tag(skrifa::raw::types::Tag::new(tag)).is_some();
    if gasp.is_none() && !has_program(b"fpgm") && !has_program(b"prep") {
        return Rasterization::Unhinted;
    }
    rasterization_for(gasp, size)
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
        let instance = match rasterization {
            Rasterization::Unhinted => None,
            Rasterization::Hinted { vertical_smoothing } => HintingInstance::new(
                &font.outline_glyphs(),
                Size::new(size),
                LocationRef::new(coords),
                HintingOptions {
                    engine: Engine::Interpreter,
                    target: Target::Smooth {
                        mode: SmoothMode::Normal,
                        symmetric_rendering: vertical_smoothing,
                        preserve_linear_metrics: true,
                    },
                },
            )
            .ok(),
        };
        let hinting = Rc::new(Hinting {
            // A font whose program fails to run draws unhinted.
            rasterization: if instance.is_some() { rasterization } else { Rasterization::Unhinted },
            instance,
        });
        if instances.len() >= MAX_INSTANCES {
            instances.remove(0);
        }
        instances.push((key, hinting.clone()));
        hinting
    })
}

/// The glyph's coverage when the font is hinted at this size, `None` when it draws unhinted
/// (the caller then renders the plain outline).
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
    let key =
        InstanceKey { font_blob_id, font_index, size: (size * 64.0).round() as u32, coords_hash };
    let hinting = hinting(&font, key, size, &coords);
    let (Rasterization::Hinted { vertical_smoothing }, Some(instance)) =
        (hinting.rasterization, hinting.instance.as_ref())
    else {
        return Some(None);
    };
    let glyph = font.outline_glyphs().get(GlyphId::new(glyph_id as u32))?;
    if vertical_smoothing {
        let mut pen = ZenoPen(Vec::new());
        glyph.draw(instance, &mut pen).ok()?;
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
    glyph.draw(instance, &mut pen).ok()?;
    Some(Some(rasterize_row_centers(&pen.edges, offset_x)))
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

/// Fills the outline with the nonzero rule, one sample per pixel row at the row's center
/// line and exact coverage across. Rows are counted up from the baseline, as in `Placement`.
fn rasterize_row_centers(edges: &[[f32; 4]], offset_x: f32) -> (Vec<u8>, Placement) {
    let empty = (Vec::new(), Placement { left: 0, top: 0, width: 0, height: 0 });
    if edges.is_empty() {
        return empty;
    }
    let (mut x_min, mut x_max, mut y_min, mut y_max) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for e in edges {
        x_min = x_min.min(e[0]).min(e[2]);
        x_max = x_max.max(e[0]).max(e[2]);
        y_min = y_min.min(e[1]).min(e[3]);
        y_max = y_max.max(e[1]).max(e[3]);
    }
    // Row k spans [k, k + 1] and samples at k + 0.5.
    let first_row = (y_min - 0.5).ceil() as i32;
    let last_row = (y_max - 0.5).floor() as i32;
    if last_row < first_row {
        return empty;
    }
    let left = (x_min + offset_x).floor() as i32;
    let width = ((x_max + offset_x).ceil() as i32 - left).max(0) as usize;
    let height = (last_row - first_row + 1) as usize;
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
    (out, Placement { left, top: last_row + 1, width: width as u32, height: height as u32 })
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

    const GRIDFIT: GaspRangeBehavior = GaspRangeBehavior::GASP_GRIDFIT;
    const DOGRAY: GaspRangeBehavior = GaspRangeBehavior::GASP_DOGRAY;
    const SYM_GRIDFIT: GaspRangeBehavior = GaspRangeBehavior::GASP_SYMMETRIC_GRIDFIT;
    const SYM_SMOOTHING: GaspRangeBehavior = GaspRangeBehavior::GASP_SYMMETRIC_SMOOTHING;

    #[test]
    fn rasterization_follows_the_gasp_flags_like_directwrite() {
        // Microsoft YaHei UI up to 21 ppem, and above.
        assert_eq!(
            rasterization_for(Some(GRIDFIT | DOGRAY | SYM_GRIDFIT), 15.0),
            Rasterization::Hinted { vertical_smoothing: false }
        );
        assert_eq!(
            rasterization_for(Some(GRIDFIT | DOGRAY | SYM_SMOOTHING), 22.5),
            Rasterization::Unhinted
        );
        // Segoe UI above 19 ppem: fitted and smoothed.
        assert_eq!(
            rasterization_for(Some(GRIDFIT | DOGRAY | SYM_GRIDFIT | SYM_SMOOTHING), 20.0),
            Rasterization::Hinted { vertical_smoothing: true }
        );
        // An unhinted font that asks for smoothing only.
        assert_eq!(rasterization_for(Some(DOGRAY | SYM_SMOOTHING), 12.0), Rasterization::Unhinted);
        // No gasp table.
        assert_eq!(
            rasterization_for(None, 20.0),
            Rasterization::Hinted { vertical_smoothing: false }
        );
        assert_eq!(
            rasterization_for(None, 20.5),
            Rasterization::Hinted { vertical_smoothing: true }
        );
    }

    fn placed(p: Placement) -> (i32, i32, u32, u32) {
        (p.left, p.top, p.width, p.height)
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
        let (data, placement) = rasterize_row_centers(&rectangle(0.25, 3.0, 2.5, 4.0), 0.0);
        assert_eq!(placed(placement), (0, 4, 3, 1));
        assert_eq!(data, [191, 255, 128]);
        // The subpixel offset moves it across, not down.
        let (data, placement) = rasterize_row_centers(&rectangle(0.25, 3.0, 2.5, 4.0), 0.75);
        assert_eq!(placed(placement), (1, 4, 3, 1));
        assert_eq!(data, [255, 255, 64]);
    }

    #[test]
    fn a_stroke_that_misses_every_row_center_leaves_no_row() {
        // 0.4 pixels tall, between two centers.
        let (data, placement) = rasterize_row_centers(&rectangle(0.0, 3.55, 2.0, 3.95), 0.0);
        assert_eq!(placement.height, 0);
        assert!(data.is_empty());
    }

    #[test]
    fn overlapping_contours_count_once_and_holes_stay_empty() {
        let mut edges = rectangle(0.0, 0.0, 4.0, 3.0);
        edges.extend(rectangle(1.0, 0.0, 3.0, 3.0)); // same winding: overlaps
        let (data, _) = rasterize_row_centers(&edges, 0.0);
        assert!(data.iter().all(|&a| a == 255));
        let mut edges = rectangle(0.0, 0.0, 4.0, 3.0);
        // Reversed winding: a hole in the middle row's middle.
        let mut hole = rectangle(1.0, 1.0, 3.0, 2.0);
        hole.iter_mut().for_each(|e| *e = [e[2], e[3], e[0], e[1]]);
        edges.extend(hole);
        let (data, placement) = rasterize_row_centers(&edges, 0.0);
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
        ];
        let reference = include_str!("directwrite_rows.txt");
        let mut scale_context = swash::scale::ScaleContext::new();
        let mut checked = 0;
        for (blob_id, (name, file, index)) in fonts.into_iter().enumerate() {
            let Ok(data) = std::fs::read(std::format!("C:/Windows/Fonts/{file}")) else { continue };
            let face = skrifa::FontRef::from_index(&data, index).unwrap();
            // [without vertical smoothing, with it]: (glyphs, glyphs inking the same rows)
            let mut counts = [(0, 0); 2];
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
                let smoothed = rasterization(&face, size)
                    != Rasterization::Hinted { vertical_smoothing: false };
                let count = &mut counts[smoothed as usize];
                count.0 += 1;
                if ours.as_ref().is_some_and(|(t, p)| *t == top && p == rows) {
                    count.1 += 1;
                } else if !smoothed && first_miss.is_none() {
                    first_miss = Some((size, gid, top, alloc::string::String::from(rows), ours));
                }
            }
            std::println!(
                "{name}: same rows {:?} without vertical smoothing, {:?} with",
                counts[0],
                counts[1]
            );
            // Where rows are fitted and sampled once, the rows must match. With vertical
            // smoothing a faint edge row can fall on either side of the ink threshold, so
            // those only have to mostly match.
            let [(sharp, sharp_same), (smooth, smooth_same)] = counts;
            assert!(
                sharp_same * 100 >= sharp * 98,
                "{name}: {sharp_same} of {sharp} glyphs ink the rows DirectWrite inks; first miss {first_miss:?}"
            );
            assert!(
                smooth_same * 100 >= smooth * 70,
                "{name}: {smooth_same} of {smooth} smoothed glyphs"
            );
            checked += sharp + smooth;
        }
        std::println!("checked {checked} glyphs");
    }
}
