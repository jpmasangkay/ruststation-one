use crate::bus::RAM_BYTES;
use alloc::vec::Vec;

pub const PSX_EXE_HEADER_BYTES: usize = 0x800;
pub const BIOS_SHELL_ENTRY_PC: u32 = 0x8003_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExeError {
    TooSmall(usize),
    InvalidMagic,
    LoadAddressOutOfBounds { load_offset: usize, size: usize },
}

/// Parsed Sony `PS-X EXE` executable image (`2048`-byte header + MIPS payload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PsxExe {
    pub initial_pc: u32,
    pub initial_gp: u32,
    pub load_addr: u32,
    pub file_size: u32,
    pub memfill_addr: u32,
    pub memfill_size: u32,
    pub sp_base: u32,
    pub sp_offset: u32,
    pub payload: Vec<u8>,
}

impl PsxExe {
    /// Parses and validates a raw `PS-X EXE` binary at the system boundary.
    pub fn parse(raw: &[u8]) -> Result<Self, ExeError> {
        if raw.len() < PSX_EXE_HEADER_BYTES {
            return Err(ExeError::TooSmall(raw.len()));
        }
        if &raw[0..8] != b"PS-X EXE" {
            return Err(ExeError::InvalidMagic);
        }

        let initial_pc = u32::from_le_bytes([raw[0x10], raw[0x11], raw[0x12], raw[0x13]]);
        let initial_gp = u32::from_le_bytes([raw[0x14], raw[0x15], raw[0x16], raw[0x17]]);
        let load_addr = u32::from_le_bytes([raw[0x18], raw[0x19], raw[0x1A], raw[0x1B]]);
        let file_size = u32::from_le_bytes([raw[0x1C], raw[0x1D], raw[0x1E], raw[0x1F]]);
        let memfill_addr = u32::from_le_bytes([raw[0x28], raw[0x29], raw[0x2A], raw[0x2B]]);
        let memfill_size = u32::from_le_bytes([raw[0x2C], raw[0x2D], raw[0x2E], raw[0x2F]]);
        let sp_base = u32::from_le_bytes([raw[0x30], raw[0x31], raw[0x32], raw[0x33]]);
        let sp_offset = u32::from_le_bytes([raw[0x34], raw[0x35], raw[0x36], raw[0x37]]);

        let body = &raw[PSX_EXE_HEADER_BYTES..];
        let copy_len = if file_size == 0 {
            body.len()
        } else {
            (file_size as usize).min(body.len())
        };
        let alloc_len = (file_size as usize).max(copy_len);

        let ram_offset = (load_addr as usize) & (RAM_BYTES - 1);
        if ram_offset.saturating_add(alloc_len) > RAM_BYTES {
            return Err(ExeError::LoadAddressOutOfBounds {
                load_offset: ram_offset,
                size: alloc_len,
            });
        }

        let mut payload = Vec::with_capacity(alloc_len);
        payload.extend_from_slice(&body[..copy_len]);
        if alloc_len > copy_len {
            payload.resize(alloc_len, 0);
        }

        Ok(Self {
            initial_pc,
            initial_gp,
            load_addr,
            file_size,
            memfill_addr,
            memfill_size,
            sp_base,
            sp_offset,
            payload,
        })
    }

    /// Computes initial `$sp` (`r29`) and `$fp` (`r30`) if `sp_base != 0`.
    #[inline]
    #[must_use]
    pub const fn initial_sp(&self) -> Option<u32> {
        if self.sp_base != 0 {
            Some(self.sp_base.wrapping_add(self.sp_offset))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn parses_valid_psx_exe_header_and_rejects_bad_magic_or_bounds() {
        assert_eq!(PsxExe::parse(&[0u8; 16]), Err(ExeError::TooSmall(16)));
        assert_eq!(
            PsxExe::parse(&[0u8; PSX_EXE_HEADER_BYTES]),
            Err(ExeError::InvalidMagic)
        );

        let mut raw = vec![0u8; PSX_EXE_HEADER_BYTES + 8];
        raw[0..8].copy_from_slice(b"PS-X EXE");
        raw[0x10..0x14].copy_from_slice(&0x8001_0100u32.to_le_bytes()); // initial_pc
        raw[0x14..0x18].copy_from_slice(&0x8002_0000u32.to_le_bytes()); // initial_gp
        raw[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes()); // load_addr
        raw[0x1C..0x20].copy_from_slice(&8u32.to_le_bytes()); // file_size
        raw[0x30..0x34].copy_from_slice(&0x801F_FFF0u32.to_le_bytes()); // sp_base
        raw[0x34..0x38].copy_from_slice(&0x0000_0008u32.to_le_bytes()); // sp_offset
        raw[0x800..0x808].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);

        let exe = PsxExe::parse(&raw).unwrap();
        assert_eq!(exe.initial_pc, 0x8001_0100);
        assert_eq!(exe.initial_gp, 0x8002_0000);
        assert_eq!(exe.load_addr, 0x8001_0000);
        assert_eq!(exe.initial_sp(), Some(0x801F_FFF8));
        assert_eq!(exe.payload, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
