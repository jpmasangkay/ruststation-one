use psx_core::{PsxExe, System, TtySink, pad_buttons};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const DEFAULT_MAX_CYCLES: u64 = 250_000_000;
const PAD_CHUNK_CYCLES: u64 = 1_000_000;

#[derive(Copy, Clone, Debug)]
struct PadEvent {
    mask: u16,
    start_cycle: u64,
    end_cycle: u64,
}

fn parse_button_name(name: &str) -> Result<u16, String> {
    match name.to_ascii_lowercase().as_str() {
        "select" => Ok(pad_buttons::SELECT),
        "start" => Ok(pad_buttons::START),
        "up" => Ok(pad_buttons::UP),
        "right" => Ok(pad_buttons::RIGHT),
        "down" => Ok(pad_buttons::DOWN),
        "left" => Ok(pad_buttons::LEFT),
        "l2" => Ok(pad_buttons::L2),
        "r2" => Ok(pad_buttons::R2),
        "l1" => Ok(pad_buttons::L1),
        "r1" => Ok(pad_buttons::R1),
        "triangle" => Ok(pad_buttons::TRIANGLE),
        "circle" => Ok(pad_buttons::CIRCLE),
        "cross" | "x" => Ok(pad_buttons::CROSS),
        "square" => Ok(pad_buttons::SQUARE),
        other => Err(format!("unknown pad button `{other}`")),
    }
}

fn parse_pad_spec(spec: &str) -> Result<PadEvent, String> {
    let (btn_part, range_part) = spec
        .split_once('@')
        .ok_or_else(|| format!("invalid --pad spec `{spec}` (expected <btn>@<start>:<end>)"))?;
    let (start_str, end_str) = range_part
        .split_once(':')
        .ok_or_else(|| format!("invalid --pad range `{spec}` (expected <btn>@<start>:<end>)"))?;

    let mut mask = 0u16;
    for b in btn_part.split('+') {
        mask |= parse_button_name(b)?;
    }
    let start_cycle = start_str
        .parse::<u64>()
        .map_err(|_| format!("invalid --pad start cycle `{start_str}`"))?;
    let end_cycle = end_str
        .parse::<u64>()
        .map_err(|_| format!("invalid --pad end cycle `{end_str}`"))?;
    if end_cycle <= start_cycle {
        return Err(format!("--pad end_cycle must be > start_cycle in `{spec}`"));
    }
    Ok(PadEvent {
        mask,
        start_cycle,
        end_cycle,
    })
}

struct CliConfig {
    bios_path: Option<PathBuf>,
    exe_path: Option<PathBuf>,
    exe_args: Vec<String>,
    show_tty: bool,
    trace_len: usize,
    max_cycles: u64,
    stop_at_shell: bool,
    pad_events: Vec<PadEvent>,
}

impl CliConfig {
    fn parse_args() -> Result<Self, String> {
        let mut bios_path = None;
        let mut exe_path = None;
        let mut exe_args = Vec::new();
        let mut show_tty = false;
        let mut trace_len = 0usize;
        let mut max_cycles = DEFAULT_MAX_CYCLES;
        let mut stop_at_shell = false;
        let mut pad_events = Vec::new();

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
                "--exe-arg" => {
                    let val = args.next().ok_or("--exe-arg requires an argument string")?;
                    exe_args.push(val);
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
                "--pad" => {
                    let val = args
                        .next()
                        .ok_or("--pad requires `<btn>@<start_cycle>:<end_cycle>`")?;
                    pad_events.push(parse_pad_spec(&val)?);
                }
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
            exe_args,
            show_tty,
            trace_len,
            max_cycles,
            stop_at_shell,
            pad_events,
        })
    }
}

fn print_usage() {
    println!(
        "psx-cli — Headless PlayStation 1 Phase 1 Runner & Hardware Test Harness\n\n\
         USAGE:\n  \
           psx-cli [OPTIONS]\n\n\
         OPTIONS:\n  \
           --bios <path>               Load a 512 KB PS1 BIOS ROM (OpenBIOS or retail scph*.bin)\n  \
           --exe <path>                Sideload a PS-X EXE (at 0x80030000 shell hook, or directly if no --bios)\n  \
           --exe-arg <str>             Pass a PS-X loader argv string via Scratchpad 0x1F800000 (repeatable)\n  \
           --tty                       Stream guest BIOS/DUART TTY output live to stdout\n  \
           --trace <n>                 Record and dump the last <n> instructions on exit\n  \
           --max-cycles <n>            Maximum 33.8688 MHz cycles (0 = unbounded, default: {DEFAULT_MAX_CYCLES})\n  \
           --stop-at-shell             Halt cleanly when BIOS reaches shell entry (0x80030000)\n  \
           --pad <btn>@<start>:<end>   Hold Digital Pad button(s) across cycle window (repeatable)\n  \
           -h, --help                  Show this help message"
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
        let _ = out.flush();
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
    if !cfg.exe_args.is_empty() {
        sys.set_exe_args(cfg.exe_args);
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
    let cycle_limit = if cfg.max_cycles == 0 {
        u64::MAX
    } else {
        cfg.max_cycles
    };

    let stop_reason = if cfg.pad_events.is_empty() {
        sys.run_until_cycle(cycle_limit, cfg.stop_at_shell, &mut tty_sink)
    } else {
        let mut reason = psx_core::system::StepStopReason::MaxCyclesReached;
        while sys.scheduler.now() < cycle_limit {
            let now = sys.scheduler.now();
            let mut active_mask = 0u16;
            let mut next_edge = cycle_limit.min(now.saturating_add(PAD_CHUNK_CYCLES));
            for ev in &cfg.pad_events {
                if now >= ev.start_cycle && now < ev.end_cycle {
                    active_mask |= ev.mask;
                    next_edge = next_edge.min(ev.end_cycle);
                } else if now < ev.start_cycle {
                    next_edge = next_edge.min(ev.start_cycle);
                }
            }
            sys.bus.set_pad_buttons(active_mask);
            reason = sys.run_until_cycle(next_edge, cfg.stop_at_shell, &mut tty_sink);
            if reason == psx_core::system::StepStopReason::ReachedShell {
                break;
            }
        }
        reason
    };

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
