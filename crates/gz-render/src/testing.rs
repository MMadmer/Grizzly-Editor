//! A graphics device for tests, which must neither hang nor fail on a machine without one.
//!
//! A process stuck in the GPU driver once made device creation hang for every program until a
//! restart: a test that needs a device asks [`test_device`], which gives up after a limit and
//! says so, and the test then passes as skipped.

use std::sync::mpsc;
use std::time::Duration;

/// How long [`test_device`] waits by default.
pub const TEST_DEVICE_LIMIT: Duration = Duration::from_secs(5);

/// A device and its queue, or `None` (printed as `skipped: no graphics device`) when none comes
/// within `limit`. The creation runs on a thread of its own; one that hangs is left behind.
#[must_use]
pub fn test_device(limit: Duration) -> Option<(wgpu::Device, wgpu::Queue)> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("gz-test-device".into()).spawn(move || {
        let _ = tx.send(futures::executor::block_on(create()));
    });
    let got = match spawned {
        Ok(_) => rx.recv_timeout(limit).ok().flatten(),
        Err(_) => None,
    };
    if got.is_none() {
        println!("skipped: no graphics device");
    }
    got
}

async fn create() -> Option<(wgpu::Device, wgpu::Queue)> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::DX12 | wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc);
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        })
        .await
        .ok()?;
    let required_features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("gz-render test"),
            required_features,
            ..Default::default()
        })
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use glam::{Mat4, Vec3};

    use super::{TEST_DEVICE_LIMIT, test_device};
    use crate::{
        Camera, Color, DrawList, EditorMaterial, Instance, MaterialDesc, MeshData, Renderer, View,
        build_packet,
    };

    #[test]
    fn a_red_box_renders_offscreen() {
        let Some((device, queue)) = test_device(TEST_DEVICE_LIMIT) else { return };
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 2, io_threads: 1 })
                .unwrap();
        let renderer = Renderer::new(device, queue, gz_task::Gate::default(), rt.exec()).unwrap();
        let assets = renderer.assets().clone();
        let red = assets.add_mesh(
            "test\\box",
            MeshData::cube(MaterialDesc::color(EditorMaterial::SelfLit, Color::rgb(255, 0, 0))),
        );
        let mut list = DrawList::new();
        list.push(Instance::new(red, Mat4::from_scale(Vec3::splat(2.0))));
        let view = View::new(Camera::look_at(Vec3::new(0.0, 0.0, -6.0), Vec3::ZERO), [64, 48]);
        assert!(assets.prepare(&list, Duration::from_secs(5)));
        let picture = renderer.render_offscreen(&build_packet(&list, &view, &assets), view.size).unwrap();
        assert_eq!((picture.width, picture.height), (64, 48));
        // the middle of the picture is the box's face, the corner the background
        let at = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            [picture.rgba[i], picture.rgba[i + 1], picture.rgba[i + 2]]
        };
        let middle = at(32, 24);
        // red, under the editor's studio shading
        assert!(
            middle[0] > middle[1].saturating_add(80) && middle[0] > middle[2].saturating_add(80),
            "{middle:?}"
        );
        assert_ne!(at(0, 0), middle);
    }
}
