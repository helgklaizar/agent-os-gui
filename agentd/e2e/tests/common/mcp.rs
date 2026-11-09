//! Helpers for the MCP end-to-end tests: the `mcp-fake` test server, server setup, call log.
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

use super::Env;

pub const FAKE: &str = env!("CARGO_BIN_EXE_mcp-fake");

pub fn log_of(e: &Env) -> std::path::PathBuf {
    e.dir.path().join("mcp.log")
}

pub fn log_lines(e: &Env) -> Vec<String> {
    std::fs::read_to_string(log_of(e))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

pub fn stdio_spec(e: &Env, tools: Value, extra: Value) -> Value {
    let mut spec = json!({"name": "fake", "transport": "stdio", "command": FAKE, "args": [],
        "env": {"MCP_FAKE_LOG": log_of(e).to_str().unwrap()}, "tools": tools});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        spec[k] = v;
    }
    spec
}

pub async fn set_mcp(e: &Env, servers: Value) -> (u16, Value) {
    let (_, page) = e.admin.get("/settings/connections").await;
    let rev = page["rev"].as_i64().unwrap_or(0);
    e.admin
        .patch_if_match("/settings/connections", json!({"mcp": servers}), rev)
        .await
}

pub async fn setup(e: &Env, tools: Value, extra: Value) {
    let (st, v) = set_mcp(e, json!([stdio_spec(e, tools, extra)])).await;
    assert_eq!(st, 200, "{v}");
}

pub async fn ask(e: &Env, s: i64, scenario: &str) {
    e.say(s, "c1", &format!("#scn:{scenario} call")).await;
}

pub fn body_of(tool: &Value) -> String {
    tool["body_md"].as_str().unwrap().to_string()
}

/// `--http` server; killed when the test exits, including on panic.
pub struct HttpFake(Child);

impl Drop for HttpFake {
    fn drop(&mut self) {
        drop(self.0.kill());
        drop(self.0.wait());
    }
}

pub fn http_server(e: &Env) -> (HttpFake, u16) {
    let mut child = Command::new(FAKE)
        .arg("--http")
        .env("MCP_FAKE_LOG", log_of(e))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port = line.trim().strip_prefix("PORT ").unwrap().parse().unwrap();
    (HttpFake(child), port)
}
