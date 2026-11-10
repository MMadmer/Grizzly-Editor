//! Thumbnails: rendered at the size a tile asks for, on demand, kept by content.

use std::sync::Arc;
use std::time::Duration;

use gz_fmt_tex::Image;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::assets::Assets;
use crate::draw_list::DrawList;
use crate::packet::build_packet;
use crate::renderer::Renderer;
use crate::view::View;
use crate::Result;

/// Pictures by `(content key, size)`, the least recently used dropped beyond a byte budget.
/// The key is the caller's: a hash of the file's bytes, or its revision - whatever changes
/// when the content does.
pub struct ThumbnailCache {
    budget: usize,
    inner: Mutex<Lru>,
}

#[derive(Default)]
struct Lru {
    map: FxHashMap<(u64, [u32; 2]), (Arc<Image>, u64)>,
    bytes: usize,
    tick: u64,
}

impl Lru {
    fn get(&mut self, key: (u64, [u32; 2])) -> Option<Arc<Image>> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(&key).map(|(img, used)| {
            *used = tick;
            img.clone()
        })
    }

    fn put(&mut self, key: (u64, [u32; 2]), img: Arc<Image>, budget: usize) {
        self.tick += 1;
        self.bytes += img.rgba.len();
        if let Some((old, _)) = self.map.insert(key, (img, self.tick)) {
            self.bytes -= old.rgba.len();
        }
        if self.bytes > budget {
            let mut order: Vec<(u64, (u64, [u32; 2]))> = self.map.iter().filter(|(k, _)| **k != key).map(|(k, (_, t))| (*t, *k)).collect();
            order.sort_unstable();
            for (_, k) in order {
                if self.bytes <= budget {
                    break;
                }
                if let Some((img, _)) = self.map.remove(&k) {
                    self.bytes -= img.rgba.len();
                }
            }
        }
    }
}

impl ThumbnailCache {
    #[must_use]
    pub fn new(budget_bytes: usize) -> Self {
        Self { budget: budget_bytes, inner: Mutex::new(Lru::default()) }
    }

    /// A picture made before, if it is still kept.
    #[must_use]
    pub fn get(&self, key: u64, size: [u32; 2]) -> Option<Arc<Image>> {
        self.inner.lock().get((key, size))
    }

    /// The picture of `key` at `size`, rendered when it is not kept: `scene` describes it,
    /// whatever it draws is loaded first (up to `wait`), the view is drawn as a
    /// [`crate::ViewFlags::PREVIEW`] picture. Blocks: call it from a job.
    pub fn get_or_render(
        &self,
        renderer: &Renderer,
        key: u64,
        size: [u32; 2],
        wait: Duration,
        scene: impl FnOnce(&Assets) -> (DrawList, View),
    ) -> Result<Arc<Image>> {
        if let Some(img) = self.get(key, size) {
            return Ok(img);
        }
        let assets = renderer.assets();
        let (list, view) = scene(assets);
        let view = View { size, ..view.as_preview() };
        assets.prepare(&list, wait);
        let packet = build_packet(&list, &view, assets);
        let img = Arc::new(renderer.render_offscreen(&packet, size)?);
        // a picture with placeholders in it is not kept: the next ask draws it again
        if !packet.waiting_on_assets() && packet.stats.placeholders == 0 {
            self.inner.lock().put((key, size), img.clone(), self.budget);
        }
        Ok(img)
    }

    /// Drops every size of `key`.
    pub fn forget(&self, key: u64) {
        let mut lru = self.inner.lock();
        let gone: Vec<_> = lru.map.keys().filter(|k| k.0 == key).copied().collect();
        for k in gone {
            if let Some((img, _)) = lru.map.remove(&k) {
                lru.bytes -= img.rgba.len();
            }
        }
    }

    /// Bytes kept.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.inner.lock().bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(side: u32) -> Arc<Image> {
        Arc::new(Image { width: side, height: side, rgba: vec![0; (side * side * 4) as usize] })
    }

    #[test]
    fn the_least_recently_used_goes_first() {
        let cache = ThumbnailCache::new(3 * 64 * 64 * 4);
        {
            let mut l = cache.inner.lock();
            l.put((1, [64, 64]), img(64), cache.budget);
            l.put((2, [64, 64]), img(64), cache.budget);
            l.put((3, [64, 64]), img(64), cache.budget);
        }
        assert!(cache.get(1, [64, 64]).is_some());
        cache.inner.lock().put((4, [64, 64]), img(64), cache.budget);
        // 2 was the oldest untouched
        assert!(cache.get(2, [64, 64]).is_none());
        assert!(cache.get(1, [64, 64]).is_some() && cache.get(3, [64, 64]).is_some() && cache.get(4, [64, 64]).is_some());
        assert_eq!(cache.bytes(), 3 * 64 * 64 * 4);
        // the same content at another size is another picture
        assert!(cache.get(1, [128, 128]).is_none());
        cache.forget(1);
        assert!(cache.get(1, [64, 64]).is_none());
        assert_eq!(cache.bytes(), 2 * 64 * 64 * 4);
        // replacing a picture does not count it twice
        cache.inner.lock().put((3, [64, 64]), img(64), cache.budget);
        assert_eq!(cache.bytes(), 2 * 64 * 64 * 4);
    }
}
