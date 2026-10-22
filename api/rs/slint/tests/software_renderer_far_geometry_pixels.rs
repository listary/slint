// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! The software renderer draws in 16-bit physical coordinates.
//! Geometry reaching past ±32767 pixels renders its visible part exactly like geometry that
//! differs from it only outside the window.

mod common;

use slint::platform::software_renderer::{
    LineBufferProvider, MinimalSoftwareWindow, RenderingRotation, RepaintBufferType,
};
use slint::{ComponentHandle, Rgb8Pixel};

const WIDTH: f32 = 200.;
const HEIGHT: f32 = 40.;

slint::slint! {
    export component Shape inherits Window {
        width: 200px;
        height: 40px;
        background: white;
        in property <length> rx;
        in property <length> rw;
        in property <length> ry: 4px;
        in property <length> rh: 32px;
        in property <length> radius;
        in property <length> border;
        in property <bool> gradient;
        Rectangle {
            x: root.rx;
            y: root.ry;
            width: root.rw;
            height: root.rh;
            background: root.gradient ? @linear-gradient(180deg, #f00, #00f) : #f00;
            border-width: root.border;
            border-color: #00f;
            border-radius: root.radius;
        }
    }

    // A selected line, slid so its visible part lies at `offset`, the way an editable
    // single-line box scrolls. Glyphs are transparent, so only the highlight is drawn.
    export component Selection inherits Window {
        width: 200px;
        height: 40px;
        background: white;
        in property <string> text;
        in property <length> offset;
        forward-focus: ti;
        public function select-all() {
            ti.focus();
            ti.select-all();
        }
        Rectangle {
            clip: true;
            ti := TextInput {
                x: root.offset;
                width: self.preferred-width;
                text: root.text;
                font-size: 12px;
                single-line: true;
                color: transparent;
                selection-foreground-color: transparent;
                selection-background-color: #0f0;
            }
        }
    }

    // A rounded rectangle filling a window that may be as large as the renderer's range.
    export component Filled inherits Window {
        in property <length> ww;
        in property <length> wh;
        width: root.ww;
        height: root.wh;
        background: white;
        Rectangle {
            background: #f00;
            border-radius: 6px;
        }
    }

    export component FarPath inherits Window {
        width: 200px;
        height: 40px;
        background: white;
        in property <length> px;
        in property <length> pw;
        Path {
            x: root.px;
            width: root.pw;
            height: 40px;
            commands: "M 0 0 L 100 100";
            stroke: #000;
            stroke-width: 2px;
        }
    }
}

struct Frame {
    width: usize,
    pixels: Vec<Rgb8Pixel>,
}

impl core::fmt::Debug for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}x{} frame", self.width, self.pixels.len() / self.width.max(1))
    }
}

/// Compares two frames pixel by pixel, naming the first difference.
fn assert_same(far: &Frame, near: &Frame, what: &str) {
    assert_close(far, near, 0, what);
}

/// Like [`assert_same`], but lets each channel differ by up to `tolerance`.
fn assert_close(far: &Frame, near: &Frame, tolerance: u8, what: &str) {
    assert_eq!(far.width, near.width, "{what}: stride");
    for (i, (a, b)) in far.pixels.iter().zip(&near.pixels).enumerate() {
        let close = |x: u8, y: u8| x.abs_diff(y) <= tolerance;
        assert!(
            close(a.r, b.r) && close(a.g, b.g) && close(a.b, b.b),
            "{what}: first difference at ({}, {}): {a:?} and {b:?}",
            i % far.width,
            i / far.width
        );
    }
}

fn physical_size(scale: f32, rotation: RenderingRotation) -> (usize, usize) {
    let (w, h) = ((WIDTH * scale).round() as usize, (HEIGHT * scale).round() as usize);
    match rotation {
        RenderingRotation::Rotate90 | RenderingRotation::Rotate270 => (h, w),
        _ => (w, h),
    }
}

fn show(window: &slint::Window, scale: f32) {
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: scale });
    window.show().unwrap();
}

/// Renders a whole frame into a fresh buffer.
fn render(window: &MinimalSoftwareWindow, scale: f32, rotation: RenderingRotation) -> Frame {
    let (width, height) = physical_size(scale, rotation);
    let mut pixels = vec![Rgb8Pixel::new(0, 0, 0); width * height];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.set_rendering_rotation(rotation);
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render(pixels.as_mut_slice(), width);
    });
    Frame { width, pixels }
}

struct Lines<'a>(&'a mut Frame);

impl LineBufferProvider for Lines<'_> {
    type TargetPixel = Rgb8Pixel;
    fn process_line(
        &mut self,
        line: usize,
        range: core::ops::Range<usize>,
        render_fn: impl FnOnce(&mut [Rgb8Pixel]),
    ) {
        let start = line * self.0.width;
        render_fn(&mut self.0.pixels[start + range.start..start + range.end]);
    }
}

/// Renders a whole frame line by line, which goes through the scene the renderer prepares.
fn render_by_line(window: &MinimalSoftwareWindow, scale: f32) -> Frame {
    let (width, height) = physical_size(scale, RenderingRotation::NoRotation);
    let mut frame = Frame { width, pixels: vec![Rgb8Pixel::new(0, 0, 0); width * height] };
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.set_rendering_rotation(RenderingRotation::NoRotation);
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render_by_line(Lines(&mut frame));
    });
    frame
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f32,
    width: f32,
    y: f32,
    height: f32,
    radius: f32,
    border: f32,
    gradient: bool,
}

const PLAIN: Rect =
    Rect { x: 0., width: 0., y: 4., height: 32., radius: 0., border: 0., gradient: false };

fn shape_frames(r: Rect, scale: f32, rotation: RenderingRotation) -> (Frame, Frame) {
    let (w, h) = physical_size(scale, RenderingRotation::NoRotation);
    let window = common::setup(w as u32, h as u32);
    let app = Shape::new().unwrap();
    app.set_rx(r.x);
    app.set_rw(r.width);
    app.set_ry(r.y);
    app.set_rh(r.height);
    app.set_radius(r.radius);
    app.set_border(r.border);
    app.set_gradient(r.gradient);
    show(app.window(), scale);
    let frame = render(&window, scale, rotation);
    let lines = if rotation == RenderingRotation::NoRotation {
        render_by_line(&window, scale)
    } else {
        render(&window, scale, rotation)
    };
    app.hide().unwrap();
    (frame, lines)
}

/// Renders `far` and `near`, which differ only outside the window, and expects the same pixels.
fn assert_renders_like(far: Rect, near: Rect, scale: f32, rotation: RenderingRotation) {
    let what = format!("{far:?} like {near:?} at {scale}x, {rotation:?}");
    let (far_frame, far_lines) = shape_frames(far, scale, rotation);
    let (near_frame, near_lines) = shape_frames(near, scale, rotation);
    // Turned 90°, a vertical gradient's colors already shift by one with the shape's length,
    // also between two shapes that fit the 16-bit range.
    let tolerance = if far.gradient && rotation == RenderingRotation::Rotate90 { 1 } else { 0 };
    assert_close(&far_frame, &near_frame, tolerance, &what);
    assert_close(&far_lines, &near_lines, tolerance, &format!("{what}, by line"));
    // The near shape must show up, or the comparison proves nothing.
    assert!(near_frame.pixels.iter().any(|p| *p != Rgb8Pixel::new(255, 255, 255)), "{what}");
}

/// Each far shape and its near twin. The x offsets and widths are multiples of 4 logical
/// pixels, so they land on whole physical pixels at every tested scale.
fn twins() -> Vec<(Rect, Rect)> {
    let mut twins = Vec::new();
    for radius in [0., 6.] {
        for border in [0., 2.] {
            for gradient in [false, true] {
                let style = Rect { radius, border, gradient, ..PLAIN };
                // Both ends far outside.
                twins.push((
                    Rect { x: -40000., width: 80200., ..style },
                    Rect { x: -12., width: 224., ..style },
                ));
                // The left end far outside, the right one visible.
                twins.push((
                    Rect { x: -40000., width: 40100., ..style },
                    Rect { x: -12., width: 112., ..style },
                ));
                // The left end visible, the right one far outside.
                twins.push((
                    Rect { x: 100., width: 40000., ..style },
                    Rect { x: 100., width: 112., ..style },
                ));
            }
        }
    }
    twins
}

#[test]
fn far_rectangles_render_their_visible_part() {
    for scale in [1., 1.25, 1.5, 2.] {
        for (far, near) in twins() {
            assert_renders_like(far, near, scale, RenderingRotation::NoRotation);
        }
    }
}

#[test]
fn far_rectangles_render_their_visible_part_rotated() {
    for rotation in
        [RenderingRotation::Rotate90, RenderingRotation::Rotate180, RenderingRotation::Rotate270]
    {
        for (far, near) in twins() {
            assert_renders_like(far, near, 1., rotation);
        }
    }
}

/// Edges right at the end of the 16-bit range, at 1x so the logical pixels are physical ones.
#[test]
fn rectangles_at_the_edge_of_the_renderer_range() {
    for border in [0., 2.] {
        let style = Rect { border, ..PLAIN };
        for x in [-32767., -32768., -32769.] {
            assert_renders_like(
                Rect { x, width: 100. - x, ..style },
                Rect { x: -12., width: 112., ..style },
                1.,
                RenderingRotation::NoRotation,
            );
        }
        for width in [32767., 32768., 32769.] {
            assert_renders_like(
                Rect { x: 100., width, ..style },
                Rect { x: 100., width: 112., ..style },
                1.,
                RenderingRotation::NoRotation,
            );
        }
    }
}

/// The corners of a rounded rectangle with a wide border lie far outside the window, so in the
/// window it draws like the same rectangle without a radius: border where the border reaches
/// in, fill elsewhere.
#[test]
fn rounded_rectangles_with_a_wide_border_render_like_square_ones() {
    let shapes = [
        // The border ends some 1000 pixels before the window on every side.
        Rect { x: -10000., width: 20000., y: -10000., height: 20000., border: 9000., ..PLAIN },
        // The border reaches into the window on the left and at the bottom.
        Rect { x: -10000., width: 20400., y: -10080., height: 20160., border: 10050., ..PLAIN },
    ];
    for far in shapes {
        for scale in [1., 2.] {
            assert_renders_like(
                Rect { radius: 17., ..far },
                far,
                scale,
                RenderingRotation::NoRotation,
            );
        }
        for rotation in [
            RenderingRotation::Rotate90,
            RenderingRotation::Rotate180,
            RenderingRotation::Rotate270,
        ] {
            assert_renders_like(Rect { radius: 17., ..far }, far, 1., rotation);
        }
    }
}

/// A rounded rectangle as wide or as tall as the renderer's range draws its straight edges.
#[test]
fn rounded_rectangles_spanning_the_renderer_range() {
    let red = Rgb8Pixel::new(255, 0, 0);
    for extent in [32766, 32767] {
        for (width, height) in [(extent, 40), (200, extent)] {
            let what = format!("{width}x{height}");
            let window = common::setup(width as u32, height as u32);
            let app = Filled::new().unwrap();
            app.set_ww(width as f32);
            app.set_wh(height as f32);
            show(app.window(), 1.);
            let mut frame = Frame { width, pixels: vec![Rgb8Pixel::new(0, 0, 0); width * height] };
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
                renderer.render(frame.pixels.as_mut_slice(), width);
            });
            let mut lines = Frame { width, pixels: vec![Rgb8Pixel::new(0, 0, 0); width * height] };
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
                renderer.render_by_line(Lines(&mut lines));
            });
            app.hide().unwrap();
            assert_same(&frame, &lines, &what);
            // The middle of each edge and the center are inside the rectangle.
            for (x, y) in [
                (width / 2, 0),
                (width / 2, height - 1),
                (0, height / 2),
                (width - 1, height / 2),
                (width / 2, height / 2),
            ] {
                assert_eq!(frame.pixels[y * width + x], red, "{what} at ({x}, {y})");
            }
        }
    }
}

#[test]
fn rectangles_without_a_finite_geometry_render_nothing() {
    for (x, width) in
        [(f32::NAN, 100.), (0., f32::NAN), (0., f32::INFINITY), (f32::NEG_INFINITY, 100.)]
    {
        let (frame, lines) =
            shape_frames(Rect { x, width, border: 2., ..PLAIN }, 1., RenderingRotation::NoRotation);
        let white = Rgb8Pixel::new(255, 255, 255);
        assert!(frame.pixels.iter().all(|p| *p == white), "x {x}, width {width}");
        assert!(lines.pixels.iter().all(|p| *p == white), "x {x}, width {width}, by line");
    }
}

fn selection_frame(text: &str, offset: f32, scale: f32) -> (Frame, Frame) {
    let (w, h) = physical_size(scale, RenderingRotation::NoRotation);
    let window = common::setup(w as u32, h as u32);
    let app = Selection::new().unwrap();
    app.set_text(text.into());
    app.set_offset(offset);
    show(app.window(), scale);
    app.window().dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    app.invoke_select_all();
    let frame = render(&window, scale, RenderingRotation::NoRotation);
    let lines = render_by_line(&window, scale);
    app.hide().unwrap();
    (frame, lines)
}

/// A highlight some 60000 pixels wide, starting far left of the window or in it, covers the
/// window like the highlight of a line just wider than the window.
#[test]
fn a_far_selection_renders_its_visible_part() {
    let long = "W".repeat(6000);
    let short = "W".repeat(30);
    for scale in [1., 1.25, 1.5, 2.] {
        let (near, near_lines) = selection_frame(&short, 0., scale);
        assert!(near.pixels.contains(&Rgb8Pixel::new(0, 255, 0)), "the highlight shows");
        for offset in [-59800., 0.] {
            let what = format!("offset {offset} at {scale}x");
            let (far, far_lines) = selection_frame(&long, offset, scale);
            assert_same(&far, &near, &what);
            assert_same(&far_lines, &near_lines, &format!("{what}, by line"));
        }
    }
}

/// A `Path` whose box does not fit the renderer's range can't be fitted to its visible part,
/// so it is skipped as a whole. One that fits still draws.
#[test]
fn a_path_past_the_renderer_range_is_skipped() {
    let draw = |px: f32, pw: f32| {
        let window = common::setup(WIDTH as u32, HEIGHT as u32);
        let app = FarPath::new().unwrap();
        app.set_px(px);
        app.set_pw(pw);
        show(app.window(), 1.);
        let frame = render(&window, 1., RenderingRotation::NoRotation);
        app.hide().unwrap();
        frame.pixels.iter().filter(|p| **p != Rgb8Pixel::new(255, 255, 255)).count()
    };
    assert!(draw(0., 200.) > 0);
    assert_eq!(draw(-40000., 40100.), 0);
    assert_eq!(draw(0., 40000.), 0);
}
