//! Real-disc Win32 shim tests migrated from `core/tests/{old_file_api,shims}.rs` (#407).
//!
//! Keeps `core/tests/` 100% free of concrete title discs while preserving every
//! real-disc `OpenFile`, Joliet, `AVIFIL32`, and `MSACM32` test assertion.

use lego_legacy_catalog::{LEGO_CHESS, LEGO_ISLAND};
use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::Stop;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::Machine;
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::win32::{self, Contents, Ctx, Object};
use lego_legacy_core::{disc, hash, testkit, Title};

const DATA: u32 = 0x0050_0000;
const OUT: u32 = 0x0050_1000;
const ADPCM: u32 = 0x0060_0000;
const DECODED: u32 = 0x0068_0000;

fn machine() -> Machine {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    let mut m = Machine::new(&Title::STUB, &mut Headless::new()).expect("a machine");
    m.state.current_directory = Title::STUB.install_dir.to_owned();
    m
}

fn call(machine: &mut Machine, dll: &str, name: &str, args: &[u32]) -> Result<u32, Stop> {
    let shim = win32::find(dll, name).unwrap_or_else(|| panic!("{dll}!{name} is written"));
    let mut log = RunLog::create("catalog-shim", Trace::Off).expect("a run directory");
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

fn words(xs: &[u32]) -> Vec<u8> {
    xs.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn dib_header(width: u32, height: u32, bits: u32, image: u32) -> Vec<u8> {
    [
        words(&[40, width, height, 1 | bits << 16, 0, image]),
        vec![0; 16],
    ]
    .concat()
}

fn ofstruct(m: &Machine, at: u32) -> (u8, u8, u16, String) {
    let bytes = m.arena.slice(at, 136).expect("in the arena");
    let path = bytes[8..].split(|b| *b == 0).next().unwrap_or_default();
    (
        bytes[0],
        bytes[1],
        u16::from_le_bytes([bytes[2], bytes[3]]),
        String::from_utf8_lossy(path).into_owned(),
    )
}

#[test]
fn island_s_world_is_found_and_its_biggest_script_is_read_where_it_lies() {
    const OF_READ: u32 = 0x0000;
    const OF_EXIST: u32 = 0x4000;
    const FILE_END: u32 = 2;
    const HFILE_ERROR: u32 = 0xffff_ffff;
    let island = &LEGO_ISLAND.image;
    if !testkit::rips(island) {
        return;
    }
    let mut iso = disc::open_pressing(island.disc_dir, island.pressing).expect("the pressing");
    let script = "LEGO/SCRIPTS/ISLE/ISLE.SI";
    let (start, len) = iso.lies(script).expect("the script is on the disc");
    assert!(len > 150_000_000, "{len} bytes");
    let head = iso.read_range(script, 0, 16).expect("it reads");
    let tail = iso.read_range(script, len - 4, 4).expect("it reads");

    let mut m = machine();
    m.tree
        .set_copied(&mut iso, "DATA/disk", Title::STUB.install_dir)
        .expect("DATA/disk is copied");
    m.tree.set_disc(iso);
    let (installed, on_cd, isle) = (DATA, DATA + 0x100, DATA + 0x200);
    let (installed_record, cd_record, isle_record, buf) =
        (DATA + 0x300, DATA + 0x400, DATA + 0x500, DATA + 0x600);
    m.arena
        .write(installed, b"C:\\STUB\\lego\\data\\world.wdb\0")
        .unwrap();
    m.arena
        .write(on_cd, b"D:\\lego\\data\\world.wdb\0")
        .unwrap();
    m.arena
        .write(isle, b"D:\\lego\\scripts\\isle\\isle.si\0")
        .unwrap();

    assert_eq!(
        call(
            &mut m,
            "KERNEL32",
            "OpenFile",
            &[installed, installed_record, OF_EXIST]
        ),
        Ok(1),
        "the world is where it was installed"
    );
    assert_eq!(ofstruct(&m, installed_record).1, 1, "on a fixed disk");
    assert_eq!(
        call(
            &mut m,
            "KERNEL32",
            "OpenFile",
            &[on_cd, cd_record, OF_EXIST]
        ),
        Ok(HFILE_ERROR),
        "and the CD has none"
    );
    assert_ne!(ofstruct(&m, cd_record).2, 0);

    let file = call(
        &mut m,
        "KERNEL32",
        "OpenFile",
        &[isle, isle_record, OF_READ],
    )
    .unwrap();
    assert_ne!(file, HFILE_ERROR);
    assert_eq!(
        ofstruct(&m, isle_record),
        (136, 0, 0, "D:\\lego\\scripts\\isle\\isle.si".to_owned())
    );
    assert_eq!(call(&mut m, "KERNEL32", "GetFileSize", &[file, 0]), Ok(len));
    assert_eq!(call(&mut m, "KERNEL32", "_hread", &[file, buf, 16]), Ok(16));
    assert_eq!(m.arena.slice(buf, 16).unwrap(), head);
    assert_eq!(
        call(
            &mut m,
            "KERNEL32",
            "_llseek",
            &[file, -4i32 as u32, FILE_END]
        ),
        Ok(len - 4)
    );
    assert_eq!(
        call(&mut m, "KERNEL32", "_hread", &[file, buf + 16, 16]),
        Ok(4),
        "short at the end"
    );
    assert_eq!(m.arena.slice(buf + 16, 4).unwrap(), tail);
    let Some(Object::File { contents, .. }) = m.state.object(file) else {
        panic!("the handle is a file");
    };
    assert_eq!(*contents, Contents::Lies { start, len });
}

#[test]
fn lego_chess_s_bumper_opens_by_its_long_name_on_its_own_disc() {
    let chess = &LEGO_CHESS.image;
    if !testkit::rips(chess) {
        return;
    }
    let pressing = || disc::open_pressing(chess.disc_dir, chess.pressing).expect("the pressing");
    let (bumper, short) = ("intro/Chess Bumper01.bmp", "INTRO/CHESSB~1.BMP");
    let mut iso = pressing();
    assert_eq!(
        iso.has(bumper).ok(),
        Some(false),
        "the primary tree has only its 8.3 name"
    );
    let at = iso.lies(short).expect("the 8.3 name is the primary tree's");
    assert_eq!(iso.read_joliet().ok(), Some(true));
    assert_eq!(
        iso.lies(bumper).ok(),
        Some(at),
        "the long name is the same file"
    );
    assert_eq!(at.1, 196_664);
    assert_eq!(
        iso.read_range(bumper, 0, 2).ok().as_deref(),
        Some(&b"BM"[..])
    );
    assert!(iso
        .list("intro")
        .expect("the directory lists")
        .iter()
        .any(|e| e.name == "Chess Bumper01.bmp"));

    for (path, opens) in [
        ("D:\\intro\\Chess Bumper01.bmp", true),
        ("D:\\INTRO\\CHESSB~1.BMP", false),
    ] {
        let mut iso = pressing();
        iso.read_joliet().expect("the descriptors read");
        let mut m = machine();
        m.tree.set_disc(iso);
        m.arena
            .write(DATA, format!("{path}\0").as_bytes())
            .expect("in the arena");
        let opened = call(
            &mut m,
            "KERNEL32",
            "CreateFileA",
            &[DATA, 0x8000_0000, 1, 0, 3, 0, 0],
        );
        if opens {
            let file = opened.expect("opens");
            assert_eq!(m.tree.wall(), None);
            assert_eq!(
                call(&mut m, "KERNEL32", "GetFileSize", &[file, 0]),
                Ok(196_664)
            );
        } else {
            assert_eq!(opened, Ok(0xffff_ffff));
            let wall = m.tree.wall();
            assert!(
                matches!(&wall, Some(what) if what.contains(short) && what.contains("q2")),
                "{wall:?}"
            );
        }
    }
}

#[test]
fn the_intro_s_movies_answer_their_streams_off_the_disc() {
    let image = &LEGO_CHESS.image;
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

#[test]
fn the_logo_s_frames_are_bottom_up_555_dibs_decoded_in_order() {
    const WANTED: u32 = DATA + 0x200;
    let image = &LEGO_CHESS.image;
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

#[test]
fn the_intro_movie_decodes_to_its_last_picture_and_its_sound_to_its_end() {
    const WANTED: u32 = DATA + 0x200;
    const BUFFER: u32 = 0x0060_0000;
    let image = &LEGO_CHESS.image;
    if !testkit::rips(image) {
        return;
    }
    let mut m = machine();
    m.tree
        .set_disc(disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens"));
    m.arena
        .write(DATA, b"D:\\INTRO\\INTRO.AVI\0")
        .expect("in the arena");
    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
        Ok(0)
    );
    let file = m.arena.read_u32(OUT).expect("in the arena");
    let stream = |m: &mut Machine, n: u32| {
        assert_eq!(
            call(m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, n]),
            Ok(0)
        );
        m.arena.read_u32(OUT).expect("in the arena")
    };
    let (video, sound) = (stream(&mut m, 0), stream(&mut m, 1));

    m.arena
        .write(WANTED, &dib_header(320, 200, 16, 128_000))
        .expect("in the arena");
    let getter = call(
        &mut m,
        "AVIFIL32",
        "AVIStreamGetFrameOpen",
        &[video, WANTED],
    )
    .expect("a getter");
    let bits = |m: &mut Machine, n: u32| {
        let dib = call(m, "AVIFIL32", "AVIStreamGetFrame", &[getter, n]).expect("answered");
        assert_ne!(dib, 0, "frame {n}");
        hash::sha256_hex(m.arena.slice(dib + 40, 128_000).expect("in the arena"))
    };
    assert_eq!(
        bits(&mut m, 150),
        "4c882e29d0e67c99f02560f2ba01747ca9b6cf18230f8d21e731aa1ddbc7250f"
    );
    assert_eq!(
        bits(&mut m, 498),
        "5f3506c8cbc653dd2f620fdb149d945d967546ada494df315b4ec4decfe4de67"
    );

    assert_eq!(
        call(&mut m, "AVIFIL32", "AVIStreamLength", &[sound]),
        Ok(733_515)
    );
    let (mut at, mut bytes) = (0u32, 0u32);
    loop {
        let answer = call(
            &mut m,
            "AVIFIL32",
            "AVIStreamRead",
            &[sound, at, 1_470, BUFFER, 5_880, OUT, OUT + 4],
        );
        assert_eq!(answer, Ok(0), "at sample {at}");
        let read = m.arena.read_u32(OUT + 4).expect("in the arena");
        bytes += m.arena.read_u32(OUT).expect("in the arena");
        if read == 0 {
            break;
        }
        at += read;
    }
    assert_eq!((at, bytes), (733_515, 2_934_060));
}

#[test]
fn the_advisors_decode_through_avifile_off_the_disc() {
    const WANTED: u32 = DATA + 0x200;
    const DIB_BYTES: usize = 128 * 128 * 3;
    let image = &LEGO_CHESS.image;
    if !testkit::rips(image) {
        return;
    }
    let mut iso = disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens");
    let main_z = iso.read("MAIN.Z").expect("MAIN.Z reads");
    let archive = lego_legacy_core::is3::Archive::open(&main_z).expect("MAIN.Z opens");

    type Advisor<'a> = (&'a str, u32, [(u32, &'a str); 3]);
    let cases: &[Advisor<'_>] = &[
        (
            "admiral.avi",
            153,
            [
                (
                    0,
                    "f96182fc3fae959295c6bcced332aa32b9736e23fd6486ac55b5134079227d28",
                ),
                (
                    76,
                    "7b4011e870440a66cde14544982ebd62d7a753a8233d7b2819659d4848407fdd",
                ),
                (
                    152,
                    "c8e1b240502482f1566e14c10059e55d0ae2fcfa2b208bd873c970f44584cfb2",
                ),
            ],
        ),
        (
            "baddude.avi",
            145,
            [
                (
                    0,
                    "79ad26b8de19f87002c1b0a2ea9a83c35adb0ded1c486d8d803d5c8f7a318534",
                ),
                (
                    72,
                    "f36505d94117af95109d674ec987d1e1d7be3e243841c0983503402c6e661497",
                ),
                (
                    144,
                    "0074e5d59ac5fb96d6fb4c16d53b2bb6ed3b179fa314d73d8f9c8fe207a4a828",
                ),
            ],
        ),
        (
            "blueking.avi",
            161,
            [
                (
                    0,
                    "c01f3cd0c15dddc8948fc46eb88be2e50608a84a7b67ca1acd127ede8bd247f3",
                ),
                (
                    80,
                    "b615bd00f387749044cc0f6edf13690c622a50d3a9daed0824abb5eaecbad176",
                ),
                (
                    160,
                    "45c79762935a6ac1c952f05e15b3abf2f3d30ad9a6ccee1c40d028cdfee5e187",
                ),
            ],
        ),
        (
            "pirate.avi",
            137,
            [
                (
                    0,
                    "98655a829137cf8f6d8813ba5bc7c7413195bfee0590d35322a7816bd8c44f86",
                ),
                (
                    68,
                    "8b409c747dc900dfb543bf9248c92f8032c103f930a15c347b259b1932af29a2",
                ),
                (
                    136,
                    "157a9a7dc0b7f62842bf04e40e8ecebbb816bbe7c2cfacb03838b967f4ac5c16",
                ),
            ],
        ),
        (
            "redking.avi",
            161,
            [
                (
                    0,
                    "2e6b00010cd81a3f825a7ba27c65f741c7cc36ff3e82746ae66493c3d850fafe",
                ),
                (
                    80,
                    "43f10d05be00a5779b68d9bdacced3c628e38922d7dcc29a32b40d8c9b173da2",
                ),
                (
                    160,
                    "7f9a91b54cf50570876a72528cd860bcaf822135655f3accfeeabc0a6d0ea20f",
                ),
            ],
        ),
        (
            "sherriff.avi",
            155,
            [
                (
                    0,
                    "ec7ba76aef68996c7bd46708f067e2d78072e587482e1449644437629e836444",
                ),
                (
                    77,
                    "025df27694b1653b06925a0ec529c6dd1764316a0cb18d576737bd759bdbfe4b",
                ),
                (
                    154,
                    "5dcffd67f74855795b2746773b4c940e4f7553a6cef749b6f70987afb75bea5e",
                ),
            ],
        ),
    ];

    let mut m = machine();
    for &(name, frames, pinned) in cases {
        let guest_path = format!("C:\\STUB\\{name}");
        let bytes = archive
            .read(&main_z, &format!("Advisors\\{name}"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        m.tree.declare(&guest_path, bytes);
        m.arena
            .write(DATA, format!("{guest_path}\0").as_bytes())
            .expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIFileOpenA", &[OUT, DATA, 0, 0]),
            Ok(0),
            "{name}"
        );
        let file = m.arena.read_u32(OUT).expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIFileGetStream", &[file, OUT, 0, 0]),
            Ok(0),
            "{name}"
        );
        let video = m.arena.read_u32(OUT).expect("in the arena");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIStreamLength", &[video]),
            Ok(frames),
            "{name}"
        );
        m.arena
            .write(WANTED, &dib_header(128, 128, 24, DIB_BYTES as u32))
            .expect("in the arena");
        let getter = call(
            &mut m,
            "AVIFIL32",
            "AVIStreamGetFrameOpen",
            &[video, WANTED],
        )
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_ne!(getter, 0, "{name}");

        let fetch = |m: &mut Machine, n: u32| -> Vec<u8> {
            let dib = call(m, "AVIFIL32", "AVIStreamGetFrame", &[getter, n]).expect("frame");
            assert_ne!(dib, 0, "{name} frame {n}");
            m.arena
                .slice(dib + 40, DIB_BYTES)
                .expect("in the arena")
                .to_vec()
        };
        for (frame_idx, expected_hash) in pinned {
            let bits = fetch(&mut m, frame_idx);
            assert_eq!(
                hash::sha256_hex(&bits),
                expected_hash,
                "{name} frame {frame_idx}"
            );
            let key_pixels = bits
                .as_chunks::<3>()
                .0
                .iter()
                .filter(|p| **p == [0x90, 0x80, 0x80])
                .count();
            assert!(
                key_pixels > 1_000 && key_pixels < 16_384,
                "{name} frame {frame_idx}: {key_pixels} key pixels"
            );
        }
        let looped = fetch(&mut m, 0);
        assert_eq!(hash::sha256_hex(&looped), pinned[0].1, "{name} loop");
        assert_eq!(
            call(&mut m, "AVIFIL32", "AVIStreamGetFrameClose", &[getter]),
            Ok(0),
            "{name}"
        );
    }
}

#[test]
fn a_sound_off_the_disc_decodes_through_acm_as_ffmpeg_decodes_it() {
    let image = &LEGO_CHESS.image;
    if !testkit::rips(image) {
        return;
    }
    let mut iso = disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens");
    let bytes = iso.read("MAIN.Z").expect("the archive reads");
    let archive = lego_legacy_core::is3::Archive::open(&bytes).expect("an InstallShield 3 archive");
    let wav = archive
        .read(&bytes, "sounds\\chimp.wav")
        .expect("it expands");
    let chunk = |id: &[u8]| {
        let at = wav
            .windows(4)
            .position(|w| w == id)
            .expect("the chunk is there");
        let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().expect("four bytes"));
        &wav[at + 8..at + 8 + len as usize]
    };
    let (format, data) = (chunk(b"fmt "), chunk(b"data"));
    assert_eq!(
        (data.len(), u16::from_le_bytes([format[12], format[13]])),
        (15_492, 512)
    );
    let mut m = machine();
    const FROM: u32 = DATA + 0x100;
    const TO: u32 = DATA + 0x200;
    const HEADER: u32 = DATA + 0x300;
    // 16-bit mono 22,050 Hz PCM WAVEFORMATEX (18 bytes).
    let pcm: [u8; 18] = [
        1, 0, 1, 0, 0x22, 0x56, 0, 0, 0x44, 0xac, 0, 0, 2, 0, 16, 0, 0, 0,
    ];
    m.arena.write(FROM, format).expect("in the arena");
    m.arena.write(TO, &pcm).expect("in the arena");
    assert_eq!(
        call(
            &mut m,
            "MSACM32",
            "acmStreamOpen",
            &[OUT, 0, FROM, TO, 0, 0, 0, 0x4],
        ),
        Ok(0)
    );
    let stream = m.arena.read_u32(OUT).expect("in the arena");
    m.arena.write(ADPCM, data).expect("in the arena");
    let from = u32::try_from(data.len()).expect("small");
    assert_eq!(
        call(&mut m, "MSACM32", "acmStreamSize", &[stream, from, OUT, 0]),
        Ok(0)
    );
    let room = m.arena.read_u32(OUT).expect("in the arena");
    assert_eq!(room, 31 * 1012 * 2, "the short block counted whole");
    let mut header = [0u32; 21];
    header[0] = 0x54;
    (header[3], header[4], header[7], header[8]) = (ADPCM, from, DECODED, room);
    m.arena
        .write(HEADER, &words(&header))
        .expect("in the arena");
    for (name, flags) in [("acmStreamPrepareHeader", 0), ("acmStreamConvert", 0x10)] {
        assert_eq!(
            call(&mut m, "MSACM32", name, &[stream, HEADER, flags]),
            Ok(0)
        );
    }
    let used = m.arena.read_u32(HEADER + 0x24).expect("in the arena");
    assert_eq!(used, (30 * 1012 + 252) * 2);
    let decoded = m.arena.slice(DECODED, used as usize).expect("in the arena");
    assert_eq!(
        hash::sha256_hex(decoded),
        "dc8f093547adab95c318016c282b0bba80188140e07aab7a895bc798bdc119b6"
    );
}
