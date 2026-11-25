//! `lego-mcp` stdio JSON-RPC 2.0 MCP server entry point (#312).
//!
//! Strictly reserves `stdout` for newline-delimited JSON-RPC 2.0 frames; all
//! logs and usage errors go to `stderr`.

use lego_mcp::{serve_stdio, ServerDefaults};
use std::io::{self, BufReader};
use std::process::ExitCode;

const USAGE: &str = "\
usage: lego-mcp [--url <URL>] [--title <NAME>] [--token <TOK>]

Zero-crate stdio JSON-RPC 2.0 MCP server wrapping lego-legacy's HTTP/UDS control plane.
Reads newline-delimited JSON-RPC 2.0 requests from stdin and writes replies to stdout.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut defaults = ServerDefaults::default();
    let mut i = 0usize;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(v) = a.strip_prefix("--url=") {
            defaults.url = Some(v.to_owned());
        } else if let Some(v) = a.strip_prefix("--title=") {
            defaults.title = Some(v.to_owned());
        } else if let Some(v) = a.strip_prefix("--token=") {
            defaults.token = Some(v.to_owned());
        } else {
            match a {
                "--url" => {
                    i += 1;
                    let Some(v) = args.get(i) else {
                        eprintln!("lego-mcp: `--url` requires a value");
                        return ExitCode::FAILURE;
                    };
                    defaults.url = Some(v.clone());
                }
                "--title" => {
                    i += 1;
                    let Some(v) = args.get(i) else {
                        eprintln!("lego-mcp: `--title` requires a value");
                        return ExitCode::FAILURE;
                    };
                    defaults.title = Some(v.clone());
                }
                "--token" => {
                    i += 1;
                    let Some(v) = args.get(i) else {
                        eprintln!("lego-mcp: `--token` requires a value");
                        return ExitCode::FAILURE;
                    };
                    defaults.token = Some(v.clone());
                }
                "-h" | "--help" => {
                    eprintln!("{USAGE}");
                    return ExitCode::SUCCESS;
                }
                "-V" | "--version" => {
                    eprintln!("lego-mcp {}", env!("CARGO_PKG_VERSION"));
                    return ExitCode::SUCCESS;
                }
                other => {
                    eprintln!("lego-mcp: unknown argument `{other}`");
                    eprintln!("{USAGE}");
                    return ExitCode::FAILURE;
                }
            }
        }
        i += 1;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let reader = BufReader::new(stdin.lock());
    let writer = stdout.lock();

    match serve_stdio(reader, writer, &defaults) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lego-mcp: stdio error: {err}");
            ExitCode::FAILURE
        }
    }
}
