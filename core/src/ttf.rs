//! Pure-Rust, zero-dependency TrueType (`sfnt` `0x00010000`) font parser and
//! deterministic fixed-point scanline rasterizer (#394).
//!
//! Used by `win32::gdi32` to render TrueType fonts registered through
//! `AddFontResourceA` onto DirectDraw surface device contexts with bit-exact
//! cross-platform determinism (Golden Rule 1 & 5).

use std::ops::Range;
use std::rc::Rc;

/// A parsed TrueType font backed by immutable font bytes.
#[derive(Clone, Debug)]
pub struct Font {
    data: Rc<[u8]>,
    pub family: String,
    pub units_per_em: u16,
    pub ascent: i16,
    pub descent: i16,
    pub line_gap: i16,
    index_to_loc_format: i16,
    num_glyphs: u16,
    num_h_metrics: u16,
    cmap: Range<usize>,
    hmtx: Range<usize>,
    loca: Range<usize>,
    glyf: Range<usize>,
}

/// One rasterized 1-bit coverage glyph ready to blit onto a pixel buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterGlyph {
    pub x_off: i32,
    pub y_off: i32,
    pub width: i32,
    pub height: i32,
    pub advance: i32,
    /// Row-major `width * height` mask (`1` = inked, `0` = background).
    pub mask: Vec<u8>,
}

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn be_i16(b: &[u8], at: usize) -> Option<i16> {
    Some(be_u16(b, at)? as i16)
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// Safe integer scale factor for the built-in 5×7 fallback bitmap font.
#[must_use]
pub fn fallback_scale(lf_height: i32) -> i32 {
    let mag = lf_height.unsigned_abs().clamp(7, 42) as i32;
    (mag / 7).clamp(1, 6)
}

impl Font {
    /// Parse a TrueType font from raw bytes.
    #[must_use]
    pub fn parse(bytes: Rc<[u8]>) -> Option<Self> {
        let d = &bytes[..];
        let sfnt = be_u32(d, 0)?;
        if sfnt != 0x0001_0000 && sfnt != u32::from_be_bytes(*b"true") {
            return None;
        }
        let num_tables = usize::from(be_u16(d, 4)?);
        let mut head = None;
        let mut hhea = None;
        let mut maxp = None;
        let mut name = None;
        let mut cmap = None;
        let mut hmtx = None;
        let mut loca = None;
        let mut glyf = None;

        for i in 0..num_tables {
            let rec = 12usize.checked_add(i.checked_mul(16)?)?;
            let tag = d.get(rec..rec + 4)?;
            let off = be_u32(d, rec + 8)? as usize;
            let len = be_u32(d, rec + 12)? as usize;
            let end = off.checked_add(len)?;
            if end > d.len() {
                return None;
            }
            let range = off..end;
            match tag {
                b"head" => head = Some(range),
                b"hhea" => hhea = Some(range),
                b"maxp" => maxp = Some(range),
                b"name" => name = Some(range),
                b"cmap" => cmap = Some(range),
                b"hmtx" => hmtx = Some(range),
                b"loca" => loca = Some(range),
                b"glyf" => glyf = Some(range),
                _ => {}
            }
        }

        let head = head?;
        if head.len() < 54 {
            return None;
        }
        let units_per_em = be_u16(d, head.start + 18)?;
        if units_per_em == 0 {
            return None;
        }
        let index_to_loc_format = be_i16(d, head.start + 50)?;

        let hhea = hhea?;
        if hhea.len() < 36 {
            return None;
        }
        let ascent = be_i16(d, hhea.start + 4)?;
        let descent = be_i16(d, hhea.start + 6)?;
        let line_gap = be_i16(d, hhea.start + 8)?;
        let num_h_metrics = be_u16(d, hhea.start + 34)?;

        let maxp = maxp?;
        if maxp.len() < 6 {
            return None;
        }
        let num_glyphs = be_u16(d, maxp.start + 4)?;

        let family = name
            .and_then(|r| parse_family_name(&d[r]))
            .unwrap_or_default();

        Some(Self {
            data: bytes,
            family,
            units_per_em,
            ascent,
            descent,
            line_gap,
            index_to_loc_format,
            num_glyphs,
            num_h_metrics,
            cmap: cmap?,
            hmtx: hmtx?,
            loca: loca?,
            glyf: glyf?,
        })
    }

    /// Fixed-point `(num, den)` scale ratio for a Win32 `LOGFONT.lfHeight`,
    /// clamped to `[1, 256]` px so extreme guest values cannot overflow.
    fn height_ratio(&self, lf_height: i32) -> (i64, i64) {
        let em = i64::from(self.units_per_em).max(1);
        let cell = (i64::from(self.ascent) - i64::from(self.descent)).max(1);
        if lf_height < 0 {
            (i64::from(lf_height.unsigned_abs().clamp(1, 256)), em)
        } else if lf_height > 0 {
            (i64::from(lf_height.clamp(1, 256)), cell)
        } else {
            (16, cell)
        }
    }

    /// `(ascent_px, line_height_px)` for the given Win32 `lfHeight`.
    #[must_use]
    pub fn line_metrics(&self, lf_height: i32) -> (i32, i32) {
        let (num, den) = self.height_ratio(lf_height);
        let ascent_px = ((i64::from(self.ascent) * num + den / 2) / den).clamp(1, 512) as i32;
        let descent_px = ((-i64::from(self.descent) * num + den / 2) / den).clamp(0, 512) as i32;
        let gap_px = ((i64::from(self.line_gap.max(0)) * num + den / 2) / den).clamp(0, 512) as i32;
        (ascent_px, (ascent_px + descent_px + gap_px).max(1))
    }

    /// Look up glyph index for a Unicode codepoint via `cmap` (prefers Format 4
    /// Windows Unicode BMP, falls back to Format 0 Mac Roman).
    #[must_use]
    pub fn glyph_id(&self, ch: char) -> u16 {
        let cmap = &self.data[self.cmap.clone()];
        let Some(nsub) = be_u16(cmap, 2) else {
            return 0;
        };
        let cp = ch as u32;
        let mut fmt4_off = None;
        let mut fmt0_off = None;
        for i in 0..usize::from(nsub) {
            let rec = 4 + i * 8;
            let (Some(pid), Some(eid), Some(sub)) = (
                be_u16(cmap, rec),
                be_u16(cmap, rec + 2),
                be_u32(cmap, rec + 4),
            ) else {
                break;
            };
            let sub = sub as usize;
            let Some(fmt) = be_u16(cmap, sub) else {
                continue;
            };
            if fmt == 4 && (pid == 3 && (eid == 1 || eid == 10) || pid == 0) {
                fmt4_off = Some(sub);
                if pid == 3 && eid == 1 {
                    break;
                }
            } else if fmt == 0 && fmt0_off.is_none() {
                fmt0_off = Some(sub);
            }
        }
        if let Some(sub) = fmt4_off {
            if cp <= 0xffff {
                if let Some(gid) = lookup_cmap_format4(cmap, sub, cp as u16) {
                    return gid;
                }
            }
        }
        if let Some(sub) = fmt0_off {
            if cp < 256 {
                if let Some(&gid) = cmap.get(sub + 6 + cp as usize) {
                    return u16::from(gid);
                }
            }
        }
        0
    }

    fn advance_units(&self, gid: u16) -> u16 {
        let hmtx = &self.data[self.hmtx.clone()];
        let nh = usize::from(self.num_h_metrics);
        if nh == 0 {
            return self.units_per_em / 2;
        }
        let idx = usize::from(gid).min(nh - 1);
        be_u16(hmtx, idx * 4).unwrap_or(self.units_per_em / 2)
    }

    /// Horizontal advance in pixels for `ch` at Win32 `lf_height`.
    #[must_use]
    pub fn advance_px(&self, ch: char, lf_height: i32) -> i32 {
        let gid = self.glyph_id(ch);
        let adv = i64::from(self.advance_units(gid));
        let (num, den) = self.height_ratio(lf_height);
        ((adv * num + den / 2) / den).clamp(1, 512) as i32
    }

    fn glyph_slice(&self, gid: u16) -> Option<&[u8]> {
        if gid >= self.num_glyphs {
            return None;
        }
        let loca = &self.data[self.loca.clone()];
        let glyf = &self.data[self.glyf.clone()];
        let idx = usize::from(gid);
        let (start, end) = if self.index_to_loc_format == 0 {
            let s = usize::from(be_u16(loca, idx * 2)?) * 2;
            let e = usize::from(be_u16(loca, (idx + 1) * 2)?) * 2;
            (s, e)
        } else {
            let s = be_u32(loca, idx * 4)? as usize;
            let e = be_u32(loca, (idx + 1) * 4)? as usize;
            (s, e)
        };
        if end <= start || end > glyf.len() {
            return None;
        }
        Some(&glyf[start..end])
    }

    fn decode_contours(&self, gid: u16, depth: u8) -> Vec<Vec<(i32, i32, bool)>> {
        if depth > 8 {
            return Vec::new();
        }
        let Some(g) = self.glyph_slice(gid) else {
            return Vec::new();
        };
        let Some(n_contours) = be_i16(g, 0) else {
            return Vec::new();
        };
        if n_contours > 0 {
            decode_simple_glyph(g, n_contours as usize)
        } else if n_contours < 0 {
            self.decode_compound_glyph(g, depth + 1)
        } else {
            Vec::new()
        }
    }

    fn decode_compound_glyph(&self, g: &[u8], depth: u8) -> Vec<Vec<(i32, i32, bool)>> {
        const ARG_1_AND_2_ARE_WORDS: u16 = 0x0001;
        const ARGS_ARE_XY_VALUES: u16 = 0x0002;
        const WE_HAVE_A_SCALE: u16 = 0x0008;
        const MORE_COMPONENTS: u16 = 0x0020;
        const WE_HAVE_AN_X_AND_Y_SCALE: u16 = 0x0040;
        const WE_HAVE_A_TWO_BY_TWO: u16 = 0x0080;

        let mut out = Vec::new();
        let mut pos = 10usize;
        while let (Some(flags), Some(comp_gid)) = (be_u16(g, pos), be_u16(g, pos + 2)) {
            pos += 4;
            let (dx, dy) = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
                let (Some(a), Some(b)) = (be_i16(g, pos), be_i16(g, pos + 2)) else {
                    break;
                };
                pos += 4;
                if flags & ARGS_ARE_XY_VALUES != 0 {
                    (i32::from(a), i32::from(b))
                } else {
                    (0, 0)
                }
            } else {
                let (Some(&a), Some(&b)) = (g.get(pos), g.get(pos + 1)) else {
                    break;
                };
                pos += 2;
                if flags & ARGS_ARE_XY_VALUES != 0 {
                    (i32::from(a as i8), i32::from(b as i8))
                } else {
                    (0, 0)
                }
            };
            let (mut xx, mut xy, mut yx, mut yy) = (16384i64, 0i64, 0i64, 16384i64);
            if flags & WE_HAVE_A_SCALE != 0 {
                let Some(s) = be_i16(g, pos) else { break };
                pos += 2;
                xx = i64::from(s);
                yy = i64::from(s);
            } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
                let (Some(sx), Some(sy)) = (be_i16(g, pos), be_i16(g, pos + 2)) else {
                    break;
                };
                pos += 4;
                xx = i64::from(sx);
                yy = i64::from(sy);
            } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
                let (Some(a), Some(b), Some(c), Some(d)) = (
                    be_i16(g, pos),
                    be_i16(g, pos + 2),
                    be_i16(g, pos + 4),
                    be_i16(g, pos + 6),
                ) else {
                    break;
                };
                pos += 8;
                xx = i64::from(a);
                yx = i64::from(b);
                xy = i64::from(c);
                yy = i64::from(d);
            }
            for contour in self.decode_contours(comp_gid, depth) {
                let transformed = contour
                    .into_iter()
                    .map(|(x, y, on)| {
                        let tx = ((i64::from(x) * xx + i64::from(y) * xy) >> 14) as i32 + dx;
                        let ty = ((i64::from(x) * yx + i64::from(y) * yy) >> 14) as i32 + dy;
                        (tx, ty, on)
                    })
                    .collect();
                out.push(transformed);
            }
            if flags & MORE_COMPONENTS == 0 {
                break;
            }
        }
        out
    }

    /// Rasterize `ch` at Win32 `lf_height` using purely integer 26.6 fixed-point
    /// non-zero winding scan conversion (2×2 deterministic supersampling).
    /// `x_off`/`y_off` are relative to the baseline origin `(pen_x, baseline_y)`.
    #[must_use]
    pub fn rasterize(&self, ch: char, lf_height: i32, bold: bool) -> RasterGlyph {
        let gid = self.glyph_id(ch);
        let advance = self.advance_px(ch, lf_height) + i32::from(bold);
        let empty = || RasterGlyph {
            x_off: 0,
            y_off: 0,
            width: 0,
            height: 0,
            advance,
            mask: Vec::new(),
        };
        let contours = self.decode_contours(gid, 0);
        if contours.is_empty() {
            return empty();
        }

        let (num, den) = self.height_ratio(lf_height);
        let scale_x = |fx: i32| -> i64 { (i64::from(fx) * num * 64) / den };
        let scale_y = |fy: i32| -> i64 { (-i64::from(fy) * num * 64) / den };

        let mut edges: Vec<(i64, i64, i64, i64)> = Vec::new();
        let mut min_x = i64::MAX;
        let mut max_x = i64::MIN;
        let mut min_y = i64::MAX;
        let mut max_y = i64::MIN;

        let mut push_edge = |x0: i64, y0: i64, x1: i64, y1: i64| {
            min_x = min_x.min(x0).min(x1);
            max_x = max_x.max(x0).max(x1);
            min_y = min_y.min(y0).min(y1);
            max_y = max_y.max(y0).max(y1);
            if y0 != y1 {
                edges.push((x0, y0, x1, y1));
            }
        };

        for contour in &contours {
            if contour.len() < 2 {
                continue;
            }
            let pts: Vec<(i64, i64, bool)> = contour
                .iter()
                .map(|&(x, y, on)| (scale_x(x), scale_y(y), on))
                .collect();
            let n = pts.len();
            let start = if pts[0].2 {
                (pts[0].0, pts[0].1)
            } else if pts[n - 1].2 {
                (pts[n - 1].0, pts[n - 1].1)
            } else {
                ((pts[n - 1].0 + pts[0].0) / 2, (pts[n - 1].1 + pts[0].1) / 2)
            };
            let mut cur = start;
            let mut i = usize::from(pts[0].2);
            while i < n {
                let (px, py, on) = pts[i];
                if on {
                    push_edge(cur.0, cur.1, px, py);
                    cur = (px, py);
                    i += 1;
                } else {
                    let next = if i + 1 < n {
                        let (nx, ny, non) = pts[i + 1];
                        if non {
                            i += 2;
                            (nx, ny)
                        } else {
                            i += 1;
                            ((px + nx) / 2, (py + ny) / 2)
                        }
                    } else {
                        i += 1;
                        start
                    };
                    const STEPS: i64 = 8;
                    let mut prev = cur;
                    for s in 1..=STEPS {
                        let u = STEPS - s;
                        let denom = STEPS * STEPS;
                        let qx = (u * u * cur.0 + 2 * u * s * px + s * s * next.0) / denom;
                        let qy = (u * u * cur.1 + 2 * u * s * py + s * s * next.1) / denom;
                        push_edge(prev.0, prev.1, qx, qy);
                        prev = (qx, qy);
                    }
                    cur = next;
                }
            }
            if cur != start {
                push_edge(cur.0, cur.1, start.0, start.1);
            }
        }

        if edges.is_empty() || min_x >= max_x || min_y >= max_y {
            return empty();
        }

        let x0 = (min_x >> 6).clamp(-2048, 2048) as i32;
        let x1 = ((max_x + 63) >> 6).clamp(-2048, 2048) as i32 + i32::from(bold);
        let y0 = (min_y >> 6).clamp(-2048, 2048) as i32;
        let y1 = ((max_y + 63) >> 6).clamp(-2048, 2048) as i32;
        let width = (x1 - x0).clamp(0, 256);
        let height = (y1 - y0).clamp(0, 256);
        if width == 0 || height == 0 {
            return empty();
        }

        let mut mask = vec![0u8; (width * height) as usize];
        const SUBS: [i64; 2] = [16, 48];
        let mut crossings: Vec<(i64, i32)> = Vec::new();
        for row in 0..height {
            let mut row_hits = vec![0u8; width as usize];
            for &sy in &SUBS {
                let scan_y = i64::from(y0 + row) * 64 + sy;
                crossings.clear();
                for &(ex0, ey0, ex1, ey1) in &edges {
                    let (lo, hi, dir) = if ey0 < ey1 {
                        (ey0, ey1, 1)
                    } else {
                        (ey1, ey0, -1)
                    };
                    if scan_y >= lo && scan_y < hi {
                        let ix = ex0 + ((scan_y - ey0) * (ex1 - ex0)) / (ey1 - ey0);
                        crossings.push((ix, dir));
                    }
                }
                if crossings.is_empty() {
                    continue;
                }
                crossings.sort_unstable_by_key(|&(ix, _)| ix);
                for (col, hit) in row_hits.iter_mut().enumerate() {
                    let base_x = i64::from(x0 + col as i32) * 64;
                    for &sx in &SUBS {
                        let sample_x = base_x + sx;
                        let mut winding = 0i32;
                        for &(ix, dir) in &crossings {
                            if ix > sample_x {
                                break;
                            }
                            winding += dir;
                        }
                        if winding != 0 {
                            *hit += 1;
                        }
                    }
                }
            }
            let row_base = (row * width) as usize;
            for (col, &hits) in row_hits.iter().enumerate() {
                if hits >= 2 {
                    mask[row_base + col] = 1;
                    if bold && col + 1 < width as usize {
                        mask[row_base + col + 1] = 1;
                    }
                }
            }
        }

        RasterGlyph {
            x_off: x0,
            y_off: y0,
            width,
            height,
            advance,
            mask,
        }
    }
}

fn parse_family_name(name: &[u8]) -> Option<String> {
    let count = usize::from(be_u16(name, 2)?);
    let str_off = usize::from(be_u16(name, 4)?);
    let mut mac_fallback = None;
    for i in 0..count {
        let rec = 6 + i * 12;
        let (Some(pid), Some(nid), Some(len), Some(off)) = (
            be_u16(name, rec),
            be_u16(name, rec + 6),
            be_u16(name, rec + 8),
            be_u16(name, rec + 10),
        ) else {
            break;
        };
        if nid != 1 {
            continue;
        }
        let start = str_off.checked_add(usize::from(off))?;
        let slice = name.get(start..start.checked_add(usize::from(len))?)?;
        if pid == 3 {
            let units: Vec<u16> = slice
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            return Some(String::from_utf16_lossy(&units));
        } else if pid == 1 && mac_fallback.is_none() {
            mac_fallback = Some(slice.iter().map(|&b| char::from(b)).collect());
        }
    }
    mac_fallback
}

fn lookup_cmap_format4(cmap: &[u8], sub: usize, cp: u16) -> Option<u16> {
    let seg_count_x2 = usize::from(be_u16(cmap, sub + 6)?);
    let seg_count = seg_count_x2 / 2;
    let end_codes = sub + 14;
    let start_codes = end_codes + seg_count_x2 + 2;
    let id_deltas = start_codes + seg_count_x2;
    let id_range_offs = id_deltas + seg_count_x2;

    for i in 0..seg_count {
        let end = be_u16(cmap, end_codes + i * 2)?;
        if cp > end {
            continue;
        }
        let start = be_u16(cmap, start_codes + i * 2)?;
        if cp < start {
            return Some(0);
        }
        let delta = be_i16(cmap, id_deltas + i * 2)?;
        let ro_at = id_range_offs + i * 2;
        let ro = usize::from(be_u16(cmap, ro_at)?);
        let gid = if ro == 0 {
            cp.wrapping_add(delta as u16)
        } else {
            let idx_at = ro_at + ro + usize::from(cp - start) * 2;
            let raw = be_u16(cmap, idx_at)?;
            if raw == 0 {
                0
            } else {
                raw.wrapping_add(delta as u16)
            }
        };
        return Some(gid);
    }
    Some(0)
}

fn decode_simple_glyph(g: &[u8], n_contours: usize) -> Vec<Vec<(i32, i32, bool)>> {
    let mut end_pts = Vec::with_capacity(n_contours);
    for i in 0..n_contours {
        let Some(ep) = be_u16(g, 10 + i * 2) else {
            return Vec::new();
        };
        end_pts.push(usize::from(ep));
    }
    let Some(&last_pt) = end_pts.last() else {
        return Vec::new();
    };
    let n_pts = last_pt + 1;
    let ins_at = 10 + n_contours * 2;
    let Some(ins_len) = be_u16(g, ins_at) else {
        return Vec::new();
    };
    let mut pos = ins_at + 2 + usize::from(ins_len);

    let mut flags = Vec::with_capacity(n_pts);
    while flags.len() < n_pts {
        let Some(&f) = g.get(pos) else {
            return Vec::new();
        };
        pos += 1;
        flags.push(f);
        if f & 0x08 != 0 {
            let Some(&rep) = g.get(pos) else {
                return Vec::new();
            };
            pos += 1;
            for _ in 0..rep {
                if flags.len() < n_pts {
                    flags.push(f);
                }
            }
        }
    }

    let mut decode_axis = |short_bit: u8, same_bit: u8| -> Option<Vec<i32>> {
        let mut coords = Vec::with_capacity(n_pts);
        let mut val = 0i32;
        for &f in &flags {
            if f & short_bit != 0 {
                let b = i32::from(*g.get(pos)?);
                pos += 1;
                val += if f & same_bit != 0 { b } else { -b };
            } else if f & same_bit == 0 {
                val += i32::from(be_i16(g, pos)?);
                pos += 2;
            }
            coords.push(val);
        }
        Some(coords)
    };
    let Some(xs) = decode_axis(0x02, 0x10) else {
        return Vec::new();
    };
    let Some(ys) = decode_axis(0x04, 0x20) else {
        return Vec::new();
    };

    let mut contours = Vec::with_capacity(n_contours);
    let mut start = 0usize;
    for end in end_pts {
        if end < start || end >= n_pts {
            break;
        }
        let mut pts = Vec::with_capacity(end - start + 1);
        for i in start..=end {
            pts.push((xs[i], ys[i], flags[i] & 0x01 != 0));
        }
        contours.push(pts);
        start = end + 1;
    }
    contours
}

/// Built-in 5×7 ASCII bitmap fallback used when a GDI DC has no loaded
/// TrueType font matching the selected typeface.
#[must_use]
pub fn fallback_glyph(ch: char, pixel_height: i32) -> RasterGlyph {
    let scale = fallback_scale(pixel_height);
    let rows = fallback_5x7(ch);
    let width = 5 * scale;
    let height = 7 * scale;
    let advance = 6 * scale;
    let mut mask = vec![0u8; (width * height) as usize];
    for (ry, &bits) in rows.iter().enumerate() {
        for rx in 0..5 {
            if bits & (1 << (4 - rx)) != 0 {
                for sy in 0..scale {
                    for sx in 0..scale {
                        let px = rx * scale + sx;
                        let py = (ry as i32) * scale + sy;
                        mask[(py * width + px) as usize] = 1;
                    }
                }
            }
        }
    }
    RasterGlyph {
        x_off: 0,
        y_off: -height,
        width,
        height,
        advance,
        mask,
    }
}

fn fallback_5x7(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        '"' => [0x0a, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x00],
        '#' => [0x0a, 0x1f, 0x0a, 0x0a, 0x1f, 0x0a, 0x00],
        '$' => [0x04, 0x0f, 0x14, 0x0e, 0x05, 0x1e, 0x04],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '&' => [0x0c, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0d],
        '\'' => [0x04, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '*' => [0x00, 0x0a, 0x04, 0x1f, 0x04, 0x0a, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1f, 0x04, 0x04, 0x00],
        ',' => [0x00, 0x00, 0x00, 0x00, 0x04, 0x04, 0x08],
        '-' => [0x00, 0x00, 0x00, 0x1f, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x0c],
        '/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1f],
        '3' => [0x1f, 0x02, 0x04, 0x06, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
        ':' => [0x00, 0x0c, 0x0c, 0x00, 0x0c, 0x0c, 0x00],
        ';' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x08],
        '<' => [0x02, 0x04, 0x08, 0x10, 0x08, 0x04, 0x02],
        '=' => [0x00, 0x00, 0x1f, 0x00, 0x1f, 0x00, 0x00],
        '>' => [0x08, 0x04, 0x02, 0x01, 0x02, 0x04, 0x08],
        '?' => [0x0e, 0x11, 0x01, 0x02, 0x04, 0x00, 0x04],
        '@' => [0x0e, 0x11, 0x17, 0x15, 0x17, 0x10, 0x0e],
        'A' => [0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'B' => [0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e],
        'C' => [0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e],
        'D' => [0x1c, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1c],
        'E' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f],
        'F' => [0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10],
        'G' => [0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f],
        'H' => [0x11, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11],
        'I' => [0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0e],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0c],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f],
        'M' => [0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        'O' => [0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'P' => [0x1e, 0x11, 0x11, 0x1e, 0x10, 0x10, 0x10],
        'Q' => [0x0e, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0d],
        'R' => [0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11],
        'S' => [0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e],
        'T' => [0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1b, 0x11],
        'X' => [0x11, 0x11, 0x0a, 0x04, 0x0a, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1f],
        '[' => [0x0e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0e],
        '\\' => [0x10, 0x08, 0x08, 0x04, 0x02, 0x02, 0x01],
        ']' => [0x0e, 0x02, 0x02, 0x02, 0x02, 0x02, 0x0e],
        '^' => [0x04, 0x0a, 0x11, 0x00, 0x00, 0x00, 0x00],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1f],
        _ => [0x1f, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1f],
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a minimal valid TrueType font containing a solid square glyph for `'A'`.
    pub(crate) fn synthetic_square_ttf(family: &str) -> Vec<u8> {
        let mut head = vec![0u8; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&0i16.to_be_bytes());

        let mut hhea = vec![0u8; 36];
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());

        let mut maxp = vec![0u8; 6];
        maxp[4..6].copy_from_slice(&2u16.to_be_bytes());

        let fam_u16: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut name = Vec::new();
        name.extend_from_slice(&0u16.to_be_bytes());
        name.extend_from_slice(&1u16.to_be_bytes());
        name.extend_from_slice(&18u16.to_be_bytes());
        name.extend_from_slice(&3u16.to_be_bytes());
        name.extend_from_slice(&1u16.to_be_bytes());
        name.extend_from_slice(&0x0409u16.to_be_bytes());
        name.extend_from_slice(&1u16.to_be_bytes());
        name.extend_from_slice(&(fam_u16.len() as u16).to_be_bytes());
        name.extend_from_slice(&0u16.to_be_bytes());
        name.extend_from_slice(&fam_u16);

        let mut cmap = Vec::new();
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        cmap.extend_from_slice(&3u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        cmap.extend_from_slice(&12u32.to_be_bytes());
        cmap.extend_from_slice(&4u16.to_be_bytes());
        cmap.extend_from_slice(&32u16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&4u16.to_be_bytes());
        cmap.extend_from_slice(&4u16.to_be_bytes());
        cmap.extend_from_slice(&1u16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&65u16.to_be_bytes());
        cmap.extend_from_slice(&0xffffu16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&65u16.to_be_bytes());
        cmap.extend_from_slice(&0xffffu16.to_be_bytes());
        cmap.extend_from_slice(&(-64i16).to_be_bytes());
        cmap.extend_from_slice(&1i16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());
        cmap.extend_from_slice(&0u16.to_be_bytes());

        let mut hmtx = Vec::new();
        hmtx.extend_from_slice(&600u16.to_be_bytes());
        hmtx.extend_from_slice(&0i16.to_be_bytes());
        hmtx.extend_from_slice(&700u16.to_be_bytes());
        hmtx.extend_from_slice(&50i16.to_be_bytes());

        let mut glyf = Vec::new();
        glyf.extend_from_slice(&1i16.to_be_bytes());
        glyf.extend_from_slice(&100i16.to_be_bytes());
        glyf.extend_from_slice(&0i16.to_be_bytes());
        glyf.extend_from_slice(&600i16.to_be_bytes());
        glyf.extend_from_slice(&700i16.to_be_bytes());
        glyf.extend_from_slice(&3u16.to_be_bytes());
        glyf.extend_from_slice(&0u16.to_be_bytes());
        glyf.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
        for dx in [100i16, 0, 500, 0] {
            glyf.extend_from_slice(&dx.to_be_bytes());
        }
        for dy in [0i16, 700, 0, -700] {
            glyf.extend_from_slice(&dy.to_be_bytes());
        }
        while !glyf.len().is_multiple_of(4) {
            glyf.push(0);
        }

        let mut loca = Vec::new();
        loca.extend_from_slice(&0u16.to_be_bytes());
        loca.extend_from_slice(&0u16.to_be_bytes());
        loca.extend_from_slice(&((glyf.len() / 2) as u16).to_be_bytes());

        let tables: &[(&[u8; 4], &[u8])] = &[
            (b"cmap", &cmap),
            (b"glyf", &glyf),
            (b"head", &head),
            (b"hhea", &hhea),
            (b"hmtx", &hmtx),
            (b"loca", &loca),
            (b"maxp", &maxp),
            (b"name", &name),
        ];
        let mut out = Vec::new();
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0u8; 6]);
        let mut offset = 12 + tables.len() * 16;
        for (tag, data) in tables {
            out.extend_from_slice(*tag);
            out.extend_from_slice(&0u32.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            offset += (data.len() + 3) & !3;
        }
        for (_, data) in tables {
            out.extend_from_slice(data);
            while !out.len().is_multiple_of(4) {
                out.push(0);
            }
        }
        out
    }

    #[test]
    fn parses_and_rasterizes_truetype_square_glyph_and_handles_extreme_heights() {
        let bytes = synthetic_square_ttf("TestFace");
        let font = Font::parse(bytes.into()).expect("valid ttf");
        assert_eq!(font.family, "TestFace");
        assert_eq!(font.glyph_id('A'), 1);
        assert_eq!(font.glyph_id('Z'), 0);

        let glyph = font.rasterize('A', -20, false);
        assert_eq!(glyph.advance, 14);
        assert!(glyph.width >= 9 && glyph.height >= 13);
        let inked: usize = glyph.mask.iter().filter(|&&b| b != 0).count();
        assert!(
            inked >= 100,
            "solid square should ink interior pixels: {inked}"
        );

        // Extreme lfHeight values (i32::MIN / i32::MAX) must never panic.
        let _ = font.rasterize('A', i32::MIN, true);
        let _ = font.rasterize(' ', i32::MAX, true);
        assert_eq!(fallback_scale(i32::MIN), 6);
    }
}
