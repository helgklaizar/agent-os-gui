//! `fake-agentd` binary: core mock for manual GUI development and UI scenarios.
//!
//! `fake-agentd --socket <path> [--tcp 127.0.0.1:0] [--token-file <path>] [--scenario <file>] [--clock manual|real]`.
//! The chosen TCP port and socket path are printed to stdout; token values are never printed.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use fake_agentd::{Config, FakeAgentd, Scenario};

fn usage() -> ! {
    eprintln!(
        "usage: fake-agentd --socket <path> [--tcp <addr>] [--token-file <path>] [--scenario <file>] [--clock manual|real] [--db <path>] [--reply-delay-ms <ms>]"
    );
    std::process::exit(2);
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
    let mut config = Config::temp().reply_delay(Duration::from_millis(2500));
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag}: value required"));
        match flag.as_str() {
            "--socket" => config = config.socket(PathBuf::from(value()?)),
            "--tcp" => {
                config = config.tcp_addr(
                    value()?
                        .parse::<SocketAddr>()
                        .map_err(|e| format!("--tcp: {e}"))?,
                )
            }
            "--token-file" => config = config.token_file(PathBuf::from(value()?)),
            "--scenario" => {
                config = config.scenario(
                    Scenario::from_file(&PathBuf::from(value()?)).map_err(|e| e.to_string())?,
                )
            }
            "--clock" => match value()?.as_str() {
                "manual" => config = config.manual_clock(),
                "real" => {}
                other => return Err(format!("--clock: manual or real, got {other}")),
            },
            "--reply-delay-ms" => {
                let ms = value()?
                    .parse::<u64>()
                    .map_err(|e| format!("--reply-delay-ms: {e}"))?;
                config = config.reply_delay(Duration::from_millis(ms));
            }
            "--db" => config = config.db(PathBuf::from(value()?)),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(config)
}

#[tokio::main]
async fn main() {
    let config = match parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            usage();
        }
    };
    let daemon = match FakeAgentd::start(config).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fake-agentd failed to start: {e}");
            std::process::exit(1);
        }
    };
    println!("socket {}", daemon.socket_path().display());
    if let Some(addr) = daemon.tcp_addr() {
        println!("tcp {addr}");
    }
    // Runs until SIGINT/SIGTERM (default handler); the socket stays as a file, a later start removes it as the owner.
    std::future::pending::<()>().await;
}
