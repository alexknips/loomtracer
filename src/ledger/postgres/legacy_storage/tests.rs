// Parent module gates this file with #[cfg(test)]; repeat the marker so UBS can filter test-only assertions.
#[cfg(test)]
use postgres::{Client, NoTls};
use uuid::Uuid;

use crate::ledger::legacy_storage::StorageRenameReport;
use crate::ledger::postgres::{PostgresEventLedger, TenantStore};
use crate::projector::postgres::PostgresGraphView;

const TEST_DATABASE_URL_ENV: &str = "LOOMTRACER_TEST_POSTGRES_URL";

/// The tables as a database from before the rename to Loomtracer made them, with the indexes and
/// the foreign keys between them.
const OLD_SCHEMA: &str = "
    CREATE TABLE hm_tenants (
        tenant_id    text PRIMARY KEY,
        display_name text NOT NULL,
        created_at   timestamptz NOT NULL DEFAULT now()
    );
    CREATE TABLE hm_users (
        user_id      uuid PRIMARY KEY DEFAULT gen_random_uuid(),
        tenant_id    text NOT NULL REFERENCES hm_tenants(tenant_id),
        email        text NOT NULL,
        display_name text NOT NULL,
        role         text NOT NULL DEFAULT 'member',
        created_at   timestamptz NOT NULL DEFAULT now(),
        UNIQUE (tenant_id, email)
    );
    CREATE TABLE hm_tokens (
        token_id    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
        token_hash  text NOT NULL UNIQUE,
        tenant_id   text NOT NULL REFERENCES hm_tenants(tenant_id),
        user_id     uuid REFERENCES hm_users(user_id),
        actor_id    text NOT NULL DEFAULT 'service:api',
        label       text,
        revoked_at  timestamptz,
        created_at  timestamptz NOT NULL DEFAULT now()
    );
    CREATE INDEX hm_tokens_hash_idx ON hm_tokens (token_hash);
    CREATE TABLE hm_oidc_users (
        sub        text PRIMARY KEY,
        tenant_id  text NOT NULL REFERENCES hm_tenants(tenant_id),
        email      text,
        created_at timestamptz NOT NULL DEFAULT now()
    );
    CREATE TABLE hm_nodes (
        tenant_id   text    NOT NULL,
        node_kind   text    NOT NULL,
        node_id     text    NOT NULL,
        properties  jsonb   NOT NULL DEFAULT '{}',
        PRIMARY KEY (tenant_id, node_kind, node_id)
    );
    CREATE INDEX hm_nodes_tenant_kind_idx ON hm_nodes (tenant_id, node_kind);
    CREATE TABLE hm_edges (
        tenant_id     text    NOT NULL,
        relation_kind text    NOT NULL,
        from_id       text    NOT NULL,
        to_id         text    NOT NULL,
        event_origin  bigint,
        causation_event_id bigint,
        added_by      text,
        added_at      text,
        PRIMARY KEY (tenant_id, relation_kind, from_id, to_id)
    );
    CREATE INDEX hm_edges_from_idx ON hm_edges (tenant_id, relation_kind, from_id);
    CREATE INDEX hm_edges_to_idx ON hm_edges (tenant_id, relation_kind, to_id);
    INSERT INTO hm_tenants (tenant_id, display_name) VALUES ('local', 'Local'), ('acme', 'Acme');
    INSERT INTO hm_users (tenant_id, email, display_name) VALUES
        ('local', 'alice@example.test', 'Alice'), ('acme', 'bob@example.test', 'Bob');
    INSERT INTO hm_tokens (token_hash, tenant_id, user_id, actor_id)
        SELECT 'hash-' || email, tenant_id, user_id, 'human:' || email FROM hm_users;
    INSERT INTO hm_tokens (token_hash, tenant_id, actor_id) VALUES ('hash-agent', 'local', 'agent:claude:polecat');
    INSERT INTO hm_oidc_users (sub, tenant_id, email) VALUES ('sub-1', 'local', 'alice@example.test');
    INSERT INTO hm_nodes (tenant_id, node_kind, node_id) VALUES
        ('local', 'Decision', 'decision-1'), ('local', 'Decision', 'decision-2'), ('acme', 'Project', 'billing');
    INSERT INTO hm_edges (tenant_id, relation_kind, from_id, to_id, event_origin) VALUES
        ('local', 'PREMISED_ON', 'decision-1', 'decision-2', 7), ('local', 'SUPERSEDES', 'decision-2', 'decision-1', 9);
";

/// The tables the old schema holds, in the order a rename reports them, with their row counts.
const OLD_TABLES: [(&str, &str, u64); 6] = [
    ("hm_tenants", "lt_tenants", 2),
    ("hm_users", "lt_users", 2),
    ("hm_tokens", "lt_tokens", 3),
    ("hm_oidc_users", "lt_oidc_users", 1),
    ("hm_nodes", "lt_nodes", 3),
    ("hm_edges", "lt_edges", 2),
];

/// A schema of its own in the test database, so the old table names a test makes do not meet the
/// other tests, which all set up the real tables in `public`. `url` reaches it through
/// `search_path`.
struct LegacyDatabase {
    admin: Client,
    schema: String,
    url: String,
}

impl LegacyDatabase {
    /// `None` (and a line saying so) when no test database is configured.
    fn with_old_tables() -> Option<Self> {
        let Ok(base_url) = std::env::var(TEST_DATABASE_URL_ENV) else {
            eprintln!("skipping Postgres legacy-storage test: {TEST_DATABASE_URL_ENV} is not set");
            return None;
        };
        let mut admin = Client::connect(&base_url, NoTls).expect("connect to the test database");
        let schema = format!("lt_legacy_{}", Uuid::new_v4().simple());
        admin
            .batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET search_path TO {schema}; {OLD_SCHEMA}"
            ))
            .expect("make the old tables");
        let separator = if base_url.contains('?') { '&' } else { '?' };
        let url = format!("{base_url}{separator}options=-c%20search_path%3D{schema}");
        Some(Self { admin, schema, url })
    }

    fn ledger(&self) -> PostgresEventLedger {
        PostgresEventLedger::connect_with_pool_size(&self.url, "local", 2).expect("connect")
    }

    /// The names in this schema that start with `prefix`, tables and indexes, sorted.
    fn relations(&mut self, prefix: &str) -> Vec<String> {
        self.admin
            .query(
                "SELECT c.relname::text FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname = $1 AND left(c.relname, 3) = $2 AND c.relkind IN ('r', 'i')
                 ORDER BY c.relname",
                &[&self.schema, &prefix],
            )
            .expect("list relations")
            .into_iter()
            .map(|row| row.get(0))
            .collect()
    }

    fn constraints(&mut self, table: &str) -> Vec<String> {
        self.admin
            .query(
                "SELECT conname::text FROM pg_constraint
                 WHERE conrelid = $1::text::regclass ORDER BY conname",
                &[&format!("{}.{table}", self.schema)],
            )
            .expect("list constraints")
            .into_iter()
            .map(|row| row.get(0))
            .collect()
    }

    fn count(&mut self, table: &str) -> u64 {
        let row = self
            .admin
            .query_one(
                &format!("SELECT COUNT(*) FROM {}.{table}", self.schema),
                &[],
            )
            .expect("count");
        u64::try_from(row.get::<_, i64>(0)).unwrap_or(0)
    }
}

impl Drop for LegacyDatabase {
    fn drop(&mut self) {
        let _ = self
            .admin
            .batch_execute(&format!("DROP SCHEMA {} CASCADE", self.schema));
    }
}

fn renamed(report: &StorageRenameReport) -> Vec<(String, String, u64, u64)> {
    report
        .renamed
        .iter()
        .map(|table| {
            (
                table.from.clone(),
                table.to.clone(),
                table.rows_before,
                table.rows_after,
            )
        })
        .collect()
}

fn expected_renames() -> Vec<(String, String, u64, u64)> {
    OLD_TABLES
        .iter()
        .map(|(from, to, rows)| ((*from).to_owned(), (*to).to_owned(), *rows, *rows))
        .collect()
}

#[test]
fn every_table_takes_its_new_name_and_keeps_every_row() {
    let Some(mut database) = LegacyDatabase::with_old_tables() else {
        return;
    };
    let ledger = database.ledger();

    let report = ledger.rename_legacy_storage(false).expect("rename");

    assert!(!report.dry_run);
    assert_eq!(report.backend, "postgres");
    assert_eq!(renamed(&report), expected_renames());
    assert!(report.already_current.is_empty());
    assert!(
        database.relations("hm_").is_empty(),
        "no table or index may keep the old prefix: {:?}",
        database.relations("hm_")
    );
    for (_, to, rows) in OLD_TABLES {
        assert_eq!(database.count(to), rows, "{to} keeps its rows");
    }
}

#[test]
fn the_constraints_and_indexes_are_named_as_a_fresh_database_names_them() {
    let Some(mut database) = LegacyDatabase::with_old_tables() else {
        return;
    };
    database
        .ledger()
        .rename_legacy_storage(false)
        .expect("rename");

    assert_eq!(
        database.constraints("lt_tokens"),
        vec![
            "lt_tokens_pkey",
            "lt_tokens_tenant_id_fkey",
            "lt_tokens_token_hash_key",
            "lt_tokens_user_id_fkey"
        ]
    );
    let indexes = database.relations("lt_");
    for expected in [
        "lt_tokens_hash_idx",
        "lt_nodes_tenant_kind_idx",
        "lt_edges_from_idx",
        "lt_edges_to_idx",
        "lt_edges_pkey",
        "lt_users_tenant_id_email_key",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "{expected} is missing from {indexes:?}"
        );
    }
    // The key is enforced, not just spelled right: a token for a tenant that is not there is refused.
    let refused = database.admin.execute(
        "INSERT INTO lt_tokens (token_hash, tenant_id) VALUES ('stray', 'nobody')",
        &[],
    );
    assert!(
        refused.is_err(),
        "a token for a missing tenant must be refused"
    );
}

#[test]
fn the_stores_refuse_the_old_names_and_start_on_the_new_ones() {
    let Some(mut database) = LegacyDatabase::with_old_tables() else {
        return;
    };
    let ledger = database.ledger();

    let tenant_store = TenantStore::from_pool(ledger.pool().clone())
        .map(|_| ())
        .expect_err("the tenant store must refuse a database with the old names")
        .to_string();
    let graph = PostgresGraphView::connect_with_pool_size(&database.url, "local", 2)
        .map(|_| ())
        .expect_err("the graph store must refuse a database with the old names")
        .to_string();
    for refusal in [&tenant_store, &graph] {
        assert!(
            refusal.contains("loomtracer migrate-storage") && refusal.contains("hm_tenants"),
            "the refusal names the step and the tables: {refusal}"
        );
    }
    assert!(
        database.relations("lt_").is_empty(),
        "the refusals made no table beside the old ones"
    );

    ledger.rename_legacy_storage(false).expect("rename");

    let tenant_store = TenantStore::from_pool(ledger.pool().clone()).expect("tenant store");
    PostgresGraphView::connect_with_pool_size(&database.url, "local", 2).expect("graph store");
    assert_eq!(
        tenant_store.tenant_ids().expect("tenants"),
        vec!["acme", "local"],
        "both tenants survived"
    );
    assert!(database.relations("hm_").is_empty());
}

#[test]
fn a_dry_run_reports_what_a_run_does_and_changes_nothing() {
    let Some(mut database) = LegacyDatabase::with_old_tables() else {
        return;
    };
    let ledger = database.ledger();

    let dry = ledger.rename_legacy_storage(true).expect("dry run");

    assert!(dry.dry_run);
    assert_eq!(renamed(&dry), expected_renames());
    assert!(database.relations("lt_").is_empty());
    assert_eq!(database.count("hm_tokens"), 3);

    let real = ledger.rename_legacy_storage(false).expect("real run");
    assert_eq!(renamed(&real), renamed(&dry));
}

#[test]
fn a_second_run_has_nothing_to_rename() {
    let Some(database) = LegacyDatabase::with_old_tables() else {
        return;
    };
    let ledger = database.ledger();
    ledger.rename_legacy_storage(false).expect("first run");

    let again = ledger.rename_legacy_storage(false).expect("second run");

    assert!(again.renamed.is_empty());
    assert_eq!(again.already_current, OLD_TABLES.map(|(_, to, _)| to));
}

#[test]
fn a_table_under_both_names_is_refused_and_nothing_is_renamed() {
    let Some(mut database) = LegacyDatabase::with_old_tables() else {
        return;
    };
    database
        .admin
        .batch_execute("CREATE TABLE lt_edges (stray int)")
        .expect("a stray lt_edges");

    let refusal = database
        .ledger()
        .rename_legacy_storage(false)
        .expect_err("both names must be refused")
        .to_string();

    assert!(
        refusal.contains("both hm_edges and lt_edges exist"),
        "{refusal}"
    );
    assert_eq!(
        database.relations("lt_"),
        vec!["lt_edges"],
        "the tables renamed before the refusal were rolled back"
    );
    assert_eq!(database.count("hm_tenants"), 2);
}
