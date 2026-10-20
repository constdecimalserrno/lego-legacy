//! Direct3D's fixed pipeline from a title's vertices to the screen.
//!
//! Vertices come in whatever flexible format a title declared; those not yet
//! transformed go through the world, view and projection, are lit from the
//! material and the lights, cut to the view volume and put on the viewport.
//! What comes out is what [`crate::raster`] draws.

use super::scene::{normalise, Matrix};
use crate::raster::Vertex;

/// `D3DFVF_*` bits, and the layout they give a vertex.
pub mod fvf {
    pub const XYZ: u32 = 0x002;
    pub const XYZRHW: u32 = 0x004;
    pub const POSITION_MASK: u32 = 0x00e;
    pub const NORMAL: u32 = 0x010;
    pub const RESERVED1: u32 = 0x020;
    pub const DIFFUSE: u32 = 0x040;
    pub const SPECULAR: u32 = 0x080;
    pub const TEXCOUNT_MASK: u32 = 0xf00;
}

/// One vertex as a title wrote it, before anything is done to it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub position: [f32; 4],
    pub normal: Option<[f32; 3]>,
    pub diffuse: Option<u32>,
    pub specular: Option<u32>,
    pub uv: [f32; 2],
}

/// How many bytes a vertex of a format takes, or `None` for a format with
/// blend weights, which nothing here reads.
#[must_use]
pub fn vertex_size(format: u32) -> Option<usize> {
    let position = match format & fvf::POSITION_MASK {
        fvf::XYZ => 12,
        fvf::XYZRHW => 16,
        _ => return None,
    };
    let flag = |bit: u32, size: usize| if format & bit != 0 { size } else { 0 };
    let textures = ((format & fvf::TEXCOUNT_MASK) >> 8) as usize;
    Some(
        position
            + flag(fvf::NORMAL, 12)
            + flag(fvf::RESERVED1, 4)
            + flag(fvf::DIFFUSE, 4)
            + flag(fvf::SPECULAR, 4)
            + textures * 8,
    )
}

/// Read one vertex of a format from its bytes, which must be
/// [`vertex_size`] long.
#[must_use]
pub fn read_vertex(format: u32, bytes: &[u8]) -> Input {
    let mut at = 0;
    let mut float = || {
        let v = f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        at += 4;
        v
    };
    let mut out = Input {
        position: [float(), float(), float(), 1.0],
        ..Input::default()
    };
    if format & fvf::POSITION_MASK == fvf::XYZRHW {
        out.position[3] = float();
    }
    if format & fvf::NORMAL != 0 {
        out.normal = Some([float(), float(), float()]);
    }
    if format & fvf::RESERVED1 != 0 {
        float();
    }
    let mut word = |present: bool| present.then(|| float().to_bits());
    out.diffuse = word(format & fvf::DIFFUSE != 0);
    out.specular = word(format & fvf::SPECULAR != 0);
    if format & fvf::TEXCOUNT_MASK != 0 {
        out.uv = [float(), float()];
    }
    out
}

/// A `D3DCOLOR` as red, green, blue and alpha from 0 to 1.
#[must_use]
pub fn colour(value: u32) -> [f32; 4] {
    let [b, g, r, a] = value.to_le_bytes();
    [r, g, b, a].map(|c| f32::from(c) / 255.0)
}

/// A `D3DMATERIAL`'s colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub specular: [f32; 4],
    pub emissive: [f32; 4],
    pub power: f32,
}

impl Material {
    /// From a `D3DMATERIAL`'s bytes: its size, then four colours of four
    /// floats each, then the power.
    #[must_use]
    pub fn read(bytes: &[u8]) -> Option<Self> {
        let float = |i: usize| {
            let at = 4 + i * 4;
            bytes
                .get(at..at + 4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let four = |from: usize| {
            Some([
                float(from)?,
                float(from + 1)?,
                float(from + 2)?,
                float(from + 3)?,
            ])
        };
        Some(Self {
            diffuse: four(0)?,
            ambient: four(4)?,
            specular: four(8)?,
            emissive: four(12)?,
            power: float(16)?,
        })
    }
}

/// A light, where it is in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Light {
    Ambient {
        colour: [f32; 3],
    },
    Directional {
        colour: [f32; 3],
        direction: [f32; 3],
    },
    Point {
        colour: [f32; 3],
        position: [f32; 3],
        range: f32,
        attenuation: [f32; 3],
    },
    Spot {
        colour: [f32; 3],
        position: [f32; 3],
        direction: [f32; 3],
        range: f32,
        attenuation: [f32; 3],
        /// Half the inner cone's angle and half the outer's, as cosines.
        inner: f32,
        outer: f32,
    },
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// A point through a matrix, the whole row: the fourth coordinate included.
#[must_use]
pub fn project(m: &Matrix, p: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0; 4];
    for (j, o) in out.iter_mut().enumerate() {
        *o = p[0] * m[0][j] + p[1] * m[1][j] + p[2] * m[2][j] + p[3] * m[3][j];
    }
    out
}

/// A vertex's colour under the lights, in the world: what the material gives
/// off, what it takes from the ambient light, and what each light puts on it
/// by how squarely it faces it.
///
/// There is no specular term: a draw that asks for highlights on lit
/// vertices stops at a wall before it gets here.
// ponytail: the normal is turned by the world matrix rather than its inverse
// transpose, which is only right while a title's frames scale evenly; and a
// spot light's cone falls off in a straight line, Direct3D's default falloff
// of one, which retained mode has no way to change.
#[must_use]
pub fn light(
    position: [f32; 3],
    normal: [f32; 3],
    material: &Material,
    ambient: [f32; 3],
    lights: &[Light],
) -> [f32; 4] {
    lit(position, normal, material, ambient, lights, None).0
}

/// [`light`]'s colour, and with `highlight` the highlight each light puts on
/// the vertex too: Direct3D's specular term, with the viewer as far away as
/// can be in the direction given and the material's power, which is how
/// retained mode lights a mesh unless a title asks its device for highlights
/// that depend on where the viewer is (#120).
#[must_use]
pub fn lit(
    position: [f32; 3],
    normal: [f32; 3],
    material: &Material,
    ambient: [f32; 3],
    lights: &[Light],
    viewer: Option<[f32; 3]>,
) -> ([f32; 4], [f32; 3]) {
    let normal = normalise(normal).unwrap_or([0.0, 0.0, 0.0]);
    let mut ambient = ambient;
    let mut lit = [0.0f32; 3];
    let mut shine = [0.0f32; 3];
    // The highlight of a light towards `l` with this much of its strength
    // reaching the vertex, on a side the light falls on.
    let mut highlight = |colour: [f32; 3], l: [f32; 3], reaching: f32| {
        let Some(h) = viewer.and_then(|v| normalise(std::array::from_fn(|i| l[i] + v[i]))) else {
            return;
        };
        let along = dot(normal, h);
        if dot(normal, l) <= 0.0 || along <= 0.0 {
            return;
        }
        // The engine's own powers, never the host's.
        let power = crate::cpu::x87::maths::pow2(
            f64::from(material.power) * crate::cpu::x87::maths::log2(f64::from(along)),
        ) as f32;
        for (s, c) in shine.iter_mut().zip(colour) {
            *s += c * power * reaching;
        }
    };
    let mut add = |colour: [f32; 3], strength: f32| {
        for (l, c) in lit.iter_mut().zip(colour) {
            *l += c * strength;
        }
    };
    let toward = |from: [f32; 3]| {
        let v = [
            from[0] - position[0],
            from[1] - position[1],
            from[2] - position[2],
        ];
        (normalise(v), dot(v, v).sqrt())
    };
    let falloff =
        |d: f32, a: [f32; 3]| 1.0 / (a[0] + a[1] * d + a[2] * d * d).max(f32::MIN_POSITIVE);
    for light in lights {
        match *light {
            Light::Ambient { colour } => {
                for (a, c) in ambient.iter_mut().zip(colour) {
                    *a += c;
                }
            }
            Light::Directional { colour, direction } => {
                if let Some(d) = normalise(direction) {
                    let towards = d.map(|c| -c);
                    add(colour, dot(normal, towards).max(0.0));
                    highlight(colour, towards, 1.0);
                }
            }
            Light::Point {
                colour,
                position: at,
                range,
                attenuation,
            } => {
                let (Some(l), d) = toward(at) else { continue };
                if d <= range {
                    add(colour, dot(normal, l).max(0.0) * falloff(d, attenuation));
                    highlight(colour, l, falloff(d, attenuation));
                }
            }
            Light::Spot {
                colour,
                position: at,
                direction,
                range,
                attenuation,
                inner,
                outer,
            } => {
                let (Some(l), d) = toward(at) else { continue };
                let Some(axis) = normalise(direction) else {
                    continue;
                };
                if d > range {
                    continue;
                }
                let rho = dot(l.map(|c| -c), axis);
                let cone = if rho > inner {
                    1.0
                } else if rho <= outer {
                    0.0
                } else {
                    (rho - outer) / (inner - outer).max(f32::MIN_POSITIVE)
                };
                add(
                    colour,
                    dot(normal, l).max(0.0) * falloff(d, attenuation) * cone,
                );
                highlight(colour, l, falloff(d, attenuation) * cone);
            }
        }
    }
    let channel = |i: usize| {
        (material.emissive[i] + ambient[i] * material.ambient[i] + lit[i] * material.diffuse[i])
            .clamp(0.0, 1.0)
    };
    (
        [
            channel(0),
            channel(1),
            channel(2),
            material.diffuse[3].clamp(0.0, 1.0),
        ],
        std::array::from_fn(|i| (shine[i] * material.specular[i]).clamp(0.0, 1.0)),
    )
}

/// A vertex in clip space, with what it carries to the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clipped {
    pub clip: [f32; 4],
    pub diffuse: [f32; 4],
    pub specular: [f32; 3],
    pub uv: [f32; 2],
    /// How much of this vertex's own colour the fog leaves, from the title's
    /// own per-vertex factor. One where there is no fog.
    pub fog: f32,
}

impl Clipped {
    /// A vertex a title put on the screen itself: its position is pixels,
    /// depth and the reciprocal of its distance already.
    #[must_use]
    pub fn on_screen(&self) -> Vertex {
        Vertex {
            x: self.clip[0],
            y: self.clip[1],
            z: self.clip[2],
            rhw: self.clip[3],
            diffuse: self.diffuse,
            specular: self.specular,
            uv: self.uv,
            fog: self.fog,
        }
    }
}

fn lerp(a: &Clipped, b: &Clipped, t: f32) -> Clipped {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    Clipped {
        clip: std::array::from_fn(|i| mix(a.clip[i], b.clip[i])),
        diffuse: std::array::from_fn(|i| mix(a.diffuse[i], b.diffuse[i])),
        specular: std::array::from_fn(|i| mix(a.specular[i], b.specular[i])),
        uv: std::array::from_fn(|i| mix(a.uv[i], b.uv[i])),
        fog: mix(a.fog, b.fog),
    }
}

/// Cut a polygon to the view volume: `-w <= x, y <= w` and `0 <= z <= w`.
#[must_use]
pub fn clip(polygon: &[Clipped]) -> Vec<Clipped> {
    // Most of what a title draws is wholly inside, and cutting nothing is
    // cheaper said than done.
    if polygon.iter().all(|v| {
        let [x, y, z, w] = v.clip;
        -w <= x && x <= w && -w <= y && y <= w && 0.0 <= z && z <= w
    }) {
        return polygon.to_vec();
    }
    // Each plane as the distance of a clip-space point inside it.
    let planes: [fn(&[f32; 4]) -> f32; 6] = [
        |c| c[3] + c[0],
        |c| c[3] - c[0],
        |c| c[3] + c[1],
        |c| c[3] - c[1],
        |c| c[2],
        |c| c[3] - c[2],
    ];
    let mut out = polygon.to_vec();
    for inside in planes {
        if out.is_empty() {
            break;
        }
        let input = std::mem::take(&mut out);
        for (i, current) in input.iter().enumerate() {
            let previous = &input[(i + input.len() - 1) % input.len()];
            let (dc, dp) = (inside(&current.clip), inside(&previous.clip));
            if (dc >= 0.0) != (dp >= 0.0) {
                out.push(lerp(previous, current, dp / (dp - dc)));
            }
            if dc >= 0.0 {
                out.push(*current);
            }
        }
    }
    out
}

/// Where the viewport puts clip space on the surface: its rectangle in pixels,
/// which shows all of clip space over the whole of the depth range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Viewport {
    /// A viewport over a rectangle that shows all of clip space.
    #[must_use]
    pub fn whole(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x: x as f32,
            y: y as f32,
            width: width as f32,
            height: height as f32,
        }
    }

    /// A clipped vertex on the screen.
    #[must_use]
    pub fn place(&self, v: &Clipped) -> Vertex {
        let rhw = 1.0 / v.clip[3];
        let (nx, ny, nz) = (v.clip[0] * rhw, v.clip[1] * rhw, v.clip[2] * rhw);
        Vertex {
            x: self.x + (nx + 1.0) / 2.0 * self.width,
            y: self.y + (1.0 - ny) / 2.0 * self.height,
            // From the near end of the range, so that minus zero lands on zero.
            z: 0.0 + nz,
            rhw,
            diffuse: v.diffuse,
            specular: v.specular,
            uv: v.uv,
            fog: v.fog,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clip, colour, light, read_vertex, vertex_size, Clipped, Light, Material, Viewport,
    };

    #[test]
    fn a_vertex_is_read_in_the_order_its_format_lays_it_out() {
        // XYZ | NORMAL | TEX1, Rock Raiders' meshes: position, normal, uv.
        let format = 0x112;
        assert_eq!(vertex_size(format), Some(32));
        let floats = [1.0f32, 2.0, 3.0, 0.0, 1.0, 0.0, 0.25, 0.75];
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        let v = read_vertex(format, &bytes);
        assert_eq!(v.position, [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(v.normal, Some([0.0, 1.0, 0.0]));
        assert_eq!(v.uv, [0.25, 0.75]);
        assert_eq!(v.diffuse, None);
        // D3DLVERTEX: position, reserved, colour, specular, uv.
        assert_eq!(vertex_size(0x1e2), Some(32));
        assert_eq!(colour(0x80ff_0000), [1.0, 0.0, 0.0, 128.0 / 255.0]);
    }

    #[test]
    fn a_surface_facing_a_light_is_lit_and_one_facing_away_is_not() {
        let material = Material {
            diffuse: [1.0, 0.5, 0.0, 1.0],
            ambient: [1.0; 4],
            specular: [0.0; 4],
            emissive: [0.0; 4],
            power: 0.0,
        };
        let sun = [Light::Directional {
            colour: [1.0, 1.0, 1.0],
            direction: [0.0, -1.0, 0.0],
        }];
        let up = light([0.0; 3], [0.0, 1.0, 0.0], &material, [0.1; 3], &sun);
        assert_eq!(up, [1.0, 0.6, 0.1, 1.0]);
        let down = light([0.0; 3], [0.0, -1.0, 0.0], &material, [0.1; 3], &sun);
        assert_eq!(down, [0.1, 0.1, 0.1, 1.0]);
        // A spot light lights what is in its cone and not what is outside it.
        let spot = [Light::Spot {
            colour: [1.0; 3],
            position: [0.0, 10.0, 0.0],
            direction: [0.0, -1.0, 0.0],
            range: 100.0,
            attenuation: [1.0, 0.0, 0.0],
            inner: 0.99,
            outer: 0.9,
        }];
        let under = light([0.0; 3], [0.0, 1.0, 0.0], &material, [0.0; 3], &spot);
        let aside = light(
            [50.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            &material,
            [0.0; 3],
            &spot,
        );
        assert_eq!((under[0], aside[0]), (1.0, 0.0));
    }

    /// A highlight is brightest where the surface faces halfway between the
    /// light and the viewer, and sharper the greater the material's power
    /// (#120).
    #[test]
    fn a_highlight_is_where_the_surface_faces_between_light_and_viewer() {
        let shiny = |power: f32| Material {
            diffuse: [0.0, 0.0, 0.0, 1.0],
            ambient: [0.0; 4],
            specular: [1.0; 4],
            emissive: [0.0; 4],
            power,
        };
        // The light and the viewer both straight out from the surface.
        let sun = [Light::Directional {
            colour: [1.0; 3],
            direction: [0.0, 0.0, 1.0],
        }];
        let up = [0.0, 0.0, -1.0];
        let (_, square) = super::lit([0.0; 3], up, &shiny(10.0), [0.0; 3], &sun, Some(up));
        assert!(square.iter().all(|s| (s - 1.0).abs() < 1e-6), "{square:?}");
        // Tilted, less of it, and less again for a higher power.
        let tilted = [0.0, 0.5, -1.0];
        let shine =
            |power: f32| super::lit([0.0; 3], tilted, &shiny(power), [0.0; 3], &sun, Some(up)).1[0];
        assert!(shine(10.0) < 1.0 && shine(20.0) < shine(10.0));
        // And none without a viewer to see it.
        assert_eq!(
            super::lit([0.0; 3], up, &shiny(10.0), [0.0; 3], &sun, None).1,
            [0.0; 3]
        );
    }

    #[test]
    fn a_triangle_through_the_near_plane_is_cut_where_it_crosses() {
        let at = |z: f32| Clipped {
            clip: [0.0, 0.0, z, 1.0],
            uv: [z, 0.0],
            ..Clipped::default()
        };
        // One vertex behind the eye's front plane, two in front of it.
        let cut = clip(&[
            at(-1.0),
            at(0.5),
            Clipped {
                clip: [0.5, 0.0, 0.5, 1.0],
                ..at(0.5)
            },
        ]);
        assert_eq!(cut.len(), 4, "a corner cut off makes a quadrilateral");
        assert!(cut.iter().all(|v| v.clip[2] >= 0.0));
        assert!(cut.iter().any(|v| v.clip[2] == 0.0 && v.uv[0] == 0.0));
        // Wholly behind it, nothing is left.
        assert!(clip(&[at(-1.0), at(-0.5), at(-0.2)]).is_empty());
    }

    #[test]
    fn a_viewport_puts_clip_space_on_its_own_rectangle() {
        let viewport = Viewport::whole(16, 13, 151, 151);
        let corner = |x, y| Clipped {
            clip: [x, y, 0.5, 1.0],
            ..Clipped::default()
        };
        let top_left = viewport.place(&corner(-1.0, 1.0));
        let bottom_right = viewport.place(&corner(1.0, -1.0));
        assert_eq!((top_left.x, top_left.y), (16.0, 13.0));
        assert_eq!((bottom_right.x, bottom_right.y), (167.0, 164.0));
        assert_eq!(top_left.z, 0.5);
    }
}
