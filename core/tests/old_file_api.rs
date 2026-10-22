//! The old file API LEGO Island reads its scripts and world through (#116):
//! `OpenFile`, `_hread`, `_hwrite`, `_llseek` and `_lclose`, with
//! `GetFileSize` and `SetEndOfFile`, over the handles `CreateFileA` makes; and
//! `MoveFileA`, which moves its saves from slot to slot (#125); and the CD
//! read through its Joliet tree, with the walls it stops at (#144).
//!
//! Each is called across the boundary by a guest of the test's own making, as
//! LEGO1's `MxIOInfo` calls them, so what is held is what a title would be
//! answered: its arguments taken off the stack as Windows takes them, and
//! every answer and `GetLastError` after it read back out of guest memory.

use lego_legacy_core::cli::Trace;
use lego_legacy_core::cpu::ESP;
use lego_legacy_core::disc;
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::{Machine, Outcome, STACK_TOP};
use lego_legacy_core::roster::{LEGO_CHESS, LEGO_ISLAND};
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::win32::{Contents, Object};
use lego_legacy_core::{testkit, Title};
use std::path::PathBuf;

const TITLE: Title = Title::STUB;

/// Where the guest's code is, where it keeps each call's answer and what
/// `GetLastError` said after it, and where what it passes is.
const CODE: u32 = 0x0040_0000;
const ANSWERS: u32 = 0x0030_0000;
const DATA: u32 = 0x0031_0000;

const OF_READ: u32 = 0x0000;
const OF_READWRITE: u32 = 0x0002;
const OF_SHARE_EXCLUSIVE: u32 = 0x0010;
const OF_SHARE_DENY_WRITE: u32 = 0x0020;
const OF_CREATE: u32 = 0x1000;
const OF_EXIST: u32 = 0x4000;
const FILE_BEGIN: u32 = 0;
const FILE_CURRENT: u32 = 1;
const FILE_END: u32 = 2;
const HFILE_ERROR: u32 = 0xffff_ffff;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_WRITE_PROTECT: u32 = 19;
const ERROR_SHARING_VIOLATION: u32 = 32;
const ERROR_NEGATIVE_SEEK: u32 = 131;

/// What a call passes: a value, or what an earlier call answered.
#[derive(Clone, Copy)]
enum Arg {
    Value(u32),
    Answer(usize),
}
use Arg::{Answer, Value};

/// A guest that makes one call after another across the boundary, keeps what
/// each answered, and exits.
struct Guest {
    machine: Machine,
    host: Headless,
    code: Vec<u8>,
    calls: usize,
    data: u32,
}

impl Guest {
    fn new() -> Self {
        std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
        let mut host = Headless::new();
        let mut machine = Machine::new(&TITLE, &mut host).expect("a machine");
        machine.state.current_directory = TITLE.install_dir.to_owned();
        Self {
            machine,
            host,
            code: Vec::new(),
            calls: 0,
            data: DATA,
        }
    }

    /// Somewhere for what the guest writes, empty.
    fn profile(&mut self, name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("old-file-api-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        self.machine.tree.set_profile(dir.clone());
        dir
    }

    /// Bytes the guest can pass, and where they are.
    fn put(&mut self, bytes: &[u8]) -> u32 {
        let at = self.data;
        self.machine.arena.write(at, bytes).expect("in the arena");
        self.data += (bytes.len() as u32 + 1).next_multiple_of(16);
        at
    }

    fn text(&mut self, text: &str) -> u32 {
        self.put(format!("{text}\0").as_bytes())
    }

    fn buffer(&mut self, len: usize) -> u32 {
        self.put(&vec![0xee; len])
    }

    /// Call one of KERNEL32's and keep its answer; which call it is.
    fn call(&mut self, name: &str, args: &[Arg]) -> usize {
        for arg in args.iter().rev() {
            match *arg {
                // push <value>
                Value(value) => {
                    self.code.push(0x68);
                    self.code.extend_from_slice(&value.to_le_bytes());
                }
                // push [<what call k answered>]
                Answer(k) => {
                    self.code.extend_from_slice(&[0xff, 0x35]);
                    self.code
                        .extend_from_slice(&(ANSWERS + 8 * k as u32).to_le_bytes());
                }
            }
        }
        let n = self.calls;
        self.cross(name, ANSWERS + 8 * n as u32);
        self.cross("GetLastError", ANSWERS + 8 * n as u32 + 4);
        self.calls += 1;
        n
    }

    /// `mov eax, <the name's place on the boundary>; call eax; mov [keep], eax`
    fn cross(&mut self, name: &str, keep: u32) {
        let at = self.machine.boundary.bind_named("KERNEL32", name);
        self.code.push(0xb8);
        self.code.extend_from_slice(&at.to_le_bytes());
        self.code.extend_from_slice(&[0xff, 0xd0, 0xa3]);
        self.code.extend_from_slice(&keep.to_le_bytes());
    }

    /// Run every call made, then `ExitProcess(0)`, and say how it ended.
    fn run(&mut self) -> Outcome {
        self.call("ExitProcess", &[Value(0)]);
        self.machine
            .arena
            .write(CODE, &self.code)
            .expect("in the arena");
        self.machine.cpu.eip = CODE;
        self.machine.cpu.r[ESP] = STACK_TOP - 16;
        let mut log = RunLog::create("old-file-api", Trace::Off).expect("a run directory");
        let outcome = self.machine.run(&mut self.host, &mut log, 100_000).outcome;
        if outcome == (Outcome::Exited { code: 0 }) {
            assert_eq!(
                self.machine.cpu.r[ESP],
                STACK_TOP - 16 - 8,
                "every call took its own arguments off the stack, as stdcall does, \
                 and ExitProcess's call is all that is left on it"
            );
        }
        outcome
    }

    fn word(&self, at: u32) -> u32 {
        self.machine.arena.read_u32(at).expect("in the arena")
    }

    fn answer(&self, call: usize) -> u32 {
        self.word(ANSWERS + 8 * call as u32)
    }

    /// What a call answered, and what `GetLastError` said after it.
    fn failed(&self, call: usize) -> (u32, u32) {
        (self.answer(call), self.word(ANSWERS + 8 * call as u32 + 4))
    }

    fn bytes(&self, at: u32, len: usize) -> Vec<u8> {
        self.machine
            .arena
            .slice(at, len)
            .expect("in the arena")
            .to_vec()
    }

    /// An `OFSTRUCT`: its length, whether the file is on a fixed disk, the
    /// error, and the path.
    fn ofstruct(&self, at: u32) -> (u8, u8, u16, String) {
        let bytes = self.bytes(at, 136);
        let path = bytes[8..].split(|b| *b == 0).next().unwrap_or_default();
        (
            bytes[0],
            bytes[1],
            u16::from_le_bytes([bytes[2], bytes[3]]),
            String::from_utf8_lossy(path).into_owned(),
        )
    }
}

#[test]
fn a_file_opened_by_its_old_name_reads_seeks_and_closes_as_windows_answers() {
    let mut g = Guest::new();
    g.machine
        .tree
        .declare("C:\\STUB\\DATA\\TEN.BIN", b"0123456789".to_vec());
    let (name, record, buf, high) = (
        g.text("DATA\\TEN.BIN"),
        g.buffer(136),
        g.buffer(16),
        g.buffer(4),
    );
    let file = g.call("OpenFile", &[Value(name), Value(record), Value(OF_READ)]);
    let four = g.call("_hread", &[Answer(file), Value(buf), Value(4)]);
    let here = g.call("_llseek", &[Answer(file), Value(0), Value(FILE_CURRENT)]);
    let back = g.call(
        "_llseek",
        &[Answer(file), Value(-2i32 as u32), Value(FILE_END)],
    );
    let short = g.call("_hread", &[Answer(file), Value(buf + 4), Value(4)]);
    let none = g.call("_hread", &[Answer(file), Value(buf + 8), Value(4)]);
    let past = g.call("_llseek", &[Answer(file), Value(100), Value(FILE_BEGIN)]);
    let beyond = g.call("_hread", &[Answer(file), Value(buf + 8), Value(4)]);
    let before = g.call(
        "_llseek",
        &[Answer(file), Value(-1i32 as u32), Value(FILE_BEGIN)],
    );
    let size = g.call("GetFileSize", &[Answer(file), Value(high)]);
    let write = g.call("_hwrite", &[Answer(file), Value(buf), Value(1)]);
    let end = g.call("SetEndOfFile", &[Answer(file)]);
    let closed = g.call("_lclose", &[Answer(file)]);
    let again = g.call("_lclose", &[Answer(file)]);
    let read_closed = g.call("_hread", &[Answer(file), Value(buf), Value(4)]);
    let seek_closed = g.call("_llseek", &[Answer(file), Value(0), Value(FILE_BEGIN)]);
    let size_closed = g.call("GetFileSize", &[Answer(file), Value(0)]);
    let end_closed = g.call("SetEndOfFile", &[Answer(file)]);
    assert_eq!(g.run(), Outcome::Exited { code: 0 });

    assert_ne!(g.answer(file), HFILE_ERROR);
    assert_eq!(
        g.ofstruct(record),
        (136, 1, 0, "C:\\STUB\\DATA\\TEN.BIN".to_owned()),
        "its length, a fixed disk, no error, and where it was found"
    );
    assert_eq!(g.answer(four), 4);
    assert_eq!(g.bytes(buf, 4), b"0123");
    assert_eq!(g.answer(here), 4);
    assert_eq!(g.answer(back), 8);
    assert_eq!(g.answer(short), 2, "short of the count at the end");
    assert_eq!(g.bytes(buf + 4, 4), b"89\xee\xee");
    assert_eq!(g.answer(none), 0, "and nothing once there");
    assert_eq!(g.answer(past), 100, "past the end is somewhere");
    assert_eq!(g.answer(beyond), 0, "with nothing in it");
    assert_eq!(g.failed(before), (HFILE_ERROR, ERROR_NEGATIVE_SEEK));
    assert_eq!(g.answer(size), 10);
    assert_eq!(g.word(high), 0, "and the high half");
    assert_eq!(g.failed(write), (HFILE_ERROR, ERROR_ACCESS_DENIED));
    assert_eq!(g.failed(end), (0, ERROR_ACCESS_DENIED));
    assert_eq!(g.answer(closed), 0);
    for call in [again, read_closed, seek_closed] {
        assert_eq!(g.failed(call), (HFILE_ERROR, ERROR_INVALID_HANDLE));
    }
    assert_eq!(g.failed(size_closed), (0xffff_ffff, ERROR_INVALID_HANDLE));
    assert_eq!(g.failed(end_closed), (0, ERROR_INVALID_HANDLE));
}

#[test]
fn a_missing_file_a_question_and_a_file_another_handle_will_not_share() {
    let mut g = Guest::new();
    g.machine
        .tree
        .declare("C:\\STUB\\TEN.BIN", b"0123456789".to_vec());
    let (elsewhere, missing_name, ten) = (
        g.text("C:\\ELSEWHERE"),
        g.text("C:\\STUB\\NONE.BIN"),
        g.text("TEN.BIN"),
    );
    let (missing_record, exist_record, denied_record) =
        (g.buffer(136), g.buffer(136), g.buffer(136));
    // A bare name is found where the title started, wherever it is now.
    g.call("SetCurrentDirectoryA", &[Value(elsewhere)]);
    let missing = g.call(
        "OpenFile",
        &[Value(missing_name), Value(missing_record), Value(OF_READ)],
    );
    let absent = g.call(
        "OpenFile",
        &[Value(missing_name), Value(0), Value(OF_EXIST)],
    );
    let exists = g.call(
        "OpenFile",
        &[Value(ten), Value(exist_record), Value(OF_EXIST)],
    );
    // The question left nothing open, so a handle that shares with nobody
    // opens, and then nothing else does.
    let alone = g.call(
        "OpenFile",
        &[Value(ten), Value(0), Value(OF_READ | OF_SHARE_EXCLUSIVE)],
    );
    let second = g.call(
        "OpenFile",
        &[Value(ten), Value(denied_record), Value(OF_READ)],
    );
    let closed = g.call("_lclose", &[Answer(alone)]);
    let reader = g.call(
        "OpenFile",
        &[Value(ten), Value(0), Value(OF_READ | OF_SHARE_DENY_WRITE)],
    );
    let another = g.call("OpenFile", &[Value(ten), Value(0), Value(OF_READ)]);
    let writer = g.call("OpenFile", &[Value(ten), Value(0), Value(OF_READWRITE)]);
    assert_eq!(g.run(), Outcome::Exited { code: 0 });

    assert_eq!(g.failed(missing), (HFILE_ERROR, ERROR_FILE_NOT_FOUND));
    assert_eq!(
        g.ofstruct(missing_record),
        (136, 1, 2, "C:\\STUB\\NONE.BIN".to_owned())
    );
    assert_eq!(g.failed(absent), (HFILE_ERROR, ERROR_FILE_NOT_FOUND));
    assert_eq!(g.answer(exists), 1, "TRUE, not a handle");
    assert_eq!(
        g.ofstruct(exist_record),
        (136, 1, 0, "C:\\STUB\\TEN.BIN".to_owned())
    );
    assert_ne!(g.answer(alone), HFILE_ERROR);
    assert_eq!(g.failed(second), (HFILE_ERROR, ERROR_SHARING_VIOLATION));
    assert_eq!(
        g.ofstruct(denied_record).2,
        32,
        "the MS-DOS error is the same"
    );
    assert_eq!(g.answer(closed), 0);
    assert_ne!(g.answer(reader), HFILE_ERROR);
    assert_ne!(g.answer(another), HFILE_ERROR, "it shares reading");
    assert_eq!(
        g.failed(writer),
        (HFILE_ERROR, ERROR_SHARING_VIOLATION),
        "and not writing"
    );
}

#[test]
fn a_file_made_by_its_old_name_is_written_cut_and_kept() {
    let mut g = Guest::new();
    let profile = g.profile("writing");
    let (name, disc, disc_record) = (
        g.text("C:\\STUB\\SAVE.GSI"),
        g.text("D:\\SAVE.GSI"),
        g.buffer(136),
    );
    let (text, buf) = (g.put(b"abcdefZ"), g.buffer(16));
    let file = g.call("OpenFile", &[Value(name), Value(0), Value(OF_CREATE)]);
    let wrote = g.call("_hwrite", &[Answer(file), Value(text), Value(6)]);
    let past = g.call("_llseek", &[Answer(file), Value(10), Value(FILE_BEGIN)]);
    let z = g.call("_hwrite", &[Answer(file), Value(text + 6), Value(1)]);
    let longer = g.call("GetFileSize", &[Answer(file), Value(0)]);
    g.call("_llseek", &[Answer(file), Value(3), Value(FILE_BEGIN)]);
    let cut = g.call("SetEndOfFile", &[Answer(file)]);
    let shorter = g.call("GetFileSize", &[Answer(file), Value(0)]);
    // Writing nothing is MS-DOS's way of saying where the file ends.
    g.call("_llseek", &[Answer(file), Value(2), Value(FILE_BEGIN)]);
    let nothing = g.call("_hwrite", &[Answer(file), Value(text), Value(0)]);
    let shortest = g.call("GetFileSize", &[Answer(file), Value(0)]);
    // A file being made is read through as well as written.
    g.call("_llseek", &[Answer(file), Value(0), Value(FILE_BEGIN)]);
    let back = g.call("_hread", &[Answer(file), Value(buf), Value(16)]);
    let closed = g.call("_lclose", &[Answer(file)]);
    let reopened = g.call("OpenFile", &[Value(name), Value(0), Value(OF_READWRITE)]);
    let again = g.call("_hread", &[Answer(reopened), Value(buf + 8), Value(16)]);
    let refused = g.call(
        "OpenFile",
        &[Value(disc), Value(disc_record), Value(OF_CREATE)],
    );
    assert_eq!(g.run(), Outcome::Exited { code: 0 });

    assert_ne!(g.answer(file), HFILE_ERROR);
    assert_eq!(g.answer(wrote), 6);
    assert_eq!(g.answer(past), 10);
    assert_eq!(g.answer(z), 1);
    assert_eq!(
        g.answer(longer),
        11,
        "the write past the end made it that long"
    );
    assert_eq!((g.answer(cut), g.answer(shorter)), (1, 3));
    assert_eq!((g.answer(nothing), g.answer(shortest)), (0, 2));
    assert_eq!(g.answer(back), 2);
    assert_eq!(g.bytes(buf, 2), b"ab");
    assert_eq!(g.answer(closed), 0);
    assert_eq!(
        std::fs::read(profile.join("C/STUB/SAVE.GSI"))
            .ok()
            .as_deref(),
        Some(&b"ab"[..]),
        "closing it kept it in the profile"
    );
    assert_eq!(g.answer(again), 2, "where the next open finds it");
    assert_eq!(g.bytes(buf + 8, 2), b"ab");
    assert_eq!(g.failed(refused), (HFILE_ERROR, ERROR_WRITE_PROTECT));
    assert_eq!(
        g.ofstruct(disc_record),
        (136, 0, 19, "D:\\SAVE.GSI".to_owned()),
        "a CD-ROM is not a fixed disk, and is not written to"
    );
}

const ERROR_ALREADY_EXISTS: u32 = 183;

/// LEGO Island moves every player's save down a slot when a new name signs its
/// book, `G0.GS` to `G1.GS` and on (#125): a slot with no save in it fails to
/// move, and so does one whose next slot is taken, and the game carries on.
#[test]
fn a_save_moves_to_a_free_slot_and_nowhere_else() {
    let mut g = Guest::new();
    let profile = g.profile("moving");
    g.machine
        .tree
        .declare("C:\\STUB\\TEN.BIN", b"0123456789".to_vec());
    for (slot, bytes) in [("G0", "zero"), ("G2", "two")] {
        g.machine
            .tree
            .write(&format!("C:\\STUB\\{slot}.GS"), bytes.as_bytes())
            .expect("the profile takes it");
    }
    let [zero, one, two, ten] = ["G0.GS", "G1.GS", "C:\\STUB\\G2.GS", "TEN.BIN"].map(|n| g.text(n));
    let moved = g.call("MoveFileA", &[Value(zero), Value(one)]);
    let gone = g.call("MoveFileA", &[Value(zero), Value(one)]);
    let taken = g.call("MoveFileA", &[Value(two), Value(one)]);
    let pressing = g.call("MoveFileA", &[Value(two), Value(ten)]);
    assert_eq!(g.run(), Outcome::Exited { code: 0 });

    assert_eq!(g.answer(moved), 1);
    assert_eq!(g.failed(gone), (0, ERROR_FILE_NOT_FOUND), "it has gone");
    assert_eq!(g.failed(taken), (0, ERROR_ALREADY_EXISTS));
    assert_eq!(
        g.failed(pressing),
        (0, ERROR_ALREADY_EXISTS),
        "the pressing's own file is there too"
    );
    let read = |g: &mut Guest, slot: &str| g.machine.tree.read(&format!("c:/stub/{slot}.gs"));
    assert_eq!(read(&mut g, "G0"), None);
    assert_eq!(read(&mut g, "G1").as_deref(), Some(&b"zero"[..]));
    assert_eq!(read(&mut g, "G2").as_deref(), Some(&b"two"[..]));
    assert!(
        profile.join("C/STUB/G1.GS").is_file(),
        "in the profile, where the next run finds it"
    );
}

/// What the declared machine cannot move as Windows would is a wall: a file
/// a handle holds open, the pressing's own, and anything onto the disc.
#[test]
fn a_move_the_machine_cannot_make_is_a_wall() {
    for (from, to, open) in [
        ("C:\\STUB\\G0.GS", "C:\\STUB\\G1.GS", true),
        ("C:\\STUB\\TEN.BIN", "C:\\STUB\\G1.GS", false),
        ("C:\\STUB\\G0.GS", "D:\\G1.GS", false),
    ] {
        let mut g = Guest::new();
        g.profile("not-moving");
        g.machine
            .tree
            .declare("C:\\STUB\\TEN.BIN", b"0123456789".to_vec());
        g.machine
            .tree
            .write("C:\\STUB\\G0.GS", b"zero")
            .expect("the profile takes it");
        let (named, onto) = (g.text(from), g.text(to));
        if open {
            g.call("OpenFile", &[Value(named), Value(0), Value(OF_READ)]);
        }
        g.call("MoveFileA", &[Value(named), Value(onto)]);
        let outcome = g.run();
        let wall = format!("MoveFileA: {from} to {to}");
        assert!(
            matches!(&outcome, Outcome::Wall { what } if what.contains(&wall)),
            "{outcome}"
        );
    }
}

#[test]
fn a_style_that_asks_for_more_than_opening_a_file_is_a_wall() {
    // OF_PARSE, OF_DELETE, OF_VERIFY, OF_PROMPT and OF_REOPEN, an access mode
    // and a sharing mode the documentation does not have.
    for style in [0x0100, 0x0200, 0x0400, 0x2000, 0x8000, 0x0003, 0x0050] {
        let mut g = Guest::new();
        let name = g.text("C:\\STUB\\TEN.BIN");
        g.call("OpenFile", &[Value(name), Value(0), Value(style)]);
        let outcome = g.run();
        assert!(
            matches!(&outcome, Outcome::Wall { what } if what.contains("OpenFile")),
            "style {style:#06x} ended {outcome}"
        );
    }
}

/// On LEGO Island's own disc, the probe LEGO1 makes for its world (RVA
/// `0x66b40`) finds it where `DATA\disk` was installed and not on the CD,
/// which is what faulted the old tree a billion instructions later; and
/// `Isle\ISLE.SI`, 157 MB, is read where it lies rather than held whole.
#[test]
fn island_s_world_is_found_and_its_biggest_script_is_read_where_it_lies() {
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

    let mut g = Guest::new();
    g.machine
        .tree
        .set_copied(&mut iso, "DATA/disk", TITLE.install_dir)
        .expect("DATA/disk is copied");
    g.machine.tree.set_disc(iso);
    let (installed, on_cd, isle) = (
        g.text("C:\\STUB\\lego\\data\\world.wdb"),
        g.text("D:\\lego\\data\\world.wdb"),
        g.text("D:\\lego\\scripts\\isle\\isle.si"),
    );
    let (installed_record, cd_record, isle_record, buf) =
        (g.buffer(136), g.buffer(136), g.buffer(136), g.buffer(32));
    let world = g.call(
        "OpenFile",
        &[Value(installed), Value(installed_record), Value(OF_EXIST)],
    );
    let cd = g.call(
        "OpenFile",
        &[Value(on_cd), Value(cd_record), Value(OF_EXIST)],
    );
    let file = g.call(
        "OpenFile",
        &[Value(isle), Value(isle_record), Value(OF_READ)],
    );
    let size = g.call("GetFileSize", &[Answer(file), Value(0)]);
    let first = g.call("_hread", &[Answer(file), Value(buf), Value(16)]);
    let end = g.call(
        "_llseek",
        &[Answer(file), Value(-4i32 as u32), Value(FILE_END)],
    );
    let last = g.call("_hread", &[Answer(file), Value(buf + 16), Value(16)]);
    assert_eq!(g.run(), Outcome::Exited { code: 0 });

    assert_eq!(g.answer(world), 1, "the world is where it was installed");
    assert_eq!(g.ofstruct(installed_record).1, 1, "on a fixed disk");
    assert_eq!(g.answer(cd), HFILE_ERROR, "and the CD has none");
    assert_ne!(g.ofstruct(cd_record).2, 0);
    assert_ne!(g.answer(file), HFILE_ERROR);
    assert_eq!(
        g.ofstruct(isle_record),
        (136, 0, 0, "D:\\lego\\scripts\\isle\\isle.si".to_owned())
    );
    assert_eq!(g.answer(size), len);
    assert_eq!((g.answer(first), g.bytes(buf, 16)), (16, head));
    assert_eq!(g.answer(end), len - 4);
    assert_eq!(
        (g.answer(last), g.bytes(buf + 16, 4)),
        (4, tail),
        "short at the end"
    );
    // What the open handle holds is where the script lies, not its bytes.
    let Some(Object::File { contents, .. }) = g.machine.state.object(g.answer(file)) else {
        panic!("the handle is a file");
    };
    assert_eq!(*contents, Contents::Lies { start, len });
}

/// A search of the CD is a wall, for every title, until a title asks for one:
/// none does yet, and what Windows answers for a disc read through its Joliet
/// tree is not known (#144). A pattern relative to a directory on the CD is a
/// search of it too, and a search of the declared disc still answers.
#[test]
fn a_search_of_the_cd_is_a_wall() {
    for (current, pattern) in [
        ("C:\\STUB", "D:\\*"),
        ("C:\\STUB", "D:*.bmp"),
        ("D:\\INTRO", "*.bmp"),
    ] {
        let mut g = Guest::new();
        g.machine.state.current_directory = current.to_owned();
        let (named, found) = (g.text(pattern), g.buffer(0x140));
        g.call("FindFirstFileA", &[Value(named), Value(found)]);
        let outcome = g.run();
        assert!(
            matches!(&outcome, Outcome::Wall { what } if what.contains("FindFirstFileA")),
            "{pattern} from {current} ended {outcome}"
        );
    }
    let mut g = Guest::new();
    let (named, found) = (g.text("C:\\STUB\\*.BIN"), g.buffer(0x140));
    let search = g.call("FindFirstFileA", &[Value(named), Value(found)]);
    assert_eq!(g.run(), Outcome::Exited { code: 0 });
    assert_eq!(g.failed(search), (u32::MAX, ERROR_FILE_NOT_FOUND));
}

/// After its intro LEGO Chess opens `D:\intro\Chess Bumper01.bmp`, a name only
/// its disc's Joliet tree has: the primary tree calls the same file
/// `INTRO/CHESSB~1.BMP` (#144). On the title's own rip, read through its
/// Joliet tree, the long name opens as the title opens it, and the 8.3 name
/// stops the run at a wall naming it.
#[test]
fn lego_chess_s_bumper_opens_by_its_long_name_on_its_own_disc() {
    // The pressing #142 takes, read here for its Joliet tree alone.
    const CHESS: EntryImage = LEGO_CHESS.image;
    if !testkit::rips(&CHESS) {
        return;
    }
    let pressing = || disc::open_pressing(CHESS.disc_dir, CHESS.pressing).expect("the pressing");
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
        let mut g = Guest::new();
        g.machine.tree.set_disc(iso);
        let named = g.text(path);
        // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING.
        let file = g.call(
            "CreateFileA",
            &[
                Value(named),
                Value(0x8000_0000),
                Value(1),
                Value(0),
                Value(3),
                Value(0),
                Value(0),
            ],
        );
        let size = g.call("GetFileSize", &[Answer(file), Value(0)]);
        let outcome = g.run();
        if opens {
            assert_eq!(outcome, Outcome::Exited { code: 0 });
            assert_eq!(g.answer(size), 196_664);
        } else {
            assert!(
                matches!(&outcome, Outcome::Wall { what } if what.contains(short) && what.contains("q2")),
                "{outcome}"
            );
        }
    }
}
