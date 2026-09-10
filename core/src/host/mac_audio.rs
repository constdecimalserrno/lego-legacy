//! The Mac's audio device, for `--audio` (ADR-0001).
//!
//! AudioToolbox's audio queue, reached through `dlopen`. The machine mixes
//! whether or not this exists; it only takes the blocks the mix hands the host
//! and lets the queue pull them on its own thread.

use super::{sys, HostError};
use crate::sound::RATE;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Mutex, PoisonError};

const AUDIO_TOOLBOX: &str = "/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox";

/// Bytes in one frame: two channels of sixteen-bit samples.
const FRAME_BYTES: u32 = 4;

/// Frames in one of the queue's buffers, about 23 ms at the mix's rate.
const BUFFER_FRAMES: u32 = 512;

/// Buffers the queue holds at once: how far behind the mix a person hears.
const BUFFERS: usize = 3;

/// Samples kept waiting before the oldest are dropped, a quarter of a second:
/// a run that falls behind the wall clock plays late, not ever later.
const MOST_WAITING: usize = RATE as usize / 4 * 2;

/// An `AudioStreamBasicDescription`.
#[repr(C)]
struct Format {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

/// An `AudioQueueBuffer`.
#[repr(C)]
struct Buffer {
    capacity: u32,
    data: *mut u8,
    size: u32,
    user_data: *mut c_void,
    packet_capacity: u32,
    packets: *mut c_void,
    packet_count: u32,
}

type Queue = *mut c_void;
type Refill = extern "C" fn(*mut c_void, Queue, *mut Buffer);
type NewOutput = unsafe extern "C" fn(
    *const Format,
    Refill,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    u32,
    *mut Queue,
) -> i32;
type AllocateBuffer = unsafe extern "C" fn(Queue, u32, *mut *mut Buffer) -> i32;
type EnqueueBuffer = unsafe extern "C" fn(Queue, *mut Buffer, u32, *const c_void) -> i32;
type Start = unsafe extern "C" fn(Queue, *const c_void) -> i32;
type Dispose = unsafe extern "C" fn(Queue, u8) -> i32;

/// What the queue's thread and the run share: the samples waiting to be heard.
struct Shared {
    waiting: Mutex<VecDeque<i16>>,
    enqueue: EnqueueBuffer,
}

/// The Mac's default output, playing the mix.
pub struct MacAudio {
    queue: Queue,
    dispose: Dispose,
    /// Read by the queue's thread until the queue is disposed, so it is
    /// dropped after.
    shared: Box<Shared>,
    _toolbox: sys::Library,
}

impl MacAudio {
    /// Open the default output and start it playing silence.
    ///
    /// # Errors
    /// When AudioToolbox is not there or the queue will not start.
    pub fn open() -> Result<Self, HostError> {
        let toolbox = sys::Library::open(AUDIO_TOOLBOX)?;
        // Safety: each symbol is cast to the signature AudioQueue.h gives it.
        let (new_output, allocate, enqueue, start, dispose) = unsafe {
            (
                std::mem::transmute::<*mut c_void, NewOutput>(
                    toolbox.symbol("AudioQueueNewOutput")?,
                ),
                std::mem::transmute::<*mut c_void, AllocateBuffer>(
                    toolbox.symbol("AudioQueueAllocateBuffer")?,
                ),
                std::mem::transmute::<*mut c_void, EnqueueBuffer>(
                    toolbox.symbol("AudioQueueEnqueueBuffer")?,
                ),
                std::mem::transmute::<*mut c_void, Start>(toolbox.symbol("AudioQueueStart")?),
                std::mem::transmute::<*mut c_void, Dispose>(toolbox.symbol("AudioQueueDispose")?),
            )
        };
        let format = Format {
            sample_rate: f64::from(RATE),
            format_id: u32::from_be_bytes(*b"lpcm"),
            // Signed integers, packed, little-endian: what the mix makes.
            format_flags: 0x4 | 0x8,
            bytes_per_packet: FRAME_BYTES,
            frames_per_packet: 1,
            bytes_per_frame: FRAME_BYTES,
            channels_per_frame: 2,
            bits_per_channel: 16,
            reserved: 0,
        };
        let shared = Box::new(Shared {
            waiting: Mutex::new(VecDeque::new()),
            enqueue,
        });
        let mut queue: Queue = std::ptr::null_mut();
        // Safety: the format and the shared state outlive the queue, which
        // calls back on its own thread because no run loop is given.
        let status = unsafe {
            new_output(
                &format,
                refill,
                std::ptr::from_ref::<Shared>(&shared).cast_mut().cast(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut queue,
            )
        };
        checked("AudioQueueNewOutput", status)?;
        // From here the queue is disposed of if anything below fails.
        let audio = Self {
            queue,
            dispose,
            shared,
            _toolbox: toolbox,
        };
        for _ in 0..BUFFERS {
            let mut buffer: *mut Buffer = std::ptr::null_mut();
            // Safety: a live queue; the buffer it hands back is its own.
            checked("AudioQueueAllocateBuffer", unsafe {
                allocate(audio.queue, BUFFER_FRAMES * FRAME_BYTES, &mut buffer)
            })?;
            // The first buffers are silence; each one the queue finishes is
            // refilled from what is waiting.
            refill(
                std::ptr::from_ref::<Shared>(&audio.shared)
                    .cast_mut()
                    .cast(),
                audio.queue,
                buffer,
            );
        }
        // Safety: a live queue with buffers enqueued; a null time is now.
        checked("AudioQueueStart", unsafe {
            start(audio.queue, std::ptr::null())
        })?;
        Ok(audio)
    }

    /// Take one block of the mix, to be heard as soon as the queue gets to it.
    pub fn play(&self, samples: &[i16]) {
        let mut waiting = self
            .shared
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        keep(&mut waiting, samples);
    }

    /// How many samples are still waiting to be heard.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.shared
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

impl Drop for MacAudio {
    fn drop(&mut self) {
        // Safety: disposing of the queue we made, at once, so its thread has
        // stopped calling back before the shared state goes.
        unsafe {
            (self.dispose)(self.queue, 1);
        }
    }
}

fn checked(what: &'static str, status: i32) -> Result<(), HostError> {
    if status == 0 {
        Ok(())
    } else {
        Err(HostError::Status { what, status })
    }
}

/// Fill a buffer the queue has finished with and hand it back.
extern "C" fn refill(user: *mut c_void, queue: Queue, buffer: *mut Buffer) {
    // Safety: `user` is the shared state, alive until the queue is disposed,
    // and `buffer` is one of this queue's own, with `capacity` bytes behind
    // `data`.
    unsafe {
        let shared = &*user.cast::<Shared>();
        let buffer = &mut *buffer;
        let bytes = std::slice::from_raw_parts_mut(buffer.data, buffer.capacity as usize);
        let mut waiting = shared
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        take(&mut waiting, bytes);
        drop(waiting);
        buffer.size = buffer.capacity;
        (shared.enqueue)(queue, buffer, 0, std::ptr::null());
    }
}

/// Add samples to those waiting, dropping the oldest whole frames past
/// [`MOST_WAITING`].
fn keep(waiting: &mut VecDeque<i16>, samples: &[i16]) {
    waiting.extend(samples);
    let over = waiting.len().saturating_sub(MOST_WAITING) & !1;
    waiting.drain(..over);
}

/// Fill `bytes` with whole frames of what is waiting, and silence where there
/// is not a whole frame to take, so the channels never swap.
fn take(waiting: &mut VecDeque<i16>, bytes: &mut [u8]) {
    for frame in bytes.as_chunks_mut::<{ FRAME_BYTES as usize }>().0 {
        let (left, right) = match (waiting.front(), waiting.get(1)) {
            (Some(&left), Some(&right)) => {
                waiting.drain(..2);
                (left, right)
            }
            _ => (0, 0),
        };
        frame[..2].copy_from_slice(&left.to_le_bytes());
        frame[2..].copy_from_slice(&right.to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::{keep, take, MacAudio, MOST_WAITING};
    use std::collections::VecDeque;

    #[test]
    fn a_buffer_takes_whole_frames_and_is_silent_past_them() {
        let mut waiting = VecDeque::from([1, -1, 2, -2, 3]);
        let mut bytes = [0xaa; 12];
        take(&mut waiting, &mut bytes);
        assert_eq!(bytes, [1, 0, 0xff, 0xff, 2, 0, 0xfe, 0xff, 0, 0, 0, 0]);
        assert_eq!(waiting, [3], "half a frame waits for its other half");
    }

    #[test]
    fn a_run_that_falls_behind_drops_its_oldest_frames() {
        let mut waiting = VecDeque::new();
        let block: Vec<i16> = (0..2048).collect();
        for _ in 0..(MOST_WAITING / block.len() + 2) {
            keep(&mut waiting, &block);
        }
        assert!(waiting.len() <= MOST_WAITING);
        assert_eq!(waiting.len() % 2, 0);
        assert_eq!(waiting.back(), Some(&2047), "the newest is kept");
        assert_eq!(waiting.front().map(|s| s % 2), Some(0), "a frame's left");
    }

    #[test]
    fn the_device_pulls_what_it_is_given() {
        let Ok(audio) = MacAudio::open() else {
            println!("skipping: no audio device to open");
            return;
        };
        // Silence, so a test run stays silent: a tenth of a second of it.
        audio.play(&[0; 4410]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while audio.waiting() > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(audio.waiting(), 0, "the queue took the samples");
    }
}
