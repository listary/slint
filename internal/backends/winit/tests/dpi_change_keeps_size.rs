// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH (scale-without-resize): a maximized window stays maximized, at its size, under
//! a new scale.
//!
//! On a DPI change winit keeps the size of a maximized or fullscreen window, so no `Resized`
//! follows the new scale. The window item kept its old logical size, so every frame asked for
//! the old logical size at the new scale, which the software renderer could not fit in the
//! buffer. Then the window properties took that stale logical size for a size the window should
//! have, and resized the native window to it, which also took it out of the maximized state.
//!
//! The window is not really maximized: a `WM_SIZE` with `SIZE_MAXIMIZED` sets winit's flag, and
//! the `WM_DPICHANGED` is sent directly. It creates a window that does not take the foreground.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    use std::time::Duration;

    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const WM_SIZE: u32 = 0x0005;
    const SIZE_MAXIMIZED: usize = 2;
    const WM_DPICHANGED: u32 = 0x02E0;

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
    }

    slint::slint! {
        export component App inherits Window {
            out property <length> logical-width: root.width;
            out property <length> logical-height: root.height;
        }
    }

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
        .select()
        .unwrap();

    let app = App::new().unwrap();
    app.window().set_size(slint::LogicalSize::new(100., 80.));
    app.show().unwrap();

    let app_weak = app.as_weak();
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        let app_weak = app_weak.clone();
        slint::invoke_from_event_loop(move || {
            let app = app_weak.upgrade().unwrap();
            let raw = app
                .window()
                .with_winit_window(|window| window.window_handle().unwrap().as_raw())
                .unwrap();
            let RawWindowHandle::Win32(handle) = raw else { unreachable!() };
            let hwnd = handle.hwnd.get();

            let size = app.window().size();
            let lparam = ((size.height as isize) << 16) | size.width as isize;
            unsafe { SendMessageW(hwnd, WM_SIZE, SIZE_MAXIMIZED, lparam) };

            assert!(app.window().with_winit_window(|w| w.is_maximized()).unwrap());
            let new_dpi: usize = if app.window().scale_factor() == 2.0 { 144 } else { 192 };
            let mut rect = Rect::default();
            unsafe { GetWindowRect(hwnd, &mut rect) };
            unsafe {
                SendMessageW(hwnd, WM_DPICHANGED, (new_dpi << 16) | new_dpi, &rect as *const _ as _)
            };

            slint::Timer::single_shot(Duration::from_millis(300), move || {
                let app = app_weak.upgrade().unwrap();
                let scale = app.window().scale_factor();
                let physical = app.window().size();
                let logical = (app.get_logical_width(), app.get_logical_height());
                let maximized = app.window().with_winit_window(|w| w.is_maximized()).unwrap();
                println!(
                    "scale {scale}, size {physical:?}, logical {logical:?}, maximized {maximized}"
                );
                assert_eq!(scale, new_dpi as f32 / 96.);
                assert!(maximized, "the window is no longer maximized");
                assert_eq!(physical, size, "the window changed its size");
                assert!(
                    (logical.0 * scale - physical.width as f32).abs() < 1.
                        && (logical.1 * scale - physical.height as f32).abs() < 1.,
                    "the logical size {logical:?} at scale {scale} is not the size {physical:?}"
                );
                slint::quit_event_loop().unwrap();
            });
        })
        .unwrap();
    });
    slint::run_event_loop_until_quit().unwrap();
    println!("dpi_change_keeps_size: ok");
}
