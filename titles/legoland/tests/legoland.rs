//! LEGOLAND, run headless from the player's rip: the launcher finding the
//! Masterpiece pressing, its declared InstallShield 3 `main.z` tree and Joliet
//! `.res` access, its unflagged first-light route through `lmi.avi` ->
//! `Intro.avi` (`0x0011` IMA ADPCM) -> `Player Details` -> `"ADA"` ->
//! `Title Screen` -> clean exit, media codecs (`IV32`, `0x0011` IMA ADPCM,
//! `0x0002` MS ADPCM `Speech/*.wav`, `AD_*.avi` overlays), determinism, and
//! `LLSNAP v2` cold snapshot restore parity (#393, #394, #395).
//!
//! Disc tests need the player's own rip under `game-data/`. Without it the
//! disc tests print a skip line and pass, which is a gate that did not run
//! rather than one that held: `LEGO_LEGACY_REQUIRE_DISC=1` turns the skip into
//! a failure.

use lego_legacy_core::cli::{Cli, Trace};
use lego_legacy_core::dls::Bank;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::route::Route;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::snap::{Reader, Snap, Visitor, Writer};
use lego_legacy_core::synth::Synth;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{avi, disc, hash, iv32, iv50, pe, sound, testkit, Title};
use legoland::LEGOLAND;
use std::path::PathBuf;

const ROOT: &str = LEGOLAND.install_dir;
const UNFLAGGED_BOOT_BUDGET: u64 = 50_000_000;
const FIRST_LIGHT_BUDGET: u64 = 3_000_000_000;

fn route_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("routes")
        .join(name)
}

fn pcm_rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|&s| {
            let x = f64::from(s);
            x * x
        })
        .sum();
    (sum_sq / samples.len() as f64).sqrt()
}

/// Unflagged cold boot: loads `Legoland.exe`, reads PE `RT_VERSION`, mounts
/// `.res` volumes over Joliet, initializes DirectDraw/DirectInput/DirectSound,
/// and starts decoding `D:\lmi.avi` (#394).
fn boot() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        trace: Trace::Off,
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &LEGOLAND,
        &cli,
        &[0, 1],
        UNFLAGGED_BOOT_BUDGET,
    )
}

/// Unflagged first-light route: plays `lmi.avi` (skipped via `LeftControl+Q`),
/// plays `Intro.avi` (`IV50` + `0x0011` IMA ADPCM, skipped via `Space`),
/// boots to `Player Details`, creates profile `"ADA"`, clicks `Accept`,
/// renders `Title Screen`, and exits cleanly via `Exit Game -> Yes` (#394, #395).
fn first_light() -> Option<&'static testkit::Ran> {
    static RAN: std::sync::OnceLock<Option<testkit::Ran>> = std::sync::OnceLock::new();
    let cli = Cli {
        route: Some(route_path("first-light.route")),
        ..Cli::default()
    };
    testkit::replay(
        &RAN,
        scratch,
        &LEGOLAND,
        &cli,
        &[25, 45, 72, 110, 134],
        FIRST_LIGHT_BUDGET,
    )
}

/// Point the run at the checkout's rips and keep run directories out of it.
fn scratch() -> bool {
    runs();
    testkit::rips(&LEGOLAND.image)
}

/// Keep run directories out of the checkout.
fn runs() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

#[test]
fn the_launcher_finds_the_rip_ready_to_start() {
    // It lists the title seventh, after the six earlier crates on the roster.
    assert_eq!(
        roster::ROSTER.iter().position(|e| e.key == LEGOLAND.key),
        Some(6)
    );
    if !scratch() {
        return;
    }
    assert_eq!(
        roster::look_for(&roster::LEGOLAND),
        Found::Runnable {
            pressing: "masterpiece-legoland".to_owned()
        }
    );
}

#[test]
fn a_missing_rip_names_the_path_it_looked_at() {
    runs();
    const ABSENT: Title = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..LEGOLAND.image
        },
        ..LEGOLAND
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
fn a_pressing_that_is_not_the_declared_one_is_refused_by_its_own_digest() {
    if !scratch() {
        return;
    }
    const PATCHED: Title = Title {
        image: EntryImage {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..LEGOLAND.image
        },
        ..LEGOLAND
    };
    let err = testkit::try_run_headless(&PATCHED, &Cli::default(), 1_000)
        .expect_err("a title is the bytes it is made of");

    assert!(
        err.contains("c50865b6"),
        "the refusal names what it found: {err}"
    );
}

#[test]
fn its_disc_carries_the_label_joliet_volumes_and_absent_indeo_library() {
    assert_eq!(LEGOLAND.declared.absent_libraries.len(), 1);
    assert_eq!(LEGOLAND.declared.absent_libraries[0].0, "Ir50_32.dll");

    if !scratch() {
        return;
    }
    let iso = disc::open_pressing(LEGOLAND.image.disc_dir, LEGOLAND.image.pressing)
        .expect("the pressing opens");
    assert_eq!(iso.label, "LEGOLAND");

    let mut tree = testkit::tree(&LEGOLAND);
    for (name, len) in [
        ("Legoland.res", 16_424_086),
        ("Graphics1.res", 19_962_774),
        ("Graphics2.res", 120_357_911),
    ] {
        assert!(
            !tree.holds(&format!("{ROOT}\\{name}")),
            "{name} stays on D:\\ rather than installed locally"
        );
        assert_eq!(
            tree.lies(&format!("D:\\{name}")).map(|(_, l)| l),
            Some(len),
            "{name} is readable via Joliet at its declared size"
        );
    }
    assert!(
        tree.wall().is_none(),
        "Joliet `.res` lookups must not raise a primary-tree wall"
    );
}

#[test]
fn the_declared_tree_unpacks_main_z_and_verifies_legoland_exe() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&LEGOLAND);
    assert_eq!(tree.declared_files().len(), 331);

    let exe = tree
        .read(&format!("{ROOT}\\Legoland.exe"))
        .expect("Legoland.exe is unpacked from main.z");
    assert_eq!(exe.len(), 802_857);
    assert_eq!(hash::sha256_hex(&exe), LEGOLAND.image.sha256);

    let exe_pe = pe::inspect(&exe).expect("Legoland.exe is a 32-bit PE");
    assert_eq!(pe::wrapper_scheme(&exe_pe.sections), None);
    assert_eq!(exe_pe.preferred_base, 0x0040_0000);
    assert_eq!(exe_pe.entry, 0x000a_08cd);
    assert_eq!(exe_pe.size, 0x0043_6000);
    assert!(
        pe::version_resource(&exe).is_some(),
        "Legoland.exe carries an RT_VERSION resource"
    );

    for name in [
        "Lego.TTF",
        "strings\\stab.str",
        "IMusic\\Band19.bnd",
        "zbuffers\\balloonz.bnv",
    ] {
        assert!(
            tree.readable(&format!("{ROOT}\\{name}")),
            "{name} is unpacked from main.z"
        );
    }
}

#[test]
fn unflagged_boot_loads_fonts_resources_and_plays_lmi_avi() {
    let Some(ran) = boot() else {
        return;
    };
    ran.assert_logged("image Legoland.exe")
        .assert_logged("base 0x00400000")
        .assert_logged("entry 0x004a08cd")
        .assert_logged("imports 201")
        .assert_logged("installed 331 files")
        .assert_logged("gdi AddFontResourceA: Lego.ttf -> family \"Lego\"")
        .assert_logged("movie D:\\lmi.avi opened through AVIFile: 2 streams");
    assert!(
        ran.finish.flips >= 2,
        "unflagged boot should decode and present lmi.avi frames"
    );
}

#[test]
fn first_light_plays_intros_with_audio_creates_ada_reaches_title_and_exits_cleanly() {
    let Some(ran) = first_light() else {
        return;
    };
    ran.assert_exited(0)
        .assert_flips(145)
        .assert_logged("gdi AddFontResourceA: Lego.ttf -> family \"Lego\"")
        .assert_logged("movie D:\\lmi.avi opened through AVIFile: 2 streams")
        .assert_logged("movie D:\\Intro.avi opened through AVIFile: 2 streams")
        .assert_logged("IMA ADPCM at 22050 Hz in 2 channels to 16-bit PCM")
        .assert_logged("MS ADPCM at 22050 Hz in 1 channels to 16-bit PCM")
        .assert_logged("movie AD_Blink.avi opened through AVIFile: 1 streams");

    if roster::find_bank(roster::LEGOLAND.bank).is_ok() {
        ran.assert_logged("dmusic: performance initialized")
            .assert_logged("dmusic: created software synthesizer port")
            .assert_logged("dmusic: ScanDirectory d2ac288a-b39b-11d1-8704-00600893b1bd *.sty")
            .assert_logged("dmusic: PlaySegment \"themeintro\"");
        // Flips 42..76 sit on Player Details before any speech WAV triggers (flip 77):
        // non-silent RMS here proves DirectMusic synthesized Segtheme1.sgt into the mix.
        let details_rms = ran.loudness(42, 76);
        let details_peak = details_rms.iter().copied().fold(0.0f64, f64::max);
        assert!(
            details_peak > 50.0,
            "DirectMusic should play audible themeintro music on Player Details (peak RMS {details_peak:.1}, blocks={})",
            details_rms.len()
        );
        // Title Screen flips 102..130 also carry continuous DirectMusic playback.
        let title_rms = ran.loudness(102, 130);
        let title_peak = title_rms.iter().copied().fold(0.0f64, f64::max);
        assert!(
            title_peak > 50.0,
            "DirectMusic should play audible themeintro music on Title Screen (peak RMS {title_peak:.1}, blocks={})",
            title_rms.len()
        );
    } else {
        ran.assert_logged("com: d2ac2890-b39b-11d1-8704-00600893b1bd is not registered");
    }

    let intro = ran.host.frame(25).expect("Intro.avi flip 25 kept");
    let details = ran.host.frame(45).expect("Player Details flip 45 kept");
    let typed = ran.host.frame(72).expect("ADA typed flip 72 kept");
    let title = ran.host.frame(110).expect("Title Screen flip 110 kept");
    let exit_dlg = ran.host.frame(134).expect("Exit dialog flip 134 kept");
    for (name, frame) in [
        ("Intro.avi", intro),
        ("Player Details", details),
        ("ADA typed", typed),
        ("Title Screen", title),
        ("Exit dialog", exit_dlg),
    ] {
        assert_eq!((frame.width, frame.height), (640, 480), "{name} resolution");
        assert!(!frame.is_uniform(), "{name} renders non-uniform pixels");
    }
    assert_ne!(
        intro.pixels, details.pixels,
        "Space skips Intro.avi to Player Details"
    );
    assert_ne!(
        details.pixels, typed.pixels,
        "typing ADA visibly updates Player Details"
    );
    assert_ne!(
        typed.pixels, title.pixels,
        "Accept transitions from Player Details to Title Screen"
    );
    assert_ne!(
        title.pixels, exit_dlg.pixels,
        "Exit Game opens the Exit confirmation dialog"
    );

    // Mixed PCM audio during Intro.avi (0x0011 IMA ADPCM) and UI/speech must be non-silent.
    assert!(
        ran.host.audio_blocks.len() >= 40,
        "expected >= 40 mixed audio blocks across unflagged first-light, got {}",
        ran.host.audio_blocks.len()
    );
    let intro_window: Vec<i16> = ran.host.audio_blocks[10..35]
        .iter()
        .flatten()
        .copied()
        .collect();
    let intro_rms = pcm_rms(&intro_window);
    assert!(
        intro_rms > 50.0,
        "Intro.avi 0x0011 IMA ADPCM audio should be non-silent (RMS={intro_rms:.2})"
    );
    let post_intro_window: Vec<i16> = ran.host.audio_blocks[35..]
        .iter()
        .flatten()
        .copied()
        .collect();
    let post_rms = pcm_rms(&post_intro_window);
    assert!(
        post_rms > 5.0,
        "UI SFX / MS ADPCM audio after Intro.avi should be non-silent (RMS={post_rms:.2})"
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
    let second = testkit::run_headless(&LEGOLAND, &cli, FIRST_LIGHT_BUDGET);
    first.assert_same_machine(&second);
    assert_eq!(
        first.host.audio_blocks, second.host.audio_blocks,
        "DirectMusic + SFX mixed PCM audio is bit-exact across replays"
    );
}

#[test]
fn disc_media_decodes_ima_adpcm_speech_ms_adpcm_advisors_and_iv32_california() {
    if !scratch() {
        return;
    }
    let mut tree = testkit::tree(&LEGOLAND);

    // 1. California.avi: IV32 Indeo 3.2 (320x200, 587 frames) + mid-stream LLSNAP v2 parity.
    let cal_bytes = tree
        .read("D:\\California.avi")
        .expect("California.avi on disc");
    let cal_avi = avi::parse(&cal_bytes).expect("parse California.avi");
    let cal_vid = cal_avi
        .streams
        .iter()
        .find(|s| s.is_video())
        .expect("California.avi video stream");
    assert_eq!(&cal_vid.compression(), b"IV32");
    assert_eq!(cal_vid.picture_size(), (320, 200));
    assert_eq!(cal_vid.chunks.len(), 587);

    let mut dec = iv32::Decoder::new();
    let mut pic = iv50::Picture::default();
    let mut snap_bytes = None;
    let mut snap_pic = None;
    let mut frames_after_snap = Vec::new();
    for (idx, &(start, len)) in cal_vid.chunks.iter().enumerate() {
        dec.decode(&cal_bytes[start..start + len], &mut pic)
            .unwrap_or_else(|e| panic!("California.avi IV32 frame {idx} failed: {e}"));
        assert_eq!((pic.width, pic.height), (320, 200));
        if idx == 150 {
            let mut w = Writer::new();
            dec.snap(&mut w);
            snap_bytes = Some(w.out);
            snap_pic = Some(pic.clone());
        } else if idx > 150 && idx <= 170 {
            frames_after_snap.push(pic.clone());
        }
    }
    // Confirm non-uniform decoded frame and exact mid-movie IV32 snapshot continuity.
    assert!(pic.y.iter().any(|&p| p != pic.y[0]));
    let mut restored_dec = iv32::Decoder::new();
    let mut r = Reader::new(snap_bytes.as_ref().expect("snapshot captured"));
    restored_dec.snap(&mut r);
    assert!(!r.failed());
    let mut restored_pic = snap_pic.expect("snap pic");
    for (offset, &(start, len)) in cal_vid.chunks[151..=170].iter().enumerate() {
        restored_dec
            .decode(&cal_bytes[start..start + len], &mut restored_pic)
            .expect("restored IV32 decode");
        assert_eq!(
            restored_pic,
            frames_after_snap[offset],
            "IV32 frame {} after LLSNAP v2 restore matches baseline bit-exact",
            151 + offset
        );
    }

    // 2. All 6 IMA ADPCM (0x0011) FMVs decode to non-silent PCM.
    for name in [
        "Intro.avi",
        "Billund.avi",
        "egypt.avi",
        "INCA.AVI",
        "Medieval.avi",
        "SPIDER.AVI",
    ] {
        let bytes = tree
            .read(&format!("D:\\{name}"))
            .unwrap_or_else(|| panic!("{name} on disc"));
        let parsed = avi::parse(&bytes).unwrap_or_else(|e| panic!("parse {name}: {e}"));
        let audio = parsed
            .streams
            .iter()
            .find(|s| s.is_audio())
            .unwrap_or_else(|| panic!("{name} audio stream"));
        let fmt = sound::Format::parse(&audio.format).expect("parse WAVEFORMATEX");
        assert_eq!(fmt.tag, 0x0011, "{name} should be IMA ADPCM");
        let pcm = parsed
            .decode_audio(&bytes)
            .unwrap_or_else(|e| panic!("decode {name} audio: {e}"))
            .expect("audio decoded");
        let rms = pcm_rms(&pcm);
        assert!(
            rms > 50.0,
            "{name} IMA ADPCM decoded audio should be non-silent (RMS={rms:.2})"
        );
    }

    // 3. Speech/*.wav (0x0002 MS ADPCM mono 22,050 Hz) decodes to non-silent PCM.
    let wav = tree
        .read("D:\\Speech\\BRIEFING TUTORIAL TWO_01.wav")
        .expect("Speech WAV on disc");
    let (fmt_bytes, data_bytes) = parse_riff_wav(&wav);
    let speech_fmt = sound::Format::parse(fmt_bytes).expect("parse speech WAVEFORMATEX");
    assert_eq!(speech_fmt.tag, 2);
    assert_eq!((speech_fmt.channels, speech_fmt.rate), (1, 22_050));
    let speech_pcm = speech_fmt.decode(data_bytes).expect("decode speech WAV");
    let speech_rms = pcm_rms(&speech_pcm);
    assert!(
        speech_rms > 100.0,
        "Speech MS ADPCM WAV should be non-silent (RMS={speech_rms:.2})"
    );

    // 4. AD_Blink.avi (112x96 IV50 advisor overlay from main.z) decodes distinct frames.
    let adv_bytes = tree
        .read(&format!("{ROOT}\\AD_Blink.avi"))
        .expect("AD_Blink.avi unpacked from main.z");
    let adv_avi = avi::parse(&adv_bytes).expect("parse AD_Blink.avi");
    let adv_vid = adv_avi
        .streams
        .iter()
        .find(|s| s.is_video())
        .expect("AD_Blink.avi video");
    assert_eq!(adv_vid.picture_size(), (112, 96));
    let mut adv_dec = iv50::Decoder::new();
    let mut adv_pic = iv50::Picture::default();
    let mut first_y = None;
    let mut changed = false;
    for &(start, len) in &adv_vid.chunks {
        if len > 0 {
            adv_dec
                .decode(&adv_bytes[start..start + len], &mut adv_pic)
                .expect("decode AD_Blink.avi frame");
            if let Some(ref f) = first_y {
                if &adv_pic.y != f {
                    changed = true;
                }
            } else {
                first_y = Some(adv_pic.y.clone());
            }
        }
    }
    assert!(changed, "AD_Blink.avi should produce animating frames");
}

fn parse_riff_wav(wav: &[u8]) -> (&[u8], &[u8]) {
    assert_eq!(&wav[0..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    let mut pos = 12;
    let mut fmt = None;
    let mut data = None;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let len =
            u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let body = &wav[pos + 8..(pos + 8 + len).min(wav.len())];
        if id == b"fmt " {
            fmt = Some(body);
        } else if id == b"data" {
            data = Some(body);
        }
        pos += 8 + len + (len & 1);
    }
    (fmt.expect("WAV fmt chunk"), data.expect("WAV data chunk"))
}

#[test]
fn first_light_without_gm16_bank_disables_directmusic_cleanly() {
    if !scratch() {
        return;
    }
    let route = Route::load(&route_path("first-light.route")).expect("load first-light.route");
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("legoland-nobank-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);

    let mut host = Headless::new();
    let mut machine = Machine::new(&LEGOLAND, &mut host).expect("machine");
    machine.load_title(&LEGOLAND).expect("load title");
    machine.route = Some(route);
    machine.tree.set_profile(data_root.clone());
    let mut log = RunLog::create("legoland-nobank", Trace::Off).expect("log");
    let dir = log.dir().to_path_buf();
    let finish = machine.run(&mut host, &mut log, FIRST_LIGHT_BUDGET);
    drop(log);
    let text = std::fs::read_to_string(dir.join("log")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&data_root);

    assert_eq!(
        finish.outcome,
        lego_legacy_core::machine::Outcome::Exited { code: 0 }
    );
    assert_eq!(finish.flips, 145);
    assert!(
        text.contains("com: d2ac2890-b39b-11d1-8704-00600893b1bd is not registered"),
        "without GM16.DLS DirectMusic must refuse cleanly via REGDB_E_CLASSNOTREG"
    );
}

fn open_gm16_if_present(machine: &mut Machine) {
    if let Ok((_, bytes)) = roster::find_bank(roster::LEGOLAND.bank) {
        let bank = Bank::parse(&bytes).expect("parse GM16.DLS");
        machine.state.mixer.open_synth(Synth::new(bank));
    }
}

fn assert_snapshot_round_trip(baseline: &testkit::Ran, label: &str, snap_flip: u64) {
    let route = Route::load(&route_path("first-light.route")).expect("load first-light.route");
    let data_root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("legoland-snap-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_root);

    let mut host_a = Headless::new();
    let mut machine_a = Machine::new(&LEGOLAND, &mut host_a).expect("machine");
    machine_a.load_title(&LEGOLAND).expect("load title");
    open_gm16_if_present(&mut machine_a);
    machine_a.route = Some(route.clone());
    machine_a.tree.set_profile(data_root.join("a"));
    let mut log_a = RunLog::create(&format!("legoland-snap-{label}-a"), Trace::Off).expect("log");
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

    // Copy any profile files written before the snapshot into B's isolated
    // profile directory (cold restore semantics: guest disk + snapshot state).
    let prof_a = data_root.join("a");
    let prof_b = data_root.join("b");
    if prof_a.exists() {
        copy_dir_all(&prof_a, &prof_b).expect("copy profile for cold restore");
    }

    let mut host_b = Headless::new();
    let mut machine_b = Machine::new(&LEGOLAND, &mut host_b).expect("machine");
    machine_b.load_title(&LEGOLAND).expect("load title");
    open_gm16_if_present(&mut machine_b);
    machine_b.route = Some(route);
    machine_b.tree.set_profile(prof_b);
    machine_b.restore_v2(&snap).expect("cold restore LLSNAP v2");
    let mut log_b = RunLog::create(&format!("legoland-snap-{label}-b"), Trace::Off).expect("log");
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
    let _ = std::fs::remove_dir_all(&data_root);
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

#[test]
fn first_light_mid_movie_player_details_and_title_screen_snapshots_round_trip_bit_exact() {
    let Some(baseline) = first_light() else {
        return;
    };
    for (label, snap_flip) in [
        ("intro-avi", 20u64),
        ("player-details", 55u64),
        ("title-screen", 110u64),
    ] {
        assert_snapshot_round_trip(baseline, label, snap_flip);
    }
}
