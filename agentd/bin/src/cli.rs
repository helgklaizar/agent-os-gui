//! Command line and path parsing. Paths before the DB is opened come only from flags and environment variables (docs/architecture.md section 10).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

// `Serve` carries all startup paths and settings; the command is created once per process, boxing gains nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve(Paths),
    Migrate {
        db: PathBuf,
        check: bool,
    },
    GuiLaunch {
        launch_socket: PathBuf,
        command: Vec<String>,
    },
    /// A `kind = agent` token for a session's agent process (D31): same service socket, descriptor in `AGENTD_TOKEN_FD`.
    AgentLaunch {
        launch_socket: PathBuf,
        session: i64,
        command: Vec<String>,
    },
    Diagnose {
        db: PathBuf,
        socket: PathBuf,
    },
    Backup {
        db: PathBuf,
        root: PathBuf,
        recipients: Vec<String>,
        /// Directory for the second copy (external disk or network path, S10-DB-01).
        copy_to: Option<PathBuf>,
    },
    /// Archive check of `backup` by decryption and parsing (S10-DB-02); a failure is an `error` event in the feed.
    BackupVerify {
        archive: PathBuf,
        identity: PathBuf,
        db: PathBuf,
    },
    /// Restore from a `backup` archive (A7-09) or from `pre-migration-VERSION.db` (S10-DB-03): live files are moved
    /// to `*.bak-YYYYMMDD`, not deleted.
    Restore {
        source: crate::restore::Source,
        db: PathBuf,
        root: PathBuf,
        /// Core socket: while it answers, `agentd` is alive and restoring is forbidden.
        socket: Option<PathBuf>,
    },
    /// Agent runner (A7-01): the connection is standard input (socket activation `Accept=yes`).
    Runner {
        allow_uid: u32,
        dirs: Vec<PathBuf>,
    },
    /// Observer as a separate process (A7-03): the connection is standard input (socket activation `Accept=yes`).
    Watch {
        allow_uid: Option<u32>,
        rules_dir: Option<PathBuf>,
    },
    Token {
        db: PathBuf,
        admin_socket: PathBuf,
        op: crate::admin::TokenOp,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Paths {
    pub db: PathBuf,
    pub socket: PathBuf,
    pub tcp: Option<SocketAddr>,
    /// Browser client: static directory, own hosts and origin, dev CORS (ADR-0010).
    pub web: crate::web_cfg::WebOpts,
    /// Service socket for GUI token issuance (`agentd-launch.sock` next to the main one).
    pub launch_socket: PathBuf,
    /// Service socket for `agentd token …` (`agentd-admin.sock` next to the main one).
    pub admin_socket: PathBuf,
    /// Process name of "our own cage" for the ancestor check.
    pub launch_ancestor: String,
    /// logind session service (`greetd`); not checked without a value.
    pub launch_service: Option<String>,
    /// Name of the process whose descendants get an agent token (the agent unit under `systemd --user`); default `systemd`.
    pub agent_ancestor: String,
    /// Agent unit name (`{}` is the session number); default `agent@{}.service`.
    pub agent_unit: String,
}

/// Browser client directory in the `.deb` package (`agentd/packaging/build-deb.sh`): used if `--web-root` is not set.
pub const PACKAGED_WEB_ROOT: &str = "/usr/share/agentd/web";

/// Installed client: a directory with `index.html`; otherwise static files are not served.
pub fn installed_web_root(dir: &Path) -> Option<PathBuf> {
    dir.join("index.html").is_file().then(|| dir.to_path_buf())
}

#[derive(Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

pub const USAGE: &str = "usage: agentd serve [--db PATH] [--socket PATH] [--tcp ADDR] | agentd gui-launch [--launch-socket PATH] -- <command> | agentd agent-launch [--launch-socket PATH] --session N -- <command> | agentd token issue --name NAME --scope read|chat [--ttl-days N] [--decide] | agentd token list | agentd token revoke --id N | agentd backup [--recipient age1…]… [--copy-to DIR] [--db PATH] [--root PATH] | agentd backup-verify --archive PATH --identity PATH [--db PATH] | agentd restore (--archive PATH --identity PATH | --snapshot PATH) [--socket PATH] [--db PATH] [--root PATH] | agentd runner --allow-uid N --allow-dir PATH… (connection is stdin) | agentd watch [--allow-uid N] [--rules-dir PATH] (connection is stdin) | agentd --diagnose | agentd migrate [--check] [--db PATH]\n\
serve flags: --web-root DIR (browser client), --allow-host, --allow-origin, --dev-cors (repeatable), --launch-socket, --launch-ancestor (default cage), --launch-service, --agent-ancestor (default systemd), --agent-unit (default agent@{}.service)
variables: AGENTD_DB, AGENTD_SOCKET, AGENTD_TCP, AGENTD_LAUNCH_SOCKET, AGENTD_LAUNCH_ANCESTOR, AGENTD_LAUNCH_SERVICE, HOME, XDG_RUNTIME_DIR; migrate --check codes: 0 up to date, 1 something to apply, 3 DB newer than code";

#[derive(Default)]
struct Flags {
    db: Option<String>,
    socket: Option<String>,
    tcp: Option<String>,
    web_root: Option<String>,
    allow_hosts: Vec<String>,
    allow_origins: Vec<String>,
    dev_cors: Vec<String>,
    launch_socket: Option<String>,
    launch_ancestor: Option<String>,
    launch_service: Option<String>,
    agent_ancestor: Option<String>,
    agent_unit: Option<String>,
    check: bool,
    name: Option<String>,
    scope: Option<String>,
    ttl_days: Option<String>,
    decide: bool,
    id: Option<String>,
    root: Option<String>,
    archive: Option<String>,
    identity: Option<String>,
    recipients: Vec<String>,
    copy_to: Option<String>,
    snapshot: Option<String>,
    allow_uid: Option<String>,
    allow_dirs: Vec<String>,
    rules_dir: Option<String>,
}

fn flags(mut it: impl Iterator<Item = String>) -> Result<Flags, UsageError> {
    let mut f = Flags::default();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .ok_or_else(|| UsageError(format!("flag {name} has no value")))
        };
        match a.as_str() {
            "--db" => f.db = Some(value("--db")?),
            "--socket" => f.socket = Some(value("--socket")?),
            "--tcp" => f.tcp = Some(value("--tcp")?),
            "--web-root" => f.web_root = Some(value("--web-root")?),
            "--allow-host" => f.allow_hosts.push(value("--allow-host")?),
            "--allow-origin" => f.allow_origins.push(value("--allow-origin")?),
            "--dev-cors" => f.dev_cors.push(value("--dev-cors")?),
            "--launch-socket" => f.launch_socket = Some(value("--launch-socket")?),
            "--launch-ancestor" => f.launch_ancestor = Some(value("--launch-ancestor")?),
            "--launch-service" => f.launch_service = Some(value("--launch-service")?),
            "--agent-ancestor" => f.agent_ancestor = Some(value("--agent-ancestor")?),
            "--agent-unit" => f.agent_unit = Some(value("--agent-unit")?),
            "--check" => f.check = true,
            "--name" => f.name = Some(value("--name")?),
            "--scope" => f.scope = Some(value("--scope")?),
            "--ttl-days" => f.ttl_days = Some(value("--ttl-days")?),
            "--decide" => f.decide = true,
            "--id" => f.id = Some(value("--id")?),
            "--root" => f.root = Some(value("--root")?),
            "--archive" => f.archive = Some(value("--archive")?),
            "--identity" => f.identity = Some(value("--identity")?),
            "--allow-uid" => f.allow_uid = Some(value("--allow-uid")?),
            "--rules-dir" => f.rules_dir = Some(value("--rules-dir")?),
            "--allow-dir" => f.allow_dirs.push(value("--allow-dir")?),
            "--recipient" => f.recipients.push(value("--recipient")?),
            "--copy-to" => f.copy_to = Some(value("--copy-to")?),
            "--snapshot" => f.snapshot = Some(value("--snapshot")?),
            other => return Err(UsageError(format!("unknown argument {other}"))),
        }
    }
    Ok(f)
}

fn db_path(f: &Flags, env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, UsageError> {
    if let Some(p) = f.db.clone().or_else(|| env("AGENTD_DB")) {
        return Ok(PathBuf::from(p));
    }
    let home = env("HOME")
        .ok_or_else(|| UsageError("DB path not set: --db, AGENTD_DB or HOME is required".into()))?;
    Ok(PathBuf::from(home).join(".agentos/sessions.db"))
}

fn socket_path(f: &Flags, env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, UsageError> {
    if let Some(p) = f.socket.clone().or_else(|| env("AGENTD_SOCKET")) {
        return Ok(PathBuf::from(p));
    }
    let dir = env("XDG_RUNTIME_DIR").ok_or_else(|| {
        UsageError(
            "socket path not set: --socket, AGENTD_SOCKET or XDG_RUNTIME_DIR is required".into(),
        )
    })?;
    Ok(PathBuf::from(dir).join("agentd.sock"))
}

fn tcp_addr(
    f: &Flags,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<SocketAddr>, UsageError> {
    f.tcp
        .clone()
        .or_else(|| env("AGENTD_TCP"))
        .map(|s| {
            s.parse()
                .map_err(|_| UsageError(format!("TCP address not parsed: {s}")))
        })
        .transpose()
}

fn token_op(sub: &str, f: &Flags) -> Result<crate::admin::TokenOp, UsageError> {
    use crate::admin::TokenOp;
    match sub {
        "issue" => {
            let name = f
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .ok_or_else(|| UsageError("--name is required".into()))?;
            let scope = match f.scope.as_deref() {
                Some("read") => agentd_core::Scope::Read,
                Some("chat") => agentd_core::Scope::Chat,
                _ => return Err(UsageError("--scope read or chat is required".into())),
            };
            let ttl_days = f
                .ttl_days
                .as_deref()
                .map(|t| {
                    t.parse::<u32>()
                        .map_err(|_| UsageError("--ttl-days: an integer".into()))
                })
                .transpose()?;
            if f.decide && scope != agentd_core::Scope::Chat {
                return Err(UsageError(
                    "--decide is allowed only with --scope chat".into(),
                ));
            }
            if f.decide && ttl_days.is_some_and(|d| d == 0 || d > 30) {
                return Err(UsageError("--decide: --ttl-days from 1 to 30".into()));
            }
            Ok(TokenOp::Issue {
                name,
                scope,
                ttl_days,
                decide: f.decide,
            })
        }
        "list" => Ok(TokenOp::List),
        "revoke" => {
            let id =
                f.id.as_deref()
                    .and_then(|i| i.parse::<i64>().ok())
                    .ok_or_else(|| UsageError("--id is required (an integer)".into()))?;
            Ok(TokenOp::Revoke { id })
        }
        other => Err(UsageError(format!("unknown token subcommand {other}"))),
    }
}

/// `env` is the source of environment variables (replaced in parsing tests).
pub fn parse(
    args: Vec<String>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Command, UsageError> {
    let mut it = args.into_iter();
    let Some(cmd) = it.next() else {
        return Err(UsageError("no subcommand given".into()));
    };
    match cmd.as_str() {
        "serve" => {
            let f = flags(it)?;
            let socket = socket_path(&f, env)?;
            let launch_socket = f
                .launch_socket
                .clone()
                .or_else(|| env("AGENTD_LAUNCH_SOCKET"))
                .map_or_else(
                    || socket.with_file_name("agentd-launch.sock"),
                    PathBuf::from,
                );
            Ok(Command::Serve(Paths {
                admin_socket: socket.with_file_name("agentd-admin.sock"),
                db: db_path(&f, env)?,
                socket,
                tcp: tcp_addr(&f, env)?,
                web: crate::web_cfg::WebOpts {
                    root: f
                        .web_root
                        .clone()
                        .or_else(|| env("AGENTD_WEB_ROOT"))
                        .map(PathBuf::from)
                        .or_else(|| installed_web_root(Path::new(PACKAGED_WEB_ROOT))),
                    allow_hosts: f.allow_hosts.clone(),
                    allow_origins: f.allow_origins.clone(),
                    dev_cors: f.dev_cors.clone(),
                },
                launch_socket,
                launch_ancestor: f
                    .launch_ancestor
                    .clone()
                    .or_else(|| env("AGENTD_LAUNCH_ANCESTOR"))
                    .unwrap_or_else(|| "cage".to_string()),
                launch_service: f
                    .launch_service
                    .clone()
                    .or_else(|| env("AGENTD_LAUNCH_SERVICE")),
                agent_ancestor: f
                    .agent_ancestor
                    .clone()
                    .or_else(|| env("AGENTD_AGENT_ANCESTOR"))
                    .unwrap_or_else(|| "systemd".to_string()),
                agent_unit: f
                    .agent_unit
                    .clone()
                    .or_else(|| env("AGENTD_AGENT_UNIT"))
                    .unwrap_or_else(|| crate::launch::DEFAULT_AGENT_UNIT.to_string()),
            }))
        }
        "backup" => {
            let f = flags(it)?;
            let db = db_path(&f, env)?;
            let root = f.root.clone().map_or_else(
                || {
                    db.parent()
                        .map_or_else(|| PathBuf::from("."), PathBuf::from)
                },
                PathBuf::from,
            );
            Ok(Command::Backup {
                db,
                root,
                recipients: f.recipients.clone(),
                copy_to: f.copy_to.clone().map(PathBuf::from),
            })
        }
        "backup-verify" => {
            let f = flags(it)?;
            Ok(Command::BackupVerify {
                archive: f
                    .archive
                    .clone()
                    .map(PathBuf::from)
                    .ok_or_else(|| UsageError("--archive PATH is required".into()))?,
                identity: f.identity.clone().map(PathBuf::from).ok_or_else(|| {
                    UsageError("--identity PATH is required (age secret key)".into())
                })?,
                db: db_path(&f, env)?,
            })
        }
        "restore" => {
            let f = flags(it)?;
            let db = db_path(&f, env)?;
            let root = f.root.clone().map_or_else(
                || {
                    db.parent()
                        .map_or_else(|| PathBuf::from("."), PathBuf::from)
                },
                PathBuf::from,
            );
            let source = match (&f.snapshot, &f.archive, &f.identity) {
                (Some(snap), None, None) => crate::restore::Source::Snapshot(PathBuf::from(snap)),
                (None, Some(a), Some(i)) => crate::restore::Source::Archive {
                    archive: PathBuf::from(a),
                    identity: PathBuf::from(i),
                },
                (None, Some(_), None) => {
                    return Err(UsageError(
                        "--identity PATH is required (age secret key)".into(),
                    ));
                }
                (None, None, _) => {
                    return Err(UsageError(
                        "--archive PATH with --identity PATH or --snapshot PATH is required".into(),
                    ));
                }
                _ => {
                    return Err(UsageError(
                        "--snapshot is incompatible with --archive and --identity".into(),
                    ));
                }
            };
            let socket = socket_path(&f, env).ok();
            Ok(Command::Restore {
                source,
                db,
                root,
                socket,
            })
        }
        "runner" => {
            let f = flags(it)?;
            let allow_uid = f
                .allow_uid
                .as_deref()
                .and_then(|u| u.parse().ok())
                .ok_or_else(|| UsageError("--allow-uid is required (an integer)".into()))?;
            if f.allow_dirs.is_empty() {
                return Err(UsageError("at least one --allow-dir is required".into()));
            }
            Ok(Command::Runner {
                allow_uid,
                dirs: f.allow_dirs.iter().map(PathBuf::from).collect(),
            })
        }
        "watch" => {
            let f = flags(it)?;
            let allow_uid = match f.allow_uid.as_deref() {
                None => None,
                Some(u) => Some(
                    u.parse()
                        .map_err(|_| UsageError("--allow-uid: an integer".into()))?,
                ),
            };
            Ok(Command::Watch {
                allow_uid,
                rules_dir: f.rules_dir.as_deref().map(PathBuf::from),
            })
        }
        "token" => {
            let sub = it
                .next()
                .ok_or_else(|| UsageError("subcommand issue, list or revoke is required".into()))?;
            let f = flags(it)?;
            let op = token_op(&sub, &f)?;
            let socket = socket_path(&f, env)?;
            Ok(Command::Token {
                db: db_path(&f, env)?,
                admin_socket: socket.with_file_name("agentd-admin.sock"),
                op,
            })
        }
        "migrate" => {
            let f = flags(it)?;
            Ok(Command::Migrate {
                db: db_path(&f, env)?,
                check: f.check,
            })
        }
        "gui-launch" | "agent-launch" => {
            let agent = cmd == "agent-launch";
            let mut launch_socket = None;
            let mut session = None;
            let mut command = Vec::new();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--launch-socket" => {
                        launch_socket = Some(it.next().ok_or_else(|| {
                            UsageError("flag --launch-socket has no value".into())
                        })?);
                    }
                    "--session" if agent => {
                        session = Some(
                            it.next()
                                .ok_or_else(|| UsageError("flag --session has no value".into()))?,
                        );
                    }
                    "--" => {
                        command = it.by_ref().collect();
                    }
                    other => return Err(UsageError(format!("unknown argument {other}"))),
                }
            }
            let launch_socket = match launch_socket.or_else(|| env("AGENTD_LAUNCH_SOCKET")) {
                Some(p) => PathBuf::from(p),
                None => PathBuf::from(env("XDG_RUNTIME_DIR").ok_or_else(|| {
                                        UsageError("token issuance socket not set: --launch-socket, AGENTD_LAUNCH_SOCKET or XDG_RUNTIME_DIR is required".into())
                })?)
                .join("agentd-launch.sock"),
            };
            if !agent {
                return Ok(Command::GuiLaunch {
                    launch_socket,
                    command,
                });
            }
            // The session number is also checked on the server; here only the request shape.
            let session = session
                .ok_or_else(|| UsageError("agent-launch: --session N is required".into()))?
                .parse::<i64>()
                .map_err(|_| UsageError("--session: an integer".into()))?;
            Ok(Command::AgentLaunch {
                launch_socket,
                session,
                command,
            })
        }
        "--diagnose" | "diagnose" => {
            let f = flags(it)?;
            let home_db = || {
                env("HOME").map_or_else(
                    || PathBuf::from("sessions.db"),
                    |h| PathBuf::from(h).join(".agentos/sessions.db"),
                )
            };
            let db =
                f.db.clone()
                    .or_else(|| env("AGENTD_DB"))
                    .map_or_else(home_db, PathBuf::from);
            let socket = f
                .socket
                .clone()
                .or_else(|| env("AGENTD_SOCKET"))
                .map(PathBuf::from)
                .or_else(|| env("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("agentd.sock")))
                .unwrap_or_else(|| PathBuf::from("agentd.sock"));
            Ok(Command::Diagnose { db, socket })
        }
        other => Err(UsageError(format!("unknown subcommand {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| (*v).to_string())
        }
    }

    #[test]
    fn installed_web_root_needs_index_html() {
        let d = std::env::temp_dir().join(format!("agentd-webroot-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(
            installed_web_root(&d),
            None,
            "empty directory: no static files"
        );
        assert_eq!(installed_web_root(&d.join("missing")), None);
        std::fs::write(d.join("index.html"), "x").unwrap();
        assert_eq!(installed_web_root(&d), Some(d.clone()));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn token_issue_decide_bounds() {
        let env = env_of(&[("XDG_RUNTIME_DIR", "/run/u"), ("HOME", "/h")]);
        let p = |extra: &[&str]| {
            let mut v = vec![
                "token", "issue", "--name", "w", "--scope", "chat", "--decide",
            ];
            v.extend_from_slice(extra);
            parse(args(&v), &env)
        };
        let decide_of = |r: Result<Command, UsageError>| match r {
            Ok(Command::Token {
                op: crate::admin::TokenOp::Issue { decide, .. },
                ..
            }) => Some(decide),
            _ => None,
        };
        assert_eq!(decide_of(p(&[])), Some(true));
        assert_eq!(decide_of(p(&["--ttl-days", "1"])), Some(true));
        assert_eq!(decide_of(p(&["--ttl-days", "30"])), Some(true));
        assert_eq!(decide_of(p(&["--ttl-days", "31"])), None);
        assert_eq!(decide_of(p(&["--ttl-days", "0"])), None);
        let read = parse(
            args(&[
                "token", "issue", "--name", "w", "--scope", "read", "--decide",
            ]),
            &env,
        );
        assert!(read.is_err());
        let plain = parse(
            args(&[
                "token",
                "issue",
                "--name",
                "w",
                "--scope",
                "chat",
                "--ttl-days",
                "90",
            ]),
            &env,
        );
        assert_eq!(decide_of(plain), Some(false));
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn flags_override_env_and_defaults_come_from_home_and_runtime_dir() {
        let env = env_of(&[
            ("HOME", "/h"),
            ("XDG_RUNTIME_DIR", "/run/u"),
            ("AGENTD_DB", "/env.db"),
        ]);
        let Ok(Command::Serve(p)) = parse(args(&["serve"]), &env) else {
            panic!("serve expected")
        };
        assert_eq!(
            (p.db.to_str(), p.socket.to_str(), p.tcp),
            (Some("/env.db"), Some("/run/u/agentd.sock"), None)
        );
        let Ok(Command::Serve(p)) = parse(
            args(&[
                "serve",
                "--db",
                "/f.db",
                "--socket",
                "/f.sock",
                "--tcp",
                "127.0.0.1:9",
            ]),
            &env,
        ) else {
            panic!()
        };
        assert_eq!(
            (p.db.to_str(), p.socket.to_str()),
            (Some("/f.db"), Some("/f.sock"))
        );
        assert_eq!(p.tcp.map(|a| a.to_string()), Some("127.0.0.1:9".into()));
        let env = env_of(&[("HOME", "/h")]);
        let Ok(Command::Migrate { db, check }) = parse(args(&["migrate", "--check"]), &env) else {
            panic!()
        };
        assert_eq!(
            (db.to_str(), check),
            (Some("/h/.agentos/sessions.db"), true)
        );
    }

    #[test]
    fn missing_paths_bad_flags_and_bad_addresses_are_errors() {
        let none = env_of(&[]);
        assert!(
            parse(args(&["serve", "--db", "/x"]), &none)
                .unwrap_err()
                .0
                .contains("XDG_RUNTIME_DIR")
        );
        assert!(
            parse(args(&["serve", "--socket", "/x"]), &none)
                .unwrap_err()
                .0
                .contains("HOME")
        );
        assert!(parse(args(&["serve", "--bogus"]), &none).is_err());
        assert!(parse(args(&["serve", "--db"]), &none).is_err());
        assert!(
            parse(
                args(&[
                    "serve",
                    "--db",
                    "/x",
                    "--socket",
                    "/y",
                    "--tcp",
                    "not an address"
                ]),
                &none
            )
            .is_err()
        );
        assert!(parse(args(&[]), &none).is_err());
        assert!(parse(args(&["bogus"]), &none).is_err());
        assert!(
            parse(args(&["gui-launch", "--", "true"]), &none)
                .unwrap_err()
                .0
                .contains("XDG_RUNTIME_DIR")
        );
        let env = env_of(&[("XDG_RUNTIME_DIR", "/run/u")]);
        assert_eq!(
            parse(args(&["gui-launch", "--", "true", "-x"]), &env),
            Ok(Command::GuiLaunch {
                launch_socket: "/run/u/agentd-launch.sock".into(),
                command: vec!["true".into(), "-x".into()],
            })
        );
        assert!(parse(args(&["gui-launch", "--bogus"]), &env).is_err());
    }
}
