//! Every effect and group of the game's `particles.xr` (and of the editor's own) plays for one
//! second of simulated time, in the engine's 33 ms steps, without a panic and without a value
//! that is not a number.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use glam::{Mat4, Vec3};
use gz_corpus::{check_all, finish};
use gz_particles::{Library, SpriteBatch};

fn finite(v: Vec3) -> bool {
    v.is_finite()
}

/// Plays `name` from a moving emitter for a second and checks every particle and sprite.
fn play(lib: &Arc<Library>, name: &str, alive: &AtomicUsize) -> Result<(), String> {
    let mut sys = lib.create(name).ok_or("the library cannot make it")?.with_seed(7);
    sys.play();
    let mut batches: Vec<SpriteBatch> = Vec::new();
    for step in 0..30 {
        let at = Mat4::from_translation(Vec3::new(step as f32 * 0.1, 0.0, 0.0));
        sys.update_parent(&at, Vec3::new(3.0, 0.0, 0.0), false);
        sys.on_frame(33, None);
        for p in sys.particles() {
            if !(finite(p.pos) && finite(p.vel) && finite(p.size) && finite(p.rot) && p.age.is_finite()) {
                return Err(format!("a particle is not finite at step {step}: {p:?}"));
            }
        }
        if let Some((lo, hi)) = sys.bounds()
            && !(finite(lo) && finite(hi))
        {
            return Err(format!("the bounds are not finite at step {step}"));
        }
        batches.clear();
        sys.sprites(&mut batches);
        for b in &batches {
            if b.sprites.iter().any(|s| !(finite(s.pos) && s.half.is_finite() && s.angle.is_finite())) {
                return Err(format!("a sprite is not finite at step {step}"));
            }
        }
    }
    alive.fetch_add(sys.particle_count(), Ordering::Relaxed);
    Ok(())
}

fn play_all(what: &str, bytes: &[u8]) {
    let started = Instant::now();
    let file = gz_fmt_xr::Particles::read(bytes).unwrap_or_else(|e| panic!("{what}: {e}"));
    let lib = Arc::new(Library::new(&file));
    let names: Vec<String> = lib
        .effects()
        .iter()
        .map(|e| e.name.to_string())
        .chain(lib.groups().iter().map(|g| g.name.to_string()))
        .collect();
    let alive = AtomicUsize::new(0);
    let failed = check_all(&names, Clone::clone, |n| {
        catch_unwind(AssertUnwindSafe(|| play(&lib, n, &alive))).unwrap_or_else(|_| Err("panicked".into()))
    });
    let alive = alive.into_inner();
    println!("{what}: {alive} particles alive after a second");
    // the systems do emit
    assert!(alive > names.len(), "{alive}");
    finish(what, names.len(), started, &failed);
}

#[test]
fn every_effect_and_group_of_the_editor_plays_a_second() {
    play_all(
        "editor particles",
        &std::fs::read(gz_corpus::repo().join("gamedata").join("particles.xr")).unwrap(),
    );
}

#[test]
fn every_effect_and_group_of_the_game_plays_a_second() {
    let Some(game) = gz_corpus::game() else { return };
    let game = game.unwrap();
    play_all("game particles", &game.read_path("particles.xr").unwrap());
}
