use glam::{Vec2, Vec3};

use crate::grid::weight;
use crate::*;

fn flat_terrain(edits: Vec<Edit>) -> Terrain {
    Terrain {
        centre: Vec3::ZERO,
        heading: 0.0,
        spec: Spec {
            size: Vec2::new(20.0, 20.0),
            cell: 1.0,
            base: Base::Height(0.0),
            edits,
            ..Spec::default()
        },
    }
}

/// The height at the grid point of local (x, z), on a 1 m grid from -10.
fn at(g: &Grid, x: i32, z: i32) -> f32 {
    g.at((x + 10) as usize, (z + 10) as usize)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn the_falloff_is_a_smooth_step_over_its_width() {
    assert_eq!(weight(-1.0, 4.0), 1.0);
    assert_eq!(weight(0.0, 4.0), 1.0);
    assert!(near(weight(2.0, 4.0), 0.5));
    assert!(near(weight(1.0, 4.0), 1.0 - (0.25 * 0.25 * (3.0 - 0.5))));
    assert_eq!(weight(4.0, 4.0), 0.0);
    // no falloff: a hard border
    assert_eq!(weight(0.01, 0.0), 0.0);
}

#[test]
fn flatten_rect_raise_and_ramp_reach_their_heights() {
    let g = flat_terrain(vec![Edit::Flatten { at: Vec2::ZERO, radius: 4.0, falloff: 4.0, height: 10.0 }])
        .heights(None)
        .expect("heights");
    assert!(near(at(&g, 0, 0), 10.0) && near(at(&g, 4, 0), 10.0));
    assert!(near(at(&g, 6, 0), 5.0), "{}", at(&g, 6, 0));
    assert!(near(at(&g, 9, 0), 0.0));

    let g = flat_terrain(vec![Edit::Rect {
        min: Vec2::splat(-2.0),
        max: Vec2::splat(2.0),
        falloff: 2.0,
        height: 4.0,
    }])
    .heights(None)
    .expect("heights");
    assert!(near(at(&g, 2, 2), 4.0) && near(at(&g, 3, 0), 2.0) && near(at(&g, 5, 0), 0.0));

    let g = flat_terrain(vec![
        Edit::Flatten { at: Vec2::ZERO, radius: 20.0, falloff: 0.0, height: 1.0 },
        Edit::Raise { at: Vec2::ZERO, radius: 0.0, falloff: 4.0, amount: 2.0 },
    ])
    .heights(None)
    .expect("heights");
    // amount is relative: on top of the flatten
    assert!(near(at(&g, 0, 0), 3.0) && near(at(&g, 2, 0), 2.0) && near(at(&g, 5, 0), 1.0));

    let g = flat_terrain(vec![Edit::Ramp {
        from: Vec2::new(-5.0, 0.0),
        to: Vec2::new(5.0, 0.0),
        width: 2.0,
        falloff: 0.0,
        heights: [0.0, 10.0],
    }])
    .heights(None)
    .expect("heights");
    assert!(near(at(&g, 0, 0), 5.0) && near(at(&g, 5, 1), 10.0) && near(at(&g, -5, 0), 0.0));
    assert!(near(at(&g, 0, 3), 0.0), "beside the ramp nothing changes");
}

#[test]
fn smooth_averages_and_noise_is_the_same_every_time() {
    let g = flat_terrain(vec![
        Edit::Raise { at: Vec2::ZERO, radius: 0.0, falloff: 0.0, amount: 9.0 },
        Edit::Smooth { at: Vec2::ZERO, radius: 0.5, falloff: 0.0, strength: 1.0 },
    ])
    .heights(None)
    .expect("heights");
    assert!(near(at(&g, 0, 0), 1.0), "{}", at(&g, 0, 0));
    assert!(near(at(&g, 1, 0), 0.0));

    let noisy = |seed| {
        flat_terrain(vec![Edit::Noise { amount: 0.3, scale: 8.0, seed }]).heights(None).expect("heights")
    };
    let a = noisy(1);
    assert_eq!(a, noisy(1));
    assert_ne!(a, noisy(2));
    assert!(a.heights.iter().all(|h| h.abs() <= 0.3 + 1e-6));
    assert!(a.heights.iter().any(|h| h.abs() > 0.01));
}

#[test]
fn a_numeric_base_counts_from_the_centre_and_rows_are_the_heights() {
    let mut t = flat_terrain(Vec::new());
    t.centre.y = 7.0;
    t.spec.base = Base::Height(1.5);
    assert!(t.heights(None).expect("heights").heights.iter().all(|h| near(*h, 8.5)));
    t.spec =
        Spec { size: Vec2::new(2.0, 1.0), rows: vec![vec![1.0, 2.0, 3.0], vec![4.0, 5.0, 6.0]], ..t.spec };
    let g = t.heights(None).expect("heights");
    assert_eq!((g.columns, g.rows), (3, 2));
    assert_eq!(g.at(2, 0), 3.0);
    assert_eq!(g.at(0, 1), 4.0);
    assert_eq!(g.as_rows(), t.spec.rows);
}

#[test]
fn over_the_ground_the_border_ends_at_the_grounds_heights() {
    let t = Terrain {
        centre: Vec3::ZERO,
        heading: 0.0,
        spec: Spec {
            size: Vec2::new(20.0, 20.0),
            cell: 1.0,
            base: Base::Ground,
            edge: 4.0,
            edits: vec![Edit::Flatten { at: Vec2::ZERO, radius: 100.0, falloff: 0.0, height: 20.0 }],
            ..Spec::default()
        },
    };
    let (cols, rows) = t.dims();
    let mut ground = Grid::flat(cols, rows, 0.0);
    for j in 0..rows {
        for i in 0..cols {
            ground.heights[j * cols + i] = i as f32 * 0.1;
        }
    }
    let g = t.heights(Some(&ground)).expect("heights");
    for j in 0..rows {
        for i in 0..cols {
            if i == 0 || j == 0 || i == cols - 1 || j == rows - 1 {
                assert!(near(g.at(i, j), ground.at(i, j)), "border {i} {j}");
            }
        }
    }
    assert!(near(g.at(10, 10), 20.0));
    // two metres in: half way (the smooth step at a half)
    let (b, v) = (ground.at(2, 10), g.at(2, 10));
    assert!(near(v, b + (20.0 - b) * 0.5), "{v}");
    // without the ground's heights a terrain on the ground cannot be made
    assert!(t.heights(None).is_err());
}

#[test]
fn values_out_of_range_are_sentences() {
    let mut t = flat_terrain(Vec::new());
    t.spec.cell = 5.0;
    assert_eq!(t.check().unwrap_err(), "the cell takes 0.5 to 4 metres");
    t.spec.cell = 1.0;
    t.spec.size = Vec2::new(600.0, 40.0);
    let e = t.check().unwrap_err();
    assert!(e.contains("601 x 41 points") && e.contains("513"), "{e}");
    t.spec.size = Vec2::new(-1.0, 4.0);
    assert!(t.check().unwrap_err().contains("above 0"));
    t.spec.size = Vec2::new(4.0, 4.0);
    t.spec.rows = vec![vec![0.0; 2]];
    assert_eq!(t.check().unwrap_err(), "rows holds 1 rows of 2 heights; this terrain has 5 rows of 5");
    t.spec.rows.clear();
    t.spec.edits =
        vec![Edit::Ramp { from: Vec2::ZERO, to: Vec2::ZERO, width: 1.0, falloff: 0.0, heights: [0.0, 1.0] }];
    assert_eq!(t.check().unwrap_err(), "a ramp edit its from and to are the same point");
    t.spec.edits = vec![Edit::Smooth { at: Vec2::ZERO, radius: 1.0, falloff: 0.0, strength: 2.0 }];
    assert!(t.check().unwrap_err().contains("strength"));
    t.spec.edits = vec![Edit::Flatten { at: Vec2::ZERO, radius: f32::NAN, falloff: 0.0, height: 0.0 }];
    assert!(t.heights(None).unwrap_err().contains("finite"));
}

#[test]
fn the_mesh_has_one_diagonal_a_skirt_and_faces_up() {
    let mut t = flat_terrain(vec![Edit::Flatten { at: Vec2::ZERO, radius: 3.0, falloff: 2.0, height: 2.0 }]);
    t.spec.size = Vec2::new(4.0, 2.0);
    t.centre = Vec3::new(100.0, 0.0, 50.0);
    t.heading = 0.4;
    let g = t.heights(None).expect("heights");
    let look = Look { texture: "terrain\\grass".into(), ..Look::default() };
    let o = t.mesh(&g, Looks::Table(&look));
    let m = &o.meshes[0];
    let (cols, rows) = t.dims();
    let ring = 2 * (cols - 1) + 2 * (rows - 1);
    assert_eq!(m.points.len(), cols * rows + ring);
    assert_eq!(m.faces.len(), (cols - 1) * (rows - 1) * 2 + ring * 2);
    assert_eq!(o.surfaces.len(), 1);
    assert_eq!(o.surfaces[0].texture, "terrain\\grass");
    assert!(m.smooth_groups.iter().all(|s| *s == 0), "every edge soft");
    for f in m.faces.iter() {
        let p = f.v.map(|v| m.points[v.point as usize]);
        assert!((p[1] - p[0]).cross(p[2] - p[0]).y > 0.0, "a face looks up");
    }
    // the grid's diagonal: every cell's first face holds (i, j + 1) and (i + 1, j)
    let first = m.faces[0].v.map(|v| v.point);
    assert_eq!(first, [0, cols as u32, 1]);
    // the skirt: out by 0.3 m and down by 0.2 m from the border
    let corner = m.points[cols * rows];
    let border = m.points[0];
    assert!(near(border.y - corner.y, SKIRT_DROP));
    let out = t.to_local(corner) - t.to_local(border);
    assert!(near(out.x, -SKIRT_WIDTH) && near(out.y, -SKIRT_WIDTH), "{out:?}");
    // the same text gives the same bytes
    assert_eq!(o.write(), t.mesh(&t.heights(None).expect("heights"), Looks::Table(&look)).write());
}

fn quad(y: f32, lo: Vec2, hi: Vec2, look: u32) -> [GroundTri; 2] {
    let p = |x: f32, z: f32| Vec3::new(x, y, z);
    let uv = |x: f32, z: f32| Vec2::new(x / 10.0, z / 10.0);
    [
        GroundTri {
            p: [p(lo.x, lo.y), p(lo.x, hi.y), p(hi.x, lo.y)],
            uv: [uv(lo.x, lo.y), uv(lo.x, hi.y), uv(hi.x, lo.y)],
            look,
        },
        GroundTri {
            p: [p(hi.x, lo.y), p(lo.x, hi.y), p(hi.x, hi.y)],
            uv: [uv(hi.x, lo.y), uv(lo.x, hi.y), uv(hi.x, hi.y)],
            look,
        },
    ]
}

#[test]
fn the_ground_is_the_surface_under_the_centre_and_faces_wear_its_look() {
    let mut t = flat_terrain(Vec::new());
    t.spec.base = Base::Ground;
    t.spec.edge = 0.0;
    let mut tris: Vec<GroundTri> = quad(2.0, Vec2::splat(-50.0), Vec2::splat(50.0), 0).to_vec();
    // a roof over the corner x, z >= 5 and a crown over the middle: neither is the ground
    tris.extend(quad(5.0, Vec2::splat(5.0), Vec2::splat(50.0), 1));
    tris.extend(quad(7.0, Vec2::splat(-2.0), Vec2::splat(2.0), 1));
    // a slab 1 m over the ground at x <= -5: the ground goes on under it
    tris.extend(quad(3.0, Vec2::new(-50.0, -50.0), Vec2::new(-5.0, 50.0), 0));
    // one facing down is no ground
    let mut under = quad(9.0, Vec2::splat(-50.0), Vec2::splat(50.0), 1);
    for t in &mut under {
        t.p.swap(1, 2);
    }
    tris.extend(under);
    let s = sample_ground(&t, &tris);
    let g = s.grid(&t);
    assert!(near(at(&g, 0, 0), 2.0) && near(at(&g, 8, 8), 2.0) && near(at(&g, 9, -9), 2.0));
    assert!(near(at(&g, -9, 9), 2.0));
    // written at the crown's height, the terrain lies on the crown where there is one and on
    // the ground it comes down to around it
    let mut high = t.clone();
    high.centre.y = 6.9;
    let up = sample_ground(&high, &tris).grid(&high);
    assert!(near(at(&up, 0, 0), 7.0) && near(at(&up, 2, -2), 7.0) && near(at(&up, 8, 8), 2.0));
    // and one whose middle is on the roof takes the roof as far as it goes
    let mut roof = t.clone();
    roof.centre = Vec3::new(8.0, 5.2, 8.0);
    let on = sample_ground(&roof, &tris).grid(&roof);
    assert!(near(at(&on, 0, 0), 5.0) && near(at(&on, 9, 9), 5.0) && near(at(&on, -9, -9), 2.0));
    let looks = [
        Look { texture: "grass".into(), ..Look::default() },
        Look { texture: "roof".into(), ..Look::default() },
    ];
    let faces = s.face_tris(&t);
    let o = t.mesh(
        &t.heights(Some(&g)).expect("heights"),
        Looks::Ground { faces: &faces, tris: &tris, looks: &looks },
    );
    let textures: Vec<&str> = o.surfaces.iter().map(|s| s.texture.as_str()).collect();
    assert_eq!(textures, ["grass"]);
    // the texture coordinates are the ground's, carried to the corners
    let m = &o.meshes[0];
    for f in 0..4 {
        let uv = m.face_uv(f).expect("uv");
        for (k, v) in m.faces[f].v.iter().enumerate() {
            let p = m.points[v.point as usize];
            assert!(near(uv[k].x, p.x / 10.0) && near(uv[k].y, p.z / 10.0));
        }
    }
    // no ground at all: the centre's height, and a face over nothing takes its neighbour's look
    let none = sample_ground(&t, &[]);
    assert!(none.grid(&t).heights.iter().all(|h| *h == 0.0));
    let half: Vec<GroundTri> = quad(1.0, Vec2::new(-50.0, -50.0), Vec2::new(0.0, 50.0), 0).to_vec();
    let s = sample_ground(&t, &half);
    assert!(s.faces.iter().any(Option::is_none));
    assert!(s.face_tris(&t).iter().all(Option::is_some));
    assert!(near(at(&s.grid(&t), 9, 0), 1.0));
}

#[test]
fn the_surface_over_a_point_is_the_meshs() {
    let mut t = flat_terrain(vec![Edit::Ramp {
        from: Vec2::new(-10.0, 0.0),
        to: Vec2::new(10.0, 0.0),
        width: 40.0,
        falloff: 0.0,
        heights: [0.0, 20.0],
    }]);
    t.centre = Vec3::new(5.0, 0.0, 5.0);
    let g = t.heights(None).expect("heights");
    let (y, n) = t.surface_at(&g, Vec3::new(5.5, 0.0, 5.25)).expect("over it");
    assert!(near(y, 10.5), "{y}");
    // a slope of 1 in 1 along +x: the normal leans back along -x
    assert!(near(n.y, std::f32::consts::FRAC_1_SQRT_2) && n.x < 0.0, "{n:?}");
    assert!(t.surface_at(&g, Vec3::new(40.0, 0.0, 0.0)).is_none());
}

#[test]
fn the_region_cuts_where_the_terrain_lies() {
    let mut t = flat_terrain(Vec::new());
    t.centre = Vec3::new(30.0, 0.0, -10.0);
    t.heading = 0.7;
    t.spec.size = Vec2::new(8.0, 4.0);
    let (cols, rows) = t.dims();
    let old = Grid::flat(cols, rows, 0.0);
    let r = t.region(&old);
    assert_eq!((r.columns as usize, r.rows as usize), (cols, rows));
    assert!(near(r.tolerance, TOLERANCE) && near(r.yaw, -0.7));
    let small = |l: Vec2| {
        let c = t.to_world(l, 0.1);
        [c, c + Vec3::new(0.0, 0.0, 0.05), c + Vec3::new(0.05, 0.0, 0.0)]
    };
    // a triangle near a local corner inside the rectangle is cut; one beyond its long side is not
    let inside = small(Vec2::new(3.5, 1.5));
    assert!(clip_triangle(&r, inside[0], inside[1], inside[2]).is_some_and(|o| o.triangles.is_empty()));
    let beyond = small(Vec2::new(0.0, 2.6));
    assert!(clip_triangle(&r, beyond[0], beyond[1], beyond[2]).is_none());
    assert!(
        t.covers(t.to_world(Vec2::new(3.9, -1.9), 0.0)) && !t.covers(t.to_world(Vec2::new(4.1, 0.0), 0.0))
    );
    // and the corners go round the rectangle
    let c = t.corners(0.0);
    assert!(near(c[0].distance(c[1]), 8.0) && near(c[1].distance(c[2]), 4.0));
}
