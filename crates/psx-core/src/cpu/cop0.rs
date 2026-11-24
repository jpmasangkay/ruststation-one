use serde::{Deserialize, Serialize};

/// MIPS R3000A exception codes stored in `CAUSE[6:2]`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ExceptionCode {
    /// `0x00`: External hardware or software interrupt (`Int`).
    Interrupt = 0x00,
    /// `0x04`: Address error on instruction fetch or data load (`AdEL`).
    AddressErrorLoad = 0x04,
    /// `0x05`: Address error on data store (`AdES`).
    AddressErrorStore = 0x05,
    /// `0x06`: Bus error on instruction fetch (`IBE`).
    InstructionBusError = 0x06,
    /// `0x07`: Bus error on data load/store (`DBE`).
    DataBusError = 0x07,
    /// `0x08`: Unconditional `SYSCALL` trap (`Sys`).
    Syscall = 0x08,
    /// `0x09`: Software `BREAK` instruction or hardware `DCIC` debug breakpoint (`Bp`).
    Breakpoint = 0x09,
    /// `0x0A`: Reserved / illegal opcode (`RI`).
    ReservedInstruction = 0x0A,
    /// `0x0B`: Coprocessor unusable (`CpU`).
    CoprocessorUnusable = 0x0B,
    /// `0x0C`: Signed two's-complement arithmetic overflow (`Ov`).
    ArithmeticOverflow = 0x0C,
}

/// System Control Coprocessor (`COP0`) registers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cop0 {
    /// `r3`: Breakpoint on execute address (`BPC`).
    pub bpc: u32,
    /// `r5`: Breakpoint on data access address (`BDA`).
    pub bda: u32,
    /// `r6`: Memorized jump target address (`JUMPDEST` / `TAR`, read-only).
    pub jumpdest: u32,
    /// `r7`: Breakpoint / debug control & status (`DCIC`).
    pub dcic: u32,
    /// `r8`: Bad Virtual Address (`BadVAddr`, latched on `AdEL`/`AdES`, read-only).
    pub bad_vaddr: u32,
    /// `r9`: Data address breakpoint mask (`BDAM`).
    pub bdam: u32,
    /// `r11`: Execute PC breakpoint mask (`BPCM`).
    pub bpcm: u32,
    /// `r12`: Status Register (`SR`, reset = `0x0040_0000` with `BEV = 1`).
    pub sr: u32,
    /// `r13`: Exception Cause Register (`CAUSE`).
    pub cause: u32,
    /// `r14`: Exception Program Counter (`EPC`, read-only).
    pub epc: u32,
}

impl Default for Cop0 {
    fn default() -> Self {
        Self::new_reset()
    }
}

impl Cop0 {
    /// Processor Revision Identifier (`PRId`, `COP0 r15`) on Sony CXD8530 / CW33300.
    pub const PRID: u32 = 0x0000_0002;

    /// Writable bitmask for `SR` (`r12`): `CU0..3`, `RE`, `BEV`, `TS`, `PE`, `CM`, `PZ`, `SwC`, `IsC`, `Im0..7`, `[KU,IE]` stack.
    const SR_WRITE_MASK: u32 = 0xF27F_FF3F;

    /// Writable bitmask for `DCIC` (`r7`).
    const DCIC_WRITE_MASK: u32 = 0xFF80_F03F;

    #[must_use]
    pub const fn new_reset() -> Self {
        Self {
            bpc: 0,
            bda: 0,
            jumpdest: 0,
            dcic: 0,
            bad_vaddr: 0,
            bdam: 0,
            bpcm: 0,
            sr: 0x0040_0000, // BEV = 1
            cause: 0,
            epc: 0,
        }
    }

    /// Returns `true` when CPU is currently in User Mode (`SR.KUc == 1`).
    #[inline(always)]
    #[must_use]
    pub const fn is_user_mode(&self) -> bool {
        (self.sr & 0x02) != 0
    }

    /// Returns `true` when Cache Isolation is active (`SR.IsC == 1`, bit 16).
    #[inline(always)]
    #[must_use]
    pub const fn is_cache_isolated(&self) -> bool {
        (self.sr & (1 << 16)) != 0
    }

    /// Returns `true` when Boot Exception Vectors are active (`SR.BEV == 1`, bit 22).
    #[inline(always)]
    #[must_use]
    pub const fn is_bev(&self) -> bool {
        (self.sr & (1 << 22)) != 0
    }

    /// Returns `true` if coprocessor `cop_num` (`0..=3`) is accessible in the current mode.
    #[inline(always)]
    #[must_use]
    pub const fn is_cop_usable(&self, cop_num: u8) -> bool {
        if cop_num == 0 && !self.is_user_mode() {
            return true;
        }
        (self.sr & (1u32 << (28 + cop_num))) != 0
    }

    #[inline]
    #[must_use]
    pub const fn read_reg(&self, rd: u8) -> u32 {
        match rd {
            3 => self.bpc,
            5 => self.bda,
            6 => self.jumpdest,
            7 => self.dcic,
            8 => self.bad_vaddr,
            9 => self.bdam,
            11 => self.bpcm,
            12 => self.sr,
            13 => self.cause,
            14 => self.epc,
            15 => Self::PRID,
            _ => 0,
        }
    }

    #[inline]
    pub fn write_reg(&mut self, rd: u8, val: u32) {
        match rd {
            3 => self.bpc = val,
            5 => self.bda = val,
            7 => self.dcic = val & Self::DCIC_WRITE_MASK,
            9 => self.bdam = val,
            11 => self.bpcm = val,
            12 => self.sr = val & Self::SR_WRITE_MASK,
            // Only software interrupt bits Sw0..Sw1 (bits 8..9) are writable in CAUSE!
            13 => self.cause = (self.cause & !0x0300) | (val & 0x0300),
            _ => {}
        }
    }

    /// Executes `RFE` (Restore From Exception): shifts the 3-level `[KU, IE]` mode stack right by 2 bits,
    /// preserving `[KUo, IEo]` in bits `4..5`. Does NOT modify `PC`!
    #[inline]
    pub fn rfe(&mut self) {
        let shifted = (self.sr & 0x3F) >> 2;
        let preserved_outer = self.sr & 0x30;
        self.sr = (self.sr & !0x3F) | shifted | preserved_outer;
    }

    /// Updates hardware interrupt line `IP2` (`CAUSE` bit 10) and returns `true` if an interrupt should fire.
    #[inline(always)]
    pub fn sync_irq_and_check(&mut self, hw_irq_pending: bool) -> bool {
        if hw_irq_pending {
            self.cause |= 1 << 10;
        } else {
            self.cause &= !(1 << 10);
        }
        let iec = (self.sr & 1) != 0;
        iec && ((self.sr & self.cause & 0xFF00) != 0)
    }

    /// Checks hardware PC execution breakpoint (`BPC` / `BPCM` under `DCIC` enables).
    /// Returns `true` if a debug breakpoint trap to `0x8000_0040` / `0xBFC0_0140` triggered.
    #[inline]
    pub fn check_exec_breakpoint(&mut self, pc: u32) -> bool {
        // Super-master bit 23 (DE), master bit 30, and PC-break bit 24 must all be enabled
        const REQ: u32 = (1 << 23) | (1 << 24) | (1 << 30);
        if (self.dcic & REQ) != REQ {
            return false;
        }
        if ((pc ^ self.bpc) & self.bpcm) == 0 {
            self.dcic |= (1 << 0) | (1 << 1); // Any-break + Code-break status bits
            return (self.dcic & (1 << 31)) != 0; // Trap enabled if bit 31 (TR) set
        }
        false
    }

    /// Checks hardware Data read/write breakpoint (`BDA` / `BDAM` under `DCIC` enables).
    #[inline]
    pub fn check_data_breakpoint(&mut self, vaddr: u32, is_write: bool) -> bool {
        const REQ: u32 = (1 << 23) | (1 << 25) | (1 << 30);
        if (self.dcic & REQ) != REQ {
            return false;
        }
        let dir_bit = if is_write { 1 << 27 } else { 1 << 26 };
        if (self.dcic & dir_bit) == 0 {
            return false;
        }
        if ((vaddr ^ self.bda) & self.bdam) == 0 {
            let status_dir = if is_write { 1 << 4 } else { 1 << 3 };
            self.dcic |= (1 << 0) | (1 << 2) | status_dir;
            return (self.dcic & (1 << 31)) != 0;
        }
        false
    }
}
