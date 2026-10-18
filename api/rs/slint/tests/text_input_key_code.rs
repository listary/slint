// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Standard editing commands in `TextInput` recognize the key by its Windows virtual key code,
//! so they keep working when Caps Lock is on or the layout produces non-Latin text.

#![cfg(target_os = "windows")]

use i_slint_core::input::{InternalKeyEvent, KeyEvent, KeyEventType};
use i_slint_core::platform::Clipboard;
use i_slint_core::window::WindowInner;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

slint::slint! {
    export component TestCase inherits Window {
        width: 200px;
        height: 50px;
        in-out property <string> text <=> input.text;
        in property <bool> read-only <=> input.read-only;
        out property <int> cursor: input.cursor-position-byte-offset;
        out property <int> anchor: input.anchor-position-byte-offset;
        out property <int> pressed-code;
        out property <bool> pressed-repeat;
        out property <int> released-code;
        input := TextInput {
            key-pressed(event) => {
                root.pressed-code = event.windows-virtual-key;
                root.pressed-repeat = event.repeat;
                reject
            }
            key-released(event) => {
                root.released-code = event.windows-virtual-key;
                reject
            }
        }
        public function focus-input() {
            input.focus();
        }
        public function select(anchor: int, cursor: int) {
            input.set-selection-offsets(anchor, cursor);
        }
    }
}

const VK_A: i32 = 0x41;
const VK_C: i32 = 0x43;
const VK_V: i32 = 0x56;
const VK_X: i32 = 0x58;
const VK_Y: i32 = 0x59;
const VK_Z: i32 = 0x5A;
const VK_INSERT: i32 = 0x2D;
const VK_DELETE: i32 = 0x2E;

fn setup(text: &str) -> TestCase {
    i_slint_backend_testing::init_no_event_loop();
    let instance = TestCase::new().unwrap();
    instance.show().unwrap();
    instance.set_text(text.into());
    instance.invoke_focus_input();
    set_clipboard(&instance, "clip");
    instance
}

fn set_clipboard(instance: &TestCase, text: &str) {
    WindowInner::from_pub(instance.window())
        .context()
        .platform()
        .set_clipboard_text(text, Clipboard::DefaultClipboard);
}

fn clipboard(instance: &TestCase) -> String {
    WindowInner::from_pub(instance.window())
        .context()
        .platform()
        .clipboard_text(Clipboard::DefaultClipboard)
        .unwrap_or_default()
}

fn modifier(instance: &TestCase, key: Key, pressed: bool) {
    let text: SharedString = key.into();
    instance.window().dispatch_event(if pressed {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
}

fn key_event(
    text: impl Into<SharedString>,
    code: i32,
    event_type: KeyEventType,
    repeat: bool,
) -> InternalKeyEvent {
    let mut key_event = KeyEvent::default();
    key_event.text = text.into();
    key_event.windows_virtual_key = code;
    key_event.repeat = repeat;
    InternalKeyEvent { key_event, event_type, ..Default::default() }
}

/// Presses and releases one key, as the winit backend reports it on Windows.
fn tap(instance: &TestCase, text: impl Into<SharedString>, code: i32) {
    let text = text.into();
    let window = instance.window();
    window.dispatch_event(WindowEvent::internal(key_event(
        text.clone(),
        code,
        KeyEventType::KeyPressed,
        false,
    )));
    window.dispatch_event(WindowEvent::internal(key_event(
        text,
        code,
        KeyEventType::KeyReleased,
        false,
    )));
}

fn with_modifiers(instance: &TestCase, modifiers: &[Key], f: impl FnOnce()) {
    for m in modifiers {
        modifier(instance, *m, true);
    }
    f();
    for m in modifiers.iter().rev() {
        modifier(instance, *m, false);
    }
}

fn ctrl_tap(instance: &TestCase, text: impl Into<SharedString>, code: i32) {
    with_modifiers(instance, &[Key::Control], || tap(instance, text, code));
}

#[test]
fn key_code_reaches_callbacks() {
    let instance = setup("");
    let window = instance.window();
    window.dispatch_event(WindowEvent::internal(key_event(
        "v",
        VK_V,
        KeyEventType::KeyPressed,
        true,
    )));
    assert_eq!(instance.get_pressed_code(), VK_V);
    assert!(instance.get_pressed_repeat());
    window.dispatch_event(WindowEvent::internal(key_event(
        "v",
        VK_V,
        KeyEventType::KeyReleased,
        false,
    )));
    assert_eq!(instance.get_released_code(), VK_V);

    // Events without a key code, such as the ones dispatched through the public API, report 0.
    window.dispatch_event(WindowEvent::KeyPressed { text: "w".into() });
    assert_eq!(instance.get_pressed_code(), 0);
    window.dispatch_event(WindowEvent::KeyReleased { text: "w".into() });
    assert_eq!(instance.get_released_code(), 0);
}

#[test]
fn ctrl_v_pastes_with_caps_lock() {
    let instance = setup("");
    ctrl_tap(&instance, "V", VK_V);
    assert_eq!(instance.get_text(), "clip");
}

#[test]
fn ctrl_v_pastes_with_caps_lock_without_key_code() {
    let instance = setup("");
    ctrl_tap(&instance, "V", 0);
    assert_eq!(instance.get_text(), "clip");
}

#[test]
fn editing_commands_on_russian_layout() {
    let instance = setup("abc");

    ctrl_tap(&instance, "ф", VK_A);
    assert_eq!((instance.get_anchor(), instance.get_cursor()), (0, 3), "Ctrl+A selects all");

    ctrl_tap(&instance, "с", VK_C);
    assert_eq!(clipboard(&instance), "abc", "Ctrl+C copies");

    instance.invoke_select(0, 1);
    ctrl_tap(&instance, "ч", VK_X);
    assert_eq!(instance.get_text(), "bc", "Ctrl+X cuts");
    assert_eq!(clipboard(&instance), "a");

    ctrl_tap(&instance, "я", VK_Z);
    assert_eq!(instance.get_text(), "abc", "Ctrl+Z undoes");

    ctrl_tap(&instance, "н", VK_Y);
    assert_eq!(instance.get_text(), "bc", "Ctrl+Y redoes");

    instance.invoke_select(2, 2);
    ctrl_tap(&instance, "м", VK_V);
    assert_eq!(instance.get_text(), "bca", "Ctrl+V pastes");
}

#[test]
fn ctrl_with_russian_text_and_no_key_code_is_not_a_command() {
    let instance = setup("abc");
    ctrl_tap(&instance, "м", 0);
    assert_eq!(instance.get_text(), "abc");
}

#[test]
fn ctrl_shift_v_is_not_paste() {
    let instance = setup("");
    with_modifiers(&instance, &[Key::Control, Key::Shift], || tap(&instance, "V", VK_V));
    assert_eq!(instance.get_text(), "");
}

#[test]
fn altgr_letter_is_text_not_a_command() {
    // Polish (Programmers): AltGr+A types "ą". Windows reports AltGr as Ctrl+Alt.
    let instance = setup("");
    with_modifiers(&instance, &[Key::Control, Key::Alt], || {
        let window = instance.window();
        for event_type in [KeyEventType::KeyPressed, KeyEventType::KeyReleased] {
            let mut event = key_event("ą", VK_A, event_type, false);
            event.text_without_modifiers = "a".into();
            window.dispatch_event(WindowEvent::internal(event));
        }
    });
    assert_eq!(instance.get_text(), "ą");
    assert_eq!(instance.get_anchor(), instance.get_cursor());
}

#[test]
fn ctrl_insert_copies() {
    let instance = setup("abc");
    instance.invoke_select(0, 2);
    ctrl_tap(&instance, Key::Insert, VK_INSERT);
    assert_eq!(clipboard(&instance), "ab");
    assert_eq!(instance.get_text(), "abc");
}

#[test]
fn shift_insert_pastes() {
    let instance = setup("abc");
    instance.invoke_select(3, 3);
    with_modifiers(&instance, &[Key::Shift], || tap(&instance, Key::Insert, VK_INSERT));
    assert_eq!(instance.get_text(), "abcclip");
}

#[test]
fn shift_delete_cuts_the_selection() {
    let instance = setup("abc");
    instance.invoke_select(1, 3);
    with_modifiers(&instance, &[Key::Shift], || tap(&instance, Key::Delete, VK_DELETE));
    assert_eq!(instance.get_text(), "a");
    assert_eq!(clipboard(&instance), "bc");

    ctrl_tap(&instance, "z", VK_Z);
    assert_eq!(instance.get_text(), "abc");
}

#[test]
fn shift_delete_without_selection_deletes_forward() {
    let instance = setup("abc");
    instance.invoke_select(1, 1);
    with_modifiers(&instance, &[Key::Shift], || tap(&instance, Key::Delete, VK_DELETE));
    assert_eq!(instance.get_text(), "ac");
    assert_eq!(clipboard(&instance), "clip");
}

#[test]
fn read_only_copies_but_does_not_change_text() {
    let instance = setup("abc");
    instance.set_read_only(true);
    instance.invoke_select(0, 2);

    with_modifiers(&instance, &[Key::Shift], || tap(&instance, Key::Delete, VK_DELETE));
    assert_eq!(instance.get_text(), "abc", "Shift+Delete");
    assert_eq!(clipboard(&instance), "clip", "Shift+Delete");

    with_modifiers(&instance, &[Key::Shift], || tap(&instance, Key::Insert, VK_INSERT));
    assert_eq!(instance.get_text(), "abc", "Shift+Insert");

    ctrl_tap(&instance, "м", VK_V);
    assert_eq!(instance.get_text(), "abc", "Ctrl+V");

    ctrl_tap(&instance, Key::Insert, VK_INSERT);
    assert_eq!(clipboard(&instance), "ab", "Ctrl+Insert");
}
