/// A colour: 8 bits a channel, straight alpha, in the gamma space the game lights in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const WHITE: Color = Color([255, 255, 255, 255]);
    pub const BLACK: Color = Color([0, 0, 0, 255]);
    pub const TRANSPARENT: Color = Color([0, 0, 0, 0]);
    /// The flat grey of a placeholder mesh.
    pub const PLACEHOLDER: Color = Color([150, 150, 150, 255]);

    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self([r, g, b, a])
    }

    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b, 255])
    }

    /// The engine's `D3DCOLOR`: `0xAARRGGBB`.
    #[must_use]
    pub const fn from_argb(v: u32) -> Self {
        Self([(v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8])
    }

    /// The same colour with another alpha.
    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self([self.0[0], self.0[1], self.0[2], a])
    }

    #[must_use]
    pub fn to_f32(self) -> [f32; 4] {
        self.0.map(|c| f32::from(c) / 255.0)
    }

    /// The colour with its channels multiplied by `k` (alpha kept): `Fcolor::mul_rgb`.
    #[must_use]
    pub fn scaled(self, k: f32) -> Self {
        let s = |c: u8| (f32::from(c) * k).round().clamp(0.0, 255.0) as u8;
        Self([s(self.0[0]), s(self.0[1]), s(self.0[2]), self.0[3]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_colours_unpack() {
        assert_eq!(Color::from_argb(0xff90_8070), Color([0x90, 0x80, 0x70, 0xff]));
        assert_eq!(Color::from_argb(0x0055_5555).0[3], 0);
        assert_eq!(Color::rgb(200, 100, 0).scaled(0.5), Color([100, 50, 0, 255]));
    }
}
