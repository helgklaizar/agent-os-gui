//! The `agentd` binary: subcommands `serve`, `gui-launch`, `migrate [--check]`.

mod admin;
mod archive;
mod backup;
mod boot;
mod cli;
mod dbus_notify;
mod diagnose;
mod heartbeat;
mod json_export;
mod launch;
mod logout;
mod perms;
mod restore;
mod runner;
mod runner_client;
mod runner_req;
mod serve;
mod shell_tokens;
mod stack;
mod sys_probe;
mod tunnel_proc;
mod verify;
mod watch;
mod watch_proc;
mod web_cfg;

use std::process::ExitCode;

use cli::{Command, USAGE, parse};

fn init_logs() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // Output as text to stderr; journald and structured fields are A1-12.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

#[tokio::main]
async fn main() -> ExitCode {
    let env = |k: &str| std::env::var(k).ok();
    let cmd = match parse(std::env::args().skip(1).collect(), &env) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("agentd: {}\n{USAGE}", e.0);
            return ExitCode::from(2);
        }
    };
    init_logs();
    let result = match cmd {
        Command::Serve(paths) => serve::serve(paths).await.map(|()| 0),
        Command::Migrate { db, check } => serve::migrate(&db, check),
        Command::GuiLaunch {
            launch_socket,
            command,
        } => return ExitCode::from(launch::gui_launch(&launch_socket, &command)),
        Command::AgentLaunch {
            launch_socket,
            session,
            command,
        } => return ExitCode::from(launch::agent_launch(&launch_socket, session, &command)),
        Command::Diagnose { db, socket } => {
            for line in diagnose::report(&db, &socket) {
                println!("{line}");
            }
            return ExitCode::SUCCESS;
        }
        Command::Backup {
            db,
            root,
            recipients,
            copy_to,
        } => backup::run(&db, &root, &recipients, copy_to.as_deref()).map(|paths| {
            for p in paths {
                println!("{}", p.display());
            }
            0
        }),
        Command::BackupVerify {
            archive,
            identity,
            db,
        } => verify::run(&archive, &identity, &db).map(|n| {
            println!("archive readable; files: {n}");
            0
        }),
        Command::Restore {
            source,
            db,
            root,
            socket,
        } => restore::run(&source, &db, &root, socket.as_deref()).map(|moved| {
            println!("restored; moved to *.bak: {moved}");
            0
        }),
        Command::Runner { allow_uid, dirs } => runner::run(allow_uid, dirs).await.map(|()| 0),
        Command::Watch {
            allow_uid,
            rules_dir,
        } => watch_proc::run(allow_uid, rules_dir).await.map(|()| 0),
        Command::Token {
            db,
            admin_socket,
            op,
        } => admin::run(op, admin_socket, db).await,
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("agentd: {e:#}");
            // Newer DB than code: a separate exit code (as for `migrate --check`), the unit is not restarted.
            let newer = e.chain().any(|c| {
                matches!(
                    c.downcast_ref::<agentd_db::DbError>(),
                    Some(agentd_db::DbError::NewerThanCode { .. })
                )
            });
            ExitCode::from(if newer { 3 } else { 1 })
        }
    }
}
