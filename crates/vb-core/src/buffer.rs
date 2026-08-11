//! Audio buffers: channel-major sample storage allocated off the RT thread.

/// A block of audio: `channels` rows of `frames` samples, stored channel-major
/// (all of channel 0, then all of channel 1, …).
///
/// Allocation happens in [`AudioBuffer::new`], never on the audio thread; the RT path
/// only reads and writes samples in an already-sized buffer (see [`AudioBuffer::clear`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioBuffer {
    channels: usize,
    frames: usize,
    samples: Vec<f32>,
}

impl AudioBuffer {
    /// Allocates a zeroed buffer of `channels` × `frames` samples.
    #[must_use]
    pub fn new(channels: usize, frames: usize) -> Self {
        Self {
            channels,
            frames,
            samples: vec![0.0; channels * frames],
        }
    }

    /// Number of channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Number of frames (samples per channel).
    #[must_use]
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// Immutable view of channel `c`, or `None` if `c` is out of range.
    #[must_use]
    pub fn channel(&self, c: usize) -> Option<&[f32]> {
        if c >= self.channels {
            return None;
        }
        let start = c * self.frames;
        Some(&self.samples[start..start + self.frames])
    }

    /// Mutable view of channel `c`, or `None` if `c` is out of range.
    pub fn channel_mut(&mut self, c: usize) -> Option<&mut [f32]> {
        if c >= self.channels {
            return None;
        }
        let start = c * self.frames;
        Some(&mut self.samples[start..start + self.frames])
    }

    /// Zeroes every sample. Does not allocate — safe to call on the audio thread.
    pub fn clear(&mut self) {
        self.samples.fill(0.0);
    }

    /// Iterates every channel as a mutable slice, channel 0 first.
    pub fn channels_mut(&mut self) -> impl Iterator<Item = &mut [f32]> {
        let frames = self.frames;
        self.samples.chunks_mut(frames.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::AudioBuffer;

    #[test]
    fn new_allocates_zeroed_channels_and_frames() {
        let buf = AudioBuffer::new(2, 4);
        assert_eq!(buf.channels(), 2);
        assert_eq!(buf.frames(), 4);
        assert_eq!(buf.channel(0).unwrap(), &[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(buf.channel(1).unwrap(), &[0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn channels_are_independent_and_channel_major() {
        let mut buf = AudioBuffer::new(2, 3);
        buf.channel_mut(0)
            .unwrap()
            .copy_from_slice(&[1.0, 2.0, 3.0]);
        buf.channel_mut(1)
            .unwrap()
            .copy_from_slice(&[4.0, 5.0, 6.0]);
        assert_eq!(buf.channel(0).unwrap(), &[1.0, 2.0, 3.0]);
        assert_eq!(buf.channel(1).unwrap(), &[4.0, 5.0, 6.0]);
    }

    #[test]
    fn out_of_range_channel_is_none() {
        let mut buf = AudioBuffer::new(1, 2);
        assert!(buf.channel(1).is_none());
        assert!(buf.channel_mut(9).is_none());
    }

    #[test]
    fn clear_zeroes_without_changing_shape() {
        let mut buf = AudioBuffer::new(1, 3);
        buf.channel_mut(0)
            .unwrap()
            .copy_from_slice(&[1.0, 2.0, 3.0]);
        buf.clear();
        assert_eq!(buf.channel(0).unwrap(), &[0.0, 0.0, 0.0]);
        assert_eq!(buf.frames(), 3);
    }
}
