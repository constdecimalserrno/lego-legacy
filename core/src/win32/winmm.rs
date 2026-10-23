//! The multimedia library: the declared clock as a title of this era reads
//! it, what it asks of sound hardware, the RIFF reader it loads its sounds
//! with, and the drive it plays its Red Book audio from.

use super::{stdcall, Ctx, Object, Shim};
use crate::cpu::Stop;
use crate::guest::GuestMem;
use std::collections::VecDeque;

pub const SHIMS: &[Shim] = &[
    stdcall("WINMM", "timeGetTime", 0, time_get_time),
    stdcall("WINMM", "timeGetSystemTime", 2, time_get_system_time),
    stdcall("WINMM", "mixerGetNumDevs", 0, mixer_devices),
    stdcall("WINMM", "mmioOpenA", 3, mmio_open),
    stdcall("WINMM", "mmioDescend", 4, mmio_descend),
    stdcall("WINMM", "mmioAscend", 3, mmio_ascend),
    stdcall("WINMM", "mmioRead", 3, mmio_read),
    stdcall("WINMM", "mmioSeek", 3, mmio_seek),
    stdcall("WINMM", "mmioGetInfo", 3, mmio_get_info),
    stdcall("WINMM", "mmioSetInfo", 3, mmio_set_info),
    stdcall("WINMM", "mmioAdvance", 3, mmio_advance),
    stdcall("WINMM", "mmioClose", 2, mmio_close),
    stdcall("WINMM", "mciSendStringA", 4, mci_send_string),
    stdcall("WINMM", "mciGetErrorStringA", 3, mci_error_string),
    stdcall("WINMM", "mciSendCommandA", 4, mci_send_command),
    stdcall("WINMM", "midiStreamOpen", 6, midi_stream_open),
    stdcall("WINMM", "midiStreamClose", 1, midi_stream_close),
    stdcall("WINMM", "midiStreamProperty", 3, midi_stream_property),
    stdcall("WINMM", "midiStreamOut", 3, midi_stream_out),
    stdcall("WINMM", "midiStreamRestart", 1, midi_stream_restart),
    stdcall("WINMM", "midiStreamPause", 1, midi_stream_pause),
    stdcall("WINMM", "midiStreamStop", 1, midi_stream_stop),
    stdcall("WINMM", "midiOutGetDevCapsA", 3, midi_out_caps),
    stdcall("WINMM", "midiOutSetVolume", 2, midi_out_volume),
    stdcall("WINMM", "midiOutPrepareHeader", 3, midi_out_prepare),
    stdcall("WINMM", "midiOutUnprepareHeader", 3, midi_out_unprepare),
    stdcall("WINMM", "midiOutReset", 1, midi_out_reset),
];

/// The `cdaudio` device: whether it is open; the title's Red Book pressing,
/// once it is open, if the title has one; whether it counts positions in
/// tracks; and the sector it started playing from and the declared
/// millisecond it started at, while it plays.
#[derive(Default)]
pub struct CdAudio {
    open: bool,
    disc: Option<crate::disc::RedBook>,
    tracks: bool,
    playing: Option<(u64, u64)>,
}

impl CdAudio {
    /// The sector the drive has played to at a declared millisecond, while it
    /// is playing: past the end of the disc, it has stopped.
    fn at(&self, millis: u64) -> Option<u64> {
        let (disc, (sector, started)) = (self.disc.as_ref()?, self.playing?);
        let at = sector + millis.saturating_sub(started) * crate::disc::SECTORS_PER_SECOND / 1000;
        (at < disc.sectors).then_some(at)
    }
}

/// `MCIERR_HARDWARE`, `_DEVICE_NOT_READY` and `_OUTOFRANGE`.
const MCIERR_HARDWARE: u32 = 262;
const MCIERR_DEVICE_NOT_READY: u32 = 276;
const MCIERR_OUTOFRANGE: u32 = 282;

/// Which voice of the mix the drive plays in: no COM object is at address
/// zero.
const CD_VOICE: u32 = 0;

/// Open the `cdaudio` device, for either face of MCI.
///
/// A title with a Red Book pressing has its tracks in the drive; a missing
/// rip is a drive with no disc of audio in it, which is not ready, and one
/// that cannot be read has a problem. A title with none has its own data disc
/// in the drive, and the device opens all the same, as Windows' mcicda opens
/// it: Wine's conformance tests (`dlls/winmm/tests/mcicda.c`), which run on
/// Windows 9x too, record `open cdaudio` succeeding with a data-only disc in
/// the drive and with none, and failing only with no drive or one already
/// taken (#149).
fn open_cdaudio(ctx: &mut Ctx<'_>) -> Result<(), u32> {
    let red_book = ctx
        .title
        .red_book
        .map(|pressing| crate::disc::open_red_book(ctx.title.image.disc_dir, pressing));
    match red_book {
        None => {}
        Some(Err(crate::disc::DiscError::Missing(_))) => return Err(MCIERR_DEVICE_NOT_READY),
        Some(Err(_)) => return Err(MCIERR_HARDWARE),
        Some(Ok(disc)) => ctx.state.cd.disc = Some(disc),
    }
    ctx.state.cd.open = true;
    Ok(())
}

fn mci_send_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mciSendStringA(command, answer, answer length, window). The commands a
    // title of this era plays its Red Book audio with: open the drive, count
    // in tracks, play from a track to the end of the disc, ask whether it is
    // still playing and which track it is on, stop, close. The tracks are the
    // title's Red Book pressing's (ADR-0006), mixed on the declared clock
    // like every other sound, and where the drive has got to follows from
    // when it started.
    let command = ctx.string(args[0]).map_err(Stop::Fault)?;
    let lower = command.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let millis = ctx.clock_millis;
    let nanos = millis * 1_000_000;
    let answer = match words[..] {
        ["open", "cdaudio"] => open_cdaudio(ctx).map(|()| String::new()),
        // Tracks, minutes, seconds and frames: a position is a track.
        ["set", "cdaudio", "time", "format", "tmsf"] => {
            ctx.state.cd.tracks = true;
            Ok(String::new())
        }
        ["play", "cdaudio", "from", _] if !ctx.state.cd.tracks => {
            return Err(Stop::NotWritten {
                what: format!("mciSendStringA(\"{command}\"): a position that is not a track"),
                caller: 0,
            })
        }
        ["play", "cdaudio", "from", track] => {
            let wanted: Option<u32> = track.parse().ok();
            let cd = &ctx.state.cd;
            match (cd.disc.as_ref(), wanted) {
                (None, _) => Err(MCIERR_DEVICE_NOT_READY),
                (Some(disc), Some(n)) => match disc.tracks.iter().find(|t| t.number == n) {
                    None => Err(MCIERR_OUTOFRANGE),
                    // ponytail: everything from the track to the end of the
                    // disc is read and decoded when it is asked for, a pause
                    // of a fraction of a second on the host at each track; a
                    // host that stutters there wants the mix to read the rip
                    // as it goes.
                    Some(track) => match disc.samples_from(track.start) {
                        Err(_) => Err(MCIERR_HARDWARE),
                        Ok(samples) => {
                            ctx.state.cd.playing = Some((track.start, millis));
                            ctx.state.mixer.play(CD_VOICE, samples, nanos);
                            Ok(String::new())
                        }
                    },
                },
                (Some(_), None) => Err(MCIERR_OUTOFRANGE),
            }
        }
        ["status", "cdaudio", "mode"] => match (&ctx.state.cd.disc, ctx.state.cd.at(millis)) {
            (None, _) => Err(MCIERR_DEVICE_NOT_READY),
            (Some(_), Some(_)) => Ok("playing".to_owned()),
            (Some(_), None) => Ok("stopped".to_owned()),
        },
        ["status", "cdaudio", "current", "track"] => {
            let cd = &ctx.state.cd;
            match &cd.disc {
                None => Err(MCIERR_DEVICE_NOT_READY),
                // A drive that is not playing is on the first track.
                Some(disc) => Ok(cd
                    .at(millis)
                    .and_then(|at| disc.track_at(at))
                    .unwrap_or(1)
                    .to_string()),
            }
        }
        ["stop", "cdaudio"] | ["close", "cdaudio"] => {
            ctx.state.cd.playing = None;
            ctx.state.mixer.stop(CD_VOICE, nanos);
            if words[0] == "close" {
                ctx.state.cd = CdAudio::default();
            }
            Ok(String::new())
        }
        _ => {
            return Err(Stop::NotWritten {
                what: format!("mciSendStringA(\"{command}\")"),
                caller: 0,
            })
        }
    };
    // Few enough to keep in every run's log, and what a person asks first
    // when what is heard is wrong.
    ctx.log
        .line(&format!("mci \"{command}\" answered {answer:?}"));
    match answer {
        Ok(text) => {
            ctx.write_string(args[1], args[2], &text)
                .map_err(Stop::Fault)?;
            Ok(0)
        }
        Err(code) => Ok(code),
    }
}

fn mci_error_string(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mciGetErrorStringA(error, buffer, length): what an error code says, in
    // the words Windows uses for it. A title asks when a command failed, as
    // `open cdaudio` does when its Red Book pressing's rip is missing.
    let text = match args[0] {
        MCIERR_HARDWARE => "There is a problem with your media device.",
        MCIERR_DEVICE_NOT_READY => "The device is not ready.",
        MCIERR_OUTOFRANGE => "The specified parameter is out of range for the specified command.",
        _ => return Ok(0),
    };
    ctx.write_string(args[1], args[2], text)
        .map_err(Stop::Fault)?;
    Ok(1)
}

/// The device id the `cdaudio` device answers to once it is open: the first
/// MCI hands out, and the machine has no other device.
const CD_DEVICE: u32 = 1;

/// mciSendCommandA(device, message, flags, parameters): the numeric face of
/// MCI, on the same `cdaudio` device as the string face. LEGO Chess's
/// framework opens it by its type's number (0x0044EBEB), counts in tracks,
/// minutes, seconds and frames, and stops and closes it as it leaves
/// (0x0044ECF0): those are answered as the string face answers them. Its
/// other CD routines (0x0044EEB0–0x0044F6F0) are never called, so a status,
/// a play or anything else, a device other than the first of its type, or a
/// notification asked for, is a wall that names it (#149).
fn mci_send_command(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    const MCI_OPEN: u32 = 0x0803;
    const MCI_CLOSE: u32 = 0x0804;
    const MCI_STOP: u32 = 0x0808;
    const MCI_SET: u32 = 0x080d;
    const MCI_NOTIFY: u32 = 0x0001;
    const MCI_WAIT: u32 = 0x0002;
    const MCI_OPEN_SHAREABLE: u32 = 0x0100;
    const MCI_OPEN_TYPE_ID: u32 = 0x1000;
    const MCI_OPEN_TYPE: u32 = 0x2000;
    const MCI_SET_TIME_FORMAT: u32 = 0x0400;
    const MCI_DEVTYPE_CD_AUDIO: u32 = 0x0204;
    const MCI_FORMAT_TMSF: u32 = 10;
    let (device, message, flags, parameters) = (args[0], args[1], args[2], args[3]);
    let read = |o: u32| ctx.mem.read_u32(parameters + o).map_err(Stop::Fault);
    let open = ctx.state.cd.open && device == CD_DEVICE;
    let answer = match message {
        // MCI_OPEN_PARMS: the callback, the device id answered, then the
        // type, its low word the type and its high word which of its devices.
        MCI_OPEN
            if flags & !(MCI_WAIT | MCI_OPEN_SHAREABLE) == MCI_OPEN_TYPE | MCI_OPEN_TYPE_ID
                && parameters != 0
                && read(8)? == MCI_DEVTYPE_CD_AUDIO =>
        {
            match open_cdaudio(ctx) {
                Ok(()) => {
                    ctx.mem
                        .write_u32(parameters + 4, CD_DEVICE)
                        .map_err(Stop::Fault)?;
                    0
                }
                Err(code) => code,
            }
        }
        // MCI_SET_PARMS: the callback, then the time format.
        MCI_SET
            if open
                && flags & !MCI_WAIT == MCI_SET_TIME_FORMAT
                && parameters != 0
                && read(4)? == MCI_FORMAT_TMSF =>
        {
            ctx.state.cd.tracks = true;
            0
        }
        MCI_STOP | MCI_CLOSE if open && flags & MCI_NOTIFY == 0 => {
            ctx.state.cd.playing = None;
            ctx.state.mixer.stop(CD_VOICE, ctx.clock_millis * 1_000_000);
            if message == MCI_CLOSE {
                ctx.state.cd = CdAudio::default();
            }
            0
        }
        _ => {
            return Err(Stop::NotWritten {
                what: format!(
                    "WINMM.dll!mciSendCommandA: message {message:#06x} to device {device}, \
                     flags {flags:#x}"
                ),
                caller: 0,
            })
        }
    };
    // As the string face does: what a person asks first when what is heard
    // is wrong.
    ctx.log.line(&format!(
        "mci command {message:#06x} to device {device}, flags {flags:#x}, answered {answer}"
    ));
    Ok(answer)
}

// The machine's MIDI out, as a title reaches it through WINMM's MIDI stream
// (#154, ADR-0008). What every call answers is Windows' own, read from
// Microsoft's page for it and from Wine's `dlls/winmm/winmm.c` and its
// conformance tests (`dlls/winmm/tests/midi.c`, which pass on Windows), read
// as documentation; what no title has asked for is a wall that names it.

const MMSYSERR_BADDEVICEID: u32 = 2;
const MMSYSERR_NOMEM: u32 = 7;
const MMSYSERR_INVALPARAM: u32 = 11;
const MIDIERR_UNPREPARED: u32 = 64;
const MIDIERR_STILLPLAYING: u32 = 65;

/// The device id of the MIDI mapper, and `midiStreamOpen`'s one callback
/// kind a title uses.
const MIDI_MAPPER: u32 = 0xffff_ffff;
const CALLBACK_FUNCTION: u32 = 0x0003_0000;

/// What a stream's callback is told, as `mmsystem.h` numbers them.
const MOM_OPEN: u32 = 0x3c7;
const MOM_CLOSE: u32 = 0x3c8;
const MOM_DONE: u32 = 0x3c9;
const MOM_POSITIONCB: u32 = 0x3ca;

/// A `MIDIHDR`'s flags, and how much of one a call must be given: up to its
/// `dwOffset`, as Wine's `midiStreamOut` and the conformance tests' smallest
/// header have it.
const MHDR_DONE: u32 = 1;
const MHDR_PREPARED: u32 = 2;
const MHDR_INQUEUE: u32 = 4;
const MHDR_ISSTRM: u32 = 8;
const MIDIHDR_LEAST: u32 = 28;

/// A `MIDIEVENT`'s flags, in the high byte of its event.
const MEVT_F_CALLBACK: u32 = 0x4000_0000;

/// The machine's MIDI out as WINMM serves it: the player thread it plays a
/// stream on, the stream open on it, and the calls the player owes the
/// title.
pub struct Midi {
    /// The player, by its place among the machine's threads.
    player: usize,
    stream: Option<Stream>,
    /// Calls the player owes the title, oldest first: the `MOM_DONE` of each
    /// buffer a stop, a reset or a close returned.
    owed: VecDeque<(u32, [u32; 5])>,
}

/// A MIDI stream a title has open: whom it calls back, its time division and
/// tempo, the buffers it has queued and where it has played to.
struct Stream {
    handle: u32,
    callback: u32,
    instance: u32,
    /// Ticks to a quarter note, and microseconds to a quarter note.
    division: u32,
    tempo: u32,
    /// The headers queued, the first playing, and the offset of its next
    /// event.
    queue: VecDeque<u32>,
    at: u32,
    /// The stream's own time of the last event played, in nanoseconds, and
    /// what the time division left over of it: a tick is rarely a whole
    /// number of nanoseconds, and nothing is lost between events.
    played: u64,
    left_over: u128,
    /// The declared nanosecond it was last restarted or paused at and its
    /// own time then, and whether it has played on since.
    clock: (u64, u64),
    playing: bool,
    stopped: bool,
    /// It has been closed, and is kept only while the player gives back the
    /// buffers the close returned: their callbacks may still name it.
    closing: bool,
    /// The notes it has turned on and not off, a bit a key, by channel, and
    /// the status a message without one runs on with.
    sounding: [u128; 16],
    running: u8,
}

impl Stream {
    /// Note what a short message does to the notes sounding.
    fn track(&mut self, message: u32) {
        let [mut status, mut key, mut velocity, _] = message.to_le_bytes();
        match status {
            0..0x80 => (status, key, velocity) = (self.running, status, key),
            0x80..0xf0 => self.running = status,
            0xf0..0xf8 => self.running = 0,
            _ => {}
        }
        let (channel, bit) = (usize::from(status & 0x0f), 1u128 << (key & 0x7f));
        match status & 0xf0 {
            0x90 if velocity > 0 => self.sounding[channel] |= bit,
            0x80 | 0x90 => self.sounding[channel] &= !bit,
            _ => {}
        }
    }
}

/// What the declared clock reads, in nanoseconds, as a shim sees it.
fn now(ctx: &Ctx<'_>) -> u64 {
    ctx.clock_millis * 1_000_000
}

/// The stream a handle names, if it is the one open.
fn stream<'a>(ctx: &'a mut Ctx<'_>, handle: u32) -> Option<&'a mut Stream> {
    ctx.state
        .midi
        .as_mut()?
        .stream
        .as_mut()
        .filter(|s| s.handle == handle)
}

/// Hand the player something to do. It runs before the caller goes on, as
/// Windows gives a time-critical thread it has just made ready the processor,
/// and as Wine's caller waits for its player (`midistream_post_message_and_wait`).
fn wake_the_player(state: &mut super::State) {
    if let Some(midi) = &state.midi {
        state.threads[midi.player].wake_at = 0;
    }
    state.yielded = true;
}

/// Send the synth a short message at a declared time: a message no synth
/// hears is never dropped as if it were heard (#155).
fn send(ctx: &mut Ctx<'_>, nanos: u64, message: u32) -> Result<(), Stop> {
    if ctx.state.mixer.midi(nanos, message) {
        return Ok(());
    }
    Err(Stop::NotWritten {
        what: format!("a MIDI message {message:#08x} with no synth to hear it"),
        caller: 0,
    })
}

/// Return every buffer the stream has queued, `MHDR_DONE` set and out of the
/// queue, as `midiStreamStop`'s and `midiOutReset`'s pages say: the player
/// owes the callback a `MOM_DONE` for each, and makes them before the caller
/// goes on.
fn give_back(ctx: &mut Ctx<'_>, handle: u32) -> Result<(), Stop> {
    let Some(stream) = stream(ctx, handle) else {
        return Ok(());
    };
    let headers: Vec<u32> = stream.queue.drain(..).collect();
    stream.at = 0;
    let (callback, instance) = (stream.callback, stream.instance);
    for &header in &headers {
        let args = done(ctx, handle, instance, header)?;
        if let Some(midi) = ctx.state.midi.as_mut() {
            midi.owed.push_back((callback, args));
        }
    }
    if !headers.is_empty() {
        wake_the_player(ctx.state);
    }
    Ok(())
}

/// A buffer is done: `MHDR_DONE` set and out of the queue, and, as Windows 9x
/// has it, a stream's (`MHDR_ISSTRM`). What its callback is told.
fn done(ctx: &mut Ctx<'_>, handle: u32, instance: u32, header: u32) -> Result<[u32; 5], Stop> {
    let flags = ctx.mem.read_u32(header + 16).map_err(Stop::Fault)?;
    let done = (flags | MHDR_DONE | MHDR_ISSTRM) & !MHDR_INQUEUE;
    ctx.mem.write_u32(header + 16, done).map_err(Stop::Fault)?;
    Ok([handle, MOM_DONE, instance, header, 0])
}

/// Turn off the notes the stream turned on, as `midiStreamStop` does.
fn notes_off(ctx: &mut Ctx<'_>, handle: u32) -> Result<(), Stop> {
    let at = now(ctx);
    let Some(stream) = stream(ctx, handle) else {
        return Ok(());
    };
    let sounding = std::mem::take(&mut stream.sounding);
    for (channel, keys) in (0u32..).zip(sounding) {
        for key in (0..128).filter(|k| keys & (1u128 << k) != 0) {
            send(ctx, at, 0x80 | channel | key << 8)?;
        }
    }
    Ok(())
}

/// A wall for what no title asks of a stream.
fn not_written(what: String) -> Result<u32, Stop> {
    Err(Stop::NotWritten { what, caller: 0 })
}

/// midiStreamOpen(stream, device, count, callback, instance, flags).
///
/// The declared machine's MIDI out is the Microsoft GS Wavetable SW Synth,
/// and the mapper, device -1, sends to it; it has one only with the player's
/// bank (ADR-0008). Without it no device is in range, the mapper included:
/// `MMSYSERR_BADDEVICEID`, which the call's own page lists and Wine's
/// conformance tests record every Windows answering for the mapper with no
/// port (#149). A stream of other than one device, or with nowhere to say
/// which, is refused as invalid, as the same page and Wine's code have it. A
/// stream opens paused, at Windows' 24 ticks a quarter note and 500,000
/// microseconds to it, the defaults the conformance tests read back; its
/// callback is told `MOM_OPEN` on the caller's thread before the call
/// returns, as Wine's `midiStreamOpen` tells it. Its player is a thread of
/// its own, made with the first stream and kept for every stream after it:
/// LEGO Chess closes its stream and opens another at each change of tune.
fn midi_stream_open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (stream_at, device_at, count, callback, instance, flags) =
        (args[0], args[1], args[2], args[3], args[4], args[5]);
    let answer = if stream_at == 0 || device_at == 0 || count != 1 {
        MMSYSERR_INVALPARAM
    } else if !ctx.state.mixer.midi_out()
        || ![MIDI_MAPPER, 0].contains(&ctx.mem.read_u32(device_at).map_err(Stop::Fault)?)
    {
        MMSYSERR_BADDEVICEID
    } else {
        open_stream(ctx, stream_at, callback, instance, flags)?
    };
    ctx.log.line(&format!("midi stream open answered {answer}"));
    Ok(answer)
}

fn open_stream(
    ctx: &mut Ctx<'_>,
    stream_at: u32,
    callback: u32,
    instance: u32,
    flags: u32,
) -> Result<u32, Stop> {
    if flags != CALLBACK_FUNCTION || callback == 0 {
        return not_written(format!(
            "WINMM.dll!midiStreamOpen: a callback {callback:#x} of flags {flags:#x}"
        ));
    }
    if ctx.state.midi.as_ref().is_some_and(|m| m.stream.is_some()) {
        return not_written("WINMM.dll!midiStreamOpen: a second stream at once".to_owned());
    }
    if ctx.state.midi.is_none() {
        let start = ctx.boundary.bind_midi_player();
        if super::kernel32::spawn(ctx, 0, start, 0, false)?.is_none() {
            return Ok(MMSYSERR_NOMEM);
        }
        let player = ctx.state.threads.len() - 1;
        let thread = &mut ctx.state.threads[player];
        // Time-critical, as Wine makes its player; with no stream to play
        // yet, it waits.
        thread.time_critical = true;
        thread.wake_at = u64::MAX;
        ctx.state.midi = Some(Midi {
            player,
            stream: None,
            owed: VecDeque::new(),
        });
    }
    let handle = ctx.state.give(Object::MidiStream);
    ctx.mem.write_u32(stream_at, handle).map_err(Stop::Fault)?;
    let clock = (now(ctx), 0);
    if let Some(midi) = ctx.state.midi.as_mut() {
        midi.stream = Some(Stream {
            handle,
            callback,
            instance,
            division: 24,
            tempo: 500_000,
            queue: VecDeque::new(),
            at: 0,
            played: 0,
            left_over: 0,
            clock,
            playing: false,
            stopped: false,
            closing: false,
            sounding: [0; 16],
            running: 0,
        });
    }
    ctx.call_guest = Some((callback, vec![handle, MOM_OPEN, instance, 0, 0]));
    Ok(0)
}

/// midiStreamClose(stream): it stops the stream first, as Wine's does, its
/// queued buffers coming back to the callback on the player, which may still
/// name the stream as they do; then the callback is told `MOM_CLOSE` on the
/// caller's thread, and the handle names nothing.
fn midi_stream_close(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let handle = args[0];
    let Some(open) = stream(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    let (callback, instance) = (open.callback, open.instance);
    if ctx.state.midi.as_ref().map(|m| m.player) == Some(ctx.state.thread) {
        return not_written("WINMM.dll!midiStreamClose from its own callback".to_owned());
    }
    notes_off(ctx, handle)?;
    give_back(ctx, handle)?;
    if let Some(stream) = stream(ctx, handle) {
        (stream.stopped, stream.closing) = (true, true);
    }
    forget_closed(ctx);
    ctx.call_guest = Some((callback, vec![handle, MOM_CLOSE, instance, 0, 0]));
    Ok(0)
}

/// Let a closed stream go once the player owes nothing for it: its handle
/// then names nothing.
fn forget_closed(ctx: &mut Ctx<'_>) {
    let Some(midi) = ctx.state.midi.as_mut() else {
        return;
    };
    if !midi.owed.is_empty() || !midi.stream.as_ref().is_some_and(|s| s.closing) {
        return;
    }
    if let Some(stream) = midi.stream.take() {
        ctx.state.close(stream.handle);
    }
}

/// midiStreamProperty(stream, data, property): the time division, set while
/// the stream does not play, as the call's page says and Wine refuses it
/// with `MMSYSERR_INVALPARAM` while it plays. Getting a property, the tempo,
/// and a division in SMPTE frames are walls: no title asks for them.
fn midi_stream_property(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    const MIDIPROP_SET_TIMEDIV: u32 = 0x8000_0001;
    let (handle, data, property) = (args[0], args[1], args[2]);
    if stream(ctx, handle).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    if property != MIDIPROP_SET_TIMEDIV {
        return not_written(format!(
            "WINMM.dll!midiStreamProperty: property {property:#x}"
        ));
    }
    let size = ctx.mem.read_u32(data).map_err(Stop::Fault)?;
    let division = ctx.mem.read_u32(data + 4).map_err(Stop::Fault)?;
    let Some(stream) = stream(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    if size != 8 || stream.playing {
        return Ok(MMSYSERR_INVALPARAM);
    }
    if division == 0 || division & 0x8000 != 0 {
        return not_written(format!(
            "WINMM.dll!midiStreamProperty: a time division of {division:#x}"
        ));
    }
    stream.division = division;
    stream.left_over = 0;
    Ok(0)
}

/// midiStreamOut(stream, header, size): queue a prepared buffer, as Wine's
/// does: a header too small, with no data, more recorded than it holds, or
/// recorded in other than whole words is invalid; one not prepared is
/// `MIDIERR_UNPREPARED`, and one already queued `MIDIERR_STILLPLAYING`. It
/// plays from where the queue reaches it.
fn midi_stream_out(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, header, size) = (args[0], args[1], args[2]);
    if size < MIDIHDR_LEAST || header == 0 {
        return Ok(MMSYSERR_INVALPARAM);
    }
    let read = |o: u32| ctx.mem.read_u32(header + o).map_err(Stop::Fault);
    let (data, length, recorded, flags) = (read(0)?, read(4)?, read(8)?, read(16)?);
    if data == 0 || recorded > length || recorded % 4 != 0 {
        return Ok(MMSYSERR_INVALPARAM);
    }
    if flags & MHDR_PREPARED == 0 {
        return Ok(MIDIERR_UNPREPARED);
    }
    if flags & MHDR_INQUEUE != 0 {
        return Ok(MIDIERR_STILLPLAYING);
    }
    let Some(stream) = stream(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    if stream.stopped {
        return not_written("WINMM.dll!midiStreamOut on a stream stopped".to_owned());
    }
    stream.queue.push_back(header);
    let playing = stream.playing;
    // Queued, and not done; Windows 9x sets `MHDR_ISSTRM` only once it is
    // done, as the conformance tests record.
    let queued = (flags | MHDR_INQUEUE) & !MHDR_DONE;
    ctx.mem
        .write_u32(header + 16, queued)
        .map_err(Stop::Fault)?;
    if playing {
        wake_the_player(ctx.state);
    }
    Ok(0)
}

/// midiStreamRestart(stream): it plays on from where it was paused, the
/// time it was paused not counted, as the conformance tests measure; one
/// already playing is left as it is, as the call's page says.
fn midi_stream_restart(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let at = now(ctx);
    let Some(stream) = stream(ctx, args[0]) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    if stream.stopped {
        return not_written("WINMM.dll!midiStreamRestart on a stream stopped".to_owned());
    }
    if !stream.playing {
        stream.clock = (at, stream.clock.1);
        stream.playing = true;
        wake_the_player(ctx.state);
    }
    Ok(0)
}

/// midiStreamPause(stream): where it has got to is kept, as the call's page
/// says; one already paused is left as it is. Notes sounding go on sounding:
/// nothing documents a pause turning them off, and Wine's does not.
fn midi_stream_pause(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let at = now(ctx);
    let Some(stream) = stream(ctx, args[0]) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    if stream.stopped {
        return not_written("WINMM.dll!midiStreamPause on a stream stopped".to_owned());
    }
    if stream.playing {
        let (since, then) = stream.clock;
        stream.clock = (at, then + at.saturating_sub(since));
        stream.playing = false;
    }
    Ok(0)
}

/// midiStreamStop(stream): it turns off the notes the stream turned on, a
/// note-off each, and returns its queued buffers to the callback, as the
/// call's page says, and goes back to its start, as Wine's does. The page
/// sends no more than those note-offs, sustain included, unlike
/// `midiOutReset`. A stopped stream is only reset
/// or closed after: what else Windows answers on one is not documented.
fn midi_stream_stop(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, at) = (args[0], now(ctx));
    if stream(ctx, handle).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    notes_off(ctx, handle)?;
    give_back(ctx, handle)?;
    if let Some(stream) = stream(ctx, handle) {
        (stream.played, stream.left_over, stream.clock) = (0, 0, (at, 0));
        (stream.playing, stream.stopped) = (false, true);
    }
    Ok(0)
}

/// midiOutReset(stream): it returns the stream's queued buffers to the
/// callback, and sends a note-off for every note on every channel and turns
/// each channel's sustain off, as the call's page says.
fn midi_out_reset(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, at) = (args[0], now(ctx));
    let Some(stream) = stream(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    stream.sounding = [0; 16];
    give_back(ctx, handle)?;
    for channel in 0..16 {
        for key in 0..128 {
            send(ctx, at, 0x80 | channel | key << 8)?;
        }
        send(ctx, at, 0xb0 | channel | 64 << 8)?;
    }
    Ok(0)
}

/// midiOutPrepareHeader(stream, header, size): a header too small, or a
/// stream buffer over 64K, is invalid, as the call's page says; one prepared
/// already is left as it is. Windows 9x leaves the other flags as they were,
/// as the conformance tests record.
fn midi_out_prepare(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, header, size) = (args[0], args[1], args[2]);
    if stream(ctx, handle).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    if header == 0 || size < MIDIHDR_LEAST {
        return Ok(MMSYSERR_INVALPARAM);
    }
    if ctx.mem.read_u32(header + 4).map_err(Stop::Fault)? > 0x1_0000 {
        return Ok(MMSYSERR_INVALPARAM);
    }
    let flags = ctx.mem.read_u32(header + 16).map_err(Stop::Fault)?;
    ctx.mem
        .write_u32(header + 16, flags | MHDR_PREPARED)
        .map_err(Stop::Fault)?;
    Ok(0)
}

/// midiOutUnprepareHeader(stream, header, size): one not prepared is left as
/// it is, and one still queued is `MIDIERR_STILLPLAYING`, as the call's page
/// says and the conformance tests check in that order.
fn midi_out_unprepare(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, header, size) = (args[0], args[1], args[2]);
    if stream(ctx, handle).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    if header == 0 || size < MIDIHDR_LEAST {
        return Ok(MMSYSERR_INVALPARAM);
    }
    let flags = ctx.mem.read_u32(header + 16).map_err(Stop::Fault)?;
    if flags & MHDR_PREPARED == 0 {
        return Ok(0);
    }
    if flags & MHDR_INQUEUE != 0 {
        return Ok(MIDIERR_STILLPLAYING);
    }
    ctx.mem
        .write_u32(header + 16, flags & !MHDR_PREPARED)
        .map_err(Stop::Fault)?;
    Ok(0)
}

/// What the MIDI mapper says it is, as `midiOutGetDevCapsA` writes its
/// `MIDIOUTCAPSA`: what Windows answers for its mapper over the GS Wavetable
/// synth, as Wine's conformance tests print it on Windows 11
/// (test.winehq.org, `winmm:midi`): "Microsoft MIDI Mapper", manufacturer 1
/// (`MM_MICROSOFT`), product 1 (`MM_MIDI_MAPPER`), `MOD_MAPPER`, no voices or
/// notes of its own, and support 9, `MIDICAPS_VOLUME | MIDICAPS_STREAM`, its
/// synth's volume and streams, as Wine's midimap notes the mapper answers.
/// The channel mask is every channel, the GS synth's, which Matthew van
/// Eerde's `midienum` prints for it (2012); the driver version is zero, no
/// answer seen having printed one. LEGO Chess reads only the support.
fn mapper_caps() -> Vec<u8> {
    let mut caps = Vec::with_capacity(52);
    caps.extend_from_slice(&1u16.to_le_bytes());
    caps.extend_from_slice(&1u16.to_le_bytes());
    caps.extend_from_slice(&0u32.to_le_bytes());
    let mut name = [0u8; 32];
    name[..21].copy_from_slice(b"Microsoft MIDI Mapper");
    caps.extend_from_slice(&name);
    for half in [5u16, 0, 0, 0xffff] {
        caps.extend_from_slice(&half.to_le_bytes());
    }
    caps.extend_from_slice(&9u32.to_le_bytes());
    caps
}

/// midiOutGetDevCapsA(device, caps, size): the mapper's, as much of them as
/// the caller has room for. Without a MIDI out the mapper is out of range,
/// as the conformance tests record Windows answering with no MIDI device. Any
/// other device, or a handle, is a wall: LEGO Chess asks only the mapper.
fn midi_out_caps(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (device, caps, size) = (args[0], args[1], args[2]);
    if !ctx.state.mixer.midi_out() {
        return Ok(MMSYSERR_BADDEVICEID);
    }
    if device != MIDI_MAPPER {
        return not_written(format!("WINMM.dll!midiOutGetDevCapsA: device {device:#x}"));
    }
    if caps == 0 {
        return Ok(MMSYSERR_INVALPARAM);
    }
    let bytes = mapper_caps();
    let take = bytes.len().min(size as usize);
    ctx.mem.write(caps, &bytes[..take]).map_err(Stop::Fault)?;
    Ok(0)
}

/// midiOutSetVolume(stream, volume): the synth's volume, from the declared
/// moment on. Microsoft's page has 0xFFFF full and 0x0000 silence,
/// "interpreted logarithmically", so that each step up is heard as the same
/// rise: so it is decibels, straight from 0 dB at 0xFFFF down to -96 dB, the
/// bottom of sixteen-bit sound and the synth's own end of sound, at 0x0000.
/// The mapper has no `MIDICAPS_LRVOLUME`, so the low word is the volume and
/// the high one is not heard, as the page says of such a device. A device id,
/// or any handle but the stream's, is a wall: LEGO Chess sets only its
/// stream's.
fn midi_out_volume(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    let (handle, volume) = (args[0], args[1]);
    if stream(ctx, handle).is_none() {
        return not_written(format!(
            "WINMM.dll!midiOutSetVolume of {handle:#x}, which is no stream open"
        ));
    }
    let quieter = i64::from(0xffff - (volume & 0xffff));
    let attenuation = (960i64 << 16) * quieter / 0xffff;
    let at = now(ctx);
    ctx.state.mixer.midi_volume(at, &*ctx.mem, attenuation);
    Ok(0)
}

/// The player's turn (#154), on its own thread: the next call it owes the
/// title, or else every event of its stream now due played, until one asks
/// for the callback or a buffer is done, and then that callback. With
/// nothing due it waits for the millisecond the next event is due in, and
/// with nothing to play, for a call to give it something.
///
/// An event's moment is its delta in ticks at the tempo then in force, from
/// the one before it, as Wine's player waits it. A short message goes to the
/// synth at that very nanosecond, which the mix hears from there unless it
/// has passed it already.
// ponytail: it usually has. The player wakes at the millisecond after the
// moment, at the next switch, and the machine mixes there before the player
// plays, so a note is heard up to a millisecond and a quantum late; holding
// the mix at the player's next event would make every note exact. A tempo applies to the deltas after it; a no-op
/// does nothing but call back; anything else is a wall. Each event played
/// leaves its offset in the header's `dwOffset`, as Windows 9x does, and one
/// marked `MEVT_F_CALLBACK` calls back with `MOM_POSITIONCB`. A buffer
/// played to its end is done, out of the queue, and calls back with
/// `MOM_DONE`, as Wine's player does.
pub(crate) fn player_turn(ctx: &mut Ctx<'_>) -> Result<Option<(u32, Vec<u32>)>, Stop> {
    let at = now(ctx);
    let owed = ctx.state.midi.as_mut().and_then(|m| m.owed.pop_front());
    if let Some((callback, args)) = owed {
        return Ok(Some(call_back(ctx, callback, &args)));
    }
    forget_closed(ctx);
    loop {
        let Some(stream) = ctx
            .state
            .midi
            .as_mut()
            .and_then(|m| m.stream.as_mut())
            .filter(|s| s.playing)
        else {
            return Ok(wait(ctx, u64::MAX));
        };
        let Some(&header) = stream.queue.front() else {
            return Ok(wait(ctx, u64::MAX));
        };
        let (handle, callback, instance) = (stream.handle, stream.callback, stream.instance);
        let read = |at: u32| ctx.mem.read_u32(at).map_err(Stop::Fault);
        let (data, recorded) = (read(header)?, read(header + 8)?);
        if stream.at + 12 > recorded {
            stream.queue.pop_front();
            stream.at = 0;
            let args = done(ctx, handle, instance, header)?;
            return Ok(Some(call_back(ctx, callback, &args)));
        }
        let offset = stream.at;
        let (delta, event) = (read(data + offset)?, read(data + offset + 8)?);
        let ticks = u128::from(delta) * u128::from(stream.tempo) * 1000 + stream.left_over;
        let division = u128::from(stream.division);
        let moment = stream.played + u64::try_from(ticks / division).unwrap_or(u64::MAX);
        let due = (stream.clock.0 + moment).saturating_sub(stream.clock.1);
        if due > at {
            return Ok(wait(ctx, due.div_ceil(1_000_000)));
        }
        (stream.played, stream.left_over) = (moment, ticks % division);
        stream.at += 12;
        let parameter = event & 0x00ff_ffff;
        let short = match (event & !MEVT_F_CALLBACK) >> 24 {
            // MEVT_SHORTMSG, MEVT_TEMPO and MEVT_NOP.
            0x00 => {
                stream.track(parameter);
                Some(parameter)
            }
            0x01 => {
                stream.tempo = parameter;
                None
            }
            0x02 => None,
            kind => {
                return Err(Stop::NotWritten {
                    what: format!(
                        "a MIDI stream event of type {kind:#04x}, at {offset} in {header:#x}"
                    ),
                    caller: 0,
                })
            }
        };
        // Whatever size the title said its header was, as Windows writes it
        // (the conformance tests' "native fills dwOffset regardless").
        ctx.mem
            .write_u32(header + 28, offset)
            .map_err(Stop::Fault)?;
        if let Some(message) = short {
            send(ctx, due, message)?;
        }
        if event & MEVT_F_CALLBACK != 0 {
            let args = [handle, MOM_POSITIONCB, instance, header, 0];
            return Ok(Some(call_back(ctx, callback, &args)));
        }
    }
}

/// The player waits until a declared millisecond: none, for a call to give
/// it something to do.
fn wait(ctx: &mut Ctx<'_>, until: u64) -> Option<(u32, Vec<u32>)> {
    let running = ctx.state.thread;
    ctx.state.threads[running].wake_at = until;
    None
}

/// The player calls the title back, and says so in a traced run.
fn call_back(ctx: &mut Ctx<'_>, callback: u32, args: &[u32; 5]) -> (u32, Vec<u32>) {
    let millis = ctx.clock_millis;
    ctx.log.at(crate::cli::Trace::Shims, || {
        format!(
            "midi player calls back {:#x} with {:#x} at {millis} ms",
            args[1], args[3]
        )
    });
    (callback, args.to_vec())
}

fn time_get_time(ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // Milliseconds on the declared clock, wrapping as the real counter does
    // (ADR-0003). A clock spin's pass relies on that grain (`spin.rs`, #217),
    // and on `timeGetSystemTime`'s, which answers the same.
    Ok((ctx.declared_millis() & 0xffff_ffff) as u32)
}

/// timeGetSystemTime(time, size): the same milliseconds as `timeGetTime`,
/// handed back in an `MMTIME` as Microsoft documents it: its `wType`, then
/// `ms`, with `wType` saying `TIME_MS`, and `TIMERR_NOERROR`. LEGO Chess
/// reads its clock this way for its waits, its two at boot included (the
/// call that returns to 0x0040E20D, #147). An `MMTIME` smaller than Win32's
/// twelve bytes, or none, is a wall that names the size: no observed Windows
/// says what it answers.
fn time_get_system_time(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    const TIME_MS: u32 = 0x0001;
    const MMTIME_BYTES: u32 = 12;
    let (time, size) = (args[0], args[1]);
    if time == 0 || size < MMTIME_BYTES {
        return Err(Stop::NotWritten {
            what: format!("WINMM.dll!timeGetSystemTime: an MMTIME at {time:#x} of {size} bytes"),
            caller: 0,
        });
    }
    let millis = time_get_time(ctx, &[])?;
    ctx.mem.write_u32(time, TIME_MS).map_err(Stop::Fault)?;
    ctx.mem.write_u32(time + 4, millis).map_err(Stop::Fault)?;
    Ok(0)
}

fn mixer_devices(_ctx: &mut Ctx<'_>, _args: &[u32]) -> Result<u32, Stop> {
    // The declared machine has no hardware mixer: what a title plays is mixed
    // on the machine's own clock (ADR-0003), and there are no lines on a card
    // for it to turn up or down. Rock Raiders reads zero as nothing to set.
    Ok(0)
}

/// `FOURCC_MEM`: a file that is a run of the caller's own memory.
const FOURCC_MEM: u32 = u32::from_le_bytes(*b"MEM ");

/// `MMIOERR_CHUNKNOTFOUND` and `MMSYSERR_INVALHANDLE`.
const MMIOERR_CHUNKNOTFOUND: u32 = 265;
const MMSYSERR_INVALHANDLE: u32 = 5;

fn mmio_open(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioOpenA(name, info, flags). A title that has its file in memory
    // already opens it through the MEM procedure, with the memory in the
    // MMIOINFO: its size at 20 and where it is at 24. A file opened by name
    // is read whole into guest memory and read from there, which is what
    // reading it through a buffer as large as the file would be.
    // ponytail: a file opened by name is copied into the guest's heap whole,
    // which moves where later allocations land; a title that streams files
    // too large for its heap needs reads from the file as it goes.
    const MMIO_ALLOCBUF: u32 = 0x0001_0000;
    let (name, info, flags) = (args[0], args[1], args[2]);
    let (procedure, custom) = if info == 0 {
        (0, 0)
    } else {
        (
            ctx.mem.read_u32(info + 4).map_err(Stop::Fault)?,
            ctx.mem.read_u32(info + 8).map_err(Stop::Fault)?,
        )
    };
    // Reading is all this opens: creating, deleting, parsing a name, asking
    // whether a file exists, or a title's own I/O procedure, is a wall.
    if flags & !MMIO_ALLOCBUF != 0 || custom != 0 {
        return Err(Stop::NotWritten {
            what: format!("WINMM.dll!mmioOpenA: flags {flags:#x}, I/O procedure {custom:#x}"),
            caller: 0,
        });
    }
    if name == 0 && procedure == FOURCC_MEM {
        let len = ctx.mem.read_u32(info + 20).map_err(Stop::Fault)?;
        let base = ctx.mem.read_u32(info + 24).map_err(Stop::Fault)?;
        return Ok(ctx.state.give(Object::Mmio {
            base,
            len,
            at: 0,
            owned: false,
        }));
    }
    if name == 0 || procedure != 0 {
        return Err(Stop::NotWritten {
            what: format!("WINMM.dll!mmioOpenA: procedure {procedure:#x}"),
            caller: 0,
        });
    }
    let path = ctx.string(name).map_err(Stop::Fault)?;
    if ctx.refuse_on_empty_floppy(&path)? {
        return Ok(0);
    }
    let Some(bytes) = ctx.open_guest_file(&path) else {
        ctx.log.line(&format!("file missing {path}"));
        return Ok(0);
    };
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let base = ctx.state.heap.alloc(len.max(1));
    ctx.mem.write(base, &bytes).map_err(Stop::Fault)?;
    ctx.log
        .at(crate::cli::Trace::Shims, || format!("mmio opened {path}"));
    Ok(ctx.state.give(Object::Mmio {
        base,
        len,
        at: 0,
        owned: true,
    }))
}

/// A memory file's run of guest memory, and where it is reading.
fn mmio(ctx: &Ctx<'_>, handle: u32) -> Option<(u32, u32, u32)> {
    match ctx.state.object(handle) {
        Some(Object::Mmio { base, len, at, .. }) => Some((*base, *len, *at)),
        _ => None,
    }
}

fn move_to(ctx: &mut Ctx<'_>, handle: u32, to: u32) {
    if let Some(Object::Mmio { at, .. }) = ctx.state.object_mut(handle) {
        *at = to;
    }
}

/// What `mmioDescend` looks for: the next chunk, a chunk by its id, or a RIFF
/// or LIST chunk by its form type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Find {
    Next,
    Chunk(u32),
    Form(u32, u32),
}

/// A chunk found: its id, size, form type, and where its data starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub(crate) id: u32,
    pub(crate) size: u32,
    pub(crate) form: u32,
    pub(crate) data: u32,
}

/// Find a chunk in RIFF bytes, from `at` to `end`, stepping over each chunk
/// that is not it, padded to an even size. A DLS bank is walked with it too
/// ([`crate::dls`]).
pub(crate) fn find_chunk(bytes: &[u8], mut at: u32, end: u32, find: Find) -> Option<Chunk> {
    let word = |o: u32| {
        bytes
            .get(o as usize..o as usize + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    while at.checked_add(8).is_some_and(|header| header <= end) {
        let (id, size) = (word(at)?, word(at + 4)?);
        let is_form = id == u32::from_le_bytes(*b"RIFF") || id == u32::from_le_bytes(*b"LIST");
        let form = if is_form {
            word(at.checked_add(8)?)?
        } else {
            0
        };
        let found = match find {
            Find::Next => true,
            Find::Chunk(want) => id == want,
            Find::Form(kind, want) => id == kind && form == want,
        };
        if found {
            return Some(Chunk {
                id,
                size,
                form,
                data: at + 8,
            });
        }
        at = at
            .checked_add(8)?
            .checked_add(size)?
            .checked_add(size % 2)?;
    }
    None
}

fn mmio_descend(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioDescend(file, chunk, parent, flags). An MMCKINFO: id, size, form
    // type, data offset, flags.
    const MMIO_FINDCHUNK: u32 = 0x10;
    const MMIO_FINDRIFF: u32 = 0x20;
    const MMIO_FINDLIST: u32 = 0x40;
    let (handle, chunk_at, parent, flags) = (args[0], args[1], args[2], args[3]);
    let Some((base, len, at)) = mmio(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    let read = |o: u32| ctx.mem.read_u32(chunk_at + o).map_err(Stop::Fault);
    let find = match flags & 0x70 {
        0 => Find::Next,
        MMIO_FINDCHUNK => Find::Chunk(read(0)?),
        MMIO_FINDRIFF => Find::Form(u32::from_le_bytes(*b"RIFF"), read(8)?),
        MMIO_FINDLIST => Find::Form(u32::from_le_bytes(*b"LIST"), read(8)?),
        other => {
            return Err(Stop::NotWritten {
                what: format!("WINMM.dll!mmioDescend: flags {other:#x}"),
                caller: 0,
            })
        }
    };
    let end = if parent == 0 {
        len
    } else {
        let data = ctx.mem.read_u32(parent + 12).map_err(Stop::Fault)?;
        let size = ctx.mem.read_u32(parent + 4).map_err(Stop::Fault)?;
        data.saturating_add(size).min(len)
    };
    let bytes = ctx.mem.slice(base, len as usize).map_err(Stop::Fault)?;
    let Some(found) = find_chunk(bytes, at, end, find) else {
        return Ok(MMIOERR_CHUNKNOTFOUND);
    };
    for (offset, value) in [
        (0, found.id),
        (4, found.size),
        (8, found.form),
        (12, found.data),
        (16, 0),
    ] {
        ctx.mem
            .write_u32(chunk_at + offset, value)
            .map_err(Stop::Fault)?;
    }
    // Into the chunk's data: past its form type, for a RIFF or LIST chunk.
    let into = found
        .data
        .saturating_add(if found.form != 0 { 4 } else { 0 });
    move_to(ctx, handle, into);
    Ok(0)
}

fn mmio_ascend(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioAscend(file, chunk, flags): out past the chunk, padded to even.
    let (handle, chunk_at) = (args[0], args[1]);
    if mmio(ctx, handle).is_none() {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    let size = ctx.mem.read_u32(chunk_at + 4).map_err(Stop::Fault)?;
    let data = ctx.mem.read_u32(chunk_at + 12).map_err(Stop::Fault)?;
    move_to(
        ctx,
        handle,
        data.saturating_add(size).saturating_add(size % 2),
    );
    Ok(0)
}

fn mmio_read(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioRead(file, into, count): how many bytes were read, or -1.
    let (handle, into, count) = (args[0], args[1], args[2]);
    let Some((base, len, at)) = mmio(ctx, handle) else {
        return Ok(u32::MAX);
    };
    let take = count.min(len.saturating_sub(at));
    let bytes = ctx
        .mem
        .slice(base + at, take as usize)
        .map_err(Stop::Fault)?
        .to_vec();
    ctx.mem.write(into, &bytes).map_err(Stop::Fault)?;
    move_to(ctx, handle, at + take);
    Ok(take)
}

fn mmio_seek(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioSeek(file, offset, origin): from the start, here, or the end; the
    // new position, or -1.
    let (handle, offset, origin) = (args[0], args[1] as i32, args[2]);
    let Some((_, len, at)) = mmio(ctx, handle) else {
        return Ok(u32::MAX);
    };
    let from = match origin {
        0 => 0,
        1 => i64::from(at),
        2 => i64::from(len),
        _ => return Ok(u32::MAX),
    };
    let to = from + i64::from(offset);
    if !(0..=i64::from(len)).contains(&to) {
        return Ok(u32::MAX);
    }
    move_to(ctx, handle, to as u32);
    Ok(to as u32)
}

/// Write a memory file's `MMIOINFO`: its whole memory is its buffer, read to
/// its end, with the next byte where the file is reading.
fn write_info(ctx: &mut Ctx<'_>, handle: u32, info: u32) -> Result<u32, Stop> {
    let Some((base, len, at)) = mmio(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    for (offset, value) in [
        (0, 0),
        (4, FOURCC_MEM),
        (20, len),
        (24, base),
        (28, base + at),
        (32, base + len),
        (36, base + len),
        (40, 0),
        (44, len),
        (68, handle),
    ] {
        ctx.mem
            .write_u32(info + offset, value)
            .map_err(Stop::Fault)?;
    }
    Ok(0)
}

/// Where a title's `MMIOINFO` says it has read to.
fn read_next(ctx: &mut Ctx<'_>, handle: u32, info: u32) -> Result<u32, Stop> {
    let Some((base, len, _)) = mmio(ctx, handle) else {
        return Ok(MMSYSERR_INVALHANDLE);
    };
    let next = ctx.mem.read_u32(info + 28).map_err(Stop::Fault)?;
    move_to(ctx, handle, next.saturating_sub(base).min(len));
    Ok(0)
}

fn mmio_get_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioGetInfo(file, info, flags): the buffer, for a title to read directly.
    write_info(ctx, args[0], args[1])
}

fn mmio_set_info(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioSetInfo(file, info, flags): how far the title read directly.
    read_next(ctx, args[0], args[1])
}

fn mmio_advance(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    // mmioAdvance(file, info, flags): the next buffer. A memory file's one
    // buffer is all of it, so past its end there is nothing more, and the
    // title finds its next byte still at the end.
    let (handle, info) = (args[0], args[1]);
    if read_next(ctx, handle, info)? != 0 {
        return Ok(MMSYSERR_INVALHANDLE);
    }
    write_info(ctx, handle, info)
}

fn mmio_close(ctx: &mut Ctx<'_>, args: &[u32]) -> Result<u32, Stop> {
    if let Some(Object::Mmio {
        base, owned: true, ..
    }) = ctx.state.object(args[0])
    {
        let base = *base;
        ctx.state.heap.free(base);
    }
    Ok(if ctx.state.close(args[0]) {
        0
    } else {
        MMSYSERR_INVALHANDLE
    })
}

#[cfg(test)]
mod tests {
    use super::{find_chunk, Chunk, Find};

    #[test]
    fn a_wave_files_chunks_are_found_by_form_and_by_id() {
        let mut wave = Vec::new();
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&30u32.to_le_bytes());
        wave.extend_from_slice(b"WAVE");
        wave.extend_from_slice(b"fmt ");
        wave.extend_from_slice(&3u32.to_le_bytes());
        wave.extend_from_slice(&[1, 2, 3, 0]); // three bytes, padded to four
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&2u32.to_le_bytes());
        wave.extend_from_slice(&[9, 9]);
        let fourcc = |s: &[u8; 4]| u32::from_le_bytes(*s);
        let riff = find_chunk(&wave, 0, 38, Find::Form(fourcc(b"RIFF"), fourcc(b"WAVE")));
        assert_eq!(
            riff,
            Some(Chunk {
                id: fourcc(b"RIFF"),
                size: 30,
                form: fourcc(b"WAVE"),
                data: 8
            })
        );
        // Inside the form, past the odd-sized format chunk, to the data.
        let data = find_chunk(&wave, 12, 38, Find::Chunk(fourcc(b"data")));
        assert_eq!(data.map(|c| (c.size, c.data)), Some((2, 32)));
        assert_eq!(
            find_chunk(&wave, 12, 38, Find::Chunk(fourcc(b"cue "))),
            None
        );
    }
}
