// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH (accesskit-reentrant-getobject): a change handler that is still pending
//! when a UI Automation client asks for the tree runs before AccessKit is borrowed.
//!
//! Building the tree for AccessKit first brings the item tree up to date, which runs pending
//! change handlers. A handler that moves the focus makes winit enable the input method, and the
//! input method pumps messages, delivering `WM_GETOBJECT` to the same window. If that happens
//! inside `update_if_active`, AccessKit's window procedure borrows its state a second time and
//! panics. The handler here sends that `WM_GETOBJECT` itself, so no input method is needed.
//!
//! Needs the `accessibility` feature; it creates a window that does not take the foreground.

#[cfg(not(all(windows, feature = "accessibility")))]
fn main() {}

#[cfg(all(windows, feature = "accessibility"))]
fn main() {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::time::Duration;

    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const WM_GETOBJECT: u32 = 0x003D;
    const OBJID_CLIENT: isize = -4;
    const UIA_ROOT_OBJECT_ID: isize = -25;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    slint::slint! {
        export component App inherits Window {
            width: 100px;
            height: 100px;
            in property <int> trigger;
            callback trigger-changed();
            changed trigger => { root.trigger-changed(); }
        }
    }

    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
        .select()
        .unwrap();

    static HWND: AtomicIsize = AtomicIsize::new(0);

    let app = App::new().unwrap();
    let handler_ran = Rc::new(Cell::new(false));
    app.on_trigger_changed({
        let handler_ran = handler_ran.clone();
        move || {
            // What the input method does when the focus moves.
            unsafe { SendMessageW(HWND.load(Ordering::Relaxed), WM_GETOBJECT, 0, OBJID_CLIENT) };
            handler_ran.set(true);
        }
    });
    app.show().unwrap();

    let app_weak = app.as_weak();
    // Timers run right before pending change handlers, so set the property from an event.
    slint::Timer::single_shot(Duration::from_millis(200), move || {
        let app_weak = app_weak.clone();
        slint::invoke_from_event_loop(move || {
            let app = app_weak.upgrade().unwrap();
            let raw = app
                .window()
                .with_winit_window(|window| window.window_handle().unwrap().as_raw())
                .unwrap();
            let RawWindowHandle::Win32(handle) = raw else { unreachable!() };
            HWND.store(handle.hwnd.get(), Ordering::Relaxed);
            app.set_trigger(1);
            // A UI Automation client asks for the root: AccessKit queues `InitialTreeRequested`,
            // which the event loop handles before the change handler of `trigger` runs.
            unsafe { SendMessageW(handle.hwnd.get(), WM_GETOBJECT, 0, UIA_ROOT_OBJECT_ID) };
            slint::Timer::single_shot(Duration::from_millis(500), || {
                slint::quit_event_loop().unwrap();
            });
        })
        .unwrap();
    });
    slint::run_event_loop_until_quit().unwrap();

    assert!(handler_ran.get(), "the change handler never ran");
    println!("accesskit_reentrant_getobject: ok");
}
