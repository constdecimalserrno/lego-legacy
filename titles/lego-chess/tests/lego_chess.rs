//! LEGO Chess, run headless from the player's rip: the launcher finding it
//! or refusing it, its declared tree, and its boot as far as the machine
//! carries it, replayed to the same machine every time.
//!
//! These need the player's own rip under `game-data/`. Without it the tests
//! print a skip line and pass, which is a gate that did not run rather than one
//! that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into a failure.

use lego_chess::LEGO_CHESS as CHESS;
use lego_legacy_core::cli::{Cli, Screenshots, Trace};
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, hash, testkit, Title};

const ROOT: &str = CHESS.install_dir;

/// Enough instructions for the C runtime to start, the game to make its
/// window and its framework to come up, and its logo movie to start: the
/// boot ends by it inside the logo's first frame (#150), waiting on the
/// clock, which a traced run logs a line of for every read.
const BOOT_BUDGET: u64 = 1_000_000;

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&CHESS.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    // It lists the title fifth, after the four released ones.
    assert_eq!(
        roster::ROSTER.iter().position(|e| e.key == CHESS.key),
        Some(4)
    );
    if !scratch() {
        return;
    }
    // Which means it read the game out of MAIN.Z and found its bytes.
    assert_eq!(
        roster::look_for(&roster::LEGO_CHESS),
        Found::Runnable {
            pressing: "masterpiece-chess".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..CHESS.image
        },
        ..CHESS
    };
    let err = testkit::try_run_headless(&ABSENT, &Cli::default(), 1_000)
        .expect_err("there is no such pressing");

    assert!(
        err.contains("no-such-pressing"),
        "the refusal names the path: {err}"
    );
    assert!(
        err.contains("ripped from your own disc"),
        "and says what to do: {err}"
    );
}

#[test]
fn its_machine_reads_long_names_and_lacks_glide_and_the_opengl_plugin() {
    if !scratch() {
        return;
    }
    // Its bumper is named only in the disc's Joliet tree (#144), and the
    // machine reads the disc through it.
    let mut tree = testkit::tree(&CHESS);
    assert!(tree.lies("D:\\intro\\Chess Bumper01.bmp").is_some());
    let mut absent: Vec<&str> = CHESS
        .declared
        .absent_libraries
        .iter()
        .map(|&(name, _)| name)
        .collect();
    absent.sort_unstable();
    assert_eq!(absent, ["KLIB_OpenGL_PlugIn.dll", "glide2x.dll"]);
}

#[test]
fn its_disc_carries_the_label_its_cd_check_compares() {
    if !scratch() {
        return;
    }
    // Byte for byte, at 0x4054E2 (#145).
    let iso = disc::open_pressing(CHESS.image.disc_dir, CHESS.image.pressing)
        .expect("the pressing opens");
    assert_eq!(iso.label, "LEGOCHESS");
}

#[test]
fn the_declared_tree_is_main_z_expanded_with_the_game_s_strings_beside_it() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&CHESS);

    // The game and everything it ships, each expanded when it is read.
    let exe = tree
        .read(&format!("{ROOT}\\Lego Chess.exe"))
        .expect("the game is installed");
    assert_eq!(hash::sha256_hex(&exe), CHESS.image.sha256);
    for name in [
        "RCBOOK.BOK",
        "Advisors\\redking.avi",
        "profiles\\saves\\readme.txt",
    ] {
        assert!(
            tree.readable(&format!("{ROOT}\\{name}")),
            "{name} is installed"
        );
    }
    // Its renderers, as a title looking for them finds them, at their sizes.
    let mut plugins = tree.matching(ROOT, "KLIB_*.dll");
    plugins.sort();
    assert_eq!(
        plugins,
        [
            ("KLIB_DIRECTX_PLUGIN.DLL".to_owned(), 53_248),
            ("KLIB_GLIDE_PLUGIN.DLL".to_owned(), 61_440),
            ("KLIB_SOFTWARE_PLUGIN.DLL".to_owned(), 86_016),
        ]
    );

    // The game's own strings, copied out of the setup's library to the root.
    let text = tree
        .read(&format!("{ROOT}\\Text.txt"))
        .expect("its strings are installed");
    assert_eq!(
        hash::sha256_hex(&text),
        "7c7a5118710a1c0f700c7bd955c76403dfcca5dd05e6ce2708b8f4b492d86d3c"
    );

    // A Minimum install: the scenes' three archives stay on the disc and are
    // read where they lie there.
    for (name, len) in [
        ("PIRATES.RES", 59_352_329),
        ("WESTERN.RES", 69_566_827),
        ("TUTORIAL.RES", 41_677_288),
    ] {
        assert!(
            !tree.holds(&format!("{ROOT}\\{name}")),
            "{name} is not installed"
        );
        assert_eq!(tree.lies(&format!("D:\\{name}")).map(|(_, l)| l), Some(len));
    }
}

/// The run every boot test looks at, done once.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &CHESS, &cli, &[], BOOT_BUDGET)
}

#[test]
fn the_game_loads_out_of_main_z_and_makes_its_window() {
    let Some(ran) = boot() else { return };

    // Every member of MAIN.Z, and Text.txt beside them.
    ran.assert_logged("installed 882 files")
        .assert_logged("image Lego Chess.exe base 0x00400000")
        .assert_logged("imports 197")
        // Its C runtime, linked into it, starts first.
        .assert_logged("call KERNEL32.dll!GetVersion() from 0x00471300")
        // Then its framework registers its class and makes its window.
        .assert_logged("call USER32.dll!CreateWindowExA(0x0, 0x487ba8, 0x0, 0x86c80000");
}

/// Its framework reads where the pointer is confined as it starts, to put it
/// back as it leaves (#147); core's shim tests hold what it is told, the
/// whole screen.
#[test]
fn the_framework_reads_where_the_pointer_is_confined() {
    let Some(ran) = boot() else { return };

    assert!(
        ran.log
            .lines()
            .any(|l| l.starts_with("call USER32.dll!GetClipCursor(")
                && l.contains(" from 0x0045ac3c ")),
        "its framework asks where the pointer is confined\nlog:\n{}",
        ran.log
    );
}

/// Its input and its sound come up (#149). DirectInput's keyboard and mouse
/// are each set to `DISCL_NONEXCLUSIVE|DISCL_FOREGROUND` and acquired, which
/// the machine already answered. Its 3D listener is set all at once, and the
/// CD audio device it opens by number opens, with its data disc in the
/// drive, as Windows' does, and counts in tracks.
#[test]
fn its_input_and_sound_come_up() {
    let Some(ran) = boot() else { return };

    for device in ["keyboard", "mouse"] {
        ran.assert_logged(&format!("dinput {device} at "));
    }
    // Its framework sets each to the foreground, not exclusively, and acquires
    // it as it starts, once each; its window procedure acquires them again
    // whenever it is told it is in front (0x0045C646), from other places.
    let called = |call: &str, from: &str| {
        let times = ran
            .log
            .lines()
            .filter(|l| l.starts_with(call) && l.contains(&format!(" from {from} ")))
            .count();
        assert_eq!(times, 1, "{call} from {from}\nlog:\n{}", ran.log);
    };
    let foreground = "call IDirectInputDeviceA::SetCooperativeLevel(0x10010, 0x6)";
    let acquire = "call IDirectInputDeviceA::Acquire()";
    for (call, from) in [
        (foreground, "0x00455e6c"),
        (acquire, "0x00455e9e"),
        (foreground, "0x00455f6e"),
        (acquire, "0x00455fad"),
    ] {
        called(call, from);
    }
    ran.assert_logged("call IDirectSound3DListener::SetAllParameters(")
        .assert_logged("mci command 0x0803 to device 516, flags 0x3102, answered 0")
        .assert_logged("mci command 0x080d to device 1, flags 0x402, answered 0");
}

/// Its plugin walk goes through all four (#148): the DirectX plugin gets
/// Direct3D 2 off DirectDraw's `IDirectDraw2` face, as DirectX 5 hands it
/// out, and lists its devices from it; the Glide plugin finds no
/// `glide2x.dll`; the Software plugin lists its own driver; and the OpenGL
/// plugin is not there.
#[test]
fn the_plugin_walk_gets_direct3d_2() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("com IDirectDraw4 handed back IDirect3D2")
        .assert_logged("call IDirect3D2::EnumDevices(")
        .assert_logged("library missing glide2x.dll")
        .assert_logged("proc address KLIB_Software_HookDll")
        .assert_logged("library missing KLIB_OpenGL_PlugIn.dll");
}

/// The game's main asks the system's language (#147), finds its disc on D:
/// with no word to the player (#145's check, 0x004087AF), is told no screen
/// saver is enabled, and reads its three archives from the disc it found.
/// Its video setup's own auto-select then picks the DirectX plugin's
/// Direct3D 2 device on this machine with a 3D card, takes the screen and
/// makes the device.
#[test]
fn the_game_s_main_finds_its_disc_and_picks_direct3d_2() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("call KERNEL32.dll!GetSystemDefaultLangID() from 0x004086e6")
        .assert_logged("call USER32.dll!SystemParametersInfoA(0x10, 0x0, ")
        .assert_logged("file opened D:\\pirates.res")
        .assert_logged("file opened D:\\tutorial.res")
        .assert_logged("file opened D:\\western.res")
        .assert_logged("call IDirectDraw::SetDisplayMode(0x280, 0x1e0, 0x10)")
        .assert_logged("call IDirect3D2::CreateDevice(");
    assert!(
        !ran.log.contains("MessageBoxA"),
        "the disc is found with no word to the player\nlog:\n{}",
        ran.log
    );
}

/// Its strings load where it was installed (#171). Its main restores its
/// saved directory before it has saved it, at 0x0045691B, an empty name,
/// which Windows refuses and the directory stays, so `text.txt` is found
/// beside the game; with the directory emptied, it was looked for at the
/// root and missed, as it was in every run before.
#[test]
fn the_game_s_strings_load_from_where_it_was_installed() {
    let Some(ran) = boot() else { return };

    ran.assert_logged(&format!("file opened {ROOT}\\text.txt"));
    assert!(
        !ran.log.contains("file missing \\text.txt"),
        "log:\n{}",
        ran.log
    );
}

/// Its DirectX plugin lists the device's texture formats and makes it a
/// viewport (#148), and its framework then takes the screen: it confines the
/// pointer to it and says a screen saver is running, as programs of the time
/// did to keep Alt+Tab away (#147).
#[test]
fn the_device_gets_its_formats_and_viewport_and_the_framework_takes_the_screen() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("d3d2 texture formats: 6 of them")
        .assert_logged("call IDirect3D2::CreateViewport(")
        .assert_logged("call USER32.dll!ClipCursor(")
        .assert_logged("call USER32.dll!SystemParametersInfoA(0x61, 0x1, ");
}

/// Its framework tells its own window its size, and invalidates every window
/// once the mode is set (#147). The game's first movie clears the screen
/// twice, its first two flips, and its pump takes what is waiting: the
/// window is the active application's, is active, has the keyboard (#196)
/// and is painted.
#[test]
fn the_window_is_told_its_size_painted_and_focused() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("call USER32.dll!SendMessageA(0x10010, 0x5, 0x0, 0x1e00280)")
        .assert_logged(
            "call USER32.dll!DefWindowProcA(0x10010, 0x5, 0x0, 0x1e00280) from 0x0045c88e",
        )
        .assert_logged("call USER32.dll!InvalidateRect(0x0, 0x0, 0x1)")
        .assert_logged("call USER32.dll!BeginPaint(0x10010, ")
        .assert_logged("flip 1 cursor ");
    let told: Vec<&str> = ran
        .log
        .lines()
        .filter(|l| l.starts_with("window procedure given message "))
        .collect();
    for message in ["0x001c", "0x0006", "0x0007", "0x000f"] {
        assert!(
            told.iter().any(|l| l.ends_with(message)),
            "its window is given {message}\nlog:\n{}",
            ran.log
        );
    }
}

/// Its first movie, the logo, opens through AVIFile (#151): its file and
/// its two streams, their headers, and its sound's format, which the game
/// makes its DirectSound buffer in. Then it takes the first frame, as a
/// 16-bit DIB, and hands it to its DirectX plugin, which draws it doubled
/// onto its back buffer's DC with DrawDib and lets the DC go.
#[test]
fn its_first_movie_opens_and_its_streams_are_read() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("movie D:\\intro\\lmi.avi opened through AVIFile: 2 streams")
        .assert_logged("call AVIFIL32.dll!AVIFileInfoA(")
        .assert_logged("call AVIFIL32.dll!AVIFileGetStream(")
        .assert_logged("call AVIFIL32.dll!AVIStreamReadFormat(")
        .assert_logged("movie D:\\intro\\lmi.avi frames as 16-bit DIBs at ");
    // The whole screen, from the whole 320x240 frame.
    assert!(
        ran.log
            .lines()
            .any(|l| l.starts_with("call MSVFW32.dll!DrawDibDraw(")
                && l.contains(", 0x0, 0x0, 0x280, 0x1e0, ")
                && l.contains(", 0x0, 0x0, 0x0, 0x140, 0xf0, 0x0) from 0x400040ca ")),
        "DrawDibDraw draws the whole frame doubled onto the whole screen\nlog:\n{}",
        ran.log
    );
    for (call, from) in [
        ("call AVIFIL32.dll!AVIStreamAddRef(", "0x0044b094"),
        ("call AVIFIL32.dll!AVIStreamAddRef(", "0x0044b0db"),
        ("call IDirectSound::CreateSoundBuffer(", "0x00450a4a"),
        ("call AVIFIL32.dll!AVIStreamGetFrameOpen(", "0x0044b325"),
        ("call AVIFIL32.dll!AVIStreamGetFrame(", "0x0044b345"),
        ("call IDirectDrawSurface::GetDC(", "0x40004081"),
        ("call MSVFW32.dll!DrawDibOpen(", "0x4000408d"),
        ("call MSVFW32.dll!DrawDibClose(", "0x400040d0"),
        ("call IDirectDrawSurface::ReleaseDC(", "0x400040e0"),
    ] {
        assert!(
            ran.log
                .lines()
                .any(|l| l.starts_with(call) && l.contains(&format!(" from {from} "))),
            "{call} from {from}\nlog:\n{}",
            ran.log
        );
    }
}

/// Its logo's sound starts with its first frame (#150): its start and length
/// are read, eleven chunks of it, 1,470 samples each, go into the
/// DirectSound buffer it made, and the buffer plays, looping, at full volume.
#[test]
fn its_logo_s_sound_starts_with_its_first_frame() {
    let Some(ran) = boot() else { return };

    ran.assert_logged("call AVIFIL32.dll!AVIStreamStart(")
        .assert_logged("call AVIFIL32.dll!AVIStreamLength(")
        .assert_logged("call IDirectSoundBuffer::Play(0x0, 0x0, 0x1) on ")
        .assert_logged("call IDirectSoundBuffer::SetVolume(0x0) on ");
    let reads = ran
        .log
        .lines()
        .filter(|l| {
            l.starts_with("call AVIFIL32.dll!AVIStreamRead(")
                && l.contains(", 0x5be, ")
                && l.contains(" from 0x0044bb10 ")
        })
        .count();
    assert_eq!(reads, 11, "log:\n{}", ran.log);
}

/// Where the boot stops: by its budget, inside the logo's first frame, its
/// third flip presented and the game waiting on the clock for the next. What
/// it meets after that is its routes'.
#[test]
fn the_boot_runs_to_its_budget_in_the_logo_s_first_frame() {
    let Some(ran) = boot() else { return };

    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "log:\n{}",
        ran.log
    );
    ran.assert_flips(3)
        .assert_logged("flip 2 cursor 793477 640x480x16");
}

#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else { return };
    let Some(second) = ({
        static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
        let cli = Cli {
            trace: Trace::Shims,
            ..Cli::default()
        };
        testkit::replay(&AGAIN, scratch, &CHESS, &cli, &[], BOOT_BUDGET)
    }) else {
        return;
    };
    first.assert_same_machine(second);
}

/// First light (#150): the flip the game's own picture first reaches the
/// screen at, the first sliver of a brick at the top of the LEGO Media logo
/// movie. First is decided as every title's is: the first frame that is not
/// one colour. Flips 0 and 1 are the movie player's two clears; the DirectX
/// plugin fills its first four flips black whatever was drawn, so flips 2 and
/// 3, the logo's frames 0 and 1, are black; and the logo's own frames 2 to 4,
/// flips 4 to 6, are black in the movie. Flip 7 is its frame 5.
const FIRST_LIGHT: u64 = 7;

/// The first flip a person would notice something on: one grey brick at the
/// top left, the logo's frame 8, kept for a person to look at.
const FIRST_BRICK: u64 = 10;

/// Enough instructions for the logo to reach the brick, flip 10 at
/// 72,784,709, and a movie frame more.
const FIRST_LIGHT_BUDGET: u64 = 84_000_000;

/// The committed route to first light, replayed, keeping the frame before
/// first light, first light and the brick, and, when `shots`, writing the
/// last two as pictures in its run directory. Two slots, so the determinism
/// gate's two runs can go side by side; its second writes none, since nothing
/// reads them.
fn first_light(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    shots: bool,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/first-light.route"),
        ),
        screenshots: Screenshots {
            at: if shots {
                vec![FIRST_LIGHT, FIRST_BRICK]
            } else {
                Vec::new()
            },
            every: None,
        },
        ..Cli::default()
    };
    testkit::replay(
        slot,
        scratch,
        &CHESS,
        &cli,
        &[FIRST_LIGHT - 1, FIRST_LIGHT, FIRST_BRICK],
        FIRST_LIGHT_BUDGET,
    )
}

static LIT: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// First light, as the conversions #151 wrote make it: the logo's frame 5
/// decoded, taken as a 555 DIB, drawn doubled onto the 565 back buffer with
/// green's sixth bit its top one, and presented. Its 52 lit pixels are the
/// brick's first sliver, greys of no more than #212121, black to the eye.
///
/// The pinned screen is a reference decoder's picture taken the same way:
/// FFmpeg 9.0.1, run once by hand on a copy of the disc's `INTRO\LMI.AVI`,
///
///   ffmpeg -i LMI.AVI -map 0:v:0 -f rawvideo -pix_fmt yuv410p
///     -fps_mode passthrough -
///
/// writes the pictures it shows (frames 1 and 3 are null), so frame 5 is the
/// fourth it writes. Each pixel goes through BT.601 at studio range in
/// Microsoft's integer form, its chroma from its 4x4 block; is narrowed to
/// 555 by dropping each channel's low bits; is widened to 565 with green's
/// sixth bit its top one; and is doubled to 640x480. That 565 screen is the
/// sha256 below, `28b52ec6`, which is also the old tree's default-boot first
/// light; widened to the BGRA the host is handed, each channel's top bits
/// repeated, it is `6f73eca7`.
#[test]
fn the_route_reaches_first_light_as_the_logo_s_first_brick_comes_in() {
    let Some(ran) = first_light(&LIT, true) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way to first light stops the run\nlog:\n{}",
        ran.log
    );
    ran.assert_logged("flip 7 cursor 45117061 640x480x16");

    let lit = ran
        .host
        .frames
        .iter()
        .find(|f| !f.is_uniform())
        .expect("the game puts a picture on the screen");
    assert_eq!(
        lit.flip, FIRST_LIGHT,
        "first light moved: it is now flip {}",
        lit.flip
    );
    assert_eq!((lit.width, lit.height), (640, 480));
    // Before it, the player's two clears, the plugin's two flips it fills
    // black, and the logo's own black frames: every one of them black.
    for frame in &ran.host.frames[..FIRST_LIGHT as usize] {
        assert!(frame.is_uniform(), "flip {} is one colour", frame.flip);
        assert_eq!(frame.pixels[..3], [0, 0, 0], "flip {} is black", frame.flip);
    }
    let shown: Vec<(usize, usize)> = lit
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, p)| p[..3] != [0, 0, 0])
        .map(|(i, p)| {
            assert!(
                p[..3].iter().all(|c| *c <= 0x21),
                "{p:?} is a grey of the sliver"
            );
            (i % 640, i / 640)
        })
        .collect();
    assert_eq!(shown.len(), 52);
    assert!(
        shown
            .iter()
            .all(|&(x, y)| (102..=117).contains(&x) && y <= 5),
        "the sliver is at the top: {shown:?}"
    );
    assert_eq!(
        hash::sha256_hex(&lit.pixels),
        "6f73eca7d5b907a0900f3c6f45765ecb7fc6c9edd29723dfa6a2fccada780b52"
    );
    // The 565 screen it was widened from: each channel's top bits.
    let screen: Vec<u8> = lit
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[b, g, r, _]| {
            (u16::from(r >> 3) << 11 | u16::from(g >> 2) << 5 | u16::from(b >> 3)).to_le_bytes()
        })
        .collect();
    assert_eq!(
        hash::sha256_hex(&screen),
        "28b52ec6d2bc2914b91305f2d379dfbf38762dd8aeb2618f24241a8553dc5069"
    );
    // The brick a person first notices, and the pictures of both for a
    // person to look at.
    assert!(!ran
        .host
        .frame(FIRST_BRICK)
        .expect("the brick is kept")
        .is_uniform());
    for flip in [FIRST_LIGHT, FIRST_BRICK] {
        let shot = ran.dir.join(format!("shots/{flip:010}.png"));
        assert!(shot.is_file(), "{} is written", shot.display());
    }
}

#[test]
fn the_first_light_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    // The second replay runs beside the first rather than after it.
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| first_light(&AGAIN, false));
        (
            first_light(&LIT, true),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in [FIRST_LIGHT, FIRST_BRICK] {
        assert!(
            first.host.frame(flip).map(|f| &f.pixels) == second.host.frame(flip).map(|f| &f.pixels),
            "including what it presented at flip {flip}"
        );
    }
}

/// The flips of the intro a person checks by eye (#151): the logo near its
/// end, its frame 270; Krisalis' logo's last frame, its 37th; and the
/// intro's frame 10, letterboxed at y 40.
const LOGO_NEAR_ITS_END: u64 = 272;
const KRISALIS_LAST: u64 = 323;
const INTRO_TENTH: u64 = 338;

/// Enough instructions for the logo and Krisalis' logo to play through and
/// the intro to reach its frame 12, flip 340 at 3,695,193,660, and no more:
/// the route's cap, as its own comment gives it.
const INTRO_BUDGET: u64 = 3_700_000_000;

/// The committed route through the intro's first two movies and into the
/// third, replayed, keeping the flips a person checks and, when `shots`,
/// writing each as a picture in its run directory. Two slots, so the
/// determinism gate's two runs can go side by side; its second writes none,
/// since nothing reads them.
fn intro(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    shots: bool,
) -> Option<&'static testkit::Ran> {
    let kept = [LOGO_NEAR_ITS_END, KRISALIS_LAST, INTRO_TENTH];
    let cli = Cli {
        route: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/intro.route")),
        screenshots: Screenshots {
            at: if shots { kept.to_vec() } else { Vec::new() },
            every: None,
        },
        ..Cli::default()
    };
    testkit::replay(slot, scratch, &CHESS, &cli, &kept, INTRO_BUDGET)
}

static INTRO: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// The intro's movies play as the game plays them (#151): each opens, plays a
/// frame a flip with its sound, and is let go of, and the next opens. The
/// logo's flips are 0 to 283, its 280 frames and the player's two clears
/// either side; Krisalis' logo's are 284 to 325; the intro's start at 326,
/// its frames at 328.
/// The logo is heard, Krisalis' logo has no sound, and the intro is heard.
#[test]
fn the_intro_plays_the_logo_and_krisalis_through_and_starts_the_intro() {
    let Some(ran) = intro(&INTRO, true) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing in the intro stops the run\nlog:\n{}",
        ran.log
    );
    ran.assert_logged("flip 340 cursor 3695193670 640x480x16");
    // Each opens after the player's two clears before it.
    assert_eq!(ran.flip_before("movie D:\\intro\\krisalis.avi opened"), 285);
    assert_eq!(ran.flip_before("movie D:\\intro\\intro.avi opened"), 327);
    for flip in [LOGO_NEAR_ITS_END, KRISALIS_LAST, INTRO_TENTH] {
        let frame = ran.host.frame(flip).expect("the flip is kept");
        assert!(!frame.is_uniform(), "flip {flip} is a picture");
        let shot = ran.dir.join(format!("shots/{flip:010}.png"));
        assert!(shot.is_file(), "{} is written", shot.display());
    }
    // The intro is 640x400 at y 40, black above and below.
    let tenth = ran.host.frame(INTRO_TENTH).expect("kept");
    let row = |y: usize| &tenth.pixels[y * 640 * 4..(y + 1) * 640 * 4];
    assert!((0..40)
        .chain(440..480)
        .all(|y| row(y).chunks(4).all(|p| p[..3] == [0, 0, 0])));
    assert!(row(240).chunks(4).any(|p| p[..3] != [0, 0, 0]));
    // The mix, block by block.
    assert!(
        ran.loudness(2, 282).iter().any(|r| *r > 0.0),
        "the logo is heard"
    );
    let krisalis = ran.loudness(286, 324);
    assert!(
        !krisalis.is_empty() && krisalis.iter().all(|r| *r == 0.0),
        "Krisalis' logo is mixed, and has no sound"
    );
    assert!(
        ran.loudness(328, 341).iter().any(|r| *r > 0.0),
        "the intro is heard"
    );
}

/// The intro's clock spins are passed (ADR-0004, #217). Between frames
/// the movie player reads `QueryPerformanceCounter` from 0x0044B57D, 38
/// instructions a time and about 4,400 times a declared millisecond, and the
/// machine retires the passes that would read the same millisecond at once.
/// It lands exactly where running every pass lands: the same machine, every
/// flip at the same cursor, and the same mix handed over at the same flips,
/// as main made them running every pass (5d8104c, and e9bed42 after #215),
/// but for #171's 23,170 instructions at boot, which move the digest and every
/// flip's cursor and leave the mix where it was, #338's DirectDraw surface
/// reclamation, which pools freed surface buffers (moving only the memory
/// digest while keeping every flip cursor, audio mix, and handoff flip
/// bit-exact), #373's truthful D3D triangle alpha/transparency caps, which
/// add +10 instructions inside `KLIB_DirectX_PlugIn.dll`'s boot `EnumDevices`
/// callback while keeping every frame, audio mix, and handoff flip bit-exact,
/// and #385's DirectSound buffer reclamation, which pools freed sample stores
/// (moving only the memory digest while keeping every flip cursor, audio mix,
/// and handoff flip bit-exact).
#[test]
fn the_intro_s_clock_spins_are_passed_and_land_where_running_every_pass_lands() {
    let Some(ran) = intro(&INTRO, true) else {
        return;
    };
    let flips: String = ran
        .log
        .lines()
        .filter(|l| l.starts_with("flip "))
        .map(|l| format!("{l}\n"))
        .collect();
    let mix: Vec<u8> = ran
        .host
        .audio_blocks
        .iter()
        .flatten()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    // And the flip each block of the mix was handed over at.
    let handed: Vec<u8> = ran
        .host
        .audio_flips
        .iter()
        .flat_map(|f| f.to_le_bytes())
        .collect();
    assert_eq!(
        (
            ran.digest.as_str(),
            hash::sha256_hex(flips.as_bytes()).as_str(),
            hash::sha256_hex(&mix).as_str(),
            hash::sha256_hex(&handed).as_str(),
        ),
        (
            "5a051881788abacfe71477bff72d61d6618d984b056ef9c50bbcd2d5274d846e",
            "8750f27e099b58c2ca4afb927ab3faea5025e67682a7532151fe6c16cb911df9",
            "70e84ca87ff3133b07ccc0debb9dd4e8b4a06f368854e780d6ae05a8e8e54ef7",
            "77712d0720f53b658e72e2d3f300da30a2e50d3298feacb51c536e2fba2a7a42",
        ),
        "the machine, its flips, its mix and where each block of it was handed over, as \
         running every pass made them"
    );
    ran.assert_logged("clock spins: ");
}

#[test]
fn the_intro_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| intro(&AGAIN, false));
        (
            intro(&INTRO, true),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in [LOGO_NEAR_ITS_END, KRISALIS_LAST, INTRO_TENTH] {
        assert!(
            first.host.frame(flip).map(|f| &f.pixels) == second.host.frame(flip).map(|f| &f.pixels),
            "including what it presented at flip {flip}"
        );
    }
}

/// Krisalis' logo's fifth frame, where the skip route presses Space, and the
/// intro's tenth frame after the skip, where it ends (#215).
const KRISALIS_FIFTH: u64 = 290;
const SKIPPED_TO_THE_TENTH: u64 = 305;

/// Enough instructions for the logo to play through, Krisalis' logo to be
/// skipped, and the intro to reach its frame 10, flip 305 at 2,971,352,027,
/// and no more.
const SKIP_BUDGET: u64 = 2_975_000_000;

/// The committed route that skips Krisalis' logo with Space, replayed, keeping
/// the intro's tenth frame and, when `shots`, writing it as a picture in its
/// run directory. Two slots, so the determinism gate's two runs can go side by
/// side; its second writes none, since nothing reads them.
fn skipped(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    shots: bool,
) -> Option<&'static testkit::Ran> {
    let kept = [SKIPPED_TO_THE_TENTH];
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/skip-krisalis.route"),
        ),
        screenshots: Screenshots {
            at: if shots { kept.to_vec() } else { Vec::new() },
            every: None,
        },
        ..Cli::default()
    };
    testkit::replay(slot, scratch, &CHESS, &cli, &kept, SKIP_BUDGET)
}

static SKIPPED: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// Space skips a movie the game lets be skipped, and let go between flips it
/// gets the game past the wait after the movie (#215). Pressed at Krisalis'
/// logo's fifth frame, it ends the movie and its two-second hold; let go at
/// the 300th poll after that flip, inside the wait at 0x0040D220, which
/// presents nothing, it lets the game go on, and the intro opens after flip
/// 294, where with Krisalis' logo played through it opened after flip 327.
#[test]
fn space_skips_krisalis_logo_and_let_go_between_flips_gets_past_the_wait() {
    let Some(ran) = skipped(&SKIPPED, true) else {
        return;
    };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing after the skip stops the run\nlog:\n{}",
        ran.log
    );
    ran.assert_logged(&format!(
        "input flip {KRISALIS_FIFTH} Key {{ code: 57, down: true }}"
    ))
    .assert_logged(&format!(
        "input flip {KRISALIS_FIFTH} poll 300 Key {{ code: 57, down: false }}"
    ))
    .assert_logged("flip 305 cursor 2971352037 640x480x16");
    // The two clears follow at once: the hold was over as Space was seen.
    ran.assert_logged("flip 291 cursor 2706399641 640x480x16");
    assert_eq!(ran.flip_before("movie D:\\intro\\intro.avi opened"), 294);
    let tenth = ran.host.frame(SKIPPED_TO_THE_TENTH).expect("kept");
    assert!(!tenth.is_uniform(), "the intro plays");
    let shot = ran
        .dir
        .join(format!("shots/{SKIPPED_TO_THE_TENTH:010}.png"));
    assert!(shot.is_file(), "{} is written", shot.display());
}

#[test]
fn the_skip_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| skipped(&AGAIN, false));
        (
            skipped(&SKIPPED, true),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    assert!(
        first.host.frame(SKIPPED_TO_THE_TENTH).map(|f| &f.pixels)
            == second.host.frame(SKIPPED_TO_THE_TENTH).map(|f| &f.pixels),
        "including what it presented at flip {SKIPPED_TO_THE_TENTH}"
    );
}

/// The flips the throne-room route keeps (#210, #157): the bumper's first; the
/// New Player form's buttons and boxes on black, its first whole frame, its
/// first with the name box's caret on, and the name typed; "Who's playing?"
/// with the new player in it; the throne room's first, as the camera starts to
/// fly in, one with the King there, and the one before the click on the chess
/// board, which the game names under the pointer ("Quick game"); and the Quick
/// Game screen's first.
const BUMPER: u64 = 303;
const FORM_ON_BLACK: u64 = 306;
const NEW_PLAYER: u64 = 307;
const CARET: u64 = 319;
const NAMED: u64 = 335;
const WHO_IS_PLAYING: u64 = 360;
const THRONE_ROOM: u64 = 392;
const THE_KING: u64 = 400;
const BOARD_NAMED: u64 = 429;
const QUICK_GAME: u64 = 432;
const THRONE_ROOM_KEPT: [u64; 10] = [
    BUMPER,
    FORM_ON_BLACK,
    NEW_PLAYER,
    CARET,
    NAMED,
    WHO_IS_PLAYING,
    THRONE_ROOM,
    THE_KING,
    BOARD_NAMED,
    QUICK_GAME,
];

/// Enough instructions for the route to close the window at flip 470 and the
/// game to leave, at 3,246,057,181 with the bank (3,246,017,664 without), and
/// room to spare: a run that reaches the end of it has not left.
const THRONE_ROOM_BUDGET: u64 = 3_300_000_000;

/// The committed route into the throne room and onto the Quick Game screen,
/// replayed on `title`, keeping its flips and, when `shots`, writing the
/// bumper, the named form, the King and the Quick Game screen as pictures in
/// its run directory. Two slots, so the determinism gate's two runs can go
/// side by side.
fn throne_room(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    title: &'static Title,
    shots: bool,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/throne-room.route"),
        ),
        screenshots: Screenshots {
            at: if shots {
                vec![BUMPER, NAMED, THE_KING, QUICK_GAME]
            } else {
                Vec::new()
            },
            every: None,
        },
        ..Cli::default()
    };
    testkit::replay(
        slot,
        // With its music's bank too, on any of the four pressings that carry
        // it; the run takes the first present (#156).
        || scratch() && (title.key != CHESS.key || testkit::any_rip(&roster::GM16).is_some()),
        title,
        &cli,
        &THRONE_ROOM_KEPT,
        THRONE_ROOM_BUDGET,
    )
}

static THRONE: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// LEGO Chess as a title the roster does not list, so with no bank to look
/// for: the machine of a player whose discs hold none, with no MIDI out
/// (#156).
const WITHOUT_A_BANK: Title = Title {
    key: "lego-chess-with-no-bank",
    ..CHESS
};

/// Where a run saved a file, as its log says it went: `writes` times to one
/// path, so what is there at the end of the run is what it wrote.
fn saved_times(ran: &testkit::Ran, name: &str, writes: usize) -> std::path::PathBuf {
    let paths: Vec<&str> = ran
        .log
        .lines()
        .filter(|l| l.starts_with("file written") && l.contains(name))
        .map(|l| l.rsplit_once(", to ").expect("where it went").1)
        .collect();
    assert_eq!(paths.len(), writes, "{name} is saved {writes} time(s)");
    assert!(
        paths.iter().all(|p| *p == paths[0]),
        "{name} goes to one path"
    );
    paths[0].into()
}

fn saved(ran: &testkit::Ran, name: &str) -> std::path::PathBuf {
    saved_times(ran, name, 1)
}

/// Past the intro, the bumper, the New Player form and the throne room draw
/// through Direct3D 2, and the King is heard (#210); a name is typed, and the
/// chess board opens the Quick Game screen (#157). Space skips Krisalis' logo
/// and the intro; the bumper's two textures draw by their handles on white; the
/// rooms' files load from the folders the game names (#171) and their sounds
/// decode (#153); with no profile saved the game asks for a new player, every
/// letter of its form drawn at the one depth it is given. Ada is typed into its
/// name box a key at a time, through the `WM_CHAR` each key types, Shift for her
/// capital, and the tick makes her and saves her under that name. The throne
/// room, drawn with `D3DTBLEND_MODULATEALPHA` too, presents as the King starts
/// speaking, a sound decoded through ACM. The board, clicked once the camera
/// has turned after the pointer, opens the Quick Game screen, where #158
/// starts a game. The window is closed at flip 470 and the game leaves. The
/// pinned pictures are this build's own. Of them only the form's, 307 and 319,
/// were compared by eye with a picture, the old tree's one kept New Player
/// frame, as the changelog says; the bumper, 303, only with the old tree's
/// hash of it, which differs; and the rest with nothing, the old tree never
/// having got there.
///
/// With the player's bank the machine has the GS Wavetable synth (#156): the
/// game's MIDI init opens its stream while the bumper stands, and the throne
/// room's tune plays on it from the room's first flip, its callback on
/// WINMM's player thread (#154), under the King's line. Nothing else the run
/// pins moves but where the instructions the player and the callback retire
/// put the flips; the pictures are the ones the run without it presents.
#[test]
fn the_route_makes_a_named_player_and_opens_the_quick_game_screen() {
    let Some(ran) = throne_room(&THRONE, &CHESS, true) else {
        return;
    };
    ran.assert_exited(0);
    ran.assert_logged("midi bank ")
        .assert_logged("GM16.DLS from ");
    assert_eq!(ran.log.matches("midi stream open answered").count(), 1);
    assert_eq!(ran.flip_before("midi stream open answered 0"), 304);
    // #338 + #373: DirectDraw surface pooling (+2) and truthful D3D triangle
    // transparency caps (+10 inside `KLIB_DirectX_PlugIn.dll`'s `EnumDevices`)
    // shift cursors while keeping every presented frame and audio mix bit-exact.
    ran.assert_logged("flip 392 cursor 3116458147 640x480x16")
        .assert_logged("flip 432 cursor 3201821266 640x480x16")
        .assert_logged("end exited 0 cursor 3246057193 flips 471 declared-ms 24867")
        .assert_flips(471);
    the_room_is_made_and_presented(ran);
    // In the throne room its tune starts on the room's first flip, plays under
    // the King and keeps playing onto the Quick Game screen, the mix from
    // there pinned block by block.
    let (onset, mix) = throne_room_mix(ran);
    assert_eq!(onset, Some(THRONE_ROOM), "the tune from its first bar");
    let after = ran.loudness(QUICK_GAME + 2, 471);
    assert!(
        !after.is_empty() && after.iter().all(|r| *r > 0.0),
        "the tune keeps playing on the Quick Game screen"
    );
    assert_eq!(
        mix, "7372b7b8559cae28aaf6e8548e2fddca3849af3936dd9af1b23f041b51e47341",
        "the tune, and the King's line over it"
    );
    for flip in [BUMPER, NAMED, THE_KING, QUICK_GAME] {
        let shot = ran.dir.join(format!("shots/{flip:010}.png"));
        assert!(shot.is_file(), "{} is written", shot.display());
    }
}

/// Without the player's bank the machine has no MIDI out (#156): both of the
/// game's MIDI opens are refused, the game's init while the bumper stands and
/// the throne room's tune on the tick, and the run is the one before the
/// stream was written, to its end, and silent but for the King's line.
#[test]
fn without_the_bank_the_throne_room_is_silent_and_unchanged() {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let Some(ran) = throne_room(&RAN, &WITHOUT_A_BANK, false) else {
        return;
    };
    ran.assert_exited(0);
    assert!(!ran.log.contains("midi bank"), "no bank to look for");
    assert_eq!(ran.log.matches("midi stream open answered 2").count(), 2);
    assert_eq!(ran.flip_before("midi stream open answered 2"), 304);
    ran.assert_logged("flip 392 cursor 3116426928 640x480x16")
        .assert_logged("flip 432 cursor 3201786137 640x480x16")
        .assert_logged("end exited 0 cursor 3246017674 flips 471 declared-ms 24867")
        .assert_flips(471);
    the_room_is_made_and_presented(ran);
    // In the throne room nothing is heard until the King speaks, from flip
    // 394, and nothing after his line, which has died away by the Quick Game
    // screen's third flip; the mix from the throne room on, his line, is
    // pinned block by block.
    let after = ran.loudness(QUICK_GAME + 2, 471);
    assert!(
        !after.is_empty() && after.iter().all(|r| *r == 0.0),
        "the Quick Game screen is silent"
    );
    let (onset, voice) = throne_room_mix(ran);
    assert_eq!(onset, Some(394), "the King starts speaking");
    assert_eq!(
        voice, "cc927b8ad2ab5b58c547db9109172fb3bd940e98e82a71041fa62e8e9da6928c",
        "the King's line"
    );
}

/// What the throne-room route pins whatever its machine's MIDI out: the
/// named player made and saved, the rooms' sounds decoded, silence from the
/// bumper until the throne room, and the pictures.
fn the_room_is_made_and_presented(ran: &testkit::Ran) {
    ran.assert_logged("profiles\\prof1.lcp, asked from 0x0045656a")
        .assert_logged(
            "file written C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\prof1.lcp, 4544 bytes",
        )
        .assert_logged(
            "file written C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\1save.lcs, 190920 bytes",
        );
    // The profile the tick saved starts with the name typed, ended by a zero.
    let profile = std::fs::read(saved(ran, "prof1.lcp")).expect("it is there");
    assert!(
        profile.starts_with(b"Ada\0"),
        "the profile is Ada's: {:x?}",
        &profile[..profile.len().min(16)]
    );
    // The rooms' sounds, all of them MS ADPCM: fifteen while the bumper
    // stands, and twelve more as the tick takes the player in. The board's
    // click and the Quick Game screen decode none and open no stream.
    assert_eq!(ran.log.matches("acm stream ").count(), 27);
    assert_eq!(ran.flip_before("acm stream 15:"), 304);
    assert_eq!(ran.flip_before("acm stream 16:"), 391);
    assert_eq!(ran.flip_before("acm stream 27:"), 391);
    let at = |flip: u64| {
        let frame = ran.host.frame(flip).expect("presented");
        assert!(!frame.pixels.is_empty(), "flip {flip} is kept");
        frame
    };
    // The name box is the old tree's grey, 0x424142, at a vertex alpha of
    // 0x40 with alpha blending on, which over black is 0x101010 on the 565
    // screen.
    let name_box = (228 * 640 + 315) * 4;
    assert_eq!(
        at(FORM_ON_BLACK).pixels[name_box..name_box + 3],
        [0x10, 0x10, 0x10]
    );
    for (flip, pinned) in [
        (
            BUMPER,
            "52f3a8d879a16ee445d1681881960d4ad103a34718c5ce9f3ae58aa5cdb7e33c",
        ),
        (
            NEW_PLAYER,
            "5b3ad2079334df2ffc199be9258fc444d2810ef823c75136a85ad7b9b62f886c",
        ),
        (
            CARET,
            "6f7337264d3cc3382e8af771c53d01a8fb980159d541b76e248edb24f263d019",
        ),
        (
            NAMED,
            "421e501c247ad1d59f2262e3db30d850914c9e9a671686eab254a4c5884a5821",
        ),
        (
            WHO_IS_PLAYING,
            "a004e6381b53e9b76d8270069b220387b3884cb8c77aade563a2beef8a173fe9",
        ),
        (
            THE_KING,
            "1b7c3ce91f27a20b6a7564093c3be1e49af50760dea99d980252fa19f1ec41e7",
        ),
        (
            BOARD_NAMED,
            "fd644e4234353d410de9b3725a0e7f98e9f76caa77035de40cd9450971c2ae17",
        ),
        (
            QUICK_GAME,
            "590d5c72c8adfe0d247d2bc3902d126579ece1343bb0b46a7f437365b5510efc",
        ),
    ] {
        assert_eq!(hash::sha256_hex(&at(flip).pixels), pinned, "flip {flip}");
    }
    assert!(!at(THRONE_ROOM).is_uniform(), "the throne room presents");
    let silence = ran.loudness(BUMPER + 1, THRONE_ROOM);
    assert!(!silence.is_empty() && silence.iter().all(|r| *r == 0.0));
}

/// The first flip from `from` on whose mix is not silent, and the hash of the
/// mix from there on.
fn mix_from(ran: &testkit::Ran, from: u64) -> (Option<u64>, String) {
    let part: Vec<(&Vec<i16>, u64)> = ran
        .host
        .audio_blocks
        .iter()
        .zip(ran.host.audio_flips.iter().copied())
        .filter(|(_, flip)| *flip >= from)
        .collect();
    let onset = part
        .iter()
        .find(|(block, _)| block.iter().any(|s| *s != 0))
        .map(|(_, flip)| *flip);
    let mix: Vec<u8> = part
        .iter()
        .flat_map(|(block, _)| block.iter().flat_map(|s| s.to_le_bytes()))
        .collect();
    (onset, hash::sha256_hex(&mix))
}

fn throne_room_mix(ran: &testkit::Ran) -> (Option<u64>, String) {
    mix_from(ran, THRONE_ROOM)
}

#[test]
fn the_throne_room_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| throne_room(&AGAIN, &CHESS, false));
        (
            throne_room(&THRONE, &CHESS, true),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in THRONE_ROOM_KEPT {
        assert!(
            first.host.frame(flip).map(|f| &f.pixels) == second.host.frame(flip).map(|f| &f.pixels),
            "including what it presented at flip {flip}"
        );
    }
    assert!(
        first.host.audio_blocks == second.host.audio_blocks
            && first.host.audio_flips == second.host.audio_flips,
        "and what it mixed"
    );
    // And what it saved, each replay into a data root of its own.
    for name in ["prof1.lcp", "1save.lcs"] {
        let (one, two) = (saved(first, name), saved(second, name));
        assert_ne!(one, two, "two replays, two files");
        assert!(
            std::fs::read(&one).expect("first save is there")
                == std::fs::read(&two).expect("second save is there"),
            "and saved {name} the same"
        );
    }
}

/// The flips the two-player checkmate route keeps (#158): the Western board
/// ready with both Video 1 Advisors live, the pawn-shaped iris wipe into the
/// Western Queen-captures-Pawn capture cartoon, the red pawn scratching his
/// head over his pit trap in the capture cartoon, the "Checkmate!
/// Congratulations, you win!" dialog, and the throne room after clicking its
/// green tick.
const BOARD_READY: u64 = 555;
const CAPTURE_WIPE: u64 = 2180;
const CAPTURE_CARTOON: u64 = 3500;
const CHECKMATE_DIALOG: u64 = 4250;
const RETURNED_TO_THRONE: u64 = 4295;
const CHECKMATE_KEPT: [u64; 5] = [
    BOARD_READY,
    CAPTURE_WIPE,
    CAPTURE_CARTOON,
    CHECKMATE_DIALOG,
    RETURNED_TO_THRONE,
];

/// Enough instructions for the two-player Scholar's Mate route to play all
/// seven plies, its Western capture cartoon, checkmate, return to the throne
/// room and exit 0 at 11,685,933,862, with headroom so a run that reaches the
/// budget has not left.
const CHECKMATE_BUDGET: u64 = 12_000_000_000;

fn checkmate(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    shots: bool,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/checkmate.route"),
        ),
        screenshots: Screenshots {
            at: if shots {
                CHECKMATE_KEPT.to_vec()
            } else {
                Vec::new()
            },
            every: None,
        },
        ..Cli::default()
    };
    testkit::replay(
        slot,
        || scratch() && testkit::any_rip(&roster::GM16).is_some(),
        &CHESS,
        &cli,
        &CHECKMATE_KEPT,
        CHECKMATE_BUDGET,
    )
}

static CHECKMATE: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// A two-player (hot-seat) Quick Game played to checkmate with its capture
/// cartoon (#158). Extends `throne-room.route` from the Quick Game screen:
/// Black is toggled to human (keeping the matched Western sets and capture
/// animations on), the green tick starts the game, the engine allocates its
/// working table with `GlobalAlloc`/`GlobalLock`, both 16-bit Video 1 Advisors
/// (`sherriff.avi`, `baddude.avi`, #152) open, and `wtune.mid` plays on the GS
/// Wavetable synth (#156). Scholar's Mate (`1. e4 e5 2. Bc4 Nc6 3. Qh5 Nf6
/// 4. Qxf7#`) triggers the Western Queen-takes-Pawn 3D capture cartoon (12
/// stereo MS ADPCM effects) and reaches "Checkmate! Congratulations, you win!"
/// with its fanfare. Clicking the green tick frees the engine table with
/// `GlobalUnlock`/`GlobalFree` and returns to the throne room, where closing
/// the window exits 0.
#[test]
fn two_players_play_scholars_mate_with_capture_cartoon_to_checkmate() {
    let Some(ran) = checkmate(&CHECKMATE, true) else {
        return;
    };
    ran.assert_exited(0)
        .assert_flips(4311)
        .assert_logged("midi bank ")
        .assert_logged("GM16.DLS from ")
        .assert_logged("movie Advisors\\sherriff.avi opened through AVIFile: 1 streams")
        .assert_logged("movie Advisors\\baddude.avi opened through AVIFile: 1 streams")
        .assert_logged("flip 555 cursor 3628985292 640x480x16")
        .assert_logged("flip 2180 cursor 9651005883 640x480x16")
        .assert_logged("flip 3500 cursor 10594060931 640x480x16")
        .assert_logged("flip 4250 cursor 11473961232 640x480x16")
        .assert_logged("flip 4295 cursor 11651873115 640x480x16")
        .assert_logged("end exited 0 cursor 11685933866 flips 4311 declared-ms 109017");
    // Three MIDI opens, none refused: boot init (tune.mid), Western match
    // (wtune.mid), and throne room return (tune.mid).
    assert_eq!(ran.log.matches("midi stream open answered").count(), 3);
    assert_eq!(ran.log.matches("midi stream open answered 0").count(), 3);
    // 27 menu/throne-room sounds + 2 Western board + 12 capture cartoon stereo
    // effects + 1 checkmate fanfare + 12 throne room re-entry.
    assert_eq!(ran.log.matches("acm stream ").count(), 54);
    assert_eq!(ran.flip_before("acm stream 28:"), 512);
    assert_eq!(ran.flip_before("acm stream 30:"), 2190);
    assert_eq!(ran.flip_before("acm stream 43:"), 4260);
    for (flip, pinned) in [
        (
            BOARD_READY,
            "f21ff4e02f47c252d6b88a62b3ce5ad8e47db6f000a233f848dcdf079426c8d9",
        ),
        (
            CAPTURE_WIPE,
            "e7c820de1852cecbed04ef3fcf841bb7f21d58dd5d0dace61145d19526578237",
        ),
        (
            CAPTURE_CARTOON,
            "8c596cd7f4d090ebefc08e6aec2d4c946785809a3d123ab40fe8cd6081e770bc",
        ),
        (
            CHECKMATE_DIALOG,
            "d067caf185f7213c9ed0df690b3f2ff37ef144c187d57669a37cf88c0350d2da",
        ),
        (
            RETURNED_TO_THRONE,
            "a27b0a9581d3c47dfef9f8f0accf3909b5280758593722e504d545220fb26488",
        ),
    ] {
        let frame = ran.host.frame(flip).expect("presented");
        assert_eq!(hash::sha256_hex(&frame.pixels), pinned, "flip {flip}");
        let shot = ran.dir.join(format!("shots/{flip:010}.png"));
        assert!(shot.is_file(), "{} is written", shot.display());
    }
    for (label, from, to) in [
        ("chess match", BOARD_READY, CAPTURE_WIPE),
        ("capture cartoon", CAPTURE_WIPE, CHECKMATE_DIALOG),
        ("checkmate", CHECKMATE_DIALOG, RETURNED_TO_THRONE),
    ] {
        let rms = ran.loudness(from, to);
        assert!(
            !rms.is_empty() && rms.iter().any(|r| *r > 0.0),
            "{label} is heard"
        );
    }
    let (_, game_mix) = mix_from(ran, BOARD_READY);
    assert_eq!(
        game_mix, "92bfd3351f48a6159f792c34d8a528f885d4c1c49fa8dc57b0e735439e07126d",
        "the Western match, capture cartoon, checkmate fanfare and throne-room return mix"
    );
}

#[test]
fn the_checkmate_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| checkmate(&AGAIN, false));
        (
            checkmate(&CHECKMATE, true),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in CHECKMATE_KEPT {
        assert!(
            first.host.frame(flip).map(|f| &f.pixels) == second.host.frame(flip).map(|f| &f.pixels),
            "including what it presented at flip {flip}"
        );
    }
    assert!(
        first.host.audio_blocks == second.host.audio_blocks
            && first.host.audio_flips == second.host.audio_flips,
        "and what it mixed"
    );
    for (name, writes) in [("prof1.lcp", 2), ("1save.lcs", 1)] {
        let (one, two) = (
            saved_times(first, name, writes),
            saved_times(second, name, writes),
        );
        assert_ne!(one, two, "two replays, two files");
        assert!(
            std::fs::read(&one).expect("first save is there")
                == std::fs::read(&two).expect("second save is there"),
            "and saved {name} the same"
        );
    }
}

/// A Quick Game against the computer plays three replies without a wall (#158).
/// Not a committed route: the computer's replies depend on `srand(time(NULL))`
/// at engine init (`0x0046b3a3`) and the CRT `rand()` shared with animations,
/// so a core change shifting instruction counts can change which reply the
/// engine picks. White plays `1. e3`, `2. Nf3`, `3. d3` (all staying on ranks
/// 1-3 so moves 1 and 2 cannot be captured and no White piece projects into
/// ranks 5-8), and the test verifies both that each White origin square vacates
/// (proving Black's prior reply returned the turn) and that ranks 5-8 change
/// after each of the three moves, without pinning fragile engine-move hashes.
#[test]
fn a_quick_game_against_the_computer_plays_three_replies_without_a_wall() {
    if !scratch() {
        return;
    }
    let prefix = include_str!("../routes/throne-room.route")
        .strip_suffix("flip 470 close\n")
        .expect("throne-room.route ends on flip 470 close");
    let route = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("vs-computer-158-{}.route", std::process::id()));
    std::fs::write(
        &route,
        format!(
            "{prefix}\
flip 435 move 590 410\n\
flip 440 down\n\
flip 440 poll 300 up\n\
flip 555 move 356 308\n\
flip 560 down\n\
flip 563 up\n\
flip 568 move 353 272\n\
flip 573 down\n\
flip 576 up\n\
flip 850 move 511 349\n\
flip 855 down\n\
flip 858 up\n\
flip 863 move 419 272\n\
flip 868 down\n\
flip 871 up\n\
flip 1300 move 284 308\n\
flip 1305 down\n\
flip 1308 up\n\
flip 1313 move 287 272\n\
flip 1318 down\n\
flip 1321 up\n"
        ),
    )
    .expect("scratch route writes");
    const KEPT: [u64; 4] = [550, 840, 1290, 1790];
    const BUDGET: u64 = 9_500_000_000;
    let cli = Cli {
        route: Some(route),
        ..Cli::default()
    };
    let ran = testkit::run_on(
        lego_legacy_core::host::headless::Headless::new().keeping(&KEPT),
        &CHESS,
        &cli,
        BUDGET,
    );
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "three replies against the computer hit no wall\nlog:\n{}",
        ran.log
    );
    ran.assert_logged("movie Advisors\\sherriff.avi opened through AVIFile: 1 streams")
        .assert_logged("movie Advisors\\baddude.avi opened through AVIFile: 1 streams");
    // White's three moves (e2-e3, Ng1-f3, d2-d3) stay on ranks 1-3 (y >= 272);
    // ranks 5-8 (y 155..220, strictly above rank 4's border) change only as the
    // computer moves its pieces, and each White origin square vacates only once
    // the computer's previous reply has returned the turn.
    let black_side = (90, 155, 550, 220);
    let white_origins = [
        ("e2", (330, 285, 382, 330)),
        ("g1", (485, 325, 538, 372)),
        ("d2", (258, 285, 310, 330)),
    ];
    for (reply, (pair, (square, origin))) in KEPT.windows(2).zip(white_origins).enumerate() {
        let before = ran.host.frame(pair[0]).expect("before reply");
        let after = ran.host.frame(pair[1]).expect("after reply");
        let vacated = testkit::differing(before, after, origin);
        assert!(
            vacated > 200,
            "White move {} did not vacate {square}: only {vacated} pixels changed (flips {} -> {})",
            reply + 1,
            pair[0],
            pair[1],
        );
        let changed = testkit::differing(before, after, black_side);
        assert!(
            changed > 500,
            "computer reply {} changed only {changed} pixels on ranks 5-8 (flips {} -> {})",
            reply + 1,
            pair[0],
            pair[1],
        );
    }
}

// The machine has the floppy drive its CD check needs: the check at 0x405420
// looks at no drive without A: (#145). Core's shim tests run its calls on a
// machine with one and find D:.
const _: () = assert!(CHESS.declared.floppy);

/// The flips `routes/save-game.route` keeps (#159): the `1. e4 e5` board back
/// in White's view before opening Options, the "Save game" dialog showing
/// `1. 'Game'` typed into slot 1, and the Quick Game screen after resigning.
const SAVE_BOARD_BEFORE: u64 = 895;
const SAVE_DIALOG_NAMED: u64 = 988;
const SAVE_RETURNED: u64 = 1070;
const SAVE_GAME_KEPT: [u64; 3] = [SAVE_BOARD_BEFORE, SAVE_DIALOG_NAMED, SAVE_RETURNED];

/// Enough instructions for `routes/save-game.route` to save `'Game'`, resign
/// and exit 0 at 6,114,083,756, with headroom.
const SAVE_GAME_BUDGET: u64 = 6_400_000_000;

/// The flips `routes/load-game.route` keeps (#159): "Who's playing?" with Ada
/// already listed on boot, the fresh initial Western board before loading, the
/// "Load game" dialog showing `1. 'Game'`, the restored `1. e4 e5` board after
/// loading, `2. Bf1-c4` walking out on the restored board, and the Quick Game
/// screen after resigning with both sides restored to human.
const LOAD_WHO_IS_PLAYING: u64 = 310;
const LOAD_BOARD_INITIAL: u64 = 495;
const LOAD_DIALOG: u64 = 535;
const LOAD_BOARD_RESTORED: u64 = 580;
const LOAD_MOVE_PLAYED: u64 = 750;
const LOAD_RETURNED_2P: u64 = 815;
const LOAD_GAME_KEPT: [u64; 6] = [
    LOAD_WHO_IS_PLAYING,
    LOAD_BOARD_INITIAL,
    LOAD_DIALOG,
    LOAD_BOARD_RESTORED,
    LOAD_MOVE_PLAYED,
    LOAD_RETURNED_2P,
];

/// Enough instructions for `routes/load-game.route` to load Ada and `'Game'`,
/// play `2. Bf1-c4`, resign and exit 0 at 5,168,222,319, with headroom.
const LOAD_GAME_BUDGET: u64 = 5_400_000_000;

/// Two runs over one shared `--data-root`: `routes/save-game.route` followed by
/// `routes/load-game.route`, with copies of the saved files read between and
/// after them before the temporary data root is removed.
struct SaveLoadRuns {
    save_run: testkit::Ran,
    load_run: testkit::Ran,
    prof_bytes: Vec<u8>,
    save_bytes: Vec<u8>,
    after_load_prof: Vec<u8>,
    after_load_save: Vec<u8>,
}

fn save_and_load(
    slot: &'static std::sync::OnceLock<Option<SaveLoadRuns>>,
    shots: bool,
) -> Option<&'static SaveLoadRuns> {
    slot.get_or_init(|| {
        if !(scratch() && testkit::any_rip(&roster::GM16).is_some()) {
            return None;
        }
        let tag = if shots { "main" } else { "again" };
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("chess-save-load-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let run = |route: &str, kept: &[u64], budget: u64| {
            let cli = Cli {
                route: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(route)),
                data_root: Some(root.clone()),
                screenshots: Screenshots {
                    at: if shots { kept.to_vec() } else { Vec::new() },
                    every: None,
                },
                ..Cli::default()
            };
            testkit::run_on(
                lego_legacy_core::host::headless::Headless::new().keeping(kept),
                &CHESS,
                &cli,
                budget,
            )
        };

        let save_run = run("routes/save-game.route", &SAVE_GAME_KEPT, SAVE_GAME_BUDGET);
        let prof_path = saved(&save_run, "prof1.lcp");
        let save_path = saved_times(&save_run, "1save.lcs", 2);
        let prof_bytes = std::fs::read(&prof_path).expect("prof1.lcp reads");
        let save_bytes = std::fs::read(&save_path).expect("1save.lcs reads");

        let load_run = run("routes/load-game.route", &LOAD_GAME_KEPT, LOAD_GAME_BUDGET);
        let after_load_prof = std::fs::read(&prof_path).expect("prof1.lcp still reads");
        let after_load_save = std::fs::read(&save_path).expect("1save.lcs still reads");
        let _ = std::fs::remove_dir_all(&root);

        Some(SaveLoadRuns {
            save_run,
            load_run,
            prof_bytes,
            save_bytes,
            after_load_prof,
            after_load_save,
        })
    })
    .as_ref()
}

static SAVE_LOAD: std::sync::OnceLock<Option<SaveLoadRuns>> = std::sync::OnceLock::new();

fn assert_pinned_frames(ran: &testkit::Ran, pinned: &[(u64, &str)], label: &str) {
    for &(flip, expected) in pinned {
        let frame = ran.host.frame(flip).expect("presented");
        assert_eq!(
            hash::sha256_hex(&frame.pixels),
            expected,
            "{label} flip {flip}"
        );
        let shot = ran.dir.join(format!("shots/{flip:010}.png"));
        assert!(shot.is_file(), "{} is written", shot.display());
    }
}

/// Count pixels matching `pred` (given `[b, g, r]`) inside `(x0..x1, y0..y1)`.
fn matching_pixels(
    frame: &lego_legacy_core::host::headless::Presented,
    (x0, y0, x1, y1): (usize, usize, usize, usize),
    pred: impl Fn([u8; 3]) -> bool,
) -> usize {
    let stride = frame.width as usize;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (y * stride + x) * 4))
        .filter(|&at| pred([frame.pixels[at], frame.pixels[at + 1], frame.pixels[at + 2]]))
        .count()
}

/// Run 1 (#159): `routes/save-game.route` registers `Ada`, plays `1. e4 e5` in
/// a two-player Western Quick Game, opens Options -> Save Game, names slot 1
/// `'Game'`, writes `profiles\1save.lcs`, resigns cleanly, and exits 0.
/// Verifies every observed field of `prof1.lcp` (4,544 bytes) and `1save.lcs`
/// (190,920 bytes = 10 x 19,092-byte slots) and confirms `ChessSave.che` is
/// never written (its routines at 0x0040EA10 / 0x0040EBF0 are unreferenced).
#[test]
fn a_route_registers_a_player_saves_a_game_and_quits_cleanly() {
    let Some(runs) = save_and_load(&SAVE_LOAD, true) else {
        return;
    };
    let ran = &runs.save_run;
    ran.assert_exited(0)
        .assert_flips(1081)
        .assert_logged("end exited 0 cursor 6114083760 flips 1081 declared-ms 47500")
        .assert_logged(
            "file written C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\prof1.lcp, 4544 bytes",
        )
        .assert_logged(
            "file written C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\1save.lcs, 190920 bytes",
        );
    // Exactly 3 writes total: prof1.lcp once at creation and 1save.lcs twice
    // (initialized empty at creation, then updated with slot 0 'Game' on save).
    assert_eq!(ran.log.matches("file written ").count(), 3);
    assert!(
        !ran.log.contains("ChessSave.che"),
        "ChessSave.che is unreferenced dead code"
    );

    // profiles\prof1.lcp (4,544 bytes = 0x11C0).
    let prof = &runs.prof_bytes;
    assert_eq!(prof.len(), 4_544);
    assert!(prof.starts_with(b"Ada\0"));
    assert_eq!(prof[0x0014], 0, "empty last name");
    assert_eq!(prof[0x0028], 0x18, "hardcoded default age 24 (0x00416a92)");
    assert!(
        prof[0x112C..0x1198].iter().all(|&b| b == 0xFF),
        "27 x -1 puzzle/tutorial completion markers"
    );
    assert_eq!(prof[0x11AC], 200, "default SFX volume");
    assert_eq!(
        prof[0x11AD], 200,
        "default music volume (0xFA00 in midiOutSetVolume)"
    );
    assert_eq!(&prof[0x11BD..0x11BF], &[60, 60], "default 60-minute clocks");
    assert_eq!(
        hash::sha256_hex(prof),
        "dfd46ce84ec1ea9da598e151963d06b4febd4a8e07f20e6b437836c97d68491c"
    );

    // profiles\1save.lcs (190,920 bytes = 10 slots x 19,092 bytes [0x4A94]).
    let saves = &runs.save_bytes;
    const SLOT_LEN: usize = 0x4A94;
    assert_eq!(saves.len(), 10 * SLOT_LEN);
    let slot0 = &saves[..SLOT_LEN];
    assert!(slot0.starts_with(b"Game\0"), "slot 0 label is 'Game'");
    assert_eq!(
        &slot0[0x0028..0x002A],
        0u16.to_le_bytes(),
        "computer-control mask 0 (two-player human vs human)"
    );
    // Ply 0: e2 (0x34) -> e4 (0x24); Ply 1: e7 (0x0C) -> e5 (0x1C).
    assert_eq!(&slot0[0x002A..0x002C], &[0x34, 0x24], "1. e2-e4");
    assert_eq!(&slot0[0x0032..0x0034], &[0x0C, 0x1C], "1... e7-e5");
    assert_eq!(
        &slot0[0x402C..0x4030],
        2u32.to_le_bytes(),
        "2 plies recorded"
    );
    assert_eq!(
        &slot0[0x4030..0x4034],
        0i32.to_le_bytes(),
        "slot 0 active (not -1)"
    );
    assert_eq!(
        &slot0[0x4035..0x403A],
        &[0, 0, 0, 0, 75],
        "Western, diff 25 (75% rand)"
    );
    assert_eq!(&slot0[0x4048..0x404C], 1u32.to_le_bytes(), "White human");
    assert_eq!(&slot0[0x404C..0x4050], 1u32.to_le_bytes(), "Black human");
    // Starting 8x8 piece-ID map at +0x4A54 (piece IDs 1..=32 on ranks 8,7,2,1).
    let board64 = &slot0[0x4A54..0x4A94];
    assert_eq!(board64.iter().filter(|&&id| id != 0).count(), 32);
    // Slots 1..=7 remain empty ('....' + -1 marker); slots 8..=9 are zeroed.
    for s in 1..8 {
        let sl = &saves[s * SLOT_LEN..(s + 1) * SLOT_LEN];
        assert!(sl.starts_with(b"....\0"), "slot {s} empty label");
        assert_eq!(
            &sl[0x4030..0x4034],
            (-1i32).to_le_bytes(),
            "slot {s} empty marker"
        );
    }
    assert!(saves[8 * SLOT_LEN..].iter().all(|&b| b == 0));
    // #338: byte +0x0031 (uninitialized top byte of ply 0's 8-byte record)
    // moved from 0x22 to 0x21 because DirectDraw surface pooling keeps the
    // guest heap below 0x2200_0000; all other 190,919 bytes are identical.
    assert_eq!(
        hash::sha256_hex(saves),
        "7b5f0eb4ebca93ee07f6978a62a7fdadc421c4aed05b06d5a9b16b8901463aae"
    );

    assert_pinned_frames(
        ran,
        &[
            (
                SAVE_BOARD_BEFORE,
                "c7977c4f3dca2b38d1cc908d372ed6f1bcd1786c7bae47f77fcfe5e503141639",
            ),
            (
                SAVE_DIALOG_NAMED,
                "4c8ac04ba26e4cc011ffcafcc2cc3acee31638de7301eb4c1e864654cc32f59b",
            ),
            (
                SAVE_RETURNED,
                "569e88ac949c0bc29240022fdd375d0c4e1da767fe69c146a93110c885976a85",
            ),
        ],
        "save",
    );
}

/// Run 2 (#159): `routes/load-game.route` over the exact `--data-root` left by
/// `routes/save-game.route` skips the New Player form, lists `Ada` on
/// "Who's playing?", opens a default Quick Game (Black = computer), loads slot
/// 1 `'Game'` from `profiles\1save.lcs`, visibly restores `1. e4 e5` and
/// two-player mode, plays `2. Bf1-c4`, resigns (showing both sides human on
/// Quick Game), and exits 0 without overwriting the saves.
#[test]
fn a_second_run_over_the_same_data_root_finds_the_player_and_loads_the_game() {
    let Some(runs) = save_and_load(&SAVE_LOAD, true) else {
        return;
    };
    let ran = &runs.load_run;
    ran.assert_exited(0)
        .assert_flips(826)
        .assert_logged("end exited 0 cursor 5168222315 flips 826 declared-ms 37717")
        // Positive proof that 0x0041DE60 opened prof1.lcp and continued to prof2.lcp.
        .assert_logged(
            "file missing C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\prof2.lcp, asked from 0x0045656a",
        );
    assert!(
        !ran.log.contains(
            "file missing C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\prof1.lcp"
        ),
        "Run 2 finds Ada's existing prof1.lcp without asking for a new player"
    );
    assert!(
        !ran.log.contains(
            "file missing C:\\Program Files\\LEGO Media\\Games\\LEGO Chess\\profiles\\1save.lcs"
        ),
        "Run 2 finds Ada's existing 1save.lcs"
    );
    assert!(
        !ran.log.contains("file written"),
        "Run 2 only reads the saved profile and game"
    );
    assert!(
        runs.after_load_prof == runs.prof_bytes && runs.after_load_save == runs.save_bytes,
        "Run 2 leaves the saved files on disk intact"
    );

    // Camera-shift-invariant pawn-color proof on the unoccluded e-file squares
    // that loading 'Game' changed the board from the initial position (flip 495)
    // to 1. e4 e5 (flip 580): e7's red pawn vacates, and red/blue pawns appear
    // on e5 and e4.
    let initial = ran.host.frame(LOAD_BOARD_INITIAL).expect("initial board");
    let restored = ran.host.frame(LOAD_BOARD_RESTORED).expect("restored board");
    let red_pawn = |[b, g, r]: [u8; 3]| r > 150 && g < 60 && b < 60;
    let blue_pawn = |[b, _g, r]: [u8; 3]| b > 140 && r < 80;
    assert!(
        matching_pixels(initial, (320, 150, 380, 170), red_pawn) > 50
            && matching_pixels(restored, (320, 150, 380, 170), red_pawn) == 0,
        "e7 red pawn vacates after loading 1. e4 e5"
    );
    assert!(
        matching_pixels(initial, (320, 185, 380, 215), red_pawn) == 0
            && matching_pixels(restored, (320, 185, 380, 215), red_pawn) > 50,
        "e5 holds Black's red pawn after loading 1. e4 e5"
    );
    assert!(
        matching_pixels(initial, (320, 215, 380, 245), blue_pawn) == 0
            && matching_pixels(restored, (320, 215, 380, 245), blue_pawn) > 50,
        "e4 holds White's blue pawn after loading 1. e4 e5"
    );

    assert_pinned_frames(
        ran,
        &[
            (
                LOAD_WHO_IS_PLAYING,
                "61c5f8202fb6516557d6bfb7556d39f7725d41da522bd095d9e33c9b2d3f7849",
            ),
            (
                LOAD_BOARD_INITIAL,
                "305d4c40aa3d4d9c32c0f2afdf0bb3f191e201d8411f2574761714a39f8ea76f",
            ),
            (
                LOAD_DIALOG,
                "960d1ab0c58ccb288b72cec8a02bae1c8655c4f2f007ce7b4d2a59313199b178",
            ),
            (
                LOAD_BOARD_RESTORED,
                "d5a01927fdcf6ac4e4cd218812435a28ecd55f1ebbc84fedc289acdad7453770",
            ),
            (
                LOAD_MOVE_PLAYED,
                "df2217a77c502c92805dbeb53dc25a0dc9e682855f69e042361186b08843c516",
            ),
            (
                LOAD_RETURNED_2P,
                "76cfbd23120a2b6af184754b8ef3130849aeb4a18775f6cf61ae9098ba7daad1",
            ),
        ],
        "load",
    );
}

#[test]
fn the_save_and_load_routes_replay_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<SaveLoadRuns>> = std::sync::OnceLock::new();
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| save_and_load(&AGAIN, false));
        (
            save_and_load(&SAVE_LOAD, true),
            second.join().expect("the second save/load replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    for (label, a, b, kept) in [
        (
            "save",
            &first.save_run,
            &second.save_run,
            &SAVE_GAME_KEPT[..],
        ),
        (
            "load",
            &first.load_run,
            &second.load_run,
            &LOAD_GAME_KEPT[..],
        ),
    ] {
        a.assert_same_machine(b);
        for &flip in kept {
            assert!(
                a.host.frame(flip).map(|f| &f.pixels) == b.host.frame(flip).map(|f| &f.pixels),
                "{label} run presented the same at flip {flip}"
            );
        }
        assert!(
            a.host.audio_blocks == b.host.audio_blocks && a.host.audio_flips == b.host.audio_flips,
            "{label} run mixed the same"
        );
    }
    assert_ne!(
        saved(&first.save_run, "prof1.lcp"),
        saved(&second.save_run, "prof1.lcp"),
        "two replays, two data roots"
    );
    assert_ne!(
        saved_times(&first.save_run, "1save.lcs", 2),
        saved_times(&second.save_run, "1save.lcs", 2),
        "two replays, two save files"
    );
    assert!(
        first.prof_bytes == second.prof_bytes
            && first.save_bytes == second.save_bytes
            && first.after_load_prof == second.after_load_prof
            && first.after_load_save == second.after_load_save,
        "both runs wrote and kept byte-identical profiles and saves"
    );
}

/// Flips kept from `routes/pirates-vs-ai-checkmate.route` (#303, #304): the
/// Pirates 3D board ready with Admiral and Pirate Advisors, a mid-game 3D board
/// frame (`7... Ng8-e7`), the `8. Bb5xd7+` Bishop-takes-Queen 3D Pirate capture
/// cartoon, the `26. Qc6xa8#` Queen-takes-Rook Checkmate capture cartoon, the
/// `"Checkmate! Congratulations, you win!"` dialog, and the Throne Room after
/// clicking its green tick.
const PIRATES_BOARD_READY: u64 = 555;
const PIRATES_MIDGAME: u64 = 5288;
const PIRATES_CAPTURE_BISHOP_QUEEN: u64 = 5614;
const PIRATES_CAPTURE_CHECKMATE: u64 = 18352;
const PIRATES_CHECKMATE_DIALOG: u64 = 18800;
const PIRATES_RETURNED_TO_THRONE: u64 = 18850;
const PIRATES_AI_KEPT: [u64; 6] = [
    PIRATES_BOARD_READY,
    PIRATES_MIDGAME,
    PIRATES_CAPTURE_BISHOP_QUEEN,
    PIRATES_CAPTURE_CHECKMATE,
    PIRATES_CHECKMATE_DIALOG,
    PIRATES_RETURNED_TO_THRONE,
];

/// Enough instructions for the 26-move Pirates 3D Quick Game vs Computer AI
/// route to play all 51 plies, nine captures (3D Pirate capture cartoons),
/// Checkmate victory, Throne Room return, and clean exit 0 at 57,479,034,089.
const PIRATES_AI_BUDGET: u64 = 60_000_000_000;

fn pirates_vs_ai(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
    shots: bool,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("routes/pirates-vs-ai-checkmate.route"),
        ),
        screenshots: Screenshots {
            at: if shots {
                PIRATES_AI_KEPT.to_vec()
            } else {
                Vec::new()
            },
            every: None,
        },
        ..Cli::default()
    };
    testkit::replay(
        slot,
        || scratch() && testkit::any_rip(&roster::GM16).is_some(),
        &CHESS,
        &cli,
        &PIRATES_AI_KEPT,
        PIRATES_AI_BUDGET,
    )
}

static PIRATES_AI: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();

/// A full Quick Game against the Oxford Softworks Computer AI in the Pirates 3D
/// theme played to Checkmate (#303, #304). Boots from a fresh `--data-root`,
/// registers `Ada`, switches Quick Game Theme to Pirates while keeping Black as
/// Computer AI (default difficulty 25, matched sets, cartoons on), loads
/// `Advisors\admiral.avi` and `Advisors\pirate.avi` with `ptune.mid`, plays 26
/// legal moves (51 plies, 9 captures triggering 3D Pirate capture cartoons) to
/// `26. Qc6xa8#`, reaches `"Checkmate! Congratulations, you win!"`, returns to
/// the Throne Room via `(320,232)`, and exits 0 cleanly.
#[test]
fn a_quick_game_against_the_computer_in_pirates_theme_reaches_checkmate() {
    let Some(ran) = pirates_vs_ai(&PIRATES_AI, true) else {
        return;
    };
    ran.assert_exited(0)
        .assert_flips(18_861)
        .assert_logged("midi bank ")
        .assert_logged("GM16.DLS from ")
        .assert_logged("movie Advisors\\admiral.avi opened through AVIFile: 1 streams")
        .assert_logged("movie Advisors\\pirate.avi opened through AVIFile: 1 streams")
        .assert_logged("flip 555 cursor 3740281865 640x480x16")
        .assert_logged("flip 5288 cursor 18995341063 640x480x16")
        .assert_logged("flip 5614 cursor 19749024596 640x480x16")
        .assert_logged("flip 18352 cursor 56607183199 640x480x16")
        .assert_logged("flip 18800 cursor 57272521987 640x480x16")
        .assert_logged("flip 18850 cursor 57456415643 640x480x16")
        .assert_logged("end exited 0 cursor 57479034089 flips 18861 declared-ms 500683");

    // Three MIDI opens, none refused: boot init (tune.mid), Pirates match
    // (ptune.mid), and Throne Room return (tune.mid).
    assert_eq!(ran.log.matches("midi stream open answered").count(), 3);
    assert_eq!(ran.log.matches("midi stream open answered 0").count(), 3);
    // 27 menu/Throne Room + Pirates board + 9 Pirate capture cartoons +
    // checkmate fanfare + 12 Throne Room re-entry.
    assert_eq!(ran.log.matches("acm stream ").count(), 82);

    for flip in PIRATES_AI_KEPT {
        let frame = ran.host.frame(flip).expect("presented");
        assert!(!frame.is_uniform(), "flip {flip} is non-uniform");
    }
    assert_pinned_frames(
        ran,
        &[
            (
                PIRATES_BOARD_READY,
                "038b08957a52e51ce163eecf7a401dae5b10fed0bddee1c0fb453692c294f578",
            ),
            (
                PIRATES_MIDGAME,
                "a0d6f75cc4d904cffe7061228ed930dc6520dd64f9257bec016385f2bfc3a9ec",
            ),
            (
                PIRATES_CAPTURE_BISHOP_QUEEN,
                "f51b5161c824fb097a67ff038cd2531c1a6546e6efa6cd1866a4d689c7b3de1a",
            ),
            (
                PIRATES_CAPTURE_CHECKMATE,
                "7fa4d42ae0ce4e15fc667dd535d00ba1368b28ee591d535f37763dcb7c7493e2",
            ),
            (
                PIRATES_CHECKMATE_DIALOG,
                "78cb87f1066b6c76f42f52cf88e6dcb2199ad36eff9f59ec40b236f0b147b3c8",
            ),
            (
                PIRATES_RETURNED_TO_THRONE,
                "316fce2dd184ded7d07383ab1ce238169a40bc35773bed1dff92fca453bd059f",
            ),
        ],
        "pirates-vs-ai",
    );

    let checkmate_frame = ran.host.frame(PIRATES_CHECKMATE_DIALOG).expect("presented");
    let green_tick = |[b, g, r]: [u8; 3]| g > 140 && r < 80 && b < 80;
    assert!(
        matching_pixels(checkmate_frame, (295, 205, 345, 260), green_tick) > 400,
        "Checkmate victory dialog shows the green confirmation tick at (320, 232)"
    );

    for (label, from, to) in [
        (
            "Pirates match",
            PIRATES_BOARD_READY,
            PIRATES_CAPTURE_BISHOP_QUEEN,
        ),
        (
            "Pirates capture cartoons",
            PIRATES_CAPTURE_BISHOP_QUEEN,
            PIRATES_CHECKMATE_DIALOG,
        ),
        (
            "Checkmate fanfare",
            PIRATES_CHECKMATE_DIALOG,
            PIRATES_RETURNED_TO_THRONE,
        ),
    ] {
        let rms = ran.loudness(from, to);
        assert!(
            !rms.is_empty() && rms.iter().any(|r| *r > 0.0),
            "{label} is heard"
        );
    }

    // Profile is written at creation plus once per capture cartoon (10 writes)
    // and records 6 distinct Pirate capture cartoon scrapbook unlocks.
    let prof_path = saved_times(ran, "prof1.lcp", 10);
    let prof = std::fs::read(&prof_path).expect("prof1.lcp reads");
    assert_eq!(prof.len(), 4_544);
    assert!(prof.starts_with(b"Ada\0"));
    assert_eq!(
        prof[0x002C..0x112C].iter().filter(|&&b| b != 0).count(),
        6,
        "6 distinct Pirate capture cartoons unlocked in Scrapbook"
    );
    assert_eq!(
        hash::sha256_hex(&prof),
        "b629d3c62284f55276fc3c29ccfca4cbe92b0377d174c545a14b663c3149dc07"
    );
}

#[test]
fn the_pirates_vs_ai_checkmate_route_replays_to_the_same_machine_every_time() {
    static AGAIN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let (first, second) = std::thread::scope(|scope| {
        let second = scope.spawn(|| pirates_vs_ai(&AGAIN, false));
        (
            pirates_vs_ai(&PIRATES_AI, true),
            second.join().expect("the second Pirates AI replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in PIRATES_AI_KEPT {
        assert!(
            first.host.frame(flip).map(|f| &f.pixels) == second.host.frame(flip).map(|f| &f.pixels),
            "including what it presented at flip {flip}"
        );
    }
    assert!(
        first.host.audio_blocks == second.host.audio_blocks
            && first.host.audio_flips == second.host.audio_flips,
        "and what it mixed"
    );
    for (name, writes) in [("prof1.lcp", 10), ("1save.lcs", 1)] {
        let (one, two) = (
            saved_times(first, name, writes),
            saved_times(second, name, writes),
        );
        assert_ne!(one, two, "two replays, two fresh data roots");
        assert!(
            std::fs::read(&one).expect("first save is there")
                == std::fs::read(&two).expect("second save is there"),
            "and saved {name} the same"
        );
    }
}
