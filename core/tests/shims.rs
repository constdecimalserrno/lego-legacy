//! What KERNEL32 and USER32 answer, called as a title calls them but without
//! one: each shim run on the stub's machine with the arguments a title would
//! push, and what it wrote and answered read back (#114).

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
    (shim.run)(&mut ctx, args)
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
