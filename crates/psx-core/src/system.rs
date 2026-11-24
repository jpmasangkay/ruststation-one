use crate::bus::{Bus, GpuBackend, NullGpu, RAM_BYTES};
use crate::cpu::Cpu;
use crate::exe::{BIOS_SHELL_ENTRY_PC, PsxExe};
use crate::scheduler::{
    EventKind, EventScheduler, MAX_CPU_SLICE_CYCLES, NTSC_CYCLES_PER_FRAME,
    NTSC_CYCLES_PER_SCANLINE, ScheduledEvent,
};
use alloc::vec;
use alloc::vec::Vec;

pub const MAX_TRACE_CAPACITY: usize = 1_048_576;

/// Sink trait for capturing BIOS TTY stdout and Expansion 2 debug characters without `std`.
pub trait TtySink {
    fn on_tty_byte(&mut self, byte: u8);
}

impl TtySink for () {
    #[inline(always)]
    fn on_tty_byte(&mut self, _byte: u8) {}
}

impl TtySink for Vec<u8> {
    #[inline]
    fn on_tty_byte(&mut self, byte: u8) {
        self.push(byte);
    }
}

/// Snapshot of a single executed instruction stored inside the bounded trace ring buffer.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceEntry {
    pub cycle: u64,
    pub pc: u32,
    pub opcode: u32,
    pub v0: u32,
    pub a0: u32,
    pub sp: u32,
    pub ra: u32,
}

/// Pre-allocated circular trace buffer (zero heap allocations during stepping).
#[derive(Clone, Debug)]
pub struct TraceRingBuffer {
    entries: Vec<TraceEntry>,
    capacity: usize,
    head: usize,
    count: usize,
}

impl TraceRingBuffer {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.min(MAX_TRACE_CAPACITY);
        Self {
            entries: vec![TraceEntry::default(); cap],
            capacity: cap,
            head: 0,
            count: 0,
        }
    }

    #[inline(always)]
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.capacity > 0
    }

    #[inline]
    pub fn push(&mut self, entry: TraceEntry) {
        if self.capacity == 0 {
            return;
        }
        self.entries[self.head] = entry;
        self.head = (self.head + 1) % self.capacity;
        if self.count < self.capacity {
            self.count += 1;
        }
    }

    /// Returns recorded trace entries in chronological order (oldest to newest).
    #[must_use]
    pub fn snapshot_chronological(&self) -> Vec<TraceEntry> {
        let mut out = Vec::with_capacity(self.count);
        if self.count == 0 {
            return out;
        }
        let start = if self.count < self.capacity {
            0
        } else {
            self.head
        };
        for i in 0..self.count {
            let idx = (start + i) % self.capacity;
            out.push(self.entries[idx]);
        }
        out
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StepStopReason {
    CompletedSlice,
    ReachedShell,
    MaxCyclesReached,
}

pub struct System<G: GpuBackend = NullGpu> {
    pub cpu: Cpu,
    pub bus: Bus<G>,
    pub scheduler: EventScheduler,
    scanline: u16,
    pending_exe: Option<PsxExe>,
    exe_sideloaded: bool,
    trace: TraceRingBuffer,
}

impl System<NullGpu> {
    #[must_use]
    pub fn new_headless() -> Self {
        Self::new(NullGpu::new())
    }
}

impl<G: GpuBackend> System<G> {
    #[must_use]
    pub fn new(gpu: G) -> Self {
        let mut scheduler = EventScheduler::new();
        scheduler.schedule(NTSC_CYCLES_PER_SCANLINE, EventKind::GpuScanline);
        Self {
            cpu: Cpu::new_reset(),
            bus: Bus::new(gpu),
            scheduler,
            scanline: 0,
            pending_exe: None,
            exe_sideloaded: false,
            trace: TraceRingBuffer::with_capacity(0),
        }
    }

    /// Enables or resizes the fixed-capacity instruction trace ring buffer.
    pub fn enable_trace(&mut self, capacity: usize) {
        self.trace = TraceRingBuffer::with_capacity(capacity);
    }

    #[must_use]
    pub fn trace_entries(&self) -> Vec<TraceEntry> {
        self.trace.snapshot_chronological()
    }

    /// Loads a 512 KB BIOS ROM and resets CPU/Scheduler state.
    pub fn load_bios(&mut self, bios_bytes: &[u8]) -> Result<(), &'static str> {
        self.bus.load_bios(bios_bytes)?;
        self.cpu = Cpu::new_reset();
        self.scheduler = EventScheduler::new();
        self.scheduler
            .schedule(NTSC_CYCLES_PER_SCANLINE, EventKind::GpuScanline);
        self.scanline = 0;
        self.exe_sideloaded = false;
        Ok(())
    }

    /// Queues a `PS-X EXE` to be sideloaded automatically when BIOS hits shell entry (`0x8003_0000`).
    pub fn set_sideload_exe(&mut self, exe: PsxExe) {
        self.pending_exe = Some(exe);
        self.exe_sideloaded = false;
    }

    /// Applies a `PS-X EXE` immediately into Main RAM and CPU registers (used both by the
    /// `0x8003_0000` BIOS shell hook and by direct BIOS-less test execution).
    pub fn apply_exe_now(&mut self, exe: &PsxExe) {
        let ram = self.bus.ram_mut();

        // Optional BSS zero-fill region from header
        if exe.memfill_size > 0 {
            let start = (exe.memfill_addr as usize) & (RAM_BYTES - 1);
            let end = start
                .saturating_add(exe.memfill_size as usize)
                .min(RAM_BYTES);
            ram[start..end].fill(0);
        }

        let dst_start = (exe.load_addr as usize) & (RAM_BYTES - 1);
        let dst_end = (dst_start + exe.payload.len()).min(RAM_BYTES);
        let copy_len = dst_end - dst_start;
        ram[dst_start..dst_end].copy_from_slice(&exe.payload[..copy_len]);

        // Flush CPU I-Cache lines across the loaded executable range so stale BIOS unpacker lines aren't run
        for off in (0..copy_len).step_by(16) {
            self.cpu
                .invalidate_icache_line(exe.load_addr.wrapping_add(off as u32));
        }

        self.cpu.pc = exe.initial_pc;
        self.cpu.next_pc = exe.initial_pc.wrapping_add(4);
        self.cpu.regs[28] = exe.initial_gp; // $gp
        if let Some(sp) = exe.initial_sp() {
            self.cpu.regs[29] = sp; // $sp
            self.cpu.regs[30] = sp; // $fp
        }
        self.cpu.regs[0] = 0;
        self.exe_sideloaded = true;
    }

    /// Installs a minimal synthetic BIOS kernel stub at `0xA0`/`0xB0` and `0x8000_0080` so standalone
    /// CPU/GTE `.exe` test binaries can run and emit TTY output even without a 512 KB BIOS file.
    pub fn install_biosless_kernel_stub(&mut self) {
        // Return immediately (`jr $ra; nop`) at A0 (0xA0), B0 (0xB0), and C0 (0xC0) vectors
        for &vec_addr in &[0x8000_00A0u32, 0x8000_00B0, 0x8000_00C0] {
            let _ = self.bus.write32(vec_addr, 0x03E0_0008); // jr $ra
            let _ = self.bus.write32(vec_addr + 4, 0x0000_0000); // nop
        }
        // Clear BEV so exceptions go to RAM 0x8000_0080, and enable COP2 (GTE)
        self.cpu.cop0.sr = 1 << 30;
    }

    #[inline(always)]
    #[must_use]
    pub const fn exe_sideloaded(&self) -> bool {
        self.exe_sideloaded
    }

    /// Steps one NTSC frame (`565,048` cycles) without TTY capture.
    pub fn step_frame(&mut self) {
        let target = self.scheduler.now().saturating_add(NTSC_CYCLES_PER_FRAME);
        let _ = self.run_until_cycle(target, false, &mut ());
    }

    /// Runs deterministically until `max_cycle` (or `0x8003_0000` shell entry if `stop_at_shell` is set).
    pub fn run_until_cycle<T: TtySink>(
        &mut self,
        max_cycle: u64,
        stop_at_shell: bool,
        tty: &mut T,
    ) -> StepStopReason {
        while self.scheduler.now() < max_cycle {
            let slice_limit = self
                .scheduler
                .next_event_cycle()
                .min(max_cycle)
                .min(self.scheduler.now().saturating_add(MAX_CPU_SLICE_CYCLES));

            while self.scheduler.now() < slice_limit {
                // 1. Check BIOS shell hook (`0x8003_0000`) BEFORE executing shell instruction
                if self.cpu.pc == BIOS_SHELL_ENTRY_PC {
                    if let Some(exe) = self.pending_exe.take() {
                        self.apply_exe_now(&exe);
                    } else if stop_at_shell {
                        return StepStopReason::ReachedShell;
                    }
                }

                // 2. Intercept BIOS `putchar` syscalls: A0(0x3C) and B0(0x3D)
                let phys_pc = self.cpu.pc & 0x1FFF_FFFF;
                if (phys_pc == 0x0000_00B0 && self.cpu.regs[9] == 0x3D)
                    || (phys_pc == 0x0000_00A0 && self.cpu.regs[9] == 0x3C)
                {
                    tty.on_tty_byte(self.cpu.regs[4] as u8);
                }

                let step_cycle = self.scheduler.now();
                let step_pc = self.cpu.pc;
                self.bus.sync_clock(step_cycle);

                let elapsed = self.cpu.step_instruction(&mut self.bus);
                self.scheduler.advance(elapsed);

                if self.trace.is_enabled() {
                    self.trace.push(TraceEntry {
                        cycle: step_cycle,
                        pc: step_pc,
                        opcode: self.cpu.last_opcode,
                        v0: self.cpu.regs[2],
                        a0: self.cpu.regs[4],
                        sp: self.cpu.regs[29],
                        ra: self.cpu.regs[31],
                    });
                }

                // 3. Forward any Expansion 2 DUART / debug console bytes
                let (exp2_bytes, exp2_len) = self.bus.drain_exp2_tty();
                for &b in &exp2_bytes[..exp2_len] {
                    tty.on_tty_byte(b);
                }
            }

            while let Some(ev) = self.scheduler.pop_due() {
                self.dispatch_event(ev);
            }
        }

        StepStopReason::MaxCyclesReached
    }

    fn dispatch_event(&mut self, ev: ScheduledEvent) {
        match ev.kind {
            EventKind::GpuScanline => {
                self.scanline += 1;
                if self.scanline == 240 {
                    // Enter VBlank: edge-latch IRQ0 and toggle interlace field bit 31
                    self.bus.latch_irq(0);
                    self.bus.gpu.toggle_interlace_field();
                } else if self.scanline >= 263 {
                    self.scanline = 0;
                }
                self.scheduler
                    .schedule(NTSC_CYCLES_PER_SCANLINE, EventKind::GpuScanline);
            }
            EventKind::VblankEdge => {
                self.bus.latch_irq(0);
                self.bus.gpu.toggle_interlace_field();
                self.scheduler
                    .schedule(NTSC_CYCLES_PER_FRAME, EventKind::VblankEdge);
            }
            _ => {}
        }
    }

    /// Serializes deterministic CPU + Scheduler state via `postcard` (`no_std + alloc` friendly).
    pub fn serialize_cpu_state(&self) -> Result<Vec<u8>, postcard::Error> {
        postcard::to_allocvec(&(&self.cpu, &self.scheduler))
    }

    /// Restores deterministic CPU + Scheduler state via `postcard`.
    pub fn deserialize_cpu_state(&mut self, bytes: &[u8]) -> Result<(), postcard::Error> {
        let (cpu, sched): (Cpu, EventScheduler) = postcard::from_bytes(bytes)?;
        self.cpu = cpu;
        self.scheduler = sched;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_bios_b0_3d_and_a0_3c_tty_putchar_and_sideloads_at_shell() {
        let mut sys = System::new_headless();
        sys.install_biosless_kernel_stub();

        // Build a synthetic PS-X EXE that prints 'H' via B0(0x3D) and 'i' via A0(0x3C), then loops
        let code_words: [u32; 9] = [
            0x2409_003D, // addiu $t1, $zero, 0x3D
            0x2404_0048, // addiu $a0, $zero, 'H'
            0x0800_002C, // j     0x0000_00B0 (wait: use jal 0x0000_00B0 -> 0x0C00_002C)
            0x0000_0000, // nop
            0x2409_003C, // addiu $t1, $zero, 0x3C
            0x2404_0069, // addiu $a0, $zero, 'i'
            0x0C00_0028, // jal   0x0000_00A0
            0x0000_0000, // nop
            0x1000_FFFF, // b     . (infinite loop)
        ];
        let mut raw_exe = vec![0u8; 0x800 + code_words.len() * 4];
        raw_exe[0..8].copy_from_slice(b"PS-X EXE");
        raw_exe[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        raw_exe[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
        raw_exe[0x1C..0x20].copy_from_slice(&((code_words.len() * 4) as u32).to_le_bytes());
        let mut fixed_words = code_words;
        fixed_words[2] = 0x0C00_002C; // jal 0x0000_00B0
        for (i, w) in fixed_words.iter().enumerate() {
            raw_exe[0x800 + i * 4..0x800 + (i + 1) * 4].copy_from_slice(&w.to_le_bytes());
        }

        let exe = PsxExe::parse(&raw_exe).unwrap();
        sys.set_sideload_exe(exe);
        sys.enable_trace(16);

        // Simulate BIOS jumping to shell entry 0x8003_0000
        sys.cpu.pc = BIOS_SHELL_ENTRY_PC;
        sys.cpu.next_pc = BIOS_SHELL_ENTRY_PC + 4;

        let mut tty_out = Vec::new();
        let reason = sys.run_until_cycle(500, false, &mut tty_out);
        assert_eq!(reason, StepStopReason::MaxCyclesReached);
        assert!(sys.exe_sideloaded());
        assert_eq!(tty_out, b"Hi");
        assert_eq!(sys.trace_entries().len(), 16);
    }
}
