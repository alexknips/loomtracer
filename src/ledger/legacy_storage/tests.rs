// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::ledger::{SqliteEventLedger, SqliteUserStore};

use super::{rename_sqlite_storage, StorageRenameReport};

/// The two tables, and the index, as a ledger from before the rename to Loomtracer made them.
const OLD_SCHEMA: &str = "CREATE TABLE hm_users (
        user_id      TEXT PRIMARY KEY,
        tenant_id    TEXT NOT NULL DEFAULT 'local',
        email        TEXT NOT NULL,
        display_name TEXT NOT NULL,
        role         TEXT NOT NULL DEFAULT 'member',
        created_at   TEXT NOT NULL DEFAULT (datetime('now')),
        UNIQUE (tenant_id, email)
    );
    CREATE TABLE hm_tokens (
        token_id    TEXT PRIMARY KEY,
        token_hash  TEXT NOT NULL UNIQUE,
        tenant_id   TEXT NOT NULL DEFAULT 'local',
        user_id     TEXT REFERENCES hm_users(user_id),
        actor_id    TEXT NOT NULL DEFAULT 'service:api',
        label       TEXT,
        revoked_at  TEXT,
        created_at  TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE INDEX hm_tokens_hash_idx ON hm_tokens (token_hash);";

/// The secret of the one token `old_ledger` issues to Alice, as a client holds it.
const ALICE_TOKEN_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

struct OldLedger {
    dir: PathBuf,
    connection: Connection,
}

impl OldLedger {
    /// A ledger file with the old table names: two users, three tokens (one revoked, one for an
    /// agent with no user).
    fn new(prefix: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("loomtracer-{prefix}-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("create the ledger dir");
        let connection = Connection::open(dir.join("ledger.sqlite")).expect("open the ledger");
        connection.execute_batch(OLD_SCHEMA).expect("old schema");
        connection
            .execute_batch(
                "INSERT INTO hm_users (user_id, email, display_name) VALUES
                     ('user-alice', 'alice@example.test', 'Alice'),
                     ('user-bob', 'bob@example.test', 'Bob');",
            )
            .expect("users");
        let alice_hash = format!("{:x}", Sha256::digest(ALICE_TOKEN_HEX.as_bytes()));
        connection
            .execute(
                "INSERT INTO hm_tokens (token_id, token_hash, user_id, actor_id) VALUES
                     ('token-alice', ?1, 'user-alice', 'human:alice@example.test')",
                [alice_hash],
            )
            .expect("alice's token");
        connection
            .execute_batch(
                "INSERT INTO hm_tokens (token_id, token_hash, user_id, actor_id, revoked_at) VALUES
                     ('token-bob', 'hash-bob', 'user-bob', 'human:bob@example.test', '2026-10-01');
                 INSERT INTO hm_tokens (token_id, token_hash, actor_id) VALUES
                     ('token-agent', 'hash-agent', 'agent:claude:polecat');",
            )
            .expect("the other tokens");
        Self { dir, connection }
    }

    fn table_names(&self) -> Vec<String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table'
                   AND (name LIKE 'hm\\_%' ESCAPE '\\' OR name LIKE 'lt\\_%' ESCAPE '\\')
                 ORDER BY name",
            )
            .expect("prepare");
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("names");
        names
    }

    fn sql_of(&self, name: &str) -> Option<String> {
        self.connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .ok()
    }

    fn count(&self, table: &str) -> i64 {
        self.connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count")
    }
}

impl Drop for OldLedger {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn renamed(report: &StorageRenameReport) -> Vec<(&str, &str, u64, u64)> {
    report
        .renamed
        .iter()
        .map(|table| {
            (
                table.from.as_str(),
                table.to.as_str(),
                table.rows_before,
                table.rows_after,
            )
        })
        .collect()
}

#[test]
fn the_tables_take_their_new_names_and_keep_every_row() {
    let mut old = OldLedger::new("legacy-storage-rows");

    let report = rename_sqlite_storage(&mut old.connection, false).expect("rename");

    assert!(!report.dry_run);
    assert_eq!(report.backend, "sqlite");
    assert_eq!(
        renamed(&report),
        vec![
            ("hm_users", "lt_users", 2, 2),
            ("hm_tokens", "lt_tokens", 3, 3)
        ]
    );
    assert!(report.already_current.is_empty());
    assert_eq!(old.table_names(), vec!["lt_tokens", "lt_users"]);
    assert_eq!((old.count("lt_users"), old.count("lt_tokens")), (2, 3));
}

#[test]
fn the_foreign_key_and_the_index_follow_the_new_names() {
    let mut old = OldLedger::new("legacy-storage-keys");

    rename_sqlite_storage(&mut old.connection, false).expect("rename");

    let tokens_sql = old.sql_of("lt_tokens").expect("lt_tokens exists");
    assert!(
        tokens_sql.contains("REFERENCES \"lt_users\"")
            || tokens_sql.contains("REFERENCES lt_users"),
        "the foreign key must point at lt_users: {tokens_sql}"
    );
    assert!(old.sql_of("hm_tokens_hash_idx").is_none());
    assert!(old.sql_of("lt_tokens_hash_idx").is_some());
    // The key is enforced, not just spelled right: a token for a user that is not there is refused.
    old.connection
        .execute_batch("PRAGMA foreign_keys = ON")
        .expect("pragma");
    let refused = old.connection.execute(
        "INSERT INTO lt_tokens (token_id, token_hash, user_id) VALUES ('t', 'h', 'nobody')",
        [],
    );
    assert!(
        refused.is_err(),
        "a token for a missing user must be refused"
    );
}

#[test]
fn a_token_issued_before_the_rename_still_resolves_after_it() {
    let mut old = OldLedger::new("legacy-storage-token");
    let refusal = SqliteUserStore::open(&old.dir)
        .map(|_| ())
        .expect_err("the token store must refuse a ledger with the old names")
        .to_string();
    assert!(
        refusal.contains("loomtracer migrate-storage") && refusal.contains("hm_users, hm_tokens"),
        "the refusal names the step and the tables: {refusal}"
    );
    assert_eq!(
        old.table_names(),
        vec!["hm_tokens", "hm_users"],
        "the refusal made no table beside the old ones"
    );

    rename_sqlite_storage(&mut old.connection, false).expect("rename");

    let store = SqliteUserStore::open(&old.dir).expect("open after the rename");
    let resolved = store
        .resolve_token(&format!("lt_tk_{ALICE_TOKEN_HEX}"))
        .expect("resolve")
        .expect("alice's token still resolves");
    assert_eq!(resolved.actor_id, "human:alice@example.test");
    assert_eq!(
        store.list_users("local").expect("users").len(),
        2,
        "both users are still there"
    );
}

#[test]
fn a_dry_run_reports_what_a_run_does_and_changes_nothing() {
    let mut old = OldLedger::new("legacy-storage-dry");

    let dry = rename_sqlite_storage(&mut old.connection, true).expect("dry run");

    assert!(dry.dry_run);
    assert_eq!(
        renamed(&dry),
        vec![
            ("hm_users", "lt_users", 2, 2),
            ("hm_tokens", "lt_tokens", 3, 3)
        ]
    );
    assert_eq!(old.table_names(), vec!["hm_tokens", "hm_users"]);
    assert!(old.sql_of("hm_tokens_hash_idx").is_some());

    let real = rename_sqlite_storage(&mut old.connection, false).expect("real run");
    assert_eq!(renamed(&real), renamed(&dry));
}

#[test]
fn a_second_run_has_nothing_to_rename() {
    let mut old = OldLedger::new("legacy-storage-twice");
    rename_sqlite_storage(&mut old.connection, false).expect("first run");

    let again = rename_sqlite_storage(&mut old.connection, false).expect("second run");

    assert!(again.renamed.is_empty());
    assert_eq!(again.already_current, vec!["lt_users", "lt_tokens"]);
    assert_eq!((old.count("lt_users"), old.count("lt_tokens")), (2, 3));
}

#[test]
fn a_ledger_that_never_had_the_tables_has_nothing_to_rename() {
    let dir =
        std::env::temp_dir().join(format!("loomtracer-legacy-storage-none-{}", Uuid::new_v4()));
    let ledger = SqliteEventLedger::open(&dir).expect("open a fresh ledger");
    drop(ledger);

    let report = SqliteEventLedger::rename_legacy_storage(&dir, false).expect("rename");

    assert!(report.renamed.is_empty() && report.already_current.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_table_under_both_names_is_refused_and_nothing_is_renamed() {
    let mut old = OldLedger::new("legacy-storage-both");
    old.connection
        .execute_batch("CREATE TABLE lt_tokens (token_id TEXT PRIMARY KEY)")
        .expect("a stray lt_tokens");

    let refusal = rename_sqlite_storage(&mut old.connection, false)
        .expect_err("both names must be refused")
        .to_string();

    assert!(
        refusal.contains("both hm_tokens and lt_tokens exist"),
        "{refusal}"
    );
    assert_eq!(
        old.table_names(),
        vec!["hm_tokens", "hm_users", "lt_tokens"],
        "hm_users was renamed before the refusal and must have been rolled back"
    );
    assert_eq!((old.count("hm_users"), old.count("hm_tokens")), (2, 3));
}
