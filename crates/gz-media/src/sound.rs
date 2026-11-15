//! The sound asset (`*.sndasset`, `snd = 1`): a sound and the parameters the game plays it
//! with - what a sound's `.thm` says, as named keys.

use gz_fmt_sound::SoundThm;
use gz_luadata::{Table, Value};

use crate::fields::{self, SOUND};
use crate::read::{self, Cx};

/// A sound asset as its file says; `None` is a key left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundAsset {
    pub source: Option<String>,
    pub name: Option<String>,
    pub min_distance: Option<f64>,
    pub max_distance: Option<f64>,
    pub volume: Option<f64>,
    pub ai_distance: Option<f64>,
    /// [`fields::SOUND_TYPES`].
    pub ty: Option<u32>,
    pub quality: Option<f64>,
}

impl SoundAsset {
    #[must_use]
    pub fn from_value(v: &Value) -> (Self, Vec<read::Issue>) {
        let mut cx = Cx::default();
        let s = SoundAsset {
            source: read::text(&mut cx, v, "source"),
            name: read::text(&mut cx, v, "name"),
            min_distance: read::number(&mut cx, v, SOUND, "min_distance"),
            max_distance: read::number(&mut cx, v, SOUND, "max_distance"),
            volume: read::number(&mut cx, v, SOUND, "volume"),
            ai_distance: read::number(&mut cx, v, SOUND, "ai_distance"),
            ty: read::word_set(&mut cx, v, SOUND, "type"),
            quality: read::number(&mut cx, v, SOUND, "quality"),
        };
        (s, cx.issues)
    }

    /// What the game plays the sound with, and its encoding quality.
    #[must_use]
    pub fn to_thm(&self) -> SoundThm {
        let d = SoundThm::default();
        let max = self.max_distance.map_or(d.max_dist, |x| x as f32);
        SoundThm {
            quality: self.quality.map_or(d.quality, |x| x as f32),
            min_dist: self.min_distance.map_or(d.min_dist, |x| x as f32),
            max_dist: max,
            max_ai_dist: self.ai_distance.map_or(max, |x| x as f32),
            base_volume: self.volume.map_or(d.base_volume, |x| x as f32),
            game_type: self.ty.unwrap_or(d.game_type),
        }
    }

    /// The asset that says what a sound's description says, writing what differs from the
    /// defaults.
    #[must_use]
    pub fn from_thm(t: &SoundThm) -> Self {
        let d = SoundThm::default();
        let some = |v: f32, def: f32| (v != def).then_some(f64::from(v));
        SoundAsset {
            source: None,
            name: None,
            min_distance: some(t.min_dist, d.min_dist),
            max_distance: some(t.max_dist, d.max_dist),
            volume: some(t.base_volume, d.base_volume),
            ai_distance: some(t.max_ai_dist, t.max_dist),
            ty: (t.game_type != d.game_type).then_some(t.game_type),
            quality: some(t.quality, d.quality),
        }
    }

    #[must_use]
    pub fn to_table(&self) -> Table {
        let mut top = Table::new();
        top.put("snd", 1);
        for f in SOUND {
            let v: Option<Value> = match f.key {
                "source" => self.source.clone().map(Value::from),
                "name" => self.name.clone().map(Value::from),
                "min_distance" => self.min_distance.map(Value::from),
                "max_distance" => self.max_distance.map(Value::from),
                "volume" => self.volume.map(Value::from),
                "ai_distance" => self.ai_distance.map(Value::from),
                // one word where one stands for it, else the words of its bits
                "type" => self.ty.map(|n| match fields::words_of(fields::SOUND_TYPES, n).as_slice() {
                    [one] => Value::from(*one),
                    many => Value::Table(Table {
                        arr: many.iter().map(|w| Value::from(*w)).collect(),
                        fields: Vec::new(),
                    }),
                }),
                "quality" => self.quality.map(Value::from),
                _ => None,
            };
            if let Some(v) = v {
                top.put(f.key, v);
            }
        }
        top
    }
}
