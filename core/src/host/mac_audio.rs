//! The Mac's audio device, for `--audio` (ADR-0001).
//!
//! AudioToolbox's audio queue, reached through `dlopen`. The machine mixes
//! whether or not this exists; it only takes the blocks the mix hands the host
//! and lets the queue pull them on its own thread.

use super::{sys, HostError};
use crate::sound::{BLOCK_FRAMES, RATE};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::{Mutex, MutexGuard, PoisonError};

const AUDIO_TOOLBOX: &str = "/System/Library/Frameworks/AudioToolbox.framework/AudioToolbox";

/// Bytes in one frame: two channels of sixteen-bit samples.
const FRAME_BYTES: u32 = 4;

/// `kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked`, and no
/// big-endian flag: the mix's own samples.
const SIGNED_PACKED_LITTLE_ENDIAN: u32 = 0x4 | 0x8;

/// Frames in one of the queue's buffers, about 23 ms at the mix's rate.
const BUFFER_FRAMES: u32 = 512;

/// Buffers the queue holds at once.
const BUFFERS: usize = 3;

/// Samples kept waiting before the oldest are dropped: two of the mix's
/// blocks, about 93 ms. The mix hands over a block at a time, so this much
/// comes and goes on its own; more than it means the run stalled and then
/// caught up, and what a person hears catches up with it.
const MOST_WAITING: usize = 2 * BLOCK_FRAMES * 2;

/// `AudioQueueDispose`'s "stop now, not after what is queued".
const IMMEDIATELY: u8 = 1;

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
type Refill = unsafe extern "C" fn(*mut c_void, Queue, *mut Buffer);
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

impl Shared {
    fn waiting(&self) -> MutexGuard<'_, VecDeque<i16>> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The Mac's default output, playing the mix.
pub struct MacAudio {
    queue: Queue,
    dispose: Dispose,
    /// Made with `Box::into_raw` and read by the queue's thread; given back to
    /// a box only once the queue is disposed of.
    shared: *mut Shared,
    _toolbox: sys::Library,
}

impl MacAudio {
    /// Open the default output and start it playing silence.
    ///
    /// # Errors
    /// When AudioToolbox is not there or the queue will not start.
    pub fn open() -> Result<Self, HostError> {
        let toolbox = sys::Library::open(AUDIO_TOOLBOX)?;
        // Safety: each type is the signature AudioQueue.h gives the function.
        let (new_output, allocate, enqueue, start, dispose) = unsafe {
            (
                toolbox.function::<NewOutput>("AudioQueueNewOutput")?,
                toolbox.function::<AllocateBuffer>("AudioQueueAllocateBuffer")?,
                toolbox.function::<EnqueueBuffer>("AudioQueueEnqueueBuffer")?,
                toolbox.function::<Start>("AudioQueueStart")?,
                toolbox.function::<Dispose>("AudioQueueDispose")?,
            )
        };
        let format = Format {
            sample_rate: f64::from(RATE),
            format_id: u32::from_be_bytes(*b"lpcm"),
            format_flags: SIGNED_PACKED_LITTLE_ENDIAN,
            bytes_per_packet: FRAME_BYTES,
            frames_per_packet: 1,
            bytes_per_frame: FRAME_BYTES,
            channels_per_frame: 2,
            bits_per_channel: 16,
            reserved: 0,
        };
        let shared = Box::into_raw(Box::new(Shared {
            waiting: Mutex::new(VecDeque::new()),
            enqueue,
        }));
        let mut queue: Queue = std::ptr::null_mut();
        // Safety: the queue copies the format; the shared state lives until
        // the queue is disposed of, and the queue calls back on its own thread
        // because no run loop is given.
        let made = unsafe {
            new_output(
                &format,
                refill,
                shared.cast(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
                &mut queue,
            )
        };
        if let Err(e) = status("AudioQueueNewOutput", made) {
            // Safety: no queue was made, so nothing else holds the pointer.
            drop(unsafe { Box::from_raw(shared) });
            return Err(e);
        }
        // From here the queue is disposed of if anything below fails.
        let audio = Self {
            queue,
            dispose,
            shared,
            _toolbox: toolbox,
        };
        for _ in 0..BUFFERS {
            let mut buffer: *mut Buffer = std::ptr::null_mut();
            // Safety: a live queue, and the buffer it hands back is its own,
            // with `capacity` bytes behind `data`. The first buffers are
            // silence; each one the queue finishes is refilled from what is
            // waiting.
            unsafe {
                status(
                    "AudioQueueAllocateBuffer",
                    allocate(queue, BUFFER_FRAMES * FRAME_BYTES, &mut buffer),
                )?;
                std::slice::from_raw_parts_mut((*buffer).data, (*buffer).capacity as usize).fill(0);
                (*buffer).size = (*buffer).capacity;
                status(
                    "AudioQueueEnqueueBuffer",
                    enqueue(queue, buffer, 0, std::ptr::null()),
                )?;
            }
        }
        // Safety: a live queue with buffers enqueued; a null time is now.
        status("AudioQueueStart", unsafe { start(queue, std::ptr::null()) })?;
        Ok(audio)
    }

    /// Take one block of the mix, to be heard as soon as the queue gets to it.
    pub fn play(&self, samples: &[i16]) {
        // Safety: the shared state lives as long as `self`.
        let shared = unsafe { &*self.shared };
        add_waiting(&mut shared.waiting(), samples);
    }
}

impl Drop for MacAudio {
    fn drop(&mut self) {
        // Safety: the queue is ours; disposing of it immediately stops its
        // thread calling back, after which the shared state is ours alone.
        unsafe {
            (self.dispose)(self.queue, IMMEDIATELY);
            drop(Box::from_raw(self.shared));
        }
    }
}

/// An `OSStatus` as a result.
fn status(what: &'static str, status: i32) -> Result<(), HostError> {
    if status == 0 {
        Ok(())
    } else {
        Err(HostError::Status { what, status })
    }
}

/// Fill a buffer the queue has finished with and hand it back.
///
/// A refused enqueue has nowhere to be reported from the queue's thread; the
/// queue then stops asking, and the run carries on silent.
///
/// # Safety
/// `user` must be the shared state and `buffer` one of `queue`'s own.
unsafe extern "C" fn refill(user: *mut c_void, queue: Queue, buffer: *mut Buffer) {
    // Safety: as the caller promises, with `capacity` bytes behind `data`.
    unsafe {
        let shared = &*user.cast::<Shared>();
        let buffer = &mut *buffer;
        let bytes = std::slice::from_raw_parts_mut(buffer.data, buffer.capacity as usize);
        fill_frames(&mut shared.waiting(), bytes);
        buffer.size = buffer.capacity;
        (shared.enqueue)(queue, buffer, 0, std::ptr::null());
    }
}

/// Add samples to those waiting, dropping the oldest whole frames past
/// [`MOST_WAITING`].
fn add_waiting(waiting: &mut VecDeque<i16>, samples: &[i16]) {
    waiting.extend(samples);
    let over = waiting.len().saturating_sub(MOST_WAITING) & !1;
    waiting.drain(..over);
}

/// Fill `bytes` with whole frames of what is waiting, and silence where there
/// is not a whole frame to take, so the channels never swap.
fn fill_frames(waiting: &mut VecDeque<i16>, bytes: &mut [u8]) {
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
    use super::{add_waiting, fill_frames, MacAudio, MOST_WAITING};
    use std::collections::VecDeque;

    #[test]
    fn a_buffer_takes_whole_frames_and_is_silent_past_them() {
        let mut waiting = VecDeque::from([1, -1, 2, -2, 3]);
        let mut bytes = [0xaa; 12];
        fill_frames(&mut waiting, &mut bytes);
        assert_eq!(bytes, [1, 0, 0xff, 0xff, 2, 0, 0xfe, 0xff, 0, 0, 0, 0]);
        assert_eq!(waiting, [3], "half a frame waits for its other half");
    }

    #[test]
    fn a_run_that_stalls_and_catches_up_drops_its_oldest_frames() {
        let mut waiting = VecDeque::new();
        let block: Vec<i16> = (0..2048).collect();
        for _ in 0..5 {
            add_waiting(&mut waiting, &block);
        }
        assert_eq!(waiting.len(), MOST_WAITING);
        assert_eq!(waiting.back(), Some(&2047), "the newest is kept");
        assert_eq!(waiting.front().map(|s| s % 2), Some(0), "a frame's left");
    }

    #[test]
    #[ignore = "starts the real audio device; run with --ignored to check the queue"]
    fn the_device_pulls_what_it_is_given() {
        let audio = MacAudio::open().expect("the default output opens");
        // Silence: a tenth of a second of it.
        audio.play(&[0; 4410]);
        // Safety: the shared state lives as long as `audio`.
        let shared = unsafe { &*audio.shared };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !shared.waiting().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(shared.waiting().is_empty(), "the queue took the samples");
    }
}
