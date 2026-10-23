//! A MIDI stream on the declared machine, without a title (#154): WINMM's
//! `midiStream*` on the MIDI mapper as LEGO Chess calls it, its callback run
//! on WINMM's own player thread at the declared moment the stream reaches
//! each event, and its notes heard in the mix through #155's synth.
//!
//! The guest is a few hand-assembled instructions, as `threads.rs`'s are: a
//! main thread that plays two buffers of `MIDIEVENT`s, written as LEGO Chess's
//! converter (Microsoft's MSTREAM sample) writes a Standard MIDI File's
//! events, and a callback that writes down every message it is called with,
//! refills a buffer from inside the callback, and sets an event once two
//! buffers have come back. The bank is built here: one instrument, a ramp of
//! eight steps of 100, so every note's first frames say where it started.

use lego_legacy_core::cli::Trace;
use lego_legacy_core::dls::{Articulation, Bank, Instrument, Region, Sample, Wave};
use lego_legacy_core::guest::GuestMem;
use lego_legacy_core::host::headless::Headless;
use lego_legacy_core::machine::{Machine, Outcome, STACK_TOP};
use lego_legacy_core::runlog::RunLog;
use lego_legacy_core::sound::frame_at;
use lego_legacy_core::synth::Synth;
use lego_legacy_core::Title;

const STUB: Title = Title::STUB;

/// Where the main thread's code goes, the callback's, and the data.
const MAIN: u32 = 0x0040_0000;
const CALLBACK: u32 = 0x0040_1000;
const DATA: u32 = 0x0050_0000;

/// The data's words: the stream's handle, the device asked for, the event
/// the callback sets, how many messages it has written down, how many
/// refills it has left to make, how many buffers have come back since, and
/// the `MIDIPROPTIMEDIV` the main thread sets.
const HANDLE: u32 = DATA;
const DEVICE: u32 = DATA + 4;
const EVENT: u32 = DATA + 8;
const LOGGED: u32 = DATA + 12;
const REFILLS: u32 = DATA + 16;
const RETURNED: u32 = DATA + 20;
const TIMEDIV: u32 = DATA + 24;
/// What each call answered, a word apiece, in the order made.
const ANSWERS: u32 = DATA + 0x40;
/// What `midiOutUnprepareHeader` answered the callback for each buffer it
/// counted back.
const UNPREPARED: u32 = DATA + 0xc0;
/// What the callback writes down, sixteen bytes a message: the message, its
/// first parameter (the header, or zero), `timeGetTime` as it ran, and the
/// header's `dwOffset` then.
const LOG: u32 = DATA + 0x100;
/// The two `MIDIHDR`s and their buffers, and the mapper's caps.
const HEADERS: [u32; 2] = [DATA + 0x400, DATA + 0x440];
const BUFFERS: [u32; 2] = [DATA + 0x800, DATA + 0x900];
const CAPS: u32 = DATA + 0xa00;

const MIDI_MAPPER: u32 = 0xffff_ffff;
const CALLBACK_FUNCTION: u32 = 0x0003_0000;
const INSTANCE: u32 = 0x4caf_e5a8;
const MIDIPROP_SET_TIMEDIV: u32 = 0x8000_0001;
const MOM_OPEN: u32 = 0x3c7;
const MOM_CLOSE: u32 = 0x3c8;
const MOM_DONE: u32 = 0x3c9;
const MOM_POSITIONCB: u32 = 0x3ca;
const MHDR_DONE: u32 = 1;
const MHDR_PREPARED: u32 = 2;
const MHDR_INQUEUE: u32 = 4;
const MHDR_ISSTRM: u32 = 8;
const MMSYSERR_BADDEVICEID: u32 = 2;
const MMSYSERR_INVALPARAM: u32 = 11;
const MIDIERR_UNPREPARED: u32 = 64;
const MIDIERR_STILLPLAYING: u32 = 65;

/// A `MIDIEVENT` of the stream, as MSTREAM writes one: its delta in ticks,
/// the stream id it leaves zero, and the event.
fn event(delta: u32, event: u32) -> [u32; 3] {
    [delta, 0, event]
}
const MEVT_TEMPO: u32 = 0x0100_0000;
const MEVT_F_CALLBACK: u32 = 0x4000_0000;

/// The two buffers, at 192 ticks a quarter note, as LEGO Chess's tunes are
/// written. The first sets the tempo LEGO Chess's tunes have, a million
/// microseconds a quarter note, so 96 ticks are half a second; then channel
/// 1's volume, which MSTREAM marks for a callback, and its pan, hard left,
/// so the ramp is heard as it is on the left; then a note for 96 ticks. The
/// second waits 96 ticks and plays the note again for 96.
fn buffers() -> [Vec<[u32; 3]>; 2] {
    let (on, off) = (0x7f_3c_90, 0x00_3c_80);
    [
        vec![
            event(0, MEVT_TEMPO | 1_000_000),
            event(0, MEVT_F_CALLBACK | 0x7f_07_b0),
            event(0, 0x00_0a_b0),
            event(0, on),
            event(96, off),
        ],
        vec![event(96, on), event(96, off)],
    ]
}

/// The bank: one instrument on program 0 over every key, playing a ramp of
/// eight steps of 100 at the mix's rate, looping its last four, with no
/// envelope at all, so a note is at its peak at once and over at its note
/// off.
fn bank() -> Bank {
    let ramp: Vec<i16> = (0..8).map(|i| i * 100).collect();
    Bank {
        instruments: vec![Instrument {
            bank: 0,
            program: 0,
            drums: false,
            regions: vec![Region {
                keys: (0, 127),
                overlaps: false,
                group: 0,
                wave: 0,
                sample: Sample {
                    looped: Some((4, 4)),
                    ..Sample::default()
                },
                articulation: Articulation::default(),
            }],
        }],
        waves: vec![Wave {
            rate: 22_050,
            samples: ramp.into(),
        }],
    }
}

/// Code assembled by hand where it will run.
struct Code {
    at: u32,
    bytes: Vec<u8>,
    /// Where the next answer [`Code::answer`] keeps goes.
    next: u32,
}

#[derive(Clone, Copy)]
enum Arg {
    Imm(u32),
    /// The word at this address, as `push dword [at]`.
    At(u32),
}
use Arg::{At, Imm};

impl Code {
    fn new(at: u32) -> Self {
        Self {
            at,
            bytes: Vec::new(),
            next: ANSWERS,
        }
    }

    fn emit(&mut self, bytes: &[u8]) -> &mut Self {
        self.bytes.extend_from_slice(bytes);
        self
    }

    fn emit32(&mut self, op: &[u8], value: u32) -> &mut Self {
        self.emit(op).emit(&value.to_le_bytes())
    }

    /// A stdcall through the boundary, as a title's imports are called:
    /// arguments pushed last first, the answer left in `eax`.
    fn call(&mut self, m: &mut Machine, dll: &str, name: &str, args: &[Arg]) -> &mut Self {
        for arg in args.iter().rev() {
            match *arg {
                Imm(value) => self.emit32(&[0x68], value),
                At(at) => self.emit32(&[0xff, 0x35], at),
            };
        }
        let to = m.boundary.bind_named(dll, name);
        self.emit32(&[0xb8], to).emit(&[0xff, 0xd0]) // mov eax, to; call eax
    }

    /// `mov [at], eax`.
    fn store(&mut self, at: u32) -> &mut Self {
        self.emit32(&[0xa3], at)
    }

    /// Keep what `eax` holds as the next answer.
    fn answer(&mut self) -> &mut Self {
        self.next += 4;
        self.store(self.next - 4)
    }

    /// `mov eax, [at]`.
    fn load(&mut self, at: u32) -> &mut Self {
        self.emit32(&[0xa1], at)
    }

    /// A short jump forward, by `op`, to wherever [`Code::land`] is called
    /// with what this hands back.
    fn jump(&mut self, op: u8) -> usize {
        self.emit(&[op, 0]);
        self.bytes.len() - 1
    }

    fn land(&mut self, hole: usize) -> &mut Self {
        self.bytes[hole] = u8::try_from(self.bytes.len() - (hole + 1)).expect("a short jump");
        self
    }
}

/// The callback, `MidiOutProc(stream, message, instance, first, second)`,
/// stdcall: it writes down the message, its first parameter, the time and the
/// header's `dwOffset`; and at `MOM_DONE` it queues the header again with
/// `midiStreamOut` while it has refills left, as LEGO Chess's does, and
/// otherwise counts it back and unprepares it, setting the event at the
/// second.
fn callback(m: &mut Machine) -> Code {
    let mut c = Code::new(CALLBACK);
    c.emit32(&[0x8b, 0x0d], LOGGED) // mov ecx, [LOGGED]
        .emit(&[0xc1, 0xe1, 0x04]) // shl ecx, 4
        .emit32(&[0x81, 0xc1], LOG) // add ecx, LOG
        .emit(&[0x8b, 0x44, 0x24, 0x08]) // mov eax, [esp+8]: the message
        .emit(&[0x89, 0x01]) // mov [ecx], eax
        .emit(&[0x8b, 0x44, 0x24, 0x10]) // mov eax, [esp+16]: the header
        .emit(&[0x89, 0x41, 0x04]) // mov [ecx+4], eax
        .emit(&[0x85, 0xc0]); // test eax, eax
    let none = c.jump(0x74); // jz
    c.emit(&[0x8b, 0x40, 0x1c]) // mov eax, [eax+28]: dwOffset
        .emit(&[0x89, 0x41, 0x0c]) // mov [ecx+12], eax
        .land(none)
        .emit(&[0x51]) // push ecx
        .call(m, "WINMM.dll", "timeGetTime", &[])
        .emit(&[0x59]) // pop ecx
        .emit(&[0x89, 0x41, 0x08]) // mov [ecx+8], eax
        .emit32(&[0xff, 0x05], LOGGED) // inc dword [LOGGED]
        .emit(&[0x81, 0x7c, 0x24, 0x08]) // cmp dword [esp+8], MOM_DONE
        .emit(&MOM_DONE.to_le_bytes());
    let not_done = c.jump(0x75); // jne
    c.emit32(&[0x83, 0x3d], REFILLS).emit(&[0]); // cmp dword [REFILLS], 0
    let counted = c.jump(0x74); // je
    c.emit32(&[0xff, 0x0d], REFILLS) // dec dword [REFILLS]
        .emit(&[0x6a, 0x40]) // push 0x40: a MIDIHDR's size
        .emit(&[0xff, 0x74, 0x24, 0x14]) // push dword [esp+0x14]: the header
        .emit(&[0xff, 0x74, 0x24, 0x0c]) // push dword [esp+0x0c]: the stream
        .call(m, "WINMM.dll", "midiStreamOut", &[]);
    let refilled = c.jump(0xeb); // jmp
    c.land(counted)
        .emit32(&[0xff, 0x05], RETURNED) // inc dword [RETURNED]
        .emit(&[0x6a, 0x40]) // push 0x40
        .emit(&[0xff, 0x74, 0x24, 0x14]) // push dword [esp+0x14]: the header
        .emit(&[0xff, 0x74, 0x24, 0x0c]) // push dword [esp+0x0c]: the stream
        .call(m, "WINMM.dll", "midiOutUnprepareHeader", &[])
        .emit32(&[0x8b, 0x0d], RETURNED) // mov ecx, [RETURNED]
        .emit32(&[0x89, 0x04, 0x8d], UNPREPARED - 4) // mov [ecx*4+UNPREPARED-4], eax
        .emit32(&[0x83, 0x3d], RETURNED)
        .emit(&[2]); // cmp dword [RETURNED], 2
    let not_both = c.jump(0x75); // jne
    c.call(m, "KERNEL32.dll", "SetEvent", &[At(EVENT)])
        .land(not_done)
        .land(not_both)
        .land(refilled)
        .emit(&[0xc2, 0x14, 0x00]); // ret 0x14
    c
}

/// How the main thread ends the stream once it has played: by waiting for
/// the callback to say both buffers came back and closing, the same after
/// pausing it a tenth of a second in for three tenths, or a tenth of a
/// second in by stopping, resetting or closing it with both still queued.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ending {
    Played,
    Paused,
    Stop,
    Reset,
    Close,
}

/// A run of the guest: what it ended with, what its calls answered, what the
/// callback wrote down, the two headers' flags and offsets, and the mix.
struct Ran {
    outcome: Outcome,
    answers: Vec<u32>,
    log: Vec<[u32; 4]>,
    headers: [(u32, u32); 2],
    unprepared: [u32; 2],
    caps: Vec<u8>,
    left: Vec<i16>,
}

/// The main thread, as LEGO Chess plays a tune and ends it: open the stream
/// on the mapper with the callback, read the mapper's caps, set the time
/// division, prepare both headers and queue both, note the time and
/// restart; then end it as `ending` says. Each answer goes to `ANSWERS` in
/// order.
fn run(synth: bool, refills: u32, ending: Ending) -> Ran {
    std::env::set_var("LEGO_LEGACY_RUNS", env!("CARGO_TARGET_TMPDIR"));
    let mut host = Headless::new();
    let mut m = Machine::new(&STUB, &mut host).expect("a machine");
    if synth {
        m.state.mixer.open_synth(Synth::new(bank()));
    }
    let mut main = Code::new(MAIN);
    let step = |main: &mut Code, m: &mut Machine, dll: &str, name: &str, args: &[Arg]| {
        main.call(m, dll, name, args).answer();
    };
    let wm = "WINMM.dll";
    let k32 = "KERNEL32.dll";
    step(&mut main, &mut m, k32, "CreateEventA", &[Imm(0); 4]);
    main.load(ANSWERS).store(EVENT);
    let open = [
        Imm(HANDLE),
        Imm(DEVICE),
        Imm(1),
        Imm(CALLBACK),
        Imm(INSTANCE),
        Imm(CALLBACK_FUNCTION),
    ];
    step(&mut main, &mut m, wm, "midiStreamOpen", &open);
    let caps = [Imm(MIDI_MAPPER), Imm(CAPS), Imm(0x34)];
    step(&mut main, &mut m, wm, "midiOutGetDevCapsA", &caps);
    if synth {
        let timediv = [At(HANDLE), Imm(TIMEDIV), Imm(MIDIPROP_SET_TIMEDIV)];
        step(&mut main, &mut m, wm, "midiStreamProperty", &timediv);
        let [first, second] = HEADERS;
        // A header queued before it is prepared is refused.
        step(
            &mut main,
            &mut m,
            wm,
            "midiStreamOut",
            &[At(HANDLE), Imm(first), Imm(0x40)],
        );
        for header in HEADERS {
            let args = [At(HANDLE), Imm(header), Imm(0x40)];
            step(&mut main, &mut m, wm, "midiOutPrepareHeader", &args);
        }
        for header in HEADERS {
            step(
                &mut main,
                &mut m,
                wm,
                "midiStreamOut",
                &[At(HANDLE), Imm(header), Imm(0x40)],
            );
        }
        // Queued twice, or unprepared while queued, is still playing.
        step(
            &mut main,
            &mut m,
            wm,
            "midiStreamOut",
            &[At(HANDLE), Imm(second), Imm(0x40)],
        );
        step(
            &mut main,
            &mut m,
            wm,
            "midiOutUnprepareHeader",
            &[At(HANDLE), Imm(second), Imm(0x40)],
        );
        step(&mut main, &mut m, wm, "timeGetTime", &[]);
        step(&mut main, &mut m, wm, "midiStreamRestart", &[At(HANDLE)]);
        // The time division may not be set while it plays.
        let timediv = [At(HANDLE), Imm(TIMEDIV), Imm(MIDIPROP_SET_TIMEDIV)];
        step(&mut main, &mut m, wm, "midiStreamProperty", &timediv);
        if ending == Ending::Paused {
            step(&mut main, &mut m, k32, "Sleep", &[Imm(100)]);
            step(&mut main, &mut m, wm, "midiStreamPause", &[At(HANDLE)]);
            step(&mut main, &mut m, k32, "Sleep", &[Imm(300)]);
            step(&mut main, &mut m, wm, "midiStreamRestart", &[At(HANDLE)]);
        }
        match ending {
            Ending::Played | Ending::Paused => {
                step(
                    &mut main,
                    &mut m,
                    k32,
                    "WaitForSingleObject",
                    &[At(EVENT), Imm(5000)],
                );
                step(&mut main, &mut m, wm, "timeGetTime", &[]);
                step(&mut main, &mut m, wm, "midiStreamClose", &[At(HANDLE)]);
            }
            _ => {
                step(&mut main, &mut m, k32, "Sleep", &[Imm(100)]);
                let call = match ending {
                    Ending::Stop => "midiStreamStop",
                    Ending::Reset => "midiOutReset",
                    _ => "midiStreamClose",
                };
                step(&mut main, &mut m, wm, call, &[At(HANDLE)]);
                // How many had come back by the time the call returned.
                main.load(RETURNED).answer();
                step(&mut main, &mut m, wm, "timeGetTime", &[]);
                if ending != Ending::Close {
                    if ending == Ending::Stop {
                        step(&mut main, &mut m, wm, "midiOutReset", &[At(HANDLE)]);
                    }
                    step(
                        &mut main,
                        &mut m,
                        k32,
                        "WaitForSingleObject",
                        &[At(EVENT), Imm(2000)],
                    );
                    step(&mut main, &mut m, wm, "midiStreamClose", &[At(HANDLE)]);
                }
                // And on, so the mix after the call is heard.
                step(&mut main, &mut m, k32, "Sleep", &[Imm(200)]);
            }
        }
    }
    main.call(&mut m, k32, "ExitProcess", &[Imm(0)]);
    let answers = (main.next - ANSWERS) / 4;

    let callback = callback(&mut m);
    let mem = &mut m.arena;
    for code in [&main, &callback] {
        mem.write(code.at, &code.bytes).expect("the code fits");
    }
    mem.write_u32(DEVICE, MIDI_MAPPER).expect("in the arena");
    mem.write_u32(REFILLS, refills).expect("in the arena");
    mem.write_u32(TIMEDIV, 8).expect("in the arena");
    mem.write_u32(TIMEDIV + 4, 192).expect("in the arena");
    for (n, events) in buffers().iter().enumerate() {
        let bytes: Vec<u8> = events
            .iter()
            .flatten()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        mem.write(BUFFERS[n], &bytes).expect("in the arena");
        // lpData, dwBufferLength, dwBytesRecorded; dwFlags zero, as
        // `midiOutPrepareHeader` asks.
        for (offset, value) in [(0, BUFFERS[n]), (4, 0x100), (8, bytes.len() as u32)] {
            mem.write_u32(HEADERS[n] + offset, value)
                .expect("in the arena");
        }
    }
    m.cpu.eip = MAIN;
    m.cpu.r[lego_legacy_core::cpu::ESP] = STACK_TOP - 16;
    let mut log = RunLog::create("stub", Trace::Off).expect("a run directory");
    let finish = m.run(&mut host, &mut log, 50_000_000);

    let word = |at: u32| m.arena.read_u32(at).expect("in the arena");
    let logged = word(LOGGED);
    Ran {
        outcome: finish.outcome,
        answers: (0..answers).map(|n| word(ANSWERS + n * 4)).collect(),
        log: (0..logged)
            .map(|n| std::array::from_fn(|i| word(LOG + n * 16 + i as u32 * 4)))
            .collect(),
        headers: HEADERS.map(|h| (word(h + 16), word(h + 28))),
        unprepared: [word(UNPREPARED), word(UNPREPARED + 4)],
        caps: m.arena.slice(CAPS, 52).expect("in the arena").to_vec(),
        left: host
            .audio_blocks
            .concat()
            .iter()
            .step_by(2)
            .copied()
            .collect(),
    }
}

/// The frames each note started at: its ramp's 0 before its 100.
fn onsets(left: &[i16]) -> Vec<u64> {
    (1..left.len())
        .filter(|&f| left[f] == 100 && left[f - 1] == 0)
        .map(|f| f as u64 - 1)
        .collect()
}

/// The mix frame a declared millisecond starts at.
fn frame(millis: u32) -> u64 {
    frame_at(u64::from(millis) * 1_000_000)
}

/// LEGO Chess's tune, played as Windows plays it (#154). The stream opens
/// paused, on the mapper, and the callback hears `MOM_OPEN` at once. From
/// the restart, the player thread reaches each event at its declared moment:
/// the volume's `MOM_POSITIONCB` at once, with `dwOffset` on its event; the
/// first buffer done half a second in, where the callback refills it from
/// inside itself; the second a second later, and the refilled one, its
/// volume's callback first, half a second after that. The callback's event
/// wakes the main thread's wait at that moment, and `MOM_CLOSE` comes last.
/// The notes are heard in the mix from the frames they are due at, and the
/// whole run is the same every time.
#[test]
fn the_stream_reaches_each_event_at_its_moment_and_calls_back_on_its_thread() {
    let ran = run(true, 1, Ending::Played);
    assert_eq!(ran.outcome, Outcome::Exited { code: 0 });
    let [event, open, caps, timediv, unprepared, first, second, out_first, out_second, again, busy, t0, restart, playing, woke, waited, close] =
        ran.answers[..]
    else {
        panic!("every call answered: {:x?}", ran.answers);
    };
    assert_ne!(event, 0, "the event");
    assert_eq!(
        [open, caps, timediv, first, second, out_first, out_second, restart, close],
        [0; 9],
        "the stream opened, its caps read, its division set, both headers prepared and queued, \
         played and closed"
    );
    assert_eq!(
        unprepared, MIDIERR_UNPREPARED,
        "queued before it was prepared"
    );
    assert_eq!(
        [again, busy],
        [MIDIERR_STILLPLAYING; 2],
        "queued again, unprepared while queued"
    );
    assert_eq!(
        playing, MMSYSERR_INVALPARAM,
        "the division set while it plays"
    );
    assert_eq!(woke, 0, "the wait was answered by the callback's event");

    // The callback, in the order it was called, and when.
    let entries: Vec<(u32, u32, u32)> = ran.log.iter().map(|e| (e[0], e[1], e[2])).collect();
    let [first_header, second_header] = HEADERS;
    assert_eq!(
        entries,
        [
            (MOM_OPEN, 0, entries[0].2),
            (MOM_POSITIONCB, first_header, t0),
            (MOM_DONE, first_header, t0 + 500),
            (MOM_DONE, second_header, t0 + 1500),
            (MOM_POSITIONCB, first_header, t0 + 1500),
            (MOM_DONE, first_header, t0 + 2000),
            (MOM_CLOSE, 0, t0 + 2000),
        ],
        "every message at its declared moment, the restart at {t0}"
    );
    assert!(entries[0].2 <= t0, "MOM_OPEN came at the open");
    assert_eq!(ran.log[1][3], 12, "dwOffset on the volume's event");
    assert_eq!(
        waited,
        t0 + 2000,
        "the main thread woke as the event was set"
    );
    // Done, a stream's, out of the queue, and unprepared by the callback:
    // the first played last from its note off, at 48.
    assert_eq!(ran.unprepared, [0, 0]);
    assert_eq!(
        ran.headers,
        [(MHDR_DONE | MHDR_ISSTRM, 48), (MHDR_DONE | MHDR_ISSTRM, 12)]
    );

    // The notes, heard from the frames they are due at: the first as the
    // restart's millisecond is reached, the others to the frame.
    let heard = onsets(&ran.left);
    assert_eq!(heard.len(), 3, "three notes: {heard:?}");
    assert!(
        (frame(t0)..frame(t0 + 1)).contains(&heard[0]),
        "the first note at {} starts within the restart's millisecond, {t0}",
        heard[0]
    );
    assert_eq!(heard[1..], [frame(t0 + 1000), frame(t0 + 1500)]);

    let again = run(true, 1, Ending::Played);
    assert!(
        again.log == ran.log && again.left == ran.left,
        "the same callbacks at the same moments, and the same mix"
    );
}

/// Stopping, resetting or closing the stream with both buffers still queued
/// returns them both to the callback with `MOM_DONE` before the call returns
/// to its caller, as `midiStreamStop`'s and `midiOutReset`'s pages say and
/// as Wine's player answers a stop; a close does what a stop does first, and
/// `MOM_CLOSE` comes last. Their notes stop there, and the callback's event
/// is set, so the 2,000 ms wait LEGO Chess makes after a stop and a reset is
/// over at once.
#[test]
fn stop_reset_and_close_return_both_buffers_before_the_caller_goes_on() {
    for ending in [Ending::Stop, Ending::Reset, Ending::Close] {
        let ran = run(true, 0, ending);
        assert_eq!(ran.outcome, Outcome::Exited { code: 0 }, "{ending:?}");
        let t0 = ran.answers[11];
        let (call, returned, at) = (ran.answers[15], ran.answers[16], ran.answers[17]);
        assert_eq!(
            (call, returned),
            (0, 2),
            "{ending:?}: both back as it returned"
        );
        if ending != Ending::Close {
            // The wait after it is over at once: the event was set before it.
            let waited = ran.answers[ran.answers.len() - 3];
            assert_eq!(waited, 0, "{ending:?}");
        }
        let [first, second] = HEADERS;
        let got: Vec<(u32, u32, u32)> = ran.log.iter().map(|e| (e[0], e[1], e[2])).collect();
        assert_eq!(
            got[1..],
            [
                (MOM_POSITIONCB, first, t0),
                (MOM_DONE, first, at),
                (MOM_DONE, second, at),
                (MOM_CLOSE, 0, got[4].2),
            ],
            "{ending:?}: both returned at the call, a tenth of a second in, and the close last"
        );
        assert_eq!(at, t0 + 100, "{ending:?}");
        // Out of the queue, so the callback could unprepare each, the stream
        // still there to name as a close gave them back.
        assert_eq!(ran.unprepared, [0, 0], "{ending:?}");
        for (flags, _) in ran.headers {
            assert_eq!(
                flags & (MHDR_DONE | MHDR_INQUEUE | MHDR_PREPARED),
                MHDR_DONE,
                "{ending:?}"
            );
        }
        // The note sounding when it was stopped is heard no more.
        let from = frame(at) as usize + 1;
        assert!(
            from < ran.left.len() && ran.left[from..].iter().all(|s| *s == 0),
            "{ending:?}: silent from the call on"
        );
    }
}

/// Time a stream is paused for is not counted, as the conformance tests
/// measure on Windows, and a note sounding when it was paused goes on
/// sounding, nothing documenting otherwise: paused a tenth of a second in for
/// three tenths, each event comes three tenths later than it would have.
#[test]
fn a_pause_holds_the_stream_where_it_was_and_is_not_counted() {
    let ran = run(true, 0, Ending::Paused);
    assert_eq!(ran.outcome, Outcome::Exited { code: 0 });
    let (t0, paused, restarted) = (ran.answers[11], ran.answers[15], ran.answers[17]);
    assert_eq!((paused, restarted), (0, 0));
    let [first, second] = HEADERS;
    let got: Vec<(u32, u32, u32)> = ran.log.iter().map(|e| (e[0], e[1], e[2])).collect();
    assert_eq!(
        got[1..4],
        [
            (MOM_POSITIONCB, first, t0),
            (MOM_DONE, first, t0 + 800),
            (MOM_DONE, second, t0 + 1800),
        ],
        "half a second and a second and a half in, and three tenths more"
    );
    // The first note sounds through the pause and stops three tenths late;
    // the second starts three tenths late.
    let heard = onsets(&ran.left);
    assert_eq!(heard.len(), 2, "two notes: {heard:?}");
    assert_eq!(heard[1], frame(t0 + 1300));
    assert_ne!(
        ran.left[frame(t0 + 250) as usize],
        0,
        "sounding in the pause"
    );
    let between = &ran.left[frame(t0 + 801) as usize..frame(t0 + 1299) as usize];
    assert!(between.iter().all(|s| *s == 0), "and off at eight tenths");
}

/// Without the player's bank the machine has no MIDI out, the mapper
/// included (ADR-0008): the stream is refused as #149 refuses it, and the
/// mapper's caps the same way, as Wine's conformance tests record Windows
/// answering with no MIDI device.
#[test]
fn without_a_bank_there_is_no_device_to_open_or_ask() {
    let ran = run(false, 0, Ending::Played);
    assert_eq!(ran.outcome, Outcome::Exited { code: 0 });
    assert_eq!(ran.answers[1..], [MMSYSERR_BADDEVICEID; 2]);
    assert!(ran.log.is_empty(), "no callback");
}

/// The mapper's caps are what Windows answers for its MIDI mapper over the
/// GS Wavetable synth, as Wine's conformance tests print them on Windows 11:
/// Microsoft's manufacturer and the mapper's product, `MOD_MAPPER`, no voices
/// or notes of its own, and `MIDICAPS_VOLUME | MIDICAPS_STREAM`, the volume
/// bit being what LEGO Chess reads before it sets the volume.
#[test]
fn the_mapper_answers_its_caps_as_windows_does() {
    let caps = run(true, 1, Ending::Played).caps;
    let half = |at: usize| u16::from_le_bytes([caps[at], caps[at + 1]]);
    assert_eq!((half(0), half(2)), (1, 1), "MM_MICROSOFT, MM_MIDI_MAPPER");
    let name = &caps[8..40];
    let end = name.iter().position(|b| *b == 0).expect("terminated");
    assert_eq!(&name[..end], b"Microsoft MIDI Mapper");
    assert_eq!([half(40), half(42), half(44), half(46)], [5, 0, 0, 0xffff]);
    assert_eq!(
        u32::from_le_bytes([caps[48], caps[49], caps[50], caps[51]]),
        9
    );
}
