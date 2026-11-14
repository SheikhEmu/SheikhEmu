//! The single-producer single-consumer sample ring (spec §5.5.1, D-40).
//!
//! The audio path's ONLY shared structure. One producer (the event-loop
//! thread that runs the emulator, pushing resampled frames) and one
//! consumer (the audio device's callback thread) exchange mono i16
//! samples through it with **no locks of any kind** — two atomics are
//! the whole synchronization (§5.5.1: "SPSC بلا أقفال، لا Mutex في
//! المسار الحارّ"):
//!
//! - `head` is written only by the producer (one `Release` store per
//!   pushed batch, after every sample of the batch is in place),
//! - `tail` is written only by the consumer (one `Release` store per
//!   popped batch),
//! - the other side reads them with `Acquire`, which pairs with the
//!   counterpart's `Release` and makes the batch visible as a whole,
//!   so a popped sample can never be half-written.
//!
//! Both counters are absolute and monotonic; the wrap-around index is
//! `counter & mask` with a power-of-two capacity. An `usize` counter
//! cannot meaningfully wrap (2^64 samples at 48 kHz is ~12 million
//! years of audio).
//!
//! Overflow policy: the producer never overwrites unread samples —
//! when the ring is full it writes what fits and reports the remainder
//! as dropped (the engine counts and reports the drops; a stalled
//! device degrades to dropping, not to corruption). Underrun policy
//! (the consumer side, in `audio.rs`): an empty ring yields silence
//! and counts an underrun — the documented realtime behavior.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A lock-free SPSC ring buffer of mono i16 samples.
///
/// # The `UnsafeCell` safety argument
/// The buffer's interior mutability is bounded by the two atomic
/// counters: the producer only ever writes slots at or beyond the
/// published `head` (unread by the consumer until the `Release`
/// store), and the consumer only ever reads slots below the published
/// `head` and at or beyond its own `tail`. The two sides therefore
/// touch disjoint regions except across the atomic publishes, which
/// is the textbook SPSC invariant.
pub struct SpscRing {
    buffer: UnsafeCell<Box<[i16]>>,
    mask: usize,
    /// Absolute write counter, owned by the producer.
    head: AtomicUsize,
    /// Absolute read counter, owned by the consumer.
    tail: AtomicUsize,
}

// SAFETY: see the struct-level safety argument — disjoint regions
// plus Release/Acquire publishes; safe to share across exactly the
// two designated threads.
unsafe impl Sync for SpscRing {}

impl SpscRing {
    /// Create a ring with the given capacity rounded up to a power of
    /// two (the mask arithmetic needs it).
    ///
    /// # Panics
    /// Panics if `capacity` is zero or so large that no power of two
    /// above it fits in a `usize`.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "ring capacity must be nonzero");
        let rounded = capacity.next_power_of_two();
        SpscRing {
            buffer: UnsafeCell::new(vec![0i16; rounded].into_boxed_slice()),
            mask: rounded - 1,
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// The usable capacity (the power-of-two rounding is internal).
    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Producer side: write up to `samples.len()` samples, publishing
    /// them all at once. Returns the number actually written; the
    /// caller counts the rest as dropped.
    pub fn push(&self, samples: &[i16]) -> usize {
        let mut head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        let free = self.capacity() - used;
        let count = samples.len().min(free);
        // SAFETY: producer-exclusive region — slots at or beyond the
        // published `head` (the consumer only reads published slots).
        let buffer = unsafe { &mut *self.buffer.get() };
        // Wrap-aware copy in (at most) two chunks; nothing is published
        // until the store below, so the consumer never sees a torn batch.
        let start = head & self.mask;
        let first = count.min(self.capacity() - start);
        buffer[start..start + first].copy_from_slice(&samples[..first]);
        if count > first {
            buffer[..count - first].copy_from_slice(&samples[first..count]);
        }
        head = head.wrapping_add(count);
        self.head.store(head, Ordering::Release);
        count
    }

    /// Consumer side: read up to `out.len()` committed samples,
    /// publishing the consumption once. Returns the number read.
    pub fn pop(&self, out: &mut [i16]) -> usize {
        let head = self.head.load(Ordering::Acquire);
        let mut tail = self.tail.load(Ordering::Relaxed);
        let available = head.wrapping_sub(tail);
        let count = out.len().min(available);
        // SAFETY: consumer-exclusive region — only slots below the
        // published `head`, which the producer never rewrites until
        // they are consumed (the free-space rule above).
        let buffer = unsafe { &*self.buffer.get() };
        let start = tail & self.mask;
        let first = count.min(self.capacity() - start);
        out[..first].copy_from_slice(&buffer[start..start + first]);
        if count > first {
            out[first..count].copy_from_slice(&buffer[..count - first]);
        }
        tail = tail.wrapping_add(count);
        self.tail.store(tail, Ordering::Release);
        count
    }

    /// The number of samples currently queued, as seen from the
    /// producer side (the consumer's `tail` may be microseconds
    /// stale — this is a control-loop measurement, not a queue
    /// primitive).
    pub fn level(&self) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        head.wrapping_sub(tail)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn ring_is_send_and_sync() {
        // The §5.5.1 contract in one compile-time line: the shared
        // audio path is atomics only, so it crosses threads freely.
        fn check<T: Send + Sync>() {}
        check::<SpscRing>();
    }

    #[test]
    fn capacity_rounds_up_to_a_power_of_two() {
        assert_eq!(SpscRing::new(3200).capacity(), 4096);
        assert_eq!(SpscRing::new(1024).capacity(), 1024);
    }

    #[test]
    fn order_is_preserved_across_the_wrap() {
        let ring = SpscRing::new(8);
        let values: Vec<i16> = (0..40i16).collect();
        // Push in batches while draining behind the producer (as the
        // device would), collecting EVERYTHING ever consumed, so the
        // ring wraps several times over the run.
        let mut out = Vec::new();
        let mut written = 0usize;
        while written < values.len() {
            written += ring.push(&values[written..]);
            let mut sink = [0i16; 3];
            let got = ring.pop(&mut sink);
            out.extend_from_slice(&sink[..got]);
        }
        // Drain what remains and verify strict FIFO order end to end.
        let mut chunk = [0i16; 4];
        loop {
            let got = ring.pop(&mut chunk);
            if got == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..got]);
        }
        assert_eq!(out.len(), 40);
        assert_eq!(out, values);
    }

    #[test]
    fn a_full_ring_drops_instead_of_overwriting() {
        let ring = SpscRing::new(4);
        let values = [1i16, 2, 3, 4, 5, 6];
        assert_eq!(ring.push(&values), 4);
        let mut out = [0i16; 4];
        assert_eq!(ring.pop(&mut out), 4);
        assert_eq!(out, [1, 2, 3, 4]);
        // The dropped 5 and 6 never entered; the next push starts clean.
        assert_eq!(ring.push(&[7i16]), 1);
        let mut tail = [0i16; 2];
        assert_eq!(ring.pop(&mut tail), 1);
        assert_eq!(tail[0], 7);
    }

    #[test]
    fn partial_reads_and_level_accounting_agree() {
        let ring = SpscRing::new(16);
        assert_eq!(ring.push(&[10i16, 11, 12, 13]), 4);
        assert_eq!(ring.level(), 4);
        let mut out = [0i16; 2];
        assert_eq!(ring.pop(&mut out), 2);
        assert_eq!(out, [10, 11]);
        assert_eq!(ring.level(), 2);
    }

    #[test]
    fn concurrent_producer_and_consumer_preserve_every_sample() {
        // A real two-thread SPSC run: 200_000 samples from a producer
        // thread to a consumer thread, order verified at the end.
        const TOTAL: usize = 200_000;
        let ring = std::sync::Arc::new(SpscRing::new(1024));
        let producer_ring = ring.clone();
        let producer = thread::spawn(move || {
            let mut sent = 0usize;
            while sent < TOTAL {
                let batch: Vec<i16> = (sent..(sent + 64).min(TOTAL))
                    .map(|v| (v % 7) as i16 - 3)
                    .collect();
                sent += producer_ring.push(&batch);
            }
        });
        let received: Vec<i16> = thread::spawn(move || {
            let mut out = Vec::with_capacity(TOTAL);
            let mut chunk = [0i16; 48];
            while out.len() < TOTAL {
                let got = ring.pop(&mut chunk);
                if got > 0 {
                    out.extend_from_slice(&chunk[..got]);
                }
            }
            out
        })
        .join()
        .expect("consumer thread");
        producer.join().expect("producer thread");
        let expected: Vec<i16> = (0..TOTAL).map(|v| (v % 7) as i16 - 3).collect();
        assert_eq!(received.len(), TOTAL);
        assert_eq!(received, expected);
    }
}
