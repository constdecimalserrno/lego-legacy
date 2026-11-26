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
use lego_legacy_core::roster::{self, Entry, Found};
use lego_legacy_core::route::Route;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{hash, pe, testkit, Title};
use lego_racers::LEGO_RACERS as RACERS;
use std::path::PathBuf;

const ROOT: &str = RACERS.install_dir;
const BOOT_BUDGET: u64 = 60_000;
const FIRST_LIGHT_BUDGET: u64 = 200_000_000;
const FULL_INTRO_BUDGET: u64 = 250_000_000;

fn route_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("routes")
        .join(name)
}

/// The default unflagged boot path, run once for 60k instructions to verify
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
    image: testkit::LEGO_RACERS_WRAPPED,
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
    // It lists the title sixth, after the five earlier crates on the roster.
    assert_eq!(
        roster::ROSTER.iter().position(|e| e.key == RACERS.key),
        Some(5)
    );
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&roster::LEGO_RACERS),
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
    let wrapped_entry = Entry {
        image: testkit::LEGO_RACERS_WRAPPED,
        ..roster::LEGO_RACERS
    };
    assert_eq!(
        roster::look_for(&wrapped_entry),
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

#[test]
fn first_light_mid_movie_and_main_menu_snapshots_round_trip_bit_exact() {
    let Some(baseline) = first_light() else {
        return;
    };
    let route = Route::load(&route_path("first-light.route")).expect("load first-light.route");
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("racers-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);

    for (label, snap_flip) in [("mid-movie", 100u64), ("main-menu", 555u64)] {
        let profile_a = data_root.join(format!("{label}-a"));
        let profile_b = data_root.join(format!("{label}-b"));

        let mut host_a = Headless::new();
        let mut machine_a = Machine::new(&RACERS, &mut host_a).expect("machine");
        machine_a.load_title(&RACERS).expect("load title");
        machine_a.route = Some(route.clone());
        machine_a.tree.set_profile(profile_a);
        let mut log_a =
            RunLog::create(&format!("lego-racers-snap-{label}-a"), Trace::Off).expect("log");
        let dir_a = log_a.dir().to_path_buf();
        while machine_a.flips() < snap_flip {
            assert!(
                machine_a
                    .step(&mut host_a, &mut log_a, FIRST_LIGHT_BUDGET, true)
                    .is_none(),
                "should reach {label} flip {snap_flip} without ending"
            );
        }
        let snap = machine_a.snapshot_v2().expect("capture LLSNAP v2");

        let mut host_b = Headless::new();
        let mut machine_b = Machine::new(&RACERS, &mut host_b).expect("machine");
        machine_b.load_title(&RACERS).expect("load title");
        machine_b.route = Some(route.clone());
        machine_b.tree.set_profile(profile_b);
        machine_b.restore_v2(&snap).expect("cold restore LLSNAP v2");
        let mut log_b =
            RunLog::create(&format!("lego-racers-snap-{label}-b"), Trace::Off).expect("log");
        let dir_b = log_b.dir().to_path_buf();
        let finish_b = machine_b.run(&mut host_b, &mut log_b, FIRST_LIGHT_BUDGET);

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
    }
    let _ = std::fs::remove_dir_all(data_root);
}
