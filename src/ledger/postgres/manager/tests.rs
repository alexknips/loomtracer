// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use super::*;

#[test]
fn a_connect_error_without_a_server_reply_names_its_cause() {
    // Port 1 on localhost refuses the connection: an I/O cause, no DbError.
    let config: Config = "host=127.0.0.1 port=1 user=u connect_timeout=2"
        .parse()
        .expect("config");
    let tls = MakeTlsConnector::new(native_tls::TlsConnector::new().expect("tls"));
    let error = PgConnectionManager::new(config, tls)
        .connect()
        .err()
        .expect("nothing listens on port 1");
    let text = error.to_string();
    assert!(
        text.starts_with("error connecting to server: "),
        "the error keeps its own line and adds the cause: {text}"
    );
    assert!(text.len() > "error connecting to server: ".len(), "{text}");
}

/// The server's refusal, as the pool keeps it: not the bare `db error` that
/// `postgres::Error` displays. A role the server has never heard of is refused as "does not
/// exist" under `trust` and as "password authentication failed" under `scram-sha-256`, so
/// the test needs only that the role's name comes back in the server's words.
#[test]
fn a_connect_error_from_the_server_carries_the_servers_message() {
    let Ok(base_url) = std::env::var("LOOMTRACER_TEST_POSTGRES_URL") else {
        eprintln!("skipping Postgres connect-error test: LOOMTRACER_TEST_POSTGRES_URL is not set");
        return;
    };
    let mut config: Config = base_url.parse().expect("test database url");
    config.user("lt_no_such_role").password("not-its-password");
    let tls = MakeTlsConnector::new(native_tls::TlsConnector::new().expect("tls"));
    let error = PgConnectionManager::new(config, tls)
        .connect()
        .err()
        .expect("the server has no such role");
    let text = error.to_string();
    assert!(
        text.starts_with("db error: FATAL: ") && text.contains("lt_no_such_role"),
        "the server's message is in the error: {text}"
    );
}
