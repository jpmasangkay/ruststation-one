pub mod cop0;
pub mod decode;

pub use cop0::{Cop0, ExceptionCode};

use crate::bus::{Bus, BusError, GpuBackend, Segment, decode_virtual_addr};
use crate::gte::Gte;
use alloc::boxed::Box;
use alloc::vec;
use serde::{Deserialize, Serialize};

const ICACHE_LINES: usize = 256;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingLoad {
    pub reg: u8,
    pub val: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ICache {
    /// Upper tag `[31:12]` | valid bitmask in bits `[3:0]` (1 bit per word in the 4-word line).
    tags: Box<[u32]>,
    /// `256 * 4 = 1024` cached 32-bit instruction words (`4 KB` total).
    words: Box<[u32]>,
}

impl Default for ICache {
    fn default() -> Self {
        Self::new()
    }
}

impl ICache {
    fn new() -> Self {
        Self {
            tags: vec![0u32; ICACHE_LINES].into_boxed_slice(),
            words: vec![0u32; ICACHE_LINES * 4].into_boxed_slice(),
        }
    }

    #[inline(always)]
    fn invalidate_addr(&mut self, vaddr: u32) {
        let line = ((vaddr >> 4) & 0xFF) as usize;
        self.tags[line] = 0;
    }
}

/// Sony PlayStation MIPS R3000A CPU (`CW33300` @ 33.8688 MHz).
#[repr(C, align(64))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cpu {
    pub regs: [u32; 32],
    pub pc: u32,
    pub next_pc: u32,
    pub current_pc: u32,
    pub last_opcode: u32,
    pub hi: u32,
    pub lo: u32,

    pub cop0: Cop0,
    pub gte: Gte,

    /// Two-slot load-delay pipeline (`in_flight_load` commits at end of current step;
    /// `queued_load` promotes at start of next step).
    pub(crate) in_flight_load: PendingLoad,
    pub(crate) queued_load: PendingLoad,

    /// Branch delay slot tracking.
    pub(crate) in_branch_delay: bool,
    pub(crate) next_is_branch: bool,
    pub(crate) branch_taken: bool,
    pub(crate) next_branch_taken: bool,

    icache: ICache,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new_reset()
    }
}

impl Cpu {
    #[must_use]
    pub fn new_reset() -> Self {
        Self {
            regs: [0; 32],
            pc: 0xBFC0_0000,
            next_pc: 0xBFC0_0004,
            current_pc: 0xBFC0_0000,
            last_opcode: 0,
            hi: 0,
            lo: 0,
            cop0: Cop0::new_reset(),
            gte: Gte::new(),
            in_flight_load: PendingLoad::default(),
            queued_load: PendingLoad::default(),
            in_branch_delay: false,
            next_is_branch: false,
            branch_taken: false,
            next_branch_taken: false,
            icache: ICache::new(),
        }
    }

    /// Writes an immediate ALU result to `reg`, canceling any in-flight delayed load to `reg`.
    #[inline(always)]
    pub(crate) fn set_reg(&mut self, reg: u8, val: u32) {
        debug_assert!(reg < 32);
        if self.in_flight_load.reg == reg {
            self.in_flight_load.reg = 0;
        }
        if reg != 0 {
            self.regs[reg as usize] = val;
        }
    }

    /// Queues a 1-instruction delayed load (`LW`/`LH`/`LB`/`LWL`/`LWR`/`MFC0`/`MFC2`/`CFC2`).
    #[inline(always)]
    pub(crate) fn queue_load(&mut self, reg: u8, val: u32) {
        debug_assert!(reg < 32);
        if self.in_flight_load.reg == reg {
            self.in_flight_load.reg = 0;
        }
        if reg != 0 {
            self.queued_load = PendingLoad { reg, val };
        }
    }

    /// Reads `reg` with hardware `LWL`/`LWR` forwarding (merges against an in-flight load to `reg`).
    #[inline(always)]
    #[must_use]
    pub(crate) const fn read_reg_for_lwl_lwr(&self, reg: u8) -> u32 {
        if self.in_flight_load.reg == reg && reg != 0 {
            self.in_flight_load.val
        } else {
            self.regs[reg as usize]
        }
    }

    /// Enters a MIPS exception (`general` vector `0x8000_0080`/`0xBFC0_0180`, or `debug` vector `0x8000_0040`/`0xBFC0_0140`).
    #[cold]
    pub(crate) fn raise_exception(
        &mut self,
        code: ExceptionCode,
        cop_ce: u8,
        is_debug_vector: bool,
    ) {
        // Any load from instruction N-1 completes before entering the exception handler
        if self.in_flight_load.reg != 0 {
            self.regs[self.in_flight_load.reg as usize] = self.in_flight_load.val;
            self.in_flight_load.reg = 0;
        }
        // Any load initiated by the faulting instruction itself is aborted
        self.queued_load = PendingLoad::default();

        // Push 3-level [KU, IE] mode stack left by 2 bits; clear KUc & IEc
        let pushed_modes = (self.cop0.sr & 0x3F) << 2;
        self.cop0.sr = (self.cop0.sr & !0x3F) | (pushed_modes & 0x3F);

        // Update CAUSE: preserve IP bits (8..15), set ExcCode (2..6), CE (28..29), BD (31), BT (30)
        let mut cause = self.cop0.cause & 0x0000_FF00;
        cause |= (code as u32) << 2;
        cause |= ((cop_ce & 0x03) as u32) << 28;

        if self.in_branch_delay {
            cause |= 1 << 31; // BD
            if self.branch_taken {
                cause |= 1 << 30; // BT
            }
            self.cop0.epc = self.current_pc.wrapping_sub(4);
            self.cop0.jumpdest = self.pc;
        } else {
            self.cop0.epc = self.current_pc;
        }
        self.cop0.cause = cause;

        let vector = match (self.cop0.is_bev(), is_debug_vector) {
            (false, false) => 0x8000_0080,
            (true, false) => 0xBFC0_0180,
            (false, true) => 0x8000_0040,
            (true, true) => 0xBFC0_0140,
        };
        self.pc = vector;
        self.next_pc = vector.wrapping_add(4);
        self.in_branch_delay = false;
        self.next_is_branch = false;
        self.branch_taken = false;
        self.next_branch_taken = false;
        self.regs[0] = 0;
    }

    /// Fetches a 32-bit instruction word, checking I-Cache in `KSEG0` / `KUSEG` when enabled.
    #[inline]
    fn fetch_instruction<G: GpuBackend>(
        &mut self,
        bus: &mut Bus<G>,
        vaddr: u32,
    ) -> Result<(u32, u64), BusError> {
        let addr = decode_virtual_addr(vaddr);
        let icache_enabled = (bus.biu_config() & (1 << 11)) != 0;
        let cacheable = icache_enabled
            && (addr.segment == Segment::Kseg0
                || (addr.segment == Segment::Kuseg && addr.phys < 0x2000_0000));

        if cacheable {
            let line = ((addr.phys >> 4) & 0xFF) as usize;
            let word_idx = ((addr.phys >> 2) & 0x03) as usize;
            let valid_bit = 1u32 << word_idx;
            let tag = addr.phys & 0xFFFF_F000;
            let entry_tag = self.icache.tags[line];

            if (entry_tag & 0xFFFF_F000) == tag && (entry_tag & valid_bit) != 0 {
                return Ok((self.icache.words[line * 4 + word_idx], 1));
            }

            // I-Cache miss: refill from `word_idx..4` to end of 16-byte cache line
            let mut valid_mask = if (entry_tag & 0xFFFF_F000) == tag {
                entry_tag & 0x0F
            } else {
                0
            };
            let base_vaddr = vaddr & !0x0F;
            let mut total_cycles = 2u64;
            for w in word_idx..4 {
                let (word, c) = bus.read32(base_vaddr + (w as u32) * 4)?;
                self.icache.words[line * 4 + w] = word;
                valid_mask |= 1u32 << w;
                total_cycles += if w == word_idx { c } else { 1 };
            }
            self.icache.tags[line] = tag | valid_mask;
            return Ok((self.icache.words[line * 4 + word_idx], total_cycles));
        }

        bus.read32(vaddr)
    }

    #[inline(always)]
    pub(crate) fn invalidate_icache_line(&mut self, vaddr: u32) {
        self.icache.invalidate_addr(vaddr);
    }

    /// Invalidates any cached instruction line matching physical RAM write at `vaddr`.
    #[inline(always)]
    pub(crate) fn notify_ram_write(&mut self, vaddr: u32) {
        let addr = decode_virtual_addr(vaddr);
        if addr.phys < 0x0080_0000 {
            let line = ((addr.phys >> 4) & 0xFF) as usize;
            let entry_tag = self.icache.tags[line];
            if ((entry_tag ^ addr.phys) & 0x001F_F000) == 0 {
                self.icache.tags[line] = 0;
            }
        }
    }

    /// Executes a single R3000A instruction and returns elapsed master clock cycles.
    pub fn step_instruction<G: GpuBackend>(&mut self, bus: &mut Bus<G>) -> u64 {
        debug_assert_eq!(self.regs[0], 0, "$zero invariant violated before step");

        // 1. Promote previous instruction's queued delayed load into `in_flight_load`
        self.in_flight_load = self.queued_load;
        self.queued_load = PendingLoad::default();

        self.current_pc = self.pc;
        self.in_branch_delay = self.next_is_branch;
        self.branch_taken = self.next_branch_taken;
        self.next_is_branch = false;
        self.next_branch_taken = false;

        // 2. Sample hardware & software interrupts synchronously
        if self.cop0.sync_irq_and_check(bus.irq_pending()) {
            self.raise_exception(ExceptionCode::Interrupt, 0, false);
            debug_assert_eq!(self.regs[0], 0);
            return 1;
        }

        // 3. Check PC alignment and User-Mode segment protection (`KSEG*` >= 0x8000_0000)
        if (self.current_pc & 3) != 0
            || (self.cop0.is_user_mode() && self.current_pc >= 0x8000_0000)
        {
            self.cop0.bad_vaddr = self.current_pc;
            self.raise_exception(ExceptionCode::AddressErrorLoad, 0, false);
            debug_assert_eq!(self.regs[0], 0);
            return 1;
        }

        // 4. Check hardware COP0 `BPC` execution breakpoint
        if self.cop0.check_exec_breakpoint(self.current_pc) {
            self.raise_exception(ExceptionCode::Breakpoint, 0, true);
            debug_assert_eq!(self.regs[0], 0);
            return 1;
        }

        // 5. Fetch instruction word
        let (instr, mut cycles) = match self.fetch_instruction(bus, self.current_pc) {
            Ok(res) => res,
            Err(_) => {
                self.raise_exception(ExceptionCode::InstructionBusError, 0, false);
                debug_assert_eq!(self.regs[0], 0);
                return 4;
            }
        };
        self.last_opcode = instr;

        // Advance PC pipeline before decoding
        self.pc = self.next_pc;
        self.next_pc = self.next_pc.wrapping_add(4);

        // 6. Decode and execute instruction
        cycles += self.decode_and_execute(instr, bus);

        // 7. Commit `in_flight_load` unless canceled by delay-slot overwrite or exception
        if self.in_flight_load.reg != 0 {
            self.regs[self.in_flight_load.reg as usize] = self.in_flight_load.val;
            self.in_flight_load.reg = 0;
        }

        self.regs[0] = 0;
        debug_assert_eq!(self.regs[0], 0, "$zero invariant violated after step");
        cycles
    }
}
