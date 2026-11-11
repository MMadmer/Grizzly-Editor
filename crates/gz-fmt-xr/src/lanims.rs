//! `lanims.xr`: colour animations of lights. `LightAnimLibrary.cpp`.

use std::collections::BTreeMap;

use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::{Encoding, Result, Text, invalid, numbered, w_str};

const CHUNK_VERSION: u32 = 0;
const CHUNK_ITEM_LIST: u32 = 1;
const CHUNK_ITEM_COMMON: u32 = 1;
const CHUNK_ITEM_KEYS: u32 = 2;
/// `LANIM_VERSION`.
pub const LANIM_VERSION: u16 = 1;

/// `CLAItem`: keys are `frame -> 0xAARRGGBB`.
#[derive(Clone, Debug, PartialEq)]
pub struct LightAnim {
    pub name: SmolStr,
    pub fps: f32,
    pub frame_count: i32,
    pub keys: BTreeMap<i32, u32>,
}

/// The library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightAnims {
    /// The version the file had; version 0 stored BGR and was converted on reading.
    pub version: u16,
    pub items: Vec<LightAnim>,
    pub encoding: Encoding,
}

fn color_argb(a: u32, r: u32, g: u32, b: u32) -> u32 {
    ((a & 0xff) << 24) | ((r & 0xff) << 16) | ((g & 0xff) << 8) | (b & 0xff)
}

/// `Fcolor::set(u32)`.
fn fcolor(dw: u32) -> [f32; 4] {
    let f = 1.0f32 / 255.0;
    [
        f * ((dw >> 16) & 0xff) as f32,
        f * ((dw >> 8) & 0xff) as f32,
        f * (dw & 0xff) as f32,
        f * ((dw >> 24) & 0xff) as f32,
    ]
}

/// `color_rgba_f`: `clampr(iFloor(x * 255), 0, 255)`.
fn pack(c: [f32; 4]) -> u32 {
    let q = |v: f32| ((v * 255.0).floor() as i32).clamp(0, 255) as u32;
    color_argb(q(c[3]), q(c[0]), q(c[1]), q(c[2]))
}

impl LightAnim {
    /// `InterpolateRGB`: `None` for a frame past the end or an animation without keys.
    #[must_use]
    pub fn color_at(&self, frame: i32) -> Option<u32> {
        if frame > self.frame_count {
            return None;
        }
        if let Some(c) = self.keys.get(&frame) {
            return Some(*c);
        }
        let (b_frame, b) = match self.keys.range(frame + 1..).next() {
            Some((f, c)) => (*f, *c),
            None => return self.keys.values().next_back().copied(),
        };
        let (a_frame, a) = self.keys.range(..b_frame).next_back().map(|(f, c)| (*f, *c))?;
        let (a0, a1) = (a_frame as f32, b_frame as f32);
        let (c0, c1) = (fcolor(a), fcolor(b));
        let t = (frame as f32 - a0) / (a1 - a0);
        let invt = 1.0 - t;
        let mut c = [0.0f32; 4];
        for i in 0..4 {
            c[i] = c0[i] * invt + c1[i] * t;
        }
        Some(pack(c))
    }

    /// `CalculateRGB`: the colour at `t` seconds, and the frame it is.
    #[must_use]
    pub fn calculate(&self, t: f32) -> Option<(u32, i32)> {
        let length = self.frame_count as f32 / self.fps;
        let frame = ((t % length) * self.fps).floor() as i32;
        Some((self.color_at(frame)?, frame))
    }

    /// `CalculateBGR`.
    #[must_use]
    pub fn calculate_bgr(&self, t: f32) -> Option<(u32, i32)> {
        let (c, f) = self.calculate(t)?;
        Some((color_argb(c >> 24, c, c >> 8, c >> 16), f))
    }
}

impl LightAnims {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let version = match r.sub(CHUNK_VERSION)? {
            Some(mut v) => v.r_u16()?,
            None => 0,
        };
        let mut text = Text::default();
        let mut items = Vec::new();
        if let Some(list) = r.open_chunk(CHUNK_ITEM_LIST)? {
            let lr = list.reader();
            for c in numbered(&lr)? {
                let ir = c.reader();
                let mut common =
                    ir.sub(CHUNK_ITEM_COMMON)?.ok_or_else(|| invalid("a light animation has no name"))?;
                let name = text.r_str(&mut common)?;
                let fps = common.r_f32()?;
                let frame_count = common.r_u32()? as i32;
                let mut kr = ir
                    .sub(CHUNK_ITEM_KEYS)?
                    .ok_or_else(|| invalid(format!("light animation '{name}' has no keys")))?;
                let n = kr.r_u32()? as usize;
                if n.saturating_mul(8) > kr.elapsed() {
                    return Err(invalid(format!(
                        "light animation '{name}' states {n} keys and cannot hold them"
                    )));
                }
                let mut keys = BTreeMap::new();
                for _ in 0..n {
                    let frame = kr.r_u32()? as i32;
                    let mut c = kr.r_u32()?;
                    if version == 0 {
                        // `subst_alpha(bgr2rgb(c), alpha)`
                        c = color_argb(c >> 24, c, c >> 8, c >> 16);
                    }
                    keys.insert(frame, c);
                }
                items.push(LightAnim { name, fps, frame_count, keys });
            }
        }
        Ok(Self { version, items, encoding: text.encoding() })
    }

    /// `ELightAnimLibrary::Save`.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.chunk(CHUNK_VERSION, |w| w.w_u16(LANIM_VERSION));
        w.chunk(CHUNK_ITEM_LIST, |w| {
            for (i, item) in self.items.iter().enumerate() {
                w.chunk(i as u32, |w| {
                    w.chunk(CHUNK_ITEM_COMMON, |w| {
                        w_str(w, &item.name, self.encoding);
                        w.w_f32(item.fps);
                        w.w_u32(item.frame_count as u32);
                    });
                    w.chunk(CHUNK_ITEM_KEYS, |w| {
                        w.w_u32(item.keys.len() as u32);
                        for (f, c) in &item.keys {
                            w.w_u32(*f as u32);
                            w.w_u32(*c);
                        }
                    });
                });
            }
        });
        w.into_bytes()
    }

    #[must_use]
    pub fn find(&self, name: &str) -> Option<&LightAnim> {
        self.items.iter().find(|i| i.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> LightAnims {
        let keys = BTreeMap::from([(0, 0xff00_0000), (10, 0xffff_ffff)]);
        LightAnims {
            version: 1,
            items: vec![LightAnim { name: "flicker".into(), fps: 15.0, frame_count: 20, keys }],
            encoding: Encoding::Utf8,
        }
    }

    #[test]
    fn a_library_writes_reads_and_writes_the_same_bytes() {
        let l = lib();
        let b = l.write();
        let back = LightAnims::read(&b).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.write(), b);
    }

    #[test]
    fn colours_interpolate_as_the_engine_does() {
        let a = &lib().items[0];
        assert_eq!(a.color_at(0), Some(0xff00_0000));
        // half way: 0.5 * 255 = 127.5, floored to 127
        assert_eq!(a.color_at(5), Some(0xff7f_7f7f));
        // past the last key: the last key
        assert_eq!(a.color_at(15), Some(0xffff_ffff));
        assert_eq!(a.color_at(21), None);
        // t = 1/3 s at 15 fps is frame 5
        assert_eq!(a.calculate(1.0 / 3.0).map(|(_, f)| f), Some(5));
    }

    #[test]
    fn version_zero_stored_bgr() {
        let mut w = Writer::new();
        w.chunk(CHUNK_VERSION, |w| w.w_u16(0));
        w.chunk(CHUNK_ITEM_LIST, |w| {
            w.chunk(0, |w| {
                w.chunk(CHUNK_ITEM_COMMON, |w| {
                    w.w_stringz("a");
                    w.w_f32(30.0);
                    w.w_u32(1);
                });
                w.chunk(CHUNK_ITEM_KEYS, |w| {
                    w.w_u32(1);
                    w.w_u32(0);
                    w.w_u32(0x8011_2233);
                });
            });
        });
        let l = LightAnims::read(&w.into_bytes()).unwrap();
        assert_eq!(l.items[0].keys[&0], 0x8033_2211);
        let mut cut = Writer::new();
        cut.chunk(CHUNK_ITEM_LIST, |w| w.chunk(0, |w| w.chunk(CHUNK_ITEM_COMMON, |w| w.w_stringz("a"))));
        assert!(LightAnims::read(&cut.into_bytes()).is_err());
    }
}
