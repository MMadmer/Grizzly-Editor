//! Renders one offscreen frame of a hand-made scene - a textured box (selected, so outlined), an
//! alpha-tested fence, a glass pane, the grid and a few helpers - and writes it as a PNG; then
//! the same packet once more through the render thread and `capture`.
//!
//! ```text
//! cargo run -p gz-render --example offscreen -- [out.png]
//! ```

use std::sync::Arc;
use std::time::Duration;

use glam::{Mat4, Quat, Vec2, Vec3};
use gz_fmt_tex::{DdsEncoding, Image, MipFilter, encode_dds, mip_chain};
use gz_render::{
    AssetBytes, AssetRef, Camera, Color, DrawList, EditorMaterial, Environment, Instance, InstanceFlags,
    Light, LightKind, MaterialDesc, MeshData, Renderer, ShapeKind, TextureData, View, ViewMode, build_packet,
};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "Intermediate/gz-render-offscreen.png".to_owned());
    let rt = gz_task::Runtime::start(gz_task::ExecOptions::default()).map_err(|e| e.to_string())?;
    let (device, queue, info) = futures::executor::block_on(headless_device())?;
    println!("adapter: {} ({:?})", info.name, info.backend);
    let renderer = Renderer::new(device, queue, rt.exec()).map_err(|e| e.to_string())?;
    println!(
        "msaa: {}x, BC textures as they are: {}",
        renderer.samples(),
        renderer.block_compressed_textures()
    );
    let assets = renderer.assets().clone();

    // the fence texture comes through a loader, as a game's texture would: a PNG made here
    let fence_png = png_bytes(&fence(128))?;
    assets.set_loader(Some(Arc::new(move |what: AssetRef<'_>| match what {
        AssetRef::Texture("example\\fence") => {
            Ok(AssetBytes { bytes: fence_png.clone(), ext: Some("png".into()) })
        }
        other => Err(format!("{other:?} is not part of the example")),
    })));

    // the crate texture goes the DDS way: BC1 blocks straight to the GPU when it takes them
    let planks = planks(128);
    let mut levels = vec![planks.clone()];
    levels.extend(mip_chain(&planks, MipFilter::Box).map_err(|e| e.to_string())?);
    let dds = encode_dds(&levels, DdsEncoding::Dxt1).map_err(|e| e.to_string())?;
    let crate_tex = TextureData::decode(&dds, Some("dds"), renderer.block_compressed_textures())
        .map_err(|e| e.to_string())?;
    println!("crate texture: {:?}, {} levels", crate_tex.format, crate_tex.levels.len());
    assets.add_texture("example\\crate", &crate_tex).map_err(|e| e.to_string())?;

    let boxes = assets.add_mesh(
        "example\\box",
        MeshData::cube(MaterialDesc::textured(EditorMaterial::Opaque, "example\\crate")),
    );
    let fence = assets.add_mesh(
        "example\\fence",
        MeshData::quad(
            MaterialDesc::textured(EditorMaterial::AlphaTest, "example\\fence").with_two_sided(true),
            Vec2::new(2.0, 2.0),
        ),
    );
    let glass = assets.add_mesh(
        "example\\glass",
        MeshData::quad(
            MaterialDesc::color(EditorMaterial::Blended, Color::rgba(90, 170, 255, 110)).with_two_sided(true),
            Vec2::new(1.6, 1.2),
        ),
    );

    let mut list = DrawList::new();
    let box_at = Mat4::from_scale_rotation_translation(
        Vec3::splat(1.2),
        Quat::from_rotation_y(0.5),
        Vec3::new(-1.0, 0.6, 0.0),
    );
    list.push(Instance::new(boxes, box_at).with_id(1).with_flags(InstanceFlags::SELECTED));
    list.push(
        Instance::new(
            fence,
            Mat4::from_rotation_translation(Quat::from_rotation_y(-0.6), Vec3::new(1.5, 1.0, 0.4)),
        )
        .with_id(2),
    );
    list.push(
        Instance::new(
            glass,
            Mat4::from_rotation_translation(Quat::from_rotation_y(0.25), Vec3::new(0.3, 0.6, -1.5)),
        )
        .with_id(3),
    );

    let h = &mut list.helpers;
    h.shape(ShapeKind::SelectionBox, box_at, None, Some(Color::WHITE), true);
    h.sphere(Vec3::new(-1.0, 0.6, 0.0), 1.15, None, Some(Color::rgb(120, 220, 255)), true);
    h.arrow(Vec3::new(2.6, 0.0, -1.0), Vec3::new(2.6, 1.6, -1.0), 0.25, Color::rgb(255, 90, 60), false);
    h.cross(Vec3::new(0.0, 2.4, 1.0), 0.3, true, Color::rgb(255, 230, 80), true);
    h.cone(
        Vec3::new(-2.6, 1.8, 1.4),
        Vec3::new(0.3, -1.0, 0.0),
        1.2,
        0.5,
        Some(Color::rgba(255, 230, 80, 60)),
        Some(Color::rgb(255, 230, 80)),
        true,
    );
    h.label(Vec3::new(-1.0, 1.4, 0.0), "crate", Color::WHITE, true);
    h.label(Vec3::new(1.5, 0.0, 2.5), "behind the fence", Color::WHITE, true);
    h.label(Vec3::new(2.6, 1.8, -1.0), "up", Color::WHITE, false);
    // inside the box: hidden from a depth-tested label
    h.label(Vec3::new(-1.0, 0.6, 0.0), "inside", Color::WHITE, true);

    let camera = Camera::look_at(Vec3::new(3.6, 3.0, -5.2), Vec3::new(0.0, 0.6, 0.0));
    let view = View::new(camera, [800, 500]);

    let ready = assets.prepare(&list, Duration::from_secs(10));
    println!("assets ready: {ready}, {:?}", assets.stats());

    let packet = build_packet(&list, &view, &assets);
    println!("packet: {:?}", packet.stats);
    let picture = renderer.render_offscreen(&packet, view.size).map_err(|e| e.to_string())?;
    write_png(&out, &picture)?;
    println!("wrote {out} ({} x {})", picture.width, picture.height);

    // the render thread: submit, wait for the frame, take its picture
    let serial = renderer.submit(Arc::new(packet));
    let frame = renderer.wait_frame(serial, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    for l in frame.labels.iter() {
        println!("label {:?} at {:?}: visible {}", l.text, l.position, l.visible);
    }
    let shot = renderer.capture(serial, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let thread_out = out.replace(".png", "-thread.png");
    write_png(&thread_out, &shot)?;
    let target = renderer.target().ok_or("no target")?;
    println!(
        "frame {} of packet {}, target {:?} of {:?}, wrote {thread_out}",
        frame.frame, frame.packet, target.size, target.extent
    );
    if shot.rgba != picture.rgba {
        println!("note: the thread's picture differs from the offscreen one");
    }

    // the same scene in the game-lighting mode: a dusk weather, a lamp and a spot
    list.environment = Some(Environment {
        ambient: Vec3::new(0.06, 0.06, 0.08),
        hemi: Vec3::new(0.18, 0.2, 0.26),
        sun_color: Vec3::new(0.7, 0.45, 0.3),
        sun_dir: Vec3::new(-0.6, -0.35, 0.7).normalize(),
        fog_color: Vec3::new(0.32, 0.3, 0.34),
        fog_near: 5.0,
        fog_far: 40.0,
        exposure: 1.2,
        ..Environment::default()
    });
    list.lights.push(Light {
        kind: LightKind::Point,
        position: Vec3::new(0.2, 1.6, -0.6),
        range: 4.0,
        color: Vec3::new(1.2, 0.9, 0.5),
    });
    list.lights.push(Light {
        kind: LightKind::Spot { direction: Vec3::new(-0.2, -1.0, 0.1), cos_half: 0.85 },
        position: Vec3::new(1.5, 3.0, 0.4),
        range: 5.0,
        color: Vec3::new(0.4, 0.8, 1.4),
    });
    let game = View { mode: ViewMode::GameLighting, ..view };
    let packet = build_packet(&list, &game, &assets);
    let lit = renderer.render_offscreen(&packet, game.size).map_err(|e| e.to_string())?;
    let game_out = out.replace(".png", "-game.png");
    write_png(&game_out, &lit)?;
    println!("wrote {game_out}: {:?}", packet.stats);
    Ok(())
}

async fn headless_device() -> Result<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo), String> {
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
        .map_err(|e| format!("no graphics adapter: {e}"))?;
    let required_features = adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC;
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("gz-render example"),
            required_features,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("no graphics device: {e}"))?;
    Ok((device, queue, adapter.get_info()))
}

/// Wooden planks with dark seams and a frame round the edge.
fn planks(side: u32) -> Image {
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let plank = y / (side / 4);
            let grain = ((x as f32 * 0.21 + plank as f32 * 7.0).sin() * 0.5 + 0.5) * 22.0;
            let seam = y % (side / 4) < 3;
            let border = x < 6 || y < 6 || x >= side - 6 || y >= side - 6;
            let (r, g, b) = if border {
                (90.0, 60.0, 35.0)
            } else if seam {
                (60.0, 38.0, 20.0)
            } else {
                (170.0 + grain - plank as f32 * 8.0, 118.0 + grain * 0.7, 62.0 + grain * 0.3)
            };
            rgba.extend_from_slice(&[r as u8, g as u8, b as u8, 255]);
        }
    }
    Image { width: side, height: side, rgba }
}

/// A green mesh fence: wires and holes, the holes transparent.
fn fence(side: u32) -> Image {
    let mut rgba = Vec::with_capacity((side * side * 4) as usize);
    let cell = side / 8;
    for y in 0..side {
        for x in 0..side {
            let (cx, cy) = ((x + y) % cell, (x + side - y) % cell);
            let wire = cx < 3 || cy < 3;
            let frame = x < 5 || y < 5 || x >= side - 5 || y >= side - 5;
            if frame {
                rgba.extend_from_slice(&[70, 80, 60, 255]);
            } else if wire {
                rgba.extend_from_slice(&[110, 170, 90, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Image { width: side, height: side, rgba }
}

fn png_bytes(img: &Image) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&img.rgba).map_err(|e| e.to_string())?;
        writer.finish().map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

fn write_png(path: &str, img: &Image) -> Result<(), String> {
    if let Some(dir) = std::path::Path::new(path).parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, png_bytes(img)?).map_err(|e| format!("{path}: {e}"))
}
