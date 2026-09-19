//! The last few utterances, kept so they can be re-run (SPEC §12).
//!
//! Parakeet's multilingual variant is newer than Whisper large-v3 and
//! leaderboard error rates are measured on read speech, not on one person's
//! microphone and accent. Keeping the audio is how the comparison gets real
//! numbers for this use case instead of published ones.
//!
//! The PCM is behind an `Arc` so re-running an utterance through several
//! engines copies nothing, and it is dropped when the ring evicts it.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::audio::SAMPLE_RATE;

/// Utterances retained by default. Twenty is about ten minutes of
/// conversation, and at 16 kHz mono f32 costs a few tens of megabytes at worst.
pub const DEFAULT_CAPACITY: usize = 20;

/// One captured utterance, keyed by when it started.
#[derive(Debug, Clone)]
pub struct Utterance {
    /// 1-based, in the order they were captured this session.
    pub index: usize,
    /// Milliseconds since capture started.
    pub start_ms: u64,
    pub pcm: Arc<Vec<f32>>,
}

impl Utterance {
    pub fn duration_ms(&self) -> u64 {
        self.pcm.len() as u64 * 1000 / SAMPLE_RATE as u64
    }
}

/// A fixed-size ring of the most recent utterances.
#[derive(Debug)]
pub struct UtteranceRing {
    capacity: usize,
    items: VecDeque<Utterance>,
    captured: usize,
}

impl UtteranceRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            items: VecDeque::with_capacity(capacity.max(1)),
            captured: 0,
        }
    }

    /// Store an utterance, evicting the oldest if the ring is full. Returns it
    /// as stored, so the caller has the index and the shared PCM.
    pub fn push(&mut self, start_ms: u64, pcm: Vec<f32>) -> Utterance {
        self.captured += 1;
        let utterance = Utterance {
            index: self.captured,
            start_ms,
            pcm: Arc::new(pcm),
        };
        if self.items.len() == self.capacity {
            self.items.pop_front();
        }
        self.items.push_back(utterance.clone());
        utterance
    }

    /// Most recent first — the order a "re-run that" control would offer.
    /// Nothing outside the tests reads it back yet; the UI does from
    /// Milestone 5.
    #[cfg(test)]
    pub fn recent(&self) -> impl Iterator<Item = &Utterance> {
        self.items.iter().rev()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_newest_and_drops_the_oldest() {
        let mut ring = UtteranceRing::new(3);
        for i in 0..5 {
            ring.push(i * 1000, vec![0.0; SAMPLE_RATE as usize]);
        }
        assert_eq!(ring.len(), 3);
        let indexes: Vec<usize> = ring.recent().map(|u| u.index).collect();
        assert_eq!(indexes, vec![5, 4, 3], "newest first, oldest evicted");
    }

    #[test]
    fn indexes_count_every_utterance_not_just_the_retained_ones() {
        let mut ring = UtteranceRing::new(2);
        ring.push(0, vec![0.0; 10]);
        ring.push(1, vec![0.0; 10]);
        let third = ring.push(2, vec![0.0; 10]);
        assert_eq!(third.index, 3);
    }

    #[test]
    fn duration_comes_from_the_sample_count() {
        let mut ring = UtteranceRing::new(1);
        let utterance = ring.push(0, vec![0.0; SAMPLE_RATE as usize * 3 / 2]);
        assert_eq!(utterance.duration_ms(), 1500);
    }

    #[test]
    fn evicting_releases_the_audio() {
        let mut ring = UtteranceRing::new(1);
        let first = ring.push(0, vec![0.0; 16]);
        let kept = Arc::clone(&first.pcm);
        ring.push(1000, vec![0.0; 16]);
        // The ring's copy is gone; only this test still holds one.
        assert_eq!(
            Arc::strong_count(&kept),
            2,
            "ring still holds the evicted pcm"
        );
        drop(first);
        assert_eq!(Arc::strong_count(&kept), 1);
    }
}
