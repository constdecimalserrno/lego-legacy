//! The debugger.
//!
//! A person drives the same machine a route replays: one instruction at a
//! time, stopping where they say. Nothing typed here reaches the guest, so a
//! session spent stepping through a title changes nothing about what the run
//! would have done on its own.

use crate::cli::Cli;
use crate::cpu::{REG_NAMES, SEG_NAMES};
use crate::guest::GuestMem;
use crate::host::Host;
use crate::machine::{Finish, Machine, Outcome};
use crate::runlog::RunLog;
use crate::Title;
use std::io::{BufRead, Write};

/// What the debugger was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    Step(u64),
    Regs,
    Mem {
        addr: u32,
        len: u32,
    },
    Break(u32),
    UntilFlip(u64),
    UntilCursor(u64),
    /// Stop as soon as the guest is running somewhere that is not its own
    /// image: the first instruction of a run that has gone wild.
    UntilWild,
    Continue,
    Snapshot,
    Help,
    Quit,
    Unknown(String),
}

fn number(text: &str) -> Option<u64> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        text.parse().ok()
    }
}

fn parse(line: &str) -> Option<Command> {
    let mut words = line.split_whitespace();
    let verb = words.next()?;
    Some(match verb {
        "step" | "s" => Command::Step(words.next().and_then(number).unwrap_or(1)),
        "regs" | "r" => Command::Regs,
        "mem" | "m" => {
            let addr = words.next().and_then(number).unwrap_or(0);
            let len = words.next().and_then(number).unwrap_or(16);
            Command::Mem {
                addr: u32::try_from(addr).unwrap_or(0),
                len: u32::try_from(len).unwrap_or(16),
            }
        }
        "break" | "b" => Command::Break(
            words
                .next()
                .and_then(number)
                .and_then(|a| u32::try_from(a).ok())
                .unwrap_or(0),
        ),
        "until" | "u" => match words.next() {
            Some("flip") => Command::UntilFlip(words.next().and_then(number).unwrap_or(0)),
            Some("cursor") => Command::UntilCursor(words.next().and_then(number).unwrap_or(0)),
            Some("wild") => Command::UntilWild,
            _ => Command::Unknown(line.trim().to_owned()),
        },
        "continue" | "c" => Command::Continue,
        "snapshot" => Command::Snapshot,
        "help" | "?" => Command::Help,
        "quit" | "q" | "exit" => Command::Quit,
        other => Command::Unknown(other.to_owned()),
    })
}

const HELP: &str = "\
step [n]           retire n instructions (default 1)
regs               the processor, one fact per line
mem <addr> [len]   guest bytes, as hexadecimal
break <addr>       stop when the instruction pointer reaches this
until flip <n>     stop when the guest has presented n frames
until cursor <n>   stop when n instructions have retired
until wild         stop where the guest first runs outside its own image
continue           run until the guest stops, or a break is reached
snapshot           write the machine's state
quit               end the run";

/// Where the machine stands, in one line.
fn where_it_is(machine: &Machine) -> String {
    format!(
        "cursor {} eip {:#010x} flips {}",
        machine.clock.cursor(),
        machine.cpu.eip,
        machine.flips()
    )
}

/// Drive a machine from a script of commands, writing what it says.
///
/// # Errors
/// When the machine cannot be built or the title cannot be loaded.
pub fn drive(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<Finish, Box<dyn std::error::Error>> {
    let mut log = RunLog::create(title.key, cli.trace)?;
    let mut machine = Machine::new(title, host)?;
    if cli.stub {
        machine.load_stub()?;
    } else {
        machine.load_title(title)?;
        machine
            .tree
            .set_profile(crate::run::profile_dir(title, cli));
    }

    // The debugger drives the same machine a route replays, so a person can
    // walk into the middle of a recorded session and look around.
    if let Some(path) = &cli.route {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("no route at {}: {e}", path.display()))?;
        let route =
            crate::route::Route::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = writeln!(output, "route {} events {}", path.display(), route.len());
        machine.route = Some(route);
    }

    let mut breaks: Vec<u32> = Vec::new();
    let mut outcome: Option<Outcome> = None;
    let _ = writeln!(output, "{}", where_it_is(&machine));

    let mut line = String::new();
    loop {
        line.clear();
        let _ = write!(output, "> ");
        let _ = output.flush();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        let Some(command) = parse(&line) else {
            continue;
        };
        match command {
            Command::Quit => break,
            Command::Help => {
                let _ = writeln!(output, "{HELP}");
            }
            Command::Regs => {
                for (name, value) in REG_NAMES.iter().zip(machine.cpu.r) {
                    let _ = writeln!(output, "{name} {value:#010x}");
                }
                let _ = writeln!(output, "eip {:#010x}", machine.cpu.eip);
                let _ = writeln!(output, "eflags {:#010x}", machine.cpu.flags.to_eflags());
                for (i, (name, value)) in SEG_NAMES.iter().zip(machine.cpu.seg).enumerate() {
                    // A segment with a base of its own is `fs` over the thread
                    // information block (#54), and what it adds to an address
                    // is the only interesting thing about it here.
                    let base = machine.cpu.seg_base[i];
                    if base == 0 {
                        let _ = writeln!(output, "{name} {value:#06x}");
                    } else {
                        let _ = writeln!(output, "{name} {value:#06x} base {base:#010x}");
                    }
                }
                let _ = writeln!(output, "cursor {}", machine.clock.cursor());
            }
            Command::Mem { addr, len } => match machine.arena.slice(addr, len as usize) {
                Ok(bytes) => {
                    for (row, chunk) in bytes.chunks(16).enumerate() {
                        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
                        let _ = writeln!(
                            output,
                            "{:#010x} {}",
                            addr + (row as u32) * 16,
                            hex.join(" ")
                        );
                    }
                }
                Err(fault) => {
                    let _ = writeln!(output, "{fault}");
                }
            },
            Command::Break(addr) => {
                breaks.push(addr);
                let _ = writeln!(output, "break at {addr:#010x}");
            }
            Command::Snapshot => {
                let _ = writeln!(output, "snapshots are not built yet (#13)");
            }
            Command::Unknown(what) => {
                let _ = writeln!(output, "no such command: {what}");
            }
            Command::Step(n) => {
                for _ in 0..n {
                    if outcome.is_some() {
                        break;
                    }
                    outcome = machine.tick(host, &mut log);
                }
                let _ = writeln!(output, "{}", where_it_is(&machine));
                if let Some(o) = &outcome {
                    let _ = writeln!(output, "stopped: {o}");
                }
            }
            Command::Continue
            | Command::UntilFlip(_)
            | Command::UntilCursor(_)
            | Command::UntilWild => {
                let target = command.clone();
                while outcome.is_none() {
                    // A break is checked before the instruction at it runs, so
                    // stopping at an address means standing on it.
                    if breaks.contains(&machine.cpu.eip) && machine.clock.cursor() > 0 {
                        let _ = writeln!(output, "break {:#010x}", machine.cpu.eip);
                        break;
                    }
                    match &target {
                        Command::UntilFlip(n) if machine.flips() >= *n => break,
                        Command::UntilCursor(n) if machine.clock.cursor() >= *n => break,
                        Command::UntilWild if machine.is_wild() => {
                            let _ = writeln!(output, "wild at {:#010x}", machine.cpu.eip);
                            break;
                        }
                        _ => {}
                    }
                    outcome = machine.tick(host, &mut log);
                }
                let _ = writeln!(output, "{}", where_it_is(&machine));
                if let Some(o) = &outcome {
                    let _ = writeln!(output, "stopped: {o}");
                }
            }
        }
    }

    let finish = machine.finish(outcome.unwrap_or(Outcome::Closed));
    log.line(&format!("end {} cursor {}", finish.outcome, finish.cursor));
    Ok(finish)
}

/// Drive a machine from a script and hand back everything it printed.
///
/// What a test uses, and what makes the debugger checkable without a terminal.
///
/// # Errors
/// When the machine cannot be built or the title cannot be loaded.
pub fn transcript(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
    script: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut input = std::io::Cursor::new(script.as_bytes().to_vec());
    let mut output: Vec<u8> = Vec::new();
    let mut reader = std::io::BufReader::new(&mut input);
    drive(title, cli, host, &mut reader, &mut output)?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}
