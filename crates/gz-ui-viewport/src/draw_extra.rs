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

/// `ESceneGlowTool::flTestVisibility`.
const GLOW_TEST_VISIBILITY: u32 = 1 << 31;
/// `CGlow::gfFixedSize`.
const GLOW_FIXED_SIZE: u16 = 1 << 0;
/// `ssaDISCARD` of the editor's renderer.
const SSA_DISCARD: f32 = 4.0;

/// The caches of the extras: an `Editor` service. A pass takes a cache out under its lock, draws
/// with it unlocked - the drawing runs rayon work and calls into the renderer's assets - and puts it
/// back: no lock is ever held across parallel work (a pool worker waiting for a lock its own stack
/// holds would stall the pool).
#[derive(Default)]
pub struct Extras {
    aimap: Mutex<AiMapCache>,
    details: Mutex<DetailCache>,
    wallmarks: Mutex<WallmarkCache>,
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
        unlocked(&extras.aimap, |c| c.draw(x.state, x.assets, eye, built));
    }
    unlocked(&extras.details, |c| {
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
    unlocked(&extras.wallmarks, |c| c.draw(x.state, x.assets, &frustum, eye, ssa_discard / 4.0, built));
    playing
}

/// Runs `f` on the cache taken out of its lock, and puts it back: a pass that runs meanwhile finds
/// an empty cache and builds what it needs itself.
fn unlocked<T: Default>(cache: &Mutex<T>, f: impl FnOnce(&mut T)) {
    let mut taken = std::mem::take(&mut *cache.lock());
    f(&mut taken);
    *cache.lock() = taken;
}

/// `CGlow::Render`: the glow's sprite with its texture and shader at its radius - unless the
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

    #[test]
    fn a_cache_is_unlocked_while_it_draws() {
        let cache: Mutex<Vec<u32>> = Mutex::new(vec![1, 2, 3]);
        unlocked(&cache, |c| {
            assert!(cache.try_lock().is_some(), "the lock is free while the cache is in use");
            c.push(4);
        });
        assert_eq!(*cache.lock(), [1, 2, 3, 4]);
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
                                unlocked(&extras.details, |_| {
                                    (0..64).into_par_iter().for_each(|_| {
                                        unlocked(&extras.details, |_| {});
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
                                unlocked(&extras.wallmarks, |_| {});
                                unlocked(&extras.aimap, |_| {});
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
