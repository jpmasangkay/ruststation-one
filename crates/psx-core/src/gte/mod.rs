pub mod unr;

pub use unr::{UNR_TABLE, unr_divide};

use serde::{Deserialize, Serialize};

/// Bitmask of `FLAG` (`r63`) bits that set the summary Error Flag (`bit 31`):
/// bits `30..=23` (`0x7F80_0000`) and bits `18..=13` (`0x0007_E000`).
pub const FLAG_ERROR_SUMMARY_MASK: u32 = 0x7F87_E000;

/// Sony PlayStation Geometry Transformation Engine (`COP2`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gte {
    /// Raw backing storage for `COP2D r0..r31`.
    pub data_regs: [u32; 32],
    /// Raw backing storage for `COP2C r0..r31` (`r32..r63`).
    pub ctrl_regs: [u32; 32],
}

impl Default for Gte {
    fn default() -> Self {
        Self::new()
    }
}

impl Gte {
    #[must_use]
    pub const fn new() -> Self {
        let mut s = Self {
            data_regs: [0; 32],
            ctrl_regs: [0; 32],
        };
        s.data_regs[31] = 32; // LZCR for initial LZCS = 0
        s
    }

    // ========================================================================
    // Register Interface (`MFC2` / `MTC2` / `CFC2` / `CTC2` / `LWC2` / `SWC2`)
    // ========================================================================

    #[inline]
    #[must_use]
    pub fn read_data(&self, reg: u8) -> u32 {
        let r = (reg & 0x1F) as usize;
        match r {
            // Sign-extended 16-bit registers: VZ0, VZ1, VZ2, IR0, IR1, IR2, IR3
            1 | 3 | 5 | 8 | 9 | 10 | 11 => (self.data_regs[r] as i16) as i32 as u32,
            // Zero-extended 16-bit registers: OTZ, SZ0, SZ1, SZ2, SZ3
            7 | 16 | 17 | 18 | 19 => self.data_regs[r] & 0xFFFF,
            // SXYP (r15) mirrors SXY2 (r14) on read
            15 => self.data_regs[14],
            // IRGB (r28) and ORGB (r29) pack saturated (IR1..3 >> 7) into 15-bit RGB555
            28 | 29 => {
                let r5 = ((self.ir(1) >> 7).clamp(0, 0x1F)) as u32;
                let g5 = ((self.ir(2) >> 7).clamp(0, 0x1F)) as u32;
                let b5 = ((self.ir(3) >> 7).clamp(0, 0x1F)) as u32;
                r5 | (g5 << 5) | (b5 << 10)
            }
            _ => self.data_regs[r],
        }
    }

    #[inline]
    pub fn write_data(&mut self, reg: u8, val: u32) {
        let r = (reg & 0x1F) as usize;
        match r {
            1 | 3 | 5 | 8 | 9 | 10 | 11 => {
                self.data_regs[r] = (val as i16) as i32 as u32;
            }
            7 | 16 | 17 | 18 | 19 => {
                self.data_regs[r] = val & 0xFFFF;
            }
            // Writing SXYP (r15) pushes the SXY0 -> SXY1 -> SXY2 FIFO!
            15 => {
                self.data_regs[12] = self.data_regs[13];
                self.data_regs[13] = self.data_regs[14];
                self.data_regs[14] = val;
            }
            // Writing IRGB (r28) expands 5-bit unsigned channels * 0x80 into IR1..IR3
            28 => {
                self.data_regs[28] = val & 0x7FFF;
                self.data_regs[9] = ((val & 0x1F) * 0x80) as i16 as i32 as u32;
                self.data_regs[10] = (((val >> 5) & 0x1F) * 0x80) as i16 as i32 as u32;
                self.data_regs[11] = (((val >> 10) & 0x1F) * 0x80) as i16 as i32 as u32;
            }
            // ORGB (r29) and LZCR (r31) are strictly read-only!
            29 | 31 => {}
            // Writing LZCS (r30) computes leading zeros (if >= 0) or leading ones (if < 0) into LZCR (r31)
            30 => {
                self.data_regs[30] = val;
                let count = if (val as i32) < 0 {
                    (!val).leading_zeros()
                } else {
                    val.leading_zeros()
                };
                self.data_regs[31] = count;
            }
            _ => self.data_regs[r] = val,
        }
    }

    #[inline]
    #[must_use]
    pub fn read_ctrl(&self, reg: u8) -> u32 {
        let r = (reg & 0x1F) as usize;
        match r {
            // RT33 (4), L33 (12), LB3 (20), H (26!), DQA (27), ZSF3 (29), ZSF4 (30) sign-extend bit 15 on CFC2!
            4 | 12 | 20 | 26 | 27 | 29 | 30 => (self.ctrl_regs[r] as i16) as i32 as u32,
            _ => self.ctrl_regs[r],
        }
    }

    #[inline]
    pub fn write_ctrl(&mut self, reg: u8, val: u32) {
        let r = (reg & 0x1F) as usize;
        match r {
            4 | 12 | 20 | 27 | 29 | 30 => {
                self.ctrl_regs[r] = (val as i16) as i32 as u32;
            }
            // H (r26 = cop2r58) is unsigned 16-bit internally, though sign-extended on CFC2 read!
            26 => {
                self.ctrl_regs[26] = val & 0xFFFF;
            }
            // FLAG (r31 = cop2r63): low 12 bits forced 0; bit 31 recomputed from error mask!
            31 => {
                let mut f = val & 0x7FFF_F000;
                if (f & FLAG_ERROR_SUMMARY_MASK) != 0 {
                    f |= 0x8000_0000;
                }
                self.ctrl_regs[31] = f;
            }
            _ => self.ctrl_regs[r] = val,
        }
    }

    // ========================================================================
    // Typed Internal Accessors & Saturation Helpers
    // ========================================================================

    #[inline(always)]
    const fn ir(&self, idx: usize) -> i32 {
        (self.data_regs[8 + idx] as i16) as i32
    }

    #[inline(always)]
    fn set_mac0(&mut self, val: i64) -> i64 {
        if val > 0x7FFF_FFFF {
            self.set_flag_bit(16);
        } else if val < -0x8000_0000 {
            self.set_flag_bit(15);
        }
        self.data_regs[24] = (val as i32) as u32;
        val
    }

    /// Checks 44-bit signed accumulator bounds on `idx` (`1..=3`), sets `FLAG` bits `30..25`,
    /// and wraps to 44-bit signed precision (`(val << 20) >> 20`) exactly like GTE silicon!
    #[inline(always)]
    fn check_mac44(&mut self, idx: usize, val: i64) -> i64 {
        debug_assert!((1..=3).contains(&idx));
        const MAX_44: i64 = (1i64 << 43) - 1;
        const MIN_44: i64 = -(1i64 << 43);
        if val > MAX_44 {
            self.set_flag_bit((31 - idx) as u8); // 30, 29, 28
        } else if val < MIN_44 {
            self.set_flag_bit((28 - idx) as u8); // 27, 26, 25
        }
        (val << 20) >> 20
    }

    #[inline(always)]
    fn finish_mac(&mut self, idx: usize, acc44: i64, sf_shift: u8) -> i32 {
        let shifted = (acc44 >> sf_shift) as i32;
        self.data_regs[24 + idx] = shifted as u32;
        shifted
    }

    #[inline(always)]
    fn set_ir(&mut self, idx: usize, val: i32, lm: bool) {
        debug_assert!((1..=3).contains(&idx));
        let min_bound = if lm { 0 } else { -0x8000 };
        let clamped = if val < min_bound {
            self.set_flag_bit((25 - idx) as u8); // 24, 23, 22
            min_bound
        } else if val > 0x7FFF {
            self.set_flag_bit((25 - idx) as u8);
            0x7FFF
        } else {
            val
        };
        self.data_regs[8 + idx] = (clamped as i16) as i32 as u32;
    }

    #[inline(always)]
    fn set_ir0(&mut self, val: i64) {
        let clamped = if val < 0 {
            self.set_flag_bit(12);
            0i16
        } else if val > 0x1000 {
            self.set_flag_bit(12);
            0x1000i16
        } else {
            val as i16
        };
        self.data_regs[8] = (clamped as i32) as u32;
    }

    #[inline(always)]
    fn push_sz(&mut self, val: i32) -> u16 {
        let clamped = if val < 0 {
            self.set_flag_bit(18);
            0u16
        } else if val > 0xFFFF {
            self.set_flag_bit(18);
            0xFFFFu16
        } else {
            val as u16
        };
        self.data_regs[16] = self.data_regs[17];
        self.data_regs[17] = self.data_regs[18];
        self.data_regs[18] = self.data_regs[19];
        self.data_regs[19] = clamped as u32;
        clamped
    }

    #[inline(always)]
    fn set_otz(&mut self, val: i64) {
        let clamped = if val < 0 {
            self.set_flag_bit(18);
            0u32
        } else if val > 0xFFFF {
            self.set_flag_bit(18);
            0xFFFFu32
        } else {
            val as u32
        };
        self.data_regs[7] = clamped;
    }

    #[inline(always)]
    fn push_sxy(&mut self, sx_raw: i64, sy_raw: i64) {
        let sx = if sx_raw < -0x400 {
            self.set_flag_bit(14);
            -0x400i16
        } else if sx_raw > 0x3FF {
            self.set_flag_bit(14);
            0x3FFi16
        } else {
            sx_raw as i16
        };

        let sy = if sy_raw < -0x400 {
            self.set_flag_bit(13);
            -0x400i16
        } else if sy_raw > 0x3FF {
            self.set_flag_bit(13);
            0x3FFi16
        } else {
            sy_raw as i16
        };

        let packed = ((sx as u16) as u32) | (((sy as u16) as u32) << 16);
        self.data_regs[12] = self.data_regs[13];
        self.data_regs[13] = self.data_regs[14];
        self.data_regs[14] = packed;
    }

    #[inline(always)]
    fn push_rgb_from_mac(&mut self) {
        let m1 = (self.data_regs[25] as i32) >> 4;
        let m2 = (self.data_regs[26] as i32) >> 4;
        let m3 = (self.data_regs[27] as i32) >> 4;
        let r = self.clamp_color(m1, 21);
        let g = self.clamp_color(m2, 20);
        let b = self.clamp_color(m3, 19);
        let code = self.data_regs[6] & 0xFF00_0000;

        self.data_regs[20] = self.data_regs[21];
        self.data_regs[21] = self.data_regs[22];
        self.data_regs[22] = (r as u32) | ((g as u32) << 8) | ((b as u32) << 16) | code;
    }

    #[inline(always)]
    fn clamp_color(&mut self, val: i32, flag_bit: u8) -> u8 {
        if val < 0 {
            self.set_flag_bit(flag_bit);
            0
        } else if val > 0xFF {
            self.set_flag_bit(flag_bit);
            0xFF
        } else {
            val as u8
        }
    }

    #[inline(always)]
    fn set_flag_bit(&mut self, bit: u8) {
        self.ctrl_regs[31] |= 1u32 << bit;
    }

    #[inline(always)]
    fn vec_v(&self, v_idx: usize) -> [i64; 3] {
        match v_idx {
            0..=2 => {
                let xy = self.data_regs[v_idx * 2];
                let z = self.data_regs[v_idx * 2 + 1];
                [
                    (xy as i16) as i64,
                    ((xy >> 16) as i16) as i64,
                    (z as i16) as i64,
                ]
            }
            _ => [self.ir(1) as i64, self.ir(2) as i64, self.ir(3) as i64],
        }
    }

    #[inline]
    fn matrix(&self, base_ctrl: usize) -> [[i64; 3]; 3] {
        let c0 = self.ctrl_regs[base_ctrl];
        let c1 = self.ctrl_regs[base_ctrl + 1];
        let c2 = self.ctrl_regs[base_ctrl + 2];
        let c3 = self.ctrl_regs[base_ctrl + 3];
        let c4 = self.ctrl_regs[base_ctrl + 4];
        [
            [
                (c0 as i16) as i64,
                ((c0 >> 16) as i16) as i64,
                (c1 as i16) as i64,
            ],
            [
                ((c1 >> 16) as i16) as i64,
                (c2 as i16) as i64,
                ((c2 >> 16) as i16) as i64,
            ],
            [
                (c3 as i16) as i64,
                ((c3 >> 16) as i16) as i64,
                (c4 as i16) as i64,
            ],
        ]
    }

    #[inline]
    fn ctrl_vec3(&self, base_ctrl: usize) -> [i64; 3] {
        [
            (self.ctrl_regs[base_ctrl] as i32) as i64,
            (self.ctrl_regs[base_ctrl + 1] as i32) as i64,
            (self.ctrl_regs[base_ctrl + 2] as i32) as i64,
        ]
    }

    #[inline]
    fn rgbc_bytes(&self) -> [i64; 3] {
        let c = self.data_regs[6];
        [
            (c & 0xFF) as i64,
            ((c >> 8) & 0xFF) as i64,
            ((c >> 16) & 0xFF) as i64,
        ]
    }

    #[inline]
    fn rgb0_bytes(&self) -> [i64; 3] {
        let c = self.data_regs[20];
        [
            (c & 0xFF) as i64,
            ((c >> 8) & 0xFF) as i64,
            ((c >> 16) & 0xFF) as i64,
        ]
    }

    // ========================================================================
    // Main GTE Opcode Execution Entrypoint (`COP2 imm25`)
    // ========================================================================

    #[inline(never)]
    pub fn execute(&mut self, instr: u32) -> u64 {
        // Clear FLAG at the beginning of every GTE command
        self.ctrl_regs[31] = 0;

        let sf_shift: u8 = if (instr & (1 << 19)) != 0 { 12 } else { 0 };
        let lm: bool = (instr & (1 << 10)) != 0;
        let real_op = instr & 0x3F;

        let cycles = match real_op {
            0x01 => {
                self.exec_rtps(0, sf_shift, lm, true);
                15
            }
            0x06 => {
                self.exec_nclip();
                8
            }
            0x0C => {
                self.exec_op(sf_shift, lm);
                6
            }
            0x10 => {
                let rgb = self.rgbc_bytes();
                self.exec_dpcs_single(rgb, sf_shift, lm);
                8
            }
            0x11 => {
                self.exec_intpl(sf_shift, lm);
                8
            }
            0x12 => {
                self.exec_mvmva(instr, sf_shift, lm);
                8
            }
            0x13 => {
                self.exec_ncds_single(0, sf_shift, lm);
                19
            }
            0x14 => {
                self.exec_cdp(sf_shift, lm);
                13
            }
            0x16 => {
                for v in 0..3 {
                    self.exec_ncds_single(v, sf_shift, lm);
                }
                44
            }
            0x1B => {
                self.exec_nccs_single(0, sf_shift, lm);
                17
            }
            0x1C => {
                self.exec_cc(sf_shift, lm);
                11
            }
            0x1E => {
                self.exec_ncs_single(0, sf_shift, lm);
                14
            }
            0x20 => {
                for v in 0..3 {
                    self.exec_ncs_single(v, sf_shift, lm);
                }
                30
            }
            0x28 => {
                self.exec_sqr(sf_shift, lm);
                5
            }
            0x29 => {
                // DCPL ignores `sf`? Wait: DCPL uses `sf_shift` on PS1 GTE!
                self.exec_dcpl(sf_shift, lm);
                8
            }
            0x2A => {
                // DPCT runs 3 iterations, each reading RGB0 (`r20`) BEFORE pushing RGB FIFO!
                for _ in 0..3 {
                    let rgb = self.rgb0_bytes();
                    self.exec_dpcs_single(rgb, sf_shift, lm);
                }
                17
            }
            0x2D => {
                self.exec_avsz3();
                5
            }
            0x2E => {
                self.exec_avsz4();
                6
            }
            0x30 => {
                self.exec_rtps(0, sf_shift, lm, false);
                self.exec_rtps(1, sf_shift, lm, false);
                self.exec_rtps(2, sf_shift, lm, true);
                23
            }
            0x3D => {
                self.exec_gpf(sf_shift, lm);
                5
            }
            0x3E => {
                self.exec_gpl(sf_shift, lm);
                5
            }
            0x3F => {
                for v in 0..3 {
                    self.exec_nccs_single(v, sf_shift, lm);
                }
                39
            }
            _ => 5,
        };

        // Finalize FLAG bit 31 Error Summary: OR of bits 30..23 and 18..13
        if (self.ctrl_regs[31] & FLAG_ERROR_SUMMARY_MASK) != 0 {
            self.ctrl_regs[31] |= 0x8000_0000;
        }
        cycles
    }

    // ========================================================================
    // Individual GTE Command Implementations
    // ========================================================================

    fn exec_rtps(&mut self, v_idx: usize, sf_shift: u8, lm: bool, calc_depth_cue: bool) {
        let rt = self.matrix(0);
        let tr = self.ctrl_vec3(5);
        let v = self.vec_v(v_idx);

        let mut mac_z44 = 0i64;
        for i in 0..3 {
            let step1 = self.check_mac44(i + 1, (tr[i] << 12) + rt[i][0] * v[0]);
            let step2 = self.check_mac44(i + 1, step1 + rt[i][1] * v[1]);
            let step3 = self.check_mac44(i + 1, step2 + rt[i][2] * v[2]);
            let mac = self.finish_mac(i + 1, step3, sf_shift);

            if i < 2 {
                self.set_ir(i + 1, mac, lm);
            } else {
                mac_z44 = step3;
                // Hardware RTPS/RTPT IR3 quirk:
                // 1) FLAG bit 22 is checked against `(mac_z44 >> 12)` with `lm = false` (-0x8000..+0x7FFF)!
                let ir3_flag_test = (mac_z44 >> 12) as i32;
                if !(-0x8000..=0x7FFF).contains(&ir3_flag_test) {
                    self.set_flag_bit(22);
                }
                // 2) Actual IR3 value clamps `mac` (`mac_z44 >> sf_shift`) using `lm`, WITHOUT touching bit 22!
                let min_ir3 = if lm { 0 } else { -0x8000 };
                let clamped_ir3 = mac.clamp(min_ir3, 0x7FFF);
                self.data_regs[11] = (clamped_ir3 as i16) as i32 as u32;
            }
        }

        // Push SZ3 clamped from `(mac_z44 >> 12)` (ALWAYS shifted by 12 regardless of `sf`!)
        let sz3 = self.push_sz((mac_z44 >> 12) as i32);

        // Perspective projection via exact Sony UNR division
        let h = (self.ctrl_regs[26] & 0xFFFF) as u16;
        let (quotient, div_ovf) = unr_divide(h, sz3);
        if div_ovf {
            self.set_flag_bit(17);
        }

        let q = quotient as i64;
        let ofx = (self.ctrl_regs[24] as i32) as i64;
        let ofy = (self.ctrl_regs[25] as i32) as i64;

        let sx_mac0 = self.set_mac0(q * (self.ir(1) as i64) + ofx);
        let sy_mac0 = self.set_mac0(q * (self.ir(2) as i64) + ofy);
        self.push_sxy(sx_mac0 >> 16, sy_mac0 >> 16);

        if calc_depth_cue {
            let dqa = ((self.ctrl_regs[27] as i16) as i32) as i64;
            let dqb = (self.ctrl_regs[28] as i32) as i64;
            let dq_mac0 = self.set_mac0(dqb + dqa * q);
            self.set_ir0(dq_mac0 >> 12);
        }
    }

    fn exec_nclip(&mut self) {
        let sx0 = (self.data_regs[12] as i16) as i64;
        let sy0 = ((self.data_regs[12] >> 16) as i16) as i64;
        let sx1 = (self.data_regs[13] as i16) as i64;
        let sy1 = ((self.data_regs[13] >> 16) as i16) as i64;
        let sx2 = (self.data_regs[14] as i16) as i64;
        let sy2 = ((self.data_regs[14] >> 16) as i16) as i64;

        let det = sx0 * (sy1 - sy2) + sx1 * (sy2 - sy0) + sx2 * (sy0 - sy1);
        self.set_mac0(det);
    }

    fn exec_op(&mut self, sf_shift: u8, lm: bool) {
        let rt = self.matrix(0);
        let d1 = rt[0][0]; // RT11
        let d2 = rt[1][1]; // RT22
        let d3 = rt[2][2]; // RT33
        let ir1 = self.ir(1) as i64;
        let ir2 = self.ir(2) as i64;
        let ir3 = self.ir(3) as i64;

        let a1 = self.check_mac44(1, ir3 * d2 - ir2 * d3);
        let a2 = self.check_mac44(2, ir1 * d3 - ir3 * d1);
        let a3 = self.check_mac44(3, ir2 * d1 - ir1 * d2);

        let m1 = self.finish_mac(1, a1, sf_shift);
        let m2 = self.finish_mac(2, a2, sf_shift);
        let m3 = self.finish_mac(3, a3, sf_shift);

        self.set_ir(1, m1, lm);
        self.set_ir(2, m2, lm);
        self.set_ir(3, m3, lm);
    }

    fn exec_mvmva(&mut self, instr: u32, sf_shift: u8, lm: bool) {
        let mx_sel = ((instr >> 17) & 3) as usize;
        let v_sel = ((instr >> 15) & 3) as usize;
        let cv_sel = ((instr >> 13) & 3) as usize;

        let v = self.vec_v(v_sel);
        let m = match mx_sel {
            0 => self.matrix(0),  // RT
            1 => self.matrix(8),  // LLM
            2 => self.matrix(16), // LCM
            // Hardware `mx == 3` garbage matrix quirk!
            _ => {
                let r = (self.data_regs[6] & 0xFF) as i64;
                let rt = self.matrix(0);
                let ir0 = self.ir(0) as i64;
                [
                    [-(r << 4), r << 4, ir0],
                    [rt[0][2], rt[0][2], rt[0][2]],
                    [rt[1][1], rt[1][1], rt[1][1]],
                ]
            }
        };

        let cv = match cv_sel {
            0 => self.ctrl_vec3(5),  // TR
            1 => self.ctrl_vec3(13), // BK
            2 => self.ctrl_vec3(21), // FC (triggers hardware split-step bug below!)
            _ => [0, 0, 0],
        };

        for i in 0..3 {
            if cv_sel == 2 {
                // Hardware `cv == 2` (Far Color) bug:
                // Step 1 computes `(FC_i << 12) + M_i0 * V_0`, checks overflow & saturates IR flag
                // with `lm = false`, and then DROPS Step 1 from the final accumulator!
                let bug_step1 = self.check_mac44(i + 1, (cv[i] << 12) + m[i][0] * v[0]);
                let bug_shifted = (bug_step1 >> sf_shift) as i32;
                self.set_ir(i + 1, bug_shifted, false);

                // Step 2 computes ONLY `M_i1 * V_1 + M_i2 * V_2` and saturates with instruction `lm`!
                let step2 = self.check_mac44(i + 1, m[i][1] * v[1]);
                let step3 = self.check_mac44(i + 1, step2 + m[i][2] * v[2]);
                let mac = self.finish_mac(i + 1, step3, sf_shift);
                self.set_ir(i + 1, mac, lm);
            } else {
                let step1 = self.check_mac44(i + 1, (cv[i] << 12) + m[i][0] * v[0]);
                let step2 = self.check_mac44(i + 1, step1 + m[i][1] * v[1]);
                let step3 = self.check_mac44(i + 1, step2 + m[i][2] * v[2]);
                let mac = self.finish_mac(i + 1, step3, sf_shift);
                self.set_ir(i + 1, mac, lm);
            }
        }
    }

    /// Multiply `matrix` by `vec` + `(trans << 12)` cleanly (used by NCS/NCT/NCDS/NCDT/NCCS/NCCT/CC/CDP).
    fn mul_mat_vec(
        &mut self,
        mat: [[i64; 3]; 3],
        trans: [i64; 3],
        v: [i64; 3],
        sf_shift: u8,
        lm: bool,
    ) {
        for i in 0..3 {
            let s1 = self.check_mac44(i + 1, (trans[i] << 12) + mat[i][0] * v[0]);
            let s2 = self.check_mac44(i + 1, s1 + mat[i][1] * v[1]);
            let s3 = self.check_mac44(i + 1, s2 + mat[i][2] * v[2]);
            let mac = self.finish_mac(i + 1, s3, sf_shift);
            self.set_ir(i + 1, mac, lm);
        }
    }

    /// Interpolates `base44` (`[i64; 3]`, already scaled at 12-bit fractional scale) toward `FC << 12` by `IR0`.
    fn interpolate_far_color(&mut self, base: [i64; 3], sf_shift: u8, lm: bool) {
        let fc = self.ctrl_vec3(21);
        let ir0 = self.ir(0) as i64;

        // First half: `((FC << 12) - base) >> sf`, saturated into IR1..3 with `lm = false`!
        for (i, (&fc_comp, &base_comp)) in fc.iter().zip(base.iter()).enumerate() {
            let diff = self.check_mac44(i + 1, (fc_comp << 12) - base_comp);
            let m = self.finish_mac(i + 1, diff, sf_shift);
            self.set_ir(i + 1, m, false);
        }

        // Second half: `(base + IR1..3 * IR0) >> sf`, saturated into IR1..3 with `lm`!
        for (i, &base_comp) in base.iter().enumerate() {
            let interp = self.check_mac44(i + 1, base_comp + (self.ir(i + 1) as i64) * ir0);
            let m = self.finish_mac(i + 1, interp, sf_shift);
            self.set_ir(i + 1, m, lm);
        }
    }

    fn exec_dpcs_single(&mut self, rgb: [i64; 3], sf_shift: u8, lm: bool) {
        let base = [rgb[0] << 16, rgb[1] << 16, rgb[2] << 16];
        self.interpolate_far_color(base, sf_shift, lm);
        self.push_rgb_from_mac();
    }

    fn exec_intpl(&mut self, sf_shift: u8, lm: bool) {
        let base = [
            (self.ir(1) as i64) << 12,
            (self.ir(2) as i64) << 12,
            (self.ir(3) as i64) << 12,
        ];
        self.interpolate_far_color(base, sf_shift, lm);
        self.push_rgb_from_mac();
    }

    fn exec_dcpl(&mut self, sf_shift: u8, lm: bool) {
        let rgb = self.rgbc_bytes();
        let base = [
            (rgb[0] << 4) * (self.ir(1) as i64),
            (rgb[1] << 4) * (self.ir(2) as i64),
            (rgb[2] << 4) * (self.ir(3) as i64),
        ];
        self.interpolate_far_color(base, sf_shift, lm);
        self.push_rgb_from_mac();
    }

    fn exec_ncs_single(&mut self, v_idx: usize, sf_shift: u8, lm: bool) {
        let llm = self.matrix(8);
        let v = self.vec_v(v_idx);
        self.mul_mat_vec(llm, [0, 0, 0], v, sf_shift, lm);

        let lcm = self.matrix(16);
        let bk = self.ctrl_vec3(13);
        let ir_vec = self.vec_v(3);
        self.mul_mat_vec(lcm, bk, ir_vec, sf_shift, lm);
        self.push_rgb_from_mac();
    }

    fn exec_nccs_single(&mut self, v_idx: usize, sf_shift: u8, lm: bool) {
        let llm = self.matrix(8);
        let v = self.vec_v(v_idx);
        self.mul_mat_vec(llm, [0, 0, 0], v, sf_shift, lm);
        self.exec_cc(sf_shift, lm);
    }

    fn exec_cc(&mut self, sf_shift: u8, lm: bool) {
        let lcm = self.matrix(16);
        let bk = self.ctrl_vec3(13);
        let ir_vec = self.vec_v(3);
        self.mul_mat_vec(lcm, bk, ir_vec, sf_shift, lm);

        let rgb = self.rgbc_bytes();
        for (i, &c) in rgb.iter().enumerate() {
            let prod = self.check_mac44(i + 1, (c << 4) * (self.ir(i + 1) as i64));
            let m = self.finish_mac(i + 1, prod, sf_shift);
            self.set_ir(i + 1, m, lm);
        }
        self.push_rgb_from_mac();
    }

    fn exec_ncds_single(&mut self, v_idx: usize, sf_shift: u8, lm: bool) {
        let llm = self.matrix(8);
        let v = self.vec_v(v_idx);
        self.mul_mat_vec(llm, [0, 0, 0], v, sf_shift, lm);
        self.exec_cdp(sf_shift, lm);
    }

    fn exec_cdp(&mut self, sf_shift: u8, lm: bool) {
        let lcm = self.matrix(16);
        let bk = self.ctrl_vec3(13);
        let ir_vec = self.vec_v(3);
        self.mul_mat_vec(lcm, bk, ir_vec, sf_shift, lm);
        self.exec_dcpl(sf_shift, lm);
    }

    fn exec_sqr(&mut self, sf_shift: u8, lm: bool) {
        for i in 1..=3 {
            let v = self.ir(i) as i64;
            let sq = self.check_mac44(i, v * v);
            let m = self.finish_mac(i, sq, sf_shift);
            self.set_ir(i, m, lm);
        }
    }

    fn exec_avsz3(&mut self) {
        let zsf3 = ((self.ctrl_regs[29] as i16) as i32) as i64;
        let sum = (self.data_regs[17] & 0xFFFF) as i64
            + (self.data_regs[18] & 0xFFFF) as i64
            + (self.data_regs[19] & 0xFFFF) as i64;
        let prod = zsf3 * sum;
        self.set_mac0(prod);
        self.set_otz(prod >> 12);
    }

    fn exec_avsz4(&mut self) {
        let zsf4 = ((self.ctrl_regs[30] as i16) as i32) as i64;
        let sum = (self.data_regs[16] & 0xFFFF) as i64
            + (self.data_regs[17] & 0xFFFF) as i64
            + (self.data_regs[18] & 0xFFFF) as i64
            + (self.data_regs[19] & 0xFFFF) as i64;
        let prod = zsf4 * sum;
        self.set_mac0(prod);
        self.set_otz(prod >> 12);
    }

    fn exec_gpf(&mut self, sf_shift: u8, lm: bool) {
        let ir0 = self.ir(0) as i64;
        for i in 1..=3 {
            let prod = self.check_mac44(i, (self.ir(i) as i64) * ir0);
            let m = self.finish_mac(i, prod, sf_shift);
            self.set_ir(i, m, lm);
        }
        self.push_rgb_from_mac();
    }

    fn exec_gpl(&mut self, sf_shift: u8, lm: bool) {
        let ir0 = self.ir(0) as i64;
        for i in 1..=3 {
            let base = ((self.data_regs[24 + i] as i32) as i64) << sf_shift;
            let sum = self.check_mac44(i, base + (self.ir(i) as i64) * ir0);
            let m = self.finish_mac(i, sum, sf_shift);
            self.set_ir(i, m, lm);
        }
        self.push_rgb_from_mac();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lzcs_lzcr_irgb_orgb_and_sxyp_fifo_semantics() {
        let mut gte = Gte::new();
        assert_eq!(gte.read_data(31), 32);

        gte.write_data(30, 0x0001_8000); // positive -> 15 leading zeros
        assert_eq!(gte.read_data(31), 15);
        gte.write_data(30, 0xFFFF_FFFF); // negative -1 -> 32 leading ones
        assert_eq!(gte.read_data(31), 32);
        gte.write_data(30, 0xFFF0_0000); // negative -> 12 leading ones
        assert_eq!(gte.read_data(31), 12);

        // IRGB -> IR1..3 -> ORGB roundtrip
        let rgb555 = 0x001F | (0x0010 << 5) | (0x0007 << 10);
        gte.write_data(28, rgb555);
        assert_eq!(gte.read_data(29), rgb555);
        assert_eq!(gte.read_data(28), rgb555);

        // SXYP FIFO push
        gte.write_data(15, 0x0001_0002);
        gte.write_data(15, 0x0003_0004);
        gte.write_data(15, 0x0005_0006);
        assert_eq!(gte.read_data(12), 0x0001_0002);
        assert_eq!(gte.read_data(13), 0x0003_0004);
        assert_eq!(gte.read_data(14), 0x0005_0006);
        assert_eq!(gte.read_data(15), 0x0005_0006);
    }

    #[test]
    fn nclip_computes_winding_determinant_and_flag_summary() {
        let mut gte = Gte::new();
        // Counter-clockwise triangle: (0,0), (10,0), (0,20) -> 0*(0-20) + 10*(20-0) + 0*(0-0) = +200
        gte.write_data(12, 0);
        gte.write_data(13, 10);
        gte.write_data(14, 20 << 16);
        gte.execute(0x06);
        assert_eq!(gte.read_data(24) as i32, 200);
        assert_eq!(gte.read_ctrl(31), 0);

        // Force FLAG bit 24 (in 0x7F87_E000) vs bit 22 (NOT in 0x7F87_E000!) via CTC2
        gte.write_ctrl(31, 1 << 22); // IR3 sat alone does NOT set bit 31!
        assert_eq!(gte.read_ctrl(31) & 0x8000_0000, 0);
        gte.write_ctrl(31, 1 << 24); // IR1 sat DOES set bit 31!
        assert_ne!(gte.read_ctrl(31) & 0x8000_0000, 0);
    }

    #[test]
    fn rtps_projects_3d_point_accurately() {
        let mut gte = Gte::new();
        // Identity rotation matrix (1.0 = 0x1000 on diagonal: RT11, RT22, RT33)
        gte.write_ctrl(0, 0x0000_1000); // RT11 = 1.0, RT12 = 0
        gte.write_ctrl(2, 0x0000_1000); // RT22 = 1.0, RT23 = 0
        gte.write_ctrl(4, 0x0000_1000); // RT33 = 1.0
        // Translation: TRX = 0, TRY = 0, TRZ = 200
        gte.write_ctrl(7, 200);
        // Screen center offset (160.0, 120.0) in 16.16 fixed point, H = 200 (`H/SZ3 = 1.0`)
        gte.write_ctrl(24, 160 << 16);
        gte.write_ctrl(25, 120 << 16);
        gte.write_ctrl(26, 200);

        // Vertex V0 = (+40, -30, 0) -> camera space (40, -30, 200) -> screen (160 + 40 = 200, 120 - 30 = 90)
        let vx = 40i16 as u16 as u32;
        let vy = (-30i16) as u16 as u32;
        gte.write_data(0, vx | (vy << 16));
        gte.write_data(1, 0);

        // Execute RTPS with `sf = 1` (bit 19)
        let cycles = gte.execute((1 << 19) | 0x01);
        assert_eq!(cycles, 15);
        assert_eq!(gte.read_data(19), 200); // SZ3 = 200
        let sxy2 = gte.read_data(14);
        let sx2 = sxy2 as i16;
        let sy2 = (sxy2 >> 16) as i16;
        assert_eq!((sx2, sy2), (200, 90));
        assert_eq!(gte.read_ctrl(31) & 0x8000_0000, 0);
    }

    #[test]
    fn mvmva_replicates_hardware_cv2_far_color_bug_and_mx3_garbage_matrix() {
        let mut gte = Gte::new();
        // Set RT = Identity (0x1000), FC = (1000, 0, 0), V0 = (10, 20, 30)
        gte.write_ctrl(0, 0x0000_1000);
        gte.write_ctrl(2, 0x0000_1000);
        gte.write_ctrl(4, 0x0000_1000);
        gte.write_ctrl(21, 1000); // RFC = 1000 (large enough that 1000 if not dropped would shift MAC1!)
        gte.write_data(0, 10 | (20 << 16));
        gte.write_data(1, 30);

        // MVMVA with sf=1, mx=0 (RT), v=0 (V0), cv=2 (FC bug!):
        // Row 0 drops `(RFC<<12) + RT11*VX0` and computes ONLY `RT12*VY0 + RT13*VZ0 = 0`!
        // Row 1 drops `(GFC<<12) + RT21*VX0` and computes `RT22*VY0 + RT23*VZ0 = 20`!
        let cmd_cv2 = (1 << 19) | (2 << 13) | 0x12;
        gte.execute(cmd_cv2);
        assert_eq!(gte.read_data(9) as i32, 0); // IR1 = 0 (NOT 1010!)
        assert_eq!(gte.read_data(10) as i32, 20); // IR2 = 20

        // Now test mx=3 garbage matrix: Row 0 = [-R*16, +R*16, IR0]
        gte.write_data(6, 0x0000_0010); // R = 16 -> -256, +256
        gte.write_data(8, 0x1000); // IR0 = 1.0 (0x1000)
        // With V0 = (10, 20, 30), sf=0, cv=3 (None): MAC1 = -256*10 + 256*20 + 4096*30 = 2560 + 122880 = 125440
        let cmd_mx3 = (3 << 17) | (3 << 13) | 0x12;
        gte.execute(cmd_mx3);
        assert_eq!(gte.read_data(25) as i32, 125_440);
    }

    #[test]
    fn rtps_clamps_64bit_mac0_overflow_into_sxy2_and_ir0_before_32bit_truncation() {
        let mut gte = Gte::new();
        // Identity RT, TRZ = 0 -> SZ3 = 0 -> UNR divide overflows to q = 0x1_FFFF!
        gte.write_ctrl(0, 0x0000_1000);
        gte.write_ctrl(2, 0x0000_1000);
        gte.write_ctrl(4, 0x0000_1000);
        gte.write_ctrl(24, 0x7FFF_FFFF); // OFX = +i32::MAX
        gte.write_ctrl(25, 0x7FFF_FFFF); // OFY = +i32::MAX
        gte.write_ctrl(26, 0xFFFF); // H = 65535 -> div overflow q = 0x1_FFFF
        gte.write_ctrl(27, 0x7FFF); // DQA = +32767
        gte.write_ctrl(28, 0x7FFF_FFFF); // DQB = +i32::MAX -> DQB + DQA*q > i32::MAX!

        // V0 = (+0x7FFF, +0x7FFF, 0) -> IR1 = IR2 = +0x7FFF -> q*IR1 + OFX overflows i32 positive!
        gte.write_data(0, 0x7FFF_7FFF);
        gte.write_data(1, 0);

        gte.execute((1 << 19) | 0x01); // RTPS sf=1

        let sxy2 = gte.read_data(14);
        assert_eq!(sxy2 as i16, 0x03FF, "SX2 must saturate positive, not wrap");
        assert_eq!(
            (sxy2 >> 16) as i16,
            0x03FF,
            "SY2 must saturate positive, not wrap"
        );
        assert_eq!(
            gte.read_data(8) as i32,
            0x1000,
            "IR0 must saturate to +0x1000 on positive 64-bit MAC0 overflow"
        );
        let flag = gte.read_ctrl(31);
        assert_ne!(flag & (1 << 17), 0, "DIV overflow bit 17 must be set");
        assert_ne!(flag & (1 << 16), 0, "MAC0+ overflow bit 16 must be set");
        assert_ne!(flag & (1 << 14), 0, "SX2 sat bit 14 must be set");
        assert_ne!(flag & (1 << 13), 0, "SY2 sat bit 13 must be set");
        assert_ne!(flag & (1 << 12), 0, "IR0 sat bit 12 must be set");
        assert_ne!(flag & (1 << 31), 0, "Error summary bit 31 must be set");
    }
}
