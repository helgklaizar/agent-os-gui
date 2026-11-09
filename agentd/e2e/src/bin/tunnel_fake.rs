//! Fake tunnel command for A6-06 tests, standing in for `ssh` and `wg-quick`. Appends a launch line to `TUNNEL_FAKE_LOG`
//! (arguments and environment variable names), then: `up`/`down` exits with code 0; an address containing "crash" exits with code 1;
//! otherwise it lives until stopped.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut names: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
    names.sort();
    if let Some(p) = std::env::var_os("TUNNEL_FAKE_LOG")
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
    {
        drop(writeln!(
            f,
            "start {} | env {}",
            args.join(" "),
            names.join(",")
        ));
    }
    if matches!(args.first().map(String::as_str), Some("up" | "down")) {
        return;
    }
    if args.iter().any(|a| a.contains("crash")) {
        std::process::exit(1);
    }
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
