// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! The software renderer draws in 16-bit physical coordinates.
//! Geometry that reaches past ±32767 pixels, but whose visible part lies inside the window,
//! still renders without panicking.

mod common;

use common::TestPixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

const WIDTH: usize = 200;
const HEIGHT: usize = 32;

/// 6000 "W" at 12px are about 60000 pixels wide.
fn long_text() -> slint::SharedString {
    "W".repeat(6000).into()
}

/// Renders a whole frame and counts the pixels drawn.
fn render(window: &MinimalSoftwareWindow) -> usize {
    let mut buffer = vec![TestPixel(false); WIDTH * HEIGHT];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
        renderer.render(buffer.as_mut_slice(), WIDTH);
    });
    buffer.iter().filter(|p| p.0).count()
}

slint::slint! {
    // A line slid left inside a clipping parent to keep the caret in view, the way an editable
    // single-line box scrolls: Slint's `TextInput` never scrolls on its own.
    export component ScrolledLine inherits Window {
        width: 200px;
        height: 32px;
        in property <string> text;
        in property <bool> scrolled-to-end;
        forward-focus: ti;
        public function focus-input() {
            ti.focus();
        }
        public function select(anchor: int, cursor: int) {
            ti.set-selection-offsets(anchor, cursor);
        }
        Rectangle {
            clip: true;
            ti := TextInput {
                text: root.text;
                font-size: 12px;
                single-line: true;
                width: max(parent.width, self.preferred-width);
                x: root.scrolled-to-end ? parent.width - self.width : 0px;
            }
        }
    }

    export component Unscrolled inherits Window {
        width: 200px;
        height: 32px;
        in property <string> text;
        in property <bool> single-line;
        forward-focus: ti;
        public function focus-input() {
            ti.focus();
        }
        public function select(anchor: int, cursor: int) {
            ti.set-selection-offsets(anchor, cursor);
        }
        ti := TextInput {
            text: root.text;
            font-size: 12px;
            single-line: root.single-line;
            wrap: no-wrap;
        }
    }

    export component Shapes inherits Window {
        width: 200px;
        height: 32px;
        in property <int> which;
        in property <image> picture;
        in property <string> text;
        if root.which == 0: Rectangle {
            x: -20000px;
            width: 40000px;
            background: red;
        }
        if root.which == 1: Rectangle {
            x: -20000px;
            width: 40000px;
            border-width: 2px;
            border-color: blue;
        }
        if root.which == 2: Rectangle {
            x: -40000px;
            width: 40200px;
            background: red;
        }
        if root.which == 3: Image {
            x: -20000px;
            width: 40000px;
            source: root.picture;
            image-fit: fill;
        }
        if root.which == 4: Text {
            x: -50000px;
            width: 70000px;
            text: root.text;
            font-size: 12px;
        }
    }
}

fn scrolled_line(scrolled_to_end: bool, anchor: i32, cursor: i32) -> usize {
    let window = common::setup(WIDTH as u32, HEIGHT as u32);
    let app = ScrolledLine::new().unwrap();
    app.set_text(long_text());
    app.set_scrolled_to_end(scrolled_to_end);
    app.show().unwrap();
    app.window().dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    app.invoke_focus_input();
    app.invoke_select(anchor, cursor);
    render(&window)
}

fn unscrolled(single_line: bool, anchor: i32, cursor: i32) -> usize {
    let window = common::setup(WIDTH as u32, HEIGHT as u32);
    let app = Unscrolled::new().unwrap();
    app.set_text(long_text());
    app.set_single_line(single_line);
    app.show().unwrap();
    app.window().dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    app.invoke_focus_input();
    app.invoke_select(anchor, cursor);
    render(&window)
}

fn len() -> i32 {
    long_text().len() as i32
}

/// Select all with the caret at the end: the line is scrolled to its end, so the highlight
/// starts some 60000 pixels left of the window.
#[test]
fn a_selection_starting_far_left_of_the_window_renders() {
    assert!(scrolled_line(true, 0, len()) > 0);
}

/// Select all with the caret at the start: the highlight starts in the window and is some
/// 60000 pixels wide.
#[test]
fn a_selection_wider_than_the_renderer_range_renders() {
    assert!(scrolled_line(false, len(), 0) > 0);
}

/// The scrolled line keeps the caret in view.
#[test]
fn a_caret_scrolled_into_view_at_the_end_of_a_long_line_renders() {
    assert!(scrolled_line(true, len(), len()) > 0);
}

/// A `TextInput` that is not scrolled draws its caret where the text ends, far outside.
#[test]
fn a_caret_far_right_of_the_window_renders() {
    assert!(unscrolled(true, len(), len()) > 0);
}

#[test]
fn a_selection_in_a_multi_line_input_without_wrapping_renders() {
    assert!(unscrolled(false, 0, len()) > 0);
}

fn shape(which: i32) -> usize {
    let window = common::setup(WIDTH as u32, HEIGHT as u32);
    let app = Shapes::new().unwrap();
    app.set_picture(Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::new(6000, 2)));
    app.set_text(long_text());
    app.set_which(which);
    app.show().unwrap();
    render(&window)
}

#[test]
fn a_filled_rectangle_wider_than_the_renderer_range_renders() {
    assert_eq!(shape(0), WIDTH * HEIGHT);
}

#[test]
fn a_bordered_rectangle_wider_than_the_renderer_range_renders() {
    assert!(shape(1) > 0);
}

#[test]
fn a_rectangle_starting_beyond_the_renderer_range_renders() {
    assert_eq!(shape(2), WIDTH * HEIGHT);
}

#[test]
fn an_image_wider_than_the_renderer_range_renders() {
    shape(3);
}

#[test]
fn a_text_starting_far_left_of_the_window_renders() {
    shape(4);
}
