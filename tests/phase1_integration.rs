use psx_core::{BusError, EventKind, MASTER_CLOCK_HZ, NullGpu, PsxExe, System};

#[test]
fn slice1_headless_system_and_bus_invariants() {
    assert_eq!(MASTER_CLOCK_HZ, 33_868_800);
    let mut sys = System::new(NullGpu::new());
    assert_eq!(sys.cpu.pc, 0xBFC0_0000);

    // Verify BIOS 32-bit read latency is 24 cycles (8-bit bus * 4)
    let (_word, cycles) = sys.bus.read32(0xBFC0_0000).unwrap();
    assert_eq!(cycles, 24);

    // Verify Expansion 1 returns 0xFFFF_FFFF when unattached
    assert_eq!(sys.bus.read32(0x9F00_0000), Ok((0xFFFF_FFFF, 24)));

    // Verify KSEG1 Scratchpad rejection
    assert_eq!(
        sys.bus.read8(0xBF80_0000),
        Err(BusError::Kseg1ScratchpadAccess(0xBF80_0000))
    );

    // Verify scheduler integration
    sys.scheduler.schedule(1024, EventKind::VblankEdge);
    assert_eq!(sys.scheduler.next_event_cycle(), 1024);
}

#[test]
fn end_to_end_cpu_gte_exe_tty_and_postcard_roundtrip() {
    let mut sys = System::new_headless();
    sys.install_biosless_kernel_stub();

    // Small MIPS + GTE program packaged as a valid PS-X EXE:
    // 1. Load SXY0=(0,0), SXY1=(8,0), SXY2=(0,8) into GTE via MTC2
    // 2. Execute GTE `NCLIP` (COP2 0x06) -> MAC0 = +64 (`'A'` - 1 -> add 1 = `'A'` = 65!)
    // 3. MFC2 $a0, $24 (MAC0, 1-instruction load delay!)
    // 4. Print char (`'A'`) via BIOS B0(0x3D) TTY syscall
    let code: [u32; 12] = [
        0x4880_6000, // mtc2  $zero, $12 (SXY0 = 0,0)
        0x2408_0008, // addiu $t0, $zero, 8
        0x4888_6800, // mtc2  $t0, $13   (SXY1 = 8,0)
        0x3C08_0008, // lui   $t0, 8
        0x4888_7000, // mtc2  $t0, $14   (SXY2 = 0,8)
        0x4A00_0006, // cop2  0x06       (NCLIP -> MAC0 = 64)
        0x4804_C000, // mfc2  $a0, $24   (delayed load of MAC0 into $a0!)
        0x2409_003D, // addiu $t1, $zero, 0x3D (executes in MFC2 load delay slot!)
        0x2484_0001, // addiu $a0, $a0, 1      ($a0 = 64 + 1 = 65 = 'A')
        0x0C00_002C, // jal   0x0000_00B0      (B0 putchar 'A')
        0x0000_0000, // nop
        0x1000_FFFF, // b     .
    ];

    let mut raw_exe = vec![0u8; 0x800 + code.len() * 4];
    raw_exe[0..8].copy_from_slice(b"PS-X EXE");
    raw_exe[0x10..0x14].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    raw_exe[0x18..0x1C].copy_from_slice(&0x8001_0000u32.to_le_bytes());
    raw_exe[0x1C..0x20].copy_from_slice(&((code.len() * 4) as u32).to_le_bytes());
    for (i, w) in code.iter().enumerate() {
        raw_exe[0x800 + i * 4..0x800 + (i + 1) * 4].copy_from_slice(&w.to_le_bytes());
    }

    let exe = PsxExe::parse(&raw_exe).unwrap();
    sys.apply_exe_now(&exe);

    let mut tty = Vec::new();
    sys.run_until_cycle(400, false, &mut tty);
    assert_eq!(tty, b"A");

    // Verify deterministic postcard state serialization roundtrip
    let snap = sys.serialize_cpu_state().unwrap();
    let saved_pc = sys.cpu.pc;
    let saved_cycle = sys.scheduler.now();
    sys.step_frame();
    assert_ne!(sys.scheduler.now(), saved_cycle);

    sys.deserialize_cpu_state(&snap).unwrap();
    assert_eq!(sys.cpu.pc, saved_pc);
    assert_eq!(sys.scheduler.now(), saved_cycle);
}
