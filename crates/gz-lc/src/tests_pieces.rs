//! Pieces of a level: a module's piece lit beside its occluders, a level cut into streaming cells.

use std::collections::BTreeMap;

use gz_fmt_level::{BuildProject, CompiledLevel, GeomTables, Occluder, PIECE_SECTOR_AUTO, VisualKind};
use gz_lc_light::{CpuLighter, Ctl, Lighter, NullLighter, Shaders};

use crate::tests::{Scaled, project};
use crate::{Log, Options, Output, PIECE_NAME, compile_project};

fn compile(
    prj: &BuildProject,
    shaders: &Shaders,
    textures: &[gz_lc_light::BuildTexture],
    opts: &Options,
    lighter: &dyn Lighter,
    threads: usize,
) -> Output {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.install(|| {
        let mut log = Log::default();
        compile_project(prj, shaders.clone(), textures.to_vec(), opts, lighter, &Ctl::quiet(), &mut log)
            .unwrap()
    })
}

/// The test level as a piece's build: the floor is the piece, the box on it becomes occluders
/// (with `shadows`) - faces only the light's rays meet; no portal, no glow.
fn piece_project(shadows: bool) -> (BuildProject, Shaders, Vec<gz_lc_light::BuildTexture>) {
    let (mut prj, shaders, textures) = project();
    prj.params.lm_pixels_per_meter = 6.0;
    let box_texture = prj.materials[2].surface;
    let (mut faces, mut groups, mut occluders) = (Vec::new(), Vec::new(), Vec::new());
    for (f, g) in prj.faces.iter().zip(prj.smooth_groups.iter()) {
        if f.material == 2 {
            occluders.push(Occluder { p: f.v.map(|v| prj.verts[v as usize]), t: f.t, texture: box_texture });
        } else {
            faces.push(*f);
            groups.push(*g);
        }
    }
    prj.faces = faces.into();
    prj.smooth_groups = groups.into();
    prj.portals.clear();
    prj.glows.clear();
    if shadows {
        prj.occluders = occluders;
    }
    (prj, shaders, textures)
}

fn file<'a>(files: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a [u8]> {
    files.iter().find(|(n, _)| n == name).map(|(_, b)| b.as_slice())
}

/// What the engine checks before it loads a piece, on a piece's files.
fn engine_problems(files: &[(String, Vec<u8>)]) -> (CompiledLevel, Vec<String>) {
    let piece = CompiledLevel::read_piece(file(files, "piece").unwrap()).unwrap();
    let geom = GeomTables::read(file(files, "piece.geom").unwrap()).unwrap();
    let fast = file(files, "piece.geomx").map(|b| GeomTables::read(b).unwrap());
    let problems = piece.piece_problems(&geom, fast.as_ref());
    (piece, problems)
}

#[test]
fn a_piece_compiles_into_what_the_engine_loads_and_its_occluders_shadow_it() {
    let (prj, shaders, textures) = piece_project(true);
    let opts = Options { piece: true, ..Options::default() };
    let out = compile(&prj, &shaders, &textures, &opts, &CpuLighter::default(), 4);
    // the level's own files are not made: no level, no collision
    for n in ["level", "level.geom", "level.cform"] {
        assert!(file(&out.files, n).is_none(), "{n} was made for a piece");
    }
    assert_eq!(out.pieces.len(), 1);
    let p = &out.pieces[0];
    assert_eq!(p.name, PIECE_NAME);
    let names: Vec<&str> = p.files.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["piece", "piece.geom", "piece.ltx", "lmap#1_1.dds", "lmap#1_2.dds"] {
        assert!(names.contains(&want), "{want} missing: {names:?}");
    }
    let (piece, problems) = engine_problems(&p.files);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(piece.version, 14);
    assert!(piece.sectors.is_empty() && piece.portals.is_empty() && piece.lights.is_empty());
    assert!(!piece.roots.is_empty() && piece.roots.iter().all(|r| r.sector == PIECE_SECTOR_AUTO));
    // the chunks as the engine reads them: 1, the visuals and the shader table, 32 - nothing more
    let ids: Vec<u32> =
        gz_chunk::Reader::new(file(&p.files, "piece").unwrap()).chunks().map(|c| c.0).collect();
    assert_eq!(ids, [1, 3, 2, 32]);
    // the shader table names the piece's own pages
    assert!(piece.shaders.iter().any(|s| s.contains("lmap#1_1")), "{:?}", piece.shaders);
    let ltx = String::from_utf8(file(&p.files, "piece.ltx").unwrap().to_vec()).unwrap();
    assert!(ltx.starts_with("[piece]\r\nversion = 1\r\nbox     = "), "{ltx}");
    assert!(ltx.contains("sector  = auto\r\nstream  = true\r\n"), "{ltx}");

    // the box on the floor shadows it: the floor's light differs without it
    let (bare, s2, t2) = piece_project(false);
    let alone = compile(&bare, &s2, &t2, &opts, &CpuLighter::default(), 4);
    let pages = |o: &Output| -> Vec<Vec<u8>> {
        o.pieces[0].files.iter().filter(|(n, _)| n.starts_with("lmap#")).map(|(_, b)| b.clone()).collect()
    };
    assert_ne!(pages(&out), pages(&alone), "the occluders cast no shadow on the piece");
    // ...and are no part of it: the same visuals and faces with them and without
    assert_eq!(out.structure, alone.structure, "the occluders changed the piece's structure");

    // the structure does not follow the light, the lighter or the threads
    let scaled = compile(&prj, &shaders, &textures, &opts, &Scaled::default(), 4);
    let one = compile(&prj, &shaders, &textures, &opts, &CpuLighter::default(), 1);
    let none = compile(&prj, &shaders, &textures, &opts, &NullLighter::default(), 8);
    assert_eq!(out.structure, scaled.structure, "a scaled light changed the piece's structure");
    assert_eq!(out.structure, one.structure, "one thread changed the piece's structure");
    assert_eq!(out.structure, none.structure, "no light changed the piece's structure");
    assert_ne!(pages(&out), pages(&scaled));
    // one thread and four make the same bytes
    assert_eq!(out.pieces[0].files, one.pieces[0].files);
}

#[test]
fn occluders_are_only_a_pieces_and_the_options_read_from_a_command_line() {
    let (prj, shaders, textures) = piece_project(true);
    let mut log = Log::default();
    let refused = compile_project(
        &prj,
        shaders,
        textures,
        &Options::default(),
        &CpuLighter::default(),
        &Ctl::quiet(),
        &mut log,
    );
    assert!(refused.is_err());
    let (o, unknown) = Options::parse(["-piece", "-cells=48"]).unwrap();
    assert!(o.piece && o.cells == 48 && unknown.is_empty());
    assert!(Options::parse(["-cells=x"]).is_err());
    assert_eq!(Options::parse(["cells=0"]).unwrap().0.cells, 0);
}

/// Every face of the leaf visuals of a level or a piece, as its corners' bits (sorted per face),
/// and the static models' references as their transforms' bits.
fn faces_of(level: &CompiledLevel, geom: &[u8]) -> (Vec<[u32; 9]>, Vec<[u32; 16]>) {
    let top: BTreeMap<u32, &[u8]> = gz_chunk::Reader::new(geom).chunks().map(|c| (c.0, c.2)).collect();
    // the vertex buffers' positions: the first float3 of every vertex
    let mut vbs: Vec<Vec<[f32; 3]>> = Vec::new();
    let mut r = gz_chunk::Reader::new(top[&9]);
    for _ in 0..r.r_u32().unwrap() {
        let mut stride = 0usize;
        loop {
            let e = r.take(8).unwrap();
            if e[0] == 0xFF {
                break;
            }
            let size = match e[4] {
                2 => 12,
                3 => 16,
                1 | 7 | 10 | 12 | 16 => 8,
                _ => 4,
            };
            stride = stride.max(usize::from(u16::from_le_bytes([e[2], e[3]])) + size);
        }
        let n = r.r_u32().unwrap() as usize;
        let data = r.take(n * stride).unwrap();
        let f = |o: usize| f32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        vbs.push((0..n).map(|k| [f(k * stride), f(k * stride + 4), f(k * stride + 8)]).collect());
    }
    let mut ibs: Vec<Vec<u16>> = Vec::new();
    let mut r = gz_chunk::Reader::new(top[&10]);
    for _ in 0..r.r_u32().unwrap() {
        let n = r.r_u32().unwrap() as usize;
        let b = r.take(n * 2).unwrap();
        ibs.push(b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect());
    }
    let mut faces = Vec::new();
    let mut refs = Vec::new();
    for v in &level.visuals {
        match v.kind {
            VisualKind::Normal | VisualKind::Progressive => {
                let g = v.geometry.unwrap();
                let (vb, ib) = (&vbs[g.vertex_buffer as usize], &ibs[g.index_buffer as usize]);
                for t in 0..g.faces() as usize {
                    let mut c: Vec<[u32; 3]> = (0..3)
                        .map(|k| {
                            let i = ib[g.index_start as usize + t * 3 + k] as usize;
                            vb[g.vertex_start as usize + i].map(f32::to_bits)
                        })
                        .collect();
                    c.sort_unstable();
                    faces.push([
                        c[0][0], c[0][1], c[0][2], c[1][0], c[1][1], c[1][2], c[2][0], c[2][1], c[2][2],
                    ]);
                }
            }
            VisualKind::TreeStatic | VisualKind::TreeProgressive => {
                let m = v.transform.unwrap();
                let mut bits = [0u32; 16];
                for (k, x) in m.iter().flatten().enumerate() {
                    bits[k] = x.to_bits();
                }
                refs.push(bits);
            }
            _ => {}
        }
    }
    (faces, refs)
}

/// `cell_<x>_<z>` back into its cell.
fn cell_of_name(name: &str) -> [i32; 2] {
    let mut it = name.trim_start_matches(crate::CELL_PREFIX).split('_').map(|x| x.parse::<i32>().unwrap());
    [it.next().unwrap(), it.next().unwrap()]
}

/// A level of the test project cut into cells of `size` metres: the files of the level and of
/// every cell's piece, checked against the whole level.
fn cells_hold_the_whole_level(size: u32) -> (usize, usize) {
    let (mut prj, shaders, textures) = project();
    prj.params.lm_pixels_per_meter = 6.0;
    let whole = compile(&prj, &shaders, &textures, &Options::default(), &CpuLighter::default(), 4);
    let opts = Options { cells: size, ..Options::default() };
    let cut = compile(&prj, &shaders, &textures, &opts, &CpuLighter::default(), 4);
    assert!(!cut.pieces.is_empty(), "no cell at {size} m");
    let level = |o: &Output| CompiledLevel::read(file(&o.files, "level").unwrap()).unwrap();
    let (wl, cl) = (level(&whole), level(&cut));
    // the level keeps its sectors, portals, lights and glows
    assert_eq!(cl.sectors.len(), wl.sectors.len());
    assert_eq!(
        (cl.portals.len(), cl.lights.len(), cl.glows.len()),
        (wl.portals.len(), wl.lights.len(), wl.glows.len())
    );
    assert!(file(&cut.files, "level.cform") == file(&whole.files, "level.cform"), "the collision changed");

    let (mut faces, mut refs) = faces_of(&cl, file(&cut.files, "level.geom").unwrap());
    let kept = faces.len();
    // the level keeps only what is larger than a cell
    for v in cl.visuals.iter().filter(|v| v.kind.has_geometry() && v.transform.is_none()) {
        let s = v.aabb.max - v.aabb.min;
        assert!(
            s.x > size as f32 - 0.01 || s.z > size as f32 - 0.01,
            "a visual of {s:?} stayed in the level"
        );
    }
    for p in &cut.pieces {
        let (piece, problems) = engine_problems(&p.files);
        assert!(problems.is_empty(), "{}: {problems:?}", p.name);
        assert!(file(&p.files, "piece.ltx").is_some());
        // each root hangs on a sector of the level, named
        assert!(piece.roots.iter().all(|r| usize::from(r.sector) < cl.sectors.len()), "{:?}", piece.roots);
        // a visual of a cell lies in its cell: no larger than one, its centre inside
        let cell = cell_of_name(&p.name);
        for v in piece.visuals.iter().filter(|v| v.kind.has_geometry()) {
            let (s, c) = (v.aabb.max - v.aabb.min, (v.aabb.min + v.aabb.max) * 0.5);
            assert!(s.x <= size as f32 + 0.01 && s.z <= size as f32 + 0.01, "{}: a visual of {s:?}", p.name);
            let at = crate::tree::cell_of(c, size as f32);
            assert_eq!(at, cell, "{}: a visual centred at {c:?}", p.name);
        }
        let (f, r) = faces_of(&piece, file(&p.files, "piece.geom").unwrap());
        faces.extend(f);
        refs.extend(r);
        // the cell's own pages
        assert!(
            p.files.iter().all(|(n, _)| !n.starts_with("lmap#")
                || piece.shaders.iter().any(|s| s.contains(&n[..n.len() - 6])))
        );
    }
    // the level's visuals and every cell's together hold the faces of the level compiled whole
    let (mut wf, mut wr) = faces_of(&wl, file(&whole.files, "level.geom").unwrap());
    for list in [&mut faces, &mut wf] {
        list.sort_unstable();
    }
    refs.sort_unstable();
    wr.sort_unstable();
    assert_eq!(faces.len(), wf.len(), "the cells lost or doubled faces");
    assert!(faces == wf, "the cells hold other faces than the whole level");
    assert_eq!(refs, wr, "the cells hold other placements of the static models");

    // the structure of cells does not follow the light or the threads either
    let scaled = compile(&prj, &shaders, &textures, &opts, &Scaled::default(), 4);
    let one = compile(&prj, &shaders, &textures, &opts, &NullLighter::default(), 1);
    assert_eq!(cut.structure, scaled.structure);
    assert_eq!(cut.structure, one.structure);
    let names = |o: &Output| o.pieces.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&cut), names(&scaled));
    (kept, cut.pieces.len())
}

#[test]
fn the_cells_and_the_level_together_are_the_whole_level() {
    // 64 m: every visual goes into a cell, the level keeps its sectors' empty roots
    let (kept, cells) = cells_hold_the_whole_level(64);
    assert!(kept == 0 && cells >= 2, "{kept} faces kept by the level, {cells} cells");
    // 32 m and 8 m: the floor's halves (24 x 48 m) are larger than a cell and stay in the level
    for size in [32, 8] {
        let (kept, cells) = cells_hold_the_whole_level(size);
        assert!(kept > 0 && cells >= 2, "{size} m: {kept} faces kept by the level, {cells} cells");
    }
}

/// The pieces take their folders whole; the cells an earlier compile left and this one does not
/// write go, a module's piece of another name stays.
#[test]
fn the_pieces_replace_their_folders_and_stale_cells_go() {
    let dir = std::env::temp_dir().join(format!("gzlc_test_pieces_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, text) in [
        ("cell_9_9/piece", "old"),
        ("cell_0_0/piece", "old"),
        ("cell_0_0/lmap#3_1.dds", "old"),
        ("main/piece", "mine"),
    ] {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let pieces = vec![crate::PieceFiles {
        name: "cell_0_0".into(),
        files: vec![("piece".into(), b"new".to_vec()), ("lmap#1_1.dds".into(), b"p".to_vec())],
    }];
    let mut report = crate::Report::default();
    crate::write_pieces(&dir, &pieces, true, &mut report).unwrap();
    let mut left: Vec<String> = walkdir(&dir);
    left.sort();
    assert_eq!(left, ["cell_0_0/lmap#1_1.dds", "cell_0_0/piece", "main/piece"]);
    assert_eq!(std::fs::read(dir.join("cell_0_0/piece")).unwrap(), b"new");
    assert_eq!(report.pieces.len(), 1);
    // a module's piece build leaves the cells alone, and replaces its own folder whole
    let main =
        vec![crate::PieceFiles { name: "main".into(), files: vec![("piece.ltx".into(), b"x".to_vec())] }];
    crate::write_pieces(&dir, &main, false, &mut crate::Report::default()).unwrap();
    let mut left: Vec<String> = walkdir(&dir);
    left.sort();
    assert_eq!(left, ["cell_0_0/lmap#1_1.dds", "cell_0_0/piece", "main/piece.ltx"]);
    // a level without cells: every cell goes
    crate::write_pieces(&dir, &[], true, &mut crate::Report::default()).unwrap();
    assert_eq!(walkdir(&dir), ["main/piece.ltx"]);
    let _ = std::fs::remove_dir_all(&dir);
}

fn walkdir(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                out.push(e.path().strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}
