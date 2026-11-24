use psx_core::{PsxExe, System, TtySink};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const DEFAULT_MAX_CYCLES: u64 = 250_000_000;

struct CliConfig {
    bios_path: Option<PathBuf>,
    exe_path: Option<PathBuf>,
    show_tty: bool,
    trace_len: usize,
    max_cycles: u64,
    stop_at_shell: bool,
}

impl CliConfig {
    fn parse_args() -> Result<Self, String> {
        let mut bios_path = None;
        let mut exe_path = None;
        let mut show_tty = false;
        let mut trace_len = 0usize;
        let mut max_cycles = DEFAULT_MAX_CYCLES;
        let mut stop_at_shell = false;

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--bios" => {
                    let val = args.next().ok_or("--bios requires a file path")?;
                    bios_path = Some(PathBuf::from(val));
                }
                "--exe" => {
                    let val = args.next().ok_or("--exe requires a file path")?;
                    exe_path = Some(PathBuf::from(val));
                }
                "--tty" => show_tty = true,
                "--trace" => {
                    let val = args.next().ok_or("--trace requires an entry count")?;
                    trace_len = val
                        .parse::<usize>()
                        .map_err(|_| format!("invalid --trace count: {val}"))?;
                }
                "--max-cycles" => {
                    let val = args.next().ok_or("--max-cycles requires a cycle count")?;
                    max_cycles = val
                        .parse::<u64>()
                        .map_err(|_| format!("invalid --max-cycles value: {val}"))?;
                }
                "--stop-at-shell" => stop_at_shell = true,
                "--help" | "-h" => {
                    print_usage();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown flag: {other}")),
            }
        }

        if bios_path.is_none() && exe_path.is_none() {
            return Err(
                "must provide at least `--bios <path>` or `--exe <path>` (pass `--help` for usage)"
                    .to_string(),
            );
        }

        Ok(Self {
            bios_path,
            exe_path,
            show_tty,
            trace_len,
            max_cycles,
            stop_at_shell,
        })
    }
}

fn print_usage() {
    println!(
        "psx-cli — Headless PlayStation 1 Phase 1 Runner & Hardware Test Harness\n\n\
         USAGE:\n  \
           psx-cli [OPTIONS]\n\n\
         OPTIONS:\n  \
           --bios <path>       Load a 512 KB PS1 BIOS ROM (OpenBIOS or retail scph*.bin)\n  \
           --exe <path>        Sideload a PS-X EXE (at 0x80030000 shell hook, or directly if no --bios)\n  \
           --tty               Stream guest BIOS/DUART TTY output live to stdout\n  \
           --trace <n>         Record and dump the last <n> instructions on exit\n  \
           --max-cycles <n>    Maximum 33.8688 MHz clock cycles to run (default: {DEFAULT_MAX_CYCLES})\n  \
           --stop-at-shell     Halt cleanly when BIOS reaches shell entry (0x80030000)\n  \
           -h, --help          Show this help message"
    );
}

struct StdoutTtySink {
    enabled: bool,
}

impl TtySink for StdoutTtySink {
    fn on_tty_byte(&mut self, byte: u8) {
        if !self.enabled {
            return;
        }
        let mut out = io::stdout().lock();
        let _ = out.write_all(&[byte]);
        if byte == b'\n' {
            let _ = out.flush();
        }
    }
}

fn dump_trace(sys: &System) {
    let entries = sys.trace_entries();
    if entries.is_empty() {
        return;
    }
    eprintln!(
        "\n=== Instruction Trace (last {} instructions) ===",
        entries.len()
    );
    for e in entries {
        eprintln!(
            "[cyc {:>10}] PC=0x{:08X} OP=0x{:08X} | v0=0x{:08X} a0=0x{:08X} sp=0x{:08X} ra=0x{:08X}",
            e.cycle, e.pc, e.opcode, e.v0, e.a0, e.sp, e.ra
        );
    }
}

fn run(cfg: CliConfig) -> Result<(), String> {
    let mut sys = System::new_headless();
    if cfg.trace_len > 0 {
        sys.enable_trace(cfg.trace_len);
    }

    if let Some(bios_file) = &cfg.bios_path {
        let bios_bytes = fs::read(bios_file)
            .map_err(|e| format!("failed to read BIOS `{}`: {e}", bios_file.display()))?;
        sys.load_bios(&bios_bytes)
            .map_err(|e| format!("invalid BIOS `{}`: {e}", bios_file.display()))?;
    } else {
        sys.install_biosless_kernel_stub();
    }

    if let Some(exe_file) = &cfg.exe_path {
        let exe_bytes = fs::read(exe_file)
            .map_err(|e| format!("failed to read PS-X EXE `{}`: {e}", exe_file.display()))?;
        let parsed = PsxExe::parse(&exe_bytes)
            .map_err(|e| format!("invalid PS-X EXE `{}`: {e:?}", exe_file.display()))?;

        if cfg.bios_path.is_some() {
            sys.set_sideload_exe(parsed);
        } else {
            sys.apply_exe_now(&parsed);
        }
    }

    let mut tty_sink = StdoutTtySink {
        enabled: cfg.show_tty,
    };
    let stop_reason = sys.run_until_cycle(cfg.max_cycles, cfg.stop_at_shell, &mut tty_sink);
    let _ = io::stdout().flush();

    if cfg.trace_len > 0 {
        dump_trace(&sys);
    }

    eprintln!(
        "\n[psx-cli] Stopped ({stop_reason:?}) at cycle={} PC=0x{:08X} POST=0x{:X}",
        sys.scheduler.now(),
        sys.cpu.pc,
        sys.bus.post_code()
    );
    Ok(())
}

fn main() -> ExitCode {
    match CliConfig::parse_args().and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
