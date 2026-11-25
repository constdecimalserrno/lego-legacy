//! Host-side visual grounding and observation overlays over presented BGRA
//! frames (#309).
//!
//! Strictly host-side and read-only: operates only on [`FrameSnapshot`] copies
//! without touching guest memory. Supports an `80x80` labeled coordinate grid
//! (`A..H`/`1..6`), high-contrast cursor crosshair overlay, 2× nearest-neighbor
//! crop zoom, integer RGB downsampled grids, and stuck-loop warnings.

use super::bus::FrameSnapshot;
use super::json::{write_json_f64, write_json_str};
use std::fmt::Write as _;

/// Threshold of consecutive identical flips before emitting a stuck-loop warning.
pub const STUCK_LOOP_THRESHOLD: u64 = 30;

/// Options controlling host-side visual grounding overlays for `/api/frame.png`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObserveOptions {
    /// Draw an `80x80` labeled coordinate grid (`A..H`, `1..6`).
    pub grid: bool,
    /// Draw a high-contrast crosshair at `pointer_pos`.
    pub cursor: bool,
    /// Optional `(x, y, w, h)` sub-rectangle cropped and 2× nearest-neighbor zoomed.
    pub crop: Option<(u32, u32, u32, u32)>,
}

/// Parse `crop=x,y,w,h` query syntax.
///
/// # Errors
/// Returns a descriptive error if not 4 integers or if `w == 0` / `h == 0`.
pub fn parse_crop(spec: &str) -> Result<(u32, u32, u32, u32), String> {
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() != 4 {
        return Err("crop must be `x,y,w,h`".to_owned());
    }
    let parse_u = |s: &str| {
        s.trim()
            .parse::<u32>()
            .map_err(|_| format!("invalid crop coordinate `{s}`"))
    };
    let (x, y, w, h) = (
        parse_u(parts[0])?,
        parse_u(parts[1])?,
        parse_u(parts[2])?,
        parse_u(parts[3])?,
    );
    if w == 0 || h == 0 {
        return Err("crop width and height must be > 0".to_owned());
    }
    Ok((x, y, w, h))
}

/// Parse `grid=CxR` (e.g. `16x12`) query syntax for `/api/frame.json`.
///
/// # Errors
/// Returns a descriptive error on malformed grid dimensions.
pub fn parse_grid_dims(spec: &str) -> Result<(u32, u32), String> {
    let (c, r) = spec
        .split_once('x')
        .or_else(|| spec.split_once('X'))
        .ok_or_else(|| format!("grid must be `<cols>x<rows>`, got `{spec}`"))?;
    let cols = c
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid grid cols `{c}`"))?;
    let rows = r
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid grid rows `{r}`"))?;
    if !(1..=64).contains(&cols) || !(1..=64).contains(&rows) {
        return Err("grid dimensions must be in 1..=64".to_owned());
    }
    Ok((cols, rows))
}

/// Render a [`FrameSnapshot`] as PNG bytes with optional `80x80` grid overlay,
/// cursor crosshair overlay, and 2× nearest-neighbor crop zoom.
///
/// # Errors
/// Returns an error if the snapshot is empty or the crop rectangle lies outside
/// the frame.
pub fn render_frame_png(
    snap: &FrameSnapshot,
    opts: ObserveOptions,
    pointer_pos: (i32, i32),
) -> Result<Vec<u8>, String> {
    if snap.width == 0 || snap.height == 0 {
        return Err("frame is empty".to_owned());
    }
    let expected = (snap.width as usize)
        .saturating_mul(snap.height as usize)
        .saturating_mul(4);
    if snap.bgra.len() < expected {
        return Err("frame buffer truncated".to_owned());
    }

    if !opts.grid && !opts.cursor && opts.crop.is_none() {
        return Ok(crate::png::encode(
            snap.width,
            snap.height,
            &snap.bgra[..expected],
        ));
    }

    let mut buf = snap.bgra[..expected].to_vec();
    if opts.grid {
        draw_grid_overlay(&mut buf, snap.width, snap.height);
    }
    if opts.cursor {
        draw_cursor_crosshair(&mut buf, snap.width, snap.height, pointer_pos);
    }

    if let Some((cx, cy, cw, ch)) = opts.crop {
        let Some(x_end) = cx.checked_add(cw) else {
            return Err("crop rectangle overflows".to_owned());
        };
        let Some(y_end) = cy.checked_add(ch) else {
            return Err("crop rectangle overflows".to_owned());
        };
        if x_end > snap.width || y_end > snap.height {
            return Err(format!(
                "crop ({cx},{cy},{cw},{ch}) exceeds frame {}x{}",
                snap.width, snap.height
            ));
        }
        let out_w = cw * 2;
        let out_h = ch * 2;
        let mut zoomed = vec![0u8; (out_w as usize) * (out_h as usize) * 4];
        for sy in 0..ch {
            let src_y = (cy + sy) as usize;
            for sx in 0..cw {
                let src_x = (cx + sx) as usize;
                let si = (src_y * (snap.width as usize) + src_x) * 4;
                let px = [buf[si], buf[si + 1], buf[si + 2], buf[si + 3]];
                let dx = (sx * 2) as usize;
                let dy = (sy * 2) as usize;
                for oy in 0..2 {
                    for ox in 0..2 {
                        let di = ((dy + oy) * (out_w as usize) + (dx + ox)) * 4;
                        zoomed[di..di + 4].copy_from_slice(&px);
                    }
                }
            }
        }
        return Ok(crate::png::encode(out_w, out_h, &zoomed));
    }

    Ok(crate::png::encode(snap.width, snap.height, &buf))
}

#[inline]
fn blend_px(buf: &mut [u8], width: u32, height: u32, x: i32, y: i32, bgr: [u8; 3], alpha: u16) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
    let inv = 256 - alpha;
    buf[idx] = ((u16::from(buf[idx]) * inv + u16::from(bgr[0]) * alpha) >> 8) as u8;
    buf[idx + 1] = ((u16::from(buf[idx + 1]) * inv + u16::from(bgr[1]) * alpha) >> 8) as u8;
    buf[idx + 2] = ((u16::from(buf[idx + 2]) * inv + u16::from(bgr[2]) * alpha) >> 8) as u8;
    buf[idx + 3] = 255;
}

#[inline]
fn set_px(buf: &mut [u8], width: u32, height: u32, x: i32, y: i32, bgr: [u8; 3]) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let idx = ((y as usize) * (width as usize) + (x as usize)) * 4;
    buf[idx] = bgr[0];
    buf[idx + 1] = bgr[1];
    buf[idx + 2] = bgr[2];
    buf[idx + 3] = 255;
}

/// Draw an `80x80` coordinate grid labeled `A..H` across columns and `1..6`
/// down rows (generalizing cleanly to any frame size).
fn draw_grid_overlay(buf: &mut [u8], width: u32, height: u32) {
    const CELL: u32 = 80;
    // Semi-transparent yellow-cyan lines with a dark 1px shadow for contrast.
    let line_fg = [0u8, 255, 255]; // BGR yellow
    let line_bg = [0u8, 0, 0];

    let mut x = CELL;
    while x < width {
        for y in 0..height as i32 {
            blend_px(buf, width, height, x as i32 - 1, y, line_bg, 140);
            blend_px(buf, width, height, x as i32, y, line_fg, 170);
        }
        x += CELL;
    }
    let mut y = CELL;
    while y < height {
        for x in 0..width as i32 {
            blend_px(buf, width, height, x, y as i32 - 1, line_bg, 140);
            blend_px(buf, width, height, x, y as i32, line_fg, 170);
        }
        y += CELL;
    }

    let cols = width.div_ceil(CELL);
    let rows = height.div_ceil(CELL);
    for row in 0..rows {
        for col in 0..cols {
            let ox = (col * CELL) as i32 + 3;
            let oy = (row * CELL) as i32 + 3;
            let col_ch = char::from(b'A' + (col % 26) as u8);
            let label = format!("{col_ch}{}", row + 1);
            // Dark pill behind label for guaranteed legibility.
            let box_w = (label.len() as i32) * 6 + 3;
            for by in (oy - 1)..(oy + 8) {
                for bx in (ox - 1)..(ox + box_w) {
                    blend_px(buf, width, height, bx, by, [0, 0, 0], 180);
                }
            }
            for (i, ch) in label.chars().enumerate() {
                draw_glyph(buf, width, height, ox + (i as i32) * 6, oy, ch, line_fg);
            }
        }
    }
}

/// Draw a high-contrast crosshair centered at `pointer_pos`.
fn draw_cursor_crosshair(buf: &mut [u8], width: u32, height: u32, pointer_pos: (i32, i32)) {
    let (cx, cy) = pointer_pos;
    let outer = [0u8, 0, 0]; // black outline
    let inner = [255u8, 255, 0]; // bright cyan (BGR)
    let center = [0u8, 0, 255]; // bright red center dot (BGR)

    // Arms: 11px outward with a 2px gap near center so the target pixel stays visible.
    for d in -12..=12 {
        if (-2..=2).contains(&d) {
            continue;
        }
        for o in -1..=1 {
            set_px(buf, width, height, cx + d, cy + o, outer);
            set_px(buf, width, height, cx + o, cy + d, outer);
        }
    }
    for d in -11..=11 {
        if (-2..=2).contains(&d) {
            continue;
        }
        set_px(buf, width, height, cx + d, cy, inner);
        set_px(buf, width, height, cx, cy + d, inner);
    }
    set_px(buf, width, height, cx, cy, center);
}

/// Minimal 5×7 bitmap glyph lookup for uppercase letters and digits.
fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0x0e, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0e],
        '1' => [0x04, 0x0c, 0x04, 0x04, 0x04, 0x04, 0x0e],
        '2' => [0x0e, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1f],
        '3' => [0x0e, 0x11, 0x01, 0x06, 0x01, 0x11, 0x0e],
        '4' => [0x02, 0x06, 0x0a, 0x12, 0x1f, 0x02, 0x02],
        '5' => [0x1f, 0x10, 0x1e, 0x01, 0x01, 0x11, 0x0e],
        '6' => [0x06, 0x08, 0x10, 0x1e, 0x11, 0x11, 0x0e],
        '7' => [0x1f, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0e, 0x11, 0x11, 0x0e, 0x11, 0x11, 0x0e],
        '9' => [0x0e, 0x11, 0x11, 0x0f, 0x01, 0x02, 0x0c],
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
        _ => [0; 7],
    }
}

fn draw_glyph(buf: &mut [u8], width: u32, height: u32, x0: i32, y0: i32, ch: char, bgr: [u8; 3]) {
    let bits = glyph_5x7(ch);
    for (row, &mask) in bits.iter().enumerate() {
        for col in 0..5 {
            if (mask & (1 << (4 - col))) != 0 {
                set_px(buf, width, height, x0 + col, y0 + row as i32, bgr);
            }
        }
    }
}

/// Compute a `cols × rows` integer RGB downsampled grid over a dense BGRA frame.
#[must_use]
pub fn downsample_rgb_grid(
    bgra: &[u8],
    width: u32,
    height: u32,
    cols: u32,
    rows: u32,
) -> Vec<Vec<[u8; 3]>> {
    let cols = cols.clamp(1, 64);
    let rows = rows.clamp(1, 64);
    let w = width as usize;
    let h = height as usize;
    let mut grid = Vec::with_capacity(rows as usize);
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return grid;
    }

    for gy in 0..rows {
        let y0 = ((u64::from(gy) * u64::from(height)) / u64::from(rows)) as usize;
        let y1 = (((u64::from(gy + 1) * u64::from(height)) / u64::from(rows)) as usize)
            .max(y0 + 1)
            .min(h);
        let mut row_cells = Vec::with_capacity(cols as usize);
        for gx in 0..cols {
            let x0 = ((u64::from(gx) * u64::from(width)) / u64::from(cols)) as usize;
            let x1 = (((u64::from(gx + 1) * u64::from(width)) / u64::from(cols)) as usize)
                .max(x0 + 1)
                .min(w);
            let mut sr = 0u64;
            let mut sg = 0u64;
            let mut sb = 0u64;
            let mut count = 0u64;
            for y in y0..y1 {
                let row_off = y * w * 4;
                for x in x0..x1 {
                    let i = row_off + x * 4;
                    sb += u64::from(bgra[i]);
                    sg += u64::from(bgra[i + 1]);
                    sr += u64::from(bgra[i + 2]);
                    count += 1;
                }
            }
            let c = count.max(1);
            row_cells.push([(sr / c) as u8, (sg / c) as u8, (sb / c) as u8]);
        }
        grid.push(row_cells);
    }
    grid
}

/// Build the JSON payload for `GET /api/frame.json?grid=CxR`.
#[must_use]
pub fn format_frame_json(
    snap: &FrameSnapshot,
    cols: u32,
    rows: u32,
    pointer_pos: (i32, i32),
    pointer_shown: bool,
) -> String {
    let grid = downsample_rgb_grid(&snap.bgra, snap.width, snap.height, cols, rows);
    let mut warnings: Vec<String> = Vec::new();
    if snap.uniform {
        warnings.push("uniform_frame: every pixel has the same color".to_owned());
    }
    if snap.diff.unchanged_streak >= STUCK_LOOP_THRESHOLD {
        warnings.push(format!(
            "stuck_loop: frame unchanged for {} consecutive flips",
            snap.diff.unchanged_streak
        ));
    }

    let mut out = String::with_capacity(2048);
    let _ = write!(
        out,
        "{{\"ok\":true,\"flip\":{},\"width\":{},\"height\":{},\"frame_hash\":",
        snap.flip, snap.width, snap.height
    );
    write_json_str(&mut out, &snap.frame_hash);
    let _ = write!(
        out,
        ",\"uniform\":{},\"diff_pixels\":{},\"total_pixels\":{},\"diff_pct\":",
        snap.uniform, snap.diff.diff_pixels, snap.diff.total_pixels
    );
    write_json_f64(&mut out, snap.diff.diff_pct);
    out.push_str(",\"diff_bbox\":");
    match snap.diff.diff_bbox {
        Some((x, y, w, h)) => {
            let _ = write!(out, "[{x},{y},{w},{h}]");
        }
        None => out.push_str("null"),
    }
    let _ = write!(
        out,
        ",\"unchanged_streak\":{},\"pointer_pos\":[{},{}],\"pointer_shown\":{},\"warnings\":[",
        snap.diff.unchanged_streak, pointer_pos.0, pointer_pos.1, pointer_shown
    );
    for (i, w) in warnings.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_json_str(&mut out, w);
    }
    let _ = write!(
        out,
        "],\"grid\":{{\"cols\":{cols},\"rows\":{rows},\"cells\":["
    );
    for (ry, row) in grid.iter().enumerate() {
        if ry > 0 {
            out.push(',');
        }
        out.push('[');
        for (rx, rgb) in row.iter().enumerate() {
            if rx > 0 {
                out.push(',');
            }
            let _ = write!(out, "[{},{},{}]", rgb[0], rgb[1], rgb[2]);
        }
        out.push(']');
    }
    out.push_str("]}}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::bus::FrameDiff;
    use std::sync::Arc;

    fn sample_snapshot(w: u32, h: u32, streak: u64, uniform: bool) -> FrameSnapshot {
        let mut bgra = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                if uniform {
                    bgra[i..i + 4].copy_from_slice(&[20, 40, 60, 255]);
                } else {
                    bgra[i] = (x % 255) as u8;
                    bgra[i + 1] = (y % 255) as u8;
                    bgra[i + 2] = ((x + y) % 255) as u8;
                    bgra[i + 3] = 255;
                }
            }
        }
        FrameSnapshot {
            flip: 12,
            width: w,
            height: h,
            bgra: Arc::new(bgra),
            frame_hash: "0123456789abcdef".to_owned(),
            diff: FrameDiff {
                diff_pixels: if streak > 0 { 0 } else { 10 },
                total_pixels: u64::from(w) * u64::from(h),
                diff_pct: if streak > 0 { 0.0 } else { 1.0 },
                diff_bbox: None,
                unchanged_streak: streak,
            },
            uniform,
        }
    }

    #[test]
    fn grid_cursor_and_crop_render_valid_png_with_expected_dimensions() {
        let snap = sample_snapshot(640, 480, 0, false);

        let full = render_frame_png(
            &snap,
            ObserveOptions {
                grid: true,
                cursor: true,
                crop: None,
            },
            (320, 240),
        )
        .expect("full png");
        assert_eq!(&full[..8], b"\x89PNG\r\n\x1a\n");
        let w = u32::from_be_bytes([full[16], full[17], full[18], full[19]]);
        let h = u32::from_be_bytes([full[20], full[21], full[22], full[23]]);
        assert_eq!((w, h), (640, 480));

        let cropped = render_frame_png(
            &snap,
            ObserveOptions {
                grid: true,
                cursor: true,
                crop: Some((100, 100, 80, 60)),
            },
            (140, 130),
        )
        .expect("cropped png");
        let cw = u32::from_be_bytes([cropped[16], cropped[17], cropped[18], cropped[19]]);
        let ch = u32::from_be_bytes([cropped[20], cropped[21], cropped[22], cropped[23]]);
        assert_eq!((cw, ch), (160, 120), "crop must be 2x zoomed");

        assert!(render_frame_png(
            &snap,
            ObserveOptions {
                crop: Some((600, 400, 80, 100)),
                ..ObserveOptions::default()
            },
            (0, 0),
        )
        .is_err());
    }

    #[test]
    fn frame_json_downsamples_rgb_and_emits_stuck_and_uniform_warnings() {
        let snap = sample_snapshot(160, 120, STUCK_LOOP_THRESHOLD + 5, true);
        let text = format_frame_json(&snap, 16, 12, (80, 60), true);
        let parsed = crate::control::json::parse_json(&text).expect("valid json");
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj["flip"].as_u64(), Some(12));
        assert_eq!(obj["uniform"].as_bool(), Some(true));
        let warnings = obj["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 2);
        let cells = obj["grid"].as_object().unwrap()["cells"]
            .as_array()
            .unwrap();
        assert_eq!(cells.len(), 12);
        assert_eq!(cells[0].as_array().unwrap().len(), 16);
        let first_rgb = cells[0].as_array().unwrap()[0].as_array().unwrap();
        assert_eq!(first_rgb[0].as_u64(), Some(60)); // R
        assert_eq!(first_rgb[1].as_u64(), Some(40)); // G
        assert_eq!(first_rgb[2].as_u64(), Some(20)); // B
    }
}
