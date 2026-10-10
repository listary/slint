// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH (render-size-fits-buffer): rendering a window whose size disagrees with the
//! buffer does not panic.
//!
//! A backend sizes the buffer from the native window's physical size, while the renderer sizes
//! the frame from the window item's logical size times the scale factor. On Windows the new scale
//! arrives before the new size, so a frame painted in between asks for more than the buffer has.

mod common;

use common::TestPixel;
use slint::LogicalSize;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{WindowAdapter, WindowEvent};

slint::slint! {
    export component App inherits Window {
        Rectangle { background: red; }
    }
}

const WIDTH: usize = 160;
const HEIGHT: usize = 28;

/// Renders a whole frame into a new 160x28 buffer and counts the pixels drawn.
fn render(window: &MinimalSoftwareWindow) -> usize {
    let mut buffer = vec![TestPixel(false); WIDTH * HEIGHT];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render(buffer.as_mut_slice(), WIDTH);
    });
    buffer.iter().filter(|p| p.0).count()
}

#[test]
fn a_frame_between_a_new_scale_and_the_new_size_fills_the_old_buffer() {
    let window = common::setup(WIDTH as u32, HEIGHT as u32);
    let app = App::new().unwrap();
    app.show().unwrap();
    assert_eq!(render(&window), WIDTH * HEIGHT);

    // The buffer stays 160x28 while the window asks for 240x42 until the new size arrives.
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    assert_eq!(render(&window), WIDTH * HEIGHT);
}

#[test]
fn a_window_size_out_of_the_renderer_range_skips_the_frame() {
    let window = common::setup(WIDTH as u32, HEIGHT as u32);
    let app = App::new().unwrap();
    app.show().unwrap();

    // 40000 logical pixels do not fit the renderer's 16-bit physical coordinates.
    window.dispatch_event(WindowEvent::Resized { size: LogicalSize::new(40000., HEIGHT as f32) });
    assert_eq!(render(&window), 0);

    window.dispatch_event(WindowEvent::Resized { size: LogicalSize::new(f32::NAN, f32::NAN) });
    assert_eq!(render(&window), 0);

    window.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    assert_eq!(render(&window), WIDTH * HEIGHT);
}
