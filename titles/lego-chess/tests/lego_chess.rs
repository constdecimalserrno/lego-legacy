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
        .assert_logged("flip 2 cursor 770297 640x480x16");
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
/// 72,761,539, and a movie frame more.
const FIRST_LIGHT_BUDGET: u64 = 84_000_000;

/// The committed route to first light, replayed, keeping the frame before
/// first light, first light and the brick, and writing the last two as
/// pictures in its run directory. Two slots, so the determinism gate's two
/// runs can go side by side.
fn first_light(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/first-light.route"),
        ),
        screenshots: Screenshots {
            at: vec![FIRST_LIGHT, FIRST_BRICK],
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
    let Some(ran) = first_light(&LIT) else { return };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing on the way to first light stops the run\nlog:\n{}",
        ran.log
    );
    ran.assert_logged("flip 7 cursor 45093881 640x480x16");

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
        let second = scope.spawn(|| first_light(&AGAIN));
        (
            first_light(&LIT),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in [FIRST_LIGHT, FIRST_BRICK] {
        assert_eq!(
            first.host.frame(flip).map(|f| &f.pixels),
            second.host.frame(flip).map(|f| &f.pixels),
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
/// the intro to reach its frame 12, flip 340 at 3,695,170,490, and no more:
/// the route's cap, as its own comment gives it.
const INTRO_BUDGET: u64 = 3_700_000_000;

/// The committed route through the intro's first two movies and into the
/// third, replayed, keeping the flips a person checks and writing each as a
/// picture in its run directory. Two slots, so the determinism gate's two
/// runs can go side by side.
fn intro(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let kept = [LOGO_NEAR_ITS_END, KRISALIS_LAST, INTRO_TENTH];
    let cli = Cli {
        route: Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/intro.route")),
        screenshots: Screenshots {
            at: kept.to_vec(),
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
    let Some(ran) = intro(&INTRO) else { return };
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted,
        "nothing in the intro stops the run\nlog:\n{}",
        ran.log
    );
    ran.assert_logged("flip 340 cursor 3695170490 640x480x16");
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
/// as main made them running every pass (5d8104c, and e9bed42 after #215).
#[test]
fn the_intro_s_clock_spins_are_passed_and_land_where_running_every_pass_lands() {
    let Some(ran) = intro(&INTRO) else { return };
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
            "7938e0e28241b721b6c85fd7a999f8e012f9d0d7c54e1cc56ee3751b1fde5dad",
            "e99db84e0baf0ce60ecf7f43007ae0a6eba844bb8ad31e40d250379d101fa24a",
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
        let second = scope.spawn(|| intro(&AGAIN));
        (intro(&INTRO), second.join().expect("the second replay ran"))
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    for flip in [LOGO_NEAR_ITS_END, KRISALIS_LAST, INTRO_TENTH] {
        assert_eq!(
            first.host.frame(flip).map(|f| &f.pixels),
            second.host.frame(flip).map(|f| &f.pixels),
            "including what it presented at flip {flip}"
        );
    }
}

/// Krisalis' logo's fifth frame, where the skip route presses Space, and the
/// intro's tenth frame after the skip, where it ends (#215).
const KRISALIS_FIFTH: u64 = 290;
const SKIPPED_TO_THE_TENTH: u64 = 305;

/// Enough instructions for the logo to play through, Krisalis' logo to be
/// skipped, and the intro to reach its frame 10, flip 305 at 2,971,328,901,
/// and no more.
const SKIP_BUDGET: u64 = 2_975_000_000;

/// The committed route that skips Krisalis' logo with Space, replayed, keeping
/// the intro's tenth frame and writing it as a picture in its run directory.
/// Two slots, so the determinism gate's two runs can go side by side.
fn skipped(
    slot: &'static std::sync::OnceLock<Option<testkit::Ran>>,
) -> Option<&'static testkit::Ran> {
    let kept = [SKIPPED_TO_THE_TENTH];
    let cli = Cli {
        route: Some(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("routes/skip-krisalis.route"),
        ),
        screenshots: Screenshots {
            at: kept.to_vec(),
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
    let Some(ran) = skipped(&SKIPPED) else { return };
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
    .assert_logged("flip 305 cursor 2971328901 640x480x16");
    // The two clears follow at once: the hold was over as Space was seen.
    ran.assert_logged("flip 291 cursor 2706376505 640x480x16");
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
        let second = scope.spawn(|| skipped(&AGAIN));
        (
            skipped(&SKIPPED),
            second.join().expect("the second replay ran"),
        )
    });
    let (Some(first), Some(second)) = (first, second) else {
        return;
    };
    first.assert_same_machine(second);
    assert_eq!(
        first.host.frame(SKIPPED_TO_THE_TENTH).map(|f| &f.pixels),
        second.host.frame(SKIPPED_TO_THE_TENTH).map(|f| &f.pixels),
        "including what it presented at flip {SKIPPED_TO_THE_TENTH}"
    );
}

// The machine has the floppy drive its CD check needs: the check at 0x405420
// looks at no drive without A: (#145). Core's shim tests run its calls on a
// machine with one and find D:.
const _: () = assert!(CHESS.declared.floppy);
