#![forbid(unsafe_code)]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod bus;
pub mod cpu;
pub mod exe;
pub mod gte;
pub mod scheduler;
pub mod system;

pub use bus::{Bus, BusError, GpuBackend, NullGpu};
pub use cpu::Cpu;
pub use exe::PsxExe;
pub use gte::Gte;
pub use scheduler::{
    EventKind, EventScheduler, MASTER_CLOCK_HZ, MAX_CPU_SLICE_CYCLES, MAX_EVENTS,
    NTSC_CYCLES_PER_FRAME, ScheduledEvent,
};
pub use system::{System, TraceEntry, TtySink};
