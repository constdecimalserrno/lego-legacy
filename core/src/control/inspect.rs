//! Read-only machine-boundary inspector (#309).
//!
//! Takes an immutable `&Machine` at flip boundaries and answers `/api/mem`,
//! `/api/mem/map`, `/api/mem/search`, `/api/cpu`, `/api/win32`, `/api/files`,
//! `/api/files/read`, `/api/route`, and bookmark captures with ZERO mutation to
//! guest memory, CPU registers, clock, Win32 state, or ISO file handles
//! (Golden Rules 1 & 2).

use super::json::{write_json_f64_or_null, write_json_str};
use crate::cpu::{ESP, REG_NAMES, SEG_NAMES};
use crate::disc::Iso;
use crate::guest::heap::{HEAP_BASE, HEAP_BYTES};
use crate::guest::GuestMem;
use crate::machine::{Machine, PEB, STACK_BYTES, STACK_TOP, TIB};
use crate::win32::{ComState, Object};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

/// Hard upper bound for a single `/api/mem` read (1 MiB).
pub const MAX_MEM_READ_LEN: usize = 1024 * 1024;
/// Hard upper bound for an explicit `/api/mem/search` span (64 MiB).
pub const MAX_MEM_SEARCH_SPAN: usize = 64 * 1024 * 1024;
/// Hard upper bound on `/api/mem/search` returned match count.
pub const MAX_MEM_SEARCH_LIMIT: usize = 4096;
/// Hard upper bound on a single `/api/files/read` payload (64 MiB).
pub const MAX_FILE_READ_LEN: usize = 64 * 1024 * 1024;

/// Output formatting for `GET /api/mem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemFmt {
    Hex,
    Raw,
    U8,
    U16,
    U32,
    I32,
    F32,
    CStr,
}

impl MemFmt {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hex" => Some(Self::Hex),
            "raw" => Some(Self::Raw),
            "u8" => Some(Self::U8),
            "u16" => Some(Self::U16),
            "u32" => Some(Self::U32),
            "i32" => Some(Self::I32),
            "f32" => Some(Self::F32),
            "cstr" => Some(Self::CStr),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::Raw => "raw",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::I32 => "i32",
            Self::F32 => "f32",
            Self::CStr => "cstr",
        }
    }
}

/// Which root `/api/files` and `/api/files/read` inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileRoot {
    Disc,
    Profile,
}

impl FileRoot {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "disc" => Some(Self::Disc),
            "profile" => Some(Self::Profile),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disc => "disc",
            Self::Profile => "profile",
        }
    }
}

/// Read-only inspection request executed on `&Machine` at a flip boundary.
#[derive(Clone, Debug)]
pub enum InspectQuery {
    MemRead {
        addr: u32,
        len: usize,
        fmt: MemFmt,
    },
    MemMap,
    MemSearch {
        needle: Vec<u8>,
        start: Option<u32>,
        end: Option<u32>,
        limit: usize,
    },
    Cpu,
    Win32,
    FilesList {
        root: FileRoot,
        path: String,
    },
    FilesRead {
        root: FileRoot,
        path: String,
    },
    Route,
    BookmarkCapture,
}

/// Deterministic Tier-1 route checkpoint captured from `&Machine`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarkData {
    pub flip: u64,
    pub cursor: u64,
    pub declared_ms: u64,
    pub digest: String,
    pub route_text: String,
}

/// Response produced by [`execute_inspect`].
#[derive(Clone, Debug)]
pub enum InspectReply {
    Json(String),
    Raw {
        content_type: &'static str,
        bytes: Vec<u8>,
        sha256: Option<String>,
    },
    Text(String),
    Bookmark(BookmarkData),
    Error {
        status: u16,
        message: String,
    },
}

/// Execute a read-only [`InspectQuery`] against `&Machine`.
#[must_use]
pub fn execute_inspect(machine: &Machine, query: InspectQuery) -> InspectReply {
    match query {
        InspectQuery::MemRead { addr, len, fmt } => inspect_mem_read(machine, addr, len, fmt),
        InspectQuery::MemMap => InspectReply::Json(inspect_mem_map(machine)),
        InspectQuery::MemSearch {
            needle,
            start,
            end,
            limit,
        } => inspect_mem_search(machine, &needle, start, end, limit),
        InspectQuery::Cpu => InspectReply::Json(inspect_cpu(machine)),
        InspectQuery::Win32 => InspectReply::Json(inspect_win32(machine)),
        InspectQuery::FilesList { root, path } => inspect_files_list(machine, root, &path),
        InspectQuery::FilesRead { root, path } => inspect_files_read(machine, root, &path),
        InspectQuery::Route => {
            let text = machine.recording.as_ref().map_or_else(
                || "#lego-legacy-route 1\n".to_owned(),
                crate::route::Route::to_text,
            );
            InspectReply::Text(text)
        }
        InspectQuery::BookmarkCapture => {
            let route_text = machine.recording.as_ref().map_or_else(
                || "#lego-legacy-route 1\n".to_owned(),
                crate::route::Route::to_text,
            );
            InspectReply::Bookmark(BookmarkData {
                flip: machine.flips(),
                cursor: machine.clock.cursor(),
                declared_ms: machine.clock.millis(),
                digest: machine.state_digest(),
                route_text,
            })
        }
    }
}

fn inspect_mem_read(machine: &Machine, addr: u32, len: usize, fmt: MemFmt) -> InspectReply {
    let capped = len.min(MAX_MEM_READ_LEN);
    let bytes = match machine.arena.slice(addr, capped) {
        Ok(b) => b,
        Err(fault) => {
            return InspectReply::Error {
                status: 422,
                message: fault.to_string(),
            };
        }
    };

    if fmt == MemFmt::Raw {
        return InspectReply::Raw {
            content_type: "application/octet-stream",
            bytes: bytes.to_vec(),
            sha256: None,
        };
    }

    let mut out = String::with_capacity(128 + capped * 2);
    let _ = write!(
        out,
        "{{\"ok\":true,\"addr\":\"{addr:#010x}\",\"addr_u32\":{addr},\"len\":{},\"fmt\":",
        bytes.len()
    );
    write_json_str(&mut out, fmt.as_str());
    match fmt {
        MemFmt::Hex => {
            out.push_str(",\"hex\":");
            write_json_str(&mut out, &crate::hash::hex(bytes));
        }
        MemFmt::U8 => {
            out.push_str(",\"values\":[");
            for (i, &b) in bytes.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{b}");
            }
            out.push(']');
        }
        MemFmt::U16 => {
            out.push_str(",\"values\":[");
            for (i, c) in bytes.as_chunks::<2>().0.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", u16::from_le_bytes(*c));
            }
            out.push(']');
        }
        MemFmt::U32 => {
            out.push_str(",\"values\":[");
            for (i, c) in bytes.as_chunks::<4>().0.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", u32::from_le_bytes(*c));
            }
            out.push(']');
        }
        MemFmt::I32 => {
            out.push_str(",\"values\":[");
            for (i, c) in bytes.as_chunks::<4>().0.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(out, "{}", i32::from_le_bytes(*c));
            }
            out.push(']');
        }
        MemFmt::F32 => {
            out.push_str(",\"values\":[");
            for (i, c) in bytes.as_chunks::<4>().0.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_f64_or_null(&mut out, f64::from(f32::from_le_bytes(*c)));
            }
            out.push(']');
        }
        MemFmt::CStr => {
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            let text = String::from_utf8_lossy(&bytes[..end]);
            out.push_str(",\"text\":");
            write_json_str(&mut out, &text);
        }
        MemFmt::Raw => unreachable!(),
    }
    out.push('}');
    InspectReply::Json(out)
}

fn inspect_mem_map(machine: &Machine) -> String {
    let mut out = String::with_capacity(1024);
    let _ = write!(
        out,
        "{{\"ok\":true,\"arena_bytes\":{},\"image\":",
        machine.arena.len()
    );
    match &machine.image {
        Some(img) => {
            let _ = write!(
                out,
                "{{\"base\":\"{:#010x}\",\"base_u32\":{},\"preferred_base\":\"{:#010x}\",\"size\":{},\"entry\":\"{:#010x}\",\"sections\":[",
                img.base,
                img.base,
                img.preferred_base,
                img.size,
                img.base.wrapping_add(img.entry)
            );
            for (i, sec) in img.sections.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str("{\"name\":");
                write_json_str(&mut out, &sec.name);
                let _ = write!(
                    out,
                    ",\"addr\":\"{:#010x}\",\"addr_u32\":{},\"size\":{}}}",
                    sec.address, sec.address, sec.size
                );
            }
            out.push_str("]}");
        }
        None => out.push_str("null"),
    }

    out.push_str(",\"modules\":[");
    for (i, m) in machine.modules.modules().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_json_str(&mut out, &m.name);
        let _ = write!(
            out,
            ",\"base\":\"{:#010x}\",\"base_u32\":{},\"size\":{},\"entry\":\"{:#010x}\"}}",
            m.base, m.base, m.size, m.entry
        );
    }

    let heap_extent = machine.state.heap.extent();
    let heap_live = machine.state.heap.live_bytes();
    let esp = machine.cpu.r[ESP];
    let stack_bottom = STACK_TOP - STACK_BYTES;
    let _ = write!(
        out,
        "],\"heap\":{{\"base\":\"{HEAP_BASE:#010x}\",\"base_u32\":{HEAP_BASE},\"capacity\":{HEAP_BYTES},\"extent\":{heap_extent},\"live_bytes\":{heap_live},\"end\":\"{:#010x}\"}},\"stack\":{{\"bottom\":\"{stack_bottom:#010x}\",\"top\":\"{STACK_TOP:#010x}\",\"size\":{STACK_BYTES},\"esp\":\"{esp:#010x}\",\"esp_u32\":{esp}}},\"tib\":\"{TIB:#010x}\",\"peb\":\"{PEB:#010x}\",\"claimed\":[",
        HEAP_BASE.wrapping_add(heap_extent)
    );
    for (i, &(start, end)) in machine.state.claimed.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"start\":\"{start:#010x}\",\"end\":\"{end:#010x}\",\"size\":{}}}",
            end.saturating_sub(start)
        );
    }
    out.push_str("],\"reserved\":[");
    for (i, (&base, &(size, protect))) in machine.state.reserved.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"base\":\"{base:#010x}\",\"size\":{size},\"protect\":\"{protect:#x}\"}}"
        );
    }
    let _ = write!(
        out,
        "],\"committed_pages\":{},\"surface\":",
        machine.state.committed.len()
    );
    match machine.surface() {
        Some(s) => {
            let _ = write!(
                out,
                "{{\"addr\":\"{:#010x}\",\"addr_u32\":{},\"width\":{},\"height\":{},\"pitch\":{}}}",
                s.addr, s.addr, s.width, s.height, s.pitch
            );
        }
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

fn active_search_regions(machine: &Machine) -> Vec<(u32, usize)> {
    let mut regions = Vec::new();
    if let Some(img) = &machine.image {
        regions.push((img.base, img.size as usize));
    }
    for m in machine.modules.modules() {
        regions.push((m.base, m.size as usize));
    }
    for &(start, end) in &machine.state.claimed {
        regions.push((start, end.saturating_sub(start) as usize));
    }
    let extent = machine.state.heap.extent() as usize;
    if extent > 0 {
        regions.push((HEAP_BASE, extent));
    }
    regions.push((STACK_TOP - STACK_BYTES, STACK_BYTES as usize));
    regions.push((TIB, (PEB + 0x1000 - TIB) as usize));
    if let Some(s) = machine.surface() {
        regions.push((s.addr, (s.pitch * s.height) as usize));
    }
    regions.retain(|&(_, len)| len > 0);
    regions.sort_by_key(|&(start, _)| start);
    regions
}

fn inspect_mem_search(
    machine: &Machine,
    needle: &[u8],
    start: Option<u32>,
    end: Option<u32>,
    limit: usize,
) -> InspectReply {
    if needle.is_empty() {
        return InspectReply::Error {
            status: 400,
            message: "search pattern must not be empty".to_owned(),
        };
    }
    let limit = limit.clamp(1, MAX_MEM_SEARCH_LIMIT);

    let ranges: Vec<(u32, usize)> = match (start, end) {
        (Some(s), Some(e)) => {
            if e <= s {
                return InspectReply::Error {
                    status: 400,
                    message: "search `end` must be > `start`".to_owned(),
                };
            }
            let span = (e - s) as usize;
            if span > MAX_MEM_SEARCH_SPAN {
                return InspectReply::Error {
                    status: 400,
                    message: format!("explicit search span exceeds {MAX_MEM_SEARCH_SPAN} bytes"),
                };
            }
            vec![(s, span)]
        }
        (Some(s), None) => {
            let max_end = u32::try_from(machine.arena.len()).unwrap_or(u32::MAX);
            if s >= max_end {
                return InspectReply::Error {
                    status: 400,
                    message: "search `start` is outside arena".to_owned(),
                };
            }
            let span = ((max_end - s) as usize).min(MAX_MEM_SEARCH_SPAN);
            vec![(s, span)]
        }
        (None, Some(e)) => {
            let mut filtered = Vec::new();
            for (rs, rlen) in active_search_regions(machine) {
                if rs >= e {
                    continue;
                }
                let rend = rs.saturating_add(rlen as u32).min(e);
                if rend > rs {
                    filtered.push((rs, (rend - rs) as usize));
                }
            }
            filtered
        }
        (None, None) => active_search_regions(machine),
    };

    let mut matches = Vec::new();
    let mut truncated = false;
    let nlen = needle.len();
    let first = needle[0];

    'outer: for (base, len) in ranges {
        let Ok(slice) = machine.arena.slice(base, len) else {
            continue;
        };
        if slice.len() < nlen {
            continue;
        }
        let mut i = 0usize;
        let last = slice.len() - nlen;
        while i <= last {
            if slice[i] == first && &slice[i..i + nlen] == needle {
                if matches.len() >= limit {
                    truncated = true;
                    break 'outer;
                }
                matches.push(base.wrapping_add(i as u32));
            }
            i += 1;
        }
    }

    let mut out = String::with_capacity(64 + matches.len() * 24);
    let _ = write!(
        out,
        "{{\"ok\":true,\"count\":{},\"truncated\":{truncated},\"matches\":[",
        matches.len()
    );
    for (i, &addr) in matches.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{addr:#010x}\"");
    }
    out.push_str("],\"addresses\":[");
    for (i, &addr) in matches.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{addr}");
    }
    out.push_str("]}");
    InspectReply::Json(out)
}

fn inspect_cpu(machine: &Machine) -> String {
    let cpu = &machine.cpu;
    let mut out = String::with_capacity(1024);
    out.push_str("{\"ok\":true,\"regs\":{");
    for (i, name) in REG_NAMES.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{name}\":{}", cpu.r[i]);
    }
    let eflags = cpu.flags.to_eflags();
    let _ = write!(
        out,
        ",\"eip\":{},\"eflags\":{eflags}}},\"regs_hex\":{{",
        cpu.eip
    );
    for (i, name) in REG_NAMES.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{name}\":\"{:#010x}\"", cpu.r[i]);
    }
    let _ = write!(
        out,
        ",\"eip\":\"{:#010x}\",\"eflags\":\"{eflags:#010x}\"}},\"seg\":{{",
        cpu.eip
    );
    for (i, name) in SEG_NAMES.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{name}\":{}", cpu.seg[i]);
    }
    out.push_str("},\"seg_base\":{");
    for (i, name) in SEG_NAMES.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{name}\":{}", cpu.seg_base[i]);
    }

    let fpu = &cpu.fpu;
    let status = fpu.status();
    let control = fpu.control();
    let top = (status >> 11) & 7;
    let mut used_mask = 0u8;
    for i in 0..8usize {
        if fpu.occupied(i) {
            let phys = (usize::from(top) + i) & 7;
            used_mask |= 1u8 << phys;
        }
    }
    let _ = write!(
        out,
        "}},\"tsc\":{},\"x87\":{{\"top\":{top},\"used\":{used_mask},\"control\":{control},\"control_hex\":\"{control:#06x}\",\"status\":{status},\"status_hex\":\"{status:#06x}\",\"st\":[",
        cpu.tsc
    );
    for i in 0..8usize {
        if i > 0 {
            out.push(',');
        }
        let val = fpu.st(i);
        let occ = fpu.occupied(i);
        let _ = write!(out, "{{\"index\":{i},\"occupied\":{occ},\"value\":");
        write_json_f64_or_null(&mut out, val);
        let _ = write!(out, ",\"bits\":\"{:#018x}\"}}", val.to_bits());
    }

    out.push_str("]},\"threads\":[");
    for (idx, t) in machine.state.threads.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let active = idx == machine.state.thread;
        let t_cpu = if active { cpu } else { &t.cpu };
        let _ = write!(
            out,
            "{{\"index\":{idx},\"id\":{},\"handle\":{},\"active\":{active},\"suspended\":{},\"ended\":{},\"time_critical\":{},\"eip\":\"{:#010x}\",\"esp\":\"{:#010x}\",\"wake_at\":{},\"waiting_on\":[",
            t.id,
            t.handle,
            t.suspended,
            t.ended,
            t.time_critical,
            t_cpu.eip,
            t_cpu.r[ESP],
            t.wake_at
        );
        for (wi, &h) in t.waiting_on.iter().enumerate() {
            if wi > 0 {
                out.push(',');
            }
            let _ = write!(out, "{h}");
        }
        out.push_str("]}");
    }

    let digest = machine.state_digest();
    let _ = write!(
        out,
        "],\"clock\":{{\"hz\":{},\"cycles\":{},\"declared_ms\":{},\"declared_nanos\":{}}},\"cursor\":{},\"flips\":{},\"instructions_passed\":{},\"state_digest\":",
        machine.clock.hz(),
        machine.clock.cycles(),
        machine.clock.millis(),
        machine.clock.nanos(),
        machine.clock.cursor(),
        machine.flips(),
        machine.instructions_passed()
    );
    write_json_str(&mut out, &digest);
    out.push('}');
    out
}

fn inspect_win32(machine: &Machine) -> String {
    let st = &machine.state;
    let declared_ms = machine.clock.millis();
    let mut out = String::with_capacity(2048);

    // Windows & messages.
    let _ = write!(
        out,
        "{{\"ok\":true,\"window\":{{\"made\":{},\"procedure\":\"{:#010x}\",\"thread\":{},\"style\":\"{:#010x}\",\"instance\":\"{:#010x}\",\"message_queue_len\":{},\"timer_count\":{},\"modal_dialog\":{},\"quit\":",
        st.window_made,
        st.window_procedure,
        st.window_thread,
        st.window_style,
        st.window_instance,
        st.messages.len(),
        st.timers.len(),
        st.dialog.is_some()
    );
    match st.quit {
        Some((t, code)) => {
            let _ = write!(out, "{{\"thread\":{t},\"code\":{code}}}");
        }
        None => out.push_str("null"),
    }

    // Cursor & input.
    let _ = write!(
        out,
        "}},\"input\":{{\"pointer_pos\":[{},{}],\"last_move\":[{},{}],\"pointer_shown\":{},\"cursor_show_count\":{},\"cursor_handle\":",
        st.pointer.0,
        st.pointer.1,
        st.last_move.0,
        st.last_move.1,
        st.cursor_shown >= 0,
        st.cursor_shown
    );
    match st.cursor {
        Some(c) => {
            let _ = write!(out, "\"{c:#x}\"");
        }
        None => out.push_str("null"),
    }
    let _ = write!(
        out,
        ",\"cursor_shapes_loaded\":{},\"mouse_left_down\":{},\"mouse_right_down\":{},\"clip_rect\":",
        st.cursors.len(),
        st.mouse_down,
        st.right_down
    );
    match st.clip {
        Some([l, t, r, b]) => {
            let _ = write!(out, "[{l},{t},{r},{b}]");
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"held_keys\":[");
    for (i, &k) in st.keys_down.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{k}");
    }

    // Display & DirectDraw surfaces / palettes.
    let _ = write!(
        out,
        "]}},\"display\":{{\"width\":{},\"height\":{},\"bpp\":{},\"palette\":\"{:#010x}\"}},\"ddraw\":{{\"surfaces\":[",
        st.display_mode.0, st.display_mode.1, st.display_mode.2, st.palette
    );
    let mut surface_count = 0usize;
    let mut palettes = Vec::new();
    for (addr, obj) in st.com_objects() {
        match &obj.state {
            ComState::Surface {
                pixels,
                width,
                height,
                pitch,
                format,
                primary,
                caps,
                back,
                palette,
            } => {
                if surface_count > 0 {
                    out.push(',');
                }
                surface_count += 1;
                let _ = write!(
                    out,
                    "{{\"addr\":\"{addr:#010x}\",\"pixels\":\"{pixels:#010x}\",\"width\":{width},\"height\":{height},\"pitch\":{pitch},\"bpp\":{},\"primary\":{primary},\"caps\":\"{caps:#x}\",\"back\":\"{back:#010x}\",\"palette\":\"{palette:#010x}\"}}",
                    format.bits
                );
            }
            ComState::Palette { store } => {
                palettes.push((addr, *store));
            }
            _ => {}
        }
    }
    out.push_str("],\"palettes\":[");
    for (i, &(addr, store)) in palettes.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"addr\":\"{addr:#010x}\",\"store\":\"{store:#010x}\"}}"
        );
    }
    let _ = write!(
        out,
        "],\"clippers_count\":{},\"surface_dcs_count\":{}}}",
        st.clip_lists.len(),
        st.surface_dcs.len()
    );

    // Direct3D devices & states.
    let d3d_devs = st.immediate.device_summaries();
    let _ = write!(
        out,
        ",\"d3d\":{{\"device_count\":{},\"materials_count\":{},\"viewports_count\":{},\"devices\":[",
        d3d_devs.len(),
        st.immediate.materials_count(),
        st.immediate.viewports_count()
    );
    for (i, d) in d3d_devs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"addr\":\"{:#010x}\",\"in_scene\":{},\"current_viewport\":\"{:#010x}\",\"render_states_set\":{},\"active_textures\":[",
            d.addr, d.in_scene, d.current_viewport, d.render_states_set
        );
        for (ti, &(stage, tex)) in d.active_textures.iter().enumerate() {
            if ti > 0 {
                out.push(',');
            }
            let _ = write!(out, "{{\"stage\":{stage},\"texture\":\"{tex:#010x}\"}}");
        }
        out.push_str("]}");
    }

    // DirectSound active voices & mixer.
    let active_voices = st.sound.active_buffers(declared_ms);
    let _ = write!(
        out,
        "]}},\"dsound\":{{\"buffer_count\":{},\"active_voices_count\":{},\"mixer_active_voices\":{},\"active_voices\":[",
        st.sound.buffer_count(),
        active_voices.len(),
        st.mixer.active_voice_count()
    );
    for (i, v) in active_voices.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"addr\":\"{:#010x}\",\"bytes\":{},\"volume\":{},\"pan\":{},\"looping\":{},\"frequency\":",
            v.addr, v.bytes, v.volume, v.pan, v.looping
        );
        match v.frequency {
            Some(f) => {
                let _ = write!(out, "{f}");
            }
            None => out.push_str("null"),
        }
        out.push('}');
    }

    // MIDI & CD audio.
    let midi_has_synth = st.mixer.midi_out();
    let midi_open = st
        .midi
        .as_ref()
        .is_some_and(crate::win32::winmm::Midi::has_stream);
    let midi_playing = st
        .midi
        .as_ref()
        .is_some_and(crate::win32::winmm::Midi::is_playing);
    let midi_queued = st
        .midi
        .as_ref()
        .map_or(0, crate::win32::winmm::Midi::queued_buffers);
    let midi_tempo = st.midi.as_ref().and_then(crate::win32::winmm::Midi::tempo);
    let _ = write!(
        out,
        "]}},\"midi\":{{\"has_synth\":{midi_has_synth},\"open\":{midi_open},\"playing\":{midi_playing},\"queued_buffers\":{midi_queued},\"tempo\":"
    );
    match midi_tempo {
        Some(t) => {
            let _ = write!(out, "{t}");
        }
        None => out.push_str("null"),
    }

    let cd = st.cd.status_summary(declared_ms);
    let _ = write!(
        out,
        "}},\"cd\":{{\"open\":{},\"playing\":{},\"tracks_mode\":{},\"track_count\":{},\"sector\":",
        cd.open, cd.playing, cd.tracks_mode, cd.track_count
    );
    match cd.sector {
        Some(s) => {
            let _ = write!(out, "{s}");
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"current_track\":");
    match cd.current_track {
        Some(t) => {
            let _ = write!(out, "{t}");
        }
        None => out.push_str("null"),
    }

    // Open handles & files summary.
    let mut total_open = 0usize;
    let mut modules = 0usize;
    let mut heaps = 0usize;
    let mut named = 0usize;
    let mut semaphores = 0usize;
    let mut mappings = 0usize;
    let mut keys = 0usize;
    let mut mmio = 0usize;
    let mut find = 0usize;
    let mut other = 0usize;
    let mut open_files = Vec::new();
    for (handle, obj) in st.open_handles() {
        total_open += 1;
        match obj {
            Object::File {
                path,
                contents,
                at,
                wrote,
                ..
            } => open_files.push((handle, path.as_str(), contents.size(), *at, *wrote)),
            Object::Module { .. } => modules += 1,
            Object::Heap => heaps += 1,
            Object::Named { .. } => named += 1,
            Object::Semaphore { .. } => semaphores += 1,
            Object::Mapping { .. } => mappings += 1,
            Object::Key { .. } => keys += 1,
            Object::Mmio { .. } => mmio += 1,
            Object::Find { .. } => find += 1,
            Object::Std(_) | Object::Hook(_) | Object::MidiStream => other += 1,
        }
    }
    let _ = write!(
        out,
        "}},\"handles\":{{\"total_open\":{total_open},\"files\":[",
    );
    for (i, &(handle, path, size, pos, wrote)) in open_files.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{{\"handle\":\"{handle:#x}\",\"path\":");
        write_json_str(&mut out, path);
        let _ = write!(out, ",\"size\":{size},\"pos\":{pos},\"wrote\":{wrote}}}");
    }
    let _ = write!(
        out,
        "],\"counts\":{{\"files\":{},\"modules\":{modules},\"heaps\":{heaps},\"named\":{named},\"semaphores\":{semaphores},\"mappings\":{mappings},\"keys\":{keys},\"mmio\":{mmio},\"find\":{find},\"other\":{other}}}}}}}",
        open_files.len()
    );
    out
}

/// Safely resolve a relative path inside `root_dir`, rejecting any `..`, NUL,
/// drive prefix, or root escape.
pub(crate) fn safe_join_profile(root_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    if rel.contains('\0') {
        return Err("path contains NUL byte".to_owned());
    }
    let normalized = rel.replace('\\', "/");
    let trimmed = normalized.trim_start_matches('/');
    let mut out = root_dir.to_path_buf();
    for comp in Path::new(trimmed).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path traversal outside profile root is forbidden".to_owned());
            }
        }
    }
    Ok(out)
}

fn validate_disc_path(path: &str) -> Result<String, String> {
    if path.contains('\0') {
        return Err("path contains NUL byte".to_owned());
    }
    let normalized = path.replace('\\', "/");
    for seg in normalized.split('/') {
        if seg == ".." {
            return Err("path traversal `..` is forbidden".to_owned());
        }
    }
    Ok(normalized)
}

fn inspect_files_list(machine: &Machine, root: FileRoot, path: &str) -> InspectReply {
    match root {
        FileRoot::Profile => {
            let Some(profile_dir) = machine.tree.profile_dir() else {
                return InspectReply::Json(
                    "{\"ok\":true,\"root\":\"profile\",\"path\":\"\",\"entries\":[]}".to_owned(),
                );
            };
            let target = match safe_join_profile(profile_dir, path) {
                Ok(p) => p,
                Err(message) => {
                    return InspectReply::Error {
                        status: 400,
                        message,
                    }
                }
            };
            if !target.exists() {
                return InspectReply::Json(format_files_json("profile", path, &[]));
            }
            if !target.is_dir() {
                return InspectReply::Error {
                    status: 400,
                    message: "path is not a directory".to_owned(),
                };
            }
            let Ok(read_dir) = std::fs::read_dir(&target) else {
                return InspectReply::Error {
                    status: 500,
                    message: "could not list profile directory".to_owned(),
                };
            };
            let mut entries = Vec::new();
            for item in read_dir.flatten() {
                let name = item.file_name().to_string_lossy().into_owned();
                let item_path = item.path();
                let is_dir = item_path.is_dir();
                let size = item.metadata().map(|m| m.len()).unwrap_or(0);
                let sha256 = if !is_dir && size <= 16 * 1024 * 1024 {
                    std::fs::read(&item_path)
                        .ok()
                        .map(|b| crate::hash::sha256_hex(&b))
                } else {
                    None
                };
                let rel_path = if path.is_empty() {
                    name.clone()
                } else {
                    format!("{}/{}", path.trim_end_matches(['/', '\\']), name)
                };
                entries.push(FileEntry {
                    name,
                    path: rel_path,
                    is_dir,
                    size,
                    kind: if is_dir { "dir" } else { "profile" },
                    sha256,
                });
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            InspectReply::Json(format_files_json("profile", path, &entries))
        }
        FileRoot::Disc => {
            let clean = match validate_disc_path(path) {
                Ok(p) => p,
                Err(message) => {
                    return InspectReply::Error {
                        status: 400,
                        message,
                    }
                }
            };
            let trimmed = clean.trim_matches('/');
            // If caller asks for `C:...` or `"installed"` or when no ISO is mounted (e.g. stub),
            // list the declared install tree.
            let upper = trimmed.to_ascii_uppercase();
            if upper == "INSTALLED" || upper.starts_with("C:") || machine.tree.disc_path().is_none()
            {
                let prefix = if upper == "INSTALLED" || trimmed.is_empty() {
                    String::new()
                } else {
                    upper.replace('/', "\\")
                };
                let mut entries: Vec<FileEntry> = machine
                    .tree
                    .declared_files()
                    .into_iter()
                    .filter(|(p, _, _)| prefix.is_empty() || p.starts_with(&prefix))
                    .map(|(p, kind, size)| FileEntry {
                        name: p.rsplit('\\').next().unwrap_or(&p).to_owned(),
                        path: p,
                        is_dir: false,
                        size: u64::from(size),
                        kind,
                        sha256: None,
                    })
                    .collect();
                entries.sort_by(|a, b| a.path.cmp(&b.path));
                return InspectReply::Json(format_files_json("disc", trimmed, &entries));
            }

            let Some(iso_path) = machine.tree.disc_path() else {
                return InspectReply::Json(format_files_json("disc", trimmed, &[]));
            };
            let mut iso = match Iso::open(iso_path) {
                Ok(iso) => iso,
                Err(e) => {
                    return InspectReply::Error {
                        status: 500,
                        message: e.to_string(),
                    };
                }
            };
            if machine.title().declared.joliet {
                let _ = iso.read_joliet();
            }
            let iso_dir = trimmed
                .strip_prefix("D:/")
                .or_else(|| trimmed.strip_prefix("d:/"))
                .or_else(|| trimmed.strip_prefix("D:"))
                .or_else(|| trimmed.strip_prefix("d:"))
                .unwrap_or(trimmed);
            let listed = match iso.list(iso_dir) {
                Ok(list) => list,
                Err(e) => {
                    return InspectReply::Error {
                        status: 404,
                        message: e.to_string(),
                    };
                }
            };
            let entries: Vec<FileEntry> = listed
                .into_iter()
                .map(|e| {
                    let rel = if iso_dir.is_empty() {
                        e.name.clone()
                    } else {
                        format!("{}/{}", iso_dir.trim_end_matches('/'), e.name)
                    };
                    FileEntry {
                        name: e.name,
                        path: rel,
                        is_dir: e.is_dir,
                        size: u64::from(e.len),
                        kind: if e.is_dir { "dir" } else { "disc" },
                        sha256: None,
                    }
                })
                .collect();
            InspectReply::Json(format_files_json("disc", trimmed, &entries))
        }
    }
}

fn inspect_files_read(machine: &Machine, root: FileRoot, path: &str) -> InspectReply {
    if path.trim().is_empty() {
        return InspectReply::Error {
            status: 400,
            message: "missing `path` parameter".to_owned(),
        };
    }
    match root {
        FileRoot::Profile => {
            let Some(profile_dir) = machine.tree.profile_dir() else {
                return InspectReply::Error {
                    status: 404,
                    message: "no profile directory configured".to_owned(),
                };
            };
            let target = match safe_join_profile(profile_dir, path) {
                Ok(p) => p,
                Err(message) => {
                    return InspectReply::Error {
                        status: 400,
                        message,
                    }
                }
            };
            if !target.is_file() {
                return InspectReply::Error {
                    status: 404,
                    message: format!("profile file not found: {path}"),
                };
            }
            let size = target.metadata().map(|m| m.len()).unwrap_or(0);
            if size > MAX_FILE_READ_LEN as u64 {
                return InspectReply::Error {
                    status: 413,
                    message: format!("file exceeds {MAX_FILE_READ_LEN} byte limit"),
                };
            }
            match std::fs::read(&target) {
                Ok(bytes) => {
                    let sha = crate::hash::sha256_hex(&bytes);
                    InspectReply::Raw {
                        content_type: "application/octet-stream",
                        bytes,
                        sha256: Some(sha),
                    }
                }
                Err(e) => InspectReply::Error {
                    status: 500,
                    message: e.to_string(),
                },
            }
        }
        FileRoot::Disc => {
            let clean = match validate_disc_path(path) {
                Ok(p) => p,
                Err(message) => {
                    return InspectReply::Error {
                        status: 400,
                        message,
                    }
                }
            };
            let joliet = machine.title().declared.joliet;
            match machine.tree.read_readonly(&clean, joliet) {
                Some(bytes) if bytes.len() <= MAX_FILE_READ_LEN => {
                    let sha = crate::hash::sha256_hex(&bytes);
                    InspectReply::Raw {
                        content_type: "application/octet-stream",
                        bytes,
                        sha256: Some(sha),
                    }
                }
                Some(_) => InspectReply::Error {
                    status: 413,
                    message: format!("file exceeds {MAX_FILE_READ_LEN} byte limit"),
                },
                None => InspectReply::Error {
                    status: 404,
                    message: format!("disc file not found: {path}"),
                },
            }
        }
    }
}

struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    kind: &'static str,
    sha256: Option<String>,
}

fn format_files_json(root: &str, path: &str, entries: &[FileEntry]) -> String {
    let mut out = String::with_capacity(128 + entries.len() * 96);
    out.push_str("{\"ok\":true,\"root\":");
    write_json_str(&mut out, root);
    out.push_str(",\"path\":");
    write_json_str(&mut out, path);
    let _ = write!(out, ",\"count\":{},\"entries\":[", entries.len());
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"name\":");
        write_json_str(&mut out, &e.name);
        out.push_str(",\"path\":");
        write_json_str(&mut out, &e.path);
        let _ = write!(
            out,
            ",\"is_dir\":{},\"size\":{},\"kind\":",
            e.is_dir, e.size
        );
        write_json_str(&mut out, e.kind);
        out.push_str(",\"sha256\":");
        match &e.sha256 {
            Some(s) => write_json_str(&mut out, s),
            None => out.push_str("null"),
        }
        out.push('}');
    }
    out.push_str("]}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::json::parse_json;
    use crate::host::headless::Headless;
    use crate::Title;

    #[test]
    fn inspect_reads_mem_cpu_win32_files_and_blocks_path_traversal_readonly() {
        let mut host = Headless::new();
        let mut machine = Machine::new(&Title::STUB, &mut host).expect("machine");
        machine.load_stub().expect("stub");
        machine
            .arena
            .write(0x0010_0100, b"HELLO\0\x78\x56\x34\x12")
            .unwrap();
        machine
            .tree
            .declare("C:\\STUB\\DATA.TXT", b"brick".to_vec());

        // Set up a temporary profile dir under Tabletop/.tmp or temp_dir.
        let tmp = std::env::temp_dir().join(format!("lego-inspect-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("SAVE.DAT"), b"saved-game").unwrap();
        machine.tree.set_profile(tmp.clone());

        let before_digest = machine.state_digest();

        // /api/mem (hex, cstr, u32, raw)
        let InspectReply::Json(hex_json) = execute_inspect(
            &machine,
            InspectQuery::MemRead {
                addr: 0x0010_0100,
                len: 10,
                fmt: MemFmt::Hex,
            },
        ) else {
            panic!("expected json");
        };
        assert!(hex_json.contains("48454c4c4f0078563412"));

        let InspectReply::Json(cstr_json) = execute_inspect(
            &machine,
            InspectQuery::MemRead {
                addr: 0x0010_0100,
                len: 16,
                fmt: MemFmt::CStr,
            },
        ) else {
            panic!("expected json");
        };
        assert_eq!(
            parse_json(&cstr_json).unwrap().as_object().unwrap()["text"].as_str(),
            Some("HELLO")
        );

        let InspectReply::Json(u32_json) = execute_inspect(
            &machine,
            InspectQuery::MemRead {
                addr: 0x0010_0106,
                len: 4,
                fmt: MemFmt::U32,
            },
        ) else {
            panic!("expected json");
        };
        assert_eq!(
            parse_json(&u32_json).unwrap().as_object().unwrap()["values"]
                .as_array()
                .unwrap()[0]
                .as_u64(),
            Some(0x1234_5678)
        );

        // /api/mem/map
        let InspectReply::Json(map_json) = execute_inspect(&machine, InspectQuery::MemMap) else {
            panic!("expected json");
        };
        let map_obj = parse_json(&map_json).unwrap();
        assert!(map_obj.as_object().unwrap()["heap"].as_object().is_some());
        assert!(map_obj.as_object().unwrap()["stack"].as_object().is_some());

        // /api/mem/search
        let InspectReply::Json(search_json) = execute_inspect(
            &machine,
            InspectQuery::MemSearch {
                needle: b"HELLO".to_vec(),
                start: Some(0x0010_0000),
                end: Some(0x0010_1000),
                limit: 16,
            },
        ) else {
            panic!("expected json");
        };
        let s_obj = parse_json(&search_json).unwrap();
        assert_eq!(s_obj.as_object().unwrap()["count"].as_u64(), Some(1));
        assert_eq!(
            s_obj.as_object().unwrap()["addresses"].as_array().unwrap()[0].as_u64(),
            Some(0x0010_0100)
        );

        // /api/cpu & /api/win32
        let InspectReply::Json(cpu_json) = execute_inspect(&machine, InspectQuery::Cpu) else {
            panic!("expected json");
        };
        assert_eq!(
            parse_json(&cpu_json).unwrap().as_object().unwrap()["state_digest"].as_str(),
            Some(before_digest.as_str())
        );
        let InspectReply::Json(win32_json) = execute_inspect(&machine, InspectQuery::Win32) else {
            panic!("expected json");
        };
        assert!(
            parse_json(&win32_json).unwrap().as_object().unwrap()["ddraw"]
                .as_object()
                .is_some()
        );

        // /api/files & /api/files/read (profile + disc + traversal refusal)
        let InspectReply::Json(plist) = execute_inspect(
            &machine,
            InspectQuery::FilesList {
                root: FileRoot::Profile,
                path: String::new(),
            },
        ) else {
            panic!("expected json");
        };
        assert!(plist.contains("SAVE.DAT"));

        let InspectReply::Raw { bytes, sha256, .. } = execute_inspect(
            &machine,
            InspectQuery::FilesRead {
                root: FileRoot::Profile,
                path: "SAVE.DAT".to_owned(),
            },
        ) else {
            panic!("expected raw bytes");
        };
        assert_eq!(bytes, b"saved-game");
        assert_eq!(
            sha256.as_deref(),
            Some(crate::hash::sha256_hex(b"saved-game").as_str())
        );

        let InspectReply::Raw { bytes: disc_b, .. } = execute_inspect(
            &machine,
            InspectQuery::FilesRead {
                root: FileRoot::Disc,
                path: "C:\\STUB\\DATA.TXT".to_owned(),
            },
        ) else {
            panic!("expected disc raw bytes");
        };
        assert_eq!(disc_b, b"brick");

        assert!(matches!(
            execute_inspect(
                &machine,
                InspectQuery::FilesRead {
                    root: FileRoot::Profile,
                    path: "../etc/passwd".to_owned(),
                }
            ),
            InspectReply::Error { status: 400, .. }
        ));
        assert!(matches!(
            execute_inspect(
                &machine,
                InspectQuery::FilesRead {
                    root: FileRoot::Disc,
                    path: "../../secret".to_owned(),
                }
            ),
            InspectReply::Error { status: 400, .. }
        ));

        // Prove zero guest state mutation.
        assert_eq!(machine.state_digest(), before_digest);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
