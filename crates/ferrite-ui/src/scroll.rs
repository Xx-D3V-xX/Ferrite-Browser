// Wheel and trackpad scrolling for the page area.
//
// Two kinds of input arrive from the OS and want different handling:
//
// - **Pixel deltas** (a trackpad, a precision wheel) are already smooth: the
//   OS streams many small deltas at its own cadence, often faster than the
//   display. They are summed and handed to the engine once per frame, so a
//   120 Hz trackpad costs one engine event per tick, not one per sample.
// - **Line deltas** (a notched mouse wheel) arrive as whole notches. Sent as
//   one jump they teleport the page; here each notch is queued as a distance
//   and delivered over the next few frames along an ease-out curve, which is
//   what Chrome and Safari do and what makes a wheel feel smooth.
//
// All of it is pure state so it is testable without a window or an engine.

/// Logical pixels one wheel line scrolls — what a notch moves at 1x. Scaled by
/// the display factor on the way to the engine, which counts device pixels.
pub(crate) const LINE_LOGICAL_PX: f32 = 60.0;

/// The fraction of the queued notch distance delivered each frame. 0.28 settles
/// a notch in roughly a quarter of a second at 60 Hz.
const EASE_PER_FRAME: f32 = 0.28;

/// Below this many pixels left, the rest is delivered at once rather than
/// trailing off in sub-pixel steps.
const SETTLE_PX: f32 = 1.0;

/// The most queued notch distance in either axis, so a burst of spins cannot
/// queue seconds of scrolling the person has to wait out.
const MAX_QUEUED_PX: f32 = 2400.0;

/// One wheel event, as the view reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Wheel {
    /// Whole notches; `y` positive scrolls content up (iced's convention).
    Lines { x: f32, y: f32 },
    /// Already in device pixels.
    Pixels { x: f32, y: f32 },
}

/// Scroll input waiting to be delivered to the engine.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct ScrollQueue {
    /// Pixel deltas received since the last frame, sent whole next frame.
    direct: (f32, f32),
    /// Notch distance still to deliver, eased out over frames.
    queued: (f32, f32),
}

impl ScrollQueue {
    /// Takes one wheel event. `scale` is device pixels per logical point.
    pub(crate) fn push(&mut self, wheel: Wheel, scale: f32) {
        match wheel {
            Wheel::Pixels { x, y } => {
                self.direct.0 += x;
                self.direct.1 += y;
            }
            Wheel::Lines { x, y } => {
                let per_line = LINE_LOGICAL_PX * scale.max(0.5);
                self.queued.0 = (self.queued.0 + x * per_line).clamp(-MAX_QUEUED_PX, MAX_QUEUED_PX);
                self.queued.1 = (self.queued.1 + y * per_line).clamp(-MAX_QUEUED_PX, MAX_QUEUED_PX);
            }
        }
    }

    /// Whether anything is waiting (the tick keeps running at full rate while so).
    pub(crate) fn is_pending(&self) -> bool {
        self.direct != (0.0, 0.0) || self.queued != (0.0, 0.0)
    }

    /// The delta to send this frame, if any, and the queue's new state.
    pub(crate) fn next_frame(&mut self) -> Option<(f32, f32)> {
        let step = |left: &mut f32| -> f32 {
            if left.abs() <= SETTLE_PX {
                std::mem::take(left)
            } else {
                let part = *left * EASE_PER_FRAME;
                // Never a step so small it stalls: at least one pixel.
                let part = if part.abs() < SETTLE_PX {
                    SETTLE_PX.copysign(*left)
                } else {
                    part
                };
                *left -= part;
                part
            }
        };
        let dx = std::mem::take(&mut self.direct.0) + step(&mut self.queued.0);
        let dy = std::mem::take(&mut self.direct.1) + step(&mut self.queued.1);
        (dx != 0.0 || dy != 0.0).then_some((dx, dy))
    }

    /// Forgets everything queued (the tab changed; the scroll belongs to the old page).
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drain(q: &mut ScrollQueue) -> Vec<(f32, f32)> {
        let mut frames = Vec::new();
        while let Some(f) = q.next_frame() {
            frames.push(f);
            assert!(frames.len() < 200, "a queue must settle");
        }
        frames
    }

    fn total(frames: &[(f32, f32)]) -> (f32, f32) {
        frames
            .iter()
            .fold((0.0, 0.0), |a, f| (a.0 + f.0, a.1 + f.1))
    }

    #[test]
    fn an_empty_queue_sends_nothing() {
        let mut q = ScrollQueue::default();
        assert!(!q.is_pending());
        assert_eq!(q.next_frame(), None);
    }

    #[test]
    fn pixel_deltas_between_frames_are_summed_into_one_event() {
        let mut q = ScrollQueue::default();
        q.push(Wheel::Pixels { x: 0.0, y: 3.0 }, 2.0);
        q.push(Wheel::Pixels { x: 1.0, y: 4.0 }, 2.0);
        q.push(Wheel::Pixels { x: 0.0, y: -1.0 }, 2.0);
        assert_eq!(q.next_frame(), Some((1.0, 6.0)));
        assert_eq!(q.next_frame(), None, "and only once");
    }

    #[test]
    fn a_wheel_notch_is_spread_over_several_frames_and_loses_nothing() {
        let mut q = ScrollQueue::default();
        q.push(Wheel::Lines { x: 0.0, y: -1.0 }, 1.0);
        let frames = drain(&mut q);
        assert!(frames.len() > 4, "a notch should ease out, got {frames:?}");
        let (x, y) = total(&frames);
        assert_eq!(x, 0.0);
        assert!((y + LINE_LOGICAL_PX).abs() < 1e-3, "delivered {y}");
        // Ease-out: the first frame moves the most.
        assert!(frames[0].1.abs() > frames[frames.len() - 1].1.abs());
        // Direction never flips mid-notch.
        assert!(frames.iter().all(|f| f.1 <= 0.0));
    }

    #[test]
    fn a_notch_scales_with_the_display_factor() {
        let mut q = ScrollQueue::default();
        q.push(Wheel::Lines { x: 0.0, y: 1.0 }, 2.0);
        let (_, y) = total(&drain(&mut q));
        assert!((y - 2.0 * LINE_LOGICAL_PX).abs() < 1e-3);
    }

    #[test]
    fn opposite_notches_cancel_instead_of_queueing_both() {
        let mut q = ScrollQueue::default();
        q.push(Wheel::Lines { x: 0.0, y: 1.0 }, 1.0);
        q.push(Wheel::Lines { x: 0.0, y: -1.0 }, 1.0);
        assert_eq!(q.next_frame(), None);
    }

    #[test]
    fn a_burst_of_notches_is_capped() {
        let mut q = ScrollQueue::default();
        for _ in 0..200 {
            q.push(Wheel::Lines { x: 0.0, y: 1.0 }, 1.0);
        }
        let (_, y) = total(&drain(&mut q));
        assert!(y <= MAX_QUEUED_PX + 1e-3, "{y}");
    }

    #[test]
    fn pixel_input_during_a_notch_is_not_delayed_by_it() {
        let mut q = ScrollQueue::default();
        q.push(Wheel::Lines { x: 0.0, y: 1.0 }, 1.0);
        q.push(Wheel::Pixels { x: 0.0, y: 10.0 }, 1.0);
        let first = q.next_frame().expect("a frame");
        assert!(
            first.1 >= 10.0,
            "the direct delta goes out at once: {first:?}"
        );
    }

    #[test]
    fn clear_forgets_queued_scroll() {
        let mut q = ScrollQueue::default();
        q.push(Wheel::Lines { x: 1.0, y: 1.0 }, 1.0);
        q.push(Wheel::Pixels { x: 5.0, y: 5.0 }, 1.0);
        q.clear();
        assert!(!q.is_pending());
        assert_eq!(q.next_frame(), None);
    }
}
