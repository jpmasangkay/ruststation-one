use super::Cpu;
use super::cop0::ExceptionCode;
use crate::bus::{Bus, GpuBackend};

impl Cpu {
    #[inline]
    pub(crate) fn decode_and_execute<G: GpuBackend>(
        &mut self,
        instr: u32,
        fetch_cycles: u64,
        bus: &mut Bus<G>,
    ) -> u64 {
        let op = instr >> 26;
        let rs = ((instr >> 21) & 0x1F) as u8;
        let rt = ((instr >> 16) & 0x1F) as u8;
        let rd = ((instr >> 11) & 0x1F) as u8;
        let shamt = (instr >> 6) & 0x1F;
        let imm_u = instr & 0xFFFF;
        let imm_s = (imm_u as i16) as i32 as u32;

        match op {
            // ================================================================
            // SPECIAL (0x00)
            // ================================================================
            0x00 => match instr & 0x3F {
                0x00 => {
                    // SLL rd, rt, shamt
                    let val = self.regs[rt as usize] << shamt;
                    self.set_reg(rd, val);
                    0
                }
                0x02 => {
                    // SRL rd, rt, shamt
                    let val = self.regs[rt as usize] >> shamt;
                    self.set_reg(rd, val);
                    0
                }
                0x03 => {
                    // SRA rd, rt, shamt
                    let val = ((self.regs[rt as usize] as i32) >> shamt) as u32;
                    self.set_reg(rd, val);
                    0
                }
                0x04 => {
                    // SLLV rd, rt, rs
                    let s = self.regs[rs as usize] & 0x1F;
                    let val = self.regs[rt as usize] << s;
                    self.set_reg(rd, val);
                    0
                }
                0x06 => {
                    // SRLV rd, rt, rs
                    let s = self.regs[rs as usize] & 0x1F;
                    let val = self.regs[rt as usize] >> s;
                    self.set_reg(rd, val);
                    0
                }
                0x07 => {
                    // SRAV rd, rt, rs
                    let s = self.regs[rs as usize] & 0x1F;
                    let val = ((self.regs[rt as usize] as i32) >> s) as u32;
                    self.set_reg(rd, val);
                    0
                }
                0x08 => {
                    // JR rs
                    let target = self.regs[rs as usize];
                    self.next_pc = target;
                    self.next_is_branch = true;
                    self.next_branch_taken = true;
                    0
                }
                0x09 => {
                    // JALR rd, rs (MUST read rs BEFORE writing rd in case rd == rs!)
                    let target = self.regs[rs as usize];
                    let ret = self.next_pc;
                    self.set_reg(rd, ret);
                    self.next_pc = target;
                    self.next_is_branch = true;
                    self.next_branch_taken = true;
                    0
                }
                0x0C => {
                    // SYSCALL
                    self.raise_exception(ExceptionCode::Syscall, 0, false);
                    0
                }
                0x0D => {
                    // BREAK
                    self.raise_exception(ExceptionCode::Breakpoint, 0, false);
                    0
                }
                0x10 => {
                    // MFHI rd
                    let val = self.hi;
                    self.set_reg(rd, val);
                    0
                }
                0x11 => {
                    // MTHI rs
                    self.hi = self.regs[rs as usize];
                    0
                }
                0x12 => {
                    // MFLO rd
                    let val = self.lo;
                    self.set_reg(rd, val);
                    0
                }
                0x13 => {
                    // MTLO rs
                    self.lo = self.regs[rs as usize];
                    0
                }
                0x18 => {
                    // MULT rs, rt
                    let a = (self.regs[rs as usize] as i32) as i64;
                    let b = (self.regs[rt as usize] as i32) as i64;
                    let prod = (a * b) as u64;
                    self.lo = prod as u32;
                    self.hi = (prod >> 32) as u32;
                    0
                }
                0x19 => {
                    // MULTU rs, rt
                    let a = self.regs[rs as usize] as u64;
                    let b = self.regs[rt as usize] as u64;
                    let prod = a * b;
                    self.lo = prod as u32;
                    self.hi = (prod >> 32) as u32;
                    0
                }
                0x1A => {
                    // DIV rs, rt (PS1 R3000A NEVER traps on div-by-zero or i32::MIN / -1!)
                    let num = self.regs[rs as usize] as i32;
                    let den = self.regs[rt as usize] as i32;
                    if den == 0 {
                        self.lo = if num >= 0 { 0xFFFF_FFFF } else { 1 };
                        self.hi = num as u32;
                    } else if num == i32::MIN && den == -1 {
                        self.lo = 0x8000_0000;
                        self.hi = 0;
                    } else {
                        self.lo = (num / den) as u32;
                        self.hi = (num % den) as u32;
                    }
                    0
                }
                0x1B => {
                    // DIVU rs, rt
                    let num = self.regs[rs as usize];
                    let den = self.regs[rt as usize];
                    if let Some(q) = num.checked_div(den) {
                        self.lo = q;
                        self.hi = num % den;
                    } else {
                        self.lo = 0xFFFF_FFFF;
                        self.hi = num;
                    }
                    0
                }
                0x20 => {
                    // ADD rd, rs, rt (traps on signed 32-bit overflow!)
                    let a = self.regs[rs as usize] as i32;
                    let b = self.regs[rt as usize] as i32;
                    match a.checked_add(b) {
                        Some(sum) => self.set_reg(rd, sum as u32),
                        None => self.raise_exception(ExceptionCode::ArithmeticOverflow, 0, false),
                    }
                    0
                }
                0x21 => {
                    // ADDU rd, rs, rt
                    let val = self.regs[rs as usize].wrapping_add(self.regs[rt as usize]);
                    self.set_reg(rd, val);
                    0
                }
                0x22 => {
                    // SUB rd, rs, rt (traps on signed 32-bit overflow!)
                    let a = self.regs[rs as usize] as i32;
                    let b = self.regs[rt as usize] as i32;
                    match a.checked_sub(b) {
                        Some(diff) => self.set_reg(rd, diff as u32),
                        None => self.raise_exception(ExceptionCode::ArithmeticOverflow, 0, false),
                    }
                    0
                }
                0x23 => {
                    // SUBU rd, rs, rt
                    let val = self.regs[rs as usize].wrapping_sub(self.regs[rt as usize]);
                    self.set_reg(rd, val);
                    0
                }
                0x24 => {
                    // AND rd, rs, rt
                    let val = self.regs[rs as usize] & self.regs[rt as usize];
                    self.set_reg(rd, val);
                    0
                }
                0x25 => {
                    // OR rd, rs, rt
                    let val = self.regs[rs as usize] | self.regs[rt as usize];
                    self.set_reg(rd, val);
                    0
                }
                0x26 => {
                    // XOR rd, rs, rt
                    let val = self.regs[rs as usize] ^ self.regs[rt as usize];
                    self.set_reg(rd, val);
                    0
                }
                0x27 => {
                    // NOR rd, rs, rt
                    let val = !(self.regs[rs as usize] | self.regs[rt as usize]);
                    self.set_reg(rd, val);
                    0
                }
                0x2A => {
                    // SLT rd, rs, rt
                    let val = u32::from(
                        (self.regs[rs as usize] as i32) < (self.regs[rt as usize] as i32),
                    );
                    self.set_reg(rd, val);
                    0
                }
                0x2B => {
                    // SLTU rd, rs, rt
                    let val = u32::from(self.regs[rs as usize] < self.regs[rt as usize]);
                    self.set_reg(rd, val);
                    0
                }
                _ => {
                    self.raise_exception(ExceptionCode::ReservedInstruction, 0, false);
                    0
                }
            },

            // ================================================================
            // REGIMM / BCONDZ (0x01): Exact R3000A hardware decoding
            // ================================================================
            0x01 => {
                // Bit 0 of `rt` chooses BGEZ (1) vs BLTZ (0); link ONLY when `(rt & 0x1E) == 0x10`!
                // Read `regs[rs]` BEFORE writing `$ra` so `bltzal $ra, label` tests old `$ra`!
                let src = self.regs[rs as usize] as i32;
                let is_bgez = (rt & 1) != 0;
                let do_link = (rt & 0x1E) == 0x10;
                let cond = if is_bgez { src >= 0 } else { src < 0 };

                if do_link {
                    let ret = self.next_pc;
                    self.set_reg(31, ret);
                }
                self.next_is_branch = true;
                if cond {
                    let target = self.pc.wrapping_add(imm_s << 2);
                    self.next_pc = target;
                    self.next_branch_taken = true;
                }
                0
            }

            // ================================================================
            // Jumps & Conditional Branches (0x02..=0x07)
            // ================================================================
            0x02 => {
                // J target
                let target = (self.pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2);
                self.next_pc = target;
                self.next_is_branch = true;
                self.next_branch_taken = true;
                0
            }
            0x03 => {
                // JAL target
                let ret = self.next_pc;
                self.set_reg(31, ret);
                let target = (self.pc & 0xF000_0000) | ((instr & 0x03FF_FFFF) << 2);
                self.next_pc = target;
                self.next_is_branch = true;
                self.next_branch_taken = true;
                0
            }
            0x04 => {
                // BEQ rs, rt, offset
                self.next_is_branch = true;
                if self.regs[rs as usize] == self.regs[rt as usize] {
                    let target = self.pc.wrapping_add(imm_s << 2);
                    self.next_pc = target;
                    self.next_branch_taken = true;
                }
                0
            }
            0x05 => {
                // BNE rs, rt, offset
                self.next_is_branch = true;
                if self.regs[rs as usize] != self.regs[rt as usize] {
                    let target = self.pc.wrapping_add(imm_s << 2);
                    self.next_pc = target;
                    self.next_branch_taken = true;
                }
                0
            }
            0x06 => {
                // BLEZ rs, offset
                self.next_is_branch = true;
                if (self.regs[rs as usize] as i32) <= 0 {
                    let target = self.pc.wrapping_add(imm_s << 2);
                    self.next_pc = target;
                    self.next_branch_taken = true;
                }
                0
            }
            0x07 => {
                // BGTZ rs, offset
                self.next_is_branch = true;
                if (self.regs[rs as usize] as i32) > 0 {
                    let target = self.pc.wrapping_add(imm_s << 2);
                    self.next_pc = target;
                    self.next_branch_taken = true;
                }
                0
            }

            // ================================================================
            // Immediate ALU (0x08..=0x0F)
            // ================================================================
            0x08 => {
                // ADDI rt, rs, imm_s
                let a = self.regs[rs as usize] as i32;
                let b = imm_s as i32;
                match a.checked_add(b) {
                    Some(sum) => self.set_reg(rt, sum as u32),
                    None => self.raise_exception(ExceptionCode::ArithmeticOverflow, 0, false),
                }
                0
            }
            0x09 => {
                // ADDIU rt, rs, imm_s
                let val = self.regs[rs as usize].wrapping_add(imm_s);
                self.set_reg(rt, val);
                0
            }
            0x0A => {
                // SLTI rt, rs, imm_s
                let val = u32::from((self.regs[rs as usize] as i32) < (imm_s as i32));
                self.set_reg(rt, val);
                0
            }
            0x0B => {
                // SLTIU rt, rs, imm_s
                let val = u32::from(self.regs[rs as usize] < imm_s);
                self.set_reg(rt, val);
                0
            }
            0x0C => {
                // ANDI rt, rs, imm_u
                let val = self.regs[rs as usize] & imm_u;
                self.set_reg(rt, val);
                0
            }
            0x0D => {
                // ORI rt, rs, imm_u
                let val = self.regs[rs as usize] | imm_u;
                self.set_reg(rt, val);
                0
            }
            0x0E => {
                // XORI rt, rs, imm_u
                let val = self.regs[rs as usize] ^ imm_u;
                self.set_reg(rt, val);
                0
            }
            0x0F => {
                // LUI rt, imm_u
                self.set_reg(rt, imm_u << 16);
                0
            }

            // ================================================================
            // Coprocessors COP0..COP3 (0x10..=0x13)
            // ================================================================
            0x10 => {
                if !self.cop0.is_cop_usable(0) {
                    self.raise_exception(ExceptionCode::CoprocessorUnusable, 0, false);
                    return 0;
                }
                match rs {
                    0x00 => {
                        // MFC0 rt, rd (1-instruction load delay!)
                        if crate::cpu::Cop0::is_reserved_reg(rd) {
                            self.raise_exception(ExceptionCode::ReservedInstruction, 0, false);
                        } else {
                            let val = self.cop0.read_reg(rd);
                            self.queue_load(rt, val);
                        }
                    }
                    0x04 => {
                        // MTC0 rt, rd (writes to SR lag interrupt enable/mask by 1 instruction!)
                        if crate::cpu::Cop0::is_reserved_reg(rd) {
                            self.raise_exception(ExceptionCode::ReservedInstruction, 0, false);
                        } else {
                            let val = self.regs[rt as usize];
                            if rd == 12 {
                                self.sr_irq_delay = Some(self.cop0.sr);
                            }
                            self.cop0.write_reg(rd, val);
                        }
                    }
                    0x10..=0x1F => {
                        if (instr & 0x3F) == 0x10 {
                            self.cop0.rfe();
                        } else {
                            self.raise_exception(ExceptionCode::ReservedInstruction, 0, false);
                        }
                    }
                    _ => self.raise_exception(ExceptionCode::ReservedInstruction, 0, false),
                }
                0
            }
            0x11 => {
                if !self.cop0.is_cop_usable(1) {
                    self.raise_exception(ExceptionCode::CoprocessorUnusable, 1, false);
                }
                0
            }
            0x12 => {
                // COP2: Geometry Transformation Engine (GTE)
                if !self.cop0.is_cop_usable(2) {
                    self.raise_exception(ExceptionCode::CoprocessorUnusable, 2, false);
                    return 0;
                }
                match rs {
                    0x00 => {
                        // MFC2 rt, rd (1-instruction load delay; stalls if GTE is busy!)
                        let stall = self.stall_for_gte(fetch_cycles);
                        let val = self.gte.read_data(rd);
                        self.queue_load(rt, val);
                        stall
                    }
                    0x02 => {
                        // CFC2 rt, rd (1-instruction load delay; stalls if GTE is busy!)
                        let stall = self.stall_for_gte(fetch_cycles);
                        let val = self.gte.read_ctrl(rd);
                        self.queue_load(rt, val);
                        stall
                    }
                    0x04 => {
                        // MTC2 rt, rd (stalls if GTE is busy)
                        let stall = self.stall_for_gte(fetch_cycles);
                        let val = self.regs[rt as usize];
                        self.gte.write_data(rd, val);
                        stall
                    }
                    0x06 => {
                        // CTC2 rt, rd (stalls if GTE is busy)
                        let stall = self.stall_for_gte(fetch_cycles);
                        let val = self.regs[rt as usize];
                        self.gte.write_ctrl(rd, val);
                        stall
                    }
                    0x10..=0x1F => {
                        // GTE command executes asynchronously in parallel with CPU!
                        let stall = self.stall_for_gte(fetch_cycles);
                        let t_issue = self
                            .cycle_count
                            .saturating_add(fetch_cycles.saturating_sub(1))
                            .saturating_add(stall);
                        let latency = self.gte.execute(instr);
                        self.gte_busy_until = t_issue.saturating_add(latency);
                        stall
                    }
                    _ => {
                        self.raise_exception(ExceptionCode::ReservedInstruction, 2, false);
                        0
                    }
                }
            }
            0x13 => {
                if !self.cop0.is_cop_usable(3) {
                    self.raise_exception(ExceptionCode::CoprocessorUnusable, 3, false);
                }
                0
            }

            // ================================================================
            // Memory Loads (0x20..=0x26)
            // ================================================================
            0x20 => {
                // LB rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 1, |cpu, bus, addr| {
                    let (b, c) = bus.read8(addr)?;
                    cpu.queue_load(rt, (b as i8) as i32 as u32);
                    Ok(c)
                })
            }
            0x21 => {
                // LH rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 2, |cpu, bus, addr| {
                    let (h, c) = bus.read16(addr)?;
                    cpu.queue_load(rt, (h as i16) as i32 as u32);
                    Ok(c)
                })
            }
            0x22 => {
                // LWL rt, imm_s(rs) — unaligned MSB merge with load-delay bypass!
                // On little-endian R3000A, LWL drives `vaddr & !3` onto the data bus (`BDA` sees aligned address).
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load_bda(bus, vaddr, vaddr & !3, 1, |cpu, bus, addr| {
                    let (mem, c) = bus.read32(addr & !3)?;
                    let cur = cpu.read_reg_for_lwl_lwr(rt);
                    let merged = match addr & 3 {
                        0 => (cur & 0x00FF_FFFF) | (mem << 24),
                        1 => (cur & 0x0000_FFFF) | (mem << 16),
                        2 => (cur & 0x0000_00FF) | (mem << 8),
                        _ => mem,
                    };
                    cpu.queue_load(rt, merged);
                    Ok(c)
                })
            }
            0x23 => {
                // LW rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 4, |cpu, bus, addr| {
                    let (w, c) = bus.read32(addr)?;
                    cpu.queue_load(rt, w);
                    Ok(c)
                })
            }
            0x24 => {
                // LBU rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 1, |cpu, bus, addr| {
                    let (b, c) = bus.read8(addr)?;
                    cpu.queue_load(rt, b as u32);
                    Ok(c)
                })
            }
            0x25 => {
                // LHU rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 2, |cpu, bus, addr| {
                    let (h, c) = bus.read16(addr)?;
                    cpu.queue_load(rt, h as u32);
                    Ok(c)
                })
            }
            0x26 => {
                // LWR rt, imm_s(rs) — unaligned LSB merge with load-delay bypass!
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 1, |cpu, bus, addr| {
                    let (mem, c) = bus.read32(addr & !3)?;
                    let cur = cpu.read_reg_for_lwl_lwr(rt);
                    let merged = match addr & 3 {
                        0 => mem,
                        1 => (cur & 0xFF00_0000) | (mem >> 8),
                        2 => (cur & 0xFFFF_0000) | (mem >> 16),
                        _ => (cur & 0xFFFF_FF00) | (mem >> 24),
                    };
                    cpu.queue_load(rt, merged);
                    Ok(c)
                })
            }

            // ================================================================
            // Memory Stores (0x28..=0x2E)
            // ================================================================
            0x28 => {
                // SB rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                let val = self.regs[rt as usize] as u8;
                self.exec_store(bus, vaddr, 1, |_cpu, bus, addr| bus.write8(addr, val))
            }
            0x29 => {
                // SH rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                let val = self.regs[rt as usize] as u16;
                self.exec_store(bus, vaddr, 2, |_cpu, bus, addr| bus.write16(addr, val))
            }
            0x2A => {
                // SWL rt, imm_s(rs)
                // On little-endian R3000A, SWL drives `vaddr & !3` onto the data bus (`BDA` sees aligned address).
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                let reg_val = self.regs[rt as usize];
                self.exec_store_bda(bus, vaddr, vaddr & !3, 1, |_cpu, bus, addr| {
                    let aligned = addr & !3;
                    let (mem, rc) = bus.read32(aligned)?;
                    let merged = match addr & 3 {
                        0 => (mem & 0xFFFF_FF00) | (reg_val >> 24),
                        1 => (mem & 0xFFFF_0000) | (reg_val >> 16),
                        2 => (mem & 0xFF00_0000) | (reg_val >> 8),
                        _ => reg_val,
                    };
                    let wc = bus.write32(aligned, merged)?;
                    Ok(rc + wc)
                })
            }
            0x2B => {
                // SW rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                let val = self.regs[rt as usize];
                self.exec_store(bus, vaddr, 4, |_cpu, bus, addr| bus.write32(addr, val))
            }
            0x2E => {
                // SWR rt, imm_s(rs)
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                let reg_val = self.regs[rt as usize];
                self.exec_store(bus, vaddr, 1, |_cpu, bus, addr| {
                    let aligned = addr & !3;
                    let (mem, rc) = bus.read32(aligned)?;
                    let merged = match addr & 3 {
                        0 => reg_val,
                        1 => (mem & 0x0000_00FF) | (reg_val << 8),
                        2 => (mem & 0x0000_FFFF) | (reg_val << 16),
                        _ => (mem & 0x00FF_FFFF) | (reg_val << 24),
                    };
                    let wc = bus.write32(aligned, merged)?;
                    Ok(rc + wc)
                })
            }

            // ================================================================
            // Coprocessor Loads & Stores LWC0..3 / SWC0..3 (0x30..=0x3B)
            // ================================================================
            0x30..=0x33 => {
                let cop_num = (op - 0x30) as u8;
                if !self.cop0.is_cop_usable(cop_num) {
                    self.raise_exception(ExceptionCode::CoprocessorUnusable, cop_num, false);
                    return 0;
                }
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                self.exec_load(bus, vaddr, 4, |cpu, bus, addr| {
                    let stall = if cop_num == 2 {
                        cpu.stall_for_gte(fetch_cycles)
                    } else {
                        0
                    };
                    let (word, c) = bus.read32(addr)?;
                    if cop_num == 2 {
                        cpu.gte.write_data(rt, word);
                    }
                    Ok(stall + c)
                })
            }
            0x38..=0x3B => {
                let cop_num = (op - 0x38) as u8;
                if !self.cop0.is_cop_usable(cop_num) {
                    self.raise_exception(ExceptionCode::CoprocessorUnusable, cop_num, false);
                    return 0;
                }
                let vaddr = self.regs[rs as usize].wrapping_add(imm_s);
                let stall = if cop_num == 2 {
                    self.stall_for_gte(fetch_cycles)
                } else {
                    0
                };
                let val = if cop_num == 2 {
                    self.gte.read_data(rt)
                } else {
                    0
                };
                self.exec_store(bus, vaddr, 4, |_cpu, bus, addr| {
                    let c = bus.write32(addr, val)?;
                    Ok(stall + c)
                })
            }

            _ => {
                self.raise_exception(ExceptionCode::ReservedInstruction, (op & 3) as u8, false);
                0
            }
        }
    }

    #[inline]
    fn exec_load<G, F>(&mut self, bus: &mut Bus<G>, vaddr: u32, align: u32, f: F) -> u64
    where
        G: GpuBackend,
        F: FnOnce(&mut Self, &mut Bus<G>, u32) -> Result<u64, crate::bus::BusError>,
    {
        self.exec_load_bda(bus, vaddr, vaddr, align, f)
    }

    #[inline]
    fn exec_load_bda<G, F>(
        &mut self,
        bus: &mut Bus<G>,
        vaddr: u32,
        bda_addr: u32,
        align: u32,
        f: F,
    ) -> u64
    where
        G: GpuBackend,
        F: FnOnce(&mut Self, &mut Bus<G>, u32) -> Result<u64, crate::bus::BusError>,
    {
        let addr_err =
            (vaddr & (align - 1)) != 0 || (self.cop0.is_user_mode() && vaddr >= 0x8000_0000);
        let bp_trap = self.cop0.check_data_breakpoint(bda_addr, false);
        if addr_err {
            self.cop0.bad_vaddr = vaddr;
            let ce = ((self.last_opcode >> 26) & 3) as u8;
            self.raise_exception(ExceptionCode::AddressErrorLoad, ce, false);
            return 0;
        }
        if bp_trap {
            self.raise_exception(ExceptionCode::Breakpoint, 0, true);
            return 0;
        }
        if self.cop0.is_cache_isolated() {
            // Isolated cache loads do not reach physical memory
            return 0;
        }
        match f(self, bus, vaddr) {
            Ok(cycles) => cycles,
            Err(_) => {
                self.raise_exception(ExceptionCode::DataBusError, 0, false);
                4
            }
        }
    }

    #[inline]
    fn exec_store<G, F>(&mut self, bus: &mut Bus<G>, vaddr: u32, align: u32, f: F) -> u64
    where
        G: GpuBackend,
        F: FnOnce(&mut Self, &mut Bus<G>, u32) -> Result<u64, crate::bus::BusError>,
    {
        self.exec_store_bda(bus, vaddr, vaddr, align, f)
    }

    #[inline]
    fn exec_store_bda<G, F>(
        &mut self,
        bus: &mut Bus<G>,
        vaddr: u32,
        bda_addr: u32,
        align: u32,
        f: F,
    ) -> u64
    where
        G: GpuBackend,
        F: FnOnce(&mut Self, &mut Bus<G>, u32) -> Result<u64, crate::bus::BusError>,
    {
        let addr_err =
            (vaddr & (align - 1)) != 0 || (self.cop0.is_user_mode() && vaddr >= 0x8000_0000);
        let bp_trap = self.cop0.check_data_breakpoint(bda_addr, true);
        if addr_err {
            self.cop0.bad_vaddr = vaddr;
            let ce = ((self.last_opcode >> 26) & 3) as u8;
            self.raise_exception(ExceptionCode::AddressErrorStore, ce, false);
            return 0;
        }
        if bp_trap {
            self.raise_exception(ExceptionCode::Breakpoint, 0, true);
            return 0;
        }
        if self.cop0.is_cache_isolated() {
            // Isolated cache writes invalidate the corresponding I-Cache line instead of writing RAM!
            self.invalidate_icache_line(vaddr);
            return 0;
        }
        match f(self, bus, vaddr) {
            Ok(cycles) => {
                self.notify_ram_write(vaddr);
                cycles
            }
            Err(_) => {
                self.raise_exception(ExceptionCode::DataBusError, 0, false);
                1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::bus::{Bus, NullGpu};
    use crate::cpu::{Cpu, ExceptionCode};

    fn setup_code(words: &[u32]) -> (Cpu, Bus<NullGpu>) {
        let mut bus = Bus::new(NullGpu::new());
        for (i, &w) in words.iter().enumerate() {
            bus.write32(0x8001_0000 + (i as u32) * 4, w).unwrap();
        }
        let mut cpu = Cpu::new_reset();
        cpu.pc = 0x8001_0000;
        cpu.next_pc = 0x8001_0004;
        // Enable BEV = 0 so exceptions vector to 0x8000_0080, and enable COP2 (CU2 = bit 30)
        cpu.cop0.sr = 1 << 30;
        (cpu, bus)
    }

    #[test]
    fn r0_remains_strictly_zero_across_alu_delayed_loads_and_mfc2() {
        let prog = [
            0x2400_1234, // addiu $zero, $zero, 0x1234
            0x8C00_2000, // lw    $zero, 0x2000($zero)
            0x4800_0800, // mfc2  $zero, $1
            0x0000_0000, // nop
            0x0000_0000, // nop
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        bus.write32(0x0000_2000, 0xDEAD_BEEF).unwrap();
        cpu.gte.write_data(1, 0xCAFE_BABE);

        for _ in 0..prog.len() {
            cpu.step_instruction(&mut bus);
            assert_eq!(cpu.regs[0], 0);
        }
    }

    #[test]
    fn two_slot_load_delay_and_delay_slot_cancellation() {
        let prog = [
            0x8C08_2000, // lw    $t0, 0x2000($zero)   -> queues $t0 = 0x1122_3344
            0x0100_4821, // addu  $t1, $t0, $zero      -> reads OLD $t0 (0x99) in delay slot!
            0x0100_5021, // addu  $t2, $t0, $zero      -> reads NEW $t0 (0x1122_3344)!
            // Now test load cancellation by an ALU write in the delay slot:
            0x8C08_2004, // lw    $t0, 0x2004($zero)   -> queues $t0 = 0xDEAD_BEEF
            0x2408_0077, // addiu $t0, $zero, 0x0077   -> overwrites $t0 AND cancels queued load!
            0x0000_0000, // nop
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        cpu.regs[8] = 0x99;
        bus.write32(0x0000_2000, 0x1122_3344).unwrap();
        bus.write32(0x0000_2004, 0xDEAD_BEEF).unwrap();

        cpu.step_instruction(&mut bus); // lw $t0
        assert_eq!(cpu.regs[8], 0x99); // Not visible yet!

        cpu.step_instruction(&mut bus); // addu $t1, $t0, $zero
        assert_eq!(cpu.regs[9], 0x99); // Delay slot saw old $t0
        assert_eq!(cpu.regs[8], 0x1122_3344); // Load committed at end of delay slot!

        cpu.step_instruction(&mut bus); // addu $t2, $t0, $zero
        assert_eq!(cpu.regs[10], 0x1122_3344);

        cpu.step_instruction(&mut bus); // lw $t0, 0x2004
        cpu.step_instruction(&mut bus); // addiu $t0, $zero, 0x77 (cancels load!)
        cpu.step_instruction(&mut bus); // nop
        assert_eq!(cpu.regs[8], 0x0077);
    }

    #[test]
    fn back_to_back_lwl_lwr_bypasses_load_delay_and_roundtrips_swl_swr() {
        let prog = [
            // Store 0xDDCC_BBAA to unaligned address 0x2001 via SWR + SWL:
            0xBA09_2001, // swr $t1, 0x2001($s0)  (LSB at +0)
            0xAA09_2004, // swl $t1, 0x2004($s0)  (MSB at +3)
            // Load back from 0x2001 into $t2 back-to-back with NO nop between LWR & LWL!
            0x9A0A_2001, // lwr $t2, 0x2001($s0)
            0x8A0A_2004, // lwl $t2, 0x2004($s0)  -> must merge against in-flight LWR!
            0x0000_0000, // nop (let final LWL commit)
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        cpu.regs[9] = 0xDDCC_BBAA; // $t1
        cpu.regs[10] = 0xFFFF_FFFF; // $t2 initial garbage
        bus.write32(0x0000_2000, 0x1111_1111).unwrap();
        bus.write32(0x0000_2004, 0x2222_2222).unwrap();

        for _ in 0..prog.len() {
            cpu.step_instruction(&mut bus);
        }
        assert_eq!(cpu.regs[10], 0xDDCC_BBAA);
        // Neighboring bytes at 0x2000 (0x11) and 0x2005..2007 (0x222222) must remain intact!
        assert_eq!(bus.read8(0x0000_2000).unwrap().0, 0x11);
        assert_eq!(bus.read8(0x0000_2005).unwrap().0, 0x22);
    }

    #[test]
    fn branch_delay_slot_exception_sets_epc_minus_four_and_bd_flag() {
        let prog = [
            0x1000_0004, // 0x8001_0000: beq $zero, $zero, +4
            0x0000_000C, // 0x8001_0004: syscall (inside branch delay slot!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        cpu.step_instruction(&mut bus); // beq
        cpu.step_instruction(&mut bus); // syscall in delay slot

        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(cpu.cop0.epc, 0x8001_0000);
        assert_ne!(cpu.cop0.cause & (1 << 31), 0, "CAUSE.BD must be set");
        assert_eq!((cpu.cop0.cause >> 2) & 0x1F, ExceptionCode::Syscall as u32);
    }

    #[test]
    fn div_and_divu_match_ps1_hardware_edge_cases_without_trapping() {
        let prog = [
            0x0109_001A, // div  $t0, $t1  (+42 / 0)
            0x012A_001A, // div  $t1, $t2  (-42 / 0) -- wait: use $t3 (-42) / $t1 (0)
            0x018D_001A, // div  $t4, $t5  (i32::MIN / -1)
            0x0109_001B, // divu $t0, $t1  (42u32 / 0)
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        cpu.regs[8] = 42; // $t0 = +42
        cpu.regs[9] = 0; // $t1 = 0
        cpu.regs[12] = 0x8000_0000; // $t4 = i32::MIN
        cpu.regs[13] = 0xFFFF_FFFF; // $t5 = -1

        // 1. +42 / 0 -> LO = -1 (0xFFFF_FFFF), HI = 42
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.lo, 0xFFFF_FFFF);
        assert_eq!(cpu.hi, 42);

        // 2. -42 / 0 -> LO = +1, HI = -42
        bus.write32(0x8001_0004, (11 << 21) | (9 << 16) | 0x1A)
            .unwrap(); // div $t3, $t1
        cpu.regs[11] = (-42i32) as u32;
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.lo, 1);
        assert_eq!(cpu.hi, (-42i32) as u32);

        // 3. i32::MIN / -1 -> LO = 0x8000_0000, HI = 0 (no trap!)
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.lo, 0x8000_0000);
        assert_eq!(cpu.hi, 0);
        assert_eq!(cpu.pc, 0x8001_000C);

        // 4. 42u32 / 0 -> LO = 0xFFFF_FFFF, HI = 42
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.lo, 0xFFFF_FFFF);
        assert_eq!(cpu.hi, 42);
    }

    #[test]
    fn sr_isc_isolated_cache_prevents_stores_from_modifying_ram() {
        let prog = [
            0xAC08_2000, // sw $t0, 0x2000($zero) while SR.IsC == 1
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        bus.write32(0x0000_2000, 0xAAAA_5555).unwrap();
        cpu.regs[8] = 0xDEAD_BEEF;
        cpu.cop0.sr |= 1 << 16; // Set IsC = 1

        cpu.step_instruction(&mut bus);
        assert_eq!(bus.read32(0x0000_2000).unwrap().0, 0xAAAA_5555);
    }

    #[test]
    fn cop0_exception_mode_stack_rfe_cpu_unusable_and_dcic_breakpoint() {
        let prog = [
            0x4808_0800, // 0x8001_0000: mfc2 $t0, $1 (with CU2 disabled -> CpU CE=2!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        // Set User mode + IRQ enabled: KUc=1, IEc=1 -> low 6 bits = 0b00_00_11 (0x03), CU2=0
        cpu.cop0.sr = 0x0000_0003;

        // Place `jr $k0; rfe` at exception vector 0x8000_0080 (with return addr in $k0)
        bus.write32(0x8000_0080, 0x0340_0008).unwrap(); // jr $k0 ($26)
        bus.write32(0x8000_0084, 0x4200_0010).unwrap(); // rfe (in branch delay slot!)
        bus.write32(0x0000_3000, 0x2409_002A).unwrap(); // user code at 0x0000_3000: addiu $t1, $zero, 42
        cpu.pc = 0x0000_3000;
        cpu.next_pc = 0x0000_3004;
        bus.write32(0x0000_3000, 0x4808_0800).unwrap(); // overwrite 0x3000 with mfc2 (faults!)
        bus.write32(0x0000_3004, 0x2409_002A).unwrap(); // 0x3004: addiu $t1, $zero, 42
        cpu.regs[26] = 0x0000_3004; // $k0 skips faulting instr to 0x3004

        cpu.step_instruction(&mut bus); // Traps CpU(2)!
        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(cpu.cop0.sr & 0x3F, 0b00_11_00); // Pushed left by 2: KUp=1, IEp=1, KUc=0, IEc=0
        assert_eq!(
            (cpu.cop0.cause >> 2) & 0x1F,
            ExceptionCode::CoprocessorUnusable as u32
        );
        assert_eq!((cpu.cop0.cause >> 28) & 0x03, 2); // CE = 2

        cpu.step_instruction(&mut bus); // jr $k0
        cpu.step_instruction(&mut bus); // rfe -> shifts 0b00_11_00 >> 2 = 0b00_00_11 (restores KUc=1, IEc=1)!
        assert_eq!(cpu.cop0.sr & 0x3F, 0b00_00_11);
        assert_eq!(cpu.pc, 0x0000_3004);

        // Now enable DCIC execution breakpoint on 0x0000_3004 -> must vector to 0x8000_0040!
        cpu.cop0.bpc = 0x0000_3004;
        cpu.cop0.bpcm = 0xFFFF_FFFF;
        cpu.cop0.dcic = (1 << 31) | (1 << 30) | (1 << 24) | (1 << 23);
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8000_0040);
        assert_eq!(cpu.cop0.dcic & 0x03, 0x03);
    }

    #[test]
    fn bcondz_handles_bltzal_with_rs_ra_and_unofficial_rt_encodings_and_overflow() {
        let prog = [
            // 0x8001_0000: bltzal $ra, +2 (rs=31, rt=0x10, offset=2 -> target 0x8001_000C)
            (0x01 << 26) | (31 << 21) | (0x10 << 16) | 2,
            0x0000_0000, // delay slot nop
            0x0000_0000, // skipped!
            // 0x8001_000C: unofficial BCONDZ rt=0x13 (odd -> BGEZ, (rt & 0x1E) != 0x10 -> NO link!)
            (0x01 << 26) | (8 << 21) | (0x13 << 16) | 1,
            0x0000_0000, // delay slot
            // 0x8001_0014: ADD $t0, $t1, $t2 with signed overflow (0x7FFF_FFFF + 1 -> Ov!)
            (9 << 21) | (10 << 16) | (8 << 11) | 0x20,
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        cpu.regs[31] = 0xFFFF_FFFE; // negative $ra -> bltzal MUST branch AND link 0x8001_0008!
        cpu.regs[8] = 5; // $t0 = +5 (>= 0)
        cpu.regs[9] = 0x7FFF_FFFF;
        cpu.regs[10] = 1;

        cpu.step_instruction(&mut bus); // bltzal $ra
        assert_eq!(cpu.regs[31], 0x8001_0008);
        cpu.step_instruction(&mut bus); // delay slot
        assert_eq!(cpu.pc, 0x8001_000C);

        cpu.step_instruction(&mut bus); // unofficial BCONDZ rt=0x13 -> BGEZ taken, $ra NOT overwritten!
        assert_eq!(cpu.regs[31], 0x8001_0008);
        cpu.step_instruction(&mut bus); // delay slot
        assert_eq!(cpu.pc, 0x8001_0014);

        cpu.step_instruction(&mut bus); // ADD signed overflow -> traps Ov without clobbering $t0!
        assert_eq!(cpu.regs[8], 5);
        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(
            (cpu.cop0.cause >> 2) & 0x1F,
            ExceptionCode::ArithmeticOverflow as u32
        );
    }

    #[test]
    fn cop0_tar_only_latches_on_branch_delay_exception() {
        let prog = [
            0x1000_0001, // 0x8001_0000: beq $zero, $zero, +1 -> target 0x8001_0008
            0x0000_0000, // 0x8001_0004: nop (normal delay slot)
            0x1000_0002, // 0x8001_0008: beq $zero, $zero, +2 -> target 0x8001_0014
            0x0000_000C, // 0x8001_000C: syscall (exception inside branch delay slot!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog);
        cpu.cop0.jumpdest = 0x1234_5678;

        // Normal taken branch + delay slot must NOT overwrite TAR / JUMPDEST
        cpu.step_instruction(&mut bus);
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8001_0008);
        assert_eq!(cpu.cop0.jumpdest, 0x1234_5678);

        // Exception inside taken branch delay slot MUST latch TAR, BD, BT, and EPC = branch PC
        cpu.step_instruction(&mut bus);
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(cpu.cop0.epc, 0x8001_0008);
        assert_eq!(cpu.cop0.jumpdest, 0x8001_0014);
        assert_eq!(cpu.cop0.cause & 0xC000_0000, 0xC000_0000); // BD | BT
    }

    #[test]
    fn mtc0_sr_delays_irq_by_one_instruction_unlike_mtc0_cause() {
        // Case A: CAUSE.Sw0 already pending; MTC0 SR enables IEc|IM0 -> next insn runs before IRQ!
        let prog_sr = [
            (0x10 << 26) | (0x04 << 21) | (8 << 16) | (12 << 11), // 0x8001_0000: mtc0 $t0, SR
            0x2409_0042, // 0x8001_0004: addiu $t1, $zero, 0x42 (MUST execute!)
            0x240A_0099, // 0x8001_0008: addiu $t2, $zero, 0x99 (IRQ fires here!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog_sr);
        cpu.cop0.cause = 0x0000_0100; // Sw0 pending
        cpu.regs[8] = 0x0000_0101; // IEc | IM0

        cpu.step_instruction(&mut bus); // mtc0 SR
        cpu.step_instruction(&mut bus); // addiu $t1, 0x42 runs under previous SR!
        assert_eq!(cpu.regs[9], 0x42);
        assert_eq!(cpu.pc, 0x8001_0008);

        cpu.step_instruction(&mut bus); // Now IRQ fires with EPC = 0x8001_0008!
        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(cpu.cop0.epc, 0x8001_0008);
        assert_eq!(cpu.regs[10], 0);

        // Case B: SR already has IEc|IM0; MTC0 CAUSE sets Sw0 -> IRQ fires immediately at +4!
        let prog_cause = [
            (0x10 << 26) | (0x04 << 21) | (8 << 16) | (13 << 11), // 0x8001_0000: mtc0 $t0, CAUSE
            0x2409_0042, // 0x8001_0004: addiu $t1, $zero, 0x42 (preempted!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog_cause);
        cpu.cop0.sr = 0x0000_0101;
        cpu.regs[8] = 0x0000_0100;

        cpu.step_instruction(&mut bus); // mtc0 CAUSE
        cpu.step_instruction(&mut bus); // IRQ fires immediately!
        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(cpu.cop0.epc, 0x8001_0004);
        assert_eq!(cpu.regs[9], 0);
    }

    #[test]
    fn cause_ce_reserved_cop0_regs_and_dcic_lwl_swl_bda() {
        // 1. Reserved COP0 register r0 raises RI; unimplemented r16 does not
        let prog_cop0 = [
            (0x10 << 26) | (8 << 16) | (16 << 11), // mfc0 $t0, $16 (reads 0, no trap)
            (0x10 << 26) | (8 << 16),              // mfc0 $t0, $0  (reserved TLB reg -> RI!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog_cop0);
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8001_0004);
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8000_0080);
        assert_eq!(
            (cpu.cop0.cause >> 2) & 0x1F,
            ExceptionCode::ReservedInstruction as u32
        );

        // 2. Illegal primary opcode sets CAUSE.CE = op & 3
        let (mut cpu, mut bus) = setup_code(&[0xF800_0000]); // op = 0x3E -> CE = 2
        cpu.step_instruction(&mut bus);
        assert_eq!(
            (cpu.cop0.cause >> 2) & 0x1F,
            ExceptionCode::ReservedInstruction as u32
        );
        assert_eq!((cpu.cop0.cause >> 28) & 3, 2);

        // 3. Misaligned LW sets CAUSE.CE = 3 AND latches DCIC BDA status bits even when TR = 0
        let prog_adel = [
            0x8D09_0001, // lw $t1, 1($t0) -> vaddr 0x8000_2001 (misaligned!)
        ];
        let (mut cpu, mut bus) = setup_code(&prog_adel);
        cpu.regs[8] = 0x8000_2000;
        cpu.cop0.bda = 0x8000_2001;
        cpu.cop0.bdam = 0xFFFF_FFFF;
        // DE(23) | DAE(25) | DR(26) | KD(29), TR(31) = 0
        cpu.cop0.dcic = (1 << 29) | (1 << 26) | (1 << 25) | (1 << 23);
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8000_0080); // AdEL wins over BDA
        assert_eq!((cpu.cop0.cause >> 28) & 3, 3); // LW op=0x23 -> CE=3
        assert_eq!(cpu.cop0.dcic & 0x0D, 0x0D); // Status bits 0, 2, 3 latched!

        // 4. LWL/SWL drive aligned `vaddr & !3` to BDA, whereas LWR/SWR drive exact `vaddr`
        let prog_unaligned = [
            0x8909_0003, // lwl $t1, 3($t0) -> BDA sees 0x8000_2000
            0x9909_0003, // lwr $t1, 3($t0) -> BDA sees 0x8000_2003
        ];
        let (mut cpu, mut bus) = setup_code(&prog_unaligned);
        cpu.regs[8] = 0x8000_2000;
        cpu.cop0.bda = 0x8000_2003;
        cpu.cop0.bdam = 0xFFFF_FFFF;
        cpu.cop0.dcic = (1 << 31) | (1 << 29) | (1 << 26) | (1 << 25) | (1 << 23);
        cpu.step_instruction(&mut bus); // LWL checks 0x8000_2000 -> no match!
        assert_eq!(cpu.pc, 0x8001_0004);
        assert_eq!(cpu.cop0.dcic & 0x1F, 0);
        cpu.step_instruction(&mut bus); // LWR checks 0x8000_2003 -> debug trap to 0x8000_0040!
        assert_eq!(cpu.pc, 0x8000_0040);
        assert_eq!(cpu.cop0.dcic & 0x0D, 0x0D);

        // 5. DCIC UD (bit 30) gates < 0x8000_0000 and must NOT match >= 0x8000_0000
        let (mut cpu, mut bus) = setup_code(&[0x0000_0000]);
        cpu.cop0.bpc = 0x8001_0000;
        cpu.cop0.bpcm = 0xFFFF_FFFF;
        cpu.cop0.dcic = (1 << 31) | (1 << 30) | (1 << 24) | (1 << 23); // UD only, KD=0
        cpu.step_instruction(&mut bus);
        assert_eq!(cpu.pc, 0x8001_0004);
        assert_eq!(cpu.cop0.dcic & 0x03, 0);
    }

    #[test]
    fn gte_async_pipeline_stall_matches_hardware_cycle_cadence() {
        let sqr_cmd = (0x12 << 26) | (1 << 25) | 0x28; // SQR (5-cycle GTE latency)
        let mfc2_ir1 = (0x12 << 26) | (8 << 16) | (9 << 11); // mfc2 $t0, IR1

        let measure = |nops: usize| -> u64 {
            let mut prog = alloc::vec::Vec::with_capacity(nops + 2);
            prog.push(sqr_cmd);
            prog.extend(core::iter::repeat_n(0x0000_0000, nops));
            prog.push(mfc2_ir1);

            let (mut cpu, mut bus) = setup_code(&prog);
            // Enable I-Cache and run once to warm cache lines, then measure cached execution
            bus.write32(0xFFFE_0130, 1 << 11).unwrap();
            for _ in 0..prog.len() {
                cpu.step_instruction(&mut bus);
            }
            cpu.pc = 0x8001_0000;
            cpu.next_pc = 0x8001_0004;
            cpu.cycle_count = 1000;
            cpu.gte_busy_until = 0;

            let mut total = 0u64;
            for _ in 0..prog.len() {
                total += cpu.step_instruction(&mut bus);
            }
            total
        };

        // 0 NOPs: SQR(1) + MFC2(1 + (5-1)+1 = 6) = 7 cycles
        assert_eq!(measure(0), 7);
        // 1 NOP: SQR(1) + NOP(1) + MFC2(1 + (5-2)+1 = 5) = 7 cycles
        assert_eq!(measure(1), 7);
        // 3 NOPs: SQR(1) + 3*NOP(3) + MFC2(1 + (5-4)+1 = 3) = 7 cycles
        assert_eq!(measure(3), 7);
        // 4 NOPs: SQR(1) + 4*NOP(4) + MFC2(1 + 0 stall) = 6 cycles (hardware +1 stall release bubble!)
        assert_eq!(measure(4), 6);
    }
}
