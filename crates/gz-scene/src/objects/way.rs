//! Way paths, sectors and portals (`WayPoint.cpp`, `sector.cpp`, `portal.cpp`).

use gz_base::SmolStr;
use gz_chunk::{Reader, Writer};

use crate::SceneError;
use crate::ltx::{IniReader, IniWriter};
use crate::math::{Aabb, Vec3};
use crate::objects::simple::Body;

pub const WAYOBJECT_VERSION: u32 = 0x13;
/// `WAYPOINT_SIZE` and its half.
pub const WAYPOINT_SIZE: f32 = 1.5;
pub const WAYPOINT_RADIUS: f32 = WAYPOINT_SIZE * 0.5;

/// `CWayPoint`.
#[derive(Clone, Debug, PartialEq)]
pub struct WayPoint {
    pub name: SmolStr,
    pub position: Vec3,
    pub flags: u32,
    pub selected: bool,
    /// The points it links to, by index, with the link's probability.
    pub links: Vec<(u32, f32)>,
}

impl WayPoint {
    #[must_use]
    pub fn new(name: &str, position: Vec3) -> Self {
        Self { name: SmolStr::new(name), position, flags: 0, selected: false, links: Vec::new() }
    }

    /// `GetBox` of one point.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - WAYPOINT_RADIUS, p.y, p.z - WAYPOINT_RADIUS),
            Vec3::new(p.x + WAYPOINT_RADIUS, p.y + WAYPOINT_SIZE, p.z + WAYPOINT_RADIUS),
        )
    }
}

/// `CWayObject`.
#[derive(Clone, Debug, PartialEq)]
pub struct WayData {
    /// `wtPatrolPath` 0.
    pub kind: u32,
    pub points: Vec<WayPoint>,
}

impl Default for WayData {
    /// `Construct`: one point named `wp`.
    fn default() -> Self {
        let mut p = WayPoint::new("wp", Vec3::ZERO);
        p.selected = true;
        Self { kind: 0, points: vec![p] }
    }
}

impl WayData {
    /// `CreateLink`: a point never links to itself.
    pub fn create_link(&mut self, from: usize, to: usize, prob: f32) {
        if from != to
            && to < self.points.len()
            && let Some(p) = self.points.get_mut(from)
        {
            p.links.push((u32::try_from(to).unwrap_or(u32::MAX), prob));
        }
    }

    /// `AppendLink`: only once.
    pub fn append_link(&mut self, from: usize, to: usize, prob: f32) -> bool {
        let to32 = u32::try_from(to).unwrap_or(u32::MAX);
        if self.points.get(from).is_some_and(|p| p.links.iter().any(|(t, _)| *t == to32)) {
            return false;
        }
        self.create_link(from, to, prob);
        true
    }

    /// `AppendWayPoint`'s name: `wp`, `wp_00`, `wp_01` ...
    #[must_use]
    pub fn next_point_name(&self) -> String {
        for i in 0u32.. {
            let n = if i == 0 { "wp".to_owned() } else { format!("wp_{:02}", i - 1) };
            if !self.points.iter().any(|p| p.name == n) {
                return n;
            }
        }
        String::new()
    }

    /// `BuildPath`: the points, each linked to the next, the last back to the first when
    /// `looped` and there are more than two.
    #[must_use]
    pub fn build(positions: &[Vec3], looped: bool) -> Self {
        let mut w = Self { kind: 0, points: Vec::with_capacity(positions.len()) };
        for p in positions {
            let name = w.next_point_name();
            for q in &mut w.points {
                q.selected = false;
            }
            let mut wp = WayPoint::new(&name, *p);
            wp.selected = true;
            w.points.push(wp);
        }
        for i in 0..w.points.len().saturating_sub(1) {
            w.append_link(i, i + 1, 1.0);
        }
        if looped && w.points.len() > 2 {
            let last = w.points.len() - 1;
            w.append_link(last, 0, 1.0);
        }
        w
    }

    /// `GetBox`.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::INVALID;
        for p in &self.points {
            b.modify(p.position);
        }
        b.max.x += WAYPOINT_RADIUS;
        b.max.z += WAYPOINT_RADIUS;
        b.max.y += WAYPOINT_SIZE;
        b.min.x -= WAYPOINT_RADIUS;
        b.min.z -= WAYPOINT_RADIUS;
        b
    }

    /// `MoveTo` (not in point mode): every point moves with the first.
    pub fn move_to(&mut self, pos: Vec3) {
        if let Some(first) = self.points.first().map(|p| p.position) {
            let diff = pos - first;
            for p in &mut self.points {
                p.position += diff;
            }
        }
    }

    /// `RayPick`: the nearest point's box.
    #[must_use]
    pub fn ray_pick(&self, start: Vec3, dir: Vec3, distance: f32) -> Option<f32> {
        let mut best = distance;
        for p in &self.points {
            if let Some((t, false)) = crate::math::ray_box(start, dir, &p.bounds())
                && t < best
            {
                best = t;
            }
        }
        (best < distance).then_some(best)
    }

    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != WAYOBJECT_VERSION {
            return Ok(Body::Refused("CWayPoint: Unsupported version.".into()));
        }
        let n = r.r_u32(s, "wp_count")?;
        let mut w = Self { kind: 0, points: Vec::new() };
        for i in 0..n.min(1 << 16) {
            w.points.push(WayPoint {
                position: r.r_fvector3(s, &format!("wp_{i}_pos"))?,
                flags: r.r_u32(s, &format!("wp_{i}_flags"))?,
                selected: r.r_bool(s, &format!("wp_{i}_selected"))?,
                name: r.r_string(s, &format!("wp_{i}_name"))?.into(),
                links: Vec::new(),
            });
        }
        // oracle: the links come back in the file's key order (`link_wp_0_10` before
        // `link_wp_0_2`), which is the order they are created in again
        for (key, _) in r.lines(s) {
            let Some(rest) = key.strip_prefix("link_wp_") else { continue };
            let mut it = rest.split('_');
            let (Some(a), Some(b)) = (it.next(), it.next()) else {
                return Err(SceneError::load(format!("bad waypoint link record format: [{s}] {key}")));
            };
            let (Ok(from), Ok(_)) = (a.parse::<usize>(), b.parse::<usize>()) else {
                return Err(SceneError::load(format!("bad waypoint link record format: [{s}] {key}")));
            };
            let v = r.r_fvector2(s, &key)?;
            w.create_link(from, v[0].floor().max(0.0) as usize, v[1]);
        }
        w.kind = r.r_u32(s, "type")?;
        Ok(Body::Read(w))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", WAYOBJECT_VERSION);
        w.w_u32(s, "wp_count", u32::try_from(self.points.len()).unwrap_or(0));
        for (i, p) in self.points.iter().enumerate() {
            w.w_fvector3(s, &format!("wp_{i}_pos"), p.position);
            w.w_u32(s, &format!("wp_{i}_flags"), p.flags);
            w.w_bool(s, &format!("wp_{i}_selected"), p.selected);
            w.w_string(s, &format!("wp_{i}_name"), &p.name);
        }
        for (i, p) in self.points.iter().enumerate() {
            for (k, (to, prob)) in p.links.iter().enumerate() {
                w.w_fvector2(s, &format!("link_wp_{i:4}_{k:4}"), [*to as f32, *prob]);
            }
        }
        w.w_u32(s, "type", self.kind);
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = match r.open_chunk(1)? {
            Some(c) => c.reader().r_u16()?,
            None => return Ok(Body::Refused("CWayPoint: no version".into())),
        };
        if u32::from(version) != WAYOBJECT_VERSION {
            return Ok(Body::Refused("CWayPoint: Unsupported version.".into()));
        }
        let (Some(pc), Some(lc), Some(tc)) = (r.open_chunk(2)?, r.open_chunk(3)?, r.open_chunk(4)?) else {
            return Ok(Body::Refused("CWayPoint: a chunk is missing".into()));
        };
        let mut q = pc.reader();
        let n = q.r_u16()? as usize;
        let mut w = Self { kind: 0, points: Vec::with_capacity(n.min(q.len() / 19 + 1)) };
        for _ in 0..n {
            let position = q.r_vec3()?;
            let flags = q.r_u32()?;
            let selected = q.r_u16()? != 0;
            let name = q.r_stringz()?;
            w.points.push(WayPoint { name: name.into(), position, flags, selected, links: Vec::new() });
        }
        let mut q = lc.reader();
        let ln = q.r_u16()?;
        for _ in 0..ln {
            let from = q.r_u16()? as usize;
            let to = q.r_u16()? as usize;
            let prob = q.r_f32()?;
            if from >= w.points.len() || to >= w.points.len() {
                return Err(SceneError::load("CWayPoint: a link names a point that does not exist"));
            }
            w.create_link(from, to, prob);
        }
        w.kind = tc.reader().r_u32()?;
        Ok(Body::Read(w))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(1, |w| w.w_u16(WAYOBJECT_VERSION as u16));
        w.chunk(2, |w| {
            w.w_u16(u16::try_from(self.points.len()).unwrap_or(u16::MAX));
            for p in &self.points {
                w.w_vec3(p.position);
                w.w_u32(p.flags);
                w.w_u16(u16::from(p.selected));
                w.w_stringz(&p.name);
            }
        });
        let links: usize = self.points.iter().map(|p| p.links.len()).sum();
        w.chunk(3, |w| {
            w.w_u16(u16::try_from(links).unwrap_or(u16::MAX));
            for (i, p) in self.points.iter().enumerate() {
                for (to, prob) in &p.links {
                    w.w_u16(u16::try_from(i).unwrap_or(u16::MAX));
                    w.w_u16(u16::try_from(*to).unwrap_or(u16::MAX));
                    w.w_f32(*prob);
                }
            }
        });
        w.chunk(4, |w| w.w_u32(self.kind));
    }
}

pub const SECTOR_VERSION: u32 = 0x12;

/// `Fcolor::set(u32)`.
#[must_use]
pub fn fcolor_from(argb: u32) -> [f32; 4] {
    let f = |s: u32| ((argb >> s) & 0xff) as f32 / 255.0;
    [f(16), f(8), f(0), f(24)]
}

/// `Fcolor::get()`.
#[must_use]
pub fn fcolor_get(c: [f32; 4]) -> u32 {
    let q = |v: f32| ((v * 255.0).floor() as i32).clamp(0, 255) as u32;
    (q(c[3]) << 24) | (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2])
}

/// `CSector`.
#[derive(Clone, Debug, PartialEq)]
pub struct SectorData {
    pub color: [f32; 4],
    pub default: bool,
    /// The scene object and mesh of every item.
    pub items: Vec<(SmolStr, SmolStr)>,
    /// `m_map_idx`: 255 for the default map.
    pub map_idx: u8,
}

impl Default for SectorData {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0, 0.0], default: false, items: Vec::new(), map_idx: u8::MAX }
    }
}

impl SectorData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        let version = r.r_u32(s, "version")?;
        if version < 0x11 {
            return Ok(Body::Refused("CSector: Unsupported version.".into()));
        }
        let mut d = Self {
            color: fcolor_from(r.r_color(s, "sector_color")?),
            default: r.r_bool(s, "default")?,
            ..Self::default()
        };
        let n = r.r_u32(s, "items_count")?;
        for i in 0..n.min(1 << 16) {
            d.items.push((
                r.r_string(s, &format!("item_object_name_{i:04}"))?.into(),
                r.r_string(s, &format!("item_mesh_name_{i:04}"))?.into(),
            ));
        }
        if version >= 0x12 {
            d.map_idx = r.r_u8(s, "change_map_to_idx")?;
        }
        Ok(Body::Read(d))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", SECTOR_VERSION);
        w.w_color(s, "sector_color", fcolor_get(self.color));
        w.w_bool(s, "default", self.default);
        w.w_u32(s, "items_count", u32::try_from(self.items.len()).unwrap_or(0));
        for (i, (o, m)) in self.items.iter().enumerate() {
            w.w_string(s, &format!("item_object_name_{i:04}"), o);
            w.w_string(s, &format!("item_mesh_name_{i:04}"), m);
        }
        w.w_u8(s, "change_map_to_idx", self.map_idx);
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = match r.open_chunk(0xF010)? {
            Some(c) => c.reader().r_u16()?,
            None => 0,
        };
        if u32::from(version) != SECTOR_VERSION {
            return Ok(Body::Refused("CSector: Unsupported version.".into()));
        }
        let (Some(cc), Some(pc)) = (r.open_chunk(0xF020)?, r.open_chunk(0xF025)?) else {
            return Ok(Body::Refused("CSector: a chunk is missing".into()));
        };
        let mut q = cc.reader();
        let mut d = Self { color: [q.r_f32()?, q.r_f32()?, q.r_f32()?, q.r_f32()?], ..Self::default() };
        d.default = pc.reader().r_u8()? != 0;
        if let Some(items) = r.open_chunk(0xF030)? {
            for (_, _, body) in items.reader().chunks() {
                if let Some(one) = Reader::new(body).open_chunk(0xF031)? {
                    let mut q = one.reader();
                    d.items.push((q.r_stringz()?.into(), q.r_stringz()?.into()));
                }
            }
        }
        if let Some(c) = r.open_chunk(0xF032)? {
            d.map_idx = c.reader().r_u8()?;
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0xF010, |w| w.w_u16(SECTOR_VERSION as u16));
        w.chunk(0xF020, |w| {
            for c in self.color {
                w.w_f32(c);
            }
        });
        w.chunk(0xF025, |w| w.w_u8(u8::from(self.default)));
        w.chunk(0xF030, |w| {
            for (i, (o, m)) in self.items.iter().enumerate() {
                w.chunk(u32::try_from(i).unwrap_or(u32::MAX), |w| {
                    w.chunk(0xF031, |w| {
                        w.w_stringz(o);
                        w.w_stringz(m);
                    });
                });
            }
        });
        w.chunk(0xF032, |w| w.w_u8(self.map_idx));
    }
}

pub const PORTAL_VERSION: u32 = 0x10;

/// `CPortal`: a polygon between two sectors, in world coordinates.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PortalData {
    pub front: SmolStr,
    pub back: SmolStr,
    pub vertices: Vec<Vec3>,
}

impl PortalData {
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb> {
        (!self.vertices.is_empty()).then(|| Aabb::from_points(self.vertices.iter()))
    }

    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != PORTAL_VERSION {
            return Ok(Body::Refused("CPortal: Unsupported version.".into()));
        }
        let mut d = Self {
            front: r.r_string(s, "sector_front")?.into(),
            back: r.r_string(s, "sector_back")?.into(),
            vertices: Vec::new(),
        };
        let n = r.r_u32(s, "vert_count")?;
        for i in 0..n.min(1 << 16) {
            d.vertices.push(r.r_fvector3(s, &format!("vertex_{i:04}"))?);
        }
        Ok(Body::Read(d))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", PORTAL_VERSION);
        w.w_string(s, "sector_front", &self.front);
        w.w_string(s, "sector_back", &self.back);
        w.w_u32(s, "vert_count", u32::try_from(self.vertices.len()).unwrap_or(0));
        for (i, v) in self.vertices.iter().enumerate() {
            w.w_fvector3(s, &format!("vertex_{i:04}"), *v);
        }
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = match r.open_chunk(0xFA10)? {
            Some(c) => c.reader().r_u16()?,
            None => 0,
        };
        if u32::from(version) != PORTAL_VERSION {
            return Ok(Body::Refused("CPortal: Unsupported version.".into()));
        }
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0xFA30)? {
            d.front = c.reader().r_stringz()?.into();
        }
        if let Some(c) = r.open_chunk(0xFA40)? {
            d.back = c.reader().r_stringz()?.into();
        }
        let Some(c) = r.open_chunk(0xFA50)? else {
            return Ok(Body::Refused("CPortal: no vertices".into()));
        };
        let mut q = c.reader();
        let n = q.r_u16()? as usize;
        if n * 12 > q.len() {
            return Err(SceneError::load("CPortal: the vertices are cut short"));
        }
        for _ in 0..n {
            d.vertices.push(q.r_vec3()?);
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0xFA10, |w| w.w_u16(PORTAL_VERSION as u16));
        if !self.front.is_empty() {
            w.chunk(0xFA30, |w| w.w_stringz(&self.front));
        }
        if !self.back.is_empty() {
            w.chunk(0xFA40, |w| w.w_stringz(&self.back));
        }
        w.chunk(0xFA50, |w| {
            w.w_u16(u16::try_from(self.vertices.len()).unwrap_or(u16::MAX));
            for v in &self.vertices {
                w.w_vec3(*v);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_path_loops_and_round_trips() {
        let w = WayData::build(&[Vec3::ZERO, Vec3::X, Vec3::Z], true);
        assert_eq!(w.points.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["wp", "wp_00", "wp_01"]);
        assert_eq!(w.points[2].links, vec![(0, 1.0)]);
        let mut iw = IniWriter::new();
        w.write_ltx(&mut iw, "o");
        let r = IniReader::parse(&iw.save(), "t").unwrap();
        let Body::Read(back) = WayData::read_ltx(&r, "o").unwrap() else { panic!() };
        assert_eq!(back, w);
        let mut sw = Writer::new();
        w.write_stream(&mut sw);
        let bytes = sw.into_bytes();
        let Body::Read(back) = WayData::read_stream(&Reader::new(&bytes)).unwrap() else { panic!() };
        assert_eq!(back, w);
    }

    #[test]
    fn sectors_and_portals_round_trip() {
        let s =
            SectorData { items: vec![("wall".into(), "mesh0".into())], map_idx: 2, ..SectorData::default() };
        let mut sw = Writer::new();
        s.write_stream(&mut sw);
        let bytes = sw.into_bytes();
        let Body::Read(back) = SectorData::read_stream(&Reader::new(&bytes)).unwrap() else { panic!() };
        assert_eq!(back, s);
        let p =
            PortalData { front: "a".into(), back: "b".into(), vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y] };
        let mut iw = IniWriter::new();
        p.write_ltx(&mut iw, "o");
        let r = IniReader::parse(&iw.save(), "t").unwrap();
        let Body::Read(back) = PortalData::read_ltx(&r, "o").unwrap() else { panic!() };
        assert_eq!(back, p);
    }
}
