use alloc::boxed::Box;
use alloc::vec;
use serde::{Deserialize, Serialize};

pub const RAM_BYTES: usize = 2 * 1024 * 1024;
pub const SCRATCHPAD_BYTES: usize = 1024;
pub const BIOS_BYTES: usize = 512 * 1024;
pub const SPU_MMIO_BYTES: usize = 0x400;
pub const MAX_DMA_OTC_WORDS: u32 = 0x0010_0000;
pub const MAX_DMA_LINKED_LIST_NODES: usize = 16_384;

/// MIPS virtual memory segment classification.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Segment {
    /// `0x0000_0000..=0x7FFF_FFFF` (User/Kernel, Cached, Scratchpad valid in low 512 MB).
    Kuseg,
    /// `0x8000_0000..=0x9FFF_FFFF` (Kernel, Cached, mirrors physical `0x0000_0000..=0x1FFF_FFFF`).
    Kseg0,
    /// `0xA000_0000..=0xBFFF_FFFF` (Kernel, Uncached, Scratchpad FORBIDDEN).
    Kseg1,
    /// `0xC000_0000..=0xFFFF_FFFF` (Kernel, Unmapped except `0xFFFE_0130` `BIU_CONFIG`).
    Kseg2,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DecodedAddr {
    pub vaddr: u32,
    pub phys: u32,
    pub segment: Segment,
}

/// Segment mask indexed by `vaddr >> 29`.
const REGION_MASK: [u32; 8] = [
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0x7FFF_FFFF,
    0x1FFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
];

#[inline(always)]
#[must_use]
pub const fn decode_virtual_addr(vaddr: u32) -> DecodedAddr {
    let top3 = (vaddr >> 29) as usize;
    let segment = match top3 {
        0..=3 => Segment::Kuseg,
        4 => Segment::Kseg0,
        5 => Segment::Kseg1,
        _ => Segment::Kseg2,
    };
    DecodedAddr {
        vaddr,
        phys: vaddr & REGION_MASK[top3],
        segment,
    }
}

/// Hardware bus error (`IBE` on instruction fetch, `DBE` on data load/store).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BusError {
    /// Attempted to access Scratchpad (`0x1F80_0000..=0x1F80_03FF`) through uncached `KSEG1` (`0xBF80_0000`).
    Kseg1ScratchpadAccess(u32),
    /// Access fell into an unmapped physical or `KSEG2` region.
    UnmappedAddress(u32),
}

/// Trait implemented by GPU rasterizers (Phase 1 `NullGpu`, Phase 2 SIMD Software, Phase 4 `wgpu`).
pub trait GpuBackend {
    fn read_gpuread(&mut self) -> u32;
    fn read_gpustat(&self) -> u32;
    fn write_gp0(&mut self, word: u32);
    fn write_gp1(&mut self, word: u32);
    fn toggle_interlace_field(&mut self);
}

/// Minimal Phase 1 GPU stub reporting ready flags in `GPUSTAT` so BIOS and CPU/GTE tests boot cleanly.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NullGpu {
    gpustat: u32,
    gpuread_latch: u32,
}

impl Default for NullGpu {
    fn default() -> Self {
        Self::new()
    }
}

impl NullGpu {
    /// Bits 26 (Ready for Cmd), 27 (Ready VRAM->CPU), 28 (Ready DMA block) set; Bit 13 interlace field 1.
    pub const INITIAL_GPUSTAT: u32 = (1 << 26) | (1 << 27) | (1 << 28) | (1 << 13) | (1 << 10);

    #[must_use]
    pub const fn new() -> Self {
        Self {
            gpustat: Self::INITIAL_GPUSTAT,
            gpuread_latch: 0,
        }
    }
}

impl GpuBackend for NullGpu {
    #[inline]
    fn read_gpuread(&mut self) -> u32 {
        self.gpuread_latch
    }

    #[inline]
    fn read_gpustat(&self) -> u32 {
        self.gpustat
    }

    #[inline]
    fn write_gp0(&mut self, word: u32) {
        let cmd = word >> 24;
        if cmd == 0xE1 {
            // Draw Mode setting updates low bits of GPUSTAT
            self.gpustat = (self.gpustat & !0x07FF) | (word & 0x07FF);
        }
    }

    #[inline]
    fn write_gp1(&mut self, word: u32) {
        let cmd = (word >> 24) & 0x3F;
        match cmd {
            0x00 => {
                self.gpustat = Self::INITIAL_GPUSTAT;
                self.gpuread_latch = 0;
            }
            0x04 => {
                let dma_dir = word & 3;
                self.gpustat = (self.gpustat & !(3 << 29)) | (dma_dir << 29);
            }
            0x10 => {
                // GPU Info query (type 2 = standard 1MB/2MB CXD8561 GPU version)
                if (word & 0xF) == 0x07 {
                    self.gpuread_latch = 2;
                }
            }
            _ => {}
        }
    }

    #[inline]
    fn toggle_interlace_field(&mut self) {
        self.gpustat ^= 1 << 31;
    }
}

#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
struct DmaChannelRegs {
    madr: u32,
    bcr: u32,
    chcr: u32,
}

#[derive(Copy, Clone, Debug, Default, Serialize, Deserialize)]
struct TimerRegs {
    counter: u16,
    mode: u16,
    target: u16,
    last_sync_cycle: u64,
}

/// Phase 1 CD-ROM stub supporting index banking and basic status/FIFO queries for BIOS boot.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CdRomStub {
    index: u8,
    irq_enable: u8,
    irq_flags: u8,
    resp_fifo: [u8; 16],
    resp_len: u8,
    resp_pos: u8,
    param_fifo: [u8; 16],
    param_len: u8,
}

impl CdRomStub {
    fn status_byte(&self) -> u8 {
        let mut st = self.index & 0x03;
        if self.param_len == 0 {
            st |= 1 << 3; // PRMEMPT
        }
        if self.param_len < 16 {
            st |= 1 << 4; // PRMWRDY
        }
        if self.resp_pos < self.resp_len {
            st |= 1 << 5; // RSLRRDY
        }
        st
    }

    fn push_resp(&mut self, bytes: &[u8]) {
        self.resp_len = 0;
        self.resp_pos = 0;
        for &b in bytes.iter().take(16) {
            self.resp_fifo[self.resp_len as usize] = b;
            self.resp_len += 1;
        }
    }

    fn execute_command(&mut self, cmd: u8) -> bool {
        let sub = if self.param_len > 0 {
            self.param_fifo[0]
        } else {
            0
        };
        self.param_len = 0;

        match cmd {
            // Getstat (0x01) / Init (0x0A) / Demute (0x0C): report Shell Open / No Disc (0x10)
            0x01 | 0x0A | 0x0C => {
                self.push_resp(&[0x10]);
                self.irq_flags = (self.irq_flags & !0x07) | 0x03; // INT3 Acknowledge
            }
            // Test (0x19) subfunction 0x20: CD-ROM BIOS version date
            0x19 if sub == 0x20 => {
                self.push_resp(&[0x97, 0x01, 0x10, 0xC2]);
                self.irq_flags = (self.irq_flags & !0x07) | 0x03; // INT3
            }
            // ReadTOC / Seek / ReadN with no disc inserted -> INT5 Error (No Disc)
            _ => {
                self.push_resp(&[0x11, 0x80]);
                self.irq_flags = (self.irq_flags & !0x07) | 0x05; // INT5 Error
            }
        }
        (self.irq_flags & self.irq_enable & 0x1F) != 0
    }
}

/// Physical PS1 Memory & MMIO Bus.
pub struct Bus<G: GpuBackend = NullGpu> {
    ram: Box<[u8]>,
    scratchpad: [u8; SCRATCHPAD_BYTES],
    bios: Box<[u8]>,
    spu_mmio: Box<[u8]>,

    /// `0x1F80_1000..=0x1F80_1020` Memory Control 1 & 2.
    memctrl: [u32; 9],
    /// `0x1F80_1060` `RAM_SIZE` (default `0x0000_0B88` = 2MB mirrored 4x across 8MB).
    ram_size_reg: u32,
    /// `0xFFFE_0130` `BIU_CONFIG` Cache Control (`KSEG2` only).
    biu_config: u32,

    /// `0x1F80_1070` edge-latched Interrupt Status (write-0-to-acknowledge).
    i_stat: u16,
    /// `0x1F80_1074` Interrupt Mask.
    i_mask: u16,

    /// 7 DMA channels + `DPCR` (`0x1F80_10F0`) + `DICR` (`0x1F80_10F4`).
    dma_channels: [DmaChannelRegs; 7],
    dma_dpcr: u32,
    dma_dicr: u32,

    /// 3 Root Counters (`0x1F80_1100..=0x1F80_112F`).
    timers: [TimerRegs; 3],

    /// `SIO0` / `SIO1` stub registers (`0x1F80_1040..=0x1F80_105F`).
    sio_regs: [u16; 16],

    /// CD-ROM stub (`0x1F80_1800..=0x1F80_1803`).
    cdrom: CdRomStub,

    /// MDEC stub status (`0x1F80_1824`).
    mdec_status: u32,

    /// Expansion 2 boot POST code (`0x1F80_2041`) & DUART debug byte capture.
    post_code: u8,
    exp2_tty_buf: [u8; 64],
    exp2_tty_len: usize,

    /// Last value read on the bus (Open-Bus behavior).
    last_open_bus: u32,
    /// Current master clock cycle synced from `System` for lazy timer reads.
    clock_cycle: u64,

    pub gpu: G,
}

impl<G: GpuBackend> Bus<G> {
    #[must_use]
    pub fn new(gpu: G) -> Self {
        let mut bios = vec![0u8; BIOS_BYTES].into_boxed_slice();
        // Default reset stub (`j 0xBFC0_0000; nop`) so an empty bus loops safely unless BIOS loaded
        bios[0..4].copy_from_slice(&0x0BF0_0000u32.to_le_bytes());

        Self {
            ram: vec![0u8; RAM_BYTES].into_boxed_slice(),
            scratchpad: [0u8; SCRATCHPAD_BYTES],
            bios,
            spu_mmio: vec![0u8; SPU_MMIO_BYTES].into_boxed_slice(),
            memctrl: [
                0x1F00_0000,
                0x1F80_2000,
                0x0013_243F,
                0x0000_3022,
                0x0013_243F,
                0x2009_31E1,
                0x0002_0843,
                0x0002_0943,
                0x0003_1125,
            ],
            ram_size_reg: 0x0000_0B88,
            biu_config: 0,
            i_stat: 0,
            i_mask: 0,
            dma_channels: [DmaChannelRegs::default(); 7],
            dma_dpcr: 0x0765_4321,
            dma_dicr: 0,
            timers: [TimerRegs::default(); 3],
            sio_regs: [0; 16],
            cdrom: CdRomStub::default(),
            mdec_status: 0x8004_0000, // FIFO empty, not busy
            post_code: 0,
            exp2_tty_buf: [0; 64],
            exp2_tty_len: 0,
            last_open_bus: 0xFFFF_FFFF,
            clock_cycle: 0,
            gpu,
        }
    }

    /// Loads a 512 KB PS1 BIOS image (`OpenBIOS` or retail `scph*.bin`).
    pub fn load_bios(&mut self, image: &[u8]) -> Result<(), &'static str> {
        if image.len() != BIOS_BYTES {
            return Err("PS1 BIOS image must be exactly 524,288 bytes (512 KB)");
        }
        self.bios.copy_from_slice(image);
        Ok(())
    }

    #[inline(always)]
    pub fn sync_clock(&mut self, now_cycle: u64) {
        self.clock_cycle = now_cycle;
    }

    #[inline(always)]
    #[must_use]
    pub const fn irq_pending(&self) -> bool {
        (self.i_stat & self.i_mask) != 0
    }

    /// Edge-latches an interrupt line (`0..=10`) into `I_STAT`.
    #[inline(always)]
    pub fn latch_irq(&mut self, irq_bit: u8) {
        debug_assert!(irq_bit <= 10);
        self.i_stat |= 1u16 << irq_bit;
    }

    #[inline(always)]
    #[must_use]
    pub const fn i_stat(&self) -> u16 {
        self.i_stat
    }

    #[inline(always)]
    #[must_use]
    pub const fn i_mask(&self) -> u16 {
        self.i_mask
    }

    #[inline(always)]
    #[must_use]
    pub const fn biu_config(&self) -> u32 {
        self.biu_config
    }

    #[inline(always)]
    #[must_use]
    pub const fn post_code(&self) -> u8 {
        self.post_code
    }

    #[inline(always)]
    #[must_use]
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    #[inline(always)]
    #[must_use]
    pub fn ram_mut(&mut self) -> &mut [u8] {
        &mut self.ram
    }

    #[inline(always)]
    #[must_use]
    pub fn scratchpad(&self) -> &[u8; SCRATCHPAD_BYTES] {
        &self.scratchpad
    }

    pub fn drain_exp2_tty(&mut self) -> ([u8; 64], usize) {
        let out = self.exp2_tty_buf;
        let len = self.exp2_tty_len;
        self.exp2_tty_len = 0;
        (out, len)
    }

    /// Returns active RAM window limit based on `RAM_SIZE` (`0x1F80_1060` bits `9..11`).
    #[inline(always)]
    const fn active_ram_window_bytes(&self) -> u32 {
        match (self.ram_size_reg >> 9) & 0x07 {
            0 | 1 => 0x0010_0000, // 1 MB
            2 | 3 => 0x0040_0000, // 4 MB
            4 | 5 => 0x0080_0000, // 8 MB (2 MB mirrored 4x — standard PS1 default)
            _ => 0x0080_0000,
        }
    }

    // ========================================================================
    // 8-bit, 16-bit, and 32-bit Bus Reads
    // ========================================================================

    pub fn read8(&mut self, vaddr: u32) -> Result<(u8, u64), BusError> {
        let addr = decode_virtual_addr(vaddr);
        if addr.segment == Segment::Kseg2 {
            if (vaddr & !3) == 0xFFFE_0130 {
                let shift = (vaddr & 3) * 8;
                let byte = ((self.biu_config >> shift) & 0xFF) as u8;
                return Ok((byte, 1));
            }
            return Err(BusError::UnmappedAddress(vaddr));
        }
        if addr.segment == Segment::Kuseg && addr.phys >= 0x2000_0000 {
            return Err(BusError::UnmappedAddress(vaddr));
        }

        let (byte, cycles) = match addr.phys {
            p if p < self.active_ram_window_bytes() => {
                (self.ram[(p as usize) & (RAM_BYTES - 1)], 5)
            }
            0x0000_0000..=0x1EFF_FFFF => return Err(BusError::UnmappedAddress(vaddr)),
            // Expansion Region 1 (PIO / GameShark): returns 0xFF when unattached
            0x1F00_0000..=0x1F7F_FFFF => (0xFF, 6),
            // 1 KB Scratchpad: FORBIDDEN in KSEG1!
            0x1F80_0000..=0x1F80_03FF => {
                if addr.segment == Segment::Kseg1 {
                    return Err(BusError::Kseg1ScratchpadAccess(vaddr));
                }
                (self.scratchpad[(addr.phys & 0x3FF) as usize], 1)
            }
            0x1F80_0400..=0x1F80_0FFF => return Err(BusError::UnmappedAddress(vaddr)),
            0x1F80_1000..=0x1F80_1FFF => (self.read_mmio8(addr.phys), 2),
            // Expansion Region 2 (DUART / Debug)
            0x1F80_2000..=0x1F80_2FFF => {
                let val = match addr.phys {
                    0x1F80_2021 | 0x1F80_2023 => 0x04, // DUART TX ready flag for debug TTY ROMs!
                    _ => 0xFF,
                };
                (val, 6)
            }
            // Expansion Region 3
            0x1FA0_0000..=0x1FBF_FFFF => (0xFF, 6),
            // 512 KB BIOS ROM (8-bit bus = 6 cycles for 1 byte)
            0x1FC0_0000..=0x1FC7_FFFF => (self.bios[(addr.phys & 0x0007_FFFF) as usize], 6),
            _ => return Err(BusError::UnmappedAddress(vaddr)),
        };

        let shift = (vaddr & 3) * 8;
        self.last_open_bus = (self.last_open_bus & !(0xFF << shift)) | ((byte as u32) << shift);
        Ok((byte, cycles))
    }

    pub fn read16(&mut self, vaddr: u32) -> Result<(u16, u64), BusError> {
        debug_assert_eq!(vaddr & 1, 0, "Unaligned read16 must trap AdEL in CPU");
        let addr = decode_virtual_addr(vaddr);
        if addr.segment == Segment::Kseg2 {
            if (vaddr & !3) == 0xFFFE_0130 {
                let shift = (vaddr & 2) * 8;
                return Ok((((self.biu_config >> shift) & 0xFFFF) as u16, 1));
            }
            return Err(BusError::UnmappedAddress(vaddr));
        }
        if addr.segment == Segment::Kuseg && addr.phys >= 0x2000_0000 {
            return Err(BusError::UnmappedAddress(vaddr));
        }

        let (half, cycles) = match addr.phys {
            p if p < self.active_ram_window_bytes() => {
                let off = (p as usize) & (RAM_BYTES - 2);
                (u16::from_le_bytes([self.ram[off], self.ram[off + 1]]), 5)
            }
            0x0000_0000..=0x1EFF_FFFF => return Err(BusError::UnmappedAddress(vaddr)),
            0x1F00_0000..=0x1F7F_FFFF => (0xFFFF, 12),
            0x1F80_0000..=0x1F80_03FF => {
                if addr.segment == Segment::Kseg1 {
                    return Err(BusError::Kseg1ScratchpadAccess(vaddr));
                }
                let off = (addr.phys & 0x3FE) as usize;
                (
                    u16::from_le_bytes([self.scratchpad[off], self.scratchpad[off + 1]]),
                    1,
                )
            }
            0x1F80_0400..=0x1F80_0FFF => return Err(BusError::UnmappedAddress(vaddr)),
            0x1F80_1000..=0x1F80_1FFF => {
                let cycles = if addr.phys >= 0x1F80_1C00 { 16 } else { 2 };
                (self.read_mmio16(addr.phys), cycles)
            }
            0x1F80_2000..=0x1F80_2FFF => (0xFFFF, 12),
            0x1FA0_0000..=0x1FBF_FFFF => (0xFFFF, 6),
            0x1FC0_0000..=0x1FC7_FFFF => {
                let off = (addr.phys & 0x0007_FFFE) as usize;
                (u16::from_le_bytes([self.bios[off], self.bios[off + 1]]), 12)
            }
            _ => return Err(BusError::UnmappedAddress(vaddr)),
        };

        let shift = (vaddr & 2) * 8;
        self.last_open_bus = (self.last_open_bus & !(0xFFFF << shift)) | ((half as u32) << shift);
        Ok((half, cycles))
    }

    pub fn read32(&mut self, vaddr: u32) -> Result<(u32, u64), BusError> {
        debug_assert_eq!(vaddr & 3, 0, "Unaligned read32 must trap AdEL in CPU");
        let addr = decode_virtual_addr(vaddr);
        if addr.segment == Segment::Kseg2 {
            if vaddr == 0xFFFE_0130 {
                self.last_open_bus = self.biu_config;
                return Ok((self.biu_config, 1));
            }
            return Err(BusError::UnmappedAddress(vaddr));
        }
        if addr.segment == Segment::Kuseg && addr.phys >= 0x2000_0000 {
            return Err(BusError::UnmappedAddress(vaddr));
        }

        let (word, cycles) = match addr.phys {
            p if p < self.active_ram_window_bytes() => {
                let off = (p as usize) & (RAM_BYTES - 4);
                let w = u32::from_le_bytes([
                    self.ram[off],
                    self.ram[off + 1],
                    self.ram[off + 2],
                    self.ram[off + 3],
                ]);
                (w, 5)
            }
            0x0000_0000..=0x1EFF_FFFF => return Err(BusError::UnmappedAddress(vaddr)),
            0x1F00_0000..=0x1F7F_FFFF => (0xFFFF_FFFF, 24),
            0x1F80_0000..=0x1F80_03FF => {
                if addr.segment == Segment::Kseg1 {
                    return Err(BusError::Kseg1ScratchpadAccess(vaddr));
                }
                let off = (addr.phys & 0x3FC) as usize;
                let w = u32::from_le_bytes([
                    self.scratchpad[off],
                    self.scratchpad[off + 1],
                    self.scratchpad[off + 2],
                    self.scratchpad[off + 3],
                ]);
                (w, 1)
            }
            0x1F80_0400..=0x1F80_0FFF => return Err(BusError::UnmappedAddress(vaddr)),
            0x1F80_1000..=0x1F80_1FFF => {
                let cycles = if addr.phys >= 0x1F80_1C00 { 32 } else { 2 };
                (self.read_mmio32(addr.phys), cycles)
            }
            0x1F80_2000..=0x1F80_2FFF => (0xFFFF_FFFF, 24),
            0x1FA0_0000..=0x1FBF_FFFF => (0xFFFF_FFFF, 12),
            0x1FC0_0000..=0x1FC7_FFFF => {
                let off = (addr.phys & 0x0007_FFFC) as usize;
                let w = u32::from_le_bytes([
                    self.bios[off],
                    self.bios[off + 1],
                    self.bios[off + 2],
                    self.bios[off + 3],
                ]);
                (w, 24)
            }
            _ => return Err(BusError::UnmappedAddress(vaddr)),
        };

        self.last_open_bus = word;
        Ok((word, cycles))
    }

    // ========================================================================
    // 8-bit, 16-bit, and 32-bit Bus Writes
    // ========================================================================

    pub fn write8(&mut self, vaddr: u32, val: u8) -> Result<u64, BusError> {
        let addr = decode_virtual_addr(vaddr);
        if addr.segment == Segment::Kseg2 {
            if (vaddr & !3) == 0xFFFE_0130 {
                let shift = (vaddr & 3) * 8;
                self.biu_config = (self.biu_config & !(0xFF << shift)) | ((val as u32) << shift);
                return Ok(1);
            }
            return Err(BusError::UnmappedAddress(vaddr));
        }
        if addr.segment == Segment::Kuseg && addr.phys >= 0x2000_0000 {
            return Err(BusError::UnmappedAddress(vaddr));
        }

        match addr.phys {
            p if p < self.active_ram_window_bytes() => {
                self.ram[(p as usize) & (RAM_BYTES - 1)] = val;
                Ok(1)
            }
            0x0000_0000..=0x1EFF_FFFF => Err(BusError::UnmappedAddress(vaddr)),
            0x1F00_0000..=0x1F7F_FFFF => Ok(1),
            0x1F80_0000..=0x1F80_03FF => {
                if addr.segment == Segment::Kseg1 {
                    return Err(BusError::Kseg1ScratchpadAccess(vaddr));
                }
                self.scratchpad[(addr.phys & 0x3FF) as usize] = val;
                Ok(1)
            }
            0x1F80_0400..=0x1F80_0FFF => Err(BusError::UnmappedAddress(vaddr)),
            0x1F80_1000..=0x1F80_1FFF => {
                self.write_mmio8(addr.phys, val);
                Ok(1)
            }
            0x1F80_2000..=0x1F80_2FFF => {
                match addr.phys {
                    0x1F80_2041 => self.post_code = val,
                    // DUART Channel A/B TX holding register & PCSX-Redux / OpenBIOS debug console ports
                    0x1F80_2023 | 0x1F80_202B | 0x1F80_2080
                        if self.exp2_tty_len < self.exp2_tty_buf.len() =>
                    {
                        self.exp2_tty_buf[self.exp2_tty_len] = val;
                        self.exp2_tty_len += 1;
                    }
                    _ => {}
                }
                Ok(1)
            }
            0x1FA0_0000..=0x1FBF_FFFF => Ok(1),
            0x1FC0_0000..=0x1FC7_FFFF => Ok(1), // Writes to read-only BIOS ROM are ignored
            _ => Err(BusError::UnmappedAddress(vaddr)),
        }
    }

    pub fn write16(&mut self, vaddr: u32, val: u16) -> Result<u64, BusError> {
        debug_assert_eq!(vaddr & 1, 0, "Unaligned write16 must trap AdES in CPU");
        let addr = decode_virtual_addr(vaddr);
        if addr.segment == Segment::Kseg2 {
            if (vaddr & !3) == 0xFFFE_0130 {
                let shift = (vaddr & 2) * 8;
                self.biu_config = (self.biu_config & !(0xFFFF << shift)) | ((val as u32) << shift);
                return Ok(1);
            }
            return Err(BusError::UnmappedAddress(vaddr));
        }
        if addr.segment == Segment::Kuseg && addr.phys >= 0x2000_0000 {
            return Err(BusError::UnmappedAddress(vaddr));
        }

        match addr.phys {
            p if p < self.active_ram_window_bytes() => {
                let off = (p as usize) & (RAM_BYTES - 2);
                self.ram[off..off + 2].copy_from_slice(&val.to_le_bytes());
                Ok(1)
            }
            0x0000_0000..=0x1EFF_FFFF => Err(BusError::UnmappedAddress(vaddr)),
            0x1F00_0000..=0x1F7F_FFFF => Ok(1),
            0x1F80_0000..=0x1F80_03FF => {
                if addr.segment == Segment::Kseg1 {
                    return Err(BusError::Kseg1ScratchpadAccess(vaddr));
                }
                let off = (addr.phys & 0x3FE) as usize;
                self.scratchpad[off..off + 2].copy_from_slice(&val.to_le_bytes());
                Ok(1)
            }
            0x1F80_0400..=0x1F80_0FFF => Err(BusError::UnmappedAddress(vaddr)),
            0x1F80_1000..=0x1F80_1FFF => {
                self.write_mmio16(addr.phys, val);
                Ok(1)
            }
            0x1F80_2000..=0x1F80_2FFF | 0x1FA0_0000..=0x1FBF_FFFF | 0x1FC0_0000..=0x1FC7_FFFF => {
                Ok(1)
            }
            _ => Err(BusError::UnmappedAddress(vaddr)),
        }
    }

    pub fn write32(&mut self, vaddr: u32, val: u32) -> Result<u64, BusError> {
        debug_assert_eq!(vaddr & 3, 0, "Unaligned write32 must trap AdES in CPU");
        let addr = decode_virtual_addr(vaddr);
        if addr.segment == Segment::Kseg2 {
            if vaddr == 0xFFFE_0130 {
                self.biu_config = val;
                return Ok(1);
            }
            return Err(BusError::UnmappedAddress(vaddr));
        }
        if addr.segment == Segment::Kuseg && addr.phys >= 0x2000_0000 {
            return Err(BusError::UnmappedAddress(vaddr));
        }

        match addr.phys {
            p if p < self.active_ram_window_bytes() => {
                let off = (p as usize) & (RAM_BYTES - 4);
                self.ram[off..off + 4].copy_from_slice(&val.to_le_bytes());
                Ok(1)
            }
            0x0000_0000..=0x1EFF_FFFF => Err(BusError::UnmappedAddress(vaddr)),
            0x1F00_0000..=0x1F7F_FFFF => Ok(1),
            0x1F80_0000..=0x1F80_03FF => {
                if addr.segment == Segment::Kseg1 {
                    return Err(BusError::Kseg1ScratchpadAccess(vaddr));
                }
                let off = (addr.phys & 0x3FC) as usize;
                self.scratchpad[off..off + 4].copy_from_slice(&val.to_le_bytes());
                Ok(1)
            }
            0x1F80_0400..=0x1F80_0FFF => Err(BusError::UnmappedAddress(vaddr)),
            0x1F80_1000..=0x1F80_1FFF => {
                self.write_mmio32(addr.phys, val);
                Ok(1)
            }
            0x1F80_2000..=0x1F80_2FFF | 0x1FA0_0000..=0x1FBF_FFFF | 0x1FC0_0000..=0x1FC7_FFFF => {
                Ok(1)
            }
            _ => Err(BusError::UnmappedAddress(vaddr)),
        }
    }

    // ========================================================================
    // Internal MMIO Dispatch (`0x1F80_1000..=0x1F80_1FFF`)
    // ========================================================================

    fn read_mmio8(&mut self, phys: u32) -> u8 {
        match phys {
            // SIO0 / SIO1
            0x1F80_1040 | 0x1F80_1050 => 0xFF, // No pad / card response
            0x1F80_1044..=0x1F80_105F => {
                let half = self.read_mmio16(phys & !1);
                ((half >> ((phys & 1) * 8)) & 0xFF) as u8
            }
            // CD-ROM banked ports
            0x1F80_1800 => self.cdrom.status_byte(),
            0x1F80_1801 => {
                if self.cdrom.resp_pos < self.cdrom.resp_len {
                    let b = self.cdrom.resp_fifo[self.cdrom.resp_pos as usize];
                    self.cdrom.resp_pos += 1;
                    b
                } else {
                    0
                }
            }
            0x1F80_1802 => 0,
            0x1F80_1803 => match self.cdrom.index & 1 {
                0 => self.cdrom.irq_enable | 0xE0,
                _ => self.cdrom.irq_flags | 0xE0,
            },
            _ => {
                let word = self.read_mmio32(phys & !3);
                ((word >> ((phys & 3) * 8)) & 0xFF) as u8
            }
        }
    }

    fn read_mmio16(&mut self, phys: u32) -> u16 {
        match phys {
            0x1F80_1040 | 0x1F80_1050 => 0xFFFF,
            0x1F80_1044 | 0x1F80_1054 => 0x0005, // TX_RDY1 | TX_RDY2
            0x1F80_1048..=0x1F80_105E => {
                let idx = ((phys - 0x1F80_1040) >> 1) as usize;
                self.sio_regs[idx]
            }
            0x1F80_1070 => self.i_stat,
            0x1F80_1074 => self.i_mask,
            // Root Counters (lazily advanced from master clock)
            0x1F80_1100..=0x1F80_112F => self.read_timer16(phys),
            // SPU registers (`0x1F80_1C00..=0x1F80_1FFF`)
            0x1F80_1C00..=0x1F80_1FFF => {
                let off = (phys - 0x1F80_1C00) as usize;
                u16::from_le_bytes([self.spu_mmio[off], self.spu_mmio[off + 1]])
            }
            _ => {
                let word = self.read_mmio32(phys & !3);
                ((word >> ((phys & 2) * 8)) & 0xFFFF) as u16
            }
        }
    }

    fn read_mmio32(&mut self, phys: u32) -> u32 {
        match phys {
            0x1F80_1000..=0x1F80_1020 => self.memctrl[((phys - 0x1F80_1000) >> 2) as usize],
            0x1F80_1060 => self.ram_size_reg,
            0x1F80_1070 => self.i_stat as u32,
            0x1F80_1074 => self.i_mask as u32,
            // DMA channels 0..6
            0x1F80_1080..=0x1F80_10EF => {
                let ch = ((phys - 0x1F80_1080) >> 4) as usize;
                match (phys >> 2) & 3 {
                    0 => self.dma_channels[ch].madr,
                    1 => self.dma_channels[ch].bcr,
                    2 => self.dma_channels[ch].chcr,
                    _ => self.last_open_bus,
                }
            }
            0x1F80_10F0 => self.dma_dpcr,
            0x1F80_10F4 => self.dma_dicr,
            0x1F80_1100..=0x1F80_112F => self.read_timer16(phys) as u32,
            0x1F80_1810 => self.gpu.read_gpuread(),
            0x1F80_1814 => self.gpu.read_gpustat(),
            0x1F80_1820 => 0,
            0x1F80_1824 => self.mdec_status,
            0x1F80_1C00..=0x1F80_1FFF => {
                let lo = self.read_mmio16(phys) as u32;
                let hi = self.read_mmio16(phys + 2) as u32;
                lo | (hi << 16)
            }
            _ => self.last_open_bus,
        }
    }

    fn write_mmio8(&mut self, phys: u32, val: u8) {
        match phys {
            0x1F80_1040..=0x1F80_105F => {
                if (phys & 1) == 0 {
                    self.write_mmio16(phys, val as u16);
                }
            }
            0x1F80_1800 => self.cdrom.index = val & 0x03,
            0x1F80_1801 => {
                if (self.cdrom.index & 3) == 0 && self.cdrom.execute_command(val) {
                    self.latch_irq(2);
                }
            }
            0x1F80_1802 => match self.cdrom.index & 3 {
                0 => {
                    if self.cdrom.param_len < 16 {
                        self.cdrom.param_fifo[self.cdrom.param_len as usize] = val;
                        self.cdrom.param_len += 1;
                    }
                }
                1 => self.cdrom.irq_enable = val & 0x1F,
                _ => {}
            },
            0x1F80_1803 => {
                if (self.cdrom.index & 3) == 1 {
                    // Write-1-to-acknowledge CD-ROM HINTSTS bits 0..4; bit 6 clears ParamFIFO
                    self.cdrom.irq_flags &= !(val & 0x1F);
                    if (val & 0x40) != 0 {
                        self.cdrom.param_len = 0;
                    }
                }
            }
            _ => {
                let aligned = phys & !3;
                let shift = (phys & 3) * 8;
                let prev = self.read_mmio32(aligned);
                let merged = (prev & !(0xFF << shift)) | ((val as u32) << shift);
                self.write_mmio32(aligned, merged);
            }
        }
    }

    fn write_mmio16(&mut self, phys: u32, val: u16) {
        match phys {
            0x1F80_1040..=0x1F80_105E => {
                let idx = ((phys - 0x1F80_1040) >> 1) as usize;
                self.sio_regs[idx] = val;
            }
            // I_STAT: write-0-to-acknowledge!
            0x1F80_1070 => self.i_stat &= val,
            0x1F80_1074 => self.i_mask = val & 0x07FF,
            0x1F80_1100..=0x1F80_112F => self.write_timer16(phys, val),
            // SPU registers
            0x1F80_1C00..=0x1F80_1FFF => {
                let off = (phys - 0x1F80_1C00) as usize;
                self.spu_mmio[off..off + 2].copy_from_slice(&val.to_le_bytes());
                // Mirror SPUCNT (0x1F80_1DAA) low 6 mode bits into SPUSTAT (0x1F80_1DAE) so BIOS boot doesn't hang!
                if phys == 0x1F80_1DAA {
                    let stat_off = (0x1F80_1DAE - 0x1F80_1C00) as usize;
                    let cur_stat =
                        u16::from_le_bytes([self.spu_mmio[stat_off], self.spu_mmio[stat_off + 1]]);
                    let next_stat = (cur_stat & !0x003F) | (val & 0x003F);
                    self.spu_mmio[stat_off..stat_off + 2].copy_from_slice(&next_stat.to_le_bytes());
                }
            }
            _ => {
                let aligned = phys & !3;
                let shift = (phys & 2) * 8;
                let prev = self.read_mmio32(aligned);
                let merged = (prev & !(0xFFFF << shift)) | ((val as u32) << shift);
                self.write_mmio32(aligned, merged);
            }
        }
    }

    fn write_mmio32(&mut self, phys: u32, val: u32) {
        match phys {
            0x1F80_1000..=0x1F80_1020 => {
                self.memctrl[((phys - 0x1F80_1000) >> 2) as usize] = val;
            }
            0x1F80_1060 => self.ram_size_reg = val,
            0x1F80_1070 => self.i_stat &= val as u16,
            0x1F80_1074 => self.i_mask = (val as u16) & 0x07FF,
            0x1F80_1080..=0x1F80_10EF => {
                let ch = ((phys - 0x1F80_1080) >> 4) as usize;
                match (phys >> 2) & 3 {
                    0 => self.dma_channels[ch].madr = val & 0x00FF_FFFF,
                    1 => self.dma_channels[ch].bcr = val,
                    2 => {
                        self.dma_channels[ch].chcr = val;
                        if (val & (1 << 24)) != 0 {
                            self.execute_dma_channel(ch);
                        }
                    }
                    _ => {}
                }
            }
            0x1F80_10F0 => self.dma_dpcr = val,
            0x1F80_10F4 => self.write_dma_dicr(val),
            0x1F80_1100..=0x1F80_112F => self.write_timer16(phys, val as u16),
            0x1F80_1810 => self.gpu.write_gp0(val),
            0x1F80_1814 => self.gpu.write_gp1(val),
            0x1F80_1820 => {}
            0x1F80_1824 => {
                if (val & 0x8000_0000) != 0 {
                    self.mdec_status = 0x8004_0000;
                }
            }
            0x1F80_1C00..=0x1F80_1FFF => {
                self.write_mmio16(phys, val as u16);
                self.write_mmio16(phys + 2, (val >> 16) as u16);
            }
            _ => {}
        }
    }

    // ========================================================================
    // Lazy Root Counter & Basic DMA Execution (OTC + GPU)
    // ========================================================================

    fn sync_timer(&mut self, idx: usize) {
        let elapsed = self
            .clock_cycle
            .saturating_sub(self.timers[idx].last_sync_cycle);
        let div = if idx == 2 && (self.timers[idx].mode & (1 << 9)) != 0 {
            8u64
        } else if idx == 1 && (self.timers[idx].mode & (1 << 8)) != 0 {
            2_148u64 // HBlank clock approximation
        } else {
            1u64
        };
        let ticks = elapsed / div;
        if ticks > 0 {
            self.timers[idx].last_sync_cycle += ticks * div;
            let cur = self.timers[idx].counter as u64 + ticks;
            if cur >= 0xFFFF {
                self.timers[idx].mode |= 1 << 12; // Reached 0xFFFF flag
            }
            self.timers[idx].counter = cur as u16;
        }
    }

    fn read_timer16(&mut self, phys: u32) -> u16 {
        let idx = ((phys - 0x1F80_1100) >> 4) as usize;
        if idx >= 3 {
            return 0;
        }
        self.sync_timer(idx);
        match (phys >> 2) & 3 {
            0 => self.timers[idx].counter,
            1 => {
                let m = self.timers[idx].mode;
                self.timers[idx].mode &= !(0x1800); // Bits 11-12 cleared on read
                m
            }
            2 => self.timers[idx].target,
            _ => 0,
        }
    }

    fn write_timer16(&mut self, phys: u32, val: u16) {
        let idx = ((phys - 0x1F80_1100) >> 4) as usize;
        if idx >= 3 {
            return;
        }
        self.sync_timer(idx);
        match (phys >> 2) & 3 {
            0 => self.timers[idx].counter = val,
            1 => {
                self.timers[idx].mode = (val & 0x03FF) | (1 << 10);
                self.timers[idx].counter = 0;
            }
            2 => self.timers[idx].target = val,
            _ => {}
        }
    }

    fn write_dma_dicr(&mut self, val: u32) {
        let prev_master = (self.dma_dicr >> 31) & 1;
        // Bits 24..30 are write-1-to-acknowledge!
        let ack_mask = val & 0x7F00_0000;
        let flags = (self.dma_dicr & 0x7F00_0000) & !ack_mask;
        let cfg = val & 0x00FF_803F;
        self.dma_dicr = cfg | flags;
        self.refresh_dma_master_irq(prev_master);
    }

    fn refresh_dma_master_irq(&mut self, prev_master: u32) {
        let force = (self.dma_dicr & (1 << 15)) != 0;
        let master_en = (self.dma_dicr & (1 << 23)) != 0;
        let enabled_flags = ((self.dma_dicr >> 24) & (self.dma_dicr >> 16)) & 0x7F;
        let next_master = u32::from(force || (master_en && enabled_flags != 0));
        self.dma_dicr = (self.dma_dicr & !(1 << 31)) | (next_master << 31);
        if prev_master == 0 && next_master == 1 {
            self.latch_irq(3);
        }
    }

    fn execute_dma_channel(&mut self, ch: usize) {
        let enabled = ((self.dma_dpcr >> (ch * 4 + 3)) & 1) != 0;
        if !enabled {
            return;
        }

        match ch {
            // DMA2: GPU (Block & Linked-List Ordering Table)
            2 => {
                let sync_mode = (self.dma_channels[2].chcr >> 9) & 3;
                let from_ram = (self.dma_channels[2].chcr & 1) != 0;
                if sync_mode == 2 && from_ram {
                    let mut addr = self.dma_channels[2].madr & 0x001F_FFFC;
                    let mut nodes = 0usize;
                    while nodes < MAX_DMA_LINKED_LIST_NODES {
                        nodes += 1;
                        let off = addr as usize;
                        let header = u32::from_le_bytes([
                            self.ram[off],
                            self.ram[off + 1],
                            self.ram[off + 2],
                            self.ram[off + 3],
                        ]);
                        let count = header >> 24;
                        for i in 1..=count {
                            let w_off = ((addr.wrapping_add(i * 4)) & 0x001F_FFFC) as usize;
                            let word = u32::from_le_bytes([
                                self.ram[w_off],
                                self.ram[w_off + 1],
                                self.ram[w_off + 2],
                                self.ram[w_off + 3],
                            ]);
                            self.gpu.write_gp0(word);
                        }
                        let next = header & 0x00FF_FFFF;
                        if (next & 0x0080_0000) != 0 {
                            break;
                        }
                        addr = next & 0x001F_FFFC;
                    }
                }
            }
            // DMA6: OTC (Reverse RAM Ordering Table Clear)
            6 => {
                let mut count = self.dma_channels[6].bcr & 0xFFFF;
                if count == 0 {
                    count = 0x10000;
                }
                count = count.min(MAX_DMA_OTC_WORDS);
                let mut addr = self.dma_channels[6].madr & 0x001F_FFFC;
                for rem in (1..=count).rev() {
                    let val = if rem == 1 {
                        0x00FF_FFFF
                    } else {
                        addr.wrapping_sub(4) & 0x001F_FFFF
                    };
                    let off = addr as usize;
                    self.ram[off..off + 4].copy_from_slice(&val.to_le_bytes());
                    addr = addr.wrapping_sub(4) & 0x001F_FFFC;
                }
            }
            _ => {}
        }

        // Clear start/busy/trigger bits (bits 24 & 28) and raise channel completion flag
        self.dma_channels[ch].chcr &= !((1 << 24) | (1 << 28));
        let prev_master = (self.dma_dicr >> 31) & 1;
        if (self.dma_dicr & (1 << (16 + ch))) != 0 {
            self.dma_dicr |= 1 << (24 + ch);
        }
        self.refresh_dma_master_irq(prev_master);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_mirrors_four_times_across_8mb_and_across_segments() {
        let mut bus = Bus::new(NullGpu::new());
        bus.write32(0x0000_1234, 0xDEAD_BEEF).unwrap();

        // Physical 2MB mirrors (+2MB, +4MB, +6MB) across KUSEG, KSEG0, and KSEG1
        assert_eq!(bus.read32(0x0020_1234).unwrap().0, 0xDEAD_BEEF);
        assert_eq!(bus.read32(0x8040_1234).unwrap().0, 0xDEAD_BEEF);
        assert_eq!(bus.read32(0xA060_1234).unwrap().0, 0xDEAD_BEEF);

        // Above 8MB window triggers UnmappedAddress bus error
        assert_eq!(
            bus.read32(0x0080_0000),
            Err(BusError::UnmappedAddress(0x0080_0000))
        );
    }

    #[test]
    fn scratchpad_works_in_kuseg_kseg0_and_rejects_kseg1() {
        let mut bus = Bus::new(NullGpu::new());
        assert_eq!(bus.write32(0x1F80_0010, 0xCAFE_BABE), Ok(1));
        assert_eq!(bus.read32(0x9F80_0010), Ok((0xCAFE_BABE, 1)));

        // KSEG1 (0xBF80_0010) MUST reject Scratchpad reads and writes!
        assert_eq!(
            bus.read32(0xBF80_0010),
            Err(BusError::Kseg1ScratchpadAccess(0xBF80_0010))
        );
        assert_eq!(
            bus.write32(0xBF80_0010, 0x1111_2222),
            Err(BusError::Kseg1ScratchpadAccess(0xBF80_0010))
        );
        assert_eq!(bus.read32(0x1F80_0010), Ok((0xCAFE_BABE, 1)));
    }

    #[test]
    fn biu_config_only_valid_in_kseg2_and_irq_uses_write_zero_to_ack() {
        let mut bus = Bus::new(NullGpu::new());
        bus.write32(0xFFFE_0130, 0x0001_E988).unwrap();
        assert_eq!(bus.read32(0xFFFE_0130), Ok((0x0001_E988, 1)));
        assert_eq!(
            bus.read32(0xFFFE_0134),
            Err(BusError::UnmappedAddress(0xFFFE_0134))
        );

        bus.latch_irq(0); // VBlank
        bus.latch_irq(3); // DMA
        bus.write16(0x1F80_1074, 0x0009).unwrap();
        assert!(bus.irq_pending());
        assert_eq!(bus.i_stat(), 0x0009);

        // Writing 0 to bit 0 clears IRQ0 while leaving bit 3 set!
        bus.write16(0x1F80_1070, !0x0001).unwrap();
        assert_eq!(bus.i_stat(), 0x0008);
        assert!(bus.irq_pending());

        bus.write16(0x1F80_1070, !0x0008).unwrap();
        assert_eq!(bus.i_stat(), 0x0000);
        assert!(!bus.irq_pending());
    }

    #[test]
    fn dma6_otc_clears_ordering_table_backwards() {
        let mut bus = Bus::new(NullGpu::new());
        // With reset DPCR (0x0765_4321), DMA6 master enable (bit 27) is off; trigger should be ignored
        bus.write32(0x1F80_10E0, 0x0000_100C).unwrap();
        bus.write32(0x1F80_10E4, 4).unwrap();
        bus.write32(0x1F80_10E8, 0x1100_0002).unwrap();
        assert_eq!(bus.read32(0x0000_100C).unwrap().0, 0);

        // Enable DMA6 in DPCR (bit 27) and trigger OTC
        bus.write32(0x1F80_10F0, 0x0800_0000).unwrap();
        bus.write32(0x1F80_10E8, 0x1100_0002).unwrap();

        assert_eq!(bus.read32(0x0000_100C).unwrap().0, 0x0000_1008);
        assert_eq!(bus.read32(0x0000_1008).unwrap().0, 0x0000_1004);
        assert_eq!(bus.read32(0x0000_1004).unwrap().0, 0x0000_1000);
        assert_eq!(bus.read32(0x0000_1000).unwrap().0, 0x00FF_FFFF);
    }
}
