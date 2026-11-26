//! The run seam: the one way a title is run.
//!
//! A title crate's `main` is a call to [`main`] and nothing else. Tests reach
//! the same seam through [`crate::testkit`], so what a test runs is what a
//! player runs.

use crate::cli::{Cli, CliError};
use crate::control::{
    service_flip_boundary, write_session_registry, ControlConfig, ControlHost, ExecMode,
    HttpContext, HttpServer, InputOwner, SessionRegistryGuard, StdioBridge,
};
use crate::host::{headless::Headless, telemetry::TelemetryHost, Host};
use crate::machine::{Finish, Machine, Outcome};
use crate::runlog::RunLog;
use crate::title::TitleGuide;
use crate::Title;
use std::process::ExitCode;
use std::sync::Arc;

/// Where a run keeps what its guest writes: beside the pressing it was made
/// from, unless the run was told to keep it somewhere of its own, which is
/// how a test gives one run a machine nothing has been written on.
pub(crate) fn profile_dir(title: &Title, cli: &Cli) -> std::path::PathBuf {
    cli.data_root.as_ref().map_or_else(
        || crate::disc::profile_dir(title.image.disc_dir),
        |root| root.join("profile"),
    )
}

/// Run one title on one host, from a parsed command line.
///
/// # Errors
/// When the machine cannot be built, or the run cannot be set up. A guest that
/// stops at a wall is not an error here: it is an [`Outcome`].
pub fn run(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
    budget: u64,
) -> Result<(Finish, RunLog), Box<dyn std::error::Error>> {
    run_with_guide(title, &TitleGuide::EMPTY, cli, host, budget)
}

/// Run one title with its static [`TitleGuide`] metadata on one host (#311).
///
/// # Errors
/// As [`run`].
pub fn run_with_guide(
    title: &'static Title,
    guide: &'static TitleGuide,
    cli: &Cli,
    host: &mut dyn Host,
    budget: u64,
) -> Result<(Finish, RunLog), Box<dyn std::error::Error>> {
    run_digested_with_guide(title, guide, cli, host, budget).map(|(finish, log, _)| (finish, log))
}

/// The run seam, with the digest a determinism gate compares.
///
/// # Errors
/// As [`run`].
pub fn run_digested(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
    budget: u64,
) -> Result<(Finish, RunLog, String), Box<dyn std::error::Error>> {
    run_digested_with_guide(title, &TitleGuide::EMPTY, cli, host, budget)
}

/// The digested run seam with static [`TitleGuide`] metadata threaded into the
/// control plane (`ControlConfig` / `ControlHost` / `HttpContext`, #311).
///
/// # Errors
/// As [`run`].
pub fn run_digested_with_guide(
    title: &'static Title,
    guide: &'static TitleGuide,
    cli: &Cli,
    host: &mut dyn Host,
    budget: u64,
) -> Result<(Finish, RunLog, String), Box<dyn std::error::Error>> {
    if cli.control_enabled() {
        let config = ControlConfig {
            title: title.key,
            mode: cli.control_mode.unwrap_or(ExecMode::Step),
            owner: if cli.has_control_transport() {
                InputOwner::Agent
            } else {
                InputOwner::Human
            },
            start_paused: cli.paused,
            step_uncapped: true,
            pace: 1.0,
            pointer_speed: title.pointer_speed,
            flip_cost: title.flip_cost,
            guide,
        };
        let mut control = ControlHost::new(host, config).with_deferred_pause(true);
        let bus = control.bus();
        bus.set_machine_attached(true);

        let token = cli.token.clone().or_else(|| {
            std::env::var("LEGO_LEGACY_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
        });
        let http_ctx = Arc::new(HttpContext::new(bus.clone(), title, token).with_guide(guide));

        let mut http_server = if let Some(bind) = &cli.control_http {
            let server = HttpServer::start(bind, Arc::clone(&http_ctx))?;
            eprintln!("lego-legacy: control {}", server.url());
            Some(server)
        } else {
            None
        };

        #[cfg(unix)]
        let mut unix_server = if let Some(path) = &cli.control_unix {
            let server = crate::control::UnixServer::start(path.clone(), Arc::clone(&http_ctx))?;
            eprintln!("lego-legacy: control {}", server.url());
            Some(server)
        } else {
            None
        };
        #[cfg(not(unix))]
        if cli.control_unix.is_some() {
            return Err("--control-unix is only supported on Unix hosts".into());
        }

        let _session_guard: SessionRegistryGuard = if let Some(server) = &http_server {
            write_session_registry(
                title.key,
                &server.url(),
                Some(&server.watch_url()),
                !cli.headless,
            )
        } else {
            #[cfg(unix)]
            {
                if let Some(server) = &unix_server {
                    write_session_registry(title.key, &server.url(), None, !cli.headless)
                } else {
                    SessionRegistryGuard::empty()
                }
            }
            #[cfg(not(unix))]
            {
                SessionRegistryGuard::empty()
            }
        };

        if cli.control_stdio {
            StdioBridge::spawn(bus.clone());
        }

        let mut rewind_route: Option<crate::route::Route> = None;
        let (mut machine, mut log, finish) = loop {
            let (mut machine, mut log) = setup(title, cli, &mut control)?;
            if let Some(route) = rewind_route.take() {
                log.line(&format!("rewind replay {} events", route.len()));
                machine.route = Some(route);
                machine.recording = Some(crate::route::Route::new());
                // Re-anchor pacing and telemetry after `setup()` finishes so
                // disc/image reload time is not billed to flip 0 (#349).
                control.reset_for_rewind();
            }
            let obs_bus = bus.clone();
            machine.set_flip_observer(Box::new(move |m, h| {
                service_flip_boundary(&obs_bus, m, h);
            }));
            let finish = machine.run(&mut control, &mut log, budget);
            if let Some((_name, bm)) = bus.take_pending_rewind() {
                match crate::route::Route::parse(&bm.route_text) {
                    Ok(route) => {
                        rewind_route = Some(route);
                        continue;
                    }
                    Err(e) => {
                        bus.fail_pending_rewind(format!("failed to parse bookmark route: {e}"));
                    }
                }
            }
            break (machine, log, finish);
        };

        control.free_mouse();
        control.pointer_shape(None);
        let digest = wrap_up(title, &mut machine, &mut control, &mut log, &finish);
        control.mark_finished(&finish, &digest);
        if let Some(server) = &mut http_server {
            server.stop();
        }
        #[cfg(unix)]
        if let Some(server) = &mut unix_server {
            server.stop();
        }
        if cli.fps_report || cli.fps_window.is_some() {
            for line in control.telemetry_report(cli.fps_window).format_lines() {
                log.line(&line);
                if cli.control_stdio {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            }
        }
        return Ok((finish, log, digest));
    }
    if cli.fps_report || cli.fps_window.is_some() {
        let mut telemetry = TelemetryHost::new(host, title.flip_cost);
        let (mut machine, mut log) = setup(title, cli, &mut telemetry)?;
        let finish = machine.run(&mut telemetry, &mut log, budget);
        telemetry.free_mouse();
        telemetry.pointer_shape(None);
        let digest = wrap_up(title, &mut machine, &mut telemetry, &mut log, &finish);
        for line in telemetry.report(cli.fps_window).format_lines() {
            log.line(&line);
            println!("{line}");
        }
        return Ok((finish, log, digest));
    }
    let (mut machine, mut log) = setup(title, cli, host)?;
    let finish = machine.run(host, &mut log, budget);
    // Nothing reads the mouse once the run has ended, so it is the person's
    // again, freed as losing the front frees it, and stays theirs while the
    // window waits to be closed (#111, #141), in the host's own shape.
    host.free_mouse();
    host.pointer_shape(None);
    let digest = wrap_up(title, &mut machine, host, &mut log, &finish);
    Ok((finish, log, digest))
}

/// Everything a run is given before its first instruction, said in its log.
/// The debugger starts here too, so what a person steps through is what a
/// route replays.
///
/// # Errors
/// As [`run`].
pub(crate) fn setup(
    title: &'static Title,
    cli: &Cli,
    host: &mut dyn Host,
) -> Result<(Machine, RunLog), Box<dyn std::error::Error>> {
    let mut log = RunLog::create(title.key, cli.trace)?;
    // Which versions made this run: the title's crate and core's, which is
    // everything it runs, since a title pins core exactly (ADR-0007).
    log.line(&format!(
        "version {} core {}",
        title.version,
        env!("CARGO_PKG_VERSION")
    ));
    log.line(&format!("headless {}", cli.headless));
    // Whether a device was listening. Mixing runs either way, so without this
    // a log cannot be told apart from one of a run nobody could hear — and
    // "the sound is wrong" is unanswerable against a run that had none.
    log.line(&format!("audio {}", cli.audio));
    let mut machine = Machine::new(title, host)?;
    if cli.stub {
        machine.load_stub()?;
    } else {
        machine.load_title(title)?;
        // What the declared install tree holds, which is what the title's own
        // installer would have put on the machine's disc.
        log.line(&format!(
            "installed {} files",
            machine.state.installed_files
        ));
        if let Some(image) = &machine.image {
            log.line(&format!(
                "image {} base {:#010x} entry {:#010x} imports {}",
                title.image.path,
                image.base,
                image.base.wrapping_add(image.entry),
                image.imports.len()
            ));
        }
        // Every binary the title ships beside it runs as guest code
        // (ADR-0002, #55), and where each one landed is worth saying: a
        // wall's caller is otherwise an address in nothing.
        for module in machine.modules.modules() {
            log.line(&format!(
                "module {} base {:#010x} dllmain {:#010x}",
                module.name, module.base, module.entry
            ));
        }
        log.line(&format!("bound {} names", machine.bound_names));
        // The bank its MIDI out plays, a declared input like the image: with
        // it the machine has the GS Wavetable synth, and without it no MIDI
        // out at all (ADR-0008, #156). Kept on the roster rather than `Title`
        // so adding it does not break every title crate (ADR-0007).
        let places = crate::roster::ROSTER
            .iter()
            .find(|e| e.key == title.key)
            .map_or(&[][..], |e| e.bank);
        if !places.is_empty() {
            log.line(&open_bank(places, &mut machine.state.mixer)?);
        }
    }
    // A route is the run's only recorded input (ADR-0003).
    if let Some(path) = &cli.route {
        let route = crate::route::Route::load(path)?;
        log.line(&format!("route {} events {}", path.display(), route.len()));
        machine.route = Some(route);
    }
    if cli.record || cli.control_enabled() {
        machine.recording = Some(crate::route::Route::new());
    }
    machine.screenshots = cli.screenshots.clone();
    machine.snapshot_every = cli.snapshot_every;
    machine.tree.set_profile(profile_dir(title, cli));
    Ok((machine, log))
}

/// Everything a run leaves behind after its last instruction, said in its
/// log, and the digest a determinism gate compares.
pub(crate) fn wrap_up(
    title: &Title,
    machine: &mut Machine,
    host: &mut dyn Host,
    log: &mut RunLog,
    finish: &Finish,
) -> String {
    // A save the host refused was said when it happened, and is said again
    // here, where a person looking at how the run ended will see it.
    let refused = machine.state.saves_refused;
    if refused > 0 {
        eprintln!(
            "lego-legacy: saves refused this run: {refused}; the log says which: {}",
            log.dir().join("log").display()
        );
    }

    // How the pointer was driven, said in one line: a title that puts the
    // pointer somewhere every frame is steering by how far the mouse went
    // since, and that only works where the host was told and froze the
    // person's own pointer to match (#40, #64).
    let (puts, told) = machine.pointer_puts();
    if puts > 0 {
        log.line(&format!(
            "pointer: the title put it {puts} times, and the host was told {told} of them"
        ));
    }

    // How much of the run was clock spins passed, when any was (ADR-0004,
    // #217): instructions the cursor counts that no engine ran, passed a
    // declared millisecond at a time.
    let passed = machine.instructions_passed();
    if passed > 0 {
        log.line(&format!(
            "clock spins: {passed} instructions passed, a millisecond at a time"
        ));
    }

    // Whether the device taking this run's audio ever ran dry. Said out loud
    // because it is the one thing a person hears that no test can catch: the
    // mix is made on the declared clock and a test reads it as it was mixed,
    // and what reaches a device is the host's own business (#75). Zero is said too: a
    // played run that says nothing cannot be told from one nobody heard, and
    // zero is what puts a complaint about the sound on the mix (#90).
    if let Some(underruns) = host.audio_underruns() {
        let behind = if underruns > 0 {
            "; the mix did not keep up"
        } else {
            ""
        };
        log.line(&format!(
            "audio: the device ran dry {underruns} times{behind}"
        ));
    }

    // A run that stopped at a wall leaves behind everything needed to reach it
    // again: what was done, where it stopped, and a snapshot taken at the
    // wall.
    if let crate::machine::Outcome::Wall { what } = &finish.outcome {
        let dir = log.dir().to_path_buf();
        let mut report = format!(
            "#lego-legacy-wall 1\ntitle {}\nwall {what}\ncursor {}\nflips {}\n",
            title.key, finish.cursor, finish.flips
        );
        let mut route = crate::route::Route::new();
        for (at, event) in &machine.input {
            route.push(*at, *event);
        }
        report.push('\n');
        report.push_str(&route.to_text());
        let written = std::fs::write(dir.join("wall"), report).and_then(|()| {
            machine.snapshot().map_or(Ok(()), |bytes| {
                std::fs::write(dir.join("wall.snapshot"), bytes)
            })
        });
        match written {
            Ok(()) => log.line("wall report written"),
            Err(e) => log.line(&format!("wall report not written: {e}")),
        }
    }

    // What the route asked for after the last flip or poll the run reached
    // was never delivered, and saying so is the difference between a route
    // that ended and one that was cut short.
    let undelivered = machine.undelivered();
    if undelivered > 0 {
        log.line(&format!(
            "route undelivered {undelivered} events after flip {}",
            finish.flips
        ));
    }
    if let Some(recording) = &machine.recording {
        let path = log.dir().join("route");
        match std::fs::write(&path, recording.to_text()) {
            Ok(()) => log.line(&format!(
                "recorded {} events to {}",
                recording.len(),
                path.display()
            )),
            Err(e) => log.line(&format!(
                "recorded {} events, not written to {}: {e}",
                recording.len(),
                path.display()
            )),
        }
    }
    let digest = machine.state_digest();
    log.line(&format!("digest {digest}"));
    digest
}

/// What a title binary's `main` calls.
///
/// Reads the process arguments, chooses a host, runs, and reports. The exit
/// status is the guest's own when it ended the run itself.
#[must_use]
pub fn main(title: &'static Title) -> ExitCode {
    main_with_guide(title, &TitleGuide::EMPTY)
}

/// What a title binary's `main` calls when supplying static [`TitleGuide`]
/// metadata (#311).
#[must_use]
pub fn main_with_guide(title: &'static Title, guide: &'static TitleGuide) -> ExitCode {
    let cli = match Cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(CliError::Help) => {
            println!("{}", crate::cli::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            eprintln!("{}", crate::cli::USAGE);
            return ExitCode::FAILURE;
        }
    };
    if cli.repl {
        return debugger(title, &cli);
    }
    // A run stopped from its terminal or console, by Ctrl-C, `SIGTERM` or
    // closing it, ends as the window's close button ends it, with its `end`
    // line and digest, headless or not (#107, #110).
    crate::host::listen();
    if !cli.headless {
        return windowed(title, guide, &cli);
    }
    let mut host = Headless::new();
    match run_with_guide(title, guide, &cli, &mut host, u64::MAX) {
        Ok((finish, log)) => {
            report(title, &cli, &finish, &log);
            status(&finish)
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Say how a finished run ended, where, and where its log is.
fn report(title: &Title, cli: &Cli, finish: &Finish, log: &RunLog) {
    if cli.control_stdio {
        eprintln!("{}: {}", title.key, finish.outcome);
        eprintln!(
            "cursor {} flips {} declared-ms {} log {}",
            finish.cursor,
            finish.flips,
            finish.declared_millis,
            log.dir().join("log").display()
        );
    } else {
        println!("{}: {}", title.key, finish.outcome);
        println!(
            "cursor {} flips {} declared-ms {} log {}",
            finish.cursor,
            finish.flips,
            finish.declared_millis,
            log.dir().join("log").display()
        );
    }
}

/// The guest's own exit status when it ended the run itself, and failure
/// otherwise.
fn status(finish: &Finish) -> ExitCode {
    match finish.outcome {
        Outcome::Exited { code } => ExitCode::from(u8::try_from(code & 0xff).unwrap_or(0xff)),
        _ => ExitCode::FAILURE,
    }
}

/// Run a title in a window on the host's screen.
///
/// The window stays open after the guest has finished, because a frame that
/// flashes past is not something a person can look at.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
fn windowed(title: &'static Title, guide: &'static TitleGuide, cli: &Cli) -> ExitCode {
    // One window backend per operating system (ADR-0001), each opened, fed
    // and closed the same way.
    #[cfg(target_os = "linux")]
    use crate::host::linux::LinuxWindow as Window;
    #[cfg(target_os = "macos")]
    use crate::host::mac::MacWindow as Window;
    #[cfg(windows)]
    use crate::host::windows::WinWindow as Window;

    // The window opens at 640 by 480, the one screen every title on the roster
    // declares; the guest's frame is scaled into it, its aspect kept, and the
    // guest is never told.
    let mut host = match Window::open(title.key, 640, 480, title.pointer_speed) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("lego-legacy: no window: {e}");
            return ExitCode::FAILURE;
        }
    };
    // The mix runs either way; `--audio` only lets it be heard, and a host
    // with no output to open runs silent rather than not at all.
    if cli.audio {
        if let Err(e) = host.open_audio() {
            eprintln!("lego-legacy: --audio: running silent, no audio device: {e}");
        }
    }
    let ran = run_with_guide(title, guide, cli, &mut host, u64::MAX);
    host.close_audio();
    match ran {
        Ok((finish, log)) => {
            report(title, cli, &finish, &log);
            // A run the person ended by closing the window has none left to
            // close (#107). A controlled session (`--control-stdio`, `--control`,
            // `--control-unix`) exits cleanly once the run finishes rather than
            // blocking on a human window click (#308, #309).
            if !cli.has_control_transport() && finish.flips > 0 && host.closes() == 0 {
                println!("close the window to end");
                while host.closes() == 0 {
                    host.pump();
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
            status(&finish)
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn windowed(_title: &'static Title, _guide: &'static TitleGuide, _cli: &Cli) -> ExitCode {
    eprintln!("lego-legacy: this operating system has no window backend yet; run with --headless");
    ExitCode::FAILURE
}

/// Drive a title from the debugger on standard input.
fn debugger(title: &'static Title, cli: &Cli) -> ExitCode {
    let mut host = crate::host::headless::Headless::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut output = std::io::stdout();
    match crate::repl::drive(title, cli, &mut host, &mut input, &mut output) {
        Ok(finish) => {
            println!("{}: {}", title.key, finish.outcome);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lego-legacy: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Open the General MIDI synth on the first place that holds the bank, or
/// leave the mixer without a MIDI out and return the line to log.
fn open_bank(
    places: &[crate::title::EntryImage],
    mixer: &mut crate::sound::Mixer,
) -> Result<String, Box<dyn std::error::Error>> {
    match crate::roster::find_bank(places) {
        Ok((place, bytes)) => {
            let bank = crate::dls::Bank::parse(&bytes)
                .map_err(|e| format!("midi bank {} from {}: {e}", place.path, place.pressing))?;
            mixer.open_synth(crate::synth::Synth::new(bank));
            Ok(format!("midi bank {} from {}", place.path, place.pressing))
        }
        Err(why) => Ok(format!("midi bank unavailable, so no MIDI out: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::open_bank;
    use crate::roster::GM16;
    use crate::sound::Mixer;
    use crate::title::EntryImage;

    #[test]
    fn a_missing_bank_leaves_the_machine_with_no_midi_out() {
        let absent = EntryImage {
            pressing: "no-such-pressing",
            ..GM16[0]
        };
        let mut mixer = Mixer::default();
        let line = open_bank(&[absent], &mut mixer).expect("missing bank is not fatal");
        assert!(!mixer.midi_out(), "no synth opened without the bank");
        assert_eq!(
            line,
            "midi bank unavailable, so no MIDI out: no rip of the no-such-pressing pressing"
        );
    }
}
