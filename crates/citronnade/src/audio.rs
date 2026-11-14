//! Sound through the DSP (the "NDSP" driver): 24 channels that play
//! queued buffers of samples, mixed by the DSP at 32728 Hz.
//!
//! ```ignore
//! let audio = Audio::new()?;               // loads /3ds/dspfirm.cdc
//! let ch = audio.channel(0);
//! ch.set_format(SampleFormat::Pcm16Mono);
//! ch.set_sample_rate(OUTPUT_RATE);
//! let mut buf = WaveBuf::new(4096, 1);
//! fill(buf.samples_mut().unwrap());
//! let buf = Arc::new(buf);
//! ch.queue(&buf);                          // plays it; Arc::get_mut works again once done
//! ```
//!
//! The DSP firmware is not part of the system: the user dumps their own
//! console's (with the DSP1 homebrew) to `/3ds/dspfirm.cdc`.
//!
//! Every audio frame (160 samples, about 4.9 ms) a high-priority thread
//! waits for the DSP, reads each channel's state (which buffers finished)
//! and writes the next configuration (rates, mixes, buffers to play) into
//! the DSP's shared memory, double-buffered by frame. The structures and
//! the sequence follow libctru's NDSP.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering};

use citronnade_rt::sync::{Event, Mutex};
use citronnade_rt::thread::{Builder, JoinHandle};
use citronnade_rt::{heap, memory, svc};
use citronnade_services::dsp;

use crate::ResultCode;

/// The DSP's output sample rate.
pub const OUTPUT_RATE: f32 = 32728.498;
/// Samples per audio frame.
pub const FRAME_SAMPLES: usize = 160;
/// Number of channels.
pub const CHANNELS: usize = 24;

const FIRMWARE_PATH: &str = "/3ds/dspfirm.cdc";

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct AdpcmData {
    index: u16,
    history0: i16,
    history1: i16,
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct DspChnBuf {
    paddr: u32,
    sample_count: u32,
    adpcm: AdpcmData,
    has_adpcm: u8,
    looping: u8,
    seq_id: u16,
    padding: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct DspChnStruct {
    flags: u32,
    mix: [f32; 12],
    rate: f32,
    rim: [u8; 2],
    iir_type: u16,
    iir_mono: [u16; 2],
    iir_biquad: [u16; 5],
    active_buffers: u16,
    buffers: [DspChnBuf; 4],
    _pad0: u32,
    play_status: u16,
    sync_count: u16,
    unknown: u32,
    _pad1: u32,
    paddr: u32,
    sample_count: u32,
    cnt_flags: u16,
    adpcm: AdpcmData,
    more_flags: u16,
    seq_id: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct DspChnStatus {
    flags: u16,
    sync_count: u16,
    sample_pos: u32,
    cur_seq: u16,
    last_seq: u16,
}

#[derive(Clone, Copy)]
#[repr(C)]
struct DspMasterStatus {
    flags: u32,
    master_vol: f32,
    aux_return_vol: [f32; 2],
    out_buf_count: u16,
    _pad0: [u16; 2],
    output_mode: u16,
    clipping_mode: u16,
    headset: u16,
    surround_depth: u16,
    surround_pos: u16,
    _pad1: u16,
    rear_ratio: u16,
    aux_front_bypass: [u16; 2],
    aux_bus_enable: [u16; 2],
    delay: [[u16; 10]; 2],
    reverb: [[u16; 26]; 2],
    sync_mode: u16,
    _pad2: u16,
    unknown: u32,
}

const _: () = assert!(core::mem::size_of::<DspChnStruct>() == 0xC0);
const _: () = assert!(core::mem::size_of::<DspChnStatus>() == 0xC);

// Per-channel dirty flags.
const C_INIT: u32 = 1 << 0;
const C_SYNC: u32 = 1 << 1;
const C_PLAY: u32 = 1 << 2;
const C_INTERP: u32 = 1 << 3;
const C_IIRTYPE: u32 = 1 << 4;
const C_RATE: u32 = 1 << 5;
const C_MIX: u32 = 1 << 6;
const C_ADPCM: u32 = 1 << 7;

/// Sample formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Pcm8Mono,
    Pcm8Stereo,
    Pcm16Mono,
    Pcm16Stereo,
}

impl SampleFormat {
    fn bits(self) -> u16 {
        match self {
            SampleFormat::Pcm8Mono => 1,
            SampleFormat::Pcm8Stereo => 2,
            SampleFormat::Pcm16Mono => 1 | (1 << 2),
            SampleFormat::Pcm16Stereo => 2 | (1 << 2),
        }
    }
}

/// How the DSP resamples a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Polyphase = 0,
    Linear = 1,
    None = 2,
}

/// A wave buffer's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferStatus {
    Free,
    Queued,
    Playing,
    Done,
}

/// Samples in linear memory, playable by a channel. Share it with
/// [`Channel::queue`] through an `Arc`; while queued, `Arc::get_mut` fails,
/// so it cannot be changed under the DSP's feet.
pub struct WaveBuf {
    ptr: NonNull<u8>,
    bytes: usize,
    frames: u32,
    channels: u8,
    looping: bool,
    status: AtomicU8,
    sequence: AtomicU16,
}

unsafe impl Send for WaveBuf {}
unsafe impl Sync for WaveBuf {}

impl WaveBuf {
    /// A silent buffer of 16-bit samples (`frames` per channel).
    pub fn new(frames: usize, channels: u8) -> WaveBuf {
        let bytes = (frames * channels as usize * 2).max(4);
        let layout = Layout::from_size_align(bytes, 32).unwrap();
        let ptr = heap::linear_alloc(layout).expect("out of linear memory for a wave buffer");
        unsafe { ptr.as_ptr().write_bytes(0, bytes) };
        WaveBuf { ptr, bytes, frames: frames as u32, channels, looping: false, status: AtomicU8::new(0), sequence: AtomicU16::new(0) }
    }

    /// A buffer holding a copy of `samples` (interleaved when stereo).
    pub fn from_samples(samples: &[i16], channels: u8) -> WaveBuf {
        let mut b = WaveBuf::new(samples.len() / channels as usize, channels);
        let n = b.frames as usize * channels as usize;
        b.samples_mut().copy_from_slice(&samples[..n]);
        b
    }

    /// The samples (interleaved when stereo).
    pub fn samples_mut(&mut self) -> &mut [i16] {
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr().cast(), self.frames as usize * self.channels as usize) }
    }

    pub fn samples(&self) -> &[i16] {
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr().cast(), self.frames as usize * self.channels as usize) }
    }

    /// Frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.frames as usize
    }

    /// Plays only the first `frames` frames.
    pub fn set_frames(&mut self, frames: usize) {
        self.frames = (frames.min(self.bytes / 2 / self.channels as usize)) as u32;
    }

    /// Loops until the channel is reset.
    pub fn set_looping(&mut self, looping: bool) {
        self.looping = looping;
    }

    pub fn status(&self) -> BufferStatus {
        match self.status.load(Ordering::Acquire) {
            1 => BufferStatus::Queued,
            2 => BufferStatus::Playing,
            3 => BufferStatus::Done,
            _ => BufferStatus::Free,
        }
    }

    /// Whether the DSP is done with it (or it was never queued).
    pub fn is_free(&self) -> bool {
        matches!(self.status(), BufferStatus::Free | BufferStatus::Done)
    }

    fn set_status(&self, s: BufferStatus) {
        self.status.store(
            match s {
                BufferStatus::Free => 0,
                BufferStatus::Queued => 1,
                BufferStatus::Playing => 2,
                BufferStatus::Done => 3,
            },
            Ordering::Release,
        );
    }
}

impl Drop for WaveBuf {
    fn drop(&mut self) {
        unsafe { heap::linear_free(self.ptr, Layout::from_size_align(self.bytes, 32).unwrap()) };
    }
}

struct ChannelState {
    flags: u32,
    sync_count: u16,
    sample_pos: u32,
    queue: VecDeque<Arc<WaveBuf>>,
    in_dsp: usize,
    id_next: u16,
    playing: bool,
    paused: bool,
    interp: u8,
    format: u16,
    seq: u16,
    rate: f32,
    mix: [f32; 12],
}

impl ChannelState {
    const fn new() -> ChannelState {
        ChannelState {
            flags: 0,
            sync_count: 0,
            sample_pos: 0,
            queue: VecDeque::new(),
            in_dsp: 0,
            id_next: 0,
            playing: false,
            paused: false,
            interp: 0,
            format: 5,
            seq: 0,
            rate: 1.0,
            mix: [0.0; 12],
        }
    }

    fn reset(&mut self) {
        self.flags = !0;
        self.sync_count = self.sync_count.wrapping_add(1);
        self.sample_pos = 0;
        for b in self.queue.drain(..) {
            b.set_status(BufferStatus::Done);
        }
        self.in_dsp = 0;
        self.id_next = 0;
        self.seq = 0;
        self.playing = false;
        self.paused = false;
        self.interp = 0;
        self.format = SampleFormat::Pcm16Mono.bits();
        self.rate = 1.0;
        self.mix = [0.0; 12];
        self.mix[0] = 1.0;
        self.mix[1] = 1.0;
    }
}

struct Master {
    flags: u32,
    volume: f32,
    output_mode: u16,
    clipping_mode: u16,
    output_count: u16,
}

static CHANNEL_STATE: [Mutex<ChannelState>; CHANNELS] = [const { Mutex::new(ChannelState::new()) }; CHANNELS];
static MASTER: Mutex<Master> = Mutex::new(Master { flags: !0, volume: 1.0, output_mode: 1, clipping_mode: 1, output_count: 2 });
static VARS: [[AtomicU32; 2]; 16] = [const { [AtomicU32::new(0), AtomicU32::new(0)] }; 16];
static FRAME_ID: AtomicU16 = AtomicU16::new(0);
static BUFFER_CUR_ID: AtomicU16 = AtomicU16::new(0);
static BUFFER_ID: AtomicU16 = AtomicU16::new(0);
static IRQ_EVENT: AtomicU32 = AtomicU32::new(0);
static SEMAPHORE: AtomicU32 = AtomicU32::new(0);
static READY: AtomicBool = AtomicBool::new(false);
static ENTERING_SLEEP: AtomicBool = AtomicBool::new(false);
static SLEEPING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static FRAMES: AtomicU32 = AtomicU32::new(0);
static DROPPED: AtomicU32 = AtomicU32::new(0);
static SLEEP_EVENT: Event = Event::sticky();
static IRQ_LOCK: Mutex<()> = Mutex::new(());
static BACKUP: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static THREAD: Mutex<Option<JoinHandle>> = Mutex::new(None);
static ALIVE: AtomicBool = AtomicBool::new(false);

fn var(i: usize, which: usize) -> u32 {
    VARS[i][which].load(Ordering::Acquire)
}

fn set_counter(which: usize, value: u16) {
    unsafe { (var(0, which) as *mut u16).write_volatile(value) };
}

fn counter(which: usize) -> u16 {
    unsafe { (var(0, which) as *const u16).read_volatile() }
}

fn rot(x: u32) -> u32 {
    x.rotate_left(16)
}

/// Why audio could not start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    /// `/3ds/dspfirm.cdc` is missing (dump it with the DSP1 homebrew).
    NoFirmware,
    /// The DSP refused the firmware.
    FirmwareRejected,
    /// Audio is already running.
    AlreadyRunning,
    System(ResultCode),
}

impl From<ResultCode> for AudioError {
    fn from(e: ResultCode) -> Self {
        AudioError::System(e)
    }
}

impl core::fmt::Display for AudioError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AudioError::NoFirmware => write!(f, "the DSP firmware {FIRMWARE_PATH} is missing (dump it with DSP1)"),
            AudioError::FirmwareRejected => f.write_str("the DSP refused the firmware"),
            AudioError::AlreadyRunning => f.write_str("audio is already running"),
            AudioError::System(e) => write!(f, "system error {e}"),
        }
    }
}

fn wait_irq(timeout_ns: i64) -> bool {
    let _g = IRQ_LOCK.lock();
    let irq = IRQ_EVENT.load(Ordering::Acquire);
    if irq == 0 {
        return false;
    }
    match svc::wait_synchronization(irq, timeout_ns) {
        Ok(()) => {
            let _ = svc::clear_event(irq);
            true
        }
        Err(e) if e.description() == 0x3FE => false, // timed out
        Err(_) => svc::break_(svc::break_reason::PANIC),
    }
}

fn initialize(resume: bool) -> Result<(), ResultCode> {
    let irq = svc::create_event(svc::ResetType::Sticky)?;
    IRQ_EVENT.store(irq, Ordering::Release);
    dsp::register_interrupt_events(irq, dsp::INTERRUPT_PIPE, 2)?;
    SEMAPHORE.store(dsp::get_semaphore_handle()?, Ordering::Release);
    dsp::set_semaphore_mask(0x2000)?;
    if resume {
        if let Some(b) = BACKUP.lock().as_ref() {
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), var(5, 0) as *mut u8, b.len()) };
        }
    }
    let val: u32 = if resume { 2 } else { 0 };
    dsp::write_process_pipe(2, &val.to_le_bytes())?;
    dsp::set_semaphore(0x4000)?;
    wait_irq(-1);
    let mut count = [0u8; 2];
    dsp::read_pipe_if_possible(2, 0, &mut count)?;
    let count = u16::from_le_bytes(count) as usize;
    let mut addrs = [0u8; 32];
    dsp::read_pipe_if_possible(2, 0, &mut addrs[..count.min(16) * 2])?;
    for i in 0..count.min(16) {
        let a = u16::from_le_bytes([addrs[2 * i], addrs[2 * i + 1]]) as u32;
        VARS[i][0].store(dsp::convert_process_address_from_dsp_dram(a)?, Ordering::Release);
        VARS[i][1].store(dsp::convert_process_address_from_dsp_dram(a | 0x10000)?, Ordering::Release);
    }
    dsp::set_semaphore(0x4000)?;
    FRAMES.store(0, Ordering::Release);
    set_counter(0, 4);
    FRAME_ID.store(5, Ordering::Release);
    let _ = svc::signal_event(SEMAPHORE.load(Ordering::Acquire));
    BUFFER_CUR_ID.store(1, Ordering::Release);
    BUFFER_ID.store(1, Ordering::Release);
    READY.store(true, Ordering::Release);
    if resume {
        for c in &CHANNEL_STATE {
            c.lock().flags |= !C_INIT;
        }
        update_channels();
        MASTER.lock().flags = !0;
        update_master();
    }
    Ok(())
}

fn finalize(suspend: bool) {
    let val: u32 = if suspend { 3 } else { 1 };
    let _ = dsp::write_process_pipe(2, &val.to_le_bytes());
    loop {
        if dsp::recv_data_is_ready(0).unwrap_or(false) && dsp::recv_data(0).unwrap_or(0) == 1 {
            break;
        }
        svc::sleep_thread(4_888_000);
    }
    if suspend {
        let mut backup = alloc::vec![0u8; 0x1080];
        unsafe { core::ptr::copy_nonoverlapping(var(5, 0) as *const u8, backup.as_mut_ptr(), backup.len()) };
        *BACKUP.lock() = Some(backup);
    }
    let _g = IRQ_LOCK.lock();
    READY.store(false, Ordering::Release);
    let irq = IRQ_EVENT.swap(0, Ordering::AcqRel);
    if irq != 0 {
        let _ = svc::close_handle(irq);
    }
    let _ = dsp::register_interrupt_events(0, dsp::INTERRUPT_PIPE, 2);
    let sem = SEMAPHORE.swap(0, Ordering::AcqRel);
    if sem != 0 {
        let _ = svc::close_handle(sem);
    }
}

fn on_dsp_hook(h: dsp::Hook) {
    match h {
        dsp::Hook::Sleep => {
            if !SLEEPING.load(Ordering::Acquire) {
                ENTERING_SLEEP.store(true, Ordering::Release);
                finalize(true);
                SLEEPING.store(true, Ordering::Release);
            }
        }
        dsp::Hook::Wakeup => {
            if SLEEPING.load(Ordering::Acquire) {
                if initialize(true).is_err() {
                    svc::break_(svc::break_reason::PANIC);
                }
                SLEEPING.store(false, Ordering::Release);
                ENTERING_SLEEP.store(false, Ordering::Release);
                SLEEP_EVENT.signal();
            }
        }
        dsp::Hook::Cancel => {
            if SLEEPING.load(Ordering::Acquire) {
                CANCELLED.store(true, Ordering::Release);
                SLEEPING.store(false, Ordering::Release);
                ENTERING_SLEEP.store(false, Ordering::Release);
                SLEEP_EVENT.signal();
            }
        }
    }
}

fn sync() {
    let mut receive_now = false;
    if ENTERING_SLEEP.load(Ordering::Acquire) {
        if wait_irq(9_776_000) {
            receive_now = true;
        } else {
            SLEEPING.store(true, Ordering::Release);
        }
    }
    if !receive_now {
        if SLEEPING.load(Ordering::Acquire) {
            SLEEP_EVENT.wait();
            SLEEP_EVENT.clear();
        }
        if CANCELLED.load(Ordering::Acquire) {
            svc::sleep_thread(4_888_000);
            return;
        }
        wait_irq(-1);
    }
    if READY.load(Ordering::Acquire) {
        let frame = FRAME_ID.load(Ordering::Acquire);
        let c = counter((!frame & 1) as usize);
        if c != 0 {
            let next = c.wrapping_add(1);
            let frame = if next != 0 { next } else { 2 };
            FRAME_ID.store(frame, Ordering::Release);
            BUFFER_ID.store(frame & 1, Ordering::Release);
            read_channel_states();
            let dropped = unsafe { (var(5, (frame & 1) as usize) as *const u16).add(1).read_volatile() };
            DROPPED.fetch_add(dropped as u32, Ordering::Relaxed);
        }
    }
}

fn update_master() {
    let which = BUFFER_CUR_ID.load(Ordering::Acquire) as usize;
    let p = var(4, which) as *mut DspMasterStatus;
    if p.is_null() {
        return;
    }
    let mut m = MASTER.lock();
    let mut st = unsafe { p.read_volatile() };
    let mut flags = st.flags | 0x1000_0000;
    st.headset = crate::info::headphones() as u16;
    if m.flags & 1 != 0 {
        st.master_vol = m.volume;
        flags |= 0x0001_0000;
    }
    if m.flags & 2 != 0 {
        st.output_mode = m.output_mode;
        flags |= 0x0400_0000;
    }
    if m.flags & 4 != 0 {
        st.clipping_mode = m.clipping_mode;
        flags |= 0x0800_0000;
    }
    if m.flags & 8 != 0 {
        st.out_buf_count = m.output_count;
        flags |= 0x0000_8000;
    }
    if m.flags & (1 << 5) != 0 {
        st.surround_depth = 0x7FFF;
        flags |= 0x2000_0000;
    }
    if m.flags & (1 << 7) != 0 {
        st.rear_ratio = 0x8000;
        flags |= 0x8000_0000;
    }
    st.flags = flags;
    unsafe { p.write_volatile(st) };
    m.flags = 0;
}

fn update_channels() {
    let frame = FRAME_ID.load(Ordering::Acquire);
    if var(1, 0) == 0 {
        return;
    }
    for (i, slot) in CHANNEL_STATE.iter().enumerate() {
        let p = unsafe { (var(1, (frame & 1) as usize) as *mut DspChnStruct).add(i) };
        let mut chn = slot.lock();
        let mut st = unsafe { p.read_volatile() };
        let mut flags = chn.flags;
        let mut stflags = st.flags;
        if flags & C_INIT != 0 {
            stflags |= 0x2000_0000;
        }
        if flags & C_MIX != 0 {
            st.mix = chn.mix;
            stflags |= 0x0E00_0000;
        }
        if flags & C_RATE != 0 {
            st.rate = chn.rate;
            stflags |= 0x40000;
            if chn.interp == 0 {
                flags |= C_INTERP;
            }
        }
        if flags & C_IIRTYPE != 0 {
            st.iir_type = 0;
            stflags |= 0x40_0000;
        }
        if flags & C_INTERP != 0 {
            st.rim[0] = chn.interp;
            st.rim[1] = if chn.interp == 0 {
                if chn.rate <= 1.0 { 2 } else if chn.rate <= 4.0 / 3.0 { 1 } else { 0 }
            } else {
                1
            };
            stflags |= 0x20000;
        }
        if flags & C_ADPCM != 0 {
            stflags |= 4;
        }
        if !chn.queue.is_empty() && !chn.playing {
            chn.playing = true;
            flags |= C_PLAY;
        }
        let mut j = chn.in_dsp;
        while j < chn.queue.len() && j < 5 {
            let wb = chn.queue[j].clone();
            let seq = wb.sequence.load(Ordering::Acquire);
            let paddr = rot(memory::virt_to_phys(wb.ptr.as_ptr() as usize));
            let count = rot(wb.frames);
            if chn.in_dsp == 0 {
                wb.set_status(BufferStatus::Playing);
                chn.id_next = 0;
                st.seq_id = seq;
                st.sample_count = count;
                st.paddr = paddr;
                st.cnt_flags = chn.format;
                st.more_flags = (st.more_flags & !2) | if wb.looping { 2 } else { 0 };
                st.unknown = 0;
                stflags |= 0x10 | 0x4020_0000;
            } else {
                let k = chn.id_next as usize;
                st.buffers[k] = DspChnBuf {
                    paddr,
                    sample_count: count,
                    adpcm: AdpcmData::default(),
                    has_adpcm: 0,
                    looping: wb.looping as u8,
                    seq_id: seq,
                    padding: 0,
                };
                st.active_buffers |= 1 << k;
                chn.id_next = (chn.id_next + 1) & 3;
                stflags |= 0x80000;
            }
            chn.in_dsp += 1;
            j += 1;
        }
        if flags & C_SYNC != 0 {
            st.sync_count = chn.sync_count;
            stflags |= 0x1000_0000;
        }
        if flags & C_PLAY != 0 {
            st.play_status = (st.play_status & !0xFF) | (chn.playing && !chn.paused) as u16;
            stflags |= 0x10000;
        }
        chn.flags = 0;
        st.flags = stflags;
        unsafe { p.write_volatile(st) };
    }
}

fn read_channel_states() {
    let which = BUFFER_ID.load(Ordering::Acquire) as usize;
    for (i, slot) in CHANNEL_STATE.iter().enumerate() {
        let st = unsafe { (var(2, which) as *const DspChnStatus).add(i).read_volatile() };
        let mut chn = slot.lock();
        if chn.sync_count != st.sync_count {
            continue;
        }
        chn.sample_pos = rot(st.sample_pos);
        if st.flags & 0xFF00 != 0 {
            let seq = st.cur_seq;
            while chn.in_dsp > 0 {
                let Some(front) = chn.queue.front() else { break };
                let front_seq = front.sequence.load(Ordering::Acquire);
                if front_seq == seq {
                    front.set_status(BufferStatus::Playing);
                    break;
                }
                chn.in_dsp -= 1;
                if let Some(done) = chn.queue.pop_front() {
                    done.set_status(BufferStatus::Done);
                }
                if seq == 0 && (front_seq == st.last_seq || st.last_seq == 0) {
                    break;
                }
            }
            if seq == 0 {
                chn.in_dsp = 0;
            }
        }
        chn.playing = st.flags & 0xFF == 1;
    }
}

fn thread_main() {
    while RUNNING.load(Ordering::Acquire) {
        sync();
        if SLEEPING.load(Ordering::Acquire) || CANCELLED.load(Ordering::Acquire) || !READY.load(Ordering::Acquire) {
            continue;
        }
        update_master();
        update_channels();
        let frame = FRAME_ID.load(Ordering::Acquire);
        set_counter(BUFFER_CUR_ID.load(Ordering::Acquire) as usize, frame);
        let next = frame.wrapping_add(1);
        FRAME_ID.store(next, Ordering::Release);
        let _ = svc::signal_event(SEMAPHORE.load(Ordering::Acquire));
        BUFFER_CUR_ID.store(next & 1, Ordering::Release);
        FRAMES.fetch_add(1, Ordering::Relaxed);
    }
}

/// The audio system. Dropping it stops audio.
pub struct Audio {
    _private: (),
}

/// How the console mixes to its speakers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputMode {
    Mono = 0,
    Stereo = 1,
    Surround = 2,
}

impl Audio {
    /// Starts audio: loads the DSP firmware from `/3ds/dspfirm.cdc`.
    pub fn new() -> Result<Audio, AudioError> {
        if ALIVE.swap(true, Ordering::AcqRel) {
            return Err(AudioError::AlreadyRunning);
        }
        let result = Self::start();
        if result.is_err() {
            ALIVE.store(false, Ordering::Release);
        }
        result
    }

    fn start() -> Result<Audio, AudioError> {
        let firmware = crate::fs::read(FIRMWARE_PATH).map_err(|_| AudioError::NoFirmware)?;
        SLEEP_EVENT.clear();
        dsp::init()?;
        let started = (|| -> Result<(), AudioError> {
            if !dsp::load_component(firmware, 0xFF, 0xFF)? {
                return Err(AudioError::FirmwareRejected);
            }
            initialize(false)?;
            for c in &CHANNEL_STATE {
                let mut c = c.lock();
                c.sync_count = 0;
                c.reset();
            }
            {
                let mut m = MASTER.lock();
                *m = Master { flags: !0, volume: 1.0, output_mode: OutputMode::Stereo as u16, clipping_mode: 1, output_count: 2 };
                let mut mode = [0u8; 1];
                if citronnade_services::cfg::config_block(0x70001, &mut mode).is_ok() {
                    m.output_mode = mode[0] as u16;
                }
            }
            update_master();
            RUNNING.store(true, Ordering::Release);
            let t = Builder { stack_size: 16 * 1024, priority: 0x18, core: -2 }.spawn(thread_main)?;
            *THREAD.lock() = Some(t);
            dsp::set_hook(Some(on_dsp_hook));
            Ok(())
        })();
        if let Err(e) = started {
            if READY.load(Ordering::Acquire) {
                finalize(false);
            }
            dsp::exit();
            return Err(e);
        }
        Ok(Audio { _private: () })
    }

    /// One of the 24 channels.
    pub fn channel(&self, id: usize) -> Channel<'_> {
        assert!(id < CHANNELS, "channel {id} does not exist");
        Channel { id, _audio: self }
    }

    pub fn set_master_volume(&self, volume: f32) {
        let mut m = MASTER.lock();
        m.volume = volume;
        m.flags |= 1;
    }

    pub fn set_output_mode(&self, mode: OutputMode) {
        let mut m = MASTER.lock();
        m.output_mode = mode as u16;
        m.flags |= 2;
    }

    /// Audio frames processed so far.
    pub fn frame_count(&self) -> u32 {
        FRAMES.load(Ordering::Relaxed)
    }

    /// Frames the DSP had to drop (the driver was late).
    pub fn dropped_frames(&self) -> u32 {
        DROPPED.load(Ordering::Relaxed)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::Release);
        if let Some(t) = THREAD.lock().take() {
            t.join();
        }
        dsp::set_hook(None);
        if !CANCELLED.load(Ordering::Acquire) {
            finalize(false);
        }
        ENTERING_SLEEP.store(false, Ordering::Release);
        SLEEPING.store(false, Ordering::Release);
        CANCELLED.store(false, Ordering::Release);
        for c in &CHANNEL_STATE {
            c.lock().reset();
        }
        dsp::exit();
        ALIVE.store(false, Ordering::Release);
    }
}

/// One of the DSP's channels.
pub struct Channel<'a> {
    id: usize,
    _audio: &'a Audio,
}

impl Channel<'_> {
    fn with<R>(&self, f: impl FnOnce(&mut ChannelState) -> R) -> R {
        f(&mut CHANNEL_STATE[self.id].lock())
    }

    /// Stops the channel, drops its queue and restores the defaults.
    pub fn reset(&self) {
        self.with(|c| c.reset())
    }

    pub fn set_format(&self, format: SampleFormat) {
        self.with(|c| c.format = format.bits())
    }

    /// The rate the samples were recorded at, in Hz.
    pub fn set_sample_rate(&self, hz: f32) {
        self.with(|c| {
            c.rate = hz / OUTPUT_RATE;
            c.flags |= C_RATE;
        })
    }

    pub fn set_interpolation(&self, i: Interpolation) {
        self.with(|c| {
            c.interp = i as u8;
            c.flags |= C_INTERP;
        })
    }

    /// The 12 mix volumes: front left/right, back left/right, then the
    /// same for the two auxiliary buses.
    pub fn set_mix(&self, mix: [f32; 12]) {
        self.with(|c| {
            c.mix = mix;
            c.flags |= C_MIX;
        })
    }

    /// Left and right volumes (0.0 to 1.0).
    pub fn set_volume(&self, left: f32, right: f32) {
        let mut mix = [0.0; 12];
        mix[0] = left;
        mix[1] = right;
        self.set_mix(mix)
    }

    /// A volume and a pan (-1.0 left to 1.0 right).
    pub fn set_volume_pan(&self, volume: f32, pan: f32) {
        let pan = pan.clamp(-1.0, 1.0);
        self.set_volume(volume * (1.0 - pan).min(1.0), volume * (1.0 + pan).min(1.0))
    }

    pub fn set_paused(&self, paused: bool) {
        self.with(|c| {
            c.paused = paused;
            c.flags |= C_PLAY;
        })
    }

    /// Queues a buffer after the ones already queued. Ignored if it is
    /// already queued or playing.
    pub fn queue(&self, buf: &Arc<WaveBuf>) {
        if buf.frames == 0 || matches!(buf.status(), BufferStatus::Queued | BufferStatus::Playing) {
            return;
        }
        let _ = dsp::flush_data_cache(buf.ptr.as_ptr(), buf.bytes);
        self.with(|c| {
            let seq = if c.seq == 0 { 1 } else { c.seq };
            c.seq = seq.wrapping_add(1);
            buf.sequence.store(seq, Ordering::Release);
            buf.set_status(BufferStatus::Queued);
            c.queue.push_back(buf.clone());
        })
    }

    /// Whether the DSP is playing this channel.
    pub fn is_playing(&self) -> bool {
        self.with(|c| c.playing)
    }

    /// Buffers queued or playing.
    pub fn queued(&self) -> usize {
        self.with(|c| c.queue.len())
    }

    /// The position in the current buffer, in samples.
    pub fn sample_position(&self) -> u32 {
        self.with(|c| c.sample_pos)
    }
}
