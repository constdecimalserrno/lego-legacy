//! `lego-legacy`: the one front door.
//!
//! Lists the playable titles on `core::roster::ROSTER`, shows which discs and
//! audio pressings are ready, resolves full or unique short title names, and
//! forwards flags unchanged to the chosen title's binary.

mod term;

use lego_legacy_core::roster::{self, Entry, Found};
use std::fmt::Write as _;
use std::io::{BufRead, Write};
use std::process::{Command, ExitCode};

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const RED_BOLD: &str = "\x1b[1;31m";
const GREEN_BOLD: &str = "\x1b[1;32m";
const YELLOW: &str = "\x1b[33m";
const YELLOW_BOLD: &str = "\x1b[1;33m";
const BLUE_BOLD: &str = "\x1b[1;34m";
const WHITE_BOLD: &str = "\x1b[1;97m";

fn sgr(color: bool, code: &'static str) -> &'static str {
    if color {
        code
    } else {
        ""
    }
}

#[derive(Debug, PartialEq, Eq)]
enum TitleError {
    Unknown(String),
    Ambiguous {
        query: String,
        matches: Vec<&'static str>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum Action {
    Help,
    Launch {
        key: &'static str,
        flags: Vec<String>,
    },
    Pick {
        flags: Vec<String>,
    },
    Error(TitleError),
}

struct AudioNote {
    text: String,
    missing: bool,
}

struct TitleStatus {
    entry: &'static Entry,
    found: Found,
    notes: Vec<AudioNote>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let color = term::color_enabled();

    match parse_args(&args) {
        Action::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Action::Launch { key, flags } => start(key, &flags),
        Action::Error(err) => {
            match err {
                TitleError::Unknown(query) => {
                    eprintln!("lego-legacy: no supported title is called `{query}`");
                }
                TitleError::Ambiguous { query, matches } => {
                    eprintln!(
                        "lego-legacy: ambiguous title `{query}` (matches: {})",
                        matches.join(", ")
                    );
                }
            }
            let statuses = probe_roster();
            print!("{}", render_banner(color));
            print!("{}", render_menu(&statuses, None, color));
            ExitCode::FAILURE
        }
        Action::Pick { flags } => pick_and_start(&flags, color),
    }
}

fn print_help() {
    println!("usage: lego-legacy [<title>] [flags...]");
    println!("       lego-legacy [flags...] [<title>] [flags...]");
    println!();
    println!("with no title, opens the interactive title picker and forwards any flags");
    println!("to the chosen title's own binary, unchanged.");
    println!();
    println!("titles (full key or unique short name, e.g. `racers`, `chess`, `hotwheels`):");
    for entry in roster::ROSTER {
        println!("  {:<32} {}", entry.key, entry.name);
    }
}

/// Mirrors value-taking flags in `core::cli` so leading flags like `--route foo.route`
/// do not treat their value as a title name.
fn flag_takes_value(flag: &str) -> bool {
    matches!(
        flag,
        "--route"
            | "--data-root"
            | "--trace"
            | "--screenshot"
            | "--screenshot-every"
            | "--snapshot-every"
            | "--fps-window"
            | "--control-unix"
            | "--token"
            | "--guest-args"
            | "--control-mode"
    )
}

/// Parse launcher arguments, accepting optional flags before or after a title.
///
/// Once a positional title is resolved, every remaining argument after it is
/// forwarded verbatim to the title's binary (including `--help`).
fn parse_args(args: &[String]) -> Action {
    let mut flags = Vec::new();
    let mut saw_help = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--help" || arg == "-h" {
            if i == 0 {
                return Action::Help;
            }
            saw_help = true;
            flags.push(arg.clone());
            i += 1;
            continue;
        }
        if arg.starts_with('-') {
            flags.push(arg.clone());
            let takes_next = args.get(i + 1).is_some_and(|val| {
                flag_takes_value(arg)
                    || (arg == "--control" && !val.starts_with('-') && val.contains(':'))
            });
            if takes_next {
                flags.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        return match resolve_title(arg) {
            Ok(entry) => {
                flags.extend_from_slice(&args[i + 1..]);
                Action::Launch {
                    key: entry.key,
                    flags,
                }
            }
            Err(err) => Action::Error(err),
        };
    }

    if saw_help {
        Action::Help
    } else {
        Action::Pick { flags }
    }
}

/// Resolve a full `ROSTER` key or a unique short/suffix name:
/// exact key -> exact `lego-`-stripped suffix -> hyphen-boundary prefix or suffix.
fn resolve_title(query: &str) -> Result<&'static Entry, TitleError> {
    let q = query.to_ascii_lowercase();
    if q.is_empty() {
        return Err(TitleError::Unknown(query.to_owned()));
    }

    if let Some(entry) = roster::ROSTER
        .iter()
        .copied()
        .find(|e| e.key.eq_ignore_ascii_case(&q))
    {
        return Ok(entry);
    }

    if let Some(entry) = roster::ROSTER
        .iter()
        .copied()
        .find(|e| e.key.strip_prefix("lego-") == Some(q.as_str()))
    {
        return Ok(entry);
    }

    let prefix_pat = format!("{q}-");
    let tail_pat = format!("-{q}");
    let boundary_matches: Vec<&'static Entry> = roster::ROSTER
        .iter()
        .copied()
        .filter(|e| {
            let short = e.key.strip_prefix("lego-").unwrap_or(e.key);
            e.key.starts_with(&prefix_pat)
                || e.key.ends_with(&tail_pat)
                || short.starts_with(&prefix_pat)
        })
        .collect();
    match boundary_matches.as_slice() {
        [only] => Ok(only),
        [] => Err(TitleError::Unknown(query.to_owned())),
        many => Err(TitleError::Ambiguous {
            query: query.to_owned(),
            matches: many.iter().map(|e| e.key).collect(),
        }),
    }
}

fn probe_roster() -> Vec<TitleStatus> {
    roster::ROSTER
        .iter()
        .copied()
        .map(|entry| {
            let found = roster::look_for(entry);
            let mut notes = Vec::new();
            if let Some(pressing) = entry.red_book {
                match lego_legacy_core::disc::open_red_book(entry.image.disc_dir, pressing) {
                    Err(e) => notes.push(AudioNote {
                        text: format!("music: {pressing} Red Book missing ({e})"),
                        missing: true,
                    }),
                    Ok(_) if pressing != entry.image.pressing => notes.push(AudioNote {
                        text: format!("music: {pressing} (Red Book)"),
                        missing: false,
                    }),
                    Ok(_) => {}
                }
            }
            if let Some(first) = entry.bank.first() {
                match roster::find_bank(entry.bank) {
                    Ok((place, _)) => {
                        let file = place.path.rsplit('/').next().unwrap_or(place.path);
                        notes.push(AudioNote {
                            text: format!("music: {file} from {}", place.pressing),
                            missing: false,
                        });
                    }
                    Err(why) => {
                        let file = first.path.rsplit('/').next().unwrap_or(first.path);
                        notes.push(AudioNote {
                            text: format!("music: needs {file} ({why})"),
                            missing: true,
                        });
                    }
                }
            }
            TitleStatus {
                entry,
                found,
                notes,
            }
        })
        .collect()
}

fn render_banner(color: bool) -> String {
    const STUDS: &str = "     .===.  .===.  .===.  .===.";
    const BORDER: &str = "+------------------------------+";
    const LEGO_ROWS: [&str; 4] = [
        "   _      ___    ___    ___   ",
        "  | |    | __|  / __|  / _ \\  ",
        "  | |__  | _|  | (_ | | (_) | ",
        "  |____| |___|  \\___|  \\___/  ",
    ];
    const LEGACY_ROWS: [&str; 4] = [
        " _      ___    ___     _      ___  __   __",
        "| |    | __|  / __|   /_\\    / __| \\ \\ / /",
        "| |__  | _|  | (_ |  / _ \\  | (__   \\ V /",
        "|____| |___|  \\___| /_/ \\_\\  \\___|   |_|",
    ];
    const TAGLINE: &str = "bring your own disc  |  zero crates";

    let (red, white, yellow, blue, rst) = (
        sgr(color, RED_BOLD),
        sgr(color, WHITE_BOLD),
        sgr(color, YELLOW_BOLD),
        sgr(color, BLUE_BOLD),
        sgr(color, RESET),
    );
    let mut out = String::new();
    let _ = writeln!(out, "{red}{STUDS}{rst}");
    let _ = writeln!(out, "  {red}{BORDER}{rst}");
    for (lego, legacy) in LEGO_ROWS.iter().zip(LEGACY_ROWS.iter()) {
        let _ = writeln!(
            out,
            "  {red}|{rst}{white}{lego}{rst}{red}|{rst}  {yellow}{legacy}{rst}"
        );
    }
    let _ = writeln!(out, "  {red}{BORDER}{rst}  {blue}{TAGLINE}{rst}");
    out.push('\n');
    out
}

fn format_found(found: &Found, color: bool) -> String {
    let (green, yellow, red, dim, rst) = (
        sgr(color, GREEN_BOLD),
        sgr(color, YELLOW),
        sgr(color, RED_BOLD),
        sgr(color, DIM),
        sgr(color, RESET),
    );
    match found {
        Found::Runnable { pressing } => format!("{green}[ready]{rst}    {dim}{pressing}{rst}"),
        Found::Absent { looked_at } => {
            format!("{yellow}[missing]{rst}  {dim}no rip at {looked_at}{rst}")
        }
        Found::Wrapped { pressing, scheme } => {
            format!("{red}[wrapped]{rst}  {pressing} is wrapped in {scheme} and will not be run")
        }
        Found::WrongBytes { pressing, found } => {
            let prefix = &found[..16.min(found.len())];
            format!("{red}[mismatch]{rst} {pressing} holds different bytes ({prefix}...)")
        }
        Found::Unreadable { why } => format!("{red}[error]{rst}    {why}"),
    }
}

fn render_menu(statuses: &[TitleStatus], selected: Option<usize>, color: bool) -> String {
    let column = statuses
        .iter()
        .map(|s| s.entry.name.len())
        .max()
        .unwrap_or(0);
    let (yellow_b, white_b, yellow, dim, rst) = (
        sgr(color, YELLOW_BOLD),
        sgr(color, WHITE_BOLD),
        sgr(color, YELLOW),
        sgr(color, DIM),
        sgr(color, RESET),
    );
    let mut out = String::new();
    for (i, status) in statuses.iter().enumerate() {
        let badge = format_found(&status.found, color);
        if selected == Some(i) {
            let _ = writeln!(
                out,
                "  {yellow_b}>{rst} {white_b}{}. {:<column$}{rst}  {badge}",
                i + 1,
                status.entry.name
            );
        } else {
            let prefix = if selected.is_some() { "    " } else { "  " };
            let _ = writeln!(
                out,
                "{prefix}{}. {:<column$}  {badge}",
                i + 1,
                status.entry.name
            );
        }
        let note_indent = if selected.is_some() {
            "       "
        } else {
            "     "
        };
        for note in &status.notes {
            let style = if note.missing { yellow } else { dim };
            let _ = writeln!(out, "{note_indent}{style}{}{rst}", note.text);
        }
    }
    out
}

/// Count visual terminal rows occupied by `content` on a `cols`-wide terminal,
/// ignoring ANSI CSI sequences so wrapped lines rewind cleanly on redraw.
#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "x86",
            target_arch = "arm",
            target_arch = "riscv64"
        )
    ),
    test
))]
fn visual_rows(content: &str, cols: usize) -> usize {
    let width = cols.max(1);
    content
        .lines()
        .map(|line| {
            let mut visible = 0usize;
            let mut chars = line.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\x1b' && chars.peek() == Some(&'[') {
                    chars.next();
                    for next in chars.by_ref() {
                        if (0x40..=0x7e).contains(&(next as u32)) {
                            break;
                        }
                    }
                } else {
                    visible += 1;
                }
            }
            visible.max(1).div_ceil(width)
        })
        .sum()
}

fn pick_and_start(flags: &[String], color: bool) -> ExitCode {
    let statuses = probe_roster();
    print!("{}", render_banner(color));
    if !flags.is_empty() {
        let (dim, blue, rst) = (sgr(color, DIM), sgr(color, BLUE_BOLD), sgr(color, RESET));
        println!("  {dim}flags:{rst} {blue}{}{rst}\n", flags.join(" "));
    }

    let first_ready = statuses
        .iter()
        .position(|s| matches!(s.found, Found::Runnable { .. }));
    let Some(initial) = first_ready else {
        print!("{}", render_menu(&statuses, None, color));
        println!("\nNothing can be started yet. Rip your own discs into game-data/ first.");
        return ExitCode::FAILURE;
    };

    #[cfg(any(
        target_os = "macos",
        all(
            target_os = "linux",
            any(
                target_arch = "x86_64",
                target_arch = "aarch64",
                target_arch = "x86",
                target_arch = "arm",
                target_arch = "riscv64"
            )
        )
    ))]
    if let Some(code) = run_interactive(&statuses, initial, flags, color) {
        return code;
    }
    let _ = initial;
    run_line_mode(&statuses, flags, color)
}

#[cfg(any(
    target_os = "macos",
    all(
        target_os = "linux",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "x86",
            target_arch = "arm",
            target_arch = "riscv64"
        )
    )
))]
fn run_interactive(
    statuses: &[TitleStatus],
    initial: usize,
    flags: &[String],
    color: bool,
) -> Option<ExitCode> {
    let guard = term::unix::RawModeGuard::enter()?;

    let count = statuses.len();
    let mut cursor = initial;
    let mut notice: Option<String> = None;
    let mut rendered_rows: usize = 0;
    let (dim, red, bold, rst) = (
        sgr(color, DIM),
        sgr(color, RED_BOLD),
        sgr(color, "\x1b[1m"),
        sgr(color, RESET),
    );

    enum Outcome {
        Launch(&'static Entry),
        Quit,
        Interrupt,
    }

    let outcome = loop {
        let mut content = render_menu(statuses, Some(cursor), color);
        let _ = writeln!(
            content,
            "\n  {dim}up/down or j/k: move   1-{count}: pick   enter: play   q/esc: quit{rst}"
        );
        if let Some(msg) = &notice {
            let _ = writeln!(content, "  {red}{msg}{rst}");
        }

        let mut frame = String::new();
        if rendered_rows > 0 {
            let _ = write!(frame, "\x1b[{rendered_rows}A\r\x1b[J");
        }
        frame.push_str(&content);
        rendered_rows = visual_rows(&content, term::unix::terminal_cols());

        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(frame.as_bytes());
        let _ = stdout.flush();

        let Some(key) = term::unix::read_key() else {
            break Outcome::Quit;
        };
        notice = None;
        match key {
            term::Key::Up => {
                cursor = if cursor == 0 { count - 1 } else { cursor - 1 };
            }
            term::Key::Down => {
                cursor = (cursor + 1) % count;
            }
            term::Key::Enter | term::Key::Digit(_) => {
                if let term::Key::Digit(idx) = key {
                    if idx >= count {
                        continue;
                    }
                    cursor = idx;
                }
                match &statuses[cursor].found {
                    Found::Runnable { .. } => break Outcome::Launch(statuses[cursor].entry),
                    other => {
                        notice = Some(format!(
                            "{} cannot be started: {other}",
                            statuses[cursor].entry.name
                        ));
                    }
                }
            }
            term::Key::Quit => break Outcome::Quit,
            term::Key::Interrupt => break Outcome::Interrupt,
            term::Key::Other => {}
        }
    };

    // Restore terminal mode and cursor visibility before printing launch output
    // or spawning the title binary.
    drop(guard);

    match outcome {
        Outcome::Launch(entry) => {
            if flags.is_empty() {
                println!("\n  {bold}Launching {}...{rst}", entry.name);
            } else {
                println!(
                    "\n  {bold}Launching {}{rst} {dim}({}){rst}...",
                    entry.name,
                    flags.join(" ")
                );
            }
            Some(start(entry.key, flags))
        }
        Outcome::Quit => Some(ExitCode::SUCCESS),
        Outcome::Interrupt => Some(ExitCode::from(130)),
    }
}

fn run_line_mode(statuses: &[TitleStatus], flags: &[String], color: bool) -> ExitCode {
    print!("{}", render_menu(statuses, None, color));
    print!("\nWhich one? ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return ExitCode::FAILURE;
    }
    let choice: usize = match line.trim().parse::<usize>() {
        Ok(n) if n >= 1 && n <= statuses.len() => n - 1,
        _ => {
            eprintln!("lego-legacy: not one of the numbers above");
            return ExitCode::FAILURE;
        }
    };
    let chosen = &statuses[choice];
    match &chosen.found {
        Found::Runnable { .. } => start(chosen.entry.key, flags),
        other => {
            eprintln!(
                "lego-legacy: {} cannot be started: {other}",
                chosen.entry.name
            );
            ExitCode::FAILURE
        }
    }
}

/// Start a title's own binary, passing on whatever we were given.
fn start(key: &str, flags: &[String]) -> ExitCode {
    // The title's binary sits beside this one, which is where cargo puts it
    // and where an install would too; on Windows it is `<title>.exe` (#107).
    let beside = std::env::current_exe().ok().and_then(|p| {
        p.parent()
            .map(|d| d.join(format!("{key}{}", std::env::consts::EXE_SUFFIX)))
    });
    let program = match beside {
        Some(path) if path.is_file() => path,
        _ => std::path::PathBuf::from(key),
    };
    match Command::new(&program).args(flags).status() {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(e) => {
            eprintln!("lego-legacy: could not start {}: {e}", program.display());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    fn strip_ansi(input: &str) -> String {
        let mut out = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if (0x40..=0x7e).contains(&(next as u32)) {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn resolves_exact_keys_and_short_names() {
        let cases = [
            ("lego-rock-raiders", "lego-rock-raiders"),
            ("rock-raiders", "lego-rock-raiders"),
            ("rock", "lego-rock-raiders"),
            ("raiders", "lego-rock-raiders"),
            (
                "hotwheels-stunt-track-driver",
                "hotwheels-stunt-track-driver",
            ),
            ("hotwheels", "hotwheels-stunt-track-driver"),
            ("lego-alpha-team", "lego-alpha-team"),
            ("alpha-team", "lego-alpha-team"),
            ("alpha", "lego-alpha-team"),
            ("lego-island", "lego-island"),
            ("island", "lego-island"),
            ("lego-chess", "lego-chess"),
            ("chess", "lego-chess"),
            ("lego-racers", "lego-racers"),
            ("racers", "lego-racers"),
            ("RACERS", "lego-racers"),
        ];
        for (query, expected) in cases {
            let entry = resolve_title(query).unwrap_or_else(|e| panic!("`{query}` failed: {e:?}"));
            assert_eq!(entry.key, expected, "query `{query}`");
        }
    }

    #[test]
    fn refuses_ambiguous_unknown_and_tool_binaries() {
        assert_eq!(
            resolve_title("").map(|e| e.key),
            Err(TitleError::Unknown(String::new()))
        );
        assert_eq!(
            resolve_title("unknown").map(|e| e.key),
            Err(TitleError::Unknown("unknown".to_owned()))
        );
        assert_eq!(
            resolve_title("lego-ctrl").map(|e| e.key),
            Err(TitleError::Unknown("lego-ctrl".to_owned()))
        );
        assert_eq!(
            resolve_title("lego-mcp").map(|e| e.key),
            Err(TitleError::Unknown("lego-mcp".to_owned()))
        );
        assert_eq!(
            resolve_title("r").map(|e| e.key),
            Err(TitleError::Unknown("r".to_owned()))
        );

        let err = resolve_title("lego").map(|e| e.key).unwrap_err();
        match err {
            TitleError::Ambiguous { query, matches } => {
                assert_eq!(query, "lego");
                assert!(matches.contains(&"lego-racers"));
                assert!(matches.contains(&"lego-chess"));
                assert!(matches.len() >= 5);
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn parses_leading_trailing_and_value_taking_flags() {
        assert_eq!(parse_args(&s(&[])), Action::Pick { flags: vec![] });
        assert_eq!(parse_args(&s(&["--help"])), Action::Help);
        assert_eq!(parse_args(&s(&["-h"])), Action::Help);
        assert_eq!(parse_args(&s(&["--audio", "--help"])), Action::Help);
        assert_eq!(
            parse_args(&s(&["--audio"])),
            Action::Pick {
                flags: s(&["--audio"])
            }
        );
        assert_eq!(
            parse_args(&s(&["--audio", "--record"])),
            Action::Pick {
                flags: s(&["--audio", "--record"])
            }
        );
        assert_eq!(
            parse_args(&s(&["--route", "foo.route"])),
            Action::Pick {
                flags: s(&["--route", "foo.route"])
            }
        );
        assert_eq!(
            parse_args(&s(&["--control", "127.0.0.1:8080"])),
            Action::Pick {
                flags: s(&["--control", "127.0.0.1:8080"])
            }
        );
        assert_eq!(
            parse_args(&s(&["--control", "racers"])),
            Action::Launch {
                key: "lego-racers",
                flags: s(&["--control"]),
            }
        );
        assert_eq!(
            parse_args(&s(&["racers", "--audio"])),
            Action::Launch {
                key: "lego-racers",
                flags: s(&["--audio"]),
            }
        );
        assert_eq!(
            parse_args(&s(&["racers", "--help"])),
            Action::Launch {
                key: "lego-racers",
                flags: s(&["--help"]),
            }
        );
        assert_eq!(
            parse_args(&s(&["--audio", "racers", "--record"])),
            Action::Launch {
                key: "lego-racers",
                flags: s(&["--audio", "--record"]),
            }
        );
        assert_eq!(
            parse_args(&s(&["--audio", "--route", "m1.route", "hotwheels"])),
            Action::Launch {
                key: "hotwheels-stunt-track-driver",
                flags: s(&["--audio", "--route", "m1.route"]),
            }
        );
        assert_eq!(
            parse_args(&s(&["--audio", "nope"])),
            Action::Error(TitleError::Unknown("nope".to_owned()))
        );
    }

    #[test]
    fn banner_is_clean_ascii_within_80_columns_with_and_without_color() {
        let plain = render_banner(false);
        let colored = render_banner(true);

        assert!(!plain.contains('\x1b'));
        assert!(colored.contains("\x1b[1;31m"));
        assert!(colored.contains("\x1b[1;33m"));
        assert!(colored.contains("\x1b[1;34m"));
        assert!(colored.contains("\x1b[1;97m"));
        assert_eq!(strip_ansi(&colored), plain);

        for line in plain.lines() {
            assert!(
                line.is_ascii(),
                "banner line must be pure 7-bit ASCII: {line:?}"
            );
            assert!(
                line.len() <= 80,
                "banner line exceeds 80 cols ({}): {line}",
                line.len()
            );
        }
    }

    #[test]
    fn menu_renders_roster_badges_selection_and_notes() {
        let statuses = vec![
            TitleStatus {
                entry: &roster::ROCK_RAIDERS,
                found: Found::Runnable {
                    pressing: "masterpiece-rock-raiders".to_owned(),
                },
                notes: vec![AudioNote {
                    text: "music: rock-raiders (Red Book)".to_owned(),
                    missing: false,
                }],
            },
            TitleStatus {
                entry: &roster::LEGO_CHESS,
                found: Found::Absent {
                    looked_at: "game-data/chess/masterpiece-chess/masterpiece-chess.iso".to_owned(),
                },
                notes: vec![AudioNote {
                    text: "music: needs GM16.DLS (no rip)".to_owned(),
                    missing: true,
                }],
            },
        ];

        let plain = render_menu(&statuses, Some(0), false);
        assert!(!plain.contains('\x1b'));
        assert!(plain.contains("> 1. LEGO Rock Raiders"));
        assert!(plain.contains("[ready]"));
        assert!(plain.contains("[missing]"));
        assert!(plain.contains("music: rock-raiders (Red Book)"));
        assert!(plain.contains("music: needs GM16.DLS (no rip)"));

        let colored = render_menu(&statuses, Some(0), true);
        assert_eq!(strip_ansi(&colored), plain);
    }

    #[test]
    fn visual_rows_accounts_for_ansi_empty_lines_and_wrapping() {
        assert_eq!(visual_rows("12345678\n", 8), 1);
        assert_eq!(visual_rows("123456789\n", 8), 2);
        assert_eq!(visual_rows("a\n\nb\n", 80), 3);
        assert_eq!(visual_rows("\x1b[1;31m12345678\x1b[0m\n", 8), 1);
        assert_eq!(visual_rows("\x1b[1;31m123456789\x1b[0m\n", 8), 2);
        assert_eq!(visual_rows("line one\nline two\n", 80), 2);
    }
}
