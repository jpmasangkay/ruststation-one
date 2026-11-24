use serde::{Deserialize, Serialize};

/// Sony PlayStation master CPU clock frequency (33.8688 MHz = 44,100 Hz * 768).
pub const MASTER_CLOCK_HZ: u64 = 33_868_800;

/// Approximate CPU master clock cycles per NTSC 59.94 Hz video frame.
pub const NTSC_CYCLES_PER_FRAME: u64 = 565_048;

/// Approximate CPU master clock cycles per NTSC scanline (263 lines/frame).
pub const NTSC_CYCLES_PER_SCANLINE: u64 = 2_148;

/// Hard upper bound on active scheduled hardware events.
pub const MAX_EVENTS: usize = 32;

/// Maximum CPU cycles executed in a single contiguous slice before checking hardware events.
pub const MAX_CPU_SLICE_CYCLES: u64 = 128;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EventKind {
    GpuScanline,
    VblankEdge,
    SpuCatchUp,
    CdRomIrq,
    Sio0AckIrq,
    MdecComplete,
    Timer0Target,
    Timer1Target,
    Timer2Target,
    DmaSliceResume,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledEvent {
    pub fire_cycle: u64,
    pub kind: EventKind,
}

/// Fixed-capacity monotonic priority scheduler.
///
/// Events are stored in `events[0..len]` sorted in strictly **descending** `fire_cycle`
/// order (with ties broken FIFO so earlier-scheduled events sit closer to `len - 1`).
/// This makes `next_event_cycle()` and `pop_due()` branch-minimal `O(1)` operations
/// without heap allocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventScheduler {
    now: u64,
    len: usize,
    events: [ScheduledEvent; MAX_EVENTS],
}

impl Default for EventScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl EventScheduler {
    #[must_use]
    pub const fn new() -> Self {
        let placeholder = ScheduledEvent {
            fire_cycle: u64::MAX,
            kind: EventKind::VblankEdge,
        };
        Self {
            now: 0,
            len: 0,
            events: [placeholder; MAX_EVENTS],
        }
    }

    #[inline(always)]
    #[must_use]
    pub const fn now(&self) -> u64 {
        self.now
    }

    #[inline(always)]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline(always)]
    #[must_use]
    pub const fn next_event_cycle(&self) -> u64 {
        if self.len == 0 {
            u64::MAX
        } else {
            self.events[self.len - 1].fire_cycle
        }
    }

    #[inline(always)]
    pub fn advance(&mut self, delta_cycles: u64) {
        let next = self
            .now
            .checked_add(delta_cycles)
            .expect("Master clock u64 overflow");
        debug_assert!(next >= self.now, "Master clock must be monotonic");
        self.now = next;
    }

    /// Schedules `kind` to fire at `self.now + delta_cycles`, replacing any existing
    /// pending event of the same `EventKind`.
    pub fn schedule(&mut self, delta_cycles: u64, kind: EventKind) {
        self.cancel(kind);
        assert!(
            self.len < MAX_EVENTS,
            "EventScheduler capacity ({MAX_EVENTS}) exceeded"
        );

        let fire_cycle = self
            .now
            .checked_add(delta_cycles)
            .expect("Scheduled event timestamp overflow");
        debug_assert!(fire_cycle >= self.now);

        // Keep largest `fire_cycle` at index 0 and earliest `fire_cycle` at `len - 1`.
        // Using `<=` shifts existing equal-timestamp events toward `len - 1`, preserving FIFO order!
        let mut idx = self.len;
        while idx > 0 && self.events[idx - 1].fire_cycle <= fire_cycle {
            self.events[idx] = self.events[idx - 1];
            idx -= 1;
        }
        self.events[idx] = ScheduledEvent { fire_cycle, kind };
        self.len += 1;

        self.debug_assert_sorted();
    }

    /// Cancels a pending event of `kind`, returning `true` if one was removed.
    pub fn cancel(&mut self, kind: EventKind) -> bool {
        for i in 0..self.len {
            if self.events[i].kind == kind {
                self.events.copy_within((i + 1)..self.len, i);
                self.len -= 1;
                self.debug_assert_sorted();
                return true;
            }
        }
        false
    }

    /// Pops the earliest event in `O(1)` if `fire_cycle <= self.now`.
    #[inline]
    pub fn pop_due(&mut self) -> Option<ScheduledEvent> {
        if self.len > 0 && self.events[self.len - 1].fire_cycle <= self.now {
            self.len -= 1;
            let ev = self.events[self.len];
            debug_assert!(ev.fire_cycle <= self.now);
            Some(ev)
        } else {
            None
        }
    }

    #[inline]
    fn debug_assert_sorted(&self) {
        debug_assert!(self.len <= MAX_EVENTS);
        for i in 1..self.len {
            debug_assert!(
                self.events[i - 1].fire_cycle >= self.events[i].fire_cycle,
                "EventScheduler descending sort invariant broken"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_and_pops_in_chronological_order() {
        let mut sched = EventScheduler::new();
        assert!(sched.is_empty());
        assert_eq!(sched.next_event_cycle(), u64::MAX);

        sched.schedule(300, EventKind::VblankEdge);
        sched.schedule(100, EventKind::Sio0AckIrq);
        sched.schedule(200, EventKind::CdRomIrq);

        assert_eq!(sched.len(), 3);
        assert_eq!(sched.next_event_cycle(), 100);
        assert_eq!(sched.pop_due(), None);

        sched.advance(99);
        assert_eq!(sched.pop_due(), None);

        sched.advance(1);
        assert_eq!(
            sched.pop_due(),
            Some(ScheduledEvent {
                fire_cycle: 100,
                kind: EventKind::Sio0AckIrq,
            })
        );
        assert_eq!(sched.next_event_cycle(), 200);

        sched.advance(250);
        assert_eq!(sched.now(), 350);
        assert_eq!(sched.pop_due().map(|e| e.kind), Some(EventKind::CdRomIrq));
        assert_eq!(sched.pop_due().map(|e| e.kind), Some(EventKind::VblankEdge));
        assert_eq!(sched.pop_due(), None);
        assert!(sched.is_empty());
    }

    #[test]
    fn rescheduling_same_kind_replaces_previous_entry() {
        let mut sched = EventScheduler::new();
        sched.schedule(500, EventKind::Timer0Target);
        sched.schedule(150, EventKind::Timer0Target);
        assert_eq!(sched.len(), 1);
        assert_eq!(sched.next_event_cycle(), 150);

        assert!(sched.cancel(EventKind::Timer0Target));
        assert!(!sched.cancel(EventKind::Timer0Target));
        assert!(sched.is_empty());
    }

    #[test]
    fn equal_timestamps_pop_in_fifo_order() {
        let mut sched = EventScheduler::new();
        sched.schedule(50, EventKind::Timer0Target);
        sched.schedule(50, EventKind::Timer1Target);
        sched.schedule(50, EventKind::Timer2Target);

        sched.advance(50);
        assert_eq!(
            sched.pop_due().map(|e| e.kind),
            Some(EventKind::Timer0Target)
        );
        assert_eq!(
            sched.pop_due().map(|e| e.kind),
            Some(EventKind::Timer1Target)
        );
        assert_eq!(
            sched.pop_due().map(|e| e.kind),
            Some(EventKind::Timer2Target)
        );
    }
}
