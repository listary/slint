// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

//! Delegate the rendering to the [`i_slint_renderer_software::SoftwareRenderer`]

use core::ops::DerefMut;
use i_slint_core::graphics::Rgb8Pixel;
use i_slint_core::platform::PlatformError;
// LISTARY PATCH (R-31): `RenderingState` for the notifier wrapper's lifecycle
// firing points below.
use i_slint_core::api::RenderingState;
use i_slint_core::renderer::DrawOutcome;
pub use i_slint_renderer_software::SoftwareRenderer;
use i_slint_renderer_software::{PremultipliedRgbaColor, RepaintBufferType, TargetPixel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use winit::event_loop::ActiveEventLoop;

use super::WinitCompatibleRenderer;

pub struct WinitSoftwareRenderer {
    renderer: NotifyingSoftwareRenderer,
    _context: RefCell<Option<softbuffer::Context<Arc<winit::window::Window>>>>,
    surface: RefCell<
        Option<softbuffer::Surface<Arc<winit::window::Window>, Arc<winit::window::Window>>>,
    >,
    // LISTARY PATCH (R-31): same contract as femtovg's `rendering_first_time`
    // (`i-slint-renderer-femtovg`): RenderingSetup must
    // fire on the FIRST RENDER after each surface creation, not inside `resume()` —
    // during `resume()` the window adapter has not stored the winit window yet, so a
    // notifier consumer asking for the native window handle gets nothing (measured:
    // the FSW chrome failed with "window handle is unavailable" when fired early).
    rendering_first_time: core::cell::Cell<bool>,
}

// LISTARY PATCH (R-31): the software renderer is the product's no-GPU-adapter fallback
// (`listary_ui::backend::configure_renderer_backend`), and both product
// windows depend on the rendering-lifecycle notifier for function, not telemetry: the
// FSW installs its window chrome and initial placement on `RenderingSetup`, and the
// launcher applies taskbar styles there and reveals its cloak on `AfterRendering`.
// Upstream's `SoftwareRenderer` rejects `set_rendering_notifier` with `Unsupported`,
// which silently disables all of that. This wrapper stores the notifier and the
// `WinitCompatibleRenderer` impl below fires it at the same lifecycle points as the
// femtovg renderers (`i-slint-renderer-femtovg`):
// `RenderingSetup` when the surface is created, `Before`/`AfterRendering` around each
// frame, `RenderingTeardown` when the surface is dropped.
//
// The `GraphicsAPI` argument is a stub: the software path has no native graphics API
// to hand out, and `GraphicsAPI` has no software variant. Both product consumers
// ignore the argument (see PATCH-NOTES.md §软件渲染器通知器); any future consumer
// that needs a real API must check the variant, and the null-returning
// `get_proc_address` makes misuse fail loudly rather than corrupt.
struct NotifyingSoftwareRenderer {
    inner: SoftwareRenderer,
    notifier: RefCell<Option<Box<dyn i_slint_core::api::RenderingNotifier>>>,
}

impl NotifyingSoftwareRenderer {
    fn new() -> Self {
        Self { inner: SoftwareRenderer::new(), notifier: RefCell::new(None) }
    }

    fn notify(&self, state: i_slint_core::api::RenderingState) {
        if let Some(callback) = self.notifier.borrow_mut().as_mut() {
            let api = i_slint_core::api::GraphicsAPI::NativeOpenGL {
                get_proc_address: &|_| core::ptr::null(),
            };
            callback.notify(state, &api);
        }
    }
}

// Delegation covers **every** `RendererSealed` method, not only the ones `SoftwareRenderer`
// overrides: a method left on its trait default would run the default against this wrapper
// instead of the inner renderer, and a method upstream adds later would silently fall back
// to it. 1.18.1 added four text methods (`text_layout_cache`, `text_content_widths`,
// `text_line_height`, `text_input_has_parley_layout`); missing one does not fail to
// compile, it quietly changes the software fallback's text measurement.
//
// The shared-parley variants are the ones compiled here: this file only builds with the
// `renderer-software` feature, which turns on `i-slint-renderer-software/std` →
// `systemfonts` → `i-slint-core/shared-parley`.
impl i_slint_core::renderer::RendererSealed for NotifyingSoftwareRenderer {
    fn text_layout_cache(
        &self,
    ) -> Option<&i_slint_core::textlayout::sharedparley::TextLayoutCache> {
        self.inner.text_layout_cache()
    }

    fn text_size(
        &self,
        text_item: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderString>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        max_width: Option<i_slint_core::lengths::LogicalLength>,
        text_wrap: i_slint_core::items::TextWrap,
    ) -> i_slint_core::lengths::LogicalSize {
        self.inner.text_size(text_item, item_rc, max_width, text_wrap)
    }

    fn text_content_widths(
        &self,
        text_item: core::pin::Pin<&dyn i_slint_core::item_rendering::RenderString>,
        item_rc: &i_slint_core::item_tree::ItemRc,
    ) -> Option<i_slint_core::renderer::ContentWidths> {
        self.inner.text_content_widths(text_item, item_rc)
    }

    fn char_size(
        &self,
        text_item: core::pin::Pin<&dyn i_slint_core::item_rendering::HasFont>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        ch: char,
    ) -> i_slint_core::lengths::LogicalSize {
        self.inner.char_size(text_item, item_rc, ch)
    }

    fn font_metrics(
        &self,
        font_request: i_slint_core::graphics::FontRequest,
    ) -> i_slint_core::items::FontMetrics {
        self.inner.font_metrics(font_request)
    }

    fn text_line_height(
        &self,
        font_request: i_slint_core::graphics::FontRequest,
    ) -> Option<i_slint_core::lengths::LogicalLength> {
        self.inner.text_line_height(font_request)
    }

    fn text_input_byte_offset_for_position(
        &self,
        text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        pos: i_slint_core::lengths::LogicalPoint,
    ) -> (usize, i_slint_core::items::TextCursorAffinity) {
        self.inner.text_input_byte_offset_for_position(text_input, item_rc, pos)
    }

    fn text_input_cursor_rect_for_byte_offset(
        &self,
        text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        item_rc: &i_slint_core::item_tree::ItemRc,
        byte_offset: usize,
        affinity: i_slint_core::items::TextCursorAffinity,
    ) -> i_slint_core::lengths::LogicalRect {
        self.inner.text_input_cursor_rect_for_byte_offset(
            text_input,
            item_rc,
            byte_offset,
            affinity,
        )
    }

    fn text_input_has_parley_layout(
        &self,
        text_input: core::pin::Pin<&i_slint_core::items::TextInput>,
        item_rc: &i_slint_core::item_tree::ItemRc,
    ) -> bool {
        self.inner.text_input_has_parley_layout(text_input, item_rc)
    }

    fn free_graphics_resources(
        &self,
        component: i_slint_core::item_tree::ItemTreeRef,
        items: &mut dyn Iterator<Item = core::pin::Pin<i_slint_core::items::ItemRef<'_>>>,
    ) -> Result<(), PlatformError> {
        self.inner.free_graphics_resources(component, items)
    }

    fn mark_dirty_region(&self, region: i_slint_core::partial_renderer::DirtyRegion) {
        self.inner.mark_dirty_region(region)
    }

    fn register_font_from_memory(
        &self,
        data: &'static [u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.register_font_from_memory(data)
    }

    fn register_font_from_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.inner.register_font_from_path(path)
    }

    fn register_bitmap_font(&self, font_data: &'static i_slint_core::graphics::BitmapFont) {
        self.inner.register_bitmap_font(font_data)
    }

    fn set_rendering_notifier(
        &self,
        callback: Box<dyn i_slint_core::api::RenderingNotifier>,
    ) -> Result<(), i_slint_core::api::SetRenderingNotifierError> {
        let mut notifier = self.notifier.borrow_mut();
        if notifier.replace(callback).is_some() {
            Err(i_slint_core::api::SetRenderingNotifierError::AlreadySet)
        } else {
            Ok(())
        }
    }

    fn set_window_adapter(&self, window_adapter: &Rc<dyn i_slint_core::window::WindowAdapter>) {
        self.inner.set_window_adapter(window_adapter)
    }

    fn window_adapter(&self) -> Option<Rc<dyn i_slint_core::window::WindowAdapter>> {
        self.inner.window_adapter()
    }

    fn scale_factor(&self) -> Option<i_slint_core::lengths::ScaleFactor> {
        self.inner.scale_factor()
    }

    fn slint_context(&self) -> Option<i_slint_core::SlintContext> {
        self.inner.slint_context()
    }

    fn resize(&self, size: i_slint_core::api::PhysicalSize) -> Result<(), PlatformError> {
        self.inner.resize(size)
    }

    fn take_snapshot(
        &self,
    ) -> Result<
        i_slint_core::graphics::SharedPixelBuffer<i_slint_core::graphics::Rgba8Pixel>,
        PlatformError,
    > {
        self.inner.take_snapshot()
    }

    fn supports_transformations(&self) -> bool {
        self.inner.supports_transformations()
    }
}

#[repr(transparent)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SoftBufferPixel(pub u32);

impl From<SoftBufferPixel> for PremultipliedRgbaColor {
    #[inline]
    fn from(pixel: SoftBufferPixel) -> Self {
        let v = pixel.0;
        PremultipliedRgbaColor {
            red: (v >> 16) as u8,
            green: (v >> 8) as u8,
            blue: v as u8,
            alpha: (v >> 24) as u8,
        }
    }
}

impl From<PremultipliedRgbaColor> for SoftBufferPixel {
    #[inline]
    fn from(pixel: PremultipliedRgbaColor) -> Self {
        Self(
            (pixel.alpha as u32) << 24
                | ((pixel.red as u32) << 16)
                | ((pixel.green as u32) << 8)
                | (pixel.blue as u32),
        )
    }
}

impl TargetPixel for SoftBufferPixel {
    fn blend(&mut self, color: PremultipliedRgbaColor) {
        let mut x = PremultipliedRgbaColor::from(*self);
        x.blend(color);
        *self = x.into();
    }

    fn from_rgb(r: u8, g: u8, b: u8) -> Self {
        Self(0xff000000 | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32))
    }

    fn background() -> Self {
        Self(0)
    }
}

impl WinitSoftwareRenderer {
    pub fn new_suspended(
        _shared_backend_data: &Rc<crate::SharedBackendData>,
    ) -> Result<Box<dyn WinitCompatibleRenderer>, PlatformError> {
        Ok(Box::new(Self {
            renderer: NotifyingSoftwareRenderer::new(),
            _context: RefCell::new(None),
            surface: RefCell::new(None),
            rendering_first_time: core::cell::Cell::new(true),
        }))
    }
}

impl super::WinitCompatibleRenderer for WinitSoftwareRenderer {
    fn render(&self, window: &i_slint_core::api::Window) -> Result<DrawOutcome, PlatformError> {
        let size = window.size();

        let Some((width, height)) = size.width.try_into().ok().zip(size.height.try_into().ok())
        else {
            // Nothing to render
            return Ok(DrawOutcome::Success);
        };

        let mut borrowed_surface = self.surface.borrow_mut();
        let Some(surface) = borrowed_surface.as_mut() else {
            // Nothing to render
            return Ok(DrawOutcome::Success);
        };

        let winit_window = surface.window().clone();

        surface
            .resize(width, height)
            .map_err(|e| format!("Error resizing softbuffer surface: {e}"))?;

        let mut target_buffer = surface
            .buffer_mut()
            .map_err(|e| format!("Error retrieving softbuffer rendering buffer: {e}"))?;

        let age = target_buffer.age();
        self.renderer.inner.set_repaint_buffer_type(match age {
            1 => RepaintBufferType::ReusedBuffer,
            2 => RepaintBufferType::SwappedBuffers,
            _ => RepaintBufferType::NewBuffer,
        });

        // LISTARY PATCH (R-31): same firing points as the femtovg renderers — Setup
        // once per surface on the first frame (the adapter has stored the window by
        // now, so notifier consumers can reach the native handle), BeforeRendering
        // before each scene draw.
        if self.rendering_first_time.take() {
            self.renderer.notify(RenderingState::RenderingSetup);
        }
        self.renderer.notify(RenderingState::BeforeRendering);

        let region = if std::env::var_os("SLINT_LINE_BY_LINE").is_none() {
            let buffer: &mut [SoftBufferPixel] =
                bytemuck::cast_slice_mut(target_buffer.deref_mut());
            self.renderer.inner.render(buffer, width.get() as usize)
        } else {
            // SLINT_LINE_BY_LINE is set and this is a debug mode where we also render in a Rgb565Pixel
            struct FrameBuffer<'a> {
                buffer: &'a mut [u32],
                line: Vec<i_slint_renderer_software::Rgb565Pixel>,
            }
            impl i_slint_renderer_software::LineBufferProvider for FrameBuffer<'_> {
                type TargetPixel = i_slint_renderer_software::Rgb565Pixel;
                fn process_line(
                    &mut self,
                    line: usize,
                    range: core::ops::Range<usize>,
                    render_fn: impl FnOnce(&mut [Self::TargetPixel]),
                ) {
                    let line_begin = line * self.line.len();
                    let sub = &mut self.line[..range.len()];
                    render_fn(sub);
                    for (dst, src) in self.buffer[line_begin..][range].iter_mut().zip(sub) {
                        let p = Rgb8Pixel::from(*src);
                        *dst =
                            0xff000000 | ((p.r as u32) << 16) | ((p.g as u32) << 8) | (p.b as u32);
                    }
                }
            }
            self.renderer.inner.render_by_line(FrameBuffer {
                buffer: &mut target_buffer,
                line: vec![Default::default(); width.get() as usize],
            })
        };

        // LISTARY PATCH (R-31): present the FULL buffer unconditionally instead of the
        // dirty region's bounding box. Windows clears a window's redirection surface on
        // hide/show, and softbuffer's DIB-backed `age()` cannot observe that — after a
        // re-show, a damage-only present paints just the latest dirty sliver onto an
        // otherwise fully transparent surface (measured: the summoned launcher rendered
        // as floating text over the desktop, its 612×100 panel never presented). The
        // `occluded()` hook upstream added for this relies on `WindowEvent::Occluded`,
        // which Windows does not reliably emit. Rendering stays dirty-region-cheap —
        // only the present is widened, and a full-window GDI blit is ~ms-scale at the
        // ≤4Hz idle repaint rate. Unconditional beats tracking visibility transitions:
        // every path that recreates or clears the redirection surface is covered.
        //
        // ★ Invariant: the present path passes the renderer's PREMULTIPLIED ALPHA
        // through UNCHANGED — nothing between `render()` and DWM may rewrite pixel
        // semantics. DWM composites the redirection surface per-pixel-alpha (measured:
        // a window root's transparent margin correctly shows the content behind it),
        // which is what keeps `background: transparent` windows working on this
        // renderer with zero per-window special-casing. An earlier revision of this
        // patch forced alpha opaque here to paper over the damage-present defect above;
        // that turned the launcher's transparent margin into a black border. The
        // one-authority rule: fix presentation defects in the present step (this
        // comment's block), never by mutating the rendered pixels.
        let _ = region;
        winit_window.pre_present_notify();
        target_buffer.present().map_err(|e| format!("Error presenting softbuffer buffer: {e}"))?;
        // LISTARY PATCH (R-31): fired after present, matching femtovg's AfterRendering
        // point — the launcher's cloak reveal waits for exactly this.
        self.renderer.notify(RenderingState::AfterRendering);
        Ok(DrawOutcome::Success)
    }

    fn as_core_renderer(&self) -> &dyn i_slint_core::renderer::Renderer {
        &self.renderer
    }

    fn occluded(&self, _: bool) {
        // On X11 and Windows, the buffer is completely cleared when the window is hidden
        // and the buffer age doesn't respect that, so clean the partial rendering cache
        self.renderer.inner.set_repaint_buffer_type(RepaintBufferType::NewBuffer);
    }

    fn resume(
        &self,
        active_event_loop: &ActiveEventLoop,
        window_attributes: winit::window::WindowAttributes,
        _window_adapter_weak: std::rc::Weak<crate::winitwindowadapter::WinitWindowAdapter>,
    ) -> Result<Arc<winit::window::Window>, PlatformError> {
        let winit_window =
            active_event_loop.create_window(window_attributes).map_err(|winit_os_error| {
                PlatformError::from(format!(
                    "Error creating native window for software rendering: {winit_os_error}"
                ))
            })?;
        let winit_window = Arc::new(winit_window);

        let context = softbuffer::Context::new(winit_window.clone())
            .map_err(|e| format!("Error creating softbuffer context: {e}"))?;

        let surface = softbuffer::Surface::new(&context, winit_window.clone()).map_err(
            |softbuffer_error| format!("Error creating softbuffer surface: {softbuffer_error}"),
        )?;

        *self._context.borrow_mut() = Some(context);
        *self.surface.borrow_mut() = Some(surface);

        // LISTARY PATCH (R-31): a fresh surface needs a fresh RenderingSetup — fired
        // lazily on the first render (see `rendering_first_time`).
        self.rendering_first_time.set(true);

        Ok(winit_window)
    }

    fn suspend(&self) -> Result<(), PlatformError> {
        // LISTARY PATCH (R-31): teardown only if a frame was actually rendered on this
        // surface, mirroring femtovg (`clear_graphics_context`).
        if self.surface.borrow().is_some() && !self.rendering_first_time.get() {
            self.renderer.notify(RenderingState::RenderingTeardown);
        }
        drop(self.surface.borrow_mut().take());
        drop(self._context.borrow_mut().take());
        Ok(())
    }
}
