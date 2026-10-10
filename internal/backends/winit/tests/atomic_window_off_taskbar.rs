// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH (atomic-tool-window): an atomically-presented window is a tool window from its
//! first map, so the taskbar never gets a button for it.
//!
//! The taskbar decides on a button when a window is shown: a window without an owner gets one
//! unless it is a tool window, and `WS_EX_APPWINDOW` forces one. winit gives every such window
//! `WS_EX_APPWINDOW`. Changing the style of a window already shown does not remove a button.
//!
//! A thread-local `WH_CALLWNDPROC` hook reads the extended style when the window receives its
//! first `WM_SHOWWINDOW`, before it is visible. It shows a window that does not take the
//! foreground.

#[cfg(not(all(windows, feature = "renderer-software")))]
fn main() {}

#[cfg(all(windows, feature = "renderer-software"))]
fn main() {
    use std::sync::Mutex;
    use std::time::Duration;

    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const WH_CALLWNDPROC: i32 = 4;
    const WM_SHOWWINDOW: u32 = 0x0018;
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOOLWINDOW: isize = 0x80;
    const WS_EX_APPWINDOW: isize = 0x40000;

    #[repr(C)]
    struct CwpStruct {
        lparam: isize,
        wparam: usize,
        message: u32,
        hwnd: isize,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowsHookExW(
            id: i32,
            hook: unsafe extern "system" fn(i32, usize, isize) -> isize,
            module: isize,
            thread_id: u32,
        ) -> isize;
        fn UnhookWindowsHookEx(hook: isize) -> i32;
        fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    /// The extended style of each window when it was first shown.
    static SHOWN: Mutex<Vec<(isize, isize)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn on_message(code: i32, wparam: usize, lparam: isize) -> isize {
        let message = unsafe { &*(lparam as *const CwpStruct) };
        if code >= 0 && message.message == WM_SHOWWINDOW && message.wparam != 0 {
            let mut shown = SHOWN.lock().unwrap();
            if !shown.iter().any(|(hwnd, _)| *hwnd == message.hwnd) {
                let ex_style = unsafe { GetWindowLongPtrW(message.hwnd, GWL_EXSTYLE) };
                shown.push((message.hwnd, ex_style));
            }
        }
        unsafe { CallNextHookEx(0, code, wparam, lparam) }
    }

    slint::slint! {
        export component Bar inherits Window {
            no-frame: true;
            width: 120px;
            height: 40px;
        }
    }

    slint::BackendSelector::new().backend_name("winit".into()).select().unwrap();
    let hook = unsafe { SetWindowsHookExW(WH_CALLWNDPROC, on_message, 0, GetCurrentThreadId()) };
    assert_ne!(hook, 0, "SetWindowsHookExW failed");

    i_slint_backend_winit::atomic_presentation::arm_next_window();
    let bar = Bar::new().unwrap();
    let path = i_slint_backend_winit::atomic_presentation::show(bar.window(), false).unwrap();
    assert_eq!(path, i_slint_backend_winit::atomic_presentation::ShowPath::Atomic);

    let bar_weak = bar.as_weak();
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        let bar = bar_weak.upgrade().unwrap();
        let raw = bar
            .window()
            .with_winit_window(|window| window.window_handle().unwrap().as_raw())
            .unwrap();
        let RawWindowHandle::Win32(handle) = raw else { unreachable!() };
        let hwnd = handle.hwnd.get();
        let shown = SHOWN.lock().unwrap();
        let (_, ex_style) =
            *shown.iter().find(|(shown, _)| *shown == hwnd).expect("the window was never shown");
        println!("extended style when first shown: {ex_style:#x}");
        assert!(ex_style & WS_EX_TOOLWINDOW != 0, "not a tool window when first shown");
        assert!(ex_style & WS_EX_APPWINDOW == 0, "WS_EX_APPWINDOW when first shown");
        slint::quit_event_loop().unwrap();
    });
    slint::run_event_loop_until_quit().unwrap();
    unsafe { UnhookWindowsHookEx(hook) };
    println!("atomic_window_off_taskbar: ok");
}
