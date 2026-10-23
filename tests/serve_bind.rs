//! `loomtracer serve` must not be an open server by default (hivemind-4dur).
//!
//! Process-level tests drive the real binary, so they cover the clap
//! defaults, the env-var handling and the startup refusal together. The
//! `check_bind_safety` matrix is exercised in-process.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::Parser as _;
use loomtracer::api::ApiConfig;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const LISTENING_MARKER: &str = "HTTP API listening";

fn unique_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "loomtracer-serve-bind-{label}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).expect("create test ledger dir"); // ubs:ignore: test-only; panicking is correct in tests
    dir
}

/// tracing colors its fields even on a pipe; drop the SGR escapes so the
/// assertions can match plain `key=value` text.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // Skip `ESC [ ... <final byte>`.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A `loomtracer -v serve --port 0 ...` process with the auth/bind env of the
/// developer's shell scrubbed, so each test states exactly what it sets.
struct Serve {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Serve {
    fn spawn(label: &str, args: &[&str], envs: &[(&str, &str)]) -> Self {
        Self::spawn_in(unique_dir(label), args, envs)
    }

    /// [`Serve::spawn`] on a ledger directory the caller already holds, so a test can serve a
    /// ledger it filled first.
    fn spawn_in(dir: PathBuf, args: &[&str], envs: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_loomtracer"));
        for var in [
            "ANTHROPIC_API_KEY",
            "LOOMTRACER_API_KEY",
            "LOOMTRACER_ADMIN_KEY",
            "LOOMTRACER_DATABASE_URL",
            "LOOMTRACER_BIND",
            "LOOMTRACER_PORT",
            "WORKOS_DOMAIN",
        ] {
            cmd.env_remove(var);
        }
        let mut child = cmd
            .args(["-v", "--loomtracer-dir"])
            .arg(dir)
            .args(["serve", "--port", "0"])
            .args(args)
            .envs(envs.iter().copied())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn loomtracer serve"); // ubs:ignore: test-only; panicking is correct in tests

        let stderr = child.stderr.take().expect("piped stderr"); // ubs:ignore: test-only; panicking is correct in tests
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if tx.send(strip_ansi(&line)).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// Reads stderr up to the "HTTP API listening" line and returns every
    /// line seen so far plus the `host:port` the server reports binding.
    fn wait_listening(&self) -> (Vec<String>, String) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let mut seen = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self.lines.recv_timeout(left).unwrap_or_else(|e| {
                panic!("server never reported listening ({e}); stderr so far: {seen:#?}")
            }); // ubs:ignore: test-only
            if line.contains(LISTENING_MARKER) {
                let addr = line
                    .split("addr=")
                    .nth(1)
                    .and_then(|rest| rest.split_whitespace().next())
                    .unwrap_or_else(|| panic!("no addr= in startup line: {line}")) // ubs:ignore: test-only
                    .to_owned();
                seen.push(line);
                return (seen, addr);
            }
            seen.push(line);
        }
    }

    /// Reads stderr until a line contains `needle`, appending every line read to `seen`. For a
    /// line a background task logs, which can land just after "HTTP API listening".
    fn wait_line(&self, needle: &str, seen: &mut Vec<String>) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self.lines.recv_timeout(left).unwrap_or_else(|e| {
                panic!("server never logged {needle:?} ({e}); stderr so far: {seen:#?}")
            }); // ubs:ignore: test-only
            let found = line.contains(needle);
            seen.push(line);
            if found {
                return;
            }
        }
    }

    /// Waits for the process to exit on its own (a startup refusal) and
    /// returns its exit success plus all of stderr.
    fn wait_exit(mut self) -> (bool, String) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                // ubs:ignore: test-only
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit; it started instead of refusing"
            ); // ubs:ignore: test-only
            std::thread::sleep(Duration::from_millis(50));
        };
        // The reader thread ends when the pipe closes with the process.
        let stderr: Vec<String> = self.lines.iter().collect();
        (status.success(), stderr.join("\n"))
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Minimal HTTP/1.1 client: returns the status code of one request.
fn http_status(addr: &str, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> u16 {
    let mut stream = TcpStream::connect(addr).expect("connect to server"); // ubs:ignore: test-only
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .expect("read timeout"); // ubs:ignore: test-only
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).expect("write request"); // ubs:ignore: test-only
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response"); // ubs:ignore: test-only
    response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no HTTP status line in: {response}")) // ubs:ignore: test-only
}

/// Connectable address for a bind that may be the unspecified address.
fn loopback_of(addr: &str) -> String {
    let port = addr.rsplit(':').next().expect("port"); // ubs:ignore: test-only
    format!("127.0.0.1:{port}")
}

// ---------------------------------------------------------------------------
// Defaults
// ---------------------------------------------------------------------------

#[test]
fn serve_binds_loopback_by_default() {
    let serve = Serve::spawn("default", &[], &[]);
    let (lines, addr) = serve.wait_listening();
    assert!(
        addr.starts_with("127.0.0.1:"),
        "default bind must be loopback, got {addr}"
    ); // ubs:ignore: test-only
       // Development mode is still available on loopback, and says so.
    assert!(
        lines
            .iter()
            .any(|l| l.contains("development mode (no auth)") && l.contains("bind=127.0.0.1")),
        "startup log must name development mode and the bind: {lines:#?}"
    ); // ubs:ignore: test-only
    assert_eq!(http_status(&addr, "GET", "/v1/health", &[], ""), 200); // ubs:ignore: test-only
}

#[test]
fn api_config_new_defaults_to_loopback_and_no_remote_opt_in() {
    let config = ApiConfig::new(unique_dir("config-defaults"));
    assert_eq!(config.bind, IpAddr::V4(Ipv4Addr::LOCALHOST)); // ubs:ignore: test-only
    assert!(!config.allow_unauthenticated_remote); // ubs:ignore: test-only
}

// ---------------------------------------------------------------------------
// Refusal: development mode on a non-loopback bind
// ---------------------------------------------------------------------------

#[test]
fn dev_mode_refuses_non_loopback_bind_without_opt_in() {
    let (ok, stderr) = Serve::spawn("refuse", &["--bind", "0.0.0.0"], &[]).wait_exit();
    assert!(!ok, "unauthenticated non-loopback serve must exit non-zero"); // ubs:ignore: test-only
    assert!(
        stderr.contains("refusing to serve without authentication"),
        "{stderr}"
    ); // ubs:ignore: test-only
    assert!(
        stderr.contains("--allow-unauthenticated-remote"),
        "message must name the opt-in: {stderr}"
    ); // ubs:ignore: test-only
    assert!(
        !stderr.contains(LISTENING_MARKER),
        "must refuse before listening: {stderr}"
    ); // ubs:ignore: test-only
}

#[test]
fn loomtracer_bind_env_is_honored_and_refused_the_same_way() {
    let (ok, stderr) =
        Serve::spawn("refuse-env", &[], &[("LOOMTRACER_BIND", "0.0.0.0")]).wait_exit();
    assert!(!ok, "LOOMTRACER_BIND=0.0.0.0 without auth must be refused"); // ubs:ignore: test-only
    assert!(
        stderr.contains("refusing to serve without authentication"),
        "{stderr}"
    ); // ubs:ignore: test-only
}

#[test]
fn empty_api_key_is_unset_not_an_empty_token() {
    // `LOOMTRACER_API_KEY=` (compose passes empty values through) must not count
    // as "authenticated": an empty key accepts any request that sends no token.
    let (ok, stderr) = Serve::spawn(
        "refuse-empty-key",
        &["--bind", "0.0.0.0"],
        &[("LOOMTRACER_API_KEY", "")],
    )
    .wait_exit();
    assert!(!ok, "empty LOOMTRACER_API_KEY must not lift the refusal"); // ubs:ignore: test-only
    assert!(
        stderr.contains("refusing to serve without authentication"),
        "{stderr}"
    ); // ubs:ignore: test-only
}

// ---------------------------------------------------------------------------
// The two ways to serve a non-loopback bind legitimately
// ---------------------------------------------------------------------------

#[test]
fn explicit_opt_in_allows_unauthenticated_non_loopback_and_warns_loudly() {
    let serve = Serve::spawn(
        "opt-in",
        &["--bind", "0.0.0.0", "--allow-unauthenticated-remote"],
        &[],
    );
    let (lines, addr) = serve.wait_listening();
    assert!(
        addr.starts_with("0.0.0.0:"),
        "opt-in must bind the requested address, got {addr}"
    ); // ubs:ignore: test-only
    assert!(
        lines
            .iter()
            .any(|l| l.contains("--allow-unauthenticated-remote")
                && l.contains("WITHOUT authentication")),
        "opt-in must be logged loudly: {lines:#?}"
    ); // ubs:ignore: test-only
    assert_eq!(
        http_status(&loopback_of(&addr), "GET", "/v1/health", &[], ""),
        200
    ); // ubs:ignore: test-only
}

#[test]
fn api_key_allows_non_loopback_bind_and_is_enforced() {
    let serve = Serve::spawn(
        "api-key",
        &["--bind", "0.0.0.0"],
        &[("LOOMTRACER_API_KEY", "s3cret-key")],
    );
    let (lines, addr) = serve.wait_listening();
    assert!(addr.starts_with("0.0.0.0:"), "{addr}"); // ubs:ignore: test-only
    assert!(
        lines
            .iter()
            .any(|l| l.contains(LISTENING_MARKER) && l.contains("static api key")),
        "startup log must name the auth mode: {lines:#?}"
    ); // ubs:ignore: test-only
    let target = loopback_of(&addr);
    assert_eq!(
        http_status(&target, "GET", "/v1/decisions/search?q=x", &[], ""),
        401
    ); // ubs:ignore: test-only
    assert_eq!(
        http_status(
            &target,
            "GET",
            "/v1/decisions/search?q=x",
            &[("Authorization", "Bearer s3cret-key")],
            ""
        ),
        200
    ); // ubs:ignore: test-only
}

#[test]
fn empty_admin_key_does_not_unlock_admin_routes() {
    // Compose sets LOOMTRACER_ADMIN_KEY="" when the operator did not set it. An
    // empty admin key must mean "not configured", never "the empty token".
    let serve = Serve::spawn("empty-admin", &[], &[("LOOMTRACER_ADMIN_KEY", "")]);
    let (_, addr) = serve.wait_listening();
    let status = http_status(
        &addr,
        "POST",
        "/v1/users",
        &[("Content-Type", "application/json")],
        r#"{"email":"eve@example.com","display_name":"Eve"}"#,
    );
    assert!(
        !(200..300).contains(&status),
        "token-less admin call must not succeed, got {status}"
    ); // ubs:ignore: test-only
}

// ---------------------------------------------------------------------------
// Startup: the graph is built before the socket opens (hivemind-7jon8)
// ---------------------------------------------------------------------------

#[test]
fn serve_builds_the_graph_before_it_listens() {
    // The first graph read after a start replays the whole ledger. On the cell that took 13.4 s
    // inside a caller's request. The server builds the graph first, so nothing answers
    // `/v1/health` (the cell's readiness check) until reads are warm.
    let dir = unique_dir("warm-start");
    loomtracer::cli::run(&loomtracer::cli::Cli::parse_from([
        "loomtracer",
        "--loomtracer-dir",
        &dir.display().to_string(),
        "--actor",
        "agent:claude:warm-start",
        "emit",
        "decision.capture",
        "--binds-nothing",
        "--cheap-to-undo",
        "--bet",
        "--title",
        "Use SQLite for the prototype ledger",
        "--rationale",
        "Rationale text long enough for the readable floor",
        "--topic-keys",
        "storage",
        "--options",
        "sqlite,postgres",
        "--chose",
        "sqlite",
    ]))
    .expect("seed one decision"); // ubs:ignore: test-only

    let serve = Serve::spawn_in(dir, &[], &[]);
    let (lines, addr) = serve.wait_listening();
    let warmed = lines
        .iter()
        .position(|l| l.contains("graph cache warmed"))
        .unwrap_or_else(|| panic!("no warm-up line before listening: {lines:#?}")); // ubs:ignore: test-only
    assert!(
        warmed < lines.len() - 1,
        "the graph must be warm before the server reports listening: {lines:#?}"
    ); // ubs:ignore: test-only
    assert!(
        lines[warmed].contains("tenants=1") && lines[warmed].contains("warmed=1"),
        "the local tenant's graph must be built: {}",
        lines[warmed]
    ); // ubs:ignore: test-only
    assert_eq!(http_status(&addr, "GET", "/v1/health", &[], ""), 200); // ubs:ignore: test-only
}

// ---------------------------------------------------------------------------
// Layer-3 workers run only on the ledger they can read (hivemind-tbu78)
// ---------------------------------------------------------------------------

const KEY_IGNORED: &str = "ANTHROPIC_API_KEY is set but ignored";

#[test]
fn sqlite_server_with_an_anthropic_key_starts_the_classifier_and_the_scorer() {
    let serve = Serve::spawn(
        "layer3-sqlite",
        &[],
        &[("ANTHROPIC_API_KEY", "sk-ant-test-key")],
    );
    let (mut lines, _addr) = serve.wait_listening();
    for worker in ["classifier worker started", "scorer worker started"] {
        if !lines.iter().any(|l| l.contains(worker)) {
            serve.wait_line(worker, &mut lines);
        }
    }
    assert!(
        !lines.iter().any(|l| l.contains(KEY_IGNORED)),
        "the SQLite backend uses the key; it must not call it ignored: {lines:#?}"
    ); // ubs:ignore: test-only
}

/// A Postgres server holds every capture in the database. The workers read a SQLite file under
/// `LOOMTRACER_DIR`, so a key must not start them: they would create an empty `ledger.sqlite`
/// there and classify and score nothing. Needs `--features shared-backend-postgres` and
/// `LOOMTRACER_TEST_POSTGRES_URL`; without the URL the test says so and passes.
#[cfg(feature = "shared-backend-postgres")]
mod postgres {
    use super::*;

    fn test_database_url() -> Option<String> {
        match std::env::var("LOOMTRACER_TEST_POSTGRES_URL") {
            Ok(url) if !url.is_empty() => Some(url),
            _ => {
                eprintln!("skipping Postgres serve test: LOOMTRACER_TEST_POSTGRES_URL is not set");
                None
            }
        }
    }

    #[test]
    fn postgres_server_ignores_the_anthropic_key_and_leaves_no_sqlite_ledger() {
        let Some(url) = test_database_url() else {
            return;
        };
        let dir = unique_dir("layer3-postgres");
        let serve = Serve::spawn_in(
            dir.clone(),
            &[],
            &[
                ("LOOMTRACER_DATABASE_URL", url.as_str()),
                ("ANTHROPIC_API_KEY", "sk-ant-test-key"),
            ],
        );
        let (lines, _addr) = serve.wait_listening();
        let ignored = lines.iter().filter(|l| l.contains(KEY_IGNORED)).count();
        assert_eq!(
            ignored, 1,
            "the server must say once that the key is ignored: {lines:#?}"
        ); // ubs:ignore: test-only
        assert!(
            !lines.iter().any(|l| l.contains("worker started")),
            "no Layer-3 worker may start on Postgres: {lines:#?}"
        ); // ubs:ignore: test-only
           // A worker's first pass runs as soon as it is spawned, so a moment after the server
           // reports listening a worker that had started would have made the file.
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            !dir.join("ledger.sqlite").exists(),
            "a Postgres server must not leave a SQLite ledger under its LOOMTRACER_DIR"
        ); // ubs:ignore: test-only
    }

    #[test]
    fn postgres_server_without_a_key_has_nothing_to_ignore() {
        let Some(url) = test_database_url() else {
            return;
        };
        let serve = Serve::spawn(
            "layer3-postgres-keyless",
            &[],
            &[("LOOMTRACER_DATABASE_URL", url.as_str())],
        );
        let (lines, _addr) = serve.wait_listening();
        assert!(
            !lines.iter().any(|l| l.contains(KEY_IGNORED)),
            "no key is set, so none is ignored: {lines:#?}"
        ); // ubs:ignore: test-only
    }
}

// ---------------------------------------------------------------------------
// check_bind_safety matrix
// ---------------------------------------------------------------------------

fn config(
    bind: IpAddr,
    api_key: Option<&str>,
    database_url: Option<&str>,
    allow: bool,
) -> ApiConfig {
    let mut config = ApiConfig::new(unique_dir("matrix"));
    config.bind = bind;
    config.api_key = api_key.map(str::to_owned);
    config.database_url = database_url.map(str::to_owned);
    config.allow_unauthenticated_remote = allow;
    config
}

#[test]
fn check_bind_safety_matrix() {
    let v4_loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let v6_loopback = IpAddr::V6(Ipv6Addr::LOCALHOST);
    let v4_any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let v6_any = IpAddr::V6(Ipv6Addr::UNSPECIFIED);
    let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20));

    // Unauthenticated: loopback only.
    assert!(config(v4_loopback, None, None, false)
        .check_bind_safety()
        .is_ok()); // ubs:ignore: test-only
    assert!(config(v6_loopback, None, None, false)
        .check_bind_safety()
        .is_ok()); // ubs:ignore: test-only
    for remote in [v4_any, v6_any, lan] {
        assert!(
            config(remote, None, None, false)
                .check_bind_safety()
                .is_err(),
            "{remote}"
        ); // ubs:ignore: test-only
    }

    // Explicit opt-in lifts the refusal.
    for remote in [v4_any, v6_any, lan] {
        assert!(
            config(remote, None, None, true).check_bind_safety().is_ok(),
            "{remote}"
        ); // ubs:ignore: test-only
    }

    // Either kind of auth makes any bind acceptable.
    for remote in [v4_any, v6_any, lan] {
        assert!(
            config(remote, Some("k"), None, false)
                .check_bind_safety()
                .is_ok(),
            "{remote}"
        ); // ubs:ignore: test-only
        assert!(
            config(remote, None, Some("postgres://u:p@db/loomtracer"), false)
                .check_bind_safety()
                .is_ok(),
            "{remote}"
        ); // ubs:ignore: test-only
    }
}
