use std::str::FromStr;

use super::Interpolate;

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Color {
    Rgba(Rgba),
    Hsla(Hsla),
}

impl Color {
    pub fn to_rgba(&self) -> Rgba {
        let c = match self {
            Color::Rgba(c) => *c,
            Color::Hsla(c) => c.to_rgba(),
        };

        c
    }

    pub fn with_alpha(&self, alpha: f32) -> Color {
        let mut color = *self;
        match color {
            Color::Rgba(ref mut c) => c.a = alpha,
            Color::Hsla(ref mut c) => c.a = alpha,
        }

        color
    }

    pub fn alpha(&self) -> f32 {
        match self {
            Color::Rgba(c) => c.a,
            Color::Hsla(c) => c.a,
        }
    }
}

impl FromStr for Color {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let to_b = |c| {
            if c >= b'0' && c <= b'9' {
                (c - b'0') as u8
            } else if c >= b'a' && c <= b'f' {
                ((c - b'a') as u8) + 10
            } else if c >= b'A' && c <= b'F' {
                ((c - b'A') as u8) + 10
            } else {
                0
            }
        };

        let color = if s.starts_with('#') && s.len() == 4 {
            let mut c = s.bytes();
            let _ = c.next();
            let r = c.next().map(to_b).unwrap_or_default() << 4;
            let g = c.next().map(to_b).unwrap_or_default() << 4;
            let b = c.next().map(to_b).unwrap_or_default() << 4;

            Color::Rgba(Rgba {
                r: r as f32 / 0xff as f32,
                g: g as f32 / 0xff as f32,
                b: b as f32 / 0xff as f32,
                a: 1.0,
            })
        } else if s.starts_with('#') && s.len() == 7 {
            let mut c = s.bytes();
            let _ = c.next();
            let r = c.next().map(to_b).unwrap_or_default() << 4;
            let rr = c.next().map(to_b).unwrap_or_default();
            let g = c.next().map(to_b).unwrap_or_default() << 4;
            let gg = c.next().map(to_b).unwrap_or_default();
            let b = c.next().map(to_b).unwrap_or_default() << 4;
            let bb = c.next().map(to_b).unwrap_or_default();

            let r = r | rr;
            let g = g | gg;
            let b = b | bb;

            Color::Rgba(Rgba {
                r: r as f32 / 0xff as f32,
                g: g as f32 / 0xff as f32,
                b: b as f32 / 0xff as f32,
                a: 1.0,
            })
        } else if s.starts_with("rgba(") {
            let (_, n) = s.split_once("rgba(").ok_or("invalid rgba color")?;
            let (n, _) = n.split_once(")").ok_or("invalid rgba color")?;

            let mut parts = n.split(',');
            let r = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let g = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let b = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let a = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Rgba(Rgba {
                r: r / 255.0,
                g: g / 255.0,
                b: b / 255.0,
                a,
            })
        } else if s.starts_with("rgb(") {
            let (_, n) = s.split_once("rgb(").ok_or("invalid rgb color")?;
            let (n, _) = n.split_once(")").ok_or("invalid rgb color")?;

            let mut parts = n.split(',');
            let r = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let g = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let b = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Rgba(Rgba {
                r: r / 255.0,
                g: g / 255.0,
                b: b / 255.0,
                a: 1.0,
            })
        } else if s.starts_with("hsla(") {
            let (_, n) = s.split_once("hsla(").ok_or("invalid hsla color")?;
            let (n, _) = n.split_once(")").ok_or("invalid hsla color")?;

            let mut parts = n.split(',');
            let h = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let s = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let l = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let a = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Hsla(Hsla {
                h: h / 360.0,
                s: s / 100.0,
                l: l / 100.0,
                a,
            })
        } else if s.starts_with("hsl(") {
            let (_, n) = s.split_once("hsl(").ok_or("invalid hsl color")?;
            let (n, _) = n.split_once(")").ok_or("invalid hsl color")?;

            let mut parts = n.split(',');
            let h = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let s = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();
            let l = parts
                .next()
                .map(|p| str_to_f32(p.bytes()))
                .unwrap_or_default();

            Color::Hsla(Hsla {
                h: h / 360.0,
                s: s / 100.0,
                l: l / 100.0,
                a: 1.0,
            })
        } else {
            return Err("invalid color");
        };

        Ok(color)
    }
}

fn str_to_f32<I: IntoIterator<Item = u8>>(iter: I) -> f32 {
    let mut whole = 0.0;
    let mut frac = None;
    let mut frac_scale = 1.0;

    for b in iter {
        let n = if b >= b'0' && b <= b'9' {
            (b - b'0') as f32
        } else if b == b'.' {
            frac = Some(0.0);
            continue;
        } else {
            continue;
        };

        if let Some(frac) = frac.as_mut() {
            frac_scale *= 0.1;
            *frac += frac_scale * n;
        } else {
            whole = (whole * 10.0) + n;
        }
    }

    whole + frac.unwrap_or(0.0)
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let s = String::deserialize(deserializer)?;
        Color::from_str(&s).map_err(|e| D::Error::custom(e))
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::Rgba(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        })
    }
}

impl Interpolate for Color {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        match (self, other) {
            (Color::Rgba(last), Color::Rgba(next)) => Color::Rgba(last.interpolate(factor, next)),
            (Color::Hsla(last), Color::Hsla(next)) => Color::Hsla(last.interpolate(factor, next)),
            (Color::Rgba(last), Color::Hsla(next)) => {
                Color::Rgba(last.interpolate(factor, next.to_rgba()))
            }
            (Color::Hsla(last), Color::Rgba(next)) => {
                Color::Rgba(last.to_rgba().interpolate(factor, next))
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Interpolate for Rgba {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        Rgba {
            r: self.r.interpolate(factor, other.r),
            g: self.g.interpolate(factor, other.g),
            b: self.b.interpolate(factor, other.b),
            a: self.a.interpolate(factor, other.a),
        }
    }
}
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Hsla {
    h: f32,
    s: f32,
    l: f32,
    a: f32,
}

impl Hsla {
    fn to_rgba(&self) -> Rgba {
        let h = self.h.min(1.0).max(0.0) * 360.0;
        let s = self.s.min(1.0).max(0.0);
        let l = self.l.min(1.0).max(0.0);

        let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
        let h_prime = h / 60.0;
        let x = c * (1.0 - ((h_prime % 2.0) - 1.0).abs());
        let (r, g, b) = match h_prime {
            v if 0.0 <= v && v <= 1.0 => (c, x, 0.0),
            v if 1.0 <= v && v <= 2.0 => (x, c, 0.0),
            v if 2.0 <= v && v <= 3.0 => (0.0, c, x),
            v if 3.0 <= v && v <= 4.0 => (0.0, x, c),
            v if 4.0 <= v && v <= 5.0 => (x, 0.0, c),
            v if 5.0 <= v && v <= 6.0 => (c, 0.0, x),
            _ => (0.0, 0.0, 0.0),
        };

        let m = l - (c / 2.0);

        let r = r + m;
        let g = g + m;
        let b = b + m;

        let a = self.a;

        Rgba { r, g, b, a }
    }
}

impl Interpolate for Hsla {
    fn interpolate(&self, factor: f32, other: Self) -> Self {
        Hsla {
            h: self.h.interpolate(factor, other.h),
            s: self.s.interpolate(factor, other.s),
            l: self.l.interpolate(factor, other.l),
            a: self.a.interpolate(factor, other.a),
        }
    }
}
