//! What a scene pass draws besides [`crate::draw::build`]: the parts that need the camera, the
//! pick tree or state kept between frames - particle systems playing, glows' sprites, the AI
//! map, detail objects, static wallmarks. Run on the CPU pool, never on the UI thread; the
//! caches are this module's `Editor` service.

use std::sync::Arc;

use glam::Vec3;
use gz_render::{
    Assets, Camera, Color, InstanceFlags, MaterialDesc, Sprite, SpriteAlign, SpriteBatch, SpriteCull,
    SpriteSize,
};
use gz_scene::library::Library;
use gz_scene::objects::VIS_RADIUS;
use gz_scene::pick::{ScenePick, ray_scene_objects};
use gz_scene::{ObjClass, ObjData, SceneState};
use parking_lot::Mutex;

use crate::draw::{Built, GLOW_COLOR};
use crate::draw_aimap::AiMapCache;
use crate::draw_details::{self, DetailCache};
use crate::draw_particles::{self, Particles};
use crate::draw_wallmarks::WallmarkCache;

/// The glow test-visibility flag.
const GLOW_TEST_VISIBILITY: u32 = 1 << 31;
/// The glow's fixed-size flag.
const GLOW_FIXED_SIZE: u16 = 1 << 0;
/// The discard flag of the editor's renderer.
const SSA_DISCARD: f32 = 4.0;

/// The caches of the extras: an `Editor` service. No lock is ever held across the drawing - it runs
/// rayon work and calls into the renderer's assets, and a pool worker waiting for a lock its own
/// stack holds would stall the pool (see [`Shared`]).
#[derive(Default)]
pub struct Extras {
    aimap: Shared<AiMapCache>,
    details: Shared<DetailCache>,
    wallmarks: Shared<WallmarkCache>,
}

/// A cache passes share: the finished one with its version. A pass draws with its own copy of the
/// finished cache (a copy shares the meshes and slots, so it costs a few reference counts) and
/// swaps it in when no other pass finished one meanwhile; a newer one stays. A pass never starts
/// from nothing and never makes again what a finished pass made; only two passes that meet the
/// same change at the same moment both make it (waiting for the other could stall the pool).
#[derive(Default)]
pub(crate) struct Shared<T> {
    slot: Mutex<(u64, Arc<T>)>,
}

impl<T: Clone> Shared<T> {
    pub(crate) fn with(&self, f: impl FnOnce(&mut T)) {
        let (version, finished) = {
            let s = self.slot.lock();
            (s.0, s.1.clone())
        };
        let mut mine = Arc::unwrap_or_clone(finished);
        f(&mut mine);
        let mut s = self.slot.lock();
        if s.0 == version {
            *s = (version + 1, Arc::new(mine));
        }
    }

    #[cfg(test)]
    fn version(&self) -> u64 {
        self.slot.lock().0
    }
}

impl Extras {
    #[must_use]
    pub fn of(editor: &gz_editor::Editor) -> Arc<Extras> {
        editor.services.get_or_init(Extras::default)
    }
}

/// What a pass hands the extras.
pub(crate) struct Input<'a> {
    pub state: &'a SceneState,
    pub lib: &'a Library,
    pub pick: Option<&'a ScenePick>,
    pub assets: &'a Assets,
    pub camera: &'a Camera,
    pub size: [u32; 2],
    pub particles: Option<Arc<gz_particles::Library>>,
    /// Show AI Map.
    pub ai_map_shown: bool,
}

/// Adds the extras to the pass's list. `true` while a particle system in view plays: the
/// viewport keeps drawing then.
pub(crate) fn extend(extras: &Extras, parts: &Particles, x: &Input<'_>, built: &mut Built) -> bool {
    let frustum = x.camera.frustum(x.size);
    let eye = x.camera.position;
    let screen = (x.size[0].max(1) as f32) * (x.size[1].max(1) as f32);
    let ssa_discard = SSA_DISCARD * SSA_DISCARD / screen;
    let playing = draw_particles::step_and_draw(
        parts,
        &draw_particles::Pass {
            state: x.state,
            lib: x.lib,
            pick: x.pick,
            library: x.particles.clone(),
            assets: x.assets,
            frustum: &frustum,
            shown: x.state.tools.part(ObjClass::Particles).visible,
        },
        &mut built.list,
    );
    glows(x, built);
    if x.ai_map_shown {
        extras.aimap.with(|c| c.draw(x.state, x.assets, eye, built));
    }
    extras.details.with(|c| {
        c.draw(
            &draw_details::Pass {
                state: x.state,
                lib: x.lib,
                assets: x.assets,
                frustum: &frustum,
                eye,
                ssa_discard,
            },
            built,
        );
    });
    extras.wallmarks.with(|c| c.draw(x.state, x.assets, &frustum, eye, ssa_discard / 4.0, built));
    playing
}

/// The glow's sprite with its texture and shader at its radius - unless the
/// tool tests visibility and a scene object stands between it and the camera - and the romboid
/// of a tested glow.
fn glows(x: &Input<'_>, built: &mut Built) {
    let tool = x.state.tools.part(ObjClass::Glow);
    if !tool.visible {
        return;
    }
    let test = tool.flags & GLOW_TEST_VISIBILITY != 0;
    let eye = x.camera.position;
    let table = x.assets.table();
    for o in x.state.of_class(ObjClass::Glow) {
        let ObjData::Glow(g) = &o.data else { continue };
        if !o.flags.visible {
            continue;
        }
        let p = o.position();
        let mut seen = true;
        if test {
            let to_eye = eye - p;
            let dist = to_eye.length();
            if let (Some(pick), Some(dir)) = (x.pick, to_eye.try_normalize()) {
                seen = ray_scene_objects(pick, x.state, x.lib, p, dir, dist, None).is_none();
            }
            built.list.helpers.shape(
                gz_render::ShapeKind::Romboid,
                glam::Mat4::from_scale_rotation_translation(Vec3::splat(VIS_RADIUS), glam::Quat::IDENTITY, p),
                None,
                Some(Color::from_argb(GLOW_COLOR).with_alpha(255)),
                true,
            );
        }
        if !seen {
            continue;
        }
        // without a shader and a texture the editor draws the sprite with its wire shader
        let material = if g.texture.is_empty() || g.shader.is_empty() {
            MaterialDesc::color(gz_render::EditorMaterial::UnlitColor, Color::WHITE)
        } else {
            MaterialDesc::textured(table.classify(&g.shader), &g.texture)
        };
        built.list.sprites.push(SpriteBatch {
            material: x.assets.material(material),
            sprites: vec![Sprite {
                position: p,
                half: glam::Vec2::splat(g.radius),
                angle: 0.0,
                color: Color::WHITE,
                uv: [0.0, 0.0, 1.0, 1.0],
                align: SpriteAlign::Camera,
            }],
            cull: SpriteCull::None,
            size: if g.flags & GLOW_FIXED_SIZE != 0 { SpriteSize::ScreenFixed } else { SpriteSize::Screen },
            bounds: None,
            id: o.id.0,
            flags: InstanceFlags::NO_ID,
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rayon::prelude::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn shared(v: Vec<u32>) -> Shared<Vec<u32>> {
        Shared { slot: Mutex::new((0, Arc::new(v))) }
    }

    #[test]
    fn a_cache_is_unlocked_while_it_draws() {
        let cache = shared(vec![1, 2, 3]);
        cache.with(|c| {
            assert!(cache.slot.try_lock().is_some(), "the lock is free while the cache is in use");
            c.push(4);
        });
        assert_eq!(*cache.slot.lock().1, [1, 2, 3, 4]);
        assert_eq!(cache.version(), 1);
    }

    /// A pass that runs while another draws starts from the finished cache, not from nothing, and
    /// the cache the first one finishes is the one kept: the change is made once.
    #[test]
    fn a_pass_beside_another_starts_from_the_finished_cache() {
        let cache = shared(vec![1, 2, 3]);
        let mut rebuilt = 0;
        cache.with(|outer| {
            cache.with(|inner| {
                assert_eq!(*inner, [1, 2, 3], "the second pass sees the finished cache");
                if inner.len() < 4 {
                    rebuilt += 1;
                    inner.push(4);
                }
            });
            // the outer pass's copy was taken before; its result is older than the one now in
            assert_eq!(*outer, [1, 2, 3]);
            outer.push(9);
        });
        assert_eq!(*cache.slot.lock().1, [1, 2, 3, 4], "the newer finished cache stays");
        assert_eq!(cache.version(), 1);
        // a later pass finds the change made and rebuilds nothing
        cache.with(|c| {
            if c.len() < 4 {
                rebuilt += 1;
            }
        });
        assert_eq!(rebuilt, 1);
        assert_eq!(cache.version(), 2);
    }

    /// Passes built from several threads at once on a two-thread pool, each running parallel work
    /// with its cache out, while a job on the same pool takes the same caches inside its own
    /// parallel loop: with a lock held across the parallel work, a worker that steals the other's
    /// task waits for the lock its own stack holds and the pool stalls (the deadlock the editor's
    /// stress run found). Bounded: a stall fails the test instead of hanging it.
    #[test]
    fn passes_and_a_job_on_the_same_caches_never_stall_the_pool() {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
            let extras = Extras::default();
            let parts = Particles::default();
            let state = SceneState::default();
            let lib = Library::new(Vec::new());
            let assets = Assets::headless(gz_render::MaterialTable::default());
            let camera = Camera::look_at(Vec3::new(0.0, 10.0, -10.0), Vec3::ZERO);
            let input = Input {
                state: &state,
                lib: &lib,
                pick: None,
                assets: &assets,
                camera: &camera,
                size: [64, 64],
                particles: None,
                ai_map_shown: true,
            };
            std::thread::scope(|s| {
                for _ in 0..4 {
                    s.spawn(|| {
                        for _ in 0..200 {
                            pool.install(|| {
                                let mut built = Built::default();
                                extend(&extras, &parts, &input, &mut built);
                                // what a cache's drawing does: parallel work with the cache out
                                extras.details.with(|_| {
                                    (0..64).into_par_iter().for_each(|_| {
                                        extras.details.with(|_| {});
                                        drop(parts.state.lock());
                                    });
                                });
                            });
                        }
                    });
                }
                s.spawn(|| {
                    for _ in 0..200 {
                        pool.install(|| {
                            (0..64).into_par_iter().for_each(|_| {
                                extras.wallmarks.with(|_| {});
                                extras.aimap.with(|_| {});
                                drop(parts.state.lock());
                            });
                        });
                    }
                });
            });
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_secs(60)).is_ok(), "the pool stalled");
    }
}
