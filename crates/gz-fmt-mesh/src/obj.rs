//! Wavefront OBJ with its material library.
//!
//! OBJ is right-handed with +Y up, like glTF, and takes the same change into the game's space:
//! Z negated, every face's winding reversed, so an outline seen from above keeps its shape and
//! turn. Its v runs up, the game's down: v becomes `1 - v`. A polygon of more than three corners
//! is cut into ears along its outline, so a concave face is filled where it is; one without area
//! or that crosses itself is a fan from its first corner.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gz_fmt_object::Object;
use gz_fmt_object::glam::{Vec2, Vec3};

use crate::assemble::{self, Corner, Soup, Surfaces, Tri, Weights};
use crate::math::mirror;
use crate::{Bad, MAX_ELEMENTS, MeshError, MeshSource, PictureData, Result, SourcePicture};

/// Reads an `.obj` and the `.mtl` files it names.
pub fn read_obj(path: &Path) -> Result<MeshSource> {
    let bytes = crate::read_file(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    from_text(&text, dir, &stem)
        .map_err(|message| MeshError::Invalid { file: crate::file_name(path), message })
}

/// The lines of a file, `\` at the end of a line joining the next, with their numbers.
fn lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut acc = String::new();
    let mut first = 0;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("");
        if acc.is_empty() {
            first = i + 1;
        }
        match line.trim_end().strip_suffix('\\') {
            Some(head) => {
                acc.push_str(head);
                acc.push(' ');
            }
            None => {
                acc.push_str(line);
                out.push((first, std::mem::take(&mut acc)));
            }
        }
    }
    if !acc.is_empty() {
        out.push((first, acc));
    }
    out
}

fn numbers<const N: usize>(tokens: &[&str], line: usize, what: &str) -> Result<[f32; N], Bad> {
    let mut out = [0.0f32; N];
    for (k, v) in out.iter_mut().enumerate() {
        let Some(t) = tokens.get(k) else {
            return Err(format!("line {line}: {what} needs {N} numbers"));
        };
        *v = t.parse::<f32>().map_err(|_| format!("line {line}: `{t}` is not a number"))?;
        if !v.is_finite() {
            return Err(format!("line {line}: `{t}` is not a finite number"));
        }
    }
    Ok(out)
}

/// An index of a face corner: 1-based, or counted back from the end when negative.
fn index(token: &str, count: usize, line: usize, what: &str) -> Result<usize, Bad> {
    let i: i64 = token.parse().map_err(|_| format!("line {line}: `{token}` is no {what} index"))?;
    let at = if i > 0 { i - 1 } else { count as i64 + i };
    if i == 0 || at < 0 || at >= count as i64 {
        return Err(format!("line {line}: the face points at {what} {i}, but {count} are defined so far"));
    }
    Ok(at as usize)
}

/// A material library: every material's base colour picture.
fn read_mtl(text: &str, dir: &Path, into: &mut HashMap<String, Option<PathBuf>>) {
    let mut current: Option<String> = None;
    for (_, line) in lines(text) {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        match tokens.first().copied() {
            Some("newmtl") => {
                let name = tokens[1..].join(" ");
                into.insert(name.clone(), None);
                current = Some(name);
            }
            Some(key) if key.eq_ignore_ascii_case("map_Kd") => {
                // a picture named by a rooted path (the artist's own disk) is no picture of it
                if let (Some(name), Some(file)) = (&current, map_file(&tokens[1..]))
                    && let Some(path) = crate::beside(dir, &file)
                {
                    into.insert(name.clone(), Some(path));
                }
            }
            _ => {}
        }
    }
}

/// The file of a `map_*` statement, past its options (`-s 1 1 1`, `-clamp on`, `-imfchan r`, ...).
fn map_file(tokens: &[&str]) -> Option<String> {
    let mut i = 0;
    while i < tokens.len() && tokens[i].starts_with('-') {
        i += 1;
        while i < tokens.len()
            && (tokens[i].parse::<f32>().is_ok()
                || matches!(tokens[i], "on" | "off" | "r" | "g" | "b" | "m" | "l" | "z"))
        {
            i += 1;
        }
    }
    let name = tokens.get(i..)?.join(" ");
    if name.is_empty() { None } else { Some(name) }
}

pub(crate) fn from_text(text: &str, dir: &Path, default_name: &str) -> Result<MeshSource, Bad> {
    let mut notes: Vec<String> = Vec::new();
    let note = |notes: &mut Vec<String>, n: &str| {
        if !notes.iter().any(|x| x == n) {
            notes.push(n.to_owned());
        }
    };
    let mut positions: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut library: HashMap<String, Option<PathBuf>> = HashMap::new();
    let mut surfaces: Surfaces<String> = Surfaces::default();
    let mut pictures: Vec<SourcePicture> = Vec::new();
    let mut soups = vec![Soup { name: default_name.to_owned(), tris: Vec::new() }];
    let mut material = String::new();
    let mut group = 0u32;
    let mut total = 0usize;

    for (line, text) in lines(text) {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let Some((&key, args)) = tokens.split_first() else { continue };
        match key {
            "v" => {
                if args.len() >= 6 {
                    note(&mut notes, "vertex colours were left out");
                }
                positions.push(Vec3::from_array(numbers::<3>(args, line, "a vertex")?));
            }
            "vt" => {
                let u = numbers::<1>(args, line, "a texture coordinate")?[0];
                let v = if args.len() > 1 {
                    numbers::<1>(&args[1..], line, "a texture coordinate")?[0]
                } else {
                    0.0
                };
                uvs.push(Vec2::new(u, 1.0 - v));
            }
            "vn" => normals.push(Vec3::from_array(numbers::<3>(args, line, "a normal")?)),
            "f" => {
                if args.len() < 3 {
                    return Err(format!("line {line}: a face needs three corners"));
                }
                let mut corners = Vec::with_capacity(args.len());
                for a in args {
                    let mut parts = a.split('/');
                    let v = index(parts.next().unwrap_or(""), positions.len(), line, "vertex")?;
                    let t = match parts.next() {
                        Some(t) if !t.is_empty() => Some(index(t, uvs.len(), line, "texture coordinate")?),
                        _ => None,
                    };
                    let n = match parts.next() {
                        Some(n) if !n.is_empty() => Some(index(n, normals.len(), line, "normal")?),
                        _ => None,
                    };
                    corners.push(Corner {
                        pos: mirror(positions[v]),
                        normal: n.map(|n| mirror(normals[n])).filter(|n| *n != Vec3::ZERO),
                        uv: t.map_or(Vec2::ZERO, |t| uvs[t]),
                        weights: Weights::new(),
                    });
                }
                let surface = surfaces.get(material.clone(), || {
                    if material.is_empty() {
                        return ("default".to_owned(), None, false);
                    }
                    let picture = match library.get(&material) {
                        Some(Some(path)) => Some(picture_of(path, &mut pictures)),
                        Some(None) => None,
                        None => {
                            notes.push(format!("material `{material}` is in no material library"));
                            None
                        }
                    };
                    (material.clone(), picture, false)
                });
                total += corners.len() - 2;
                if total > MAX_ELEMENTS {
                    return Err(format!(
                        "the file holds more than {MAX_ELEMENTS} triangles, the most this editor reads"
                    ));
                }
                let soup = soups.last_mut().ok_or_else(|| "no mesh to add to".to_owned())?;
                // ears cut off the outline, each triangle turned the game's way
                let outline: Vec<Vec3> = corners.iter().map(|c| c.pos).collect();
                for [a, b, c] in ears(&outline) {
                    soup.tris.push(Tri {
                        c: [corners[a].clone(), corners[c].clone(), corners[b].clone()],
                        surface,
                        group,
                    });
                }
            }
            "o" | "g" => {
                let name = args.join(" ");
                match soups.last_mut() {
                    Some(s) if s.tris.is_empty() => s.name = name,
                    _ => soups.push(Soup { name, tris: Vec::new() }),
                }
            }
            "usemtl" => material = args.join(" "),
            "mtllib" => {
                let whole = args.join(" ");
                let files: Vec<String> = if crate::beside(dir, &whole).is_some_and(|p| p.is_file()) {
                    vec![whole]
                } else {
                    args.iter().map(|s| (*s).to_owned()).collect()
                };
                for f in files {
                    // a library named by a rooted path is not followed
                    let Some(path) = crate::beside(dir, &f) else { continue };
                    match crate::read_file(&path) {
                        Ok(bytes) => {
                            let mtl_dir = path.parent().unwrap_or(dir).to_owned();
                            read_mtl(&String::from_utf8_lossy(&bytes), &mtl_dir, &mut library);
                        }
                        Err(_) => notes.push(format!("the material library `{f}` could not be read")),
                    }
                }
            }
            "s" => group = args.first().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0),
            "l" | "p" => note(&mut notes, "points and lines were left out"),
            _ => {}
        }
        if positions.len() > MAX_ELEMENTS || uvs.len() > MAX_ELEMENTS || normals.len() > MAX_ELEMENTS {
            return Err(format!("line {line}: the file holds more than {MAX_ELEMENTS} elements of a kind"));
        }
    }
    let meshes = assemble::build_meshes(&soups, &[], &mut notes)?;
    let object = Object { surfaces: surfaces.surfaces, meshes, ..Object::default() };
    Ok(MeshSource { object, materials: surfaces.materials, pictures, notes })
}

fn picture_of(path: &Path, pictures: &mut Vec<SourcePicture>) -> usize {
    if let Some(i) = pictures.iter().position(|p| p.data == PictureData::File(path.to_owned())) {
        return i;
    }
    pictures.push(SourcePicture {
        name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        mime: crate::mime_of(&path.to_string_lossy()).to_owned(),
        data: PictureData::File(path.to_owned()),
    });
    pictures.len() - 1
}

/// A polygon's triangles as indices of its corners, in the polygon's own winding: ears cut off
/// its outline as seen along its normal, so a concave face is filled where it is and nowhere
/// else. A polygon without area, or one that crosses itself, is a fan from its first corner.
fn ears(points: &[Vec3]) -> Vec<[usize; 3]> {
    let n = points.len();
    let fan = || (1..n.saturating_sub(1)).map(|i| [0, i, i + 1]).collect::<Vec<_>>();
    if n <= 3 {
        return fan();
    }
    // Newell's normal, then the plane it is longest on
    let mut normal = [0.0f64; 3];
    for i in 0..n {
        let (a, b) = (points[i], points[(i + 1) % n]);
        let (a, b) = (
            [f64::from(a.x), f64::from(a.y), f64::from(a.z)],
            [f64::from(b.x), f64::from(b.y), f64::from(b.z)],
        );
        normal[0] += (a[1] - b[1]) * (a[2] + b[2]);
        normal[1] += (a[2] - b[2]) * (a[0] + b[0]);
        normal[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    let longest = (0..3).max_by(|i, j| normal[*i].abs().total_cmp(&normal[*j].abs())).unwrap_or(1);
    if normal[longest].abs() < 1e-12 {
        return fan();
    }
    let (u, v) = match longest {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let flat: Vec<[f64; 2]> = points
        .iter()
        .map(|p| {
            let p = [f64::from(p.x), f64::from(p.y), f64::from(p.z)];
            [p[u], p[v]]
        })
        .collect();
    let cross =
        |a: [f64; 2], b: [f64; 2], c: [f64; 2]| (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let sign = normal[longest].signum();
    // a point strictly inside a triangle wound the polygon's way
    let inside = |q: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        cross(a, b, q) * sign > 1e-12 && cross(b, c, q) * sign > 1e-12 && cross(c, a, q) * sign > 1e-12
    };
    let mut left: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    while left.len() > 3 {
        let m = left.len();
        let ear = (0..m).find(|&k| {
            let (i0, i1, i2) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            cross(flat[i0], flat[i1], flat[i2]) * sign > 1e-12
                && !left
                    .iter()
                    .any(|&j| j != i0 && j != i1 && j != i2 && inside(flat[j], flat[i0], flat[i1], flat[i2]))
        });
        let Some(k) = ear else {
            // corners on one line are no ears: drop one and go on; nothing to drop means the
            // outline crosses itself
            let line = (0..m).find(|&k| {
                cross(flat[left[(k + m - 1) % m]], flat[left[k]], flat[left[(k + 1) % m]]).abs() <= 1e-12
            });
            match line {
                Some(k) => {
                    left.remove(k);
                    continue;
                }
                None => return fan(),
            }
        };
        out.push([left[(k + m - 1) % m], left[k], left[(k + 1) % m]]);
        left.remove(k);
    }
    if left.len() == 3 {
        out.push([left[0], left[1], left[2]]);
    }
    out
}

#[cfg(test)]
mod ear_tests {
    use super::*;

    fn area(points: &[Vec3], tris: &[[usize; 3]]) -> f32 {
        tris.iter()
            .map(|t| (points[t[1]] - points[t[0]]).cross(points[t[2]] - points[t[0]]).length() * 0.5)
            .sum()
    }

    #[test]
    fn a_concave_polygon_is_filled_where_it_is() {
        // a U seen from above: a 3 x 3 square with a notch of 1 x 2 cut into its far side
        let u: Vec<Vec3> = [(0, 0), (3, 0), (3, 3), (2, 3), (2, 1), (1, 1), (1, 3), (0, 3)]
            .iter()
            .map(|(x, z)| Vec3::new(*x as f32, 0.0, *z as f32))
            .collect();
        let tris = ears(&u);
        assert_eq!(tris.len(), u.len() - 2);
        assert!((area(&u, &tris) - 7.0).abs() < 1e-4, "{}", area(&u, &tris));
        // no triangle lies in the notch: its middle (1.5, 2) is in none
        let notch = Vec3::new(1.5, 0.0, 2.0);
        for t in &tris {
            let (a, b, c) = (u[t[0]], u[t[1]], u[t[2]]);
            let s = |p: Vec3, q: Vec3| (q.x - p.x) * (notch.z - p.z) - (q.z - p.z) * (notch.x - p.x);
            let (d0, d1, d2) = (s(a, b), s(b, c), s(c, a));
            assert!(!(d0 > 0.0 && d1 > 0.0 && d2 > 0.0) && !(d0 < 0.0 && d1 < 0.0 && d2 < 0.0), "{t:?}");
        }
        // every triangle keeps the polygon's winding
        let n0 = (u[1] - u[0]).cross(u[2] - u[0]);
        assert!(tris.iter().all(|t| (u[t[1]] - u[t[0]]).cross(u[t[2]] - u[t[0]]).dot(n0) > 0.0));
        // the same outline the other way round, and standing upright
        let back: Vec<Vec3> = u.iter().rev().copied().collect();
        assert!((area(&back, &ears(&back)) - 7.0).abs() < 1e-4);
        let wall: Vec<Vec3> = u.iter().map(|p| Vec3::new(p.x, p.z, 5.0)).collect();
        assert!((area(&wall, &ears(&wall)) - 7.0).abs() < 1e-4);
    }

    #[test]
    fn a_convex_polygon_a_triangle_and_a_flat_line_are_still_triangles() {
        let quad = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z];
        assert!((area(&quad, &ears(&quad)) - 1.0).abs() < 1e-5);
        let tri = [Vec3::ZERO, Vec3::X, Vec3::Z];
        assert_eq!(ears(&tri), vec![[0, 1, 2]]);
        // four corners on one line: a fan of nothing worth, and no panic
        let line = [Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0];
        assert_eq!(ears(&line).len(), 2);
        // a point on an edge of a square: still the square's area
        let five = [Vec3::ZERO, Vec3::new(0.5, 0.0, 0.0), Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z];
        assert!((area(&five, &ears(&five)) - 1.0).abs() < 1e-5);
    }
}
