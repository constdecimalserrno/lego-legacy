//! LEGO Racers, run headless from the player's rip: the launcher finding the
//! Masterpiece pressing or refusing the SafeDisc-wrapped 1999 original, its
//! declared cabinet install tree, and its boot as far as the machine carries
//! it (#369).
//!
//! Disc tests need the player's own rips under `game-data/`. Without them the
//! disc tests print a skip line and pass, which is a gate that did not run
//! rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into
//! a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::route::Route;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{hash, pe, testkit, Title};
use lego_racers::LEGO_RACERS as RACERS;
use std::path::PathBuf;

const ROOT: &str = RACERS.install_dir;
const BOOT_BUDGET: u64 = 200_000;
const FIRST_LIGHT_BUDGET: u64 = 200_000_000;
const FULL_INTRO_BUDGET: u64 = 250_000_000;
const BUILD_RACER_BUDGET: u64 = 1_000_000_000;
const FIRST_RACE_BUDGET: u64 = 6_000_000_000;

fn route_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("routes")
        .join(name)
}

/// The default unflagged boot path, run once for 200k instructions to verify
/// cabinet loading, DirectInput setup, and the start of `lmicmp.avi` playback.
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    testkit::replay(&RAN, scratch, &RACERS, &cli, &[0, 5], BOOT_BUDGET)
}

/// The unflagged first-light route: plays unskippable `lmicmp.avi`, skips
/// `hvscmp.avi` (Space) and `introcmp.avi` (mouse click), lands on the
/// animated 3D Main Menu, and exits cleanly (#370, #372).
fn first_light() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &RACERS,
        &cli,
        &[80, 239, 245, 300, 550, 580],
        FIRST_LIGHT_BUDGET,
    )
}

/// LEGO Racers pointed at its original 1999 pressing, whose cabinet-packed
/// executable is wrapped in SafeDisc v1 (ADR-0006), still declaring the bytes
/// of the Masterpiece's (#369).
const RACERS_WRAPPED: Title = Title {
    image: EntryImage {
        pressing: "racers",
        ..RACERS.image
    },
    ..RACERS
};

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&RACERS.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&RACERS),
        Found::Runnable {
            pressing: "masterpiece-racers".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..RACERS.image
        },
        ..RACERS
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
fn a_wrapped_image_is_named_and_never_run() {
    runs();
    if !testkit::rips(&RACERS_WRAPPED.image) {
        return;
    }
    assert_eq!(
        roster::look_for(&RACERS_WRAPPED),
        Found::Wrapped {
            pressing: "racers".to_owned(),
            scheme: "SafeDisc".to_owned(),
        }
    );

    let err = testkit::try_run_headless(&RACERS_WRAPPED, &Cli::default(), 100_000)
        .expect_err("a SafeDisc wrapper is refused");

    assert!(
        err.contains("SafeDisc"),
        "the refusal names the scheme: {err}"
    );
    assert!(
        err.contains(&format!("{}racers.iso", std::path::MAIN_SEPARATOR)),
        "and the image it is in: {err}"
    );
    assert!(
        err.contains("never runs, decrypts or bypasses"),
        "the refusal says what it will not do: {err}"
    );
}

#[test]
fn a_pressing_that_is_not_the_declared_one_is_refused_by_its_own_digest() {
    if !scratch() {
        return;
    }
    const PATCHED: Title = Title {
        image: EntryImage {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..RACERS.image
        },
        ..RACERS
    };
    let err = testkit::try_run_headless(&PATCHED, &Cli::default(), 1_000)
        .expect_err("a title is the bytes it is made of");

    assert!(
        err.contains("dcaea5ff"),
        "the refusal names what it found: {err}"
    );
}

#[test]
fn the_declared_tree_unpacks_the_masterpiece_cabinet() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&RACERS);
    assert_eq!(tree.declared_files().len(), 56);

    let exe = tree
        .read(&format!("{ROOT}\\LEGORacers.exe"))
        .expect("LEGORacers.exe is unpacked from DATA1.CAB");
    assert_eq!(exe.len(), 987_222);
    assert_eq!(hash::sha256_hex(&exe), RACERS.image.sha256);
    let exe_pe = pe::inspect(&exe).expect("LEGORacers.exe is a 32-bit PE");
    assert_eq!(pe::wrapper_scheme(&exe_pe.sections), None);
    assert_eq!(exe_pe.preferred_base, 0x0040_0000);
    assert_eq!(exe_pe.entry, 0x000b_ef2b);
    assert_eq!(exe_pe.size, 0x000f_f000);

    let goldp = tree
        .read(&format!("{ROOT}\\GolDP.dll"))
        .expect("GolDP.dll is unpacked from DATA1.CAB");
    assert_eq!(goldp.len(), 413_696);
    assert_eq!(
        hash::sha256_hex(&goldp),
        "cc0120eb92b1881a42ea0dcbb0a7824e0095cac0acf5d69878896005f345ea62"
    );
    let dll_pe = pe::inspect(&goldp).expect("GolDP.dll is a 32-bit PE");
    assert_eq!(pe::wrapper_scheme(&dll_pe.sections), None);
    assert_eq!(dll_pe.preferred_base, 0x1000_0000);
    assert_eq!(dll_pe.entry, 0x0004_9f97);
    assert_eq!(dll_pe.size, 0x0006_9000);

    let jam = tree
        .read(&format!("{ROOT}\\LEGO.JAM"))
        .expect("LEGO.JAM is unpacked from DATA1.CAB");
    assert_eq!(jam.len(), 19_642_024);
    assert_eq!(&jam[..4], b"LJAM");
    assert_eq!(
        hash::sha256_hex(&jam),
        "eb38feb756887e3c8ed087c38faf52302cbf427f7a753c989d8bda86724ac27d"
    );

    for name in ["theme.tun", "lmicmp.avi", "HVSCmp.avi", "introcmp.avi"] {
        assert!(
            tree.readable(&format!("{ROOT}\\{name}")),
            "{name} is declared in the cabinet"
        );
    }
}

#[test]
fn the_entry_image_loads_from_the_cabinet_and_starts_calling() {
    let Some(ran) = boot() else {
        return;
    };
    ran.assert_logged("image LEGORacers.exe")
        .assert_logged("base 0x00400000")
        .assert_logged("imports 147")
        .assert_logged("installed 56 files")
        .assert_logged("dshow lmicmp.avi rendered: 640x480 IV50, 237 frames")
        .assert_logged("dshow lmicmp.avi running");
    assert!(
        ran.finish.flips > 0,
        "unflagged boot should decode and present lmicmp.avi frames"
    );
}

#[test]
fn the_boot_replays_to_the_same_machine_every_time() {
    let Some(first) = boot() else {
        return;
    };
    let cli = Cli {
        trace: Trace::Shims,
        ..Cli::default()
    };
    let second = testkit::run_headless(&RACERS, &cli, BOOT_BUDGET);
    first.assert_same_machine(&second);
}

#[test]
fn first_light_reaches_main_menu_and_exits_cleanly() {
    let Some(ran) = first_light() else {
        return;
    };
    ran.assert_exited(0)
        .assert_flips(581)
        .assert_logged("dshow lmicmp.avi rendered: 640x480 IV50, 237 frames")
        .assert_logged("dshow hvscmp.avi rendered: 640x480 IV50, 337 frames")
        .assert_logged("dshow introcmp.avi rendered: 640x480 IV50, 1080 frames")
        .assert_logged("dshow introcmp.avi released")
        .assert_logged("load library GolDP.DLL")
        .assert_logged("d3d3 device at")
        .assert_logged("winmm timer 1:")
        .assert_logged("winmm timer 1 killed");

    // Verify visible decoded video frames across all three intro movies plus
    // splash and the animated 3D Captain Redbeard Main Menu.
    let lmicmp = ran.host.frame(80).expect("lmicmp flip 80 kept");
    let hvscmp = ran.host.frame(239).expect("hvscmp flip 239 kept");
    let introcmp = ran.host.frame(245).expect("introcmp flip 245 kept");
    let splash = ran.host.frame(300).expect("splash flip 300 kept");
    let menu_start = ran.host.frame(550).expect("menu flip 550 kept");
    let menu_end = ran.host.frame(580).expect("menu flip 580 kept");
    assert_eq!((lmicmp.width, lmicmp.height), (640, 480));
    assert_eq!((menu_start.width, menu_start.height), (640, 480));
    assert!(!lmicmp.is_uniform(), "lmicmp.avi frame 80 is non-uniform");
    assert!(!hvscmp.is_uniform(), "hvscmp.avi frame 239 is non-uniform");
    assert!(
        !introcmp.is_uniform(),
        "introcmp.avi frame 245 is non-uniform"
    );
    assert!(!splash.is_uniform(), "splash frame 300 is non-uniform");
    assert!(
        !menu_start.is_uniform(),
        "main menu flip 550 is non-uniform"
    );
    assert!(!menu_end.is_uniform(), "main menu flip 580 is non-uniform");
    assert_ne!(
        splash.pixels, menu_start.pixels,
        "splash transitions cleanly into main menu"
    );
    assert_ne!(
        menu_start.pixels, menu_end.pixels,
        "3D main menu animates across flips 550..580"
    );

    // Intro movies must mix non-silent PCM audio into the declared mixer.
    let intro_audio_nonzero = ran
        .host
        .audio_blocks
        .iter()
        .zip(&ran.host.audio_flips)
        .filter(|(_, &flip)| flip < 247)
        .any(|(block, _)| block.iter().any(|&s| s != 0));
    assert!(
        intro_audio_nonzero,
        "DirectShow intro movies should mix non-silent PCM audio"
    );
}

fn pcm_from(ran: &testkit::Ran, from_flip: u64) -> Vec<i16> {
    ran.host
        .audio_blocks
        .iter()
        .zip(&ran.host.audio_flips)
        .filter(|(_, &flip)| flip >= from_flip)
        .flat_map(|(block, _)| block.iter().copied())
        .collect()
}

#[test]
fn first_light_streams_non_silent_tun_audio_on_main_menu() {
    let Some(ran) = first_light() else {
        return;
    };
    let menu_samples = pcm_from(ran, 547);
    assert!(
        menu_samples.len() >= 10_000,
        "expected >= 10,000 stereo PCM samples on Main Menu, got {}",
        menu_samples.len()
    );

    let (sum, sum_sq) = menu_samples.iter().fold((0.0f64, 0.0f64), |(s, sq), &x| {
        let v = f64::from(x);
        (s + v, sq + v * v)
    });
    let n = menu_samples.len() as f64;
    let mean = sum / n;
    let rms = (sum_sq / n).sqrt();
    let variance = sum_sq / n - mean * mean;
    let active_blocks = ran
        .loudness(547, 582)
        .into_iter()
        .filter(|&r| r > 100.0)
        .count();

    assert!(
        rms > 500.0,
        "expected genuine non-silent theme.tun PCM audio (RMS > 500), got {rms:.1}"
    );
    assert!(
        variance > 250_000.0,
        "expected non-constant audio waveform (variance > 250,000), got {variance:.1}"
    );
    assert!(
        active_blocks >= 4,
        "expected sustained streaming music across >= 4 blocks, got {active_blocks}"
    );
}

#[test]
fn main_menu_music_streams_fresh_tun_pcm_past_the_circular_buffer_wrap() {
    if !scratch() {
        return;
    }
    // `theme.tun` pre-fills a ~2.04 s (0x2c000 B) looping DirectSound buffer
    // at flip 299. Running 160+ flips past flip 299 (~2.7 s of Main Menu)
    // crosses the circular buffer wrap: without periodic `WINMM!timeSetEvent`
    // callbacks refilling the buffer, lap 2 would repeat lap 1 verbatim.
    let mut host = Headless::new();
    host.queue(vec![(460, lego_legacy_core::host::Input::Close)]);
    let cli = Cli {
        guest_args: Some("-novideo".to_owned()),
        trace: Trace::Shims,
        ..Cli::default()
    };
    let ran = testkit::run_on(host, &RACERS, &cli, FIRST_LIGHT_BUDGET);
    ran.assert_exited(0)
        .assert_flips(461)
        .assert_logged("winmm timer 1:")
        .assert_logged("winmm timer 1 killed");

    let callback_fires = ran
        .log
        .lines()
        .filter(|l| l.contains("winmm timer 1 calls 0x004bd5e0"))
        .count();
    assert!(
        callback_fires >= 20,
        "expected >= 20 periodic 100 ms callback fires over ~2.7 s of Main Menu, got {callback_fires}"
    );

    // Buffer size is 0x2c000 bytes = 90,112 i16 stereo samples (~2.043 s at
    // 22,050 Hz). Compare a 0.5 s window in lap 1 against lap 2 at the exact
    // buffer-wrap offset.
    let pcm = pcm_from(&ran, 299);
    const SKIP_SAMPLES: usize = 4_096;
    const WRAP_SAMPLES: usize = 0x2c000 / 2;
    const WINDOW_SAMPLES: usize = 22_050;
    assert!(
        pcm.len() >= SKIP_SAMPLES + WRAP_SAMPLES + WINDOW_SAMPLES,
        "expected PCM past the 0x2c000-byte wrap ({} samples), got {}",
        SKIP_SAMPLES + WRAP_SAMPLES + WINDOW_SAMPLES,
        pcm.len()
    );
    let lap1 = &pcm[SKIP_SAMPLES..SKIP_SAMPLES + WINDOW_SAMPLES];
    let lap2 = &pcm[SKIP_SAMPLES + WRAP_SAMPLES..SKIP_SAMPLES + WRAP_SAMPLES + WINDOW_SAMPLES];
    let lap2_rms =
        (lap2.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / lap2.len() as f64).sqrt();
    let diffs = lap1.iter().zip(lap2).filter(|(a, b)| a != b).count();
    assert!(
        lap2_rms > 500.0,
        "expected non-silent streamed PCM after buffer wrap (RMS > 500), got {lap2_rms:.1}"
    );
    assert!(
        diffs > WINDOW_SAMPLES / 2,
        "expected fresh streamed PCM after circular wrap rather than stale loop repeat (diffs={diffs}/{WINDOW_SAMPLES})"
    );
}

#[test]
fn first_light_replays_to_the_same_machine_every_time() {
    let Some(first) = first_light() else {
        return;
    };
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        ..Cli::default()
    };
    let second = testkit::run_headless(&RACERS, &cli, FIRST_LIGHT_BUDGET);
    first.assert_same_machine(&second);
    assert_eq!(
        first.host.audio_blocks, second.host.audio_blocks,
        "mixed PCM audio is bit-exact across independent runs"
    );
}

#[test]
fn all_three_intro_movies_play_to_completion_and_land_on_main_menu() {
    if !scratch() {
        return;
    }
    // Space during unskippable lmicmp.avi (flip 100) must NOT skip it; all 3
    // movies then play through completion (237 + 337 + 1080 = 1654 frames)
    // before landing on Main Menu and exiting cleanly.
    let route = Route::parse(
        "#lego-legacy-route 1\nflip 100 key Space down\nflip 101 key Space up\nflip 1985 close\n",
    )
    .expect("parse full intro route");
    let mut host = Headless::new().keeping(&[80, 340, 700, 1100, 1500, 1980]);
    let mut machine = Machine::new(&RACERS, &mut host).expect("machine");
    machine.load_title(&RACERS).expect("load title");
    machine.route = Some(route);
    let mut log = RunLog::create("lego-racers-full-intro", Trace::Off).expect("log");
    let dir = log.dir().to_path_buf();
    let finish = machine.run(&mut host, &mut log, FULL_INTRO_BUDGET);
    drop(log);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        finish.outcome,
        lego_legacy_core::machine::Outcome::Exited { code: 0 }
    );
    assert_eq!(finish.flips, 1986);
    for (label, flip) in [
        ("lmicmp.avi", 80),
        ("hvscmp.avi", 340),
        ("introcmp.avi early", 700),
        ("introcmp.avi mid", 1100),
        ("introcmp.avi late", 1500),
        ("Main Menu", 1980),
    ] {
        let frame = host
            .frame(flip)
            .unwrap_or_else(|| panic!("{label} flip {flip} kept"));
        assert_eq!((frame.width, frame.height), (640, 480));
        assert!(!frame.is_uniform(), "{label} flip {flip} is non-uniform");
    }
    assert_ne!(
        host.frame(700).unwrap().pixels,
        host.frame(1500).unwrap().pixels,
        "scalable introcmp.avi advances across frames"
    );
    let introcmp_audio_nonzero = host
        .audio_blocks
        .iter()
        .zip(&host.audio_flips)
        .filter(|(_, &flip)| (574..1654).contains(&flip))
        .any(|(block, _)| block.iter().any(|&s| s != 0));
    assert!(
        introcmp_audio_nonzero,
        "introcmp.avi should mix non-silent PCM audio"
    );
}

fn assert_snapshot_round_trip(
    baseline: &testkit::Ran,
    route_name: &str,
    budget: u64,
    label: &str,
    snap_flip: u64,
) {
    let route = Route::load(&route_path(route_name)).expect("load route");
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-snap-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);

    let mut host_a = Headless::new();
    let mut machine_a = Machine::new(&RACERS, &mut host_a).expect("machine");
    machine_a.load_title(&RACERS).expect("load title");
    machine_a.route = Some(route.clone());
    machine_a.tree.set_profile(data_root.join("a"));
    let mut log_a =
        RunLog::create(&format!("lego-racers-snap-{label}-a"), Trace::Off).expect("log");
    let dir_a = log_a.dir().to_path_buf();
    while machine_a.flips() < snap_flip {
        assert!(
            machine_a
                .step(&mut host_a, &mut log_a, budget, true)
                .is_none(),
            "should reach {label} flip {snap_flip} without ending"
        );
    }
    let snap = machine_a.snapshot_v2().expect("capture LLSNAP v2");

    let mut host_b = Headless::new();
    let mut machine_b = Machine::new(&RACERS, &mut host_b).expect("machine");
    machine_b.load_title(&RACERS).expect("load title");
    machine_b.route = Some(route);
    machine_b.tree.set_profile(data_root.join("b"));
    machine_b.restore_v2(&snap).expect("cold restore LLSNAP v2");
    let mut log_b =
        RunLog::create(&format!("lego-racers-snap-{label}-b"), Trace::Off).expect("log");
    let dir_b = log_b.dir().to_path_buf();
    let finish_b = machine_b.run(&mut host_b, &mut log_b, budget);

    assert_eq!(finish_b.outcome, baseline.finish.outcome, "{label} outcome");
    assert_eq!(finish_b.cursor, baseline.finish.cursor, "{label} cursor");
    assert_eq!(finish_b.flips, baseline.finish.flips, "{label} flips");
    assert_eq!(
        finish_b.declared_millis, baseline.finish.declared_millis,
        "{label} declared_millis"
    );
    assert_eq!(machine_b.state_digest(), baseline.digest, "{label} digest");
    host_a.audio_blocks.extend(host_b.audio_blocks);
    assert_eq!(
        host_a.audio_blocks, baseline.host.audio_blocks,
        "{label} mixed PCM audio before + after LLSNAP v2 restore is bit-exact"
    );

    drop(log_a);
    drop(log_b);
    let _ = std::fs::remove_dir_all(dir_a);
    let _ = std::fs::remove_dir_all(dir_b);
    let _ = std::fs::remove_dir_all(&data_root);
}

#[test]
fn first_light_mid_movie_and_main_menu_snapshots_round_trip_bit_exact() {
    let Some(baseline) = first_light() else {
        return;
    };
    for (label, snap_flip) in [("mid-movie", 100u64), ("main-menu", 555u64)] {
        assert_snapshot_round_trip(
            baseline,
            "first-light.route",
            FIRST_LIGHT_BUDGET,
            label,
            snap_flip,
        );
    }
}

/// The committed `first-race.route`: unflagged cold boot -> SINGLE RACE ->
/// Imperial Grand Prix -> 3-2-1-GO -> Turbo Start -> full 3D Lap 1 with
/// power-ups and Power Slides -> LAP 2 -> Pause -> EXIT -> Main Menu -> close (#373).
fn first_race() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(route_path("first-race.route")),
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &RACERS,
        &cli,
        &[1572, 2182, 2619, 4851, 6067, 6368],
        FIRST_RACE_BUDGET,
    )
}

#[test]
fn build_racer_writes_save_and_reloads_cleanly_on_second_boot() {
    if !scratch() {
        return;
    }
    const SAVE_REL: &str = "profile/C/PROGRAM FILES/LEGO MEDIA/GAMES/LEGO RACERS/SAVE/0/LEGORAC1";
    const MISSING_LINE: &str =
        "file missing C:\\Program Files\\LEGO Media\\Games\\LEGO Racers\\Save\\0\\LEGORac1";

    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);

    let save_cli = Cli {
        route: Some(route_path("build-racer.route")),
        data_root: Some(data_root.clone()),
        ..Cli::default()
    };
    let build_run = testkit::run_on(
        Headless::new().keeping(&[1012, 1285, 1485]),
        &RACERS,
        &save_cli,
        BUILD_RACER_BUDGET,
    );
    build_run
        .assert_exited(0)
        .assert_flips(1501)
        .assert_logged("Save\\0\\LEGORac1, 1792 bytes");

    for flip in [1012u64, 1285, 1485] {
        assert!(!build_run.host.frame(flip).expect("frame kept").is_uniform());
    }

    let save_path = data_root.join(SAVE_REL);
    let save_bytes = std::fs::read(&save_path)
        .unwrap_or_else(|e| panic!("expected Save\\0\\LEGORac1 at {}: {e}", save_path.display()));
    assert_eq!(
        save_bytes.len(),
        1_792,
        "header (1152 B) + 1 driver (640 B)"
    );
    assert_eq!(&save_bytes[..5], b"LR\x01\x80\x04");
    assert_eq!(&save_bytes[1152..1160], b"A\x00D\x00A\x00\x00\x00");

    let data_root_b = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-save-b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root_b);
    let build_run_b = testkit::run_headless(
        &RACERS,
        &Cli {
            route: Some(route_path("build-racer.route")),
            data_root: Some(data_root_b.clone()),
            ..Cli::default()
        },
        BUILD_RACER_BUDGET,
    );
    build_run.assert_same_machine(&build_run_b);
    let save_bytes_b =
        std::fs::read(data_root_b.join(SAVE_REL)).expect("second run wrote Save\\0\\LEGORac1");
    assert_eq!(
        save_bytes, save_bytes_b,
        "Save\\0\\LEGORac1 bytes are bit-exact across independent runs"
    );

    let reload_route_path = data_root.join("reload.route");
    std::fs::write(
        &reload_route_path,
        "#lego-legacy-route 1\n\
         flip 240 key Space down\n\
         flip 241 key Space up\n\
         flip 245 move 320 240\n\
         flip 246 down left\n\
         flip 247 up left\n\
         flip 560 key Return down\n\
         flip 561 key Return up\n\
         flip 660 close\n",
    )
    .expect("write reload route");

    let reload_run = testkit::run_on(
        Headless::new().keeping(&[650]),
        &RACERS,
        &Cli {
            route: Some(reload_route_path.clone()),
            data_root: Some(data_root.clone()),
            ..Cli::default()
        },
        FIRST_LIGHT_BUDGET,
    );
    reload_run.assert_exited(0).assert_flips(661);
    assert!(
        !reload_run.log.contains(MISSING_LINE),
        "cold second boot should load Save\\0\\LEGORac1 from Tree::profile"
    );

    let empty_run = testkit::run_on(
        Headless::new().keeping(&[650]),
        &RACERS,
        &Cli {
            route: Some(reload_route_path),
            ..Cli::default()
        },
        FIRST_LIGHT_BUDGET,
    );
    empty_run.assert_exited(0).assert_logged(MISSING_LINE);

    let reloaded_frame = reload_run.host.frame(650).expect("reloaded build menu");
    let empty_frame = empty_run.host.frame(650).expect("empty build menu");
    assert!(!reloaded_frame.is_uniform());
    assert_ne!(
        reloaded_frame.pixels, empty_frame.pixels,
        "reloaded Build Menu with saved driver ADA differs visually from empty profile"
    );

    let _ = std::fs::remove_dir_all(&data_root);
    let _ = std::fs::remove_dir_all(&data_root_b);
}

#[test]
fn first_race_drives_imperial_grand_prix_with_3d_rendering_and_audio() {
    let Some(ran) = first_race() else {
        return;
    };
    ran.assert_exited(0).assert_flips(6371);

    let frames = [1572, 2182, 2619, 4851, 6067, 6368]
        .map(|f| ran.host.frame(f).unwrap_or_else(|| panic!("flip {f} kept")));
    for frame in &frames {
        assert_eq!((frame.width, frame.height), (640, 480));
        assert!(!frame.is_uniform());
    }
    for pair in frames.windows(2) {
        assert_ne!(pair[0].pixels, pair[1].pixels);
    }

    let race_samples = pcm_from(ran, 1745);
    assert!(
        race_samples.len() >= 100_000,
        "expected >= 100,000 stereo PCM samples during race, got {}",
        race_samples.len()
    );
    let rms = (race_samples
        .iter()
        .map(|&s| f64::from(s).powi(2))
        .sum::<f64>()
        / race_samples.len() as f64)
        .sqrt();
    assert!(
        rms > 500.0,
        "expected non-silent race audio (RMS > 500), got {rms:.1}"
    );
}

#[test]
fn first_race_replays_to_the_same_machine_every_time() {
    let Some(first) = first_race() else {
        return;
    };
    let cli = Cli {
        route: Some(route_path("first-race.route")),
        ..Cli::default()
    };
    let second = testkit::run_headless(&RACERS, &cli, FIRST_RACE_BUDGET);
    first.assert_same_machine(&second);
    assert_eq!(
        first.host.audio_blocks, second.host.audio_blocks,
        "mixed race PCM audio is bit-exact across independent runs"
    );
}

#[test]
fn first_race_mid_race_llsnap_v2_round_trips_bit_exact() {
    let Some(baseline) = first_race() else {
        return;
    };
    assert_snapshot_round_trip(
        baseline,
        "first-race.route",
        FIRST_RACE_BUDGET,
        "mid-race",
        2620,
    );
}

const BUILD_RACER_PARITY_FLIPS: &[u64] = &[1012, 1285, 1485];
const FIRST_RACE_PARITY_FLIPS: &[u64] = &[1572, 2182, 2619, 4851, 6067, 6368];

fn frames_and_audio_sha256(ran: &testkit::Ran, kept_flips: &[u64]) -> (String, String) {
    let mut frame_sha = hash::Sha256::new();
    for &flip in kept_flips {
        let frame = ran
            .host
            .frame(flip)
            .unwrap_or_else(|| panic!("expected kept parity frame at flip {flip}"));
        frame_sha.update(&flip.to_le_bytes());
        frame_sha.update(&frame.width.to_le_bytes());
        frame_sha.update(&frame.height.to_le_bytes());
        frame_sha.update(&frame.pixels);
    }
    let mut audio_sha = hash::Sha256::new();
    for block in &ran.host.audio_blocks {
        audio_sha.update(&(block.len() as u64).to_le_bytes());
        for &sample in block {
            audio_sha.update(&sample.to_le_bytes());
        }
    }
    (
        hash::hex(&frame_sha.finish()),
        hash::hex(&audio_sha.finish()),
    )
}

fn format_arch_summary(route_name: &str, ran: &testkit::Ran, kept_flips: &[u64]) -> String {
    ran.assert_exited(0);
    let (frames_hash, audio_hash) = frames_and_audio_sha256(ran, kept_flips);
    format!(
        "ARCH_SUMMARY|{}|{:?}|{}|{}|{}|{}|{}|{}",
        route_name,
        ran.finish.outcome,
        ran.finish.cursor,
        ran.finish.flips,
        ran.finish.declared_millis,
        ran.digest,
        frames_hash,
        audio_hash,
    )
}

fn run_route_summary(route_name: &str, budget: u64, kept_flips: &[u64]) -> String {
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-arch-{route_name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);
    let cli = Cli {
        route: Some(route_path(route_name)),
        data_root: Some(data_root.clone()),
        ..Cli::default()
    };
    let ran = testkit::run_on(Headless::new().keeping(kept_flips), &RACERS, &cli, budget);
    let _ = std::fs::remove_dir_all(&data_root);
    format_arch_summary(route_name, &ran, kept_flips)
}

/// ADR-0009 Layer 2 parity: replaying `build-racer.route` and `first-race.route`
/// under `LEGO_LEGACY_ARCH=rust` produces the exact same outcome, cursor, flips,
/// declared milliseconds, `Machine::state_digest()`, mid-run 3D frames, and
/// mixed PCM audio as the native SIMD/assembly tier (#375).
#[test]
fn build_racer_and_first_race_match_pure_rust_tier_bit_exact() {
    if let Ok(route_name) = std::env::var("LEGO_RACERS_ARCH_PARITY_CHILD") {
        assert!(
            lego_legacy_core::arch::use_rust(),
            "parity child must execute on LEGO_LEGACY_ARCH=rust"
        );
        if !scratch() {
            return;
        }
        let (budget, kept_flips) = match route_name.as_str() {
            "build-racer.route" => (BUILD_RACER_BUDGET, BUILD_RACER_PARITY_FLIPS),
            "first-race.route" => (FIRST_RACE_BUDGET, FIRST_RACE_PARITY_FLIPS),
            other => panic!("unexpected parity route {other}"),
        };
        println!("\n{}", run_route_summary(&route_name, budget, kept_flips));
        return;
    }

    if !scratch() {
        return;
    }
    // Avoid spawning nested rust children when the whole suite is already
    // invoked with `LEGO_LEGACY_ARCH=rust`.
    if lego_legacy_core::arch::use_rust() {
        return;
    }

    let exe = std::env::current_exe().expect("test executable path");
    let spawn_rust_child = |route_name: &str| {
        std::process::Command::new(&exe)
            .args([
                "--exact",
                "build_racer_and_first_race_match_pure_rust_tier_bit_exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("LEGO_LEGACY_ARCH", "rust")
            .env("LEGO_RACERS_ARCH_PARITY_CHILD", route_name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn pure-Rust parity child")
    };
    let build_child = spawn_rust_child("build-racer.route");
    let race_child = spawn_rust_child("first-race.route");

    let race_baseline = first_race().expect("disc already verified by scratch()");
    let race_native =
        format_arch_summary("first-race.route", race_baseline, FIRST_RACE_PARITY_FLIPS);
    let build_native = run_route_summary(
        "build-racer.route",
        BUILD_RACER_BUDGET,
        BUILD_RACER_PARITY_FLIPS,
    );

    for (route_name, native_summary, child) in [
        ("build-racer.route", build_native, build_child),
        ("first-race.route", race_native, race_child),
    ] {
        let output = child.wait_with_output().expect("wait for parity child");
        assert!(
            output.status.success(),
            "pure-Rust parity child failed for {route_name}:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let rust_summary = stdout
            .lines()
            .find_map(|line| line.find("ARCH_SUMMARY|").map(|idx| &line[idx..]))
            .unwrap_or_else(|| {
                panic!("missing ARCH_SUMMARY line for {route_name} in stdout:\n{stdout}")
            });
        assert_eq!(
            native_summary, rust_summary,
            "native SIMD/asm vs LEGO_LEGACY_ARCH=rust parity mismatch on {route_name}"
        );
    }
}

/// Sustained Imperial Grand Prix racing must keep guest heap extent and live
/// bytes bounded rather than leaking ~45 KB/s per `CreateSoundBuffer`/`Release`
/// cycle until the 512 MiB ceiling is exhausted (#385).
#[test]
fn imperial_grand_prix_keeps_live_heap_bounded_across_sustained_racing() {
    if !scratch() {
        return;
    }
    let full_route =
        std::fs::read_to_string(route_path("first-race.route")).expect("read first-race.route");
    let run_until = |close_flip: u64, label: &str| {
        let mut lines: Vec<String> = full_route
            .lines()
            .take_while(|l| !l.starts_with("flip 1745 "))
            .map(str::to_owned)
            .collect();
        lines.push(format!("flip {close_flip} close"));
        let route = Route::parse(&lines.join("\n")).expect("parse sustained race route");

        let mut host = Headless::new();
        let mut machine = Machine::new(&RACERS, &mut host).expect("machine");
        machine.load_title(&RACERS).expect("load title");
        machine.route = Some(route);
        let mut log = RunLog::create(label, Trace::Off).expect("log");
        let dir = log.dir().to_path_buf();
        let finish = machine.run(&mut host, &mut log, FIRST_RACE_BUDGET);
        let extent = machine.state.heap.extent();
        let live = machine.state.heap.live_bytes();
        let buffers = machine.state.sound.buffer_count();
        drop(log);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            finish.outcome,
            lego_legacy_core::machine::Outcome::Exited { code: 0 }
        );
        (extent, live, buffers, host)
    };

    let (extent_2500, live_2500, buffers_2500, _) = run_until(2_500, "lego-racers-heap-2500");
    let (extent_5500, live_5500, buffers_5500, host_5500) =
        run_until(5_500, "lego-racers-heap-5500");

    // Non-vacuous liveness proof: Imperial Grand Prix `.tun` music + racing SFX
    // actively streamed non-silent PCM across flips 2500..5500.
    let race_pcm: Vec<i16> = host_5500
        .audio_blocks
        .iter()
        .zip(&host_5500.audio_flips)
        .filter(|(_, &flip)| (2_500..5_500).contains(&flip))
        .flat_map(|(block, _)| block.iter().copied())
        .collect();
    assert!(
        race_pcm.len() >= 100_000,
        "expected >= 100,000 stereo PCM samples across racing flips 2500..5500, got {}",
        race_pcm.len()
    );
    let rms = (race_pcm.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / race_pcm.len() as f64)
        .sqrt();
    assert!(
        rms > 500.0,
        "expected active racing audio (RMS > 500) across flips 2500..5500, got {rms:.1}"
    );

    let live_growth = live_5500.saturating_sub(live_2500);
    let extent_growth = extent_5500.saturating_sub(extent_2500);
    assert!(
        live_growth <= 256 * 1024,
        "live heap grew by {live_growth} B over 3,000 racing flips (2500->{live_2500}, 5500->{live_5500})"
    );
    assert!(
        extent_growth <= 256 * 1024,
        "heap extent grew by {extent_growth} B over 3,000 racing flips (2500->{extent_2500}, 5500->{extent_5500})"
    );
    assert!(
        buffers_2500 <= 32 && buffers_5500 <= 32,
        "live DirectSound buffer count must stay bounded (2500->{buffers_2500}, 5500->{buffers_5500})"
    );
}
