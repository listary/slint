// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Path rendering support for the software renderer using zeno

use super::draw_functions::{PremultipliedRgbaColor, TargetPixel};
use super::{PhysicalRect, PhysicalRegion};
use alloc::vec;
use alloc::vec::Vec;
use zeno::{Cap, Fill, Join, Mask, Stroke, Style};

pub use zeno::Command;

/// Convert Slint's PathDataIterator to zeno's Command format
pub fn convert_path_data_to_zeno(
    path_data: i_slint_core::graphics::PathDataIterator,
    rotation: crate::RotationInfo,
    scale_factor: i_slint_core::lengths::ScaleFactor,
    offset: euclid::Vector2D<f32, i_slint_core::lengths::PhysicalPx>,
) -> Vec<Command> {
    use crate::Transform as _;
    use i_slint_core::lengths::LogicalPoint;
    use lyon_path::Event;
    let mut commands = Vec::new();

    let convert_point = |p| {
        let p = (LogicalPoint::from_untyped(p) * scale_factor + offset).transformed(rotation);
        zeno::Point::new(p.x, p.y)
    };

    for event in path_data.iter() {
        match event {
            Event::Begin { at } => {
                commands.push(Command::MoveTo(convert_point(at)));
            }
            Event::Line { to, .. } => {
                commands.push(Command::LineTo(convert_point(to)));
            }
            Event::Quadratic { ctrl, to, .. } => {
                commands.push(Command::QuadTo(convert_point(ctrl), convert_point(to)));
            }
            Event::Cubic { ctrl1, ctrl2, to, .. } => {
                commands.push(Command::CurveTo(
                    convert_point(ctrl1),
                    convert_point(ctrl2),
                    convert_point(to),
                ));
            }
            Event::End { close, .. } => {
                if close {
                    commands.push(Command::Close);
                }
            }
        }
    }

    commands
}

/// Common rendering logic for both filled and stroked paths
fn render_path_with_style<T: TargetPixel>(
    commands: &[Command],
    path_geometry: &PhysicalRect,
    clip_geometry: &PhysicalRect,
    dirty_region: &PhysicalRegion,
    rounded_clips: &[crate::RoundedClip],
    color: PremultipliedRgbaColor,
    style: zeno::Style,
    buffer: &mut impl crate::target_pixel_buffer::TargetPixelBuffer<TargetPixel = T>,
) {
    // The mask needs to be rendered at the full path size
    let path_width = path_geometry.size.width as usize;
    let path_height = path_geometry.size.height as usize;

    if path_width == 0 || path_height == 0 {
        return;
    }

    // Create a buffer for the mask output
    let mut mask_buffer = vec![0u8; path_width * path_height];

    // Render the full path into the mask
    Mask::new(commands)
        .size(path_width as u32, path_height as u32)
        .style(style)
        .render_into(&mut mask_buffer, None);

    // Calculate the intersection region - only apply within clipped area
    // clip_geometry is relative to screen, path_geometry is also relative to screen
    let clip_x_start = clip_geometry.origin.x.max(0) as usize;
    let clip_y_start = clip_geometry.origin.y.max(0) as usize;
    let clip_x_end = (clip_geometry.max_x().max(0) as usize).min(buffer.line_slice(0).len());
    let clip_y_end = (clip_geometry.max_y().max(0) as usize).min(buffer.num_lines());

    let path_x_start = path_geometry.origin.x as isize;
    let path_y_start = path_geometry.origin.y as isize;

    // LISTARY PATCH: apply the mask only where the clip meets the dirty region. A partial
    // redraw repaints every item that touches the dirty region; writing the whole clip drew the
    // part of the path outside the region over what an earlier frame had painted above it. The
    // region's rectangles may overlap, so walk its merged line ranges, as
    // `RenderToBuffer::foreach_ranges` does for every other item: each pixel is blended once.
    let mut ranges = Vec::new();
    let mut band_start = clip_y_start;
    while band_start < clip_y_end {
        let Some(next) = crate::region_line_ranges(dirty_region, band_start as i16, &mut ranges)
        else {
            break;
        };
        let next = (next.max(0) as usize).min(clip_y_end);
        if next <= band_start {
            break;
        }
        for screen_y in band_start..next {
            // Calculate the y coordinate in the mask buffer
            let mask_y = screen_y as isize - path_y_start;
            if mask_y < 0 || mask_y >= path_height as isize {
                continue;
            }
            let line = buffer.line_slice(screen_y);
            for range in &ranges {
                let x_start = (range.start.max(0) as usize).max(clip_x_start);
                let x_end = (range.end.max(0) as usize).min(clip_x_end);
                if x_start >= x_end {
                    continue;
                }
                for (i, pixel) in line[x_start..x_end].iter_mut().enumerate() {
                    let screen_x = x_start + i;

                    // Calculate the x coordinate in the mask buffer
                    let mask_x = screen_x as isize - path_x_start;
                    if mask_x < 0 || mask_x >= path_width as isize {
                        continue;
                    }

                    let mask_idx = (mask_y as usize) * path_width + (mask_x as usize);
                    // LISTARY PATCH: and only what the rounded clips let through.
                    let clip = crate::rounded_clip::coverage(
                        rounded_clips,
                        screen_x as i16,
                        screen_y as i16,
                    );
                    let coverage = (mask_buffer[mask_idx] as u16 * clip as u16 / 255) as u8;

                    if coverage > 0 {
                        // Scale all color components by coverage to maintain premultiplication
                        let coverage_factor = coverage as u16;
                        let alpha_color = PremultipliedRgbaColor {
                            red: ((color.red as u16 * coverage_factor) / 255) as u8,
                            green: ((color.green as u16 * coverage_factor) / 255) as u8,
                            blue: ((color.blue as u16 * coverage_factor) / 255) as u8,
                            alpha: ((color.alpha as u16 * coverage_factor) / 255) as u8,
                        };
                        T::blend(pixel, alpha_color);
                    }
                }
            }
        }
        band_start = next;
    }
}

/// Render a filled path
///
/// * `commands` - The path commands to render
/// * `path_geometry` - The full bounding box of the path in screen coordinates
/// * `clip_geometry` - The clipped region where the path should be rendered (intersection of path and clip)
/// * `dirty_region` - LISTARY PATCH: the region being redrawn; nothing outside it is written
/// * `rounded_clips` - LISTARY PATCH: the rounded clips the path is drawn through
/// * `color` - The color to render the path
/// * `buffer` - The target pixel buffer
pub fn render_filled_path<T: TargetPixel>(
    commands: &[Command],
    path_geometry: &PhysicalRect,
    clip_geometry: &PhysicalRect,
    dirty_region: &PhysicalRegion,
    rounded_clips: &[crate::RoundedClip],
    color: PremultipliedRgbaColor,
    buffer: &mut impl crate::target_pixel_buffer::TargetPixelBuffer<TargetPixel = T>,
) {
    render_path_with_style(
        commands,
        path_geometry,
        clip_geometry,
        dirty_region,
        rounded_clips,
        color,
        zeno::Style::Fill(Fill::NonZero),
        buffer,
    );
}

/// Render a stroked path
///
/// * `commands` - The path commands to render
/// * `path_geometry` - The full bounding box of the path in screen coordinates
/// * `clip_geometry` - The clipped region where the path should be rendered (intersection of path and clip)
/// * `dirty_region` - LISTARY PATCH: the region being redrawn; nothing outside it is written
/// * `rounded_clips` - LISTARY PATCH: the rounded clips the path is drawn through
/// * `color` - The color to render the path
/// * `stroke_width` - The width of the stroke
/// * `buffer` - The target pixel buffer
#[allow(clippy::too_many_arguments)]
pub fn render_stroked_path<T: TargetPixel>(
    commands: &[Command],
    path_geometry: &PhysicalRect,
    clip_geometry: &PhysicalRect,
    dirty_region: &PhysicalRegion,
    rounded_clips: &[crate::RoundedClip],
    color: PremultipliedRgbaColor,
    stroke_width: f32,
    stroke_line_cap: i_slint_core::items::LineCap,
    stroke_line_join: i_slint_core::items::LineJoin,
    stroke_miter_limit: f32,
    buffer: &mut impl crate::target_pixel_buffer::TargetPixelBuffer<TargetPixel = T>,
) {
    let mut stroke = Stroke::new(stroke_width);
    stroke
        .cap(match stroke_line_cap {
            i_slint_core::items::LineCap::Round => Cap::Round,
            i_slint_core::items::LineCap::Square => Cap::Square,
            i_slint_core::items::LineCap::Butt | _ => Cap::Butt,
        })
        .join(match stroke_line_join {
            i_slint_core::items::LineJoin::Round => Join::Round,
            i_slint_core::items::LineJoin::Bevel => Join::Bevel,
            i_slint_core::items::LineJoin::Miter | _ => Join::Miter,
        })
        .miter_limit(stroke_miter_limit);
    let style = Style::Stroke(stroke);
    render_path_with_style(
        commands,
        path_geometry,
        clip_geometry,
        dirty_region,
        rounded_clips,
        color,
        style,
        buffer,
    );
}
