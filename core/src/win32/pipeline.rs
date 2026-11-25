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

impl crate::snap::Snap for Light {
    fn snap(&mut self, v: &mut dyn crate::snap::Visitor) {
        let mut tag = match self {
            Self::Ambient { .. } => 0u8,
            Self::Directional { .. } => 1u8,
            Self::Point { .. } => 2u8,
            Self::Spot { .. } => 3u8,
        };
        v.u8(&mut tag);
        if v.is_reading() {
            match tag {
                0 => {
                    let mut colour = [0.0f32; 3];
                    for c in &mut colour {
                        v.f32(c);
                    }
                    *self = Self::Ambient { colour };
                }
                1 => {
                    let (mut colour, mut direction) = ([0.0f32; 3], [0.0f32; 3]);
                    for c in &mut colour {
                        v.f32(c);
                    }
                    for d in &mut direction {
                        v.f32(d);
                    }
                    *self = Self::Directional { colour, direction };
                }
                2 => {
                    let (mut colour, mut position, mut range, mut attenuation) =
                        ([0.0f32; 3], [0.0f32; 3], 0.0f32, [0.0f32; 3]);
                    for c in &mut colour {
                        v.f32(c);
                    }
                    for p in &mut position {
                        v.f32(p);
                    }
                    v.f32(&mut range);
                    for a in &mut attenuation {
                        v.f32(a);
                    }
                    *self = Self::Point {
                        colour,
                        position,
                        range,
                        attenuation,
                    };
                }
                3 => {
                    let (
                        mut colour,
                        mut position,
                        mut direction,
                        mut range,
                        mut attenuation,
                        mut inner,
                        mut outer,
                    ) = (
                        [0.0f32; 3],
                        [0.0f32; 3],
                        [0.0f32; 3],
                        0.0f32,
                        [0.0f32; 3],
                        0.0f32,
                        0.0f32,
                    );
                    for c in &mut colour {
                        v.f32(c);
                    }
                    for p in &mut position {
                        v.f32(p);
                    }
                    for d in &mut direction {
                        v.f32(d);
                    }
                    v.f32(&mut range);
                    for a in &mut attenuation {
                        v.f32(a);
                    }
                    v.f32(&mut inner);
                    v.f32(&mut outer);
                    *self = Self::Spot {
                        colour,
                        position,
                        direction,
                        range,
                        attenuation,
                        inner,
                        outer,
                    };
                }
                _ => v.mark_corrupt("pipeline::Light tag"),
            }
        } else {
            match self {
                Self::Ambient { colour } => {
                    for c in colour {
                        v.f32(c);
                    }
                }
                Self::Directional { colour, direction } => {
                    for c in colour {
                        v.f32(c);
                    }
                    for d in direction {
                        v.f32(d);
                    }
                }
                Self::Point {
                    colour,
                    position,
                    range,
                    attenuation,
                } => {
                    for c in colour {
                        v.f32(c);
                    }
                    for p in position {
                        v.f32(p);
                    }
                    v.f32(range);
                    for a in attenuation {
                        v.f32(a);
                    }
                }
                Self::Spot {
                    colour,
                    position,
                    direction,
                    range,
                    attenuation,
                    inner,
                    outer,
                } => {
                    for c in colour {
                        v.f32(c);
                    }
                    for p in position {
                        v.f32(p);
                    }
                    for d in direction {
                        v.f32(d);
                    }
                    v.f32(range);
                    for a in attenuation {
                        v.f32(a);
                    }
                    v.f32(inner);
                    v.f32(outer);
                }
            }
        }
    }
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

/// One non-ambient light with its per-pass constants precomputed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PrecomputedLight {
    Directional {
        colour: [f32; 3],
        /// Unit vector pointing toward the light (`-normalise(direction)`).
        towards: [f32; 3],
        /// Precomputed `normalise(towards + viewer)` when a viewer is present.
        halfway: Option<[f32; 3]>,
        safe_zero: bool,
    },
    Point {
        colour: [f32; 3],
        position: [f32; 3],
        range: f32,
        attenuation: [f32; 3],
        safe_zero: bool,
    },
    Spot {
        colour: [f32; 3],
        position: [f32; 3],
        /// Unit light axis (`normalise(direction)`).
        axis: [f32; 3],
        range: f32,
        attenuation: [f32; 3],
        inner: f32,
        outer: f32,
        cone_denom: f32,
        safe_zero: bool,
    },
}

/// Lights precomputed once per mesh or pass: ambient lights folded in order,
/// directional vectors and halfway vectors normalised once, spot axes
/// normalised once, and whether any positional (point/spot) light is present.
#[derive(Clone, Debug, Default)]
pub struct PreparedLights {
    pub ambient: [f32; 3],
    pub lights: Vec<PrecomputedLight>,
    pub viewer: Option<[f32; 3]>,
    pub has_positional: bool,
    viewer_finite: bool,
    all_colours_safe: bool,
}

#[inline]
fn is_safe_colour(c: [f32; 3]) -> bool {
    c[0].is_finite()
        && c[1].is_finite()
        && c[2].is_finite()
        && c[0] >= 0.0
        && c[1] >= 0.0
        && c[2] >= 0.0
}

impl PreparedLights {
    #[must_use]
    pub fn new(ambient: [f32; 3], lights: &[Light], viewer: Option<[f32; 3]>) -> Self {
        let mut prepared = Self::default();
        prepared.prepare(ambient, lights, viewer);
        prepared
    }

    pub fn prepare(&mut self, mut ambient: [f32; 3], lights: &[Light], viewer: Option<[f32; 3]>) {
        self.lights.clear();
        self.viewer = viewer;
        self.viewer_finite =
            viewer.is_some_and(|v| v[0].is_finite() && v[1].is_finite() && v[2].is_finite());
        self.has_positional = false;
        self.all_colours_safe = true;
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
                        let halfway = viewer
                            .and_then(|v| normalise(std::array::from_fn(|i| towards[i] + v[i])));
                        let safe_zero = is_safe_colour(colour);
                        self.all_colours_safe &= safe_zero;
                        self.lights.push(PrecomputedLight::Directional {
                            colour,
                            towards,
                            halfway,
                            safe_zero,
                        });
                    }
                }
                Light::Point {
                    colour,
                    position,
                    range,
                    attenuation,
                } => {
                    let safe_zero = is_safe_colour(colour);
                    self.all_colours_safe &= safe_zero;
                    self.has_positional = true;
                    self.lights.push(PrecomputedLight::Point {
                        colour,
                        position,
                        range,
                        attenuation,
                        safe_zero,
                    });
                }
                Light::Spot {
                    colour,
                    position,
                    direction,
                    range,
                    attenuation,
                    inner,
                    outer,
                } => {
                    if let Some(axis) = normalise(direction) {
                        let safe_zero = is_safe_colour(colour);
                        self.all_colours_safe &= safe_zero;
                        self.has_positional = true;
                        self.lights.push(PrecomputedLight::Spot {
                            colour,
                            position,
                            axis,
                            range,
                            attenuation,
                            inner,
                            outer,
                            cone_denom: (inner - outer).max(f32::MIN_POSITIVE),
                            safe_zero,
                        });
                    }
                }
            }
        }
        self.ambient = ambient;
    }

    #[must_use]
    #[inline]
    pub fn has_positional(&self) -> bool {
        self.has_positional
    }
}

#[inline(always)]
fn add_highlight(
    shine: &mut [f32; 3],
    colour: [f32; 3],
    normal: [f32; 3],
    halfway: [f32; 3],
    material_power: f32,
    reaching: f32,
    safe_zero: bool,
) {
    let along = dot(normal, halfway);
    if along <= 0.0 {
        return;
    }
    if material_power > 0.0 && material_power.is_finite() {
        let bits = along.to_bits();
        if bits < 0x7f80_0000 {
            let upper_log2 = if bits < 0x0080_0000 {
                -126.0f64
            } else {
                f64::from(((bits >> 23) as i32) - 126)
            };
            if upper_log2 < 0.0 && f64::from(material_power) * upper_log2 <= -151.0 {
                if !safe_zero {
                    for (s, c) in shine.iter_mut().zip(colour) {
                        *s += c * 0.0 * reaching;
                    }
                }
                return;
            }
        }
    }
    // The engine's own powers, never the host's.
    let exp = f64::from(material_power) * crate::cpu::x87::maths::log2(f64::from(along));
    if exp <= -151.0 && exp.is_finite() {
        if !safe_zero {
            for (s, c) in shine.iter_mut().zip(colour) {
                *s += c * 0.0 * reaching;
            }
        }
        return;
    }
    let power = crate::cpu::x87::maths::pow2(exp) as f32;
    for (s, c) in shine.iter_mut().zip(colour) {
        *s += c * power * reaching;
    }
}

#[inline(always)]
fn falloff(d: f32, a: [f32; 3]) -> f32 {
    1.0 / (a[0] + a[1] * d + a[2] * d * d).max(f32::MIN_POSITIVE)
}

/// [`lit`] using lights prepared once per pass via [`PreparedLights`].
///
/// When `highlights` is `false` (or `prepared.viewer` is `None`), specular
/// highlights are skipped.
#[must_use]
pub fn lit_prepared(
    position: [f32; 3],
    normal: [f32; 3],
    material: &Material,
    prepared: &PreparedLights,
    highlights: bool,
) -> ([f32; 4], [f32; 3]) {
    let normal_len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let (normal, normal_finite) = if normal_len > 0.0 {
        (
            super::scene::normalise_with_length(normal, normal_len),
            normal_len.is_finite(),
        )
    } else {
        ([0.0, 0.0, 0.0], true)
    };
    let ambient = prepared.ambient;
    let mut lit = [0.0f32; 3];
    let mut shine = [0.0f32; 3];
    let viewer = if highlights { prepared.viewer } else { None };
    let safe_zero_base = prepared.all_colours_safe;
    let safe_spot_zero = safe_zero_base
        && normal_finite
        && prepared.viewer_finite
        && material.power >= 0.0
        && material.power <= 1_000_000.0;

    for light in &prepared.lights {
        match *light {
            PrecomputedLight::Directional {
                colour,
                towards,
                halfway,
                safe_zero,
            } => {
                let ndotl = dot(normal, towards);
                let strength = ndotl.max(0.0);
                lit[0] += colour[0] * strength;
                lit[1] += colour[1] * strength;
                lit[2] += colour[2] * strength;
                if viewer.is_none() || ndotl <= 0.0 {
                    continue;
                }
                if let Some(h) = halfway {
                    add_highlight(
                        &mut shine,
                        colour,
                        normal,
                        h,
                        material.power,
                        1.0,
                        safe_zero && safe_zero_base,
                    );
                }
            }
            PrecomputedLight::Point {
                colour,
                position: at,
                range,
                attenuation,
                safe_zero,
            } => {
                let v = [
                    at[0] - position[0],
                    at[1] - position[1],
                    at[2] - position[2],
                ];
                let d = dot(v, v).sqrt();
                if d > 0.0 && d <= range {
                    let l = super::scene::normalise_with_length(v, d);
                    let ndotl = dot(normal, l);
                    let att = falloff(d, attenuation);
                    let strength = ndotl.max(0.0) * att;
                    lit[0] += colour[0] * strength;
                    lit[1] += colour[1] * strength;
                    lit[2] += colour[2] * strength;
                    let Some(view) = viewer else {
                        continue;
                    };
                    if ndotl <= 0.0 {
                        continue;
                    }
                    if let Some(h) = normalise(std::array::from_fn(|i| l[i] + view[i])) {
                        add_highlight(
                            &mut shine,
                            colour,
                            normal,
                            h,
                            material.power,
                            att,
                            safe_zero && safe_zero_base,
                        );
                    }
                }
            }
            PrecomputedLight::Spot {
                colour,
                position: at,
                axis,
                range,
                attenuation,
                inner,
                outer,
                cone_denom,
                safe_zero,
            } => {
                let v = [
                    at[0] - position[0],
                    at[1] - position[1],
                    at[2] - position[2],
                ];
                let d = dot(v, v).sqrt();
                if d <= 0.0 || d.is_nan() || d > range {
                    continue;
                }
                let l = super::scene::normalise_with_length(v, d);
                let rho = dot(l.map(|c| -c), axis);
                let cone = if rho > inner {
                    1.0
                } else if rho <= outer {
                    0.0
                } else {
                    (rho - outer) / cone_denom
                };
                let ndotl = dot(normal, l);
                let att = falloff(d, attenuation);
                let strength = ndotl.max(0.0) * att * cone;
                lit[0] += colour[0] * strength;
                lit[1] += colour[1] * strength;
                lit[2] += colour[2] * strength;
                let Some(view) = viewer else {
                    continue;
                };
                if ndotl <= 0.0 || (cone == 0.0 && safe_spot_zero && safe_zero) {
                    continue;
                }
                if let Some(h) = normalise(std::array::from_fn(|i| l[i] + view[i])) {
                    add_highlight(
                        &mut shine,
                        colour,
                        normal,
                        h,
                        material.power,
                        att * cone,
                        safe_zero && safe_zero_base,
                    );
                }
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
    let normal_len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let (normal, normal_finite) = if normal_len > 0.0 {
        (
            super::scene::normalise_with_length(normal, normal_len),
            normal_len.is_finite(),
        )
    } else {
        ([0.0, 0.0, 0.0], true)
    };
    let mut ambient = ambient;
    let mut lit = [0.0f32; 3];
    let mut shine = [0.0f32; 3];
    let viewer_finite =
        viewer.is_some_and(|v| v[0].is_finite() && v[1].is_finite() && v[2].is_finite());
    let power_safe = material.power >= 0.0 && material.power <= 1_000_000.0;
    let mut all_colours_safe = true;
    for light in lights {
        match *light {
            Light::Ambient { colour } => {
                for (a, c) in ambient.iter_mut().zip(colour) {
                    *a += c;
                }
            }
            Light::Directional { colour, direction } => {
                if let Some(d) = normalise(direction) {
                    let safe_zero = is_safe_colour(colour);
                    all_colours_safe &= safe_zero;
                    let towards = d.map(|c| -c);
                    let ndotl = dot(normal, towards);
                    let strength = ndotl.max(0.0);
                    lit[0] += colour[0] * strength;
                    lit[1] += colour[1] * strength;
                    lit[2] += colour[2] * strength;
                    let Some(view) = viewer else {
                        continue;
                    };
                    if ndotl <= 0.0 {
                        continue;
                    }
                    if let Some(h) = normalise(std::array::from_fn(|i| towards[i] + view[i])) {
                        add_highlight(
                            &mut shine,
                            colour,
                            normal,
                            h,
                            material.power,
                            1.0,
                            safe_zero && all_colours_safe,
                        );
                    }
                }
            }
            Light::Point {
                colour,
                position: at,
                range,
                attenuation,
            } => {
                let v = [
                    at[0] - position[0],
                    at[1] - position[1],
                    at[2] - position[2],
                ];
                let d = dot(v, v).sqrt();
                if d > 0.0 && d <= range {
                    let safe_zero = is_safe_colour(colour);
                    all_colours_safe &= safe_zero;
                    let l = super::scene::normalise_with_length(v, d);
                    let ndotl = dot(normal, l);
                    let att = falloff(d, attenuation);
                    let strength = ndotl.max(0.0) * att;
                    lit[0] += colour[0] * strength;
                    lit[1] += colour[1] * strength;
                    lit[2] += colour[2] * strength;
                    let Some(view) = viewer else {
                        continue;
                    };
                    if ndotl <= 0.0 {
                        continue;
                    }
                    if let Some(h) = normalise(std::array::from_fn(|i| l[i] + view[i])) {
                        add_highlight(
                            &mut shine,
                            colour,
                            normal,
                            h,
                            material.power,
                            att,
                            safe_zero && all_colours_safe,
                        );
                    }
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
                let v = [
                    at[0] - position[0],
                    at[1] - position[1],
                    at[2] - position[2],
                ];
                let d = dot(v, v).sqrt();
                if d <= 0.0 || d.is_nan() {
                    continue;
                }
                let Some(axis) = normalise(direction) else {
                    continue;
                };
                if d > range {
                    continue;
                }
                let safe_zero = is_safe_colour(colour);
                all_colours_safe &= safe_zero;
                let l = super::scene::normalise_with_length(v, d);
                let rho = dot(l.map(|c| -c), axis);
                let cone = if rho > inner {
                    1.0
                } else if rho <= outer {
                    0.0
                } else {
                    (rho - outer) / (inner - outer).max(f32::MIN_POSITIVE)
                };
                let ndotl = dot(normal, l);
                let att = falloff(d, attenuation);
                let strength = ndotl.max(0.0) * att * cone;
                lit[0] += colour[0] * strength;
                lit[1] += colour[1] * strength;
                lit[2] += colour[2] * strength;
                let Some(view) = viewer else {
                    continue;
                };
                let safe_spot_zero =
                    all_colours_safe && safe_zero && normal_finite && viewer_finite && power_safe;
                if ndotl <= 0.0 || (cone == 0.0 && safe_spot_zero) {
                    continue;
                }
                if let Some(h) = normalise(std::array::from_fn(|i| l[i] + view[i])) {
                    add_highlight(
                        &mut shine,
                        colour,
                        normal,
                        h,
                        material.power,
                        att * cone,
                        safe_zero && all_colours_safe,
                    );
                }
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

/// Stack capacity for clipping a convex triangle (at most 9 vertices against 6
/// frustum planes) or quad (at most 10) without heap allocation.
pub const CLIP_BUFFER: usize = 12;

/// Whether a clip-space vertex lies wholly inside the view volume:
/// `-w <= x, y <= w` and `0 <= z <= w`.
#[must_use]
#[inline]
pub fn inside_frustum(v: &Clipped) -> bool {
    let [x, y, z, w] = v.clip;
    -w <= x && x <= w && -w <= y && y <= w && 0.0 <= z && z <= w
}

/// Each plane of the view volume as the distance of a clip-space point inside it.
const PLANES: [fn(&[f32; 4]) -> f32; 6] = [
    |c| c[3] + c[0],
    |c| c[3] - c[0],
    |c| c[3] + c[1],
    |c| c[3] - c[1],
    |c| c[2],
    |c| c[3] - c[2],
];

fn clip_plane(
    input: &[Clipped],
    inside: fn(&[f32; 4]) -> f32,
    output: &mut [Clipped; CLIP_BUFFER],
) -> Option<usize> {
    let n = input.len();
    let mut out_len = 0;
    for (i, current) in input.iter().enumerate() {
        let previous = &input[if i == 0 { n - 1 } else { i - 1 }];
        let (dc, dp) = (inside(&current.clip), inside(&previous.clip));
        if (dc >= 0.0) != (dp >= 0.0) {
            if out_len == CLIP_BUFFER {
                return None;
            }
            output[out_len] = lerp(previous, current, dp / (dp - dc));
            out_len += 1;
        }
        if dc >= 0.0 {
            if out_len == CLIP_BUFFER {
                return None;
            }
            output[out_len] = *current;
            out_len += 1;
        }
    }
    Some(out_len)
}

/// Cut a polygon to the view volume into a caller-supplied stack buffer.
/// Returns `None` if the input or an intermediate result exceeds
/// [`CLIP_BUFFER`] (impossible for any finite convex triangle or quad, possible
/// only for large polygons or pathological NaN inputs), in which case [`clip`]
/// handles it on the heap.
#[must_use]
pub fn clip_into(polygon: &[Clipped], out: &mut [Clipped; CLIP_BUFFER]) -> Option<usize> {
    if polygon.len() > CLIP_BUFFER {
        return None;
    }
    if polygon.iter().all(inside_frustum) {
        out[..polygon.len()].copy_from_slice(polygon);
        return Some(polygon.len());
    }
    out[..polygon.len()].copy_from_slice(polygon);
    let mut scratch = [Clipped::default(); CLIP_BUFFER];
    let mut len = polygon.len();
    // Six planes is even: planes 0, 2, 4 write into `scratch` and 1, 3, 5 back
    // into `out`, so the final polygon lands in `out[..len]`.
    for (plane, inside) in PLANES.into_iter().enumerate() {
        if len == 0 {
            return Some(0);
        }
        len = if plane % 2 == 0 {
            clip_plane(&out[..len], inside, &mut scratch)?
        } else {
            clip_plane(&scratch[..len], inside, out)?
        };
    }
    Some(len)
}

#[cold]
#[inline(never)]
fn clip_vec(polygon: &[Clipped]) -> Vec<Clipped> {
    if polygon.iter().all(inside_frustum) {
        return polygon.to_vec();
    }
    let mut out = polygon.to_vec();
    for inside in PLANES {
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

/// Cut a polygon to the view volume: `-w <= x, y <= w` and `0 <= z <= w`.
#[must_use]
pub fn clip(polygon: &[Clipped]) -> Vec<Clipped> {
    let mut out = [Clipped::default(); CLIP_BUFFER];
    if let Some(len) = clip_into(polygon, &mut out) {
        out[..len].to_vec()
    } else {
        clip_vec(polygon)
    }
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
        // Minus zero depth lands on positive zero.
        let neg_zero = viewport.place(&Clipped {
            clip: [0.0, 0.0, -0.0, 1.0],
            ..Clipped::default()
        });
        assert_eq!(neg_zero.z.to_bits(), 0.0f32.to_bits());
    }

    #[test]
    fn clip_into_matches_heap_clip_bit_for_bit_including_multi_plane_and_overflow() {
        let v = |clip: [f32; 4], u: f32, t: f32| Clipped {
            clip,
            diffuse: [u, t, 0.5, 1.0],
            specular: [0.1, 0.2, 0.3],
            uv: [u, t],
            fog: 0.75,
        };
        let assert_same = |poly: &[Clipped], label: &str| {
            let expected = super::clip_vec(poly);
            let mut buf = [Clipped::default(); super::CLIP_BUFFER];
            let len = super::clip_into(poly, &mut buf).expect(label);
            assert_eq!(len, expected.len(), "{label}: vertex count");
            for (got, want) in buf[..len].iter().zip(&expected) {
                assert_eq!(
                    got.clip.map(f32::to_bits),
                    want.clip.map(f32::to_bits),
                    "{label}: clip bits"
                );
                assert_eq!(
                    got.diffuse.map(f32::to_bits),
                    want.diffuse.map(f32::to_bits),
                    "{label}: diffuse bits"
                );
                assert_eq!(
                    got.specular.map(f32::to_bits),
                    want.specular.map(f32::to_bits),
                    "{label}: specular bits"
                );
                assert_eq!(
                    got.uv.map(f32::to_bits),
                    want.uv.map(f32::to_bits),
                    "{label}: uv bits"
                );
                assert_eq!(got.fog.to_bits(), want.fog.to_bits(), "{label}: fog bits");
            }
        };
        // Wholly inside.
        assert_same(
            &[
                v([0.0, 0.0, 0.5, 1.0], 0.0, 0.0),
                v([0.5, 0.0, 0.5, 1.0], 1.0, 0.0),
                v([0.0, 0.5, 0.5, 1.0], 0.0, 1.0),
            ],
            "inside",
        );
        // Wholly outside.
        assert_same(
            &[
                v([2.0, 0.0, 0.5, 1.0], 0.0, 0.0),
                v([3.0, 0.0, 0.5, 1.0], 1.0, 0.0),
                v([2.0, 1.0, 0.5, 1.0], 0.0, 1.0),
            ],
            "outside",
        );
        // Large oblique triangle crossing all six frustum planes.
        let six_planes = [
            v([-3.0, -2.5, -1.5, 1.0], 0.0, 0.0),
            v([3.5, -1.5, 2.5, 1.0], 1.0, 0.0),
            v([0.2, 3.5, 0.5, 1.0], 0.5, 1.0),
        ];
        assert_same(&six_planes, "six planes");
        assert!(clip(&six_planes).len() <= 9);

        // Input larger than CLIP_BUFFER spills cleanly to the heap fallback.
        let big: Vec<Clipped> = (0..14)
            .map(|i| v([0.0, 0.0, i as f32 * 0.05, 1.0], i as f32, 0.0))
            .collect();
        let mut buf = [Clipped::default(); super::CLIP_BUFFER];
        assert_eq!(super::clip_into(&big, &mut buf), None);
        assert_eq!(clip(&big), super::clip_vec(&big));
    }

    /// Verbatim pre-#273 reference implementation of `lit` for bit-exact
    /// differential testing against `lit` and `lit_prepared`.
    fn reference_lit(
        position: [f32; 3],
        normal: [f32; 3],
        material: &Material,
        ambient: [f32; 3],
        lights: &[Light],
        viewer: Option<[f32; 3]>,
    ) -> ([f32; 4], [f32; 3]) {
        let normal = super::normalise(normal).unwrap_or([0.0, 0.0, 0.0]);
        let mut ambient = ambient;
        let mut lit = [0.0f32; 3];
        let mut shine = [0.0f32; 3];
        let mut highlight = |colour: [f32; 3], l: [f32; 3], ndotl: f32, reaching: f32| {
            let Some(v) = viewer else {
                return;
            };
            if ndotl <= 0.0 {
                return;
            }
            let Some(h) = super::normalise(std::array::from_fn(|i| l[i] + v[i])) else {
                return;
            };
            let along = super::dot(normal, h);
            if along <= 0.0 {
                return;
            }
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
            (super::normalise(v), super::dot(v, v).sqrt())
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
                    if let Some(d) = super::normalise(direction) {
                        let towards = d.map(|c| -c);
                        let ndotl = super::dot(normal, towards);
                        add(colour, ndotl.max(0.0));
                        highlight(colour, towards, ndotl, 1.0);
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
                        let ndotl = super::dot(normal, l);
                        let att = falloff(d, attenuation);
                        add(colour, ndotl.max(0.0) * att);
                        highlight(colour, l, ndotl, att);
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
                    let Some(axis) = super::normalise(direction) else {
                        continue;
                    };
                    if d > range {
                        continue;
                    }
                    let rho = super::dot(l.map(|c| -c), axis);
                    let cone = if rho > inner {
                        1.0
                    } else if rho <= outer {
                        0.0
                    } else {
                        (rho - outer) / (inner - outer).max(f32::MIN_POSITIVE)
                    };
                    let ndotl = super::dot(normal, l);
                    let att = falloff(d, attenuation);
                    add(colour, ndotl.max(0.0) * att * cone);
                    highlight(colour, l, ndotl, att * cone);
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

    #[test]
    fn precomputed_lights_and_lit_match_reference_bit_for_bit() {
        let lights = vec![
            Light::Ambient {
                colour: [0.15, 0.2, 0.25],
            },
            Light::Directional {
                colour: [0.9, 0.8, 0.7],
                direction: [-0.3, -0.8, 0.5],
            },
            Light::Directional {
                colour: [1.0, 1.0, 1.0],
                direction: [0.0, 0.0, 0.0], // degenerate zero direction
            },
            Light::Ambient {
                colour: [0.05, 0.05, 0.1],
            },
            Light::Point {
                colour: [0.7, 0.4, 0.2],
                position: [2.0, 3.0, -1.0],
                range: 10.0,
                attenuation: [1.0, 0.1, 0.02],
            },
            Light::Spot {
                colour: [0.8, 0.9, 1.0],
                position: [0.0, 5.0, 0.0],
                direction: [0.0, -1.0, 0.0],
                range: 20.0,
                attenuation: [1.0, 0.05, 0.01],
                inner: 0.95,
                outer: 0.80,
            },
            Light::Spot {
                colour: [1.0, 0.5, 0.5],
                position: [1.0, 2.0, 3.0],
                direction: [0.0, 0.0, 0.0], // degenerate spot axis
                range: 10.0,
                attenuation: [1.0, 0.0, 0.0],
                inner: 0.9,
                outer: 0.7,
            },
        ];

        // Positional detection: directional/ambient-only has_positional == false.
        let non_pos = super::PreparedLights::new([0.0; 3], &lights[..4], Some([0.0, 0.0, -1.0]));
        assert!(!non_pos.has_positional());
        let with_pos = super::PreparedLights::new([0.0; 3], &lights, Some([0.0, 0.0, -1.0]));
        assert!(with_pos.has_positional());

        let viewers = [None, Some([0.0, 0.0, -1.0]), Some([0.3, 0.6, -0.74])];
        let powers = [
            0.0,
            1.0,
            8.0,
            32.0,
            256.0,
            2000.0,
            -5.0,
            f32::INFINITY,
            f32::NAN,
        ];
        let positions = [
            [0.0, 0.0, 0.0],
            [2.0, 3.0, -1.0],  // coincident with point light (d == 0)
            [0.0, 1.0, 0.0],   // inside spot inner cone
            [2.0, 1.0, 0.0],   // inside spot falloff cone
            [15.0, 1.0, 0.0],  // outside spot cone (cone == 0.0) & outside point range
            [100.0, 0.0, 0.0], // outside all ranges
        ];
        let normals = [
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.3, 0.9, -0.2],
            [0.99, 0.01, 0.01], // hits exp <= -151.0 underflow skip at high power
            [f32::INFINITY, 0.0, 0.0],
            [f32::from_bits(0x7f80_0001), 1.0, 0.0],
        ];
        let base_ambient = [0.1, 0.05, 0.0];

        for viewer in viewers {
            let prepared = super::PreparedLights::new(base_ambient, &lights, viewer);
            for &power in &powers {
                let material = Material {
                    diffuse: [0.8, 0.6, 0.4, 0.75],
                    ambient: [0.5, 0.5, 0.5, 1.0],
                    specular: [0.9, 0.8, 0.7, 0.0],
                    emissive: [0.02, 0.01, 0.0, 0.0],
                    power,
                };
                for &pos in &positions {
                    for &norm in &normals {
                        let expected =
                            reference_lit(pos, norm, &material, base_ambient, &lights, viewer);
                        let got_lit =
                            super::lit(pos, norm, &material, base_ambient, &lights, viewer);
                        let got_prep =
                            super::lit_prepared(pos, norm, &material, &prepared, viewer.is_some());
                        assert_eq!(
                            got_lit.0.map(f32::to_bits),
                            expected.0.map(f32::to_bits),
                            "lit diffuse mismatch pos={pos:?} norm={norm:?} power={power} viewer={viewer:?}"
                        );
                        assert_eq!(
                            got_lit.1.map(f32::to_bits),
                            expected.1.map(f32::to_bits),
                            "lit specular mismatch pos={pos:?} norm={norm:?} power={power} viewer={viewer:?}"
                        );
                        assert_eq!(
                            got_prep.0.map(f32::to_bits),
                            expected.0.map(f32::to_bits),
                            "lit_prepared diffuse mismatch pos={pos:?} norm={norm:?} power={power} viewer={viewer:?}"
                        );
                        assert_eq!(
                            got_prep.1.map(f32::to_bits),
                            expected.1.map(f32::to_bits),
                            "lit_prepared specular mismatch pos={pos:?} norm={norm:?} power={power} viewer={viewer:?}"
                        );
                    }
                }
            }
        }
    }
}
