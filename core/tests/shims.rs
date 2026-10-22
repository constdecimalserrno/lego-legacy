//! What KERNEL32, USER32 and retained mode answer, called as a title calls
//! them but without one: each shim or method run on the stub's machine with
//! the arguments a title would push, and what it wrote and answered read back
//! (#114, #117, #118, #119, #120, #144).

use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::Stop;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::{headless::Headless, Input};
use lego_legacy_core::loader::MODULE_BASE;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::raster::PixelFormat;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::title::Declared;
use lego_legacy_core::win32::{self, user32::WINDOW, Ctx};
use lego_legacy_core::{disc, hash, roster, testkit, Title};

/// Where the tests put what they pass and what is written back.
const DATA: u32 = 0x0050_0000;
const OUT: u32 = 0x0050_1000;

fn machine() -> Machine {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    Machine::new(&Title::STUB, &mut Headless::new()).expect("a machine")
}

/// Call `dll!name` with `args`, as the boundary would.
fn call(machine: &mut Machine, dll: &str, name: &str, args: &[u32]) -> Result<u32, Stop> {
    call_as(machine, &Title::STUB, dll, name, args)
}

/// As [`call`], on `title`'s declared machine.
fn call_as(
    machine: &mut Machine,
    title: &'static Title,
    dll: &str,
    name: &str,
    args: &[u32],
) -> Result<u32, Stop> {
    let shim = win32::find(dll, name).unwrap_or_else(|| panic!("{dll}!{name} is written"));
    as_title(machine, title, |ctx| (shim.run)(ctx, args))
}

/// Call the method in `slot` of `interface` on `this`, as a guest calling
/// through the object's table would.
fn com(
    machine: &mut Machine,
    interface: &str,
    slot: usize,
    this: u32,
    args: &[u32],
) -> Result<u32, Stop> {
    let method = win32::com_method(interface, slot)
        .unwrap_or_else(|| panic!("{interface} has a slot {slot}"));
    assert_eq!(
        method.args as usize,
        args.len(),
        "{interface}::{} at slot {slot}",
        method.name
    );
    with_ctx(machine, |ctx| (method.run)(ctx, this, args))
}

fn with_ctx<T>(machine: &mut Machine, run: impl FnOnce(&mut Ctx<'_>) -> T) -> T {
    as_title(machine, &Title::STUB, run)
}

/// As [`with_ctx`], on `title`'s declared machine.
fn as_title<T>(
    machine: &mut Machine,
    title: &'static Title,
    run: impl FnOnce(&mut Ctx<'_>) -> T,
) -> T {
    let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
    let mut ctx = Ctx {
        cpu: &mut machine.cpu,
        mem: &mut machine.arena,
        state: &mut machine.state,
        title,
        log: &mut log,
        tree: &mut machine.tree,
        modules: &mut machine.modules,
        clock_millis: 0,
        boundary: &mut machine.boundary,
        call_guest: None,
        float_answer: None,
        answer_is_the_call_s: false,
    };
    run(&mut ctx)
}

fn text(machine: &Machine, at: u32) -> String {
    let bytes = machine.arena.slice(at, 64).expect("in the arena");
    let end = bytes.iter().position(|b| *b == 0).expect("terminated");
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[test]
fn a_setting_win_ini_does_not_have_is_the_callers_default_cut_to_fit() {
    let mut m = machine();
    m.arena.write(DATA, b"LEGO Island\0").expect("in the arena");
    m.arena
        .write(DATA + 16, b"MediaPath\0")
        .expect("in the arena");
    m.arena
        .write(DATA + 32, b"D:\\LEGO  \0")
        .expect("in the arena");
    let (section, key, default) = (DATA, DATA + 16, DATA + 32);

    // Trailing blanks are not part of the answer.
    let n = call(
        &mut m,
        "KERNEL32",
        "GetProfileStringA",
        &[section, key, default, OUT, 0x100],
    );
    assert_eq!(n, Ok(7));
    assert_eq!(text(&m, OUT), "D:\\LEGO");

    // Too little room: as much as fits, terminated, and that much answered.
    let n = call(
        &mut m,
        "KERNEL32",
        "GetProfileStringA",
        &[section, key, default, OUT, 4],
    );
    assert_eq!(n, Ok(3));
    assert_eq!(text(&m, OUT), "D:\\");

    // A null default is an empty string, which is what LEGO Island is given.
    let n = call(
        &mut m,
        "KERNEL32",
        "GetProfileStringA",
        &[section, key, 0, OUT, 0x100],
    );
    assert_eq!(n, Ok(0));
    assert_eq!(text(&m, OUT), "");

    // A null key asks for the section's names, which is not written.
    assert!(matches!(
        call(
            &mut m,
            "KERNEL32",
            "GetProfileStringA",
            &[section, 0, default, OUT, 0x100]
        ),
        Err(Stop::NotWritten { .. })
    ));
}

#[test]
fn a_window_style_and_instance_are_the_ones_it_was_made_with() {
    const GWL_STYLE: u32 = -16i32 as u32;
    const GWL_HINSTANCE: u32 = -6i32 as u32;
    const GWL_USERDATA: u32 = -21i32 as u32;
    const WS_CAPTION_SYSMENU: u32 = 0x00c8_0000;
    let mut m = machine();

    // No window yet, so nothing to ask about.
    assert!(call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_STYLE]).is_err());

    m.arena.write(DATA, b"LEGO\0").expect("in the arena");
    let made = call(
        &mut m,
        "USER32",
        "CreateWindowExA",
        &[
            0x0004_0000, // WS_EX_APPWINDOW
            DATA,
            DATA,
            WS_CAPTION_SYSMENU,
            0,
            0,
            641,
            481,
            0,
            0,
            0x0040_0000,
            0,
        ],
    );
    assert_eq!(made, Ok(WINDOW));
    assert_eq!(
        call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_STYLE]),
        Ok(WS_CAPTION_SYSMENU)
    );
    assert_eq!(
        call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_HINSTANCE]),
        Ok(0x0040_0000)
    );
    // Any other word is not answered.
    assert!(call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_USERDATA]).is_err());
}

/// A pointer confined nowhere is confined to the whole declared screen, in
/// the mode the screen is in; one confined is confined to its rectangle, cut
/// to the screen, until it is freed; and with nowhere to say, the call fails,
/// as Microsoft documents `GetClipCursor` and `ClipCursor` (#147).
#[test]
fn the_pointer_s_clip_is_the_screen_until_it_is_confined() {
    let mut m = machine();
    let rect = |m: &Machine| -> Vec<u32> {
        (0..4)
            .map(|i| m.arena.read_u32(OUT + 4 * i).expect("in the arena"))
            .collect()
    };
    assert_eq!(call(&mut m, "USER32", "GetClipCursor", &[OUT]), Ok(1));
    assert_eq!(rect(&m), [0, 0, 640, 480]);
    m.state.display_mode = (800, 600, 16);
    assert_eq!(call(&mut m, "USER32", "GetClipCursor", &[OUT]), Ok(1));
    assert_eq!(rect(&m), [0, 0, 800, 600]);

    // LEGO Chess confines it to its screen less a pixel at 0x0044C43C.
    for (i, word) in [0u32, 0, 639, 479].into_iter().enumerate() {
        m.arena
            .write_u32(DATA + 4 * i as u32, word)
            .expect("in the arena");
    }
    assert_eq!(call(&mut m, "USER32", "ClipCursor", &[DATA]), Ok(1));
    assert_eq!(call(&mut m, "USER32", "GetClipCursor", &[OUT]), Ok(1));
    assert_eq!(rect(&m), [0, 0, 639, 479]);
    // A rectangle past the screen is cut to it.
    m.arena.write_u32(DATA + 8, 5000).expect("in the arena");
    assert_eq!(call(&mut m, "USER32", "ClipCursor", &[DATA]), Ok(1));
    assert_eq!(call(&mut m, "USER32", "GetClipCursor", &[OUT]), Ok(1));
    assert_eq!(rect(&m), [0, 0, 800, 479]);
    // Freed, it is the screen again.
    assert_eq!(call(&mut m, "USER32", "ClipCursor", &[0]), Ok(1));
    assert_eq!(call(&mut m, "USER32", "GetClipCursor", &[OUT]), Ok(1));
    assert_eq!(rect(&m), [0, 0, 800, 600]);

    assert_eq!(call(&mut m, "USER32", "GetClipCursor", &[0]), Ok(0));
}

/// Make the title's window, with its procedure at `procedure`.
fn window(m: &mut Machine, procedure: u32) {
    m.arena.write(DATA, b"LEGO\0").expect("in the arena");
    m.state.window_procedure = procedure;
    let mut made = [0; 12];
    made[1] = DATA;
    made[2] = DATA;
    assert_eq!(call(m, "USER32", "CreateWindowExA", &made), Ok(WINDOW));
}

/// A window destroyed is told twice, `WM_DESTROY` and then `WM_NCDESTROY`,
/// whatever its procedure answers the first, and is then gone: its waiting
/// messages with it, and its timers (#138), and a second destroy fails as one
/// of no window does (#140).
#[test]
fn a_destroyed_window_is_told_and_then_gone() {
    const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
    const PROCEDURE: u32 = 0x0040_1000;
    let mut m = machine();
    let destroy = |m: &mut Machine| {
        let shim = win32::find("USER32", "DestroyWindow").expect("written");
        with_ctx(m, |ctx| ((shim.run)(ctx, &[WINDOW]), ctx.call_guest.take()))
    };
    let last_error = |m: &mut Machine| call(m, "KERNEL32", "GetLastError", &[]);

    assert_eq!(destroy(&mut m), (Ok(0), None), "no window yet");
    assert_eq!(last_error(&mut m), Ok(ERROR_INVALID_WINDOW_HANDLE));

    window(&mut m, PROCEDURE);
    assert!(!m.state.messages.is_empty(), "WM_ACTIVATEAPP waits");
    let set_timer = |m: &mut Machine| call(m, "USER32", "SetTimer", &[WINDOW, 1, 1000, 0]);
    assert_eq!(set_timer(&mut m), Ok(1));
    let (answer, program) = destroy(&mut m);
    assert_eq!(answer, Ok(1));
    assert!(m.state.messages.is_empty(), "it went with the window");
    assert!(m.state.timers.is_empty(), "and its timer");
    assert!(set_timer(&mut m).is_err(), "a window that is gone has none");
    assert_eq!(call(&mut m, "USER32", "GetActiveWindow", &[]), Ok(0));

    // The program the machine runs calls the procedure with each message,
    // its arguments pushed last first, and never looks at what it answered.
    let (at, _) = program.expect("the procedure is called");
    let code = m.arena.slice(at, 64).expect("in the arena").to_vec();
    let sent = |message: u32| {
        let mut push = vec![0x68];
        push.extend_from_slice(&message.to_le_bytes());
        push.push(0x68);
        push.extend_from_slice(&WINDOW.to_le_bytes());
        push.push(0xb8);
        push.extend_from_slice(&PROCEDURE.to_le_bytes());
        code.windows(push.len()).position(|w| w == push)
    };
    let (first, last) = (sent(0x0002), sent(0x0082));
    assert!(
        first.is_some() && first < last,
        "WM_DESTROY, then WM_NCDESTROY"
    );
    assert!(
        !code.windows(2).any(|w| w == [0x85, 0xc0]),
        "no answer looked at"
    );

    assert_eq!(destroy(&mut m), (Ok(0), None), "gone already");
    assert_eq!(last_error(&mut m), Ok(ERROR_INVALID_WINDOW_HANDLE));
}

/// A person's close reaches the window as Alt-F4's does, `WM_SYSCOMMAND` with
/// `SC_CLOSE`, and `DefWindowProcA` answers that by sending the window
/// `WM_CLOSE` and answering what the window answers (#140).
#[test]
fn a_close_is_a_system_command_and_its_default_sends_wm_close() {
    const PROCEDURE: u32 = 0x0040_1000;
    let mut m = machine();
    window(&mut m, PROCEDURE);
    m.state.messages.clear();
    win32::user32::deliver(&mut m.state, Input::Close, 0);
    let close = m.state.messages.front().expect("waiting");
    assert_eq!((close.message, close.wparam), (0x0112, 0xf060));

    let default = win32::find("USER32", "DefWindowProcA").expect("written");
    let sent = with_ctx(&mut m, |ctx| {
        let answer = (default.run)(ctx, &[WINDOW, 0x0112, 0xf063, 0]);
        (answer, ctx.call_guest.take(), ctx.answer_is_the_call_s)
    });
    assert_eq!(
        sent,
        (Ok(0), Some((PROCEDURE, vec![WINDOW, 0x0010, 0, 0])), true),
        "the low four bits are the system's own"
    );
}

/// A thread that asked to quit is handed `WM_QUIT`, with the code it asked
/// with and no window, once what was posted to it is taken and before what a
/// person did, and once (#140). A timer that has come due is heard last of
/// all, once nothing else is waiting (#138).
#[test]
fn a_thread_that_asked_to_quit_is_handed_wm_quit_after_what_was_posted() {
    let mut m = machine();
    window(&mut m, 0x0040_1000);
    m.state.timers.push(win32::user32::Timer {
        window: WINDOW,
        id: 7,
        period: 1000,
        due: 0,
        callback: 0,
    });
    win32::user32::deliver(
        &mut m.state,
        Input::Key {
            code: 0x39,
            down: true,
        },
        0,
    );
    assert_eq!(call(&mut m, "USER32", "PostQuitMessage", &[3]), Ok(0));
    let mut taken = Vec::new();
    while call(&mut m, "USER32", "PeekMessageA", &[OUT, 0, 0, 0, 1]) == Ok(1) {
        let read = |o: u32| m.arena.read_u32(OUT + o).expect("in the arena");
        taken.push((read(0), read(4), read(8)));
    }
    assert_eq!(
        taken,
        [
            (WINDOW, 0x001c, 1),
            (WINDOW, 0x0006, 1),
            (WINDOW, 0x0007, 0),
            (0, 0x0012, 3),
            (WINDOW, 0x0100, 0x20),
            (WINDOW, 0x0113, 7)
        ],
        "WM_ACTIVATEAPP, WM_ACTIVATE and WM_SETFOCUS (#196), then WM_QUIT, then the key, then \
         WM_TIMER"
    );
}

#[test]
fn mouse_trails_go_off_and_nothing_else_is_answered() {
    const SPI_SETMOUSETRAILS: u32 = 0x005d;
    let mut m = machine();
    for off in [0, 1] {
        assert_eq!(
            call(
                &mut m,
                "USER32",
                "SystemParametersInfoA",
                &[SPI_SETMOUSETRAILS, off, 0, 0]
            ),
            Ok(1)
        );
    }
    // A trail of three is one this machine would have to draw.
    assert!(call(
        &mut m,
        "USER32",
        "SystemParametersInfoA",
        &[SPI_SETMOUSETRAILS, 3, 0, 0]
    )
    .is_err());
    // SPI_GETDESKWALLPAPER, which no title has asked.
    assert!(call(
        &mut m,
        "USER32",
        "SystemParametersInfoA",
        &[0x0073, 0, OUT, 0]
    )
    .is_err());
}

/// SmartHeap, which LEGO Island's engine links in, finds the header of any
/// block it handed out by masking its address with `0xffff_0000`. That only
/// finds it if every region VirtualAlloc gives it starts on Windows' 64 KiB
/// allocation granularity (#114, #356 in the old tree).
#[test]
fn a_new_region_starts_on_the_allocation_granularity() {
    const MEM_RESERVE: u32 = 0x2000;
    const PAGE_NOACCESS: u32 = 0x01;
    let mut m = machine();
    // An odd size first, so the next region would start off the boundary if
    // nothing put it back on.
    for size in [0x1_2345, 0x17_0000, 0x5_0000] {
        let at = call(
            &mut m,
            "KERNEL32",
            "VirtualAlloc",
            &[0, size, MEM_RESERVE, PAGE_NOACCESS],
        )
        .expect("answered");
        assert_ne!(at, 0, "{size:#x} bytes are reserved");
        assert_eq!(at % 0x1_0000, 0, "{at:#x} is not on a 64 KiB boundary");
    }
}

/// A region says how much of it, from an address, is committed and how, and
/// how much only reserved, as SmartHeap in LEGO Island's engine asks (#118).
#[test]
fn a_region_says_how_much_of_it_is_committed() {
    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_DECOMMIT: u32 = 0x4000;
    const MEM_RELEASE: u32 = 0x8000;
    const MEM_PRIVATE: u32 = 0x2_0000;
    const PAGE_NOACCESS: u32 = 0x01;
    const PAGE_READWRITE: u32 = 0x04;
    let mut m = machine();
    let base = call(
        &mut m,
        "KERNEL32",
        "VirtualAlloc",
        &[0, 0x1_0000, MEM_RESERVE, PAGE_NOACCESS],
    )
    .expect("answered");
    let commit = [base, 0x5000, MEM_COMMIT, PAGE_READWRITE];
    assert_eq!(call(&mut m, "KERNEL32", "VirtualAlloc", &commit), Ok(base));
    let mut query = |at: u32| {
        assert_eq!(
            call(&mut m, "KERNEL32", "VirtualQuery", &[at, OUT, 28]),
            Ok(28)
        );
        [0, 4, 8, 12, 16, 20, 24].map(|o| m.arena.read_u32(OUT + o).expect("in the arena"))
    };
    // Base, allocation base, its protection, size, state, protection, type.
    assert_eq!(
        query(base),
        [
            base,
            base,
            PAGE_NOACCESS,
            0x5000,
            MEM_COMMIT,
            PAGE_READWRITE,
            MEM_PRIVATE
        ]
    );
    // From inside a page, the run starts on that page.
    assert_eq!(
        query(base + 0x1234)[..5],
        [base + 0x1000, base, 1, 0x4000, MEM_COMMIT]
    );
    // What is only reserved runs to the region's end, with no protection.
    assert_eq!(query(base + 0x5000)[3..6], [0xb000, MEM_RESERVE, 0]);

    // A page given back ends the committed run before it.
    let decommit = [base + 0x4000, 0x1000, MEM_DECOMMIT];
    assert_eq!(call(&mut m, "KERNEL32", "VirtualFree", &decommit), Ok(1));
    assert_eq!(
        call(&mut m, "KERNEL32", "VirtualQuery", &[base, OUT, 28]),
        Ok(28)
    );
    assert_eq!(m.arena.read_u32(OUT + 12), Ok(0x4000));

    // A region let go of is no region at all.
    let release = [base, 0, MEM_RELEASE];
    assert_eq!(call(&mut m, "KERNEL32", "VirtualFree", &release), Ok(1));
    assert!(call(&mut m, "KERNEL32", "VirtualQuery", &[base, OUT, 28]).is_err());
}

/// The least a library can be and still load: headers and nothing else,
/// asking for the address the machine puts the first one at, so it is not
/// moved.
fn library() -> Vec<u8> {
    let mut bytes = vec![0u8; 0x200];
    let mut put = |at: usize, v: &[u8]| bytes[at..at + v.len()].copy_from_slice(v);
    put(0, b"MZ");
    put(0x3c, &0x40u32.to_le_bytes());
    put(0x40, b"PE\0\0");
    put(0x44, &0x014cu16.to_le_bytes());
    put(0x54, &0xe0u16.to_le_bytes());
    put(0x58, &0x010bu16.to_le_bytes());
    put(0x58 + 28, &MODULE_BASE.to_le_bytes());
    put(0x58 + 56, &0x1000u32.to_le_bytes());
    put(0x58 + 60, &0x200u32.to_le_bytes());
    bytes
}

/// A library is loaded when the title ships it, whatever case it is asked for
/// in and with ".DLL" added when it has no extension, and handed out when it
/// is one this machine writes. One the title declares its machine lacks is not
/// there, with Windows 98's error, and anything else is a wall, as LEGO
/// Chess's walk of its renderer plugins finds (#146).
#[test]
fn a_library_the_machine_does_not_have_is_not_found() {
    let mut m = machine();
    m.tree
        .declare("C:\\STUB\\KLIB_DirectX_Plugin.dll", library());
    let load = |m: &mut Machine, name: &str| {
        m.arena
            .write(DATA, format!("{name}\0").as_bytes())
            .expect("in the arena");
        call(m, "KERNEL32", "LoadLibraryA", &[DATA])
    };
    assert_eq!(load(&mut m, "KLIB_DirectX_PlugIn"), Ok(MODULE_BASE));
    assert_eq!(load(&mut m, "KLIB_DirectX_PlugIn.dll"), Ok(MODULE_BASE));
    for written in ["ddraw.dll", "DDraw.Dll"] {
        assert!(
            load(&mut m, written).is_ok_and(|module| module != 0),
            "{written}"
        );
    }
    assert!(win32::find("DDraw.Dll", "DirectDrawCreate").is_some());
    // Asked for by a path, it is known by its file, as its entry points are.
    let ddraw = load(&mut m, "C:\\WINDOWS\\SYSTEM\\DDRAW.DLL").expect("a handle");
    m.arena
        .write(OUT, b"DirectDrawCreate\0")
        .expect("in the arena");
    assert_ne!(
        call(&mut m, "KERNEL32", "GetProcAddress", &[ddraw, OUT]),
        Ok(0)
    );
    // The stub's machine declares it has no 3dfx card, so no Glide:
    // ERROR_DLL_NOT_FOUND, Windows 9x's answer.
    m.state.last_error = 0;
    assert_eq!(load(&mut m, "glide2x.dll"), Ok(0));
    assert_eq!(m.state.last_error, 1157);
    // A plugin nobody declared absent, shipped nowhere and written by no one.
    assert!(
        matches!(
            load(&mut m, "KLIB_OpenGL_PlugIn.dll"),
            Err(Stop::NotWritten { what, .. }) if what.contains("KLIB_OpenGL_PlugIn.dll")
        ),
        "a wall"
    );
}

/// A module with no table of keys among its resources has none to load, as
/// LEGO Island's own image has none (#118); one that does stops the run.
#[test]
fn a_table_of_keys_is_looked_for_in_the_modules_resources() {
    const RT_ICON: u32 = 3;
    const RT_ACCELERATOR: u32 = 9;
    let mut m = machine();
    // Headers at 0x40, the resource directory's address at the third of the
    // optional header's data directories, and one resource type in it.
    let module = DATA;
    m.arena
        .write_u32(module + 0x3c, 0x40)
        .expect("in the arena");
    m.arena
        .write_u32(module + 0x40 + 24 + 96 + 16, 0x200)
        .expect("in the arena");
    m.arena
        .write_u32(module + 0x200 + 12, 1 << 16)
        .expect("in the arena");
    m.arena.write(OUT, b"AppAccel\0").expect("in the arena");

    m.arena
        .write_u32(module + 0x200 + 16, RT_ICON)
        .expect("in the arena");
    assert_eq!(
        call(&mut m, "USER32", "LoadAcceleratorsA", &[module, OUT]),
        Ok(0)
    );
    // ERROR_RESOURCE_TYPE_NOT_FOUND.
    assert_eq!(m.state.last_error, 1813);

    m.arena
        .write_u32(module + 0x200 + 16, RT_ACCELERATOR)
        .expect("in the arena");
    assert!(call(&mut m, "USER32", "LoadAcceleratorsA", &[module, OUT]).is_err());
}

/// DirectX 5's retained mode makes DirectX 5's texture and material (#118):
/// the texture is DirectX 6's as far as `GenerateMIPMap`, with a `Changed`
/// that takes two flags, and the material is DirectX 6's without its ambient
/// colour.
#[test]
fn directx_5_makes_its_own_textures_and_materials() {
    let mut m = machine();
    // IDirect3DRM2::CreateTextureFromSurface(surface, out), at 18.
    assert_eq!(com(&mut m, "IDirect3DRM2", 18, 0, &[0, OUT]), Ok(0));
    let texture = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(m.state.interface_of(texture), Some("IDirect3DRMTexture2"));
    let changed = win32::com_method("IDirect3DRMTexture2", 14).expect("declared");
    assert_eq!((changed.name, changed.args), ("Changed", 2));
    assert_eq!(
        win32::com_method("IDirect3DRMTexture2", 32).map(|m| m.name),
        Some("GenerateMIPMap")
    );
    assert!(win32::com_method("IDirect3DRMTexture2", 33).is_none());

    // IDirect3DRM2::CreateMaterial(power, out), at 13.
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 13, 0, &[10.0f32.to_bits(), OUT]),
        Ok(0)
    );
    let material = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(m.state.interface_of(material), Some("IDirect3DRMMaterial"));
    assert_eq!(
        win32::com_method("IDirect3DRMMaterial", 16).map(|m| m.name),
        Some("GetEmissive")
    );
    assert!(win32::com_method("IDirect3DRMMaterial", 17).is_none());
}

/// Three floats where `at` is.
fn vector(machine: &Machine, at: u32) -> [f32; 3] {
    [0, 4, 8].map(|o| f32::from_bits(machine.arena.read_u32(at + o).expect("in the arena")))
}

/// DirectX 5's frame is not DirectX 6's with a slot taken out (#117). From
/// `GetTextureTopology` at 33 up its slots sit one place past DirectX 6's,
/// from `SetTextureTopology` at 65 two places, and from `AddMoveCallback2`
/// at 69 three: a frame made through DirectX 5's retained mode is placed and
/// read back through the slots its own header gives those methods.
#[test]
fn a_directx_5_frame_answers_at_its_own_slots() {
    let mut m = machine();
    let frame3 = win32::interface_named("IDirect3DRMFrame3").expect("declared");
    for gone in [
        "GetTextureTopology",
        "SetTextureTopology",
        "AddMoveCallback2",
    ] {
        assert!(
            frame3.methods.iter().all(|method| method.name != gone),
            "DirectX 6's frame has no {gone}"
        );
    }

    // IDirect3DRM2::CreateFrame(no parent, out).
    assert_eq!(com(&mut m, "IDirect3DRM2", 4, 0, &[0, OUT]), Ok(0));
    let frame = m.arena.read_u32(OUT).expect("in the arena");

    // Slot 33 is GetTextureTopology, which is not written, and not
    // InverseTransform, which is DirectX 6's slot 33.
    assert_eq!(
        win32::com_method("IDirect3DRMFrame2", 33).map(|m| m.name),
        Some("GetTextureTopology")
    );
    assert!(com(&mut m, "IDirect3DRMFrame2", 33, frame, &[0, 0]).is_err());

    // SetPosition at 61 (DirectX 6's 60), read back by GetPosition at 24,
    // where both agree.
    let [x, y, z] = [1.0f32, 2.0, 3.0].map(f32::to_bits);
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 61, frame, &[0, x, y, z]),
        Ok(0)
    );
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 24, frame, &[0, OUT]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [1.0, 2.0, 3.0]);

    // Transform at 68 (DirectX 6's 66): the frame's origin is where it was
    // put.
    m.arena.write(DATA, &[0; 12]).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 68, frame, &[OUT, DATA]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [1.0, 2.0, 3.0]);

    // And InverseTransform at 34 (DirectX 6's 33) takes it back.
    assert_eq!(
        com(&mut m, "IDirect3DRMFrame2", 34, frame, &[OUT + 16, OUT]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT + 16), [0.0, 0.0, 0.0]);
}

/// D3DRMCreateColorRGBA, which LEGO Island's engine imports (#117): alpha in
/// the top byte, then red, green and blue, each fraction truncated to a byte
/// and held between none and all.
#[test]
fn a_colour_from_four_fractions_puts_alpha_on_top() {
    let mut m = machine();
    let mut colour = |rgba: [f32; 4]| {
        call(
            &mut m,
            "d3drm",
            "D3DRMCreateColorRGBA",
            &rgba.map(f32::to_bits),
        )
    };
    assert_eq!(colour([1.0, 0.5, 0.0, 0.25]), Ok(0x3fff_7f00));
    assert_eq!(colour([-1.0, 2.0, 0.999, 1.0]), Ok(0xff00_fffe));
}

/// A mesh LEGO Island builds its world from (#119), made through DirectX 5's
/// retained mode, with `groups` groups of four vertices and two triangles.
fn mesh(m: &mut Machine, groups: u32) -> u32 {
    // IDirect3DRM2::CreateMesh(out), at 5.
    assert_eq!(com(m, "IDirect3DRM2", 5, 0, &[OUT]), Ok(0));
    let mesh = m.arena.read_u32(OUT).expect("in the arena");
    for (at, index) in [0, 1, 2, 2, 1, 3].into_iter().enumerate() {
        m.arena
            .write_u32(DATA + 4 * at as u32, index)
            .expect("in the arena");
    }
    for id in 0..groups {
        // AddGroup(vertices, faces, vertices a face, face data, out), at 14.
        assert_eq!(
            com(m, "IDirect3DRMMesh", 14, mesh, &[4, 2, 3, DATA, OUT]),
            Ok(0)
        );
        assert_eq!(m.arena.read_u32(OUT), Ok(id), "groups are numbered in turn");
    }
    mesh
}

/// Four `D3DRMVERTEX`es at `at`, each placed at `corner` times its number
/// and with its number in every other field.
fn vertices(m: &mut Machine, at: u32, corner: [f32; 3]) -> Vec<[u32; 9]> {
    (0..4u32)
        .map(|n| {
            let mut v = [n; 9];
            for i in 0..3 {
                v[i] = (corner[i] * n as f32).to_bits();
            }
            for (i, word) in v.iter().enumerate() {
                m.arena
                    .write_u32(at + 36 * n + 4 * i as u32, *word)
                    .expect("in the arena");
            }
            v
        })
        .collect()
}

/// What retained mode keeps of a mesh (#119): each group's faces and
/// vertices copied as they were given, so a title that reuses its scratch
/// for the next group changes nothing, and what each group is drawn with.
#[test]
fn a_mesh_keeps_its_groups_as_they_were_given() {
    let mut m = machine();
    let mesh = mesh(&mut m, 2);
    let given = vertices(&mut m, DATA + 0x100, [1.0, 2.0, 3.0]);
    // SetVertices(group, first, count, vertices), at 15: all four, then the
    // scratch is written over.
    let set = |m: &mut Machine, args: [u32; 4]| com(m, "IDirect3DRMMesh", 15, mesh, &args);
    assert_eq!(set(&mut m, [0, 0, 4, DATA + 0x100]), Ok(0));
    vertices(&mut m, DATA + 0x100, [9.0, 9.0, 9.0]);
    m.arena.write_u32(DATA, 7).expect("in the arena");
    let group = |m: &Machine, id: usize| m.state.scene.meshes[&mesh].groups[id].clone();
    let kept = group(&m, 0);
    assert_eq!(
        kept.vertices,
        given.into_iter().map(Some).collect::<Vec<_>>()
    );
    assert_eq!((kept.per_face, kept.indices), (3, vec![0, 1, 2, 2, 1, 3]));
    // Only the vertices it names are set. Past the group's end, or a group
    // the mesh does not have, is a bad value.
    assert_eq!(set(&mut m, [1, 3, 1, DATA + 0x100]), Ok(0));
    assert_eq!(group(&m, 1).vertices.iter().flatten().count(), 1);
    assert_eq!(group(&m, 1).vertices[3].map(|v| v[3]), Some(0));
    assert_eq!(set(&mut m, [1, 3, 2, DATA + 0x100]), Ok(0x8876_0316));
    assert_eq!(set(&mut m, [2, 0, 1, DATA + 0x100]), Ok(0x8876_0316));

    // SetGroupColor and SetGroupColorRGB, at 16 and 17, set one colour;
    // SetGroupMapping and SetGroupQuality, at 18 and 19, are kept as given.
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 16, mesh, &[0, 0x80ff_0000]),
        Ok(0)
    );
    let [r, g, b] = [0.0, 1.0, 0.0f32].map(f32::to_bits);
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 17, mesh, &[1, r, g, b]),
        Ok(0)
    );
    assert_eq!(com(&mut m, "IDirect3DRMMesh", 18, mesh, &[0, 4]), Ok(0));
    assert_eq!(com(&mut m, "IDirect3DRMMesh", 19, mesh, &[0, 0x88]), Ok(0));
    let (first, second) = (group(&m, 0), group(&m, 1));
    assert_eq!(
        (first.colour, first.mapping, first.quality),
        (Some(0x80ff_0000), Some(4), Some(0x88))
    );
    assert_eq!(
        (second.colour, second.mapping, second.quality),
        (Some(0xff00_ff00), None, None)
    );
    // Every group at once has not been asked for.
    assert!(matches!(
        com(&mut m, "IDirect3DRMMesh", 16, mesh, &[u32::MAX, 0]),
        Err(Stop::NotWritten { .. })
    ));

    // SetGroupMaterial and SetGroupTexture, at 20 and 21: the group holds a
    // reference to each, and lets go of one it is no longer drawn with.
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 13, 0, &[1.0f32.to_bits(), OUT]),
        Ok(0)
    );
    let material = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(com(&mut m, "IDirect3DRM2", 18, 0, &[0, OUT]), Ok(0));
    let texture = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 20, mesh, &[0, material]),
        Ok(0)
    );
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 21, mesh, &[1, texture]),
        Ok(0)
    );
    assert_eq!(
        (group(&m, 0).material, group(&m, 1).texture),
        (material, texture)
    );
    // AddRef, at 1, answers how many references there are now.
    assert_eq!(com(&mut m, "IDirect3DRMMaterial", 1, material, &[]), Ok(3));
    assert_eq!(com(&mut m, "IDirect3DRMMesh", 20, mesh, &[0, 0]), Ok(0));
    assert_eq!(com(&mut m, "IDirect3DRMMaterial", 1, material, &[]), Ok(3));
    assert_eq!(group(&m, 0).material, 0);
}

/// A mesh's box holds every vertex of every group, and a clone is a mesh of
/// its own with the same groups (#119).
#[test]
fn a_mesh_has_a_box_round_its_vertices_and_clones_to_a_mesh_of_its_own() {
    let mut m = machine();
    let mesh = mesh(&mut m, 2);
    // GetBox(box), at 13: not while a vertex has nowhere to be.
    let bounds = |m: &mut Machine, of: u32| com(m, "IDirect3DRMMesh", 13, of, &[OUT]);
    assert!(matches!(bounds(&mut m, mesh), Err(Stop::NotWritten { .. })));
    vertices(&mut m, DATA + 0x100, [1.0, -2.0, 0.5]);
    vertices(&mut m, DATA + 0x200, [-1.0, 4.0, 0.25]);
    for (id, at) in [(0, DATA + 0x100), (1, DATA + 0x200)] {
        assert_eq!(
            com(&mut m, "IDirect3DRMMesh", 15, mesh, &[id, 0, 4, at]),
            Ok(0)
        );
    }
    assert_eq!(bounds(&mut m, mesh), Ok(0));
    assert_eq!(vector(&m, OUT), [-3.0, -6.0, 0.0]);
    assert_eq!(vector(&m, OUT + 12), [3.0, 12.0, 1.5]);

    // Clone(outer, interface, out), at 3, as IID_IDirect3DRMMesh.
    let iid = [
        0x01, 0x0d, 0xa8, 0xa3, 0x12, 0x6e, 0xcf, 0x11, 0xac, 0x4a, 0x00, 0x00, 0xc0, 0x38, 0x25,
        0xa1,
    ];
    m.arena.write(DATA, &iid).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 3, mesh, &[0, DATA, OUT]),
        Ok(0)
    );
    let clone = m.arena.read_u32(OUT).expect("in the arena");
    assert_ne!(clone, mesh);
    assert_eq!(m.state.interface_of(clone), Some("IDirect3DRMMesh"));
    assert_eq!(
        com(&mut m, "IDirect3DRMMesh", 16, clone, &[0, 0xff12_3456]),
        Ok(0)
    );
    let colour = |m: &Machine, of: u32| m.state.scene.meshes[&of].groups[0].colour;
    assert_eq!(
        (colour(&m, mesh), colour(&m, clone)),
        (None, Some(0xff12_3456))
    );
    assert_eq!(bounds(&mut m, clone), Ok(0));
    assert_eq!(vector(&m, OUT + 12), [3.0, 12.0, 1.5]);
    // As an aggregate, or as anything else, is a wall.
    assert!(matches!(
        com(&mut m, "IDirect3DRMMesh", 3, mesh, &[OUT, DATA, OUT]),
        Err(Stop::NotWritten { .. })
    ));
    m.arena.write(DATA, &[0; 16]).expect("in the arena");
    assert!(matches!(
        com(&mut m, "IDirect3DRMMesh", 3, mesh, &[0, DATA, OUT]),
        Err(Stop::NotWritten { .. })
    ));
}

/// IDirect3DRMViewport::ForceUpdate (#119): what retained mode draws here is
/// in the device's surface already, so a part of the viewport to copy to the
/// screen has nothing to do, and a part outside it is a wall.
#[test]
fn a_viewport_forced_to_update_has_nothing_to_copy() {
    let mut m = machine();
    // IDirect3DRM2::CreateViewport(device, camera, x, y, width, height, out),
    // at 20.
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 20, 0, &[0, 0, 0, 0, 640, 480, OUT]),
        Ok(0)
    );
    let viewport = m.arena.read_u32(OUT).expect("in the arena");
    // ForceUpdate(left, top, right, bottom), at 23.
    let force =
        |m: &mut Machine, area: [u32; 4]| com(m, "IDirect3DRMViewport", 23, viewport, &area);
    assert_eq!(force(&mut m, [0, 0, 640, 480]), Ok(0));
    assert_eq!(force(&mut m, [301, 12, 400, 90]), Ok(0));
    assert!(matches!(
        force(&mut m, [0, 0, 641, 480]),
        Err(Stop::NotWritten { .. })
    ));
}

/// IDirect3DRMViewport::SetUniformScaling (#134): uniform scaling is how a
/// viewport starts, and scaling across and up apart is a wall.
#[test]
fn a_viewport_scales_uniformly_and_never_apart() {
    let mut m = machine();
    assert_eq!(
        com(&mut m, "IDirect3DRM2", 20, 0, &[0, 0, 0, 0, 640, 480, OUT]),
        Ok(0)
    );
    let viewport = m.arena.read_u32(OUT).expect("in the arena");
    let scale = |m: &mut Machine, on: u32| {
        method(
            m,
            "IDirect3DRMViewport",
            "SetUniformScaling",
            viewport,
            &[on],
        )
    };
    assert_eq!(scale(&mut m, 1), Ok(0));
    assert!(matches!(scale(&mut m, 0), Err(Stop::NotWritten { .. })));
}

/// Call the method of `interface` called `name` on `this`.
fn method(
    m: &mut Machine,
    interface: &str,
    name: &str,
    this: u32,
    args: &[u32],
) -> Result<u32, Stop> {
    let slot = win32::interface_named(interface)
        .and_then(|i| i.methods.iter().position(|method| method.name == name))
        .unwrap_or_else(|| panic!("{interface} has {name}"));
    com(m, interface, slot, this, args)
}

/// Call a method that makes something, and hand back what it made.
fn made(m: &mut Machine, interface: &str, name: &str, this: u32, args: &[u32]) -> u32 {
    let mut args = args.to_vec();
    args.push(OUT);
    assert_eq!(method(m, interface, name, this, &args), Ok(0), "{name}");
    m.arena.read_u32(OUT).expect("in the arena")
}

fn floats<const N: usize>(values: [f32; N]) -> [u32; N] {
    values.map(f32::to_bits)
}

/// A drawing surface's pixels, 64 by 48 at sixteen bits, and a texture's.
const PIXELS: u32 = 0x0052_0000;
const TEXELS: u32 = 0x0052_8000;

fn surface(m: &mut Machine, pixels: u32, width: u32, height: u32) -> u32 {
    with_ctx(m, |ctx| {
        win32::State::make_com(
            ctx,
            &win32::ddraw::DIRECT_DRAW_SURFACE,
            win32::ComState::Surface {
                pixels,
                width,
                height,
                pitch: width * 2,
                format: PixelFormat::RGB565,
                primary: false,
                back: 0,
                palette: 0,
            },
        )
        .map_err(Stop::Fault)
    })
    .expect("a surface")
}

fn pixel(m: &Machine, x: u32, y: u32) -> u32 {
    let at = PIXELS + y * 128 + x * 2;
    let bytes = m.arena.slice(at, 2).expect("in the arena");
    u32::from(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn rgb(r: u8, g: u8, b: u8) -> u32 {
    PixelFormat::RGB565.encode([r, g, b, 255])
}

/// What LEGO Island draws its world through (#117, #120): an immediate-mode
/// device it made on a surface of its own, 64 by 48 here, and DirectX 5's
/// retained mode over that at `quality`; a scene, blue behind, and a camera
/// at its middle looking down z, seen through a viewport over the whole
/// device whose front plane is 0.1 away and whose field there is 0.05 either
/// side: a half-width of a half at every unit of distance.
fn stage(m: &mut Machine, quality: u32) -> (u32, u32) {
    let surface = surface(m, PIXELS, 64, 48);
    let immediate = with_ctx(m, |ctx| {
        win32::State::make_com(
            ctx,
            &win32::d3d::DEVICE2,
            win32::ComState::Device3D { surface },
        )
        .map_err(Stop::Fault)
    })
    .expect("a device");
    let device = made(m, "IDirect3DRM2", "CreateDeviceFromD3D", 0, &[0, immediate]);
    assert_eq!(
        method(m, "IDirect3DRMDevice2", "SetQuality", device, &[quality]),
        Ok(0)
    );
    let scene = made(m, "IDirect3DRM2", "CreateFrame", 0, &[0]);
    let camera = made(m, "IDirect3DRM2", "CreateFrame", 0, &[scene]);
    let viewport = made(
        m,
        "IDirect3DRM2",
        "CreateViewport",
        0,
        &[device, camera, 0, 0, 64, 48],
    );
    let view = |m: &mut Machine, name: &str, value: f32| {
        assert_eq!(
            method(m, "IDirect3DRMViewport", name, viewport, &[value.to_bits()]),
            Ok(0)
        );
    };
    view(m, "SetFront", 0.1);
    view(m, "SetBack", 10.0);
    view(m, "SetField", 0.05);
    let blue = floats([0.0, 0.0, 1.0]);
    assert_eq!(
        method(
            m,
            "IDirect3DRMFrame2",
            "SetSceneBackgroundRGB",
            scene,
            &blue
        ),
        Ok(0)
    );
    (viewport, scene)
}

/// A mesh of one group: a square a unit across, facing back down z at the
/// camera, wound clockwise as the camera sees it, and white; textured left
/// to right and top to bottom.
fn square(m: &mut Machine, quality: u32) -> u32 {
    let mesh = made(m, "IDirect3DRM2", "CreateMesh", 0, &[]);
    for (at, index) in [2, 3, 1, 2, 1, 0].into_iter().enumerate() {
        m.arena
            .write_u32(DATA + 4 * at as u32, index)
            .expect("in the arena");
    }
    assert_eq!(
        made(m, "IDirect3DRMMesh", "AddGroup", mesh, &[4, 2, 3, DATA]),
        0
    );
    for (n, [x, y, u, v]) in [
        [-0.5, -0.5, 0.0, 1.0],
        [0.5, -0.5, 1.0, 1.0],
        [-0.5, 0.5, 0.0, 0.0],
        [0.5, 0.5, 1.0, 0.0],
    ]
    .into_iter()
    .enumerate()
    {
        let vertex = floats([x, y, 0.0, 0.0, 0.0, -1.0, u, v]);
        for (i, word) in vertex.into_iter().chain([0]).enumerate() {
            m.arena
                .write_u32(DATA + 0x100 + 36 * n as u32 + 4 * i as u32, word)
                .expect("in the arena");
        }
    }
    let group = |m: &mut Machine, name: &str, args: &[u32]| {
        assert_eq!(
            method(m, "IDirect3DRMMesh", name, mesh, args),
            Ok(0),
            "{name}"
        );
    };
    group(m, "SetVertices", &[0, 0, 4, DATA + 0x100]);
    group(m, "SetGroupQuality", &[0, quality]);
    group(m, "SetGroupMapping", &[0, 4]);
    let white = floats([1.0, 1.0, 1.0]);
    group(m, "SetGroupColorRGB", &[0, white[0], white[1], white[2]]);
    mesh
}

/// Hang a frame in the scene `distance` in front of the camera, and the
/// mesh on a frame of its own that is that frame's visual, as LEGO Island
/// shares its models between the places it puts them.
fn hang(m: &mut Machine, scene: u32, mesh: u32, distance: f32) -> u32 {
    let place = made(m, "IDirect3DRM2", "CreateFrame", 0, &[scene]);
    let at = floats([0.0, 0.0, distance]);
    assert_eq!(
        method(
            m,
            "IDirect3DRMFrame2",
            "SetPosition",
            place,
            &[scene, at[0], at[1], at[2]]
        ),
        Ok(0)
    );
    let model = made(m, "IDirect3DRM2", "CreateFrame", 0, &[0]);
    assert_eq!(
        method(m, "IDirect3DRMFrame2", "AddVisual", model, &[mesh]),
        Ok(0)
    );
    assert_eq!(
        method(m, "IDirect3DRMFrame2", "AddVisual", place, &[model]),
        Ok(0)
    );
    place
}

fn draw(m: &mut Machine, viewport: u32, scene: u32) {
    assert_eq!(
        method(m, "IDirect3DRMViewport", "Clear", viewport, &[]),
        Ok(0)
    );
    assert_eq!(
        method(m, "IDirect3DRMViewport", "Render", viewport, &[scene]),
        Ok(0)
    );
}

/// What LEGO Island's retained mode does to draw its world (#120). DirectX
/// 5's Clear takes no flags and clears the viewport to its scene's
/// background and its depth buffer to as far as it goes; Render draws every
/// mesh in the scene where its frame puts it, lit by the scene's lights.
///
/// The field is the half-width at the front plane, not at a unit away: with
/// the front plane at 0.1 and a field of 0.05 there, a square a unit across
/// two units away covers the middle half of the viewport. Read as the
/// half-width at a unit away, it would be twenty times as wide and cover
/// all of it.
#[test]
fn a_viewport_clears_to_its_scene_and_draws_its_meshes_lit_where_they_are() {
    let mut m = machine();
    // Flat, lit and solid: the device's quality and the group's.
    let (viewport, scene) = stage(&mut m, 0x88);
    let mesh = square(&mut m, 0x88);
    let place = hang(&mut m, scene, mesh, 2.0);
    // A quarter of white all round, and half of white shining down z, full
    // on the square's face.
    let quarter = floats([0.25; 3]);
    let half = floats([0.5; 3]);
    let ambient = made(
        &mut m,
        "IDirect3DRM2",
        "CreateLightRGB",
        0,
        &[0, quarter[0], quarter[1], quarter[2]],
    );
    let sun = made(
        &mut m,
        "IDirect3DRM2",
        "CreateLightRGB",
        0,
        &[3, half[0], half[1], half[2]],
    );
    assert_eq!(
        method(&mut m, "IDirect3DRMFrame2", "AddLight", scene, &[ambient]),
        Ok(0)
    );
    assert_eq!(
        method(&mut m, "IDirect3DRMFrame2", "AddLight", scene, &[sun]),
        Ok(0)
    );

    draw(&mut m, viewport, scene);
    // A light's colour is kept in bytes: a quarter is 63 of 255 and a half
    // 127, which together are 190.
    let (lit, sky) = (rgb(190, 190, 190), rgb(0, 0, 255));
    for x in [17, 32, 46] {
        assert_eq!(pixel(&m, x, 24), lit, "{x} across is the square, lit");
    }
    for x in [14, 49] {
        assert_eq!(
            pixel(&m, x, 24),
            sky,
            "{x} across is the scene's background"
        );
    }
    assert_eq!(pixel(&m, 32, 2), sky);

    // Moved back, the square is smaller and farther than what was drawn
    // before: drawn only because Clear took the depth buffer back too.
    let at = floats([0.0, 0.0, 3.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "SetPosition",
            place,
            &[scene, at[0], at[1], at[2]]
        ),
        Ok(0)
    );
    draw(&mut m, viewport, scene);
    assert_eq!(pixel(&m, 32, 24), lit);
    assert_eq!(pixel(&m, 17, 24), sky);
}

/// A group's texture is DirectX 3's face of it, which LEGO Island asks for
/// DirectX 5's face, `{120f30c0-1629-11d0-941c-0080c80cfa7b}` as Wine's
/// `d3drmobj.h` names `IID_IDirect3DRMTexture2`; and it is drawn from its
/// surface as the surface is when it is drawn, so changing the surface's
/// pixels changes it, and `Changed` has nothing to do (#120).
#[test]
fn a_texture_is_drawn_from_its_surface_as_the_surface_is_now() {
    let mut m = machine();
    // Unlit: each pixel is the texel under it.
    let (viewport, scene) = stage(&mut m, 0x80);
    let mesh = square(&mut m, 0x80);
    hang(&mut m, scene, mesh, 2.0);
    // Two texels: red, then green.
    let (red, green, blue) = (rgb(255, 0, 0), rgb(0, 255, 0), rgb(0, 0, 255));
    let texels = |m: &mut Machine, left: u32, right: u32| {
        m.arena
            .write_u32(TEXELS, right << 16 | left)
            .expect("in the arena");
    };
    texels(&mut m, red, green);
    let image = surface(&mut m, TEXELS, 2, 1);
    let texture = made(
        &mut m,
        "IDirect3DRM2",
        "CreateTextureFromSurface",
        0,
        &[image],
    );
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMMesh",
            "SetGroupTexture",
            mesh,
            &[0, texture]
        ),
        Ok(0)
    );
    draw(&mut m, viewport, scene);
    assert_eq!((pixel(&m, 20, 24), pixel(&m, 44, 24)), (red, green));

    // GetGroupTexture hands back DirectX 3's face of the same texture.
    let older = made(&mut m, "IDirect3DRMMesh", "GetGroupTexture", mesh, &[0]);
    assert_eq!(m.state.interface_of(older), Some("IDirect3DRMTexture"));
    assert_eq!(m.state.identity(older), m.state.identity(texture));
    let iid = [
        0xc0, 0x30, 0x0f, 0x12, 0x29, 0x16, 0xd0, 0x11, 0x94, 0x1c, 0x00, 0x80, 0xc8, 0x0c, 0xfa,
        0x7b,
    ];
    m.arena.write(DATA, &iid).expect("in the arena");
    let newer = made(
        &mut m,
        "IDirect3DRMTexture",
        "QueryInterface",
        older,
        &[DATA],
    );
    assert_eq!(m.state.interface_of(newer), Some("IDirect3DRMTexture2"));

    texels(&mut m, blue, red);
    assert_eq!(
        method(&mut m, "IDirect3DRMTexture2", "Changed", newer, &[1, 0]),
        Ok(0)
    );
    draw(&mut m, viewport, scene);
    assert_eq!((pixel(&m, 20, 24), pixel(&m, 44, 24)), (blue, red));
}

/// A null reference frame is the world (#120): a frame hung from one moved
/// ten across is put where it is asked to be in the world, and read back
/// from there, whatever its parent.
#[test]
fn a_null_reference_frame_is_the_world() {
    let mut m = machine();
    let parent = made(&mut m, "IDirect3DRM2", "CreateFrame", 0, &[0]);
    let moved = floats([10.0, 0.0, 0.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "SetPosition",
            parent,
            &[0, moved[0], moved[1], moved[2]]
        ),
        Ok(0)
    );
    let child = made(&mut m, "IDirect3DRM2", "CreateFrame", 0, &[parent]);
    let at = floats([1.0, 2.0, 3.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "SetPosition",
            child,
            &[0, at[0], at[1], at[2]]
        ),
        Ok(0)
    );
    assert_eq!(
        method(&mut m, "IDirect3DRMFrame2", "GetPosition", child, &[0, OUT]),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [1.0, 2.0, 3.0]);
    assert_eq!(
        method(
            &mut m,
            "IDirect3DRMFrame2",
            "GetPosition",
            child,
            &[parent, OUT]
        ),
        Ok(0)
    );
    assert_eq!(vector(&m, OUT), [-9.0, 2.0, 3.0]);
}

/// A disc laid out as a Joliet mastering tool lays one out (ECMA-119 and the
/// Joliet specification): a primary tree of 8.3 names and a Joliet tree of
/// long ones, two big-endian bytes a character, both recording the same files.
/// The root holds `Long Folder` (`LONGFO~1`), `Long Name.txt` (`LONGNA~1.TXT`),
/// `short.txt` and `records.bin`, a file whose bytes are the folder's own
/// records; the folder holds `Inner Long Name.txt` (`INNERL~1.TXT`).
fn joliet_disc(name: &str) -> disc::Iso {
    const SECTOR: usize = 2048;
    // A directory record: its length, where its extent is and how long, both
    // ways round, whether it is a directory, and its name, padded to even.
    fn record(name: &[u8], at: u32, len: u32, dir: bool) -> Vec<u8> {
        let mut r = vec![0u8; 33];
        r[2..6].copy_from_slice(&at.to_le_bytes());
        r[6..10].copy_from_slice(&at.to_be_bytes());
        r[10..14].copy_from_slice(&len.to_le_bytes());
        r[14..18].copy_from_slice(&len.to_be_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r.extend_from_slice(name);
        r.resize(r.len().next_multiple_of(2), 0);
        r[0] = r.len() as u8;
        r
    }
    let ucs2 = |name: &str| -> Vec<u8> { name.encode_utf16().flat_map(u16::to_be_bytes).collect() };
    // A directory: itself, its parent, then what it holds.
    let dir = |at: u32, parent: u32, entries: &[(Vec<u8>, u32, u32, bool)]| {
        let mut bytes = record(&[0], at, 2048, true);
        bytes.extend(record(&[1], parent, 2048, true));
        for (name, sector, len, is_dir) in entries {
            bytes.extend(record(name, *sector, *len, *is_dir));
        }
        bytes
    };
    let mut image = vec![0u8; 34 * SECTOR];
    let mut put = |sector: usize, bytes: &[u8]| {
        image[sector * SECTOR..sector * SECTOR + bytes.len()].copy_from_slice(bytes);
    };
    // The primary volume descriptor, Joliet's supplementary one with UCS-2
    // level 1's escape sequence, and the terminator.
    for (sector, kind, root) in [(16, 1, 20), (17, 2, 21), (18, 255, 0)] {
        let mut d = vec![0u8; SECTOR];
        d[0] = kind;
        d[1..6].copy_from_slice(b"CD001");
        d[6] = 1;
        if kind == 2 {
            d[88..91].copy_from_slice(b"%/@");
        }
        if kind != 255 {
            d[156..190].copy_from_slice(&record(&[0], root, 2048, true));
        }
        put(sector, &d);
    }
    let file = |name: Vec<u8>, at: u32, len: u32| (name, at, len, false);
    put(
        20,
        &dir(
            20,
            20,
            &[
                (b"LONGFO~1".to_vec(), 22, 2048, true),
                file(b"LONGNA~1.TXT;1".to_vec(), 30, 11),
                file(b"SHORT.TXT;1".to_vec(), 31, 5),
                file(b"RECORDS.BIN;1".to_vec(), 22, 2048),
            ],
        ),
    );
    put(
        21,
        &dir(
            21,
            21,
            &[
                (ucs2("Long Folder"), 23, 2048, true),
                file(ucs2("Long Name.txt;1"), 30, 11),
                file(ucs2("short.txt;1"), 31, 5),
                file(ucs2("records.bin;1"), 23, 2048),
            ],
        ),
    );
    put(22, &dir(22, 20, &[file(b"INNERL~1.TXT;1".to_vec(), 32, 6)]));
    put(
        23,
        &dir(23, 21, &[file(ucs2("Inner Long Name.txt;1"), 32, 6)]),
    );
    put(30, b"hello world");
    put(31, b"short");
    put(32, b"inside");
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, image).expect("the rip is written");
    disc::Iso::open(&path).expect("the rip opens")
}

/// How long a file `CreateFileA` opens to read is, or nothing when it opens
/// none.
fn opened(m: &mut Machine, path: &str) -> Option<u32> {
    m.arena
        .write(DATA, format!("{path}\0").as_bytes())
        .expect("in the arena");
    // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING.
    let file = call(
        m,
        "KERNEL32",
        "CreateFileA",
        &[DATA, 0x8000_0000, 1, 0, 3, 0, 0],
    )
    .expect("answered");
    (file != u32::MAX).then(|| call(m, "KERNEL32", "GetFileSize", &[file, 0]).expect("answered"))
}

/// A disc read through its Joliet tree, as Windows 95's CDFS reads one
/// (#144): a file opens by its long name, spelled in any case, in a directory
/// named by its own.
#[test]
fn a_cd_read_through_its_joliet_tree_answers_its_long_names() {
    let mut iso = joliet_disc("joliet.iso");
    assert_eq!(iso.read_joliet().ok(), Some(true));
    let mut m = machine();
    m.tree.set_disc(iso);
    for (path, len) in [
        ("D:\\Long Name.txt", 11),
        ("D:\\long name.TXT", 11),
        ("D:\\short.txt", 5),
        ("D:\\SHORT.TXT", 5),
        ("D:\\Long Folder\\Inner Long Name.txt", 6),
    ] {
        assert_eq!(opened(&mut m, path), Some(len), "{path}");
    }
    assert_eq!(opened(&mut m, "D:\\nothing.txt"), None);
    assert_eq!(m.state.last_error, 2, "ERROR_FILE_NOT_FOUND");
    assert_eq!(
        m.tree.wall(),
        None,
        "every name asked is the Joliet tree's or no tree's"
    );
}

/// The same disc read through its Joliet tree, asked for a name only its
/// primary tree records: an 8.3 name Windows 95's CDFS may or may not answer
/// (#144's q2), so the call is told it is not there and the machine stops at
/// a wall naming it, rather than guessing either way.
#[test]
fn a_name_only_the_primary_tree_records_is_a_wall_on_a_disc_read_through_its_joliet_tree() {
    let mut iso = joliet_disc("aliases.iso");
    assert_eq!(iso.read_joliet().ok(), Some(true));
    let mut m = machine();
    m.tree.set_disc(iso);
    for path in ["D:\\LONGNA~1.TXT", "D:\\LONGFO~1\\INNERL~1.TXT"] {
        assert_eq!(opened(&mut m, path), None, "{path}");
        let wall = m.tree.wall().unwrap_or_default();
        assert!(
            wall.contains(&path[3..].replace('\\', "/")) && wall.contains("q2"),
            "{wall}"
        );
        assert_eq!(m.tree.wall(), None, "a wall is taken once");
    }
    // The ceiling the code names: a path that mixes the two trees' names is in
    // neither, and is not found.
    assert_eq!(opened(&mut m, "D:\\Long Folder\\INNERL~1.TXT"), None);
    assert_eq!(m.tree.wall(), None);
}

/// The same disc on a machine that does not read its Joliet tree, as every
/// released title's does not (#144): the primary tree's names alone, so no
/// long name opens and no name is a wall.
#[test]
fn a_cd_read_without_its_joliet_tree_answers_its_8_3_names_alone() {
    let mut m = machine();
    m.tree.set_disc(joliet_disc("primary.iso"));
    assert_eq!(opened(&mut m, "D:\\Long Name.txt"), None);
    assert_eq!(m.state.last_error, 2, "ERROR_FILE_NOT_FOUND");
    assert_eq!(opened(&mut m, "D:\\LONGNA~1.TXT"), Some(11));
    assert_eq!(opened(&mut m, "D:\\LONGFO~1\\INNERL~1.TXT"), Some(6));
    assert_eq!(m.tree.wall(), None);
}

/// A path through a file is no path, as Windows answers one, in either tree:
/// the file's bytes are not read as a directory's records, even where they are
/// a directory's records (#144).
#[test]
fn a_path_through_a_file_is_no_path() {
    for joliet in [false, true] {
        let mut iso = joliet_disc(&format!("through-a-file-{joliet}.iso"));
        let inner = if joliet {
            assert_eq!(iso.read_joliet().ok(), Some(true));
            "records.bin/Inner Long Name.txt"
        } else {
            "RECORDS.BIN/INNERL~1.TXT"
        };
        assert!(
            matches!(
                iso.list("records.bin"),
                Err(disc::DiscError::NoSuchFile { .. })
            ),
            "joliet {joliet}"
        );
        assert_eq!(iso.has(inner).ok(), Some(false), "{inner}");
        let mut m = machine();
        m.tree.set_disc(iso);
        let guest = format!("D:\\{}", inner.replace('/', "\\"));
        assert_eq!(opened(&mut m, &guest), None, "{guest}");
        assert_eq!(m.tree.wall(), None);
    }
}
/// The stub's machine with a floppy drive, as LEGO Chess declares one (#145).
static FLOPPY: Title = Title {
    declared: Declared {
        floppy: true,
        ..Title::STUB.declared
    },
    ..Title::STUB
};

/// Call `KERNEL32!name` on `title`'s machine.
fn kernel32(m: &mut Machine, title: &'static Title, name: &str, args: &[u32]) -> Result<u32, Stop> {
    call_as(m, title, "KERNEL32", name, args)
}

/// LEGO Chess's CD check, `0x405420`, as it makes its calls: it tests bit 0
/// of `GetLogicalDrives` once and never shifts, then asks each of 32 letters
/// its type, and of each CD-ROM its volume, which must be `CDFS` and
/// `LEGOCHESS`. The root of the last one that is, or none.
fn chess_cd_check(m: &mut Machine, title: &'static Title) -> Option<String> {
    let mask = kernel32(m, title, "GetLogicalDrives", &[]).expect("answered");
    let mut found = None;
    for letter in (b'A'..).take(32) {
        if mask & 1 == 0 {
            continue;
        }
        m.arena
            .write(DATA, &[letter, b':', b'\\', 0])
            .expect("in the arena");
        if kernel32(m, title, "GetDriveTypeA", &[DATA]) != Ok(5) {
            continue;
        }
        let (label, system) = (OUT, OUT + 0x100);
        let asked = [
            DATA,
            label,
            0x100,
            OUT + 0x200,
            OUT + 0x204,
            OUT + 0x208,
            system,
            0x100,
        ];
        if kernel32(m, title, "GetVolumeInformationA", &asked) == Ok(0) {
            continue;
        }
        if text(m, system) == "CDFS" && text(m, label) == "LEGOCHESS" {
            found = Some(text(m, DATA));
        }
    }
    found
}

#[test]
fn the_chess_cd_check_finds_its_disc_past_a_floppy_drive() {
    let mut m = machine();
    m.state.volume_label = "LEGOCHESS".to_owned();
    assert_eq!(chess_cd_check(&mut m, &FLOPPY), Some("D:\\".to_owned()));
    // With no A: it looks at no drive at all, which is the game's own bug.
    assert_eq!(chess_cd_check(&mut m, &Title::STUB), None);
}

#[test]
fn a_floppy_drive_is_removable_and_named_with_the_others() {
    let mut m = machine();
    m.arena.write(DATA, b"A:\\\0").expect("in the arena");

    // Bit 0 is A:, and it is named first among the roots.
    assert_eq!(
        kernel32(&mut m, &FLOPPY, "GetLogicalDrives", &[]),
        Ok(0b1101)
    );
    assert_eq!(
        kernel32(&mut m, &FLOPPY, "GetLogicalDriveStringsA", &[0x100, OUT]),
        Ok(12)
    );
    assert_eq!(
        m.arena.slice(OUT, 13).expect("in the arena"),
        b"A:\\\0C:\\\0D:\\\0\0"
    );
    // DRIVE_REMOVABLE.
    assert_eq!(kernel32(&mut m, &FLOPPY, "GetDriveTypeA", &[DATA]), Ok(2));

    // A machine with no floppy drive answers as it always has.
    assert_eq!(
        kernel32(&mut m, &Title::STUB, "GetLogicalDrives", &[]),
        Ok(0b1100)
    );
    assert_eq!(
        kernel32(&mut m, &Title::STUB, "GetDriveTypeA", &[DATA]),
        Ok(1)
    );
    assert_eq!(
        kernel32(
            &mut m,
            &Title::STUB,
            "GetLogicalDriveStringsA",
            &[0x100, OUT]
        ),
        Ok(8)
    );
    assert_eq!(
        m.arena.slice(OUT, 9).expect("in the arena"),
        b"C:\\\0D:\\\0\0"
    );
}

/// Each call that names a place on A:, as dll, name, arguments and what it
/// answers when it fails.
fn on_a(m: &mut Machine) -> Vec<(&'static str, &'static str, Vec<u32>, u32)> {
    let (root, file, here, wide) = (DATA, DATA + 16, DATA + 48, DATA + 64);
    m.arena.write(root, b"A:\\\0").expect("in the arena");
    m.arena
        .write(file, b"A:\\SAVE.DAT\0")
        .expect("in the arena");
    m.arena
        .write(here, b"C:\\SAVE.DAT\0")
        .expect("in the arena");
    let name: Vec<u8> = "A:\\INTRO.AVI\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    m.arena.write(wide, &name).expect("in the arena");
    let failed = 0xffff_ffff;
    vec![
        (
            "KERNEL32",
            "GetVolumeInformationA",
            vec![root, OUT, 0x100, 0, 0, 0, OUT + 0x100, 0x100],
            0,
        ),
        (
            "KERNEL32",
            "CreateFileA",
            vec![file, 0x8000_0000, 1, 0, 3, 0x80, 0],
            failed,
        ),
        ("KERNEL32", "GetFileAttributesA", vec![file], failed),
        ("KERNEL32", "FindFirstFileA", vec![file, OUT], failed),
        ("KERNEL32", "DeleteFileA", vec![file], 0),
        ("KERNEL32", "CreateDirectoryA", vec![file, 0], 0),
        ("KERNEL32", "MoveFileA", vec![here, file], 0),
        ("KERNEL32", "SetCurrentDirectoryA", vec![root], 0),
        ("KERNEL32", "SetFileAttributesA", vec![file, 0x80], 0),
        ("KERNEL32", "LoadLibraryA", vec![file], 0),
        (
            "VERSION",
            "GetFileVersionInfoA",
            vec![file, 0, 0x100, OUT],
            0,
        ),
        ("WINMM", "mmioOpenA", vec![file, 0, 0], 0),
        // IAMMultiMediaStream::OpenFile: HRESULT_FROM_WIN32(ERROR_NOT_READY).
        (
            "IAMMultiMediaStream",
            "OpenFile",
            vec![wide, 0],
            0x8007_0015,
        ),
    ]
}

/// Make one of [`on_a`]'s calls on `title`'s machine.
fn make(
    m: &mut Machine,
    title: &'static Title,
    dll: &str,
    name: &str,
    args: &[u32],
) -> Result<u32, Stop> {
    if dll == "IAMMultiMediaStream" {
        // Its sixteenth slot, on a stream no title made: the drive is
        // refused before the stream is looked at.
        let method = win32::com_method(dll, 16).expect("a slot 16");
        assert_eq!(method.name, name);
        return as_title(m, title, |ctx| (method.run)(ctx, 0, args));
    }
    call_as(m, title, dll, name, args)
}

#[test]
fn whatever_looks_on_the_empty_floppy_drive_is_refused() {
    const ERROR_NOT_READY: u32 = 21;
    const SEM_FAILCRITICALERRORS: u32 = 1;
    let mut m = machine();
    let calls = on_a(&mut m);
    let here = m.state.current_directory.clone();

    // A title that has not said it takes critical errors itself would be
    // shown Windows' box asking for a disk, which this machine does not
    // have. Every call that did not stop there is named.
    let unrefused: Vec<_> = calls
        .iter()
        .filter(|(dll, name, args, _)| {
            !matches!(
                make(&mut m, &FLOPPY, dll, name, args),
                Err(Stop::NotWritten { .. })
            )
        })
        .map(|(_, name, ..)| *name)
        .collect();
    assert_eq!(
        unrefused,
        Vec::<&str>::new(),
        "these did not stop at the box"
    );

    // One that has is told the drive is not ready.
    kernel32(&mut m, &FLOPPY, "SetErrorMode", &[SEM_FAILCRITICALERRORS]).expect("answered");
    for (dll, name, args, failed) in &calls {
        m.state.last_error = 0;
        assert_eq!(
            make(&mut m, &FLOPPY, dll, name, args),
            Ok(*failed),
            "{name}"
        );
        assert_eq!(m.state.last_error, ERROR_NOT_READY, "{name}");
    }
    // Nothing moved where the guest believes it is.
    assert_eq!(m.state.current_directory, here);
}

/// One `DS3DLISTENER` sets every value the listener has, and the single
/// getters read back what it set; one DirectSound refuses is refused, and
/// nothing of it kept (#149). LEGO Chess sets its listener this way as its
/// sound comes up.
#[test]
fn a_listener_set_all_at_once_reads_back_one_value_at_a_time() {
    const DSERR_INVALIDPARAM: u32 = 0x8007_0057;
    const LISTENER: &str = "IDirectSound3DListener";
    let mut m = machine();
    let set = |m: &mut Machine, words: [f32; 15], size: u32, apply: u32| {
        m.arena.write_u32(DATA, size).expect("in the arena");
        for (i, w) in words.iter().enumerate() {
            m.arena
                .write_u32(DATA + 4 + 4 * i as u32, w.to_bits())
                .expect("in the arena");
        }
        method(m, LISTENER, "SetAllParameters", 0, &[DATA, apply])
    };
    let good: [f32; 15] = [
        1.0, 2.0, 3.0, // position
        9.0, 9.0, 9.0, // velocity
        0.0, 0.0, -1.0, // front
        0.0, 1.0, 0.0, // top
        2.0, 0.5, 3.0, // distance, rolloff and Doppler factors
    ];
    assert_eq!(set(&mut m, good, 64, 0), Ok(0));

    let read = |m: &Machine, n: u32| -> Vec<f32> {
        (0..n)
            .map(|i| f32::from_bits(m.arena.read_u32(OUT + 4 * i).expect("in the arena")))
            .collect()
    };
    let check = |m: &mut Machine| {
        assert_eq!(method(m, LISTENER, "GetPosition", 0, &[OUT]), Ok(0));
        assert_eq!(read(m, 3), [1.0, 2.0, 3.0]);
        assert_eq!(
            method(m, LISTENER, "GetOrientation", 0, &[OUT, OUT + 12]),
            Ok(0)
        );
        assert_eq!(read(m, 6), [0.0, 0.0, -1.0, 0.0, 1.0, 0.0]);
        for (name, value) in [
            ("GetDistanceFactor", 2.0),
            ("GetRolloffFactor", 0.5),
            ("GetDopplerFactor", 3.0),
        ] {
            assert_eq!(method(m, LISTENER, name, 0, &[OUT]), Ok(0));
            assert_eq!(read(m, 1), [value], "{name}");
        }
    };
    check(&mut m);

    // Refused, and the listener is as it was.
    let with = |at: usize, value: f32| {
        let mut words = good;
        words[at] = value;
        words
    };
    for (words, size, apply) in [
        (good, 60, 0),
        (good, 64, 2),
        (with(12, 0.0), 64, 1),
        (with(13, 10.5), 64, 1),
        (with(14, -1.0), 64, 1),
    ] {
        assert_eq!(set(&mut m, words, size, apply), Ok(DSERR_INVALIDPARAM));
    }
    assert_eq!(
        method(&mut m, LISTENER, "SetAllParameters", 0, &[0, 0]),
        Ok(DSERR_INVALIDPARAM)
    );
    check(&mut m);
}

/// The CD audio device opened by its type's number, as LEGO Chess's framework
/// opens it, opens with no Red Book pressing, the title's data disc in the
/// drive, as Windows' mcicda opens it; it counts in tracks, and is stopped
/// and closed. Opened by name it opens the same. Another of its type, or a
/// command it is not given here, is a wall (#149).
#[test]
fn cd_audio_opens_on_a_data_disc_by_number_and_by_name() {
    const MCI_OPEN: u32 = 0x0803;
    const MCI_CLOSE: u32 = 0x0804;
    const MCI_STOP: u32 = 0x0808;
    const MCI_SET: u32 = 0x080d;
    const MCI_STATUS: u32 = 0x0814;
    const MCI_WAIT: u32 = 0x0002;
    const MCI_OPEN_SHAREABLE: u32 = 0x0100;
    const MCI_OPEN_TYPE_ID: u32 = 0x1000;
    const MCI_OPEN_TYPE: u32 = 0x2000;
    const MCI_SET_TIME_FORMAT: u32 = 0x0400;
    const MCI_DEVTYPE_CD_AUDIO: u32 = 0x0204;
    const MCI_FORMAT_TMSF: u32 = 10;
    let mut m = machine();
    let opening = |m: &mut Machine, kind: u32| {
        // MCI_OPEN_PARMS: callback, the device id it answers, then the type.
        for (offset, word) in [(0, 0), (4, 0), (8, kind), (12, 0), (16, 0)] {
            m.arena
                .write_u32(DATA + offset, word)
                .expect("in the arena");
        }
        let flags = MCI_OPEN_TYPE | MCI_OPEN_TYPE_ID | MCI_OPEN_SHAREABLE | MCI_WAIT;
        call(m, "WINMM", "mciSendCommandA", &[0, MCI_OPEN, flags, DATA])
    };
    assert!(
        opening(&mut m, 1 << 16 | MCI_DEVTYPE_CD_AUDIO).is_err(),
        "a second drive"
    );
    assert_eq!(opening(&mut m, MCI_DEVTYPE_CD_AUDIO), Ok(0));
    let device = m.arena.read_u32(DATA + 4).expect("in the arena");
    assert_ne!(device, 0);

    // MCI_SET_PARMS: callback, then the time format.
    m.arena
        .write_u32(DATA + 4, MCI_FORMAT_TMSF)
        .expect("in the arena");
    let set = [device, MCI_SET, MCI_SET_TIME_FORMAT | MCI_WAIT, DATA];
    assert_eq!(call(&mut m, "WINMM", "mciSendCommandA", &set), Ok(0));
    assert!(call(
        &mut m,
        "WINMM",
        "mciSendCommandA",
        &[device, MCI_STATUS, 0x100, DATA]
    )
    .is_err());
    for message in [MCI_STOP, MCI_CLOSE] {
        let command = [device, message, MCI_WAIT, DATA];
        assert_eq!(call(&mut m, "WINMM", "mciSendCommandA", &command), Ok(0));
    }
    let stop = [device, MCI_STOP, MCI_WAIT, DATA];
    assert!(
        call(&mut m, "WINMM", "mciSendCommandA", &stop).is_err(),
        "closed"
    );

    m.arena.write(OUT, b"open cdaudio\0").expect("in the arena");
    assert_eq!(
        call(&mut m, "WINMM", "mciSendStringA", &[OUT, 0, 0, 0]),
        Ok(0),
        "as the device opened by name"
    );
}

/// A MIDI stream on a machine with no MIDI port is out of range, the mapper
/// included, as `midiStreamOpen`'s page lists and every Windows Wine's
/// conformance tests ran on answers; a stream of two devices, or with
/// nowhere to say which, is invalid; and no stream is handed back (#149).
#[test]
fn a_midi_stream_on_a_machine_with_no_port_is_refused() {
    const MIDI_MAPPER: u32 = 0xffff_ffff;
    const CALLBACK_FUNCTION: u32 = 0x0003_0000;
    const MMSYSERR_BADDEVICEID: u32 = 2;
    const MMSYSERR_INVALPARAM: u32 = 11;
    let mut m = machine();
    m.arena.write_u32(OUT, 0x5a5a_5a5a).expect("in the arena");
    let open = [OUT, DATA, 1, 0x0041_3d70, 0, CALLBACK_FUNCTION];
    for device in [MIDI_MAPPER, 0] {
        m.arena.write_u32(DATA, device).expect("in the arena");
        assert_eq!(
            call(&mut m, "WINMM", "midiStreamOpen", &open),
            Ok(MMSYSERR_BADDEVICEID),
            "device {device:#x}"
        );
    }
    for asked in [[OUT, DATA, 2], [OUT, 0, 1], [0, DATA, 1]] {
        let mut wrong = open;
        wrong[..3].copy_from_slice(&asked);
        assert_eq!(
            call(&mut m, "WINMM", "midiStreamOpen", &wrong),
            Ok(MMSYSERR_INVALPARAM),
            "{asked:x?}"
        );
    }
    assert_eq!(
        m.arena.read_u32(OUT).expect("in the arena"),
        0x5a5a_5a5a,
        "no stream"
    );
}

/// The system's language is the declared machine's one locale, US English
/// (#147).
#[test]
fn the_system_language_is_the_declared_locale_s() {
    let mut m = machine();
    assert_eq!(
        call(&mut m, "KERNEL32", "GetSystemDefaultLangID", &[]),
        call(&mut m, "KERNEL32", "GetUserDefaultLangID", &[])
    );
    assert_eq!(
        call(&mut m, "KERNEL32", "GetSystemDefaultLangID", &[]),
        Ok(0x0409)
    );
}

/// The declared machine has no screen saver, so screen saving is not enabled;
/// and the screen saver a title says is running is kept, each setting handing
/// back the one it replaced (#147).
#[test]
fn no_screen_saver_is_enabled_and_one_said_to_run_is_kept() {
    const SPI_GETSCREENSAVEACTIVE: u32 = 0x0010;
    const SPI_SETSCREENSAVERRUNNING: u32 = 0x0061;
    let mut m = machine();
    m.arena.write_u32(OUT, 7).expect("in the arena");
    let spi = |m: &mut Machine, action, param| {
        call(
            m,
            "USER32",
            "SystemParametersInfoA",
            &[action, param, OUT, 0],
        )
    };
    assert_eq!(spi(&mut m, SPI_GETSCREENSAVEACTIVE, 0), Ok(1));
    assert_eq!(m.arena.read_u32(OUT).expect("in the arena"), 0);
    assert_eq!(
        call(
            &mut m,
            "USER32",
            "SystemParametersInfoA",
            &[SPI_GETSCREENSAVEACTIVE, 0, 0, 0]
        ),
        Ok(0),
        "nowhere to say"
    );

    // Only what LEGO Chess asks: anything else is a wall.
    let spi_all =
        |m: &mut Machine, args: [u32; 4]| call(m, "USER32", "SystemParametersInfoA", &args);
    for asked in [
        [SPI_GETSCREENSAVEACTIVE, 1, OUT, 0],
        [SPI_GETSCREENSAVEACTIVE, 0, OUT, 2],
        [SPI_SETSCREENSAVERRUNNING, 2, OUT, 0],
        [SPI_SETSCREENSAVERRUNNING, 1, 0, 0],
        [SPI_SETSCREENSAVERRUNNING, 1, OUT, 1],
    ] {
        assert!(spi_all(&mut m, asked).is_err(), "{asked:x?}");
    }

    for (running, was) in [(1, 0), (1, 1), (0, 1), (0, 0)] {
        m.arena.write_u32(OUT, 7).expect("in the arena");
        assert_eq!(spi(&mut m, SPI_SETSCREENSAVERRUNNING, running), Ok(1));
        assert_eq!(
            m.arena.read_u32(OUT).expect("in the arena"),
            was,
            "set to {running}"
        );
    }
}

/// The multimedia system time is `timeGetTime`'s, in an `MMTIME` that says it
/// holds milliseconds; one too small to hold it is a wall (#147).
#[test]
fn the_system_time_is_milliseconds_in_an_mmtime() {
    const TIME_MS: u32 = 0x0001;
    let mut m = machine();
    for offset in [0, 4, 8] {
        m.arena
            .write_u32(OUT + offset, 0xdead_beef)
            .expect("in the arena");
    }
    assert_eq!(
        call(&mut m, "WINMM", "timeGetSystemTime", &[OUT, 12]),
        Ok(0)
    );
    let now = call(&mut m, "WINMM", "timeGetTime", &[]).expect("answered");
    let read = |o| m.arena.read_u32(OUT + o).expect("in the arena");
    assert_eq!(
        (read(0), read(4), read(8)),
        (TIME_MS, now, 0xdead_beef),
        "the union's other half left alone"
    );
    // Too small, or none: a wall, with nothing written.
    m.arena.write_u32(OUT, 0xdead_beef).expect("in the arena");
    assert!(call(&mut m, "WINMM", "timeGetSystemTime", &[OUT, 8]).is_err());
    assert_eq!(m.arena.read_u32(OUT).expect("in the arena"), 0xdead_beef);
    assert!(call(&mut m, "WINMM", "timeGetSystemTime", &[0, 12]).is_err());
}

/// DirectX 5's device lists each texture format in a whole `DDSURFACEDESC`:
/// its size, flags saying it holds caps and a pixel format, the format at 72
/// and a texture's caps at 104; with no callback it is refused (#148).
#[test]
fn direct3d_2_lists_its_texture_formats_in_surface_descriptions() {
    const CALLBACK: u32 = 0x0040_2000;
    let mut m = machine();
    let list = |m: &mut Machine, callback: u32| {
        let shim = win32::com_method("IDirect3DDevice2", 9).expect("EnumTextureFormats");
        with_ctx(m, |ctx| {
            (
                (shim.run)(ctx, 0, &[callback, 0x1234]),
                ctx.call_guest.take(),
            )
        })
    };
    assert_eq!(list(&mut m, 0).0, Ok(0x8007_0057), "no callback");
    let (answer, program) = list(&mut m, CALLBACK);
    assert_eq!(answer, Ok(0));
    // The program calls the callback with each description, pushed last
    // first after the context; the first is five-six-five.
    let (at, _) = program.expect("the callback is called");
    let code = m.arena.slice(at, 16).expect("in the arena").to_vec();
    assert_eq!(
        code[..5],
        [0x68, 0x34, 0x12, 0, 0],
        "the context pushed first"
    );
    assert_eq!(code[5], 0x68);
    let desc = u32::from_le_bytes(code[6..10].try_into().expect("four bytes"));
    let read = |o: u32| m.arena.read_u32(desc + o).expect("in the arena");
    assert_eq!((read(0), read(4), read(104)), (108, 0x1001, 0x1000));
    // DDPIXELFORMAT at 72: its size, flags, bits, then the masks.
    assert_eq!(read(72), 32);
    assert_eq!(
        (
            read(72 + 12),
            read(72 + 16),
            read(72 + 20),
            read(72 + 24),
            read(72 + 28)
        ),
        (16, 0xf800, 0x07e0, 0x001f, 0)
    );
}

/// The viewport DirectX 5's Direct3D makes is one its device takes, and is
/// asked for by its DirectX 5 and DirectX 3 names too; one made inside
/// another object is refused (#148).
#[test]
fn a_direct3d_2_viewport_is_one_its_device_takes() {
    let mut m = machine();
    let d3d2 = with_ctx(&mut m, |ctx| {
        win32::State::make_com(
            ctx,
            win32::interface_named("IDirect3D2").expect("written"),
            win32::ComState::None,
        )
    })
    .expect("made");
    assert_eq!(
        method(&mut m, "IDirect3D2", "CreateViewport", d3d2, &[OUT, 1]),
        Ok(0x8004_0110),
        "inside another object"
    );
    assert_eq!(
        method(&mut m, "IDirect3D2", "CreateViewport", d3d2, &[OUT, 0]),
        Ok(0)
    );
    let viewport = m.arena.read_u32(OUT).expect("in the arena");
    let device = with_ctx(&mut m, |ctx| {
        win32::State::make_com(
            ctx,
            win32::interface_named("IDirect3DDevice2").expect("written"),
            win32::ComState::Device3D { surface: 0 },
        )
    })
    .expect("made");
    assert_eq!(
        method(
            &mut m,
            "IDirect3DDevice2",
            "AddViewport",
            device,
            &[viewport]
        ),
        Ok(0)
    );
    for iid in [
        [
            0x00, 0x15, 0x28, 0x93, 0xf8, 0x8c, 0xd0, 0x11, 0x89, 0xab, 0x00, 0xa0, 0xc9, 0x05,
            0x41, 0x29,
        ],
        [
            0x46, 0xc1, 0x17, 0x44, 0xad, 0x33, 0xcf, 0x11, 0x81, 0x6f, 0x00, 0x00, 0xc0, 0x20,
            0x15, 0x6e,
        ],
    ] {
        m.arena.write(DATA, &iid).expect("in the arena");
        assert_eq!(
            method(
                &mut m,
                "IDirect3DViewport3",
                "QueryInterface",
                viewport,
                &[DATA, OUT]
            ),
            Ok(0)
        );
        assert_eq!(m.arena.read_u32(OUT).expect("in the arena"), viewport);
    }
}

/// A message sent to the title's own window is its procedure's to answer, and
/// its answer is the call's (#147): LEGO Chess tells its window its size.
#[test]
fn a_message_sent_to_the_title_s_window_is_its_procedure_s_to_answer() {
    const PROCEDURE: u32 = 0x0040_1000;
    const WM_SIZE: u32 = 0x0005;
    let mut m = machine();
    let send = |m: &mut Machine| {
        let shim = win32::find("USER32", "SendMessageA").expect("written");
        with_ctx(m, |ctx| {
            let answer = (shim.run)(ctx, &[WINDOW, WM_SIZE, 0, 0x01e0_0280]);
            (
                answer.is_ok(),
                ctx.call_guest.take(),
                ctx.answer_is_the_call_s,
            )
        })
    };
    assert!(!send(&mut m).0, "no window yet: a wall");
    window(&mut m, PROCEDURE);
    assert_eq!(
        send(&mut m),
        (
            true,
            Some((PROCEDURE, vec![WINDOW, WM_SIZE, 0, 0x01e0_0280])),
            true
        )
    );
}

/// A window invalidated is handed `WM_PAINT` once nothing else waits, by
/// `PeekMessageA` and `GetMessageA` alike, and again until `BeginPaint`
/// empties it; `UpdateWindow` sends it at once while there is one (#147).
#[test]
fn an_invalidated_window_is_painted_until_it_begins_to_paint() {
    const PROCEDURE: u32 = 0x0040_1000;
    const WM_PAINT: u32 = 0x000f;
    let mut m = machine();
    window(&mut m, PROCEDURE);
    let message = |m: &Machine| m.arena.read_u32(OUT + 4).expect("in the arena");
    // What making the window posted comes first.
    while call(&mut m, "USER32", "PeekMessageA", &[OUT, 0, 0, 0, 1]) == Ok(1) {}
    let update = |m: &mut Machine| {
        let shim = win32::find("USER32", "UpdateWindow").expect("written");
        with_ctx(m, |ctx| ((shim.run)(ctx, &[WINDOW]), ctx.call_guest.take()))
    };
    assert_eq!(update(&mut m), (Ok(1), None), "nothing to draw");
    assert!(
        call(&mut m, "USER32", "GetMessageA", &[OUT, 0, 0, 0]).is_err(),
        "nothing waits: a wall, not a wait for ever"
    );

    // No window named is every window, which here is the title's.
    assert_eq!(call(&mut m, "USER32", "InvalidateRect", &[0, 0, 1]), Ok(1));
    assert_eq!(
        update(&mut m),
        (Ok(1), Some((PROCEDURE, vec![WINDOW, WM_PAINT, 0, 0])))
    );
    assert_eq!(
        call(&mut m, "USER32", "PeekMessageA", &[OUT, 0, 0, 0, 1]),
        Ok(1)
    );
    assert_eq!(message(&m), WM_PAINT);
    assert_eq!(
        call(&mut m, "USER32", "GetMessageA", &[OUT, 0, 0, 0]),
        Ok(1)
    );
    assert_eq!(message(&m), WM_PAINT, "taken, it is still there");

    let dc = call(&mut m, "USER32", "BeginPaint", &[WINDOW, DATA]).expect("answered");
    assert_ne!(dc, 0);
    let paint: Vec<u32> = (0..6)
        .map(|i| m.arena.read_u32(DATA + 4 * i).expect("in the arena"))
        .collect();
    assert_eq!(
        paint,
        [dc, 1, 0, 0, 640, 480],
        "its background still to erase"
    );
    assert_eq!(call(&mut m, "USER32", "EndPaint", &[WINDOW, DATA]), Ok(1));
    assert_eq!(
        call(&mut m, "USER32", "PeekMessageA", &[OUT, 0, 0, 0, 1]),
        Ok(0),
        "painted"
    );

    // With nothing due, nothing to draw and nothing to erase.
    let dc = call(&mut m, "USER32", "BeginPaint", &[WINDOW, DATA]).expect("answered");
    let paint: Vec<u32> = (0..6)
        .map(|i| m.arena.read_u32(DATA + 4 * i).expect("in the arena"))
        .collect();
    assert_eq!(paint, [dc, 0, 0, 0, 0, 0], "nothing due");
    // A rectangle of its own is not kept: a wall.
    assert!(call(&mut m, "USER32", "InvalidateRect", &[WINDOW, DATA, 0]).is_err());
    // No window, or one never made, is not updated; the desktop is.
    assert_eq!(call(&mut m, "USER32", "UpdateWindow", &[0]), Ok(0));
    assert_eq!(
        call(&mut m, "USER32", "UpdateWindow", &[0x0001_2345]),
        Ok(0)
    );
    assert_eq!(
        call(&mut m, "USER32", "UpdateWindow", &[0x0001_0001]),
        Ok(1)
    );

    // A procedure that hands it to `DefWindowProcA` has painted it too.
    assert_eq!(
        call(&mut m, "USER32", "InvalidateRect", &[WINDOW, 0, 0]),
        Ok(1)
    );
    assert_eq!(
        call(
            &mut m,
            "USER32",
            "DefWindowProcA",
            &[WINDOW, WM_PAINT, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        call(&mut m, "USER32", "PeekMessageA", &[OUT, 0, 0, 0, 1]),
        Ok(0),
        "painted by Windows' own procedure"
    );
}

/// `GetMessageA` answers zero for the quit a thread asked for, and nonzero
/// for anything else (#147).
#[test]
fn get_message_answers_zero_for_a_quit() {
    const PROCEDURE: u32 = 0x0040_1000;
    let mut m = machine();
    window(&mut m, PROCEDURE);
    assert_eq!(
        call(&mut m, "USER32", "GetMessageA", &[OUT, 0, 0, 0]),
        Ok(1)
    );
    assert_eq!(call(&mut m, "USER32", "PostQuitMessage", &[3]), Ok(0));
    // The rest of what making the window posted, then the quit.
    while call(&mut m, "USER32", "GetMessageA", &[OUT, 0, 0, 0]) == Ok(1) {}
    assert_eq!(m.arena.read_u32(OUT + 4).expect("in the arena"), 0x0012);
    assert_eq!(m.arena.read_u32(OUT + 8).expect("in the arena"), 3);
    assert_eq!(
        call(&mut m, "USER32", "GetMessageA", &[0, 0, 0, 0]),
        Ok(u32::MAX),
        "nowhere to put it"
    );
}

/// A RIFF chunk, padded to an even length as RIFF pads one.
fn riff(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(body);
    out.resize(out.len().next_multiple_of(2), 0);
    out
}

/// A RIFF list of one kind holding these chunks.
fn riff_list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
    riff(b"LIST", &[kind.to_vec(), parts.concat()].concat())
}

fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A movie laid out as LEGO Chess's are (#151): a 4x2 Indeo 5 video of three
/// frames at 15 a second whose header's frame is written as 32-bit edges, and
/// so read as vfw.h's 16-bit ones is empty, with a name; and an 8-bit mono
/// sound at 22,050 Hz with none. Its main header counts four frames and says
/// it has `streams`.
fn movie(streams: u32) -> Vec<u8> {
    // MainAVIHeader: microseconds a frame, the most bytes a second, padding,
    // flags, frames, initial frames, streams, buffer, width, height, reserved.
    let avih = words(&[
        66_667, 22_400, 0, 0x110, 4, 0, streams, 4_096, 4, 2, 0, 0, 0, 0,
    ]);
    // AVIStreamHeader: type, handler, flags, priority and language, initial
    // frames, scale, rate, start, length, buffer, quality, sample size and
    // the frame, the video's written as four 32-bit edges in 64 bytes.
    let video = [
        b"vidsIV50".to_vec(),
        words(&[0, 0, 0, 1, 15, 0, 3, 2_048, 10_000, 0, 0, 0, 4, 2]),
    ]
    .concat();
    let sound = [
        b"auds\0\0\0\0".to_vec(),
        words(&[0, 0, 11, 1, 22_050, 0, 4_410, 1_470, 0, 1, 0, 0]),
    ]
    .concat();
    // BITMAPINFOHEADER: 4x2, one plane, 24 bits, IV50.
    let picture = [
        words(&[40, 4, 2]),
        vec![1, 0, 24, 0],
        b"IV50".to_vec(),
        vec![0; 20],
    ]
    .concat();
    // PCMWAVEFORMAT: PCM, one channel, 22,050 a second of one byte each.
    let pcm = [vec![1, 0, 1, 0], words(&[22_050, 22_050]), vec![1, 0, 8, 0]].concat();
    let header = riff_list(
        b"hdrl",
        &[
            riff(b"avih", &avih),
            riff_list(
                b"strl",
                &[
                    riff(b"strh", &video),
                    riff(b"strf", &picture),
                    riff(b"strn", b"movie.avi Video #1\0"),
                ],
            ),
            riff_list(b"strl", &[riff(b"strh", &sound), riff(b"strf", &pcm)]),
        ],
    );
    let movi = riff_list(b"movi", &[riff(b"00dc", &[0x1f, 0])]);
    riff(b"RIFF", &[b"AVI ".to_vec(), header, movi].concat())
}

const MOVIE: &str = "C:\\STUB\\MOVIE.AVI";

/// A movie opened through AVIFile's C API, as LEGO Chess opens its own
/// (#151): what it holds and each stream's header, its frame filled from
/// the picture for a video stream whose own is empty, and its format.
#[test]
fn a_movie_opened_through_avifile_answers_its_headers_and_its_formats() {
    const VIDS: u32 = u32::from_le_bytes(*b"vids");
    const AUDS: u32 = u32::from_le_bytes(*b"auds");
    const INFO: u32 = OUT + 0x100;
    let mut m = machine();
    m.tree.declare(MOVIE, movie(2));
    m.arena
        .write(DATA, format!("{MOVIE}\0").as_bytes())
        .expect("in the arena");
    let read = |m: &Machine, at: u32| m.arena.read_u32(at).expect("in the arena");

    // OF_READ, through the handler its extension picks.
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
        Ok(0)
    );
    let file = read(&m, OUT);
    assert_ne!(file, 0);

    // AVIFILEINFOA, worked out from the streams as Wine works it out: the
    // most bytes a second, the video's biggest chunk at 15 a second and the
    // sound's 22,050 one-byte samples; the main header's flags; the caps; the
    // main header's two streams; the biggest chunk; the video's frame; the
    // first stream's time scale; and the longest stream's length in it,
    // whatever the main header counts.
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileInfoA", &[file, INFO, 108]),
        Ok(0)
    );
    let fields: Vec<u32> = (0..10).map(|i| read(&m, INFO + 4 * i)).collect();
    assert_eq!(fields, [2 * 15 + 22_050, 0x110, 3, 2, 2, 4, 2, 1, 15, 3]);
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileInfoA", &[file, INFO, 107]),
        Ok(0x8004_406b),
        "AVIERR_BADSIZE"
    );

    // Its streams by number, the same one each time it is asked for, and by
    // type; none past the last.
    let get = |m: &mut Machine, kind: u32, n: u32| {
        let answer = call(m, "AVIFIL32", "AVIFileGetStream", &[file, OUT + 4, kind, n]);
        (answer, read(m, OUT + 4))
    };
    let (answer, video) = get(&mut m, 0, 0);
    assert_eq!(answer, Ok(0));
    let (answer, sound) = get(&mut m, 0, 1);
    assert_eq!(answer, Ok(0));
    assert_ne!(video, sound);
    assert_eq!(get(&mut m, AUDS, 0), (Ok(0), sound));
    assert_eq!(get(&mut m, VIDS, 0), (Ok(0), video));
    assert_eq!(get(&mut m, 0, 2), (Ok(0x8004_4073), 0), "AVIERR_NODATA");
    assert_eq!(get(&mut m, VIDS, 1), (Ok(0x8004_4073), 0));
    // Asked for twice and once more by hand.
    assert_eq!(call(&mut m, "AVIFIL32", "AVIStreamAddRef", &[sound]), Ok(3));

    // AVISTREAMINFOA: the stream's header, and its name: the one written in
    // it, or the one AVIFile makes up for a stream with none.
    let info = |m: &mut Machine, stream: u32| {
        assert_eq!(
            call(m, "AVIFIL32", "AVIStreamInfoA", &[stream, INFO, 140]),
            Ok(0)
        );
        let fields: Vec<u32> = (0..18).map(|i| read(m, INFO + 4 * i)).collect();
        (fields, text(m, INFO + 76))
    };
    let (fields, name) = info(&mut m, video);
    assert_eq!(fields[..2], [VIDS, u32::from_le_bytes(*b"IV50")]);
    // Its scale, rate, start, length; its buffer, its biggest chunk, not its
    // header's 2,048; and its frame, filled from the picture.
    assert_eq!(fields[5..9], [1, 15, 0, 3]);
    assert_eq!(fields[10], 2);
    assert_eq!(fields[13..17], [0, 0, 4, 2]);
    assert_eq!(name, "movie.avi Video #1");
    let (fields, name) = info(&mut m, sound);
    assert_eq!(fields[0], AUDS);
    // Its initial frames, a buffer of nothing for a stream with no chunks,
    // its sample size, and a frame nobody fills.
    assert_eq!((fields[9], fields[10], fields[12]), (11, 0, 1));
    assert_eq!(fields[13..17], [0; 4]);
    assert_eq!(name, "MOVIE.AVI Audio #1");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIStreamInfoA", &[sound, INFO, 139]),
        Ok(0x8004_406b)
    );

    // The format: how long it is, then as much of it as fits.
    let format = |m: &mut Machine, buffer: u32, room: u32| {
        m.arena.write(INFO, &[0xcc; 16]).expect("in the arena");
        m.arena.write_u32(OUT + 8, room).expect("in the arena");
        let answer = call(
            m,
            "AVIFIL32",
            "AVIStreamReadFormat",
            &[sound, 0, buffer, OUT + 8],
        );
        (answer, read(m, OUT + 8))
    };
    assert_eq!(format(&mut m, 0, 0), (Ok(0), 16));
    assert_eq!(format(&mut m, INFO, 16), (Ok(0), 16));
    assert_eq!(m.arena.slice(INFO, 4).expect("in the arena"), [1, 0, 1, 0]);
    assert_eq!(
        format(&mut m, INFO, 4),
        (Ok(0x8004_4074), 16),
        "AVIERR_BUFFERTOOSMALL"
    );
    assert_eq!(
        m.arena.slice(INFO, 6).expect("in the arena"),
        [1, 0, 1, 0, 0xcc, 0xcc]
    );
}

/// A sound's sample size is its format's block, whatever its header says, as
/// Wine's conformance tests `test_ash1_corruption` and
/// `test_ash1_corruption2`, which pass on Windows, find it (#151).
#[test]
fn a_sound_s_sample_size_is_its_format_s_block() {
    let size = |bytes: Vec<u8>| {
        let mut m = machine();
        m.tree.declare(MOVIE, bytes);
        m.arena
            .write(DATA, format!("{MOVIE}\0").as_bytes())
            .expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
            Ok(0)
        );
        let file = m.arena.read_u32(OUT).expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, 1]),
            Ok(0)
        );
        let sound = m.arena.read_u32(OUT).expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIStreamInfoA", &[sound, OUT + 4, 140]),
            Ok(0)
        );
        m.arena.read_u32(OUT + 4 + 48).expect("in the arena")
    };
    // The sound's header and its format, found by what they say.
    let at = |bytes: &[u8], needle: &[u8]| {
        bytes
            .windows(needle.len())
            .position(|w| w == needle)
            .expect("in the movie")
    };
    let mut corrupt = movie(2);
    let header = at(&corrupt, b"auds");
    corrupt[header + 44..header + 48].copy_from_slice(&0xdead_beef_u32.to_le_bytes());
    assert_eq!(size(corrupt), 1);
    let mut corrupt = movie(2);
    let format = at(&corrupt, &[1, 0, 1, 0, 0x22, 0x56, 0, 0]);
    corrupt[format + 12..format + 14].copy_from_slice(&0xdead_u16.to_le_bytes());
    assert_eq!(size(corrupt), 0xdead);
}

/// The intro's three movies, off LEGO Chess's disc, as AVIStreamInfoA answers
/// them (#151): Indeo 5 at 15 a second; their frames, the logo's its own and
/// Krisalis' and the intro's written as 32-bit edges and so empty, filled
/// from their pictures; and their sounds' sample sizes, 8-bit mono's and
/// 16-bit stereo's blocks.
#[test]
fn the_intro_s_movies_answer_their_streams_off_the_disc() {
    let image = &roster::LEGO_CHESS.image;
    if !testkit::rips(image) {
        return;
    }
    let mut m = machine();
    m.tree
        .set_disc(disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens"));
    for (name, frames, frame, sample) in [
        ("LMI", 280, [0, 0, 320, 240], Some(1)),
        ("KRISALIS", 38, [0, 0, 320, 240], None),
        ("INTRO", 499, [0, 0, 320, 200], Some(4)),
    ] {
        m.arena
            .write(DATA, format!("D:\\INTRO\\{name}.AVI\0").as_bytes())
            .expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
            Ok(0)
        );
        let file = m.arena.read_u32(OUT).expect("in the arena");
        let info = |m: &mut Machine, n: u32| {
            let found = call(m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, n]);
            if found != Ok(0) {
                return None;
            }
            let stream = m.arena.read_u32(OUT).expect("in the arena");
            assert_eq!(
                call(m, "AVIFIL32", "AVIStreamInfoA", &[stream, OUT + 4, 140]),
                Ok(0)
            );
            Some(
                (0..17)
                    .map(|i| m.arena.read_u32(OUT + 4 + 4 * i).expect("in the arena"))
                    .collect::<Vec<u32>>(),
            )
        };
        let video = info(&mut m, 0).expect("a video stream");
        assert_eq!(
            video[..2],
            [u32::from_le_bytes(*b"vids"), u32::from_le_bytes(*b"IV50")],
            "{name}"
        );
        assert_eq!((video[5], video[6], video[8]), (1, 15, frames), "{name}");
        assert_eq!(video[13..17], frame.map(|e: i32| e as u32), "{name}");
        let sound = info(&mut m, 1).map(|s| {
            assert_eq!(s[0], u32::from_le_bytes(*b"auds"), "{name}");
            s[12]
        });
        assert_eq!(sound, sample, "{name}");
    }
}

/// A movie that is not there is one the handler cannot open, which LEGO
/// Chess skips (#151). One whose header says it holds more streams than it
/// does is a damaged rip, which Wine fails to load, and a mode other than
/// reading is not one this machine opens a movie in: each is a wall. One that
/// holds more than its header says is read as Wine reads it, as the streams
/// its header counts.
#[test]
fn a_movie_not_there_cannot_be_opened_and_a_damaged_one_is_a_wall() {
    let mut m = machine();
    m.arena
        .write(DATA, format!("{MOVIE}\0").as_bytes())
        .expect("in the arena");
    m.arena.write_u32(OUT, 0xdead).expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
        Ok(0x8004_406f),
        "AVIERR_FILEOPEN"
    );
    assert_eq!(m.arena.read_u32(OUT).expect("in the arena"), 0);
    m.tree.declare(MOVIE, movie(3));
    let wall = call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]);
    assert!(
        matches!(&wall, Err(Stop::NotWritten { what, .. })
            if what.contains("says 3 streams and it holds 2")),
        "{wall:?}"
    );
    m.tree.declare(MOVIE, movie(2));
    // OF_WRITE.
    let wall = call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 1, 0]);
    assert!(
        matches!(&wall, Err(Stop::NotWritten { what, .. }) if what.contains("mode 0x1")),
        "{wall:?}"
    );
    // A header that counts one stream of two: one stream.
    m.tree.declare("C:\\STUB\\ONE.AVI", movie(1));
    m.arena
        .write(DATA, b"C:\\STUB\\ONE.AVI\0")
        .expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
        Ok(0)
    );
    let file = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, 1]),
        Ok(0x8004_4073)
    );
}

/// A BITMAPINFOHEADER asking for a plain DIB: its size, the picture's, one
/// plane, so many bits, no codec, and the image's size.
fn dib_header(width: u32, height: u32, bits: u32, image: u32) -> Vec<u8> {
    [
        words(&[40, width, height, 1 | bits << 16, 0, image]),
        vec![0; 16],
    ]
    .concat()
}

/// A frame getter is for a video stream, in a plain DIB the movie's size
/// (#151): a sound stream has none, and the decompressor's own format or a
/// depth other than 16 or 24 bits is not one this machine chooses. A frame
/// before the first or past the last is none, and one whose picture will not
/// decode is a wall that names it.
#[test]
fn a_frame_getter_is_for_a_video_stream_in_a_plain_dib() {
    const WANTED: u32 = DATA + 0x200;
    let mut m = machine();
    m.tree.declare(MOVIE, movie(2));
    m.arena
        .write(DATA, format!("{MOVIE}\0").as_bytes())
        .expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
        Ok(0)
    );
    let file = m.arena.read_u32(OUT).expect("in the arena");
    let stream = |m: &mut Machine, n: u32| {
        assert_eq!(
            call(m, "AVIFIL32", "AVIFileGetStream", &[file, OUT + 4, 0, n]),
            Ok(0)
        );
        m.arena.read_u32(OUT + 4).expect("in the arena")
    };
    let (video, sound) = (stream(&mut m, 0), stream(&mut m, 1));
    let open = |m: &mut Machine, stream: u32, header: &[u8]| {
        m.arena.write(WANTED, header).expect("in the arena");
        call(m, "AVIFIL32", "AVIStreamGetFrameOpen", &[stream, WANTED])
    };

    assert_eq!(open(&mut m, sound, &dib_header(4, 2, 16, 16)), Ok(0));
    for (header, says) in [
        (dib_header(4, 2, 32, 32), "a 32-bit DIB"),
        (dib_header(8, 2, 16, 32), "8x2"),
        (
            [words(&[40, 4, 2, 16 << 16]), vec![0; 24]].concat(),
            "in 0 planes",
        ),
        (dib_header(4, 2, 24, 0), ""),
    ] {
        let answer = open(&mut m, video, &header);
        if says.is_empty() {
            assert!(matches!(answer, Ok(g) if g != 0), "{answer:?}");
            continue;
        }
        assert!(
            matches!(&answer, Err(Stop::NotWritten { what, .. }) if what.contains(says)),
            "{answer:?}"
        );
    }
    let own = call(&mut m, "AVIFIL32", "AVIStreamGetFrameOpen", &[video, 0]);
    assert!(
        matches!(&own, Err(Stop::NotWritten { what, .. }) if what.contains("own format")),
        "{own:?}"
    );

    let getter = open(&mut m, video, &dib_header(4, 2, 16, 16)).expect("a getter");
    let frame =
        |m: &mut Machine, n: i32| call(m, "AVIFIL32", "AVIStreamGetFrame", &[getter, n as u32]);
    // The movie has one chunk, which is no picture.
    assert_eq!(frame(&mut m, -1), Ok(0));
    assert_eq!(frame(&mut m, 1), Ok(0));
    let wall = frame(&mut m, 0);
    assert!(
        matches!(&wall, Err(Stop::NotWritten { what, .. })
            if what.contains("frame 0 of C:\\STUB\\MOVIE.AVI: IV50")),
        "{wall:?}"
    );
}

/// A movie opened again is the same bytes, held once however often it is
/// opened (#151): LEGO Chess opens each of its movies anew every time it plays
/// one, and never lets go of a file. A picture too big for memory, which only
/// a damaged header could say, stops at a wall that names it.
#[test]
fn a_movie_opened_again_is_held_once_and_a_huge_picture_is_a_wall() {
    const WANTED: u32 = DATA + 0x200;
    let mut m = machine();
    // The picture's BITMAPINFOHEADER says 4 wide and 0x0FFF_FFFF high.
    let mut huge = movie(2);
    let header = huge
        .windows(12)
        .position(|w| w == words(&[40, 4, 2]))
        .expect("in the movie");
    huge[header + 8..header + 12].copy_from_slice(&0x0fff_ffff_u32.to_le_bytes());
    m.tree.declare(MOVIE, huge);
    m.arena
        .write(DATA, format!("{MOVIE}\0").as_bytes())
        .expect("in the arena");
    for _ in 0..2 {
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
            Ok(0)
        );
    }
    assert_eq!(m.state.movie_bytes.len(), 1);
    let bytes = m.state.movie_bytes.values().next().expect("one copy");
    assert_eq!(
        std::rc::Rc::strong_count(bytes),
        3,
        "the map's and the two files'"
    );

    let file = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, 0]),
        Ok(0)
    );
    let video = m.arena.read_u32(OUT).expect("in the arena");
    m.arena
        .write(WANTED, &dib_header(4, 0x0fff_ffff, 16, 0))
        .expect("in the arena");
    let wall = call(
        &mut m,
        "AVIFIL32",
        "AVIStreamGetFrameOpen",
        &[video, WANTED],
    );
    assert!(
        matches!(&wall, Err(Stop::NotWritten { what, .. }) if what.contains("more than memory holds")),
        "{wall:?}"
    );
}

/// LEGO Chess's logo movie, off the disc, frame by frame as the game asks for
/// it (#151): each one a bottom-up 555 DIB behind the header it asked for,
/// decoded in order across its null pictures (frames 1 and 3), a frame
/// before the last one decoded starting again from the first, and the same
/// frame again the DIB as it is.
///
/// The pinned bits are a reference decoder's pictures turned into the DIB the
/// way `Picture::rgb` and `dib_bits` turn one: FFmpeg 9.0.1, run once by hand
/// on a copy of the disc's `INTRO\LMI.AVI`,
///
///   ffmpeg -i LMI.AVI -map 0:v:0 -f rawvideo -pix_fmt yuv410p
///     -fps_mode passthrough -
///
/// writes the 278 pictures it shows of the 280 (frames 1 and 3 are null), so
/// frame 5 is the fourth it writes and frame 8 the seventh. Each pixel goes
/// through BT.601 at studio range in Microsoft's integer form, its chroma from
/// its 4x4 block, and is narrowed to 555 by dropping each channel's low bits;
/// the rows go bottom-up, padded to a word; and the digest is the sha256 of
/// the DIB's 153,600 bytes of bits.
#[test]
fn the_logo_s_frames_are_bottom_up_555_dibs_decoded_in_order() {
    const WANTED: u32 = DATA + 0x200;
    let image = &roster::LEGO_CHESS.image;
    if !testkit::rips(image) {
        return;
    }
    let mut m = machine();
    m.tree
        .set_disc(disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens"));
    m.arena
        .write(DATA, b"D:\\INTRO\\LMI.AVI\0")
        .expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
        Ok(0)
    );
    let file = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, 0]),
        Ok(0)
    );
    let video = m.arena.read_u32(OUT).expect("in the arena");
    m.arena
        .write(WANTED, &dib_header(320, 240, 16, 153_600))
        .expect("in the arena");
    let getter = call(
        &mut m,
        "AVIFIL32",
        "AVIStreamGetFrameOpen",
        &[video, WANTED],
    )
    .expect("a getter");
    assert_ne!(getter, 0);

    let frame = |m: &mut Machine, n: i32| {
        let dib = call(m, "AVIFIL32", "AVIStreamGetFrame", &[getter, n as u32]).expect("answered");
        (dib != 0).then(|| {
            let header = m.arena.slice(dib, 40).expect("in the arena").to_vec();
            assert_eq!(header, dib_header(320, 240, 16, 153_600));
            let bits = m.arena.slice(dib + 40, 153_600).expect("in the arena");
            (dib, hash::sha256_hex(bits))
        })
    };
    const FIVE: &str = "27b8b87b4b8533454a7fd956d5072a712cc39310fc2b6ac092378e8f10736224";
    const EIGHT: &str = "25ab8cbaed07d27fb4a4af1282dcfaa3c6a81b0496696d146629e9bce61786d1";
    let (dib, five) = frame(&mut m, 5).expect("frame 5");
    assert_eq!(five, FIVE);
    assert_eq!(frame(&mut m, 5), Some((dib, five)));
    assert_eq!(
        frame(&mut m, 8).map(|(_, bits)| bits).as_deref(),
        Some(EIGHT)
    );
    assert_eq!(
        frame(&mut m, 5).map(|(_, bits)| bits).as_deref(),
        Some(FIVE)
    );
    assert!(frame(&mut m, 279).is_some());
    assert_eq!(frame(&mut m, 280), None);
    assert_eq!(frame(&mut m, -1), None);
}

/// A surface hands out one device context at a time, whichever of its faces
/// is asked (#151): a second before the first is let go of is
/// DDERR_DCALREADYCREATED and leaves what it was handed alone, a surface
/// with none out answers DDERR_NODC, and another DC than its own is a wall.
#[test]
fn a_surface_hands_out_one_dc_at_a_time() {
    const GET_DC: usize = 17;
    const RELEASE_DC: usize = 26;
    let mut m = machine();
    let (one, two) = (surface(&mut m, PIXELS, 4, 4), surface(&mut m, TEXELS, 4, 4));
    let read = |m: &Machine| m.arena.read_u32(OUT).expect("in the arena");

    assert_eq!(
        com(&mut m, "IDirectDrawSurface", RELEASE_DC, one, &[0x10]),
        Ok(0x8876_00cb),
        "DDERR_NODC"
    );
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", GET_DC, one, &[OUT]),
        Ok(0)
    );
    let dc = read(&m);
    m.arena.write_u32(OUT, 7).expect("in the arena");
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", GET_DC, one, &[OUT]),
        Ok(0x8876_026c),
        "DDERR_DCALREADYCREATED"
    );
    assert_eq!(read(&m), 7);
    // Its DC out, the surface is locked: Lock answers DDERR_SURFACEBUSY, as
    // Wine's `test_getdc` finds Windows answering, and a blit or a flip, which
    // nothing has observed, is a wall.
    assert_eq!(
        com(
            &mut m,
            "IDirectDrawSurface",
            25,
            one,
            &[0, OUT + 0x40, 0, 0]
        ),
        Ok(0x8876_01ae)
    );
    for (slot, args) in [(5, &[0, 0, 0, 0x400, 0][..]), (11, &[0, 0][..])] {
        let wall = com(&mut m, "IDirectDrawSurface", slot, one, args);
        assert!(
            matches!(&wall, Err(Stop::NotWritten { what, .. }) if what.contains("while its DC is out")),
            "{wall:?}"
        );
    }
    // Its DirectX 6 face is the same surface, with the same DC out.
    let face = with_ctx(&mut m, |ctx| {
        win32::State::face(ctx, one, &win32::ddraw::DIRECT_DRAW_SURFACE4)
    })
    .expect("a face");
    assert_eq!(
        com(&mut m, "IDirectDrawSurface4", GET_DC, face, &[OUT]),
        Ok(0x8876_026c)
    );
    // Another surface's is another DC.
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", GET_DC, two, &[OUT]),
        Ok(0)
    );
    assert_ne!(read(&m), dc);
    let other = read(&m);
    let stranger = com(&mut m, "IDirectDrawSurface", RELEASE_DC, one, &[other]);
    assert!(
        matches!(&stranger, Err(Stop::NotWritten { what, .. }) if what.contains("own DC")),
        "{stranger:?}"
    );
    assert_eq!(
        com(&mut m, "IDirectDrawSurface4", RELEASE_DC, face, &[dc]),
        Ok(0)
    );
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", GET_DC, one, &[OUT]),
        Ok(0)
    );
    assert_eq!(read(&m), dc, "the DC it let go of, handed out again");
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", GET_DC, two, &[0]),
        Ok(0x8007_0057)
    );
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", RELEASE_DC, one, &[dc]),
        Ok(0)
    );
    assert_eq!(
        com(
            &mut m,
            "IDirectDrawSurface",
            25,
            one,
            &[0, OUT + 0x40, 0, 0]
        ),
        Ok(0),
        "let go of, it locks"
    );

    // A screen with no surface behind it has its frame when its DC is let go
    // of, as letting go of a lock of the whole of it does.
    let screen = with_ctx(&mut m, |ctx| {
        win32::State::make_com(
            ctx,
            &win32::ddraw::DIRECT_DRAW_SURFACE,
            win32::ComState::Surface {
                pixels: PIXELS,
                width: 4,
                height: 4,
                pitch: 8,
                format: PixelFormat::RGB565,
                primary: true,
                back: 0,
                palette: 0,
            },
        )
    })
    .expect("a screen");
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", GET_DC, screen, &[OUT]),
        Ok(0)
    );
    m.state.present = None;
    let dc = read(&m);
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", RELEASE_DC, screen, &[dc]),
        Ok(0)
    );
    assert_eq!(
        m.state.present.map(|p| (p.pixels, p.width)),
        Some((PIXELS, 4))
    );
}

/// The 4x4 sixteen-bit surface at `PIXELS`, as its 565 pixels, row by row.
fn four_by_four(m: &Machine) -> Vec<u16> {
    m.arena
        .slice(PIXELS, 32)
        .expect("in the arena")
        .chunks(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect()
}

/// DrawDib draws a plain DIB onto a surface's DC (#151), as LEGO Chess's
/// DirectX plugin draws each movie frame on its back buffer: whole, each
/// pixel repeated as many times over as the destination is the DIB's size,
/// from the bottom row up, and what falls off the surface left out. A 555
/// pixel's channels are widened by their own top bits on the way to a 565
/// surface, so green's sixth bit is its top one. A top-down or empty DIB is
/// refused, as Microsoft's DrawDibDraw page and Wine's `test_DrawDib_sizeimage`
/// refuse one; a scale that is not whole, or a 24-bit DIB, is a wall; a
/// destination wholly off the surface draws nothing; and handles count up, so
/// a handle closed is one no more.
#[test]
fn drawdib_draws_a_dib_whole_onto_a_surface_s_dc() {
    const DIB: u32 = DATA + 0x400;
    let mut m = machine();
    let surface = surface(&mut m, PIXELS, 4, 4);
    assert_eq!(
        com(&mut m, "IDirectDrawSurface", 17, surface, &[OUT]),
        Ok(0)
    );
    let dc = m.arena.read_u32(OUT).expect("in the arena");
    let dd = call(&mut m, "MSVFW32", "DrawDibOpen", &[]).expect("a handle");
    assert_eq!(dd, 1, "counted from one");
    // A 2x2 DIB: on top 555's (31, 16, 0) and (0, 15, 31), below (16, 31,
    // 16) and (1, 1, 1); its bits are the bottom row first.
    let (a, b, c, d) = (0x7e00u16, 0x01ff, 0x43f0, 0x0421);
    let bits: Vec<u8> = [c, d, a, b].iter().flat_map(|p| p.to_le_bytes()).collect();
    m.arena
        .write(DIB, &[dib_header(2, 2, 16, 8), bits].concat())
        .expect("in the arena");
    let draw = |m: &mut Machine, header: u32, (x, y, w, h): (i32, i32, i32, i32)| {
        call(
            m,
            "MSVFW32",
            "DrawDibDraw",
            &[
                dd, dc, x as u32, y as u32, w as u32, h as u32, header, 0, 0, 0, 2, 2, 0,
            ],
        )
    };
    // The same four in 565, green's sixth bit its top one.
    let (a, b, c, d) = (0xfc20, 0x03df, 0x87f0, 0x0841);

    assert_eq!(draw(&mut m, DIB, (0, 0, 4, 4)), Ok(1));
    assert_eq!(
        four_by_four(&m),
        [a, a, b, b, a, a, b, b, c, c, d, d, c, c, d, d]
    );
    m.arena.write(PIXELS, &[0; 32]).expect("in the arena");
    assert_eq!(draw(&mut m, DIB, (1, 1, 4, 4)), Ok(1));
    assert_eq!(
        four_by_four(&m),
        [0, 0, 0, 0, 0, a, a, b, 0, a, a, b, 0, c, c, d]
    );

    // Off the surface: drawn, and nothing changes.
    assert_eq!(draw(&mut m, DIB, (100, 0, 4, 4)), Ok(1));
    assert_eq!(
        four_by_four(&m),
        [0, 0, 0, 0, 0, a, a, b, 0, a, a, b, 0, c, c, d]
    );
    // Top-down, and of no width: refused, and nothing drawn.
    for (at, value) in [(8, (-2i32) as u32), (4, 0)] {
        m.arena.write_u32(DIB + at, value).expect("in the arena");
        assert_eq!(draw(&mut m, DIB, (0, 0, 4, 4)), Ok(0));
        m.arena.write_u32(DIB + at, 2).expect("in the arena");
    }
    assert_eq!(four_by_four(&m)[5], a);
    let wall = draw(&mut m, DIB, (0, 0, 3, 3));
    assert!(
        matches!(&wall, Err(Stop::NotWritten { what, .. }) if what.contains("onto 3x3")),
        "{wall:?}"
    );
    // A 24-bit DIB, which nothing on the roster draws.
    m.arena
        .write(
            DIB + 0x100,
            &[dib_header(1, 1, 24, 4), vec![0x10, 0x20, 0x30, 0]].concat(),
        )
        .expect("in the arena");
    let wall = call(
        &mut m,
        "MSVFW32",
        "DrawDibDraw",
        &[dd, dc, 3, 0, 1, 1, DIB + 0x100, 0, 0, 0, 1, 1, 0],
    );
    assert!(
        matches!(&wall, Err(Stop::NotWritten { what, .. }) if what.contains("24-bit")),
        "{wall:?}"
    );

    assert_eq!(call(&mut m, "MSVFW32", "DrawDibClose", &[dd]), Ok(1));
    assert_eq!(call(&mut m, "MSVFW32", "DrawDibClose", &[dd]), Ok(0));
    assert_eq!(draw(&mut m, DIB, (0, 0, 4, 4)), Ok(0));
    // The next handle is the next one up, not the one let go of.
    assert_eq!(call(&mut m, "MSVFW32", "DrawDibOpen", &[]), Ok(2));
    assert_eq!(draw(&mut m, DIB, (0, 0, 4, 4)), Ok(0), "1 is let go of");
}
