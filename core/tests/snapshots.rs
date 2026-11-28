//! Snapshots: a cache, never evidence, that the stub can be resumed from.

use lego_legacy_core::cli::Cli;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::{stub, testkit, Title};

const STUB: Title = Title::STUB;

fn scratch() {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
}

/// Build a stub machine and run it to a cursor.
fn machine_at(cursor: u64) -> (Machine, Headless) {
    scratch();
    let mut host = Headless::new();
    let mut machine = Machine::new(&STUB, &mut host).expect("a machine");
    machine.load_stub().expect("the stub loads");
    let mut log =
        lego_legacy_core::runlog::RunLog::create("stub", lego_legacy_core::cli::Trace::Off)
            .expect("a run directory");
    while machine.clock.cursor() < cursor {
        if machine.tick(&mut host, &mut log).is_some() {
            break;
        }
    }
    (machine, host)
}

#[test]
fn resuming_from_the_middle_ends_where_running_straight_through_does() {
    // Run to a cursor, snapshot, then resume into a fresh machine and finish
    // there. The two must end identically: that is the whole promise a
    // snapshot makes.
    let (mut early, _host) = machine_at(6);
    let bytes = early.snapshot().expect("the machine snapshots");

    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let straight = testkit::run_headless(&STUB, &cli, 1_000);

    scratch();
    let mut host = Headless::new();
    let mut resumed = Machine::new(&STUB, &mut host).expect("a machine");
    resumed.load_stub().expect("the stub loads");
    resumed.resume(&bytes).expect("the snapshot is ours");
    let mut log =
        lego_legacy_core::runlog::RunLog::create("stub", lego_legacy_core::cli::Trace::Off)
            .expect("a run directory");
    let finish = resumed.run(&mut host, &mut log, 1_000);

    assert_eq!(
        finish.cursor, straight.finish.cursor,
        "the same instructions retired"
    );
    assert_eq!(
        finish.outcome, straight.finish.outcome,
        "and it ended the same way"
    );
    assert_eq!(
        resumed.state_digest(),
        straight.digest,
        "and the machine looks the same"
    );
}

/// The digest sees every page the guest can write: the main thread's whole
/// stack, not only what is above where it stands, and the thread block and
/// the process block above that (#98).
#[test]
fn the_digest_sees_the_whole_stack_and_the_blocks_above_it() {
    use lego_legacy_core::guest::GuestMem;
    use lego_legacy_core::machine::{PEB, STACK_BYTES, STACK_TOP, TIB};
    let (mut machine, _host) = machine_at(0);
    // Standing where a title's main thread starts. The stub's own surface is
    // at the bottom of the stack, so the stack is written a page above that,
    // and just below where it stands.
    machine.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
    for at in [
        STACK_TOP - STACK_BYTES + 0x1000,
        STACK_TOP - 17,
        TIB,
        PEB + 0xfff,
    ] {
        let before = machine.state_digest();
        machine.arena.write(at, &[1]).expect("it is mapped");
        assert_ne!(machine.state_digest(), before, "{at:#010x} is hashed");
    }
}

#[test]
fn a_snapshot_from_another_title_is_refused_by_name() {
    let (mut machine, _host) = machine_at(4);
    let bytes = machine.snapshot().expect("the machine snapshots");

    // The same bytes, offered to a machine running something else.
    const OTHER: Title = Title {
        key: "not-the-stub",
        ..Title::STUB
    };
    scratch();
    let mut host = Headless::new();
    let mut other = Machine::new(&OTHER, &mut host).expect("a machine");
    other.load_stub().expect("the stub loads");
    let err = other
        .resume(&bytes)
        .expect_err("this snapshot is not that machine's");
    let text = err.to_string();
    assert!(
        text.contains("different title"),
        "the refusal names which part: {text}"
    );
}

#[test]
fn something_that_is_not_a_snapshot_is_refused_before_anything_is_read() {
    scratch();
    let mut host = Headless::new();
    let mut machine = Machine::new(&STUB, &mut host).expect("a machine");
    machine.load_stub().expect("the stub loads");
    let err = machine
        .resume(b"this is not a snapshot")
        .expect_err("nor is it one");
    assert!(err.to_string().contains("not a snapshot"), "{err}");
}

#[test]
fn snapshots_are_written_on_the_cadence_that_was_asked_for() {
    scratch();
    let cli = Cli {
        stub: true,
        snapshot_every: Some(1),
        ..Cli::default()
    };
    let ran = testkit::run_headless(&STUB, &cli, 1_000);
    ran.assert_exited(stub::EXIT_CODE)
        .assert_logged("snapshot flip-1.snapshot");
    assert!(
        ran.dir.join("flip-1.snapshot").is_file(),
        "the snapshot is in the run directory"
    );
    // Taking them goes through the one run loop, so the run is the same
    // machine as one that took none (#94).
    let none = Cli {
        snapshot_every: None,
        ..cli
    };
    let plain = testkit::run_headless(&STUB, &none, 1_000);
    assert_eq!(ran.finish, plain.finish, "the same run");
    assert_eq!(ran.digest, plain.digest, "and the machine looks the same");
}

#[test]
fn nothing_is_written_when_nothing_was_asked_for() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&STUB, &cli, 1_000);
    let snapshots = std::fs::read_dir(&ran.dir)
        .expect("the run directory reads")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".snapshot"))
        .count();
    assert_eq!(
        snapshots, 0,
        "scrubbability costs nothing until it is asked for"
    );
}

/// Only a wall writes a wall report. The stub guest never reaches one, so
/// what is held here is the other half: a run cut short by its budget, which
/// is not a wall, writes none.
#[test]
fn a_budget_running_out_writes_no_wall_report() {
    scratch();
    let cli = Cli {
        stub: true,
        ..Cli::default()
    };
    let ran = testkit::run_headless(&STUB, &cli, 6);
    assert_eq!(
        ran.finish.outcome,
        lego_legacy_core::machine::Outcome::BudgetExhausted
    );
    assert!(
        !ran.dir.join("wall").is_file(),
        "a budget running out is not a wall"
    );
}

#[test]
fn x87_full_state_survives_snapshot_and_restore_bit_exact() {
    let (mut m, _host) = machine_at(4);
    // Drive non-zero top, custom control/status, -0.0, and NaN payload bits.
    m.cpu.fpu.set_control(0x0c3f);
    m.cpu.fpu.push(1.25);
    m.cpu.fpu.push(-0.0);
    m.cpu.fpu.push(f64::from_bits(0x7ff8_1234_5678_9abc));
    let _ = m.cpu.fpu.pop();
    m.cpu.fpu.push(std::f64::consts::PI);

    let expected_ctrl = m.cpu.fpu.control();
    let expected_stat = m.cpu.fpu.status();
    let expected_st: [(bool, u64); 8] =
        std::array::from_fn(|i| (m.cpu.fpu.occupied(i), m.cpu.fpu.st(i).to_bits()));

    let bytes = m.snapshot_v2().expect("snapshot v2");

    // Clobber every x87 register and control word.
    m.cpu.fpu = lego_legacy_core::cpu::x87::X87::default();
    m.cpu.fpu.set_control(0x027f);
    for k in 0..5 {
        m.cpu.fpu.push(f64::from(k) * 7.0);
    }

    m.restore_v2(&bytes).expect("restore v2");
    assert_eq!(m.cpu.fpu.control(), expected_ctrl);
    assert_eq!(m.cpu.fpu.status(), expected_stat);
    let actual_st: [(bool, u64); 8] =
        std::array::from_fn(|i| (m.cpu.fpu.occupied(i), m.cpu.fpu.st(i).to_bits()));
    assert_eq!(actual_st, expected_st);
}

#[test]
fn live_restore_zeroes_post_snapshot_dirty_pages_and_invalidates_code_cache() {
    use lego_legacy_core::guest::GuestMem;

    let (mut m, mut host) = machine_at(0);
    // Program: eax = 1; int 0x20 (present); add eax, 0x10; int 0x21 (exit).
    let prog_a: [u8; 14] = [
        0xb8,
        0x01,
        0x00,
        0x00,
        0x00, // mov eax, 1
        0xcd,
        stub::INT_PRESENT, // int 0x20
        0x83,
        0xc0,
        0x10, // add eax, 0x10
        0xcd,
        stub::INT_EXIT, // int 0x21
        0x90,
        0x90,
    ];
    m.arena.write(stub::ENTRY, &prog_a).expect("write stub");
    let mut log = lego_legacy_core::runlog::RunLog::create(
        "snap-dirty-live",
        lego_legacy_core::cli::Trace::Off,
    )
    .expect("log");

    // Step past mov eax, 1 and int 0x20 (flip 0 -> 1).
    while m.flips() < 1 {
        assert!(m.tick(&mut host, &mut log).is_none());
    }
    let snap = m.snapshot_v2().expect("snapshot");
    let snap_digest = m.state_digest();

    // Mutate guest code (self-modifying overwrite), allocate fresh heap pages,
    // and dirty pages well above snapshot heap extent so Interp decodes the new code.
    let new_alloc = m.state.heap.alloc(0x8000);
    assert_ne!(new_alloc, 0, "heap alloc");
    m.arena
        .write(new_alloc, &vec![0xab; 0x8000])
        .expect("dirty heap");
    let prog_b: [u8; 14] = [
        0xb8,
        0x01,
        0x00,
        0x00,
        0x00,
        0xcd,
        stub::INT_PRESENT,
        0x83,
        0xc0,
        0x7f,
        0xcd,
        stub::INT_EXIT,
        0x90,
        0x90,
    ];
    m.arena.write(stub::ENTRY, &prog_b).expect("overwrite code");
    let finish_b = m.run(&mut host, &mut log, 1_000);
    assert_eq!(
        finish_b.outcome,
        lego_legacy_core::machine::Outcome::Exited { code: 0x80 }
    );

    // Restore live: post-snapshot heap pages MUST be zeroed back and stale
    // decoded blocks for `prog_b` MUST be invalidated so `prog_a` executes.
    m.restore_v2(&snap).expect("live restore");
    assert_eq!(m.state_digest(), snap_digest);
    let mut zero_check = [0xffu8; 0x2000];
    m.arena
        .read(new_alloc, &mut zero_check)
        .expect("read zeroed page");
    assert!(
        zero_check.iter().all(|&b| b == 0),
        "dirty post-snapshot pages must be zeroed on restore"
    );

    let finish_a = m.run(&mut host, &mut log, 1_000);
    assert_eq!(
        finish_a.outcome,
        lego_legacy_core::machine::Outcome::Exited { code: 0x11 }
    );
}

#[test]
fn profile_overlay_and_deferred_readonly_files_round_trip_live_and_cold() {
    use lego_legacy_core::win32::{Contents, Object};

    scratch();
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("snap-profile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let profile_live = root.join("profile-live");
    let profile_cold = root.join("profile-cold");
    std::fs::create_dir_all(profile_live.join("sub")).unwrap();
    std::fs::write(profile_live.join("slot1.sav"), b"v1-save-data").unwrap();
    std::fs::write(profile_live.join("sub/meta.bin"), b"meta-bytes").unwrap();

    // Immutable >= 64 KiB declared file to exercise deferred read-only handle rehydration.
    let big_archive: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();

    let mut host = Headless::new();
    let mut m = Machine::new(&STUB, &mut host).expect("machine");
    m.load_stub().expect("stub");
    m.tree.set_profile(profile_live.clone());
    m.tree.declare("C:\\DATA\\BIG.PAK", big_archive.clone());

    let h_ro = m.state.give(Object::File {
        path: "C:\\DATA\\BIG.PAK".to_owned(),
        contents: Contents::Held(big_archive.clone()),
        at: 1234,
        access: 0x8000_0000, // GENERIC_READ
        share: 1,
        wrote: false,
    });
    let h_rw = m.state.give(Object::File {
        path: "C:\\PROFILE\\SLOT1.SAV".to_owned(),
        contents: Contents::Held(b"dirty-unflushed".to_vec()),
        at: 5,
        access: 0xc000_0000, // GENERIC_READ | GENERIC_WRITE
        share: 0,
        wrote: true,
    });

    let snap = m.snapshot_v2().expect("snapshot v2");
    let digest_0 = m.state_digest();
    // Deferred read-only 100 KB archive must NOT inflate the snapshot payload.
    assert!(
        snap.len() < big_archive.len(),
        "deferred read-only archive should be omitted from snapshot (len={})",
        snap.len()
    );
    // Live handle contents must remain intact after snapshot.
    assert_eq!(
        m.state.object(h_ro),
        Some(&Object::File {
            path: "C:\\DATA\\BIG.PAK".to_owned(),
            contents: Contents::Held(big_archive.clone()),
            at: 1234,
            access: 0x8000_0000,
            share: 1,
            wrote: false,
        })
    );

    // Mutate profile files on disk and close handles.
    std::fs::write(profile_live.join("slot1.sav"), b"OVERWRITTEN").unwrap();
    std::fs::write(profile_live.join("extra.sav"), b"brand-new").unwrap();
    std::fs::remove_file(profile_live.join("sub/meta.bin")).unwrap();
    m.state.close(h_rw);

    // 1. Live restore rolls back profile files + handles + state digest bit-for-bit.
    m.restore_v2(&snap).expect("live restore");
    assert_eq!(m.state_digest(), digest_0);
    assert_eq!(
        std::fs::read(profile_live.join("slot1.sav")).unwrap(),
        b"v1-save-data"
    );
    assert_eq!(
        std::fs::read(profile_live.join("sub/meta.bin")).unwrap(),
        b"meta-bytes"
    );
    assert!(!profile_live.join("extra.sav").exists());
    assert!(m.state.object(h_rw).is_some());
    assert_eq!(
        m.state.object(h_ro),
        Some(&Object::File {
            path: "C:\\DATA\\BIG.PAK".to_owned(),
            contents: Contents::Held(big_archive.clone()),
            at: 1234,
            access: 0x8000_0000,
            share: 1,
            wrote: false,
        })
    );

    // 2. Cold restore into a fresh Machine with an empty profile directory.
    let mut host2 = Headless::new();
    let mut cold = Machine::new(&STUB, &mut host2).expect("cold machine");
    cold.load_stub().expect("stub");
    cold.tree.set_profile(profile_cold.clone());
    cold.tree.declare("C:\\DATA\\BIG.PAK", big_archive.clone());
    cold.restore_v2(&snap).expect("cold restore");
    assert_eq!(cold.state_digest(), digest_0);
    assert_eq!(
        std::fs::read(profile_cold.join("slot1.sav")).unwrap(),
        b"v1-save-data"
    );
    assert_eq!(
        std::fs::read(profile_cold.join("sub/meta.bin")).unwrap(),
        b"meta-bytes"
    );
    assert_eq!(
        cold.state.object(h_ro),
        Some(&Object::File {
            path: "C:\\DATA\\BIG.PAK".to_owned(),
            contents: Contents::Held(big_archive),
            at: 1234,
            access: 0x8000_0000,
            share: 1,
            wrote: false,
        })
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn truncated_v1_and_route_mismatched_snapshots_are_handled_cleanly() {
    use lego_legacy_core::route::Route;

    let (mut m, _host) = machine_at(4);
    let snap = m.snapshot_v2().expect("snapshot v2");

    // 1. Legacy v1 magic is rejected cleanly.
    let mut v1 = snap.clone();
    v1[6] = 1;
    let err = m.restore_v2(&v1).expect_err("v1 must be refused");
    assert!(err.to_string().contains("not a snapshot"), "{err}");

    // 2. Truncations across header & payload refuse without panic or OOM.
    for cut in [0, 4, 8, 12, 24, 64, snap.len() / 2, snap.len() - 1] {
        let mut fresh_host = Headless::new();
        let mut fresh = Machine::new(&STUB, &mut fresh_host).expect("machine");
        fresh.load_stub().expect("stub");
        assert!(
            fresh.restore_v2(&snap[..cut]).is_err(),
            "truncation at {cut} must error"
        );
    }

    // 3. Different route is refused by strict `restore_v2`, allowed by `restore_v2_live`.
    let mut h2 = Headless::new();
    let mut other_route = Machine::new(&STUB, &mut h2).expect("machine");
    other_route.load_stub().expect("stub");
    other_route.route = Some(Route::parse("#lego-legacy-route 1\nflip 0 close\n").unwrap());
    let err = other_route
        .restore_v2(&snap)
        .expect_err("strict route mismatch must be refused");
    assert!(err.to_string().contains("different route"), "{err}");
    other_route
        .restore_v2_live(&snap)
        .expect("live restore permits differing pre-roll route");

    // 4. Path traversal inside crafted profile overlay is refused cleanly.
    let profile_dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("snap-trav-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&profile_dir);
    std::fs::create_dir_all(&profile_dir).unwrap();
    let mut crafted = snap.clone();
    // Replace trailing empty profile_count (0u32) with 1 entry "../escape.txt".
    let tail = crafted.len() - 4;
    crafted.truncate(tail);
    crafted.extend_from_slice(&1u32.to_le_bytes());
    let bad_path = b"../escape.txt";
    crafted.extend_from_slice(&(bad_path.len() as u32).to_le_bytes());
    crafted.extend_from_slice(bad_path);
    crafted.extend_from_slice(&3u32.to_le_bytes());
    crafted.extend_from_slice(b"bad");

    let mut h3 = Headless::new();
    let mut guarded = Machine::new(&STUB, &mut h3).expect("machine");
    guarded.load_stub().expect("stub");
    guarded.tree.set_profile(profile_dir.clone());
    let err = guarded
        .restore_v2(&crafted)
        .expect_err("traversal must be refused");
    assert!(err.to_string().contains("profile path"), "{err}");
    assert!(!profile_dir.parent().unwrap().join("escape.txt").exists());
    let _ = std::fs::remove_dir_all(&profile_dir);
}

#[test]
fn win32_guest_args_env_window_extra_and_mm_timers_round_trip_through_llsnap_v2() {
    use lego_legacy_core::win32::winmm::MmTimer;

    let (mut m, _host) = machine_at(4);
    m.state.guest_args = "-novideo -window".to_owned();
    m.state
        .environment
        .insert("=C:".to_owned(), Some("C:\\Save".to_owned()));
    m.state.environment.insert("TZ".to_owned(), None);
    m.state.window_extra = vec![0x44, 0xf4, 0x4e, 0x00, 0x11, 0x22, 0x33, 0x44];
    let timer = MmTimer {
        id: 7,
        delay_ms: 100,
        callback: 0x004b_d5e0,
        user: 0x0123_4567,
        periodic: true,
        due_ms: 5_200,
    };
    m.state.mm_timers.push(timer);
    m.state.next_mm_timer = 8;
    m.state.mm_timer_thread = Some(0);

    let snap = m.snapshot_v2().expect("snapshot v2");
    let mut host = Headless::new();
    let mut restored = Machine::new(&STUB, &mut host).expect("machine");
    restored.load_stub().expect("stub");
    restored.restore_v2(&snap).expect("restore v2");

    assert_eq!(restored.state.guest_args, "-novideo -window");
    assert_eq!(
        restored.state.environment.get("=C:"),
        Some(&Some("C:\\Save".to_owned()))
    );
    assert_eq!(restored.state.environment.get("TZ"), Some(&None));
    assert_eq!(
        restored.state.window_extra,
        vec![0x44, 0xf4, 0x4e, 0x00, 0x11, 0x22, 0x33, 0x44]
    );
    assert_eq!(restored.state.mm_timers, vec![timer]);
    assert_eq!(restored.state.next_mm_timer, 8);
    assert_eq!(restored.state.mm_timer_thread, Some(0));

    // Corrupt snapshot validation: zero delay_ms and OOB mm_timer_thread are refused.
    m.state.mm_timers[0].delay_ms = 0;
    let bad_delay = m.snapshot_v2().expect("serialize bad delay");
    assert!(restored.restore_v2(&bad_delay).is_err());

    m.state.mm_timers[0].delay_ms = 100;
    m.state.mm_timer_thread = Some(99);
    let bad_thread = m.snapshot_v2().expect("serialize OOB thread");
    assert!(restored.restore_v2(&bad_thread).is_err());
}

#[test]
fn directshow_filter_graph_round_trips_through_llsnap_v2_and_rehydrates_movie_bytes() {
    use lego_legacy_core::movie::MoviePlayer;
    use lego_legacy_core::win32::dshow::{FilterGraph, EC_COMPLETE};
    use lego_legacy_core::win32::Object;

    let avi_bytes = synthetic_iv50_avi_with_payload(4, 262_144);
    let (mut m, _host) = machine_at(4);
    m.tree.declare("C:\\GAME\\INTRO.AVI", avi_bytes.clone());
    let ev = m.state.give(Object::Named {
        kind: "event",
        name: String::new(),
        signalled: true,
        manual: true,
        held: None,
    });
    let mut movie = MoviePlayer::open(
        "intro.avi".to_owned(),
        "C:\\GAME\\INTRO.AVI",
        &mut m.state.movies,
        &mut m.tree,
    )
    .expect("open movie");
    movie
        .start(0x2000_0000, 66, &mut m.state.mixer)
        .expect("start");
    let buf = m.state.heap.alloc(16 * 16 * 2);
    movie
        .step_rgb565(&mut m.arena, buf, 16, 16, 32)
        .expect("step 0");
    let mut graph = FilterGraph {
        event_handle: ev,
        window_style: 0x0603_0000,
        owner: 0x10010,
        message_drain: 0x10010,
        visible: true,
        fullscreen: true,
        cursor_hidden: true,
        position: [0, 0, 640, 480],
        frame_buffer: buf,
        movie: Some(movie),
        ..FilterGraph::default()
    };
    graph.events.push_back((EC_COMPLETE, 0, 0));
    m.state.filter_graphs.insert(0x2000_0000, graph);

    let snap = m.snapshot_v2().expect("snapshot v2");
    assert!(
        snap.len() < avi_bytes.len(),
        "dshow snapshot must omit raw AVI bytes: snap={} >= raw_avi={}",
        snap.len(),
        avi_bytes.len()
    );
    let digest_0 = m.state_digest();

    let mut host = Headless::new();
    let mut restored = Machine::new(&STUB, &mut host).expect("machine");
    restored.load_stub().expect("stub");
    restored
        .tree
        .declare("C:\\GAME\\INTRO.AVI", avi_bytes.clone());
    restored.restore_v2(&snap).expect("cold restore v2");
    assert_eq!(restored.state_digest(), digest_0);

    let rg = restored
        .state
        .filter_graphs
        .get(&0x2000_0000)
        .expect("FilterGraph restored");
    assert_eq!(rg.event_handle, ev);
    assert_eq!(rg.window_style, 0x0603_0000);
    assert_eq!(rg.owner, 0x10010);
    assert_eq!(rg.message_drain, 0x10010);
    assert!(rg.visible && rg.fullscreen && rg.cursor_hidden);
    assert_eq!(rg.position, [0, 0, 640, 480]);
    assert_eq!(rg.events.front(), Some(&(EC_COMPLETE, 0, 0)));
    let rm = rg.movie.as_ref().expect("movie restored");
    assert_eq!(rm.name(), "intro.avi");
    assert_eq!(rm.next_frame(), 1);
    assert_eq!(rm.running_since(), Some(66));
    assert!(rm.is_running());
    assert!(
        restored.state.movies.contains("C:\\GAME\\INTRO.AVI"),
        "cold restore rehydrates immutable AVI bytes from Tree"
    );

    // Cold restore without the AVI in Tree must fail cleanly as corrupt.
    let mut host_missing = Headless::new();
    let mut missing = Machine::new(&STUB, &mut host_missing).expect("missing machine");
    missing.load_stub().expect("stub");
    assert!(missing.restore_v2(&snap).is_err());
}

#[test]
fn gdi_state_and_pcm_acm_stream_round_trip_through_llsnap_v2_and_rehydrate_fonts() {
    use lego_legacy_core::sound::Format;
    use lego_legacy_core::ttf::Font;
    use lego_legacy_core::win32::gdi32::{DcState, GdiObject};
    use lego_legacy_core::win32::msacm32::Stream;

    // Minimal valid TrueType font bytes with family "SnapFont".
    let mut head = vec![0u8; 54];
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[4..6].copy_from_slice(&1u16.to_be_bytes());
    let fam: Vec<u8> = "SnapFont"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let mut name = Vec::new();
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&1u16.to_be_bytes());
    name.extend_from_slice(&18u16.to_be_bytes());
    name.extend_from_slice(&3u16.to_be_bytes());
    name.extend_from_slice(&1u16.to_be_bytes());
    name.extend_from_slice(&0x0409u16.to_be_bytes());
    name.extend_from_slice(&1u16.to_be_bytes());
    name.extend_from_slice(&(fam.len() as u16).to_be_bytes());
    name.extend_from_slice(&0u16.to_be_bytes());
    name.extend_from_slice(&fam);
    let cmap = vec![0u8; 16];
    let hmtx = vec![0u8; 4];
    let loca = vec![0u8; 4];
    let glyf = vec![0u8; 4];
    let tables: &[(&[u8; 4], &[u8])] = &[
        (b"cmap", &cmap),
        (b"glyf", &glyf),
        (b"head", &head),
        (b"hhea", &hhea),
        (b"hmtx", &hmtx),
        (b"loca", &loca),
        (b"maxp", &maxp),
        (b"name", &name),
    ];
    let mut ttf_bytes = Vec::new();
    ttf_bytes.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    ttf_bytes.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    ttf_bytes.extend_from_slice(&[0u8; 6]);
    let mut off = 12 + tables.len() * 16;
    for (tag, data) in tables {
        ttf_bytes.extend_from_slice(*tag);
        ttf_bytes.extend_from_slice(&0u32.to_be_bytes());
        ttf_bytes.extend_from_slice(&(off as u32).to_be_bytes());
        ttf_bytes.extend_from_slice(&(data.len() as u32).to_be_bytes());
        off += (data.len() + 3) & !3;
    }
    for (_, data) in tables {
        ttf_bytes.extend_from_slice(data);
        while !ttf_bytes.len().is_multiple_of(4) {
            ttf_bytes.push(0);
        }
    }
    assert!(Font::parse(ttf_bytes.clone().into()).is_some());

    let (mut m, _host) = machine_at(4);
    m.tree.declare("C:\\GAME\\SNAP.TTF", ttf_bytes.clone());
    m.state.gdi.font_files.push("C:\\GAME\\SNAP.TTF".to_owned());
    m.state.gdi.next_handle = 0x0003_0010;
    m.state.gdi.objects.insert(
        0x0200_0000,
        GdiObject::Font {
            height: 24,
            weight: 700,
            face: "SnapFont".to_owned(),
        },
    );
    m.state.gdi.objects.insert(
        0x0200_0004,
        GdiObject::Pen {
            style: 0,
            width: 2,
            color: 0x0011_2233,
        },
    );
    m.state.gdi.dcs.insert(
        0x0004_0000,
        DcState {
            font: 0x0200_0000,
            pen: 0x0200_0004,
            brush: 0x01ff_ffff,
            text_color: 0x00aa_bbcc,
            bk_color: 0x00ff_ffff,
            bk_mode: 1,
            text_align: 6,
            pos: (12, 34),
            clip_rect: Some([1, 2, 100, 80]),
        },
    );
    let pcm_src = Format {
        tag: 1,
        channels: 1,
        rate: 22_050,
        block_align: 1,
        bits: 8,
        coefficients: Vec::new(),
    };
    let pcm_dst = Format {
        tag: 1,
        channels: 2,
        rate: 44_100,
        block_align: 4,
        bits: 16,
        coefficients: Vec::new(),
    };
    m.state
        .acm_streams
        .insert(7, Stream::from_formats(pcm_src, pcm_dst));

    let snap = m.snapshot_v2().expect("snapshot v2");
    let mut host = Headless::new();
    let mut restored = Machine::new(&STUB, &mut host).expect("machine");
    restored.load_stub().expect("stub");
    restored.tree.declare("C:\\GAME\\SNAP.TTF", ttf_bytes);
    restored.restore_v2(&snap).expect("cold restore v2");

    assert_eq!(restored.state.gdi.font_files, vec!["C:\\GAME\\SNAP.TTF"]);
    assert_eq!(restored.state.gdi.next_handle, 0x0003_0010);
    assert_eq!(restored.state.gdi.objects, m.state.gdi.objects);
    assert_eq!(restored.state.gdi.dcs, m.state.gdi.dcs);
    assert_eq!(restored.state.gdi.loaded_fonts.len(), 1);
    assert_eq!(restored.state.gdi.loaded_fonts[0].1.family, "SnapFont");
    assert!(restored.state.acm_streams.contains_key(&7));

    // Cold restore without the registered TTF in `Tree` must fail as corrupt.
    let mut host_missing = Headless::new();
    let mut missing = Machine::new(&STUB, &mut host_missing).expect("machine");
    missing.load_stub().expect("stub");
    assert!(missing.restore_v2(&snap).is_err());

    // Corrupt ACM stream (rate == 0) must be rejected on restore.
    let mut bad_m = m;
    bad_m.state.acm_streams.insert(
        9,
        Stream::from_formats(
            Format {
                tag: 1,
                channels: 1,
                rate: 0,
                block_align: 2,
                bits: 16,
                coefficients: Vec::new(),
            },
            Format {
                tag: 1,
                channels: 1,
                rate: 22_050,
                block_align: 2,
                bits: 16,
                coefficients: Vec::new(),
            },
        ),
    );
    let bad_snap = bad_m.snapshot_v2().expect("serialize bad acm stream");
    assert!(restored.restore_v2(&bad_snap).is_err());
}

#[test]
fn directmusic_state_and_file_backed_styles_round_trip_through_llsnap_v2() {
    use lego_legacy_core::dls::Bank;
    use lego_legacy_core::dmusic::{parse_style, Segment, StyleRef};
    use lego_legacy_core::synth::Synth;
    use lego_legacy_core::win32::dmusic::{LoadedStyle, Notification, PlayingSegment};

    // Build a valid minimal RIFF:DMST style file for Tree rehydration.
    let mut styh = vec![4u8, 4, 8, 0];
    styh.extend_from_slice(&60.0f64.to_le_bytes());
    let mut style_body = Vec::new();
    style_body.extend_from_slice(b"DMST");
    style_body.extend_from_slice(b"styh");
    style_body.extend_from_slice(&(styh.len() as u32).to_le_bytes());
    style_body.extend_from_slice(&styh);
    let mut style_bytes = Vec::new();
    style_bytes.extend_from_slice(b"RIFF");
    style_bytes.extend_from_slice(&(style_body.len() as u32).to_le_bytes());
    style_bytes.extend_from_slice(&style_body);
    let parsed_style = parse_style(&style_bytes).expect("parse minimal DMST");

    let (mut m, _host) = machine_at(4);
    let bank = Bank::default();
    m.state.mixer.open_synth(Synth::new(bank.clone()));
    m.tree
        .declare("C:\\GAME\\IMUSIC\\SNAP.STY", style_bytes.clone());
    m.state
        .dmusic
        .search_dirs
        .insert("all".to_owned(), "C:\\GAME\\IMUSIC".to_owned());
    m.state.dmusic.styles.insert(
        10,
        LoadedStyle {
            guid: [3u8; 16],
            name: "SnapStyle".to_owned(),
            filename: "SNAP.STY".to_owned(),
            source_path: Some("C:\\GAME\\IMUSIC\\SNAP.STY".to_owned()),
            raw_memory: Vec::new(),
            parsed: Some(parsed_style.clone()),
        },
    );
    m.state.dmusic.styles.insert(
        11,
        LoadedStyle {
            guid: [4u8; 16],
            name: "MemStyle".to_owned(),
            filename: String::new(),
            source_path: None,
            raw_memory: style_bytes.clone(),
            parsed: Some(parsed_style),
        },
    );
    m.state.dmusic.segments.insert(
        20,
        Segment {
            name: "SnapSeg".to_owned(),
            length: 3072,
            styles: vec![StyleRef {
                time: 0,
                guid: [3u8; 16],
                name: "SnapStyle".to_owned(),
                file: "SNAP.STY".to_owned(),
            }],
            ..Segment::default()
        },
    );
    m.state.dmusic.notify_event = 0x104;
    m.state.dmusic.notify_segment = true;
    m.state.dmusic.notify_measure_beat = true;
    m.state.dmusic.bound_dsound_buffer = 0x2000_0100;
    m.state.dmusic.master_groove_offset = 2;
    m.state.dmusic.playing = Some(PlayingSegment {
        seg_id: 20,
        anchor_nanos: 1_000_000,
        anchor_tick: 0,
        millibpm: 60_000,
        next_measure_tick: 3072,
        next_beat_tick: 768,
        measure_index: 0,
        beat_index: 1,
        repeats_left: 5,
        almost_end_sent: false,
    });
    m.state.dmusic.notifications.push_back(Notification {
        guid_type: [7u8; 16],
        option: 0,
        field1: 1,
        field2: 0,
        music_time: 768,
        ref_time: 10_000,
    });

    let snap = m.snapshot_v2().expect("capture DirectMusic LLSNAP v2");

    let mut host = Headless::new();
    let mut restored = Machine::new(&STUB, &mut host).expect("machine");
    restored.load_stub().expect("stub");
    restored.state.mixer.open_synth(Synth::new(bank.clone()));
    restored
        .tree
        .declare("C:\\GAME\\IMUSIC\\SNAP.STY", style_bytes);
    restored
        .restore_v2(&snap)
        .expect("cold restore DirectMusic");

    assert_eq!(restored.state.dmusic.playing, m.state.dmusic.playing);
    assert_eq!(
        restored.state.dmusic.notifications,
        m.state.dmusic.notifications
    );
    assert_eq!(restored.state.dmusic.master_groove_offset, 2);
    assert_eq!(restored.state.dmusic.bound_dsound_buffer, 0x2000_0100);
    assert!(restored
        .state
        .dmusic
        .styles
        .get(&10)
        .is_some_and(|s| s.parsed.is_some()));
    assert!(restored
        .state
        .dmusic
        .styles
        .get(&11)
        .is_some_and(|s| s.parsed.is_some() && !s.raw_memory.is_empty()));

    // Missing file-backed style in Tree must fail cold restore as corrupt.
    let mut host_missing = Headless::new();
    let mut missing = Machine::new(&STUB, &mut host_missing).expect("machine");
    missing.load_stub().expect("stub");
    missing.state.mixer.open_synth(Synth::new(bank));
    assert!(missing.restore_v2(&snap).is_err());
}

fn synthetic_iv50_avi_with_payload(frames: usize, junk_bytes: usize) -> Vec<u8> {
    let chunk = |id: &[u8; 4], body: &[u8]| {
        let mut out = id.to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    };
    let list = |kind: &[u8; 4], parts: &[Vec<u8>]| {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend_from_slice(p);
        }
        chunk(b"LIST", &body)
    };

    let mut strh_v = vec![0u8; 56];
    strh_v[0..4].copy_from_slice(b"vids");
    strh_v[4..8].copy_from_slice(b"IV50");
    strh_v[20..24].copy_from_slice(&1u32.to_le_bytes());
    strh_v[24..28].copy_from_slice(&15u32.to_le_bytes());
    strh_v[32..36].copy_from_slice(&(frames as u32).to_le_bytes());
    let mut strf_v = vec![0u8; 40];
    strf_v[0..4].copy_from_slice(&40u32.to_le_bytes());
    strf_v[4..8].copy_from_slice(&16u32.to_le_bytes());
    strf_v[8..12].copy_from_slice(&16u32.to_le_bytes());
    strf_v[14..16].copy_from_slice(&24u16.to_le_bytes());
    strf_v[16..20].copy_from_slice(b"IV50");
    let strl_v = list(b"strl", &[chunk(b"strh", &strh_v), chunk(b"strf", &strf_v)]);

    let mut strh_a = vec![0u8; 56];
    strh_a[0..4].copy_from_slice(b"auds");
    strh_a[20..24].copy_from_slice(&4u32.to_le_bytes());
    strh_a[24..28].copy_from_slice(&(22_050u32 * 4).to_le_bytes());
    strh_a[44..48].copy_from_slice(&4u32.to_le_bytes());
    let mut strf_a = vec![0u8; 16];
    strf_a[0..2].copy_from_slice(&1u16.to_le_bytes());
    strf_a[2..4].copy_from_slice(&2u16.to_le_bytes());
    strf_a[4..8].copy_from_slice(&22_050u32.to_le_bytes());
    strf_a[8..12].copy_from_slice(&(22_050u32 * 4).to_le_bytes());
    strf_a[12..14].copy_from_slice(&4u16.to_le_bytes());
    strf_a[14..16].copy_from_slice(&16u16.to_le_bytes());
    let strl_a = list(b"strl", &[chunk(b"strh", &strh_a), chunk(b"strf", &strf_a)]);

    let hdrl = list(b"hdrl", &[strl_v, strl_a]);
    let mut movi_parts = Vec::new();
    let pcm: Vec<u8> = (0..400usize).map(|i| ((i * 37) ^ 0x5a) as u8).collect();
    movi_parts.push(chunk(b"01wb", &pcm));
    for _ in 0..frames {
        movi_parts.push(chunk(b"00dc", &[]));
    }
    let movi = list(b"movi", &movi_parts);
    let mut riff = b"AVI ".to_vec();
    riff.extend_from_slice(&hdrl);
    if junk_bytes > 0 {
        let junk: Vec<u8> = (0..junk_bytes).map(|i| ((i * 19) ^ 0xa5) as u8).collect();
        riff.extend_from_slice(&chunk(b"JUNK", &junk));
    }
    riff.extend_from_slice(&movi);
    chunk(b"RIFF", &riff)
}

fn with_machine_ctx<R>(
    m: &mut Machine,
    tag: &str,
    f: impl FnOnce(&mut lego_legacy_core::win32::Ctx<'_>) -> R,
) -> R {
    let mut log = lego_legacy_core::runlog::RunLog::create(tag, lego_legacy_core::cli::Trace::Off)
        .expect("log");
    let dir = log.dir().to_path_buf();
    let mut ctx = lego_legacy_core::win32::Ctx {
        cpu: &mut m.cpu,
        mem: &mut m.arena,
        state: &mut m.state,
        title: &STUB,
        log: &mut log,
        tree: &mut m.tree,
        modules: &mut m.modules,
        clock_millis: m.clock.millis(),
        boundary: &mut m.boundary,
        call_guest: None,
        float_answer: None,
        answer_is_the_call_s: false,
    };
    let res = f(&mut ctx);
    drop(log);
    let _ = std::fs::remove_dir_all(dir);
    res
}

#[test]
fn amstream_movie_snapshot_omits_raw_bytes_and_cold_restores_bit_exact() {
    use lego_legacy_core::com::S_OK;
    use lego_legacy_core::guest::GuestMem;
    use lego_legacy_core::win32::amstream::{
        self, DIRECT_DRAW_MEDIA_STREAM, DIRECT_DRAW_STREAM_SAMPLE, MULTI_MEDIA_STREAM,
    };

    let avi_bytes = synthetic_iv50_avi_with_payload(4, 262_144);
    let (mut m, _host) = machine_at(4);
    m.state.current_directory = "C:\\Game".to_owned();
    m.tree.declare("C:\\GAME\\INTRO.AVI", avi_bytes.clone());

    let call_com = |m: &mut Machine,
                    iface: &'static lego_legacy_core::com::Interface,
                    slot: usize,
                    this: u32,
                    args: &[u32]|
     -> u32 {
        let f = iface.method(slot).expect("method").run;
        with_machine_ctx(m, "amstream-snap", |ctx| {
            f(ctx, this, args).expect("com call")
        })
    };
    let player = with_machine_ctx(&mut m, "amstream-snap", |ctx| {
        amstream::create(ctx).expect("create player")
    });

    // Write MSPID_PrimaryVideo GUID (a35ff56a-9fda-11d0-8fdf-00c04fd9189d) and wide path.
    let guid_at = m.state.heap.alloc(16);
    let mspid_video: [u8; 16] = [
        0x6a, 0xf5, 0x5f, 0xa3, 0xda, 0x9f, 0xd0, 0x11, 0x8f, 0xdf, 0x00, 0xc0, 0x4f, 0xd9, 0x18,
        0x9d,
    ];
    m.arena.write(guid_at, &mspid_video).unwrap();
    let out_at = m.state.heap.alloc(16);
    let name_at = m.state.heap.alloc(64);
    let wide: Vec<u8> = "intro.avi\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    m.arena.write(name_at, &wide).unwrap();

    // AddMediaStream (slot 15), OpenFile (slot 16), CreateSample (slot 13), SetState(RUN) (slot 7), Update (slot 6).
    assert_eq!(
        call_com(
            &mut m,
            &MULTI_MEDIA_STREAM,
            15,
            player,
            &[0, guid_at, 0, out_at]
        ),
        S_OK
    );
    let video_stream = m.arena.read_u32(out_at).unwrap();
    assert_eq!(
        call_com(&mut m, &MULTI_MEDIA_STREAM, 16, player, &[name_at, 0]),
        S_OK
    );
    assert_eq!(
        call_com(
            &mut m,
            &DIRECT_DRAW_MEDIA_STREAM,
            13,
            video_stream,
            &[0, 0, 0, out_at]
        ),
        S_OK
    );
    let sample = m.arena.read_u32(out_at).unwrap();
    assert_eq!(call_com(&mut m, &MULTI_MEDIA_STREAM, 7, player, &[1]), S_OK);
    assert_eq!(
        call_com(&mut m, &DIRECT_DRAW_STREAM_SAMPLE, 6, sample, &[0, 0, 0, 0]),
        S_OK
    );

    let snap = m.snapshot_v2().expect("snapshot v2");
    assert!(
        snap.len() < avi_bytes.len(),
        "amstream snapshot must omit raw AVI bytes: snap={} bytes >= raw_avi={} bytes",
        snap.len(),
        avi_bytes.len()
    );

    let digest_at_snap = m.state_digest();
    let mut host2 = Headless::new();
    let mut restored = Machine::new(&STUB, &mut host2).expect("cold machine");
    restored.load_stub().expect("stub");
    restored
        .tree
        .declare("C:\\GAME\\INTRO.AVI", avi_bytes.clone());
    restored.restore_v2(&snap).expect("cold restore v2");
    assert_eq!(restored.state_digest(), digest_at_snap);
    assert!(
        restored.state.movies.contains("C:\\GAME\\INTRO.AVI"),
        "cold restore rehydrates amstream movie bytes via MovieStore"
    );

    // Stepping frame 1 on both original and cold-restored machines must remain bit-exact.
    assert_eq!(
        call_com(&mut m, &DIRECT_DRAW_STREAM_SAMPLE, 6, sample, &[0, 0, 0, 0]),
        S_OK
    );
    assert_eq!(
        call_com(
            &mut restored,
            &DIRECT_DRAW_STREAM_SAMPLE,
            6,
            sample,
            &[0, 0, 0, 0]
        ),
        S_OK
    );
    assert_eq!(restored.state_digest(), m.state_digest());

    // Live restore sweeps movies opened after the snapshot while keeping cached pre-snapshot ones.
    let extra_avi = synthetic_iv50_avi_with_payload(2, 4096);
    m.tree.declare("C:\\GAME\\EXTRA.AVI", extra_avi);
    let extra_player = with_machine_ctx(&mut m, "amstream-snap", |ctx| {
        amstream::create(ctx).expect("extra player")
    });
    let extra_name_at = m.state.heap.alloc(64);
    let extra_wide: Vec<u8> = "extra.avi\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    m.arena.write(extra_name_at, &extra_wide).unwrap();
    assert_eq!(
        call_com(
            &mut m,
            &MULTI_MEDIA_STREAM,
            16,
            extra_player,
            &[extra_name_at, 0]
        ),
        S_OK
    );
    assert!(m.state.movies.contains("C:\\GAME\\EXTRA.AVI"));
    m.restore_v2_live(&snap).expect("live restore");
    assert!(!m.state.movies.contains("C:\\GAME\\EXTRA.AVI"));
    assert!(m.state.movies.contains("C:\\GAME\\INTRO.AVI"));
    assert_eq!(m.state_digest(), digest_at_snap);

    // Cold restore without the AVI in Tree must fail cleanly as corrupt.
    let mut host_missing = Headless::new();
    let mut missing = Machine::new(&STUB, &mut host_missing).expect("missing machine");
    missing.load_stub().expect("stub");
    assert!(missing.restore_v2(&snap).is_err());
}

#[test]
fn com_table_and_slab_pool_round_trip_through_llsnap_v2_and_reject_corrupt_data() {
    use lego_legacy_core::com::{self, ComState, ComTable, RELEASE};
    use lego_legacy_core::guest::heap::SlabPool;
    use lego_legacy_core::snap::{Reader, Snap, Visitor, Writer};

    let (mut m, _host) = machine_at(4);

    // Populate live multi-face COM objects, reclaimed COM slots, and all four SlabPools.
    let (dd, dd4, graph, mc) = with_machine_ctx(&mut m, "com-snap", |ctx| {
        let dd = com::create(
            ctx,
            &lego_legacy_core::win32::ddraw::DIRECT_DRAW,
            ComState::None,
        )
        .unwrap();
        let dd4 = com::face(ctx, dd, &lego_legacy_core::win32::ddraw::DIRECT_DRAW4).unwrap();
        let graph = lego_legacy_core::win32::dshow::create(
            ctx,
            &lego_legacy_core::win32::dshow::GRAPH_BUILDER,
        )
        .unwrap();
        let mc = com::face(ctx, graph, &lego_legacy_core::win32::dshow::MEDIA_CONTROL).unwrap();
        // Release FilterGraph so its 2 faces land in `ComTable::free_slots`.
        assert_eq!((RELEASE.run)(ctx, mc, &[]).unwrap(), 0);
        assert_eq!((RELEASE.run)(ctx, graph, &[]).unwrap(), 0);
        (dd, dd4, graph, mc)
    });

    // Populate `surface_pool` (exact), `sound_pool` (best-fit 2x), `palette_pool` (exact),
    // and `vb_pool` (best-fit 2x) in explicit FIFO order.
    let (surf_a, _) = m.state.surface_pool.alloc(&mut m.state.heap, 4096);
    let (surf_b, _) = m.state.surface_pool.alloc(&mut m.state.heap, 4096);
    m.state.surface_pool.free(&m.state.heap, surf_b);
    m.state.surface_pool.free(&m.state.heap, surf_a);

    let (snd_a, _) = m.state.sound_pool.alloc(&mut m.state.heap, 2048);
    let (snd_b, _) = m.state.sound_pool.alloc(&mut m.state.heap, 2048);
    m.state.sound_pool.free(&m.state.heap, snd_a);
    m.state.sound_pool.free(&m.state.heap, snd_b);

    let (pal_a, _) = m.state.palette_pool.alloc(&mut m.state.heap, 1024);
    let (pal_b, _) = m.state.palette_pool.alloc(&mut m.state.heap, 1024);
    m.state.palette_pool.free(&m.state.heap, pal_b);
    m.state.palette_pool.free(&m.state.heap, pal_a);

    let (vb_a, _) = m.state.vb_pool.alloc(&mut m.state.heap, 4096);
    let (vb_b, _) = m.state.vb_pool.alloc(&mut m.state.heap, 4096);
    m.state.vb_pool.free(&m.state.heap, vb_a);
    m.state.vb_pool.free(&m.state.heap, vb_b);

    let snap = m.snapshot_v2().expect("snapshot v2");
    let digest = m.state_digest();

    // 1. Cold restore into a fresh machine.
    let mut host2 = Headless::new();
    let mut restored = Machine::new(&STUB, &mut host2).expect("restored machine");
    restored.load_stub().expect("stub");
    restored.restore_v2(&snap).expect("cold restore");
    assert_eq!(restored.state_digest(), digest);
    assert_eq!(restored.state.com.interface_of(dd), Some("IDirectDraw"));
    assert_eq!(restored.state.com.interface_of(dd4), Some("IDirectDraw4"));
    assert_eq!(restored.state.com.identity(dd4), dd);
    assert_eq!(
        restored.state.com.vtable("IDirectDraw"),
        m.state.com.vtable("IDirectDraw")
    );

    // Exact FIFO reuse order in all four SlabPools and ComTable::free_slots must match bit-for-bit.
    assert_eq!(
        restored
            .state
            .surface_pool
            .alloc(&mut restored.state.heap, 4096),
        (surf_b, true)
    );
    assert_eq!(
        restored
            .state
            .surface_pool
            .alloc(&mut restored.state.heap, 4096),
        (surf_a, true)
    );
    assert_eq!(
        restored
            .state
            .sound_pool
            .alloc(&mut restored.state.heap, 1500),
        (snd_a, true)
    );
    assert_eq!(
        restored
            .state
            .sound_pool
            .alloc(&mut restored.state.heap, 1500),
        (snd_b, true)
    );
    assert_eq!(
        restored
            .state
            .palette_pool
            .alloc(&mut restored.state.heap, 1024),
        (pal_b, true)
    );
    assert_eq!(
        restored
            .state
            .palette_pool
            .alloc(&mut restored.state.heap, 1024),
        (pal_a, true)
    );
    assert_eq!(
        restored.state.vb_pool.alloc(&mut restored.state.heap, 3000),
        (vb_a, true)
    );
    assert_eq!(
        restored.state.vb_pool.alloc(&mut restored.state.heap, 3000),
        (vb_b, true)
    );
    let next_restored = with_machine_ctx(&mut restored, "com-snap", |ctx| {
        com::create(
            ctx,
            &lego_legacy_core::win32::ddraw::DIRECT_DRAW,
            ComState::None,
        )
        .unwrap()
    });
    assert!([graph, mc].contains(&next_restored));

    // 2. Live restore over mutated ComTable + drained SlabPools restores exact pre-snapshot state.
    let extra = with_machine_ctx(&mut m, "com-snap", |ctx| {
        com::create(
            ctx,
            &lego_legacy_core::win32::ddraw::DIRECT_DRAW,
            ComState::None,
        )
        .unwrap()
    });
    assert_eq!(extra, next_restored);
    let _ = m.state.surface_pool.alloc(&mut m.state.heap, 4096);
    let _ = m.state.sound_pool.alloc(&mut m.state.heap, 1500);
    let _ = m.state.palette_pool.alloc(&mut m.state.heap, 1024);
    let _ = m.state.vb_pool.alloc(&mut m.state.heap, 3000);
    m.restore_v2_live(&snap).expect("live restore");
    assert_eq!(m.state_digest(), digest);
    assert_eq!(
        m.state.surface_pool.alloc(&mut m.state.heap, 4096),
        (surf_b, true)
    );
    assert_eq!(
        m.state.sound_pool.alloc(&mut m.state.heap, 1500),
        (snd_a, true)
    );
    assert_eq!(
        m.state.palette_pool.alloc(&mut m.state.heap, 1024),
        (pal_b, true)
    );
    assert_eq!(m.state.vb_pool.alloc(&mut m.state.heap, 3000), (vb_a, true));

    // 3. Corrupt SlabPools (`max_factor == 0`, empty bucket, zero addr) are rejected cleanly.
    let mut w = Writer::new();
    let mut zero = 0u32;
    w.u32(&mut zero); // max_factor = 0
    w.u32(&mut zero); // 0 buckets
    let mut bad_pool = SlabPool::exact();
    let mut r = Reader::new(&w.out);
    bad_pool.snap(&mut r);
    assert!(r.failed(), "SlabPool with max_factor == 0 must be rejected");

    let mut w_empty_bucket = Writer::new();
    let (mut one, mut sz) = (1u32, 64u32);
    w_empty_bucket.u32(&mut one); // max_factor = 1
    w_empty_bucket.u32(&mut one); // 1 bucket
    w_empty_bucket.u32(&mut sz); // bucket size 64
    w_empty_bucket.u32(&mut zero); // 0 addrs (invalid empty bucket)
    let mut r = Reader::new(&w_empty_bucket.out);
    bad_pool.snap(&mut r);
    assert!(r.failed(), "SlabPool with empty bucket must be rejected");

    let mut w_zero_addr = Writer::new();
    w_zero_addr.u32(&mut one); // max_factor = 1
    w_zero_addr.u32(&mut one); // 1 bucket
    w_zero_addr.u32(&mut sz); // bucket size 64
    w_zero_addr.u32(&mut one); // 1 addr
    w_zero_addr.u32(&mut zero); // addr == 0 (invalid)
    let mut r = Reader::new(&w_zero_addr.out);
    bad_pool.snap(&mut r);
    assert!(r.failed(), "SlabPool with addr == 0 must be rejected");

    let mut w_dup_addr = Writer::new();
    let (mut two, mut addr_dup) = (2u32, 0x2000_0200u32);
    w_dup_addr.u32(&mut one); // max_factor = 1
    w_dup_addr.u32(&mut one); // 1 bucket
    w_dup_addr.u32(&mut sz); // bucket size 64
    w_dup_addr.u32(&mut two); // 2 addrs
    w_dup_addr.u32(&mut addr_dup);
    w_dup_addr.u32(&mut addr_dup); // duplicate addr (invalid)
    let mut r = Reader::new(&w_dup_addr.out);
    bad_pool.snap(&mut r);
    assert!(r.failed(), "SlabPool with duplicate addr must be rejected");

    // 4. Corrupt ComTable (desynced objects/faces and zero/overlapping free_slots) is rejected.
    let mut w_desync = Writer::new();
    let (mut addr, mut refs, mut app) = (0x2000_0100u32, 1i32, 0u32);
    let mut iface = "IDirectDraw".to_owned();
    let mut tag_none = 0u8;
    w_desync.u32(&mut one); // 1 object
    w_desync.u32(&mut addr);
    w_desync.string(&mut iface);
    w_desync.i32(&mut refs);
    w_desync.u32(&mut addr); // identity = addr
    w_desync.u32(&mut app);
    w_desync.u8(&mut tag_none); // ComState::None
    w_desync.u32(&mut zero); // 0 faces (desynced!)
    w_desync.u32(&mut zero); // 0 free_slots
    w_desync.u32(&mut zero); // 0 vtables
    let mut bad_com = ComTable::default();
    let mut r = Reader::new(&w_desync.out);
    bad_com.snap(&mut r);
    assert!(
        r.failed(),
        "ComTable with desynced objects/faces must be rejected"
    );

    let mut w_bad_free = Writer::new();
    w_bad_free.u32(&mut zero); // 0 objects
    w_bad_free.u32(&mut zero); // 0 faces
    w_bad_free.u32(&mut one); // 1 free_slot
    w_bad_free.u32(&mut zero); // slot == 0 (invalid)
    w_bad_free.u32(&mut zero); // 0 vtables
    let mut r = Reader::new(&w_bad_free.out);
    bad_com.snap(&mut r);
    assert!(r.failed(), "ComTable with zero free_slot must be rejected");
}
