// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

// cSpell: ignore RAII

//! The cache of shaped paragraphs, keyed by item.
//!
//! An entry holds the output of [`shaping`](super::shaping) for one item, and is invalidated by
//! the property dependencies the shaping registered.
//! [`cached_paragraphs`] is only meant to be called through `with_text_layout`, which pairs it
//! with the one shaping function every path must share.

use super::layout::RetainedLineBreaking;
use super::shaping::TextParagraph;
use super::*;

/// Shaped paragraphs together with the wrap mode they were shaped with.
///
/// The glyph geometry only depends on (text, font, wrap, scale factor): the width is applied
/// later by `break_all_lines`, and the fill/stroke/selection brushes only change colors, not
/// positions. So one entry serves measuring, hit-testing and drawing alike -- but only for the
/// wrap mode it was shaped with, because parley bakes the break opportunities into the shaped
/// layout via `WordBreak`/`OverflowWrap`/`TextWrapMode` (see `ranged_builder`). The scale factor
/// is the other input baked into the shaping, but it applies to every entry at once and so is
/// handled by the cache as a whole.
struct CachedParagraphs {
    wrap: TextWrap,
    /// What [`super::layout::layout`] derived when it last broke these paragraphs, so an
    /// unchanged-input call can skip the breaking. `None` after a reshape (fresh entries
    /// start without one) and while checked out through the guard.
    line_breaking: Option<RetainedLineBreaking>,
    /// `None` while a [`CachedParagraphsGuard`] has the paragraphs checked out; the guard puts
    /// them back when it drops. Finding `None` here therefore means the previous caller returned
    /// without handing them back, and the entry has to be reshaped rather than served empty.
    // LISTARY PATCH: a sweep also leaves `None` here; `cached_paragraphs` then shapes again and
    // refills the entry, keeping its tracker (see `sweep`).
    paragraphs: Option<Vec<TextParagraph>>,
    /// The [`TextLayoutCache::generation`] at which this entry was last served.
    last_used: u32,
}

type InnerTextLayoutCache = crate::item_rendering::ItemCache<CachedParagraphs>;

/// Entry count above which [`TextLayoutCache::sweep`] runs (~4.7KB per entry).
const ENTRY_LIMIT: usize = 1024;

/// Cache for shaped text paragraphs (before line breaking), keyed by ItemRc.
pub struct TextLayoutCache {
    inner: InnerTextLayoutCache,
    /// Caches the result of [`super::text_content_widths`]. The widths are two scalars derived from
    /// paragraphs that no other path can reuse, because they are shaped without
    /// `OverflowWrap::Anywhere`, so the result is kept instead of the paragraphs.
    content_widths: crate::item_rendering::ItemCache<crate::renderer::ContentWidths>,
    /// Bumped once per rendered frame; entries are stamped with it when served.
    generation: std::cell::Cell<u32>,
    /// Approximate entry count; a stale-high value just triggers one extra sweep.
    entry_count_estimate: std::cell::Cell<usize>,
    /// Estimate above which the next sweep runs; raised when the active set exceeds the limit.
    sweep_threshold: std::cell::Cell<usize>,
    #[cfg(feature = "testing")]
    cache_miss_count: std::cell::Cell<u64>,
    #[cfg(feature = "testing")]
    layout_miss_count: std::cell::Cell<u64>,
    #[cfg(feature = "testing")]
    content_widths_miss_count: std::cell::Cell<u64>,
}

#[allow(clippy::derivable_impls)] // clippy doesn't see the feature = "testing" code
impl Default for TextLayoutCache {
    fn default() -> Self {
        Self {
            inner: Default::default(),
            content_widths: Default::default(),
            generation: Default::default(),
            entry_count_estimate: Default::default(),
            sweep_threshold: std::cell::Cell::new(ENTRY_LIMIT),
            #[cfg(feature = "testing")]
            cache_miss_count: std::cell::Cell::new(0),
            #[cfg(feature = "testing")]
            layout_miss_count: std::cell::Cell::new(0),
            #[cfg(feature = "testing")]
            content_widths_miss_count: std::cell::Cell::new(0),
        }
    }
}

impl TextLayoutCache {
    /// Drops everything shaped for the previous scale factor. Glyph advances are in physical
    /// pixels, so a new scale factor invalidates every entry at once. Called on the way into the
    /// cache rather than when rendering starts, because the layout pass that follows a scale
    /// factor change measures before anything renders.
    pub(super) fn clear_if_scale_factor_changed(&self, window: &crate::api::Window) {
        self.inner.clear_cache_if_scale_factor_changed(window);
        self.content_widths.clear_cache_if_scale_factor_changed(window);
    }
    pub fn component_destroyed(&self, component: crate::item_tree::ItemTreeRef) {
        self.inner.component_destroyed(component);
        self.content_widths.component_destroyed(component);
    }
    pub fn clear_all(&self) {
        self.inner.clear_all();
        self.content_widths.clear_all();
    }

    /// Returns the cached content widths of `item_rc`, computing them on a miss.
    ///
    /// The entry is invalidated by the properties `compute` reads, so it must read the
    /// text, the font request and the line limit itself. The scale factor is handled by
    /// [`Self::clear_if_scale_factor_changed`], which the caller runs first.
    pub(super) fn content_widths(
        &self,
        item_rc: &crate::item_tree::ItemRc,
        compute: impl FnOnce() -> crate::renderer::ContentWidths,
    ) -> crate::renderer::ContentWidths {
        self.content_widths.get_or_update_cache_entry(item_rc, || {
            #[cfg(feature = "testing")]
            self.content_widths_miss_count.set(self.content_widths_miss_count.get() + 1);
            compute()
        })
    }

    /// Marks the beginning of a frame; called once per rendered frame.
    pub fn begin_frame(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
    }

    // LISTARY PATCH: upstream removed the stale entries here, trackers included; this keeps them
    // and drops only their paragraphs (see PATCH-NOTES.md).
    /// Drops the paragraphs of the entries that were not served in the current or the previous
    /// frame.
    ///
    /// The entries themselves stay: an entry's dependency tracker is the only thing that reads the
    /// item's text, so it is also the only link from a text change to the item's rendering (the
    /// partial renderer's per-item tracker depends on the entry, not on the text). Removing the
    /// entry would leave an item that is not redrawn for its next text change. Without its
    /// paragraphs the entry is shaped again and refilled on its next use (see `cached_paragraphs`).
    fn sweep(&self) {
        let generation = self.generation.get();
        let mut kept = 0;
        self.inner.for_each_mut(|entry| {
            if entry.paragraphs.is_none() {
                return;
            }
            if generation.wrapping_sub(entry.last_used) <= 1 {
                kept += 1;
            } else {
                entry.paragraphs = None;
                entry.line_breaking = None;
            }
        });
        self.entry_count_estimate.set(kept);
        self.sweep_threshold.set(ENTRY_LIMIT.max(kept + ENTRY_LIMIT / 2));
    }
}

#[cfg(feature = "testing")]
impl TextLayoutCache {
    pub fn cache_miss_count(&self) -> u64 {
        self.cache_miss_count.get()
    }
    pub fn reset_cache_miss_count(&self) {
        self.cache_miss_count.set(0);
    }
    /// How many times a layout pass had to break the lines of a cached item again rather than
    /// reuse the retained breaking.
    pub fn layout_miss_count(&self) -> u64 {
        self.layout_miss_count.get()
    }
    pub fn reset_layout_miss_count(&self) {
        self.layout_miss_count.set(0);
    }
    /// How many times the content widths of an item had to be shaped rather than served
    /// from the cache.
    pub fn content_widths_miss_count(&self) -> u64 {
        self.content_widths_miss_count.get()
    }
    pub fn reset_content_widths_miss_count(&self) {
        self.content_widths_miss_count.set(0);
    }
    pub(super) fn count_layout_miss(&self) {
        self.layout_miss_count.set(self.layout_miss_count.get() + 1);
    }
}

/// RAII guard: takes the shaped paragraphs out of the cache on creation, puts them back on drop.
pub(super) struct CachedParagraphsGuard<'a> {
    paragraphs: Option<Vec<TextParagraph>>,
    line_breaking: Option<RetainedLineBreaking>,
    container: Option<std::cell::RefMut<'a, CachedParagraphs>>,
}

impl CachedParagraphsGuard<'_> {
    /// Lends the paragraphs to [`layout`], which hands them back as part of its `Layout`.
    pub(super) fn take(&mut self) -> Vec<TextParagraph> {
        self.paragraphs.take().unwrap_or_default()
    }

    /// Hands the retained breaking to [`layout`], which decides whether it still applies.
    pub(super) fn take_line_breaking(&mut self) -> Option<RetainedLineBreaking> {
        self.container.as_mut().and_then(|container| container.line_breaking.take())
    }

    /// Returns the paragraphs and the line breaking they carry, so that the next caller reuses
    /// both the shaping and, with unchanged inputs, the breaking.
    pub(super) fn restore(
        &mut self,
        paragraphs: Vec<TextParagraph>,
        line_breaking: RetainedLineBreaking,
    ) {
        self.paragraphs = Some(paragraphs);
        self.line_breaking = Some(line_breaking);
    }
}

impl Drop for CachedParagraphsGuard<'_> {
    fn drop(&mut self) {
        if let Some(container) = &mut self.container {
            if let Some(paragraphs) = self.paragraphs.take() {
                container.paragraphs = Some(paragraphs);
            }
            if let Some(line_breaking) = self.line_breaking.take() {
                container.line_breaking = Some(line_breaking);
            }
        }
    }
}

/// Shapes the text of `item_rc` for `wrap`, reusing the `TextLayoutCache` entry when it holds
/// paragraphs shaped for the same wrap mode and none of the properties `shape` read have changed
/// since. Without a cache or item it just shapes, so the caller doesn't need to special-case that.
///
/// `shape` runs inside the entry's dependency tracker, so everything it reads (the text and the
/// font request, at least) invalidates the entry when it changes. Properties evaluated by the
/// caller before this point are clean by then and thus can't re-enter here.
pub(super) fn cached_paragraphs<'a>(
    cache: Option<&'a TextLayoutCache>,
    item_rc: Option<&crate::item_tree::ItemRc>,
    wrap: TextWrap,
    window: &crate::api::Window,
    font_context: &mut parley::FontContext,
    shape: &dyn Fn(&mut parley::FontContext) -> Vec<TextParagraph>,
) -> CachedParagraphsGuard<'a> {
    let Some((cache, item_rc)) = cache.zip(item_rc) else {
        return CachedParagraphsGuard {
            paragraphs: Some(shape(font_context)),
            line_breaking: None,
            container: None,
        };
    };

    cache.clear_if_scale_factor_changed(window);

    // Sweep before this item's entry is checked out and blocks `for_each_mut`'s access.
    // LISTARY PATCH: moved up from after the wrap check below, so that an entry this sweep empties,
    // this item's own included, goes through the refill below instead of being served empty.
    if cache.entry_count_estimate.get() > cache.sweep_threshold.get() {
        cache.sweep();
    }

    // Shaped geometry must never be mixed across wrap modes, and the entry only holds one mode
    // at a time. Drop a mismatching one up front so the shaping below happens in the regular
    // (vacant) path, inside a fresh dependency tracker and without the cache borrowed.
    let wrap_changed = cache
        .inner
        .with_entry(item_rc, |entry| (entry.wrap != wrap).then_some(()))
        .is_some();
    if wrap_changed {
        cache.inner.release(item_rc);
    }

    // LISTARY PATCH: an entry without paragraphs (emptied by a sweep, or never handed back) is not
    // released but refilled. Its tracker is what the item's rendering depends on; releasing it
    // would register the new tracker with whichever binding runs this lookup, which may be a
    // layout query rather than the item's rendering. Its tracker is usually clean, so the lookup
    // below would not reshape: shape now, while the cache is not borrowed, and put the result into
    // the entry. If the tracker is dirty the lookup reshapes anyway and this result is dropped.
    let refill = cache
        .inner
        .with_entry(item_rc, |entry| entry.paragraphs.is_none().then_some(()))
        .map(|()| shape(font_context));

    let mut entry = cache.inner.get_or_update_cache_entry_ref(item_rc, || {
        #[cfg(feature = "testing")]
        cache.cache_miss_count.set(cache.cache_miss_count.get() + 1);
        cache.entry_count_estimate.set(cache.entry_count_estimate.get() + 1);
        CachedParagraphs {
            wrap,
            paragraphs: Some(shape(font_context)),
            line_breaking: None,
            last_used: 0, // stamped right below, for both a hit and this miss
        }
    });
    entry.last_used = cache.generation.get();
    // LISTARY PATCH: see `refill` above.
    if entry.paragraphs.is_none()
        && let Some(paragraphs) = refill
    {
        #[cfg(feature = "testing")]
        cache.cache_miss_count.set(cache.cache_miss_count.get() + 1);
        cache.entry_count_estimate.set(cache.entry_count_estimate.get() + 1);
        entry.paragraphs = Some(paragraphs);
        entry.line_breaking = None;
    }
    let paragraphs = entry.paragraphs.take().unwrap_or_default();
    CachedParagraphsGuard {
        paragraphs: Some(paragraphs),
        line_breaking: None,
        container: Some(entry),
    }
}
