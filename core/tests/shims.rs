//! What KERNEL32, USER32 and retained mode answer, called as a title calls
//! them but without one: each shim or method run on the stub's machine with
//! the arguments a title would push, and what it wrote and answered read back
//! (#114, #117).

use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::Stop;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::win32::{self, user32::WINDOW, Ctx};
use lego_legacy_core::Title;

/// Where the tests put what they pass and what is written back.
const DATA: u32 = 0x0050_0000;
const OUT: u32 = 0x0050_1000;

fn machine() -> Machine {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    Machine::new(&Title::STUB, &mut Headless::new()).expect("a machine")
}

/// Call `dll!name` with `args`, as the boundary would.
fn call(machine: &mut Machine, dll: &str, name: &str, args: &[u32]) -> Result<u32, Stop> {
    let shim = win32::find(dll, name).unwrap_or_else(|| panic!("{dll}!{name} is written"));
    with_ctx(machine, |ctx| (shim.run)(ctx, args))
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

fn with_ctx(
    machine: &mut Machine,
    run: impl FnOnce(&mut Ctx<'_>) -> Result<u32, Stop>,
) -> Result<u32, Stop> {
    let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
    let mut ctx = Ctx {
        cpu: &mut machine.cpu,
        mem: &mut machine.arena,
        state: &mut machine.state,
        title: &Title::STUB,
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
fn a_window_style_is_the_one_it_was_made_with() {
    const GWL_STYLE: u32 = -16i32 as u32;
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
    // Any other word is not answered.
    assert!(call(&mut m, "USER32", "GetWindowLongA", &[WINDOW, GWL_USERDATA]).is_err());
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
    // SPI_GETSCREENSAVEACTIVE, which no title has asked.
    assert!(call(
        &mut m,
        "USER32",
        "SystemParametersInfoA",
        &[0x0010, 0, OUT, 0]
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
