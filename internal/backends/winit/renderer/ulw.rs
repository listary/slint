// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! LISTARY PATCH (R-89) — **whole file is ours**, there is no upstream counterpart.
//!
//! # Atomic presentation: the software renderer draws straight into a resident DIB, and
//! one `UpdateLayeredWindow` carries position + size + pixels in a single system call.
//!
//! ## Why this exists
//!
//! An ordinary window puts a frame on screen as **two independent transactions**:
//! `SetWindowPos` changes the rectangle (effective immediately) and the present swaps the
//! content (effective on the next compositor tick). DWM samples on its own clock, and a
//! sample taken between the two pins the *old* picture onto the *new* origin — the
//! displacement frame. Every ordering/racing scheme can only lower its probability;
//! `UpdateLayeredWindow` removes the intermediate state from the vocabulary, because the
//! new position, the new size and the complete new picture arrive in one call and the
//! system switches them together (MS documents this exact use: "change the position and
//! the size … permits seamless animation").
//!
//! Measured on the research rig (two independent instruments, `docs/reviews/
//! 260830-ulw-atomic-presentation.md`): 0/1280 and 0/1600 defective frames against a
//! sensitivity control of 116/120, submit cost p50 0.67 ms / p95 1.77 ms.
//!
//! ## The two style bits — the only birth requirement
//!
//! R-87c bisected 17 window configurations to the bit: a window that **lacks
//! `WS_POPUP`** or **carries `WS_CAPTION`** bleeds 40/40 even with otherwise identical
//! ULW code; `SIZEBOX` / `BORDER` / `SYSMENU` / `CLIPSIBLINGS` / a `WM_NCCALCSIZE→0`
//! surgery / `SWP_FRAMECHANGED` are all harmless. winit's own form (`WS_CAPTION` kept on
//! purpose so aero-snap works, plus no `WS_POPUP` unless the window has an owner) steps on
//! both mines, and `WindowAttributes` cannot express either bit — so
//! [`enforce_atomic_window_style`] applies them directly, and re-applies them
//! unconditionally on every present. That is not belt-and-braces: winit's
//! `WindowFlags::apply_diff` recomputes **both** style words from its own cached flags
//! whenever any flag changes (`winit-0.30.13` `window_state.rs:390`), which would strip
//! `WS_EX_LAYERED` right back off. Keeping the invariant is one `GetWindowLongPtrW` per
//! frame; reasoning about who may perturb it is a distributed protocol.
//!
//! ## Geometry is intercepted, not mirrored
//!
//! The host keeps calling `slint::Window::set_position` / `set_size`. For a window in this
//! mode the adapter does **not** hand those to winit; the origin is stashed here
//! ([`AtomicPresentation::stash_origin`]) and the size goes straight into the adapter's
//! size cache + a synchronous `Resized` dispatch. Both become real on the next present, in
//! the one `UpdateLayeredWindow`. This is the whole point of the mode, and it is why the
//! rule lives at exactly one place instead of at every call site that moves a bar.
//!
//! ## Every show chooses whether to activate (260924)
//!
//! Mapping goes through [`AtomicPresentation::set_mapped`], never through winit (see the
//! adapter's `map_native_window`). Whether a show activates is decided per show, the way
//! WPF's `ShowActivated` is read on every `Show()`: the product calls
//! `crate::atomic_presentation::show(window, activate)`, which leaves its choice here
//! through [`AtomicPresentation::set_next_map_activation`], and the next map consumes it.
//! A show nobody chose for (a plain `slint::Window::show()`) does not activate. The two
//! commands are WPF's own (`Window.nCmdForShow`): `SW_SHOW` to activate, `SW_SHOWNA`
//! otherwise. `SW_SHOWNOACTIVATE` would also restore a minimized or maximized window to
//! its normal placement, which a frameless bar never needs and which would move the window
//! by some other rule than the present that placed it.
//!
//! winit's creation-time `attributes.active` plays no part: these windows are created
//! hidden and winit never maps them. A click still activates them, since they carry no
//! `WS_EX_NOACTIVATE`.
//!
//! ## `SetLayeredWindowAttributes` is forbidden
//!
//! One call to it puts a layered window into the "constant alpha / color key" mode and
//! every subsequent `UpdateLayeredWindow` fails with `ERROR_INVALID_PARAMETER` (87,
//! measured) until the `WS_EX_LAYERED` bit is toggled off and on again. The product has
//! zero call sites and a guard test keeps it that way
//! (`listary-ui/tests/slint_discipline.rs`).
//!
//! ## What this mode deliberately does not carry
//!
//! No dirty-region submit (a full 442×N GDI blit is ~0.2 ms at bar sizes, and tracking
//! damage across a resize is pure complexity), no shadow underlay and no nine-slice (the
//! two bars have neither rounded corners nor a shadow — the C# originals do not either),
//! no GPU read-back path (that is the archived B form, only worth it if a single window's
//! software render ever exceeds ~2.5 ms; the measured worst real bar frame is 0.93 ms).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use i_slint_core::platform::PlatformError;
use i_slint_core::renderer::DrawOutcome;
use i_slint_renderer_software::RepaintBufferType;
use winit::event_loop::ActiveEventLoop;

use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GdiFlush, GetDC, ReleaseDC,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, ShowWindow, UpdateLayeredWindow,
    GWL_EXSTYLE, GWL_STYLE, SW_HIDE, SW_SHOW, SW_SHOWNA, ULW_ALPHA, WS_CAPTION, WS_EX_LAYERED,
    WS_POPUP,
};

use super::{AtomicPresentation, WinitCompatibleRenderer};

/// The DIB is allocated in steps so that a bar that grows one result row at a time does not
/// reallocate on every publication. 64 physical pixels is one step; at 442×N bar widths the
/// slack costs tens of kilobytes.
const CAPACITY_STEP: i32 = 64;

/// **The one place the two birth bits are asserted.** Idempotent, unconditional, cheap —
/// see the module header for why it runs on every present rather than "once, carefully".
fn enforce_atomic_window_style(hwnd: HWND) {
    // SAFETY: `hwnd` comes from the winit window this renderer created and is alive for as
    // long as the surface it is stored in.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let wanted = (style | WS_POPUP.0) & !WS_CAPTION.0;
        if wanted != style {
            SetWindowLongPtrW(hwnd, GWL_STYLE, wanted as isize);
        }
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let wanted_ex = ex_style | WS_EX_LAYERED.0;
        if wanted_ex != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted_ex as isize);
        }
    }
}

fn hwnd_of(window: &winit::window::Window) -> Option<HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => {
            Some(HWND(handle.hwnd.get() as *mut core::ffi::c_void))
        }
        _ => None,
    }
}

/// The resident memory DC + top-down 32bpp DIB the renderer draws into, plus the window it
/// belongs to. Top-down (negative `biHeight`) so row 0 is the top row, which is the order
/// the software renderer writes and lets `pptSrc` stay `(0, 0)` at every size.
struct UlwSurface {
    window: Arc<winit::window::Window>,
    hwnd: HWND,
    memdc: HDC,
    dib: HBITMAP,
    previous_bitmap: HGDIOBJ,
    bits: *mut u32,
    capacity: (i32, i32),
}

impl UlwSurface {
    fn new(window: Arc<winit::window::Window>, size: (i32, i32)) -> Result<Self, PlatformError> {
        let hwnd = hwnd_of(&window)
            .ok_or_else(|| PlatformError::from("atomic presentation requires a Win32 window"))?;
        enforce_atomic_window_style(hwnd);
        let capacity = (round_up(size.0), round_up(size.1));
        // SAFETY: plain GDI object creation; every handle is released in `Drop`.
        unsafe {
            let screen = GetDC(None);
            let memdc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            if memdc.is_invalid() {
                return Err(PlatformError::from(
                    "atomic presentation could not create a memory DC",
                ));
            }
            let (dib, bits) = match create_dib(memdc, capacity) {
                Ok(created) => created,
                Err(error) => {
                    let _ = DeleteDC(memdc);
                    return Err(error);
                }
            };
            let previous_bitmap = SelectObject(memdc, dib.into());
            Ok(Self { window, hwnd, memdc, dib, previous_bitmap, bits, capacity })
        }
    }

    /// Grows the DIB when a publication needs more than the resident one holds. Never
    /// shrinks: a bar oscillates between a few sizes all day and the peak is bounded by the
    /// screen.
    fn ensure_capacity(&mut self, size: (i32, i32)) -> Result<(), PlatformError> {
        if size.0 <= self.capacity.0 && size.1 <= self.capacity.1 {
            return Ok(());
        }
        let capacity = (round_up(size.0.max(self.capacity.0)), round_up(size.1.max(self.capacity.1)));
        // SAFETY: the old objects are still ours and are deleted after the new one is
        // selected in; on failure we keep the old surface intact.
        unsafe {
            let (dib, bits) = create_dib(self.memdc, capacity)?;
            let previous = SelectObject(self.memdc, dib.into());
            let _ = DeleteObject(self.dib.into());
            self.dib = dib;
            self.bits = bits;
            self.capacity = capacity;
            // `previous` is the bitmap that was selected in, i.e. our old DIB; the very
            // first `previous_bitmap` (the DC's 1×1 default) is the one Windows wants back
            // at destruction, so keep that one and drop this.
            let _ = previous;
        }
        Ok(())
    }

    fn pixels(&mut self, rows: i32) -> &mut [SoftwarePixel] {
        // SAFETY: `bits` addresses `capacity.0 * capacity.1` u32s owned by the DIB section,
        // and `rows <= capacity.1` is guaranteed by `ensure_capacity`.
        unsafe {
            core::slice::from_raw_parts_mut(
                self.bits.cast::<SoftwarePixel>(),
                (self.capacity.0 as usize) * (rows as usize),
            )
        }
    }
}

impl Drop for UlwSurface {
    fn drop(&mut self) {
        // SAFETY: every handle was created by this surface and is dropped exactly once.
        unsafe {
            SelectObject(self.memdc, self.previous_bitmap);
            let _ = DeleteObject(self.dib.into());
            let _ = DeleteDC(self.memdc);
        }
    }
}

fn round_up(value: i32) -> i32 {
    let value = value.max(1);
    ((value + CAPACITY_STEP - 1) / CAPACITY_STEP) * CAPACITY_STEP
}

unsafe fn create_dib(memdc: HDC, capacity: (i32, i32)) -> Result<(HBITMAP, *mut u32), PlatformError> {
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = core::mem::size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = capacity.0;
    // Negative height = top-down rows, matching the software renderer's row order.
    info.bmiHeader.biHeight = -capacity.1;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB.0;
    let mut bits: *mut core::ffi::c_void = core::ptr::null_mut();
    let dib = unsafe { CreateDIBSection(Some(memdc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
        .map_err(|error| PlatformError::from(format!("atomic presentation DIB failed: {error}")))?;
    if bits.is_null() {
        // SAFETY: the section was created above.
        unsafe { let _ = DeleteObject(dib.into()); }
        return Err(PlatformError::from("atomic presentation DIB has no backing memory"));
    }
    Ok((dib, bits.cast::<u32>()))
}

/// The renderer's target pixel: a premultiplied BGRA quad in one `u32`
/// (`0xAARRGGBB` little-endian = B, G, R, A in memory), which is byte-for-byte what a
/// 32bpp `BI_RGB` DIB holds and what `UpdateLayeredWindow` with `AC_SRC_ALPHA` expects.
/// Nothing between the renderer and DWM rewrites a pixel — the same invariant the R-31
/// present path carries, and for the same reason: the transparent window root has to
/// composite per-pixel.
#[repr(transparent)]
#[derive(Copy, Clone)]
struct SoftwarePixel(u32);

impl From<SoftwarePixel> for i_slint_renderer_software::PremultipliedRgbaColor {
    #[inline]
    fn from(pixel: SoftwarePixel) -> Self {
        let value = pixel.0;
        Self {
            red: (value >> 16) as u8,
            green: (value >> 8) as u8,
            blue: value as u8,
            alpha: (value >> 24) as u8,
        }
    }
}

impl From<i_slint_renderer_software::PremultipliedRgbaColor> for SoftwarePixel {
    #[inline]
    fn from(pixel: i_slint_renderer_software::PremultipliedRgbaColor) -> Self {
        Self(
            (pixel.alpha as u32) << 24
                | ((pixel.red as u32) << 16)
                | ((pixel.green as u32) << 8)
                | (pixel.blue as u32),
        )
    }
}

impl i_slint_renderer_software::TargetPixel for SoftwarePixel {
    fn blend(&mut self, color: i_slint_renderer_software::PremultipliedRgbaColor) {
        let mut blended = i_slint_renderer_software::PremultipliedRgbaColor::from(*self);
        blended.blend(color);
        *self = blended.into();
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    fn background() -> Self {
        Self(0)
    }
}

/// The per-window renderer selected for windows armed via
/// [`crate::atomic_presentation::arm_next_window`].
pub struct WinitUlwRenderer {
    /// Upstream's plain renderer. The two bars install no rendering notifier, so they do not
    /// share R-31's notifier wrapper (`sw.rs`): its hand-written delegation of
    /// `RendererSealed` can silently miss a method after an upgrade, and that risk stays on
    /// the no-GPU path instead of reaching the bars on every machine. A notifier set on a bar
    /// gets upstream's `Unsupported`.
    renderer: super::sw::SoftwareRenderer,
    surface: RefCell<Option<UlwSurface>>,
    /// The origin the next present must land on, in physical screen pixels. `None` until
    /// the host has placed the window once — then the present keeps the window where
    /// Windows already has it.
    origin: Cell<Option<(i32, i32)>>,
    /// Whether the next map activates the window (see "Every show chooses whether to
    /// activate" above). Set by `atomic_presentation::show` for the one show it makes;
    /// every [`AtomicPresentation::set_mapped`] call takes it, so the choice never outlives
    /// the map it was made for.
    next_map_activates: Cell<bool>,
}

impl WinitUlwRenderer {
    pub fn new_suspended(
        _shared_backend_data: &Rc<crate::SharedBackendData>,
    ) -> Result<Box<dyn WinitCompatibleRenderer>, PlatformError> {
        Ok(Box::new(Self {
            renderer: super::sw::SoftwareRenderer::new(),
            surface: RefCell::new(None),
            origin: Cell::new(None),
            next_map_activates: Cell::new(false),
        }))
    }
}

impl AtomicPresentation for WinitUlwRenderer {
    fn stash_origin(&self, x: i32, y: i32) {
        self.origin.set(Some((x, y)));
    }

    fn set_next_map_activation(&self, activate: bool) {
        self.next_map_activates.set(activate);
    }

    fn set_mapped(&self, visible: bool) {
        // Taken before anything else, a hide included, and before the no-surface return:
        // `show(true)` → `hide()` → plain `show()` must not activate on the last show.
        let activate = self.next_map_activates.take();
        let surface = self.surface.borrow();
        let Some(surface) = surface.as_ref() else {
            return;
        };
        let command = if !visible {
            SW_HIDE
        } else if activate {
            SW_SHOW
        } else {
            SW_SHOWNA
        };
        // SAFETY: the surface owns a live window handle.
        unsafe {
            let _ = ShowWindow(surface.hwnd, command);
        }
    }
}

impl WinitCompatibleRenderer for WinitUlwRenderer {
    fn render(&self, window: &i_slint_core::api::Window) -> Result<DrawOutcome, PlatformError> {
        let size = window.size();
        let (Ok(width), Ok(height)) = (i32::try_from(size.width), i32::try_from(size.height))
        else {
            return Ok(DrawOutcome::Success);
        };
        if width <= 0 || height <= 0 {
            return Ok(DrawOutcome::Success);
        }

        let mut borrowed = self.surface.borrow_mut();
        let Some(surface) = borrowed.as_mut() else {
            return Ok(DrawOutcome::Success);
        };
        enforce_atomic_window_style(surface.hwnd);
        surface.ensure_capacity((width, height))?;

        // Full repaint every frame. The resident DIB would survive a partial repaint, but
        // "the buffer is reusable except after a resize, a capacity growth or a hide" is
        // exactly the kind of conditional invariant AGENTS.md §3 tells us not to write: the
        // measured difference at bar sizes is 0.2 ms versus 0.63-0.93 ms per frame, against
        // a 240 Hz budget of 4.17 ms, and the bars render at most a few frames per
        // keystroke.
        self.renderer.set_repaint_buffer_type(RepaintBufferType::NewBuffer);

        let stride = surface.capacity.0 as usize;
        let _ = self.renderer.render(surface.pixels(height), stride);

        let origin = self.origin.get().unwrap_or_else(|| current_origin(surface.hwnd));
        surface.window.pre_present_notify();
        // SAFETY: all handles belong to this surface; the DIB covers `width x height` at
        // `capacity.0` stride, which is what `psize` and the top-down layout describe.
        unsafe {
            let _ = GdiFlush();
            let screen = GetDC(None);
            let destination = POINT { x: origin.0, y: origin.1 };
            let extent = SIZE { cx: width, cy: height };
            let source = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let submitted = UpdateLayeredWindow(
                surface.hwnd,
                Some(screen),
                Some(&destination),
                Some(&extent),
                Some(surface.memdc),
                Some(&source),
                COLORREF(0),
                Some(&blend),
                ULW_ALPHA,
            );
            ReleaseDC(None, screen);
            if let Err(error) = submitted {
                // Loud, not fatal: a failed submit leaves the previous frame on screen,
                // which is a legal frame under this mode's contract.
                i_slint_core::debug_log!("UpdateLayeredWindow failed: {error}");
            }
        }
        Ok(DrawOutcome::Success)
    }

    fn as_core_renderer(&self) -> &dyn i_slint_core::renderer::Renderer {
        &self.renderer
    }

    fn atomic_presentation(&self) -> Option<&dyn AtomicPresentation> {
        Some(self)
    }

    fn resume(
        &self,
        active_event_loop: &ActiveEventLoop,
        window_attributes: winit::window::WindowAttributes,
        _window_adapter_weak: std::rc::Weak<crate::winitwindowadapter::WinitWindowAdapter>,
    ) -> Result<Arc<winit::window::Window>, PlatformError> {
        let winit_window = Arc::new(active_event_loop.create_window(window_attributes).map_err(
            |error| PlatformError::from(format!("Error creating native window for atomic presentation: {error}")),
        )?);
        let size = winit_window.inner_size();
        let surface = UlwSurface::new(
            winit_window.clone(),
            (size.width.max(1) as i32, size.height.max(1) as i32),
        )?;
        *self.surface.borrow_mut() = Some(surface);
        Ok(winit_window)
    }

    fn suspend(&self) -> Result<(), PlatformError> {
        drop(self.surface.borrow_mut().take());
        Ok(())
    }
}

fn current_origin(hwnd: HWND) -> (i32, i32) {
    let mut rect = RECT::default();
    // SAFETY: `hwnd` is a live window handle.
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        (rect.left, rect.top)
    } else {
        (0, 0)
    }
}
