//! Wavefront OBJ with its material library.
//!
//! OBJ is right-handed with +Y up, like glTF, and takes the same change into the game's space:
//! Z negated, every face's winding reversed, so an outline seen from above keeps its shape and
//! turn. Its v runs up, the game's down: v becomes `1 - v`. A polygon of more than three corners
//! is a fan from its first corner, which is exact for every convex polygon and for any polygon
//! whose first corner sees all the others.

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
                if let (Some(name), Some(file)) = (&current, map_file(&tokens[1..])) {
                    into.insert(name.clone(), Some(dir.join(file)));
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
                // a fan from the first corner, each triangle turned the game's way
                for i in 1..corners.len() - 1 {
                    soup.tris.push(Tri {
                        c: [corners[0].clone(), corners[i + 1].clone(), corners[i].clone()],
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
                let files: Vec<String> = if dir.join(&whole).is_file() {
                    vec![whole]
                } else {
                    args.iter().map(|s| (*s).to_owned()).collect()
                };
                for f in files {
                    let path = dir.join(&f);
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
